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
//! An agent session never sends a report: its sandbox is not a player's
//! game.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;

pub use hearken_report::{Invalid, Report, SendError, Sender, Session};

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
}

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
    sender: Arc<dyn Sender>,
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
            sender: Arc::new(hearken_report::HttpSender::new()),
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
        self.session = Some(session);
        self.logs = logs;
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
    /// asked. `None` when the game takes no reports.
    pub fn draft(
        &self,
        description: &str,
        reply_to: &str,
        include_logs: bool,
        crashed: bool,
    ) -> Option<Report> {
        let config = self.config.as_ref()?;
        let mut report = Report::new(&config.name, &config.version, description.trim())
            .with_reply_to(reply_to)
            .crashed(crashed);
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
        let report = reports
            .draft("It broke.", " me@example.com ", true, false)
            .unwrap();
        assert_eq!(report.app, "My Game");
        assert_eq!(report.version, "1.2.0");
        assert_eq!(report.commit.as_deref(), Some("abc"));
        assert_eq!(report.reply_to.as_deref(), Some("me@example.com"));
        assert_eq!(report.attachments.len(), 1);
        assert_eq!(report.attachments[0].name, "session.log");
        let without = reports.draft("It broke.", "", false, true).unwrap();
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
