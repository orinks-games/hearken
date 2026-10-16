//! Boot a game: data folder, log, single instance, window, settings,
//! speech, then the loop.

use std::time::Duration;

#[cfg(feature = "kira")]
use hearken_audio::KiraBackend;
use hearken_audio::{Audio, Backend, LooseClips, SilentBackend};
use hearken_speech::{SpeechEnv, SpeechSink, ThreadedSpeech};

use crate::app::App;
use crate::controller::ControllerManager;
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

/// Chooses the backend `run` opens: the silent backend when the game or the
/// player asked for it, else the game's own backend, else the engine's
/// default. Runs entirely off the game loop, on `Audio::deferred`'s opener
/// thread.
fn open_backend<G: Game>() -> Result<Box<dyn Backend>, String> {
    let silent_var = format!("{}_AUDIO_BACKEND", G::ENV_PREFIX);
    let forced_silent = std::env::var(&silent_var).is_ok_and(|v| v.eq_ignore_ascii_case("silent"));
    let dummy_driver = std::env::var("SDL_AUDIODRIVER").is_ok_and(|v| v == "dummy");
    if forced_silent || dummy_driver {
        log::info!("audio: using the silent backend ({silent_var}, or SDL_AUDIODRIVER=dummy)");
        return Ok(Box::new(SilentBackend::default()));
    }
    if let Some(r) = G::audio_backend() {
        if r.is_ok() {
            log::info!("audio: using the game's own backend");
        }
        return r;
    }
    default_backend(G::ENV_PREFIX)
}

#[cfg(feature = "kira")]
fn default_backend(_prefix: &str) -> Result<Box<dyn Backend>, String> {
    let backend = KiraBackend::open()?;
    log::info!("audio: using Kira");
    Ok(Box::new(backend))
}

#[cfg(not(feature = "kira"))]
fn default_backend(_prefix: &str) -> Result<Box<dyn Backend>, String> {
    log::info!("audio: using the silent backend (no backend built in)");
    Ok(Box::new(SilentBackend::default()))
}

/// Run `G` until the player quits. Returns the process exit code.
pub fn run<G: Game>(game: G::Data, first: impl FnOnce() -> Box<dyn State<G>>) -> i32 {
    let paths = Paths::discover(G::DATA_DIR, G::ENV_PREFIX);
    let mut guard = SingleInstanceGuard::new(&mutex_name(G::DATA_DIR));
    let first_copy = guard.acquire();
    // Only the copy holding the lock may move the last session's log aside.
    if first_copy {
        crate::logging::rotate(&paths, G::ENV_PREFIX);
    }
    crate::logging::init(&paths, G::ENV_PREFIX);
    if !first_copy {
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
    ctx.install_audio(Audio::deferred(
        open_backend::<G>,
        crate::sound::clip_source::<G>(LooseClips::discover(G::ENV_PREFIX)),
    ));
    ctx.controller = ControllerManager::new(
        ctx.settings.controller.enabled,
        Some(shell.controller_factory()),
    );
    ctx.notices.extend(notice);
    ctx.apply_speech_settings();

    let mut app = App::new(ctx, first());
    app.run(&mut shell);
    guard.release();
    0
}
