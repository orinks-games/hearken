//! Boot a game: data folder, log, single instance, window, settings,
//! speech, then the loop.

use std::time::Duration;

use hearken_speech::{SpeechEnv, SpeechSink, ThreadedSpeech};

use crate::app::App;
use crate::ctx::Ctx;
use crate::game::Game;
use crate::paths::Paths;
use crate::sdl_shell::SdlShell;
use crate::settings::Settings;
use crate::single_instance::{mutex_name, SingleInstanceGuard};
use crate::state::State;

/// What a second copy says before it closes.
pub fn already_running_line<G: Game>() -> String {
    format!("{} is already running.", G::TITLE)
}

fn start_speech<G: Game>() -> ThreadedSpeech {
    ThreadedSpeech::spawn(SpeechEnv::with_prefix(G::ENV_PREFIX))
}

/// Say `line` to a player the game is about to leave with nothing else,
/// then exit with `code`. The line is flushed to the voice, bounded, and
/// given time to be spoken before speech shuts down, which would otherwise
/// skip it.
fn say_and_exit<G: Game>(line: &str, code: i32) -> i32 {
    let mut speech = start_speech::<G>();
    speech.say(line, true);
    if !speech.flush(Duration::from_secs(3)) {
        log::warn!("speech did not take the last line in time");
    }
    // A software voice is still reading it.
    std::thread::sleep(Duration::from_millis(1500));
    speech.shutdown();
    code
}

/// Run `G` until the player quits. Returns the process exit code.
pub fn run<G: Game>(game: G::Data, first: impl FnOnce() -> Box<dyn State<G>>) -> i32 {
    let paths = Paths::discover(G::DATA_DIR, G::ENV_PREFIX);
    crate::logging::init(&paths, G::ENV_PREFIX);

    let mut guard = SingleInstanceGuard::new(&mutex_name(G::DATA_DIR));
    if !guard.acquire() {
        let line = already_running_line::<G>();
        log::warn!("{line}");
        return say_and_exit::<G>(&line, 0);
    }

    // The window first, so a failure speaks through one voice started for
    // that line alone.
    let mut shell = match SdlShell::new(G::TITLE) {
        Ok(shell) => shell,
        Err(e) => {
            log::error!("could not open a window: {e}");
            return say_and_exit::<G>("The game window could not be opened.", 2);
        }
    };

    let (settings, notice) = Settings::load(&paths);
    let mut ctx = Ctx::new(Box::new(start_speech::<G>()), settings, paths, game);
    ctx.notices.extend(notice);
    ctx.apply_speech_settings();

    let mut app = App::new(ctx, first());
    app.run(&mut shell);
    guard.release();
    0
}
