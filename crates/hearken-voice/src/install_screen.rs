//! The in-game installer: a question, then a screen that downloads the
//! voice package and tells the commentary to look for it again.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

use hearken_app::screens::ConfirmScreen;
use hearken_app::{Ctx, Game, InputEvent, Key, State};

/// What the Confirm screen asks.
pub const INSTALL_QUESTION: &str = "Download the AI commentary voices? They are about 92 \
megabytes, come from Hugging Face, and need no account. Once downloaded they work offline.";

/// The question, with Yes starting the download into `voices` (usually
/// `ctx.paths.data_dir.join("voices")`, which `Voice` looks in).
pub fn install_confirm<G: Game>(voices: PathBuf) -> ConfirmScreen<G> {
    ConfirmScreen::new(INSTALL_QUESTION, move |ctx: &mut Ctx<G>| {
        ctx.replace(InstallScreen::new(voices.clone()));
    })
    .yes_label("Download")
    .no_label("Not now")
}

/// Downloads and checks the package, reading progress every quarter.
/// Escape cancels. When it is done the commentary reloads and the screen
/// closes with the result.
pub struct InstallScreen {
    voices: PathBuf,
    got: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
    done: Option<Receiver<Result<PathBuf, String>>>,
    said_quarter: u64,
}

impl InstallScreen {
    pub fn new(voices: PathBuf) -> Self {
        InstallScreen {
            voices,
            got: Arc::default(),
            total: Arc::default(),
            cancel: Arc::default(),
            done: None,
            said_quarter: 0,
        }
    }

    fn percent(&self) -> u64 {
        let total = self.total.load(Ordering::Relaxed);
        (self.got.load(Ordering::Relaxed) * 100)
            .checked_div(total)
            .unwrap_or(0)
            .min(100)
    }

    fn status(&self) -> String {
        format!(
            "Downloading the AI voices: {} percent. Escape cancels.",
            self.percent()
        )
    }
}

impl<G: Game> State<G> for InstallScreen {
    fn name(&self) -> &'static str {
        "install_voices"
    }

    fn enter(&mut self, ctx: &mut Ctx<G>) {
        let (tx, rx) = mpsc::channel();
        self.done = Some(rx);
        let (voices, got, total, cancel) = (
            self.voices.clone(),
            self.got.clone(),
            self.total.clone(),
            self.cancel.clone(),
        );
        let spawned = std::thread::Builder::new()
            .name("hearken-voice-install".into())
            .spawn(move || {
                let result = crate::install(
                    &voices,
                    &mut |g, t| {
                        got.store(g, Ordering::Relaxed);
                        total.store(t, Ordering::Relaxed);
                    },
                    &cancel,
                );
                let _ = tx.send(result.map_err(|e| e.to_string()));
            });
        if let Err(e) = spawned {
            ctx.pop_with(format!("The download could not start: {e}"));
            return;
        }
        ctx.say("Downloading the AI voices. Escape cancels.");
    }

    fn handle_input(&mut self, ctx: &mut Ctx<G>, event: &InputEvent) {
        match event.pressed() {
            Some(Key::Escape) => {
                self.cancel.store(true, Ordering::Relaxed);
                ctx.say("Cancelling.");
            }
            Some(_) => ctx.say(self.status()),
            None => {}
        }
    }

    fn update(&mut self, ctx: &mut Ctx<G>, _dt: f64) {
        let quarter = self.percent() / 25;
        if quarter > self.said_quarter && quarter < 4 {
            self.said_quarter = quarter;
            ctx.say_queued(format!("{} percent.", quarter * 25));
        }
        let Some(result) = self.done.as_ref().and_then(|rx| rx.try_recv().ok()) else {
            return;
        };
        self.done = None;
        match result {
            Ok(_) => {
                ctx.reload_commentary();
                ctx.pop_with("The AI voices are installed.");
            }
            Err(_) if self.cancel.load(Ordering::Relaxed) => {
                ctx.pop_with("Download cancelled.");
            }
            Err(e) => {
                log::warn!("voice install failed: {e}");
                ctx.pop_with(format!("The download failed: {e}"));
            }
        }
    }

    fn lines(&self, _ctx: &Ctx<G>) -> Vec<String> {
        vec!["Install AI voices".into(), self.status()]
    }

    fn help(&self, _ctx: &Ctx<G>) -> String {
        self.status()
    }
}
