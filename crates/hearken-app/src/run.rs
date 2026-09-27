//! Boot a game: data folder, log, single instance, settings, speech,
//! window, then the loop.

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

fn start_speech<G: Game>() -> Box<dyn SpeechSink> {
    Box::new(ThreadedSpeech::spawn(SpeechEnv::with_prefix(G::ENV_PREFIX)))
}

/// Run `G` until the player quits. Returns the process exit code.
pub fn run<G: Game>(game: G::Data, first: impl FnOnce() -> Box<dyn State<G>>) -> i32 {
    let paths = Paths::discover(G::DATA_DIR, G::ENV_PREFIX);
    crate::logging::init(&paths, G::ENV_PREFIX);

    let mut guard = SingleInstanceGuard::new(&mutex_name(G::DATA_DIR));
    if !guard.acquire() {
        let line = already_running_line::<G>();
        log::warn!("{line}");
        let mut speech = start_speech::<G>();
        speech.say(&line, true);
        speech.shutdown();
        return 0;
    }

    let (settings, notice) = Settings::load(&paths);
    let mut ctx = Ctx::new(start_speech::<G>(), settings, paths, game);
    ctx.notices.extend(notice);
    ctx.apply_speech_settings();

    let mut shell = match SdlShell::new(G::TITLE) {
        Ok(shell) => shell,
        Err(e) => {
            log::error!("could not open a window: {e}");
            ctx.speech.shutdown();
            return 2;
        }
    };
    let mut app = App::new(ctx, first());
    app.run(&mut shell);
    guard.release();
    0
}
