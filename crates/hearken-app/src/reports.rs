//! Problem reports: `ctx.reports`.
//!
//! A game names the relay its reports go to in `Game::reports`. The player
//! describes a problem from Settings (or wherever the game puts the
//! [`crate::screens::ReportScreen`]), the engine attaches the session log
//! and the one before it, and the relay emails the report to the
//! developer. A run that never exits cleanly is noticed at the next start,
//! and the game offers to report it once it is on a screen that allows an
//! offer (see [`crate::State::allows_update_prompt`]). Everything that is
//! not a screen is in `hearken-report`.
//!
//! A report also says where the player was, as the game tells it in
//! [`Game::report_details`]: read live for a report the player starts, and
//! kept in the crash marker as it changes, so a report of a crash names
//! where the last run was.
//!
//! An agent session never sends a report: its sandbox is not a player's
//! game.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;

pub use hearken_report::{Invalid, Opened, Report, SendError, Sender, Session};

use crate::ctx::Ctx;
use crate::game::Game;
use crate::paths::Paths;

/// The file the report is saved to when it cannot be sent, in the data
/// folder.
pub const SAVED_REPORT: &str = "problem-report.txt";

/// Where a game's reports go, and what they say about the game.
#[derive(Clone, Debug)]
pub struct ReportConfig {
    /// The relay's address, such as `https://example.com/api/problem-reports`.
    pub endpoint: String,
    /// The game's name as players hear it, and as the relay knows it.
    pub name: String,
    /// The game's version.
    pub version: String,
    /// The commit the game was built from.
    pub commit: Option<String>,
    /// Whether a start after a crash offers to report it.
    pub offer_after_crash: bool,
    /// Whether the report is written on the relay's web page, in the
    /// browser, with the logs sent ahead; otherwise in the game's own
    /// screen.
    pub web_form: bool,
}

impl ReportConfig {
    /// Reports for the game called `name` at `version`, sent to the relay
    /// at `endpoint`. `None` unless the endpoint is HTTPS, or HTTP on this
    /// computer for trying a relay.
    pub fn new(
        name: impl Into<String>,
        version: impl Into<String>,
        endpoint: impl Into<String>,
    ) -> Option<ReportConfig> {
        let endpoint = endpoint.into();
        let local = endpoint.starts_with("http://127.0.0.1")
            || endpoint.starts_with("http://localhost")
            || endpoint.starts_with("http://[::1]");
        if !endpoint.starts_with("https://") && !local {
            return None;
        }
        Some(ReportConfig {
            endpoint,
            name: name.into(),
            version: version.into(),
            commit: None,
            offer_after_crash: true,
            web_form: true,
        })
    }

    /// The commit the game was built from, for the report's details.
    pub fn commit(mut self, commit: impl Into<String>) -> ReportConfig {
        self.commit = Some(commit.into());
        self
    }

    /// Never asks to report a crash; the player reports from Settings.
    pub fn no_offer_after_crash(mut self) -> ReportConfig {
        self.offer_after_crash = false;
        self
    }

    /// The report is written in the game's own screen, never in the
    /// browser: for a relay with no web form, such as `mailbag-server`.
    pub fn in_game_form(mut self) -> ReportConfig {
        self.web_form = false;
        self
    }
}

/// Opens `url` in the player's browser, `true` when it did.
pub type Opener = Box<dyn Fn(&str) -> bool + Send>;

/// The report service: the game's relay, the logs a report carries, and
/// whether the last run crashed.
pub struct Reports {
    config: Option<ReportConfig>,
    session: Option<Session>,
    crashed_last_run: bool,
    /// Whether the offer to report the crash is still to be made.
    offer_pending: bool,
    /// The session log and the one before it, attached when they exist.
    logs: Vec<PathBuf>,
    /// Where the last run was when it ended unexpectedly, from its note.
    crash_details: Vec<(String, String)>,
    /// Where this run is, as last noted in the session marker.
    noted: Vec<(String, String)>,
    sender: Arc<dyn Sender>,
    /// Opens the web form in the browser; `None` in tests and agent runs.
    opener: Option<Opener>,
}

impl Reports {
    /// The service for `config`; dormant for `None`.
    pub fn new(config: Option<ReportConfig>) -> Reports {
        Reports {
            config,
            session: None,
            crashed_last_run: false,
            offer_pending: false,
            logs: Vec::new(),
            crash_details: Vec::new(),
            noted: Vec::new(),
            sender: Arc::new(hearken_report::HttpSender::new()),
            opener: None,
        }
    }

    /// No reports: the game named no relay, or this is an agent session.
    pub fn dormant() -> Reports {
        Reports::new(None)
    }

    /// Whether the game takes problem reports, so Settings lists the row.
    pub fn is_offered(&self) -> bool {
        self.config.is_some()
    }

    /// Whether a report is written on the relay's web page (see
    /// [`crate::screens::ReportLinkScreen`]) rather than in the game.
    pub fn uses_web_form(&self) -> bool {
        self.config.as_ref().is_some_and(|c| c.web_form)
    }

    pub fn config(&self) -> Option<&ReportConfig> {
        self.config.as_ref()
    }

    /// The game's name as players hear it; empty when reports are off.
    pub fn name(&self) -> &str {
        self.config.as_ref().map_or("", |c| c.name.as_str())
    }

    /// Takes over the run's [`Session`], begun as early as the data folder
    /// was known, and the logs a report attaches: the session log first,
    /// then the one before it.
    pub fn adopt_session(&mut self, session: Session, logs: Vec<PathBuf>) {
        self.crashed_last_run = session.ended_unexpectedly();
        self.offer_pending =
            self.crashed_last_run && self.config.as_ref().is_some_and(|c| c.offer_after_crash);
        if self.crashed_last_run {
            log::warn!("the last run did not exit cleanly");
        }
        self.crash_details = session.last_note().map(parse_note).unwrap_or_default();
        self.session = Some(session);
        self.logs = logs;
    }

    /// Where the last run was when it ended unexpectedly, as it last noted;
    /// empty when it did not crash or had noted nothing.
    pub fn crash_details(&self) -> &[(String, String)] {
        &self.crash_details
    }

    /// Where this run is, as last noted in the session marker.
    pub fn noted_details(&self) -> &[(String, String)] {
        &self.noted
    }

    /// Notes where this run is in the session marker, for a report of a
    /// crash at the next start. Writes only when `details` changed.
    pub fn note_details(&mut self, details: Vec<(String, String)>) {
        if details == self.noted {
            return;
        }
        if let Some(session) = self.session.as_mut() {
            session.note(&note_text(&details));
        }
        self.noted = details;
    }

    /// Records a clean exit. Called once as the game closes.
    pub fn end_session(&mut self) {
        if let Some(session) = self.session.take() {
            session.end();
        }
    }

    /// Whether the last run ended without exiting cleanly.
    pub fn crashed_last_run(&self) -> bool {
        self.crashed_last_run
    }

    /// Whether the offer to report the crash is still to be made.
    pub fn offers_crash_report(&self) -> bool {
        self.offer_pending && self.config.is_some()
    }

    /// Takes the one offer to report the crash: true the first time after a
    /// crashed run, false after that.
    pub fn take_crash_offer(&mut self) -> bool {
        std::mem::take(&mut self.offer_pending)
    }

    /// The logs a report attaches, those that exist right now.
    pub fn logs(&self) -> Vec<&Path> {
        self.logs
            .iter()
            .filter(|p| p.is_file())
            .map(PathBuf::as_path)
            .collect()
    }

    /// Whether any log exists to attach.
    pub fn has_logs(&self) -> bool {
        !self.logs().is_empty()
    }

    /// The report the player's answers make, with the logs attached when
    /// asked and `details` of where the player was (see [`details_for`]).
    /// `None` when the game takes no reports.
    pub fn draft(
        &self,
        description: &str,
        reply_to: &str,
        include_logs: bool,
        crashed: bool,
        details: &[(String, String)],
    ) -> Option<Report> {
        let config = self.config.as_ref()?;
        let mut report = Report::new(&config.name, &config.version, description.trim())
            .with_reply_to(reply_to)
            .crashed(crashed)
            .with_details(details.iter().cloned());
        if let Some(commit) = &config.commit {
            report = report.with_commit(commit);
        }
        if include_logs {
            for path in self.logs() {
                let Some(name) = path.file_name() else {
                    continue;
                };
                match fs::read(path) {
                    Ok(bytes) => {
                        report = report
                            .with_log(name.to_string_lossy(), &String::from_utf8_lossy(&bytes));
                    }
                    Err(e) => log::warn!("could not read {} for the report: {e}", path.display()),
                }
            }
        }
        Some(report)
    }

    /// Sends `report` to the relay on a worker thread. Poll the result from
    /// the screen; the game loop never waits on the network.
    pub fn send(&self, report: Report) -> Sending {
        let (tx, rx) = mpsc::channel();
        let sender = self.sender.clone();
        let endpoint = self.config.as_ref().map(|c| c.endpoint.clone());
        std::thread::spawn(move || {
            let result = match endpoint {
                Some(endpoint) => report.send_with(&endpoint, sender.as_ref()),
                None => Err(SendError::Failed("this game takes no reports".into())),
            };
            match &result {
                Ok(()) => log::info!("problem report sent"),
                Err(e) => log::warn!("problem report not sent: {e}"),
            }
            // The screen may have been left; then nobody is listening.
            let _ = tx.send(result);
        });
        Sending { rx }
    }

    /// Sends `report`'s logs ahead to the relay on a worker thread and
    /// brings back the page to write the rest on. Poll it from the screen.
    pub fn open(&self, report: Report) -> Opening {
        let (tx, rx) = mpsc::channel();
        let sender = self.sender.clone();
        let endpoint = self.config.as_ref().map(|c| c.endpoint.clone());
        std::thread::spawn(move || {
            let result = match endpoint {
                Some(endpoint) => report.open_with(&endpoint, sender.as_ref()),
                None => Err(SendError::Failed("this game takes no reports".into())),
            };
            match &result {
                Ok(opened) => log::info!("problem report opened at {}", opened.url),
                Err(e) => log::warn!("problem report not opened: {e}"),
            }
            let _ = tx.send(result);
        });
        Opening { rx }
    }

    /// Opens the web form at `url` in the player's browser with the opener
    /// the app set; false when there is none or it could not.
    pub fn open_url(&self, url: &str) -> bool {
        self.opener.as_ref().is_some_and(|opener| opener(url))
    }

    /// Opens pages in the browser with `opener`, as the app does at start.
    pub fn set_opener(&mut self, opener: Opener) {
        self.opener = Some(opener);
    }

    /// Saves `report` as [`SAVED_REPORT`] in the data folder, for a player
    /// to send another way, and says where.
    pub fn save(&self, report: &Report, paths: &Paths) -> io::Result<PathBuf> {
        let path = paths.data_dir.join(SAVED_REPORT);
        fs::write(&path, report.to_text())?;
        Ok(path)
    }

    /// Sends reports through `sender` instead, as a test does.
    pub fn set_sender(&mut self, sender: Box<dyn Sender>) {
        self.sender = Arc::from(sender);
    }
}

/// Where the player is, for a report: for a report of a crash, where the
/// last run was when it ended; otherwise where the game says it is now.
pub fn details_for<G: Game>(ctx: &Ctx<G>, crashed: bool) -> Vec<(String, String)> {
    if crashed {
        ctx.reports.crash_details().to_vec()
    } else {
        G::report_details(ctx)
    }
}

/// Opens Report a problem the way the game is set up for: the relay's web
/// page, or the game's own screen. For a Settings row or a pause menu.
pub fn open_report_screen<G: Game>(ctx: &mut Ctx<G>) {
    if ctx.reports.uses_web_form() {
        ctx.push(crate::screens::ReportLinkScreen::<G>::new());
    } else {
        ctx.push(crate::screens::ReportScreen::<G>::new());
    }
}

/// The session marker's note: one detail a line, name and value parted by
/// a tab.
fn note_text(details: &[(String, String)]) -> String {
    details
        .iter()
        .map(|(name, value)| {
            format!(
                "{}\t{}",
                name.replace(['\t', '\n', '\r'], " "),
                value.replace(['\t', '\n', '\r'], " ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_note(note: &str) -> Vec<(String, String)> {
    note.lines()
        .filter_map(|line| line.split_once('\t'))
        .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
        .filter(|(name, value)| !name.is_empty() && !value.is_empty())
        .collect()
}

/// A report on its way to the relay.
pub struct Sending {
    rx: Receiver<Result<(), SendError>>,
}

impl Sending {
    /// The outcome, once there is one. The worker dying counts as a failure.
    pub fn poll(&mut self) -> Option<Result<(), SendError>> {
        match self.rx.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Err(SendError::Failed("the send stopped".into())))
            }
        }
    }
}

/// A report's logs on their way to the relay, ahead of the web form.
pub struct Opening {
    rx: Receiver<Result<Opened, SendError>>,
}

impl Opening {
    /// The outcome, once there is one. The worker dying counts as a failure.
    pub fn poll(&mut self) -> Option<Result<Opened, SendError>> {
        match self.rx.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Err(SendError::Failed("the send stopped".into())))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_or_a_local_relay_is_accepted() {
        assert!(ReportConfig::new("G", "1", "https://example.com/r").is_some());
        assert!(ReportConfig::new("G", "1", "http://127.0.0.1:8025/r").is_some());
        assert!(ReportConfig::new("G", "1", "http://localhost:8025/r").is_some());
        assert!(ReportConfig::new("G", "1", "http://example.com/r").is_none());
        assert!(ReportConfig::new("G", "1", "example.com").is_none());
    }

    #[test]
    fn a_draft_carries_the_game_and_the_logs_that_exist() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("session.log");
        fs::write(&log, "line one\n").unwrap();
        let mut reports = Reports::new(Some(
            ReportConfig::new("My Game", "1.2.0", "https://example.com/r")
                .unwrap()
                .commit("abc"),
        ));
        reports.adopt_session(
            Session::begin(&dir.path().join("session.running")),
            vec![log, dir.path().join("session.log.prev")],
        );
        assert!(reports.has_logs());
        let details = [("Game".to_string(), "Belfry".to_string())];
        let report = reports
            .draft("It broke.", " me@example.com ", true, false, &details)
            .unwrap();
        assert_eq!(report.headline(), Some("Belfry"));
        assert_eq!(report.app, "My Game");
        assert_eq!(report.version, "1.2.0");
        assert_eq!(report.commit.as_deref(), Some("abc"));
        assert_eq!(report.reply_to.as_deref(), Some("me@example.com"));
        assert_eq!(report.attachments.len(), 1);
        assert_eq!(report.attachments[0].name, "session.log");
        let without = reports.draft("It broke.", "", false, true, &[]).unwrap();
        assert!(without.details.is_empty());
        assert!(without.attachments.is_empty());
        assert!(without.crashed);
        reports.end_session();
    }

    #[test]
    fn the_crash_offer_is_made_once_and_only_when_wanted() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("session.running");
        drop(Session::begin(&marker));
        let mut reports = Reports::new(ReportConfig::new("G", "1", "https://example.com/r"));
        reports.adopt_session(Session::begin(&marker), Vec::new());
        assert!(reports.crashed_last_run());
        assert!(reports.offers_crash_report());
        assert!(reports.take_crash_offer());
        assert!(!reports.take_crash_offer());
        reports.end_session();

        drop(Session::begin(&marker));
        let mut quiet = Reports::new(
            ReportConfig::new("G", "1", "https://example.com/r").map(|c| c.no_offer_after_crash()),
        );
        quiet.adopt_session(Session::begin(&marker), Vec::new());
        assert!(quiet.crashed_last_run());
        assert!(!quiet.offers_crash_report());
        quiet.end_session();
    }
}
