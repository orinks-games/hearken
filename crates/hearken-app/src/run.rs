//! Boot a game: data folder, log, single instance, window, settings,
//! speech, then the loop.

use std::time::Duration;

#[cfg(feature = "kira")]
use hearken_audio::KiraBackend;
use hearken_audio::{Audio, Backend, SilentBackend};
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
    // An agent session waits for its first tool call before anything opens,
    // saves in a sandbox, and holds a lock of its own (see `agent_server`).
    #[cfg(feature = "agent-server")]
    let mut agent = if crate::agent_server::asked() {
        match crate::agent_server::AgentServer::start() {
            Some(agent) => Some(agent),
            None => return 0,
        }
    } else {
        None
    };
    #[cfg(feature = "agent-server")]
    let (paths, lock) = if agent.is_some() {
        let real = Paths::discover(G::DATA_DIR, G::ENV_PREFIX);
        (
            real.nested("agent-sandbox"),
            format!("{}-agent", G::DATA_DIR),
        )
    } else {
        (
            Paths::discover(G::DATA_DIR, G::ENV_PREFIX),
            G::DATA_DIR.to_string(),
        )
    };
    #[cfg(not(feature = "agent-server"))]
    let (paths, lock) = (
        Paths::discover(G::DATA_DIR, G::ENV_PREFIX),
        G::DATA_DIR.to_string(),
    );
    let mut guard = SingleInstanceGuard::new(&mutex_name(&lock));
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
    let sounds = hearken_audio::discover_clips(G::ENV_PREFIX);
    ctx.install_audio(Audio::deferred(
        open_backend::<G>,
        crate::sound::clip_source::<G>(sounds.clips),
    ));
    ctx.controller = ControllerManager::new(
        ctx.settings.controller.enabled,
        Some(shell.controller_factory()),
    );
    ctx.notices.extend(notice);
    ctx.notices.extend(sounds.notice);
    ctx.apply_speech_settings();

    // An agent session plays in a sandbox and never shows on the player's
    // Discord.
    #[cfg(feature = "agent-server")]
    let sandboxed = agent.is_some();
    #[cfg(not(feature = "agent-server"))]
    let sandboxed = false;
    if let (Some(config), false) = (G::discord(), sandboxed) {
        ctx.discord = crate::discord::Discord::start(&config, ctx.settings.discord.enabled);
    }

    let mut app = App::new(ctx, first());
    #[cfg(feature = "agent-server")]
    if let Some(agent) = agent.as_mut() {
        agent.attach(&mut app);
        app.run_paced(&mut shell, |app, events, dt| agent.frame(app, events, dt));
        agent.finish(&mut app);
        guard.release();
        return 0;
    }
    app.run(&mut shell);
    guard.release();
    0
}
