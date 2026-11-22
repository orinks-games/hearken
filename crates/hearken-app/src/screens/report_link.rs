//! Report a problem on the relay's web page: the game sends its logs
//! ahead, then opens the page in the browser, where the player writes what
//! happened with the logs already attached.
//!
//! The send runs on a worker thread; this screen polls it each frame, so
//! the game never waits on the network and Escape leaves at once. When the
//! relay cannot take the logs, or the browser will not open, the game's
//! own [`ReportScreen`] takes over and says why.

use crate::ctx::Ctx;
use crate::game::Game;
use crate::input::{InputEvent, Key};
use crate::reports::{Opening, SendError};
use crate::screens::ReportScreen;
use crate::sound::UiSound;
use crate::state::State;
use crate::text::end_sentence;

/// Spoken on arrival, while the logs go ahead.
pub const PREPARING: &str = "Preparing the report.";
/// Spoken as the screen closes once the browser has the page.
pub const OPENED: &str =
    "The report opened in your browser. The game log is already attached; finish it there.";
/// Why the game's own screen opened instead: the browser would not.
pub const NO_BROWSER: &str = "The browser could not be opened, so write the report here.";

pub struct ReportLinkScreen<G: Game> {
    /// Whether this report follows a run that did not exit cleanly.
    crashed: bool,
    opening: Option<Opening>,
    _game: std::marker::PhantomData<G>,
}

impl<G: Game> ReportLinkScreen<G> {
    /// A report of a problem the player noticed.
    pub fn new() -> Self {
        ReportLinkScreen::for_crash(false)
    }

    /// A report of the last run closing unexpectedly, offered at start.
    pub fn after_crash() -> Self {
        ReportLinkScreen::for_crash(true)
    }

    /// The screen for a report that follows a crash or not.
    pub fn for_crash(crashed: bool) -> Self {
        ReportLinkScreen {
            crashed,
            opening: None,
            _game: std::marker::PhantomData,
        }
    }

    /// The game's own screen instead, saying why.
    fn fall_back(&mut self, ctx: &mut Ctx<G>, notice: String) {
        ctx.play_ui(UiSound::Error);
        ctx.replace(ReportScreen::<G>::for_crash(self.crashed).with_notice(notice));
    }
}

impl<G: Game> Default for ReportLinkScreen<G> {
    fn default() -> Self {
        ReportLinkScreen::new()
    }
}

impl<G: Game> State<G> for ReportLinkScreen<G> {
    fn name(&self) -> &'static str {
        "report_link"
    }

    fn enter(&mut self, ctx: &mut Ctx<G>) {
        let details = crate::reports::details_for(ctx, self.crashed);
        let Some(report) = ctx.reports.draft("", "", true, self.crashed, &details) else {
            ctx.pop();
            return;
        };
        ctx.say(PREPARING);
        self.opening = Some(ctx.reports.open(report));
    }

    fn handle_input(&mut self, ctx: &mut Ctx<G>, event: &InputEvent) {
        if event.is_repeat() {
            return;
        }
        match event.pressed() {
            // The worker keeps going; a page nobody opens expires on the relay.
            Some(Key::Escape) => ctx.pop(),
            Some(Key::Return | Key::Space | Key::KpEnter) => ctx.say(PREPARING),
            _ => {}
        }
    }

    fn update(&mut self, ctx: &mut Ctx<G>, _dt: f64) {
        let Some(outcome) = self.opening.as_mut().and_then(Opening::poll) else {
            return;
        };
        self.opening = None;
        match outcome {
            Ok(opened) if ctx.reports.open_url(&opened.url) => ctx.pop_with(OPENED),
            Ok(_) => self.fall_back(ctx, NO_BROWSER.to_string()),
            Err(SendError::RateLimited) => self.fall_back(
                ctx,
                "Too many reports have been sent today, so write the report here.".to_string(),
            ),
            Err(error) => {
                let reason = match error {
                    SendError::Rejected(reason) | SendError::Failed(reason) => reason,
                    other => other.to_string(),
                };
                self.fall_back(
                    ctx,
                    format!(
                        "The report form could not be opened: {} Write the report here.",
                        end_sentence(&reason)
                    ),
                );
            }
        }
    }

    fn lines(&self, _ctx: &Ctx<G>) -> Vec<String> {
        vec!["Report a problem".to_string(), PREPARING.to_string()]
    }

    fn help(&self, _ctx: &Ctx<G>) -> String {
        "The game is sending its log ahead, then opens the report form in your browser. \
         Escape goes back."
            .to_string()
    }

    fn phase(&self) -> Option<&'static str> {
        Some("preparing")
    }
}
