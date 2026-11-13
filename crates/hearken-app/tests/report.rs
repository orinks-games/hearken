//! Reporting a problem: the screen, the send, what a refusal does, saving
//! the report when the relay will not take it, and the offer after a crash.
#![cfg(feature = "report")]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use hearken_app::report::{Answer, Report, Sender, Session};
use hearken_app::screens::report::{BAD_ADDRESS, DESCRIBE_FIRST, SENT};
use hearken_app::screens::report_link::{NO_BROWSER, OPENED, PREPARING};
use hearken_app::screens::{ReportScreen, SettingsScreen};
use hearken_app::{Game, Headless, InputEvent, Key, NoAction, NoCategory, ReportConfig};

/// A game whose reports are written in its own screen.
struct Reporting;

/// A game whose reports are written on the relay's web page.
struct WebReporting;

struct Home;
impl<G: Game> hearken_app::State<G> for Home {
    fn name(&self) -> &'static str {
        "home"
    }
    fn allows_update_prompt(&self) -> bool {
        true
    }
    fn help(&self, _: &hearken_app::Ctx<G>) -> String {
        "Home".into()
    }
}

impl Game for Reporting {
    const TITLE: &'static str = "Reporting";
    const DATA_DIR: &'static str = "HearkenReportingTest";
    const ENV_PREFIX: &'static str = "HEARKEN_REPORTING_TEST";
    type Event = ();
    type Data = ();
    type Settings = ();
    type Action = NoAction;
    type Category = NoCategory;

    fn reports() -> Option<ReportConfig> {
        Some(
            ReportConfig::new(
                "Reporting",
                "1.2.0",
                "https://example.test/api/problem-reports",
            )?
            .commit("abc123")
            .in_game_form(),
        )
    }
}

thread_local! {
    /// Where WebReporting says its player is, for the test to set.
    static WHERE: std::cell::RefCell<Vec<(String, String)>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn set_where(details: &[(&str, &str)]) {
    WHERE.with(|w| {
        *w.borrow_mut() = details
            .iter()
            .map(|(n, v)| (n.to_string(), v.to_string()))
            .collect()
    });
}

impl Game for WebReporting {
    const TITLE: &'static str = "Reporting";
    const DATA_DIR: &'static str = "HearkenWebReportingTest";
    const ENV_PREFIX: &'static str = "HEARKEN_WEB_REPORTING_TEST";
    type Event = ();
    type Data = ();
    type Settings = ();
    type Action = NoAction;
    type Category = NoCategory;

    fn reports() -> Option<ReportConfig> {
        ReportConfig::new(
            "Reporting",
            "1.2.0",
            "https://example.test/api/problem-reports",
        )
    }

    fn report_details(_ctx: &hearken_app::Ctx<Self>) -> Vec<(String, String)> {
        WHERE.with(|w| w.borrow().clone())
    }
}

/// A relay that answers from a table and keeps what was posted.
#[derive(Clone)]
struct Relay {
    status: u16,
    posted: Arc<Mutex<Vec<(String, Report)>>>,
}

impl Relay {
    fn answering(status: u16) -> Relay {
        Relay {
            status,
            posted: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn reports(&self) -> Vec<Report> {
        self.posted
            .lock()
            .unwrap()
            .iter()
            .map(|(_, r)| r.clone())
            .collect()
    }
}

impl Sender for Relay {
    fn post(&self, endpoint: &str, json: &[u8]) -> Result<Answer, String> {
        let report: Report = serde_json::from_slice(json).map_err(|e| e.to_string())?;
        self.posted
            .lock()
            .unwrap()
            .push((endpoint.to_string(), report));
        Ok(Answer {
            status: self.status,
            body: if self.status == 403 {
                "unknown app".into()
            } else if endpoint.ends_with("/open") && self.status < 300 {
                r#"{"code":"abc","url":"https://example.test/report/abc"}"#.into()
            } else {
                String::new()
            },
        })
    }
}

/// The report screen over a home screen, with the session log written so
/// there is something to attach.
fn screen(relay: &Relay) -> Headless<Reporting> {
    let mut h = Headless::<Reporting>::new((), Home);
    h.app.ctx.reports.set_sender(Box::new(relay.clone()));
    let log = h.app.ctx.paths.log_file();
    std::fs::write(
        &log,
        "12:00:01 INFO started\n12:00:02 ERROR something went wrong\n",
    )
    .unwrap();
    let marker = h.app.ctx.paths.data_dir.join("session.running");
    h.app
        .ctx
        .reports
        .adopt_session(Session::begin(&marker), vec![log]);
    h.app.ctx.push(ReportScreen::<Reporting>::new());
    h.frame(0.0);
    h
}

fn type_text(h: &mut Headless<Reporting>, text: &str) {
    for ch in text.chars() {
        h.send(InputEvent::typed(ch));
    }
}

/// Fill in What happened through its field.
fn describe(h: &mut Headless<Reporting>, text: &str) {
    h.press(Key::Home);
    h.press(Key::Return);
    type_text(h, text);
    h.press(Key::Return);
}

/// Move to the Send row and choose it. Only what Send says is kept.
fn send(h: &mut Headless<Reporting>) {
    h.press(Key::End);
    h.clear();
    h.press(Key::Return);
}

/// Run frames until `needle` is spoken, or give up.
fn until_spoken<G: Game>(h: &mut Headless<G>, needle: &str) -> Vec<String> {
    for _ in 0..500 {
        h.frame(0.016);
        let spoken = h.spoken();
        if spoken.iter().any(|l| l.contains(needle)) {
            return spoken;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("{needle:?} was never spoken; heard {:?}", h.spoken());
}

#[test]
fn settings_lists_report_a_problem_for_a_game_that_takes_reports() {
    let mut h = Headless::<Reporting>::new((), Home);
    h.app.ctx.push(SettingsScreen::<Reporting>::new());
    h.frame(0.0);
    let lines = h.app.top_lines();
    assert!(
        lines.iter().any(|l| l.contains("Report a problem")),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.contains("Ask to report a problem after a crash: on")),
        "{lines:?}"
    );
}

#[test]
fn the_screen_lists_its_rows_and_refuses_to_send_nothing() {
    let relay = Relay::answering(202);
    let mut h = screen(&relay);
    assert_eq!(
        h.spoken(),
        [
            "Report a problem.",
            "What happened: nothing written yet. 1 of 5."
        ]
    );
    let lines = h.app.top_lines();
    assert_eq!(
        lines,
        [
            "Report a problem",
            "> What happened: nothing written yet",
            "  Your email address, for a reply: none",
            "  Include the game log: on",
            "  Read the report",
            "  Send",
        ]
    );
    h.press(Key::End);
    h.clear();
    h.press(Key::Return);
    assert_eq!(
        h.spoken(),
        [
            DESCRIBE_FIRST,
            "What happened: nothing written yet. 1 of 5."
        ]
    );
    assert!(relay.reports().is_empty());
}

#[test]
fn a_report_carries_the_description_the_game_and_the_log() {
    let relay = Relay::answering(202);
    let mut h = screen(&relay);
    describe(&mut h, "The dock menu is empty.");
    assert_eq!(h.phase(), Some("writing"));
    send(&mut h);
    assert_eq!(h.spoken(), ["Sending the report."]);
    assert_eq!(h.phase(), Some("sending"));
    let spoken = until_spoken(&mut h, SENT);
    assert_eq!(spoken.last().map(String::as_str), Some(SENT));
    assert_eq!(h.app.top_name(), "home");
    let reports = relay.reports();
    assert_eq!(reports.len(), 1);
    let report = &reports[0];
    assert_eq!(report.app, "Reporting");
    assert_eq!(report.version, "1.2.0");
    assert_eq!(report.commit.as_deref(), Some("abc123"));
    assert_eq!(report.description, "The dock menu is empty.");
    assert!(!report.crashed);
    assert_eq!(report.reply_to, None);
    assert_eq!(report.attachments.len(), 1);
    assert_eq!(report.attachments[0].name, "session.log");
    assert!(report.attachments[0].text.contains("something went wrong"));
}

#[test]
fn the_log_can_be_left_out_and_the_report_read_first() {
    let relay = Relay::answering(202);
    let mut h = screen(&relay);
    describe(&mut h, "It happened at a warehouse.");
    h.press(Key::Down);
    h.press(Key::Down);
    h.clear();
    h.press(Key::Right);
    assert_eq!(h.spoken(), ["Include the game log: off. 3 of 5."]);
    h.press(Key::Down);
    h.clear();
    h.press(Key::Return);
    let spoken = h.spoken();
    assert_eq!(spoken[0], "The report.");
    assert!(
        spoken[1].starts_with("[Reporting 1.2.0] It happened at a warehouse."),
        "{spoken:?}"
    );
    let lines = h.app.top_lines();
    assert!(
        lines
            .iter()
            .any(|l| l.contains("App: Reporting 1.2.0 (commit abc123)")),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.contains("No log attached.")),
        "{lines:?}"
    );
    h.press(Key::Escape);
    send(&mut h);
    until_spoken(&mut h, SENT);
    assert!(relay.reports()[0].attachments.is_empty());
}

#[test]
fn an_address_that_is_not_one_is_refused_in_the_field() {
    let relay = Relay::answering(202);
    let mut h = screen(&relay);
    h.press(Key::Down);
    h.press(Key::Return);
    type_text(&mut h, "driver at example");
    h.clear();
    h.press(Key::Return);
    assert_eq!(
        h.spoken(),
        ["That does not look like an email address. Check it, or leave it blank."]
    );
    assert_eq!(h.app.top_name(), "text_entry");
    h.press(Key::Escape);
    let lines = h.app.top_lines();
    assert!(
        lines
            .iter()
            .any(|l| l.contains("Your email address, for a reply: none")),
        "{lines:?}"
    );
}

#[test]
fn an_address_whose_domain_takes_no_mail_goes_back_to_the_address_row() {
    let relay = Relay::answering(422);
    let mut h = screen(&relay);
    describe(&mut h, "It broke.");
    h.press(Key::Down);
    h.press(Key::Return);
    type_text(&mut h, "driver@exmaple.com");
    h.press(Key::Return);
    send(&mut h);
    h.clear();
    let spoken = until_spoken(&mut h, BAD_ADDRESS);
    assert_eq!(
        spoken,
        [
            BAD_ADDRESS,
            "Your email address, for a reply: driver@exmaple.com. 2 of 5."
        ]
    );
    assert_eq!(
        relay.reports()[0].reply_to.as_deref(),
        Some("driver@exmaple.com")
    );
}

#[test]
fn a_report_the_relay_will_not_take_can_be_saved_to_a_file() {
    let relay = Relay::answering(429);
    let mut h = screen(&relay);
    describe(&mut h, "It broke.");
    send(&mut h);
    h.clear();
    let spoken = until_spoken(&mut h, "Save the report to a file instead");
    assert_eq!(
        spoken[0],
        "Too many reports have been sent today. Save the report to a file instead, to send another way?"
    );
    assert_eq!(h.app.top_name(), "confirm");
    h.clear();
    h.press(Key::Return);
    let saved = h.app.ctx.paths.data_dir.join("problem-report.txt");
    assert!(saved.is_file());
    let text = std::fs::read_to_string(&saved).unwrap();
    assert!(text.starts_with("[Reporting 1.2.0] It broke."), "{text}");
    assert!(text.contains("--- session.log ---"), "{text}");
    let spoken = h.spoken();
    assert_eq!(
        spoken[0],
        format!("The report was saved as {}.", saved.display())
    );
    assert_eq!(h.app.top_name(), "home");
}

#[test]
fn a_refusal_names_the_reason_and_not_now_keeps_the_screen() {
    let relay = Relay::answering(403);
    let mut h = screen(&relay);
    describe(&mut h, "It broke.");
    send(&mut h);
    h.clear();
    let spoken = until_spoken(&mut h, "unknown app");
    assert!(
        spoken[0].starts_with("The report could not be sent: unknown app."),
        "{spoken:?}"
    );
    h.press(Key::Escape);
    assert_eq!(h.app.top_name(), "report");
    assert!(!h.app.ctx.paths.data_dir.join("problem-report.txt").exists());
}

/// A home screen after a run that never exited cleanly.
fn after_crash() -> Headless<Reporting> {
    let mut h = Headless::<Reporting>::new((), Home);
    let marker = h.app.ctx.paths.data_dir.join("session.running");
    drop(Session::begin(&marker));
    h.app
        .ctx
        .reports
        .adopt_session(Session::begin(&marker), Vec::new());
    h
}

#[test]
fn a_crash_is_offered_for_report_once_on_the_home_screen() {
    let mut h = after_crash();
    h.frame(0.0);
    h.frame(0.0);
    assert_eq!(h.app.top_name(), "confirm");
    let spoken = h.spoken();
    assert!(
        spoken.iter().any(|l| l == "Reporting closed unexpectedly the last time it ran. Report the problem?"),
        "{spoken:?}"
    );
    h.clear();
    h.press(Key::Return);
    assert_eq!(h.app.top_name(), "report");
    assert_eq!(
        h.spoken()[0],
        "Report a problem. Reporting closed unexpectedly the last time it ran. Say what you were doing when it happened."
    );
    h.press(Key::Escape);
    for _ in 0..3 {
        h.frame(0.0);
    }
    assert_eq!(h.app.top_name(), "home");
}

#[test]
fn a_crash_report_says_it_crashed() {
    let relay = Relay::answering(202);
    let mut h = after_crash();
    h.app.ctx.reports.set_sender(Box::new(relay.clone()));
    h.frame(0.0);
    h.frame(0.0);
    h.press(Key::Return);
    describe(&mut h, "I was opening the map.");
    send(&mut h);
    until_spoken(&mut h, SENT);
    let report = &relay.reports()[0];
    assert!(report.crashed);
    assert_eq!(
        report.subject(),
        "[Reporting 1.2.0] crash: I was opening the map."
    );
}

#[test]
fn dont_ask_again_turns_the_offer_off_in_settings() {
    let mut h = after_crash();
    h.frame(0.0);
    h.frame(0.0);
    assert_eq!(h.app.top_name(), "confirm");
    h.press(Key::End);
    h.press(Key::Return);
    h.frame(0.0);
    assert_eq!(h.app.top_name(), "home");
    assert!(!h.app.ctx.settings.reports.ask_after_crash);

    let mut again = after_crash();
    again.app.ctx.settings.reports.ask_after_crash = false;
    again.frame(0.0);
    again.frame(0.0);
    assert_eq!(again.app.top_name(), "home");
}

#[test]
fn a_clean_exit_is_not_a_crash_next_time() {
    let mut h = Headless::<Reporting>::new((), Home);
    let marker = h.app.ctx.paths.data_dir.join("session.running");
    h.app
        .ctx
        .reports
        .adopt_session(Session::begin(&marker), Vec::new());
    assert!(!h.app.ctx.reports.crashed_last_run());
    h.app.ctx.reports.end_session();
    assert!(!Session::begin(&marker).ended_unexpectedly());
}

/// A browser that records what it was asked to open, and opens it or not.
fn opener(opens: bool) -> (hearken_app::reports::Opener, Arc<Mutex<Vec<String>>>) {
    let urls = Arc::new(Mutex::new(Vec::new()));
    let seen = urls.clone();
    (
        Box::new(move |url: &str| {
            seen.lock().unwrap().push(url.to_string());
            opens
        }),
        urls,
    )
}

/// Settings for a game that reports on the web, with a log to send ahead.
fn web_settings(relay: &Relay, opens: bool) -> (Headless<WebReporting>, Arc<Mutex<Vec<String>>>) {
    let mut h = Headless::<WebReporting>::new((), Home);
    h.app.ctx.reports.set_sender(Box::new(relay.clone()));
    let (open, urls) = opener(opens);
    h.app.ctx.reports.set_opener(open);
    let log = h.app.ctx.paths.log_file();
    std::fs::write(&log, "12:00:01 INFO started\n").unwrap();
    let marker = h.app.ctx.paths.data_dir.join("session.running");
    h.app
        .ctx
        .reports
        .adopt_session(Session::begin(&marker), vec![log]);
    h.app.ctx.push(SettingsScreen::<WebReporting>::new());
    h.frame(0.0);
    (h, urls)
}

/// Choose Report a problem in Settings. Only what it says is kept.
fn choose_report(h: &mut Headless<WebReporting>) {
    while !h
        .app
        .top_lines()
        .iter()
        .any(|l| l.starts_with("> Report a problem"))
    {
        h.press(Key::Down);
    }
    h.clear();
    h.press(Key::Return);
}

#[test]
fn a_web_form_report_sends_the_log_ahead_and_opens_the_page() {
    let relay = Relay::answering(201);
    let (mut h, urls) = web_settings(&relay, true);
    choose_report(&mut h);
    assert_eq!(h.app.top_name(), "report_link");
    assert_eq!(h.phase(), Some("preparing"));
    assert_eq!(h.spoken(), [PREPARING]);
    let spoken = until_spoken(&mut h, OPENED);
    assert_eq!(spoken[0], PREPARING);
    assert!(spoken.iter().any(|l| l == OPENED), "{spoken:?}");
    assert_eq!(h.app.top_name(), "settings");
    let posted = relay.posted.lock().unwrap();
    assert_eq!(posted.len(), 1);
    assert_eq!(posted[0].0, "https://example.test/api/problem-reports/open");
    let report = &posted[0].1;
    assert_eq!(report.description, "");
    assert_eq!(report.app, "Reporting");
    assert_eq!(report.attachments.len(), 1);
    assert_eq!(report.attachments[0].name, "session.log");
    assert_eq!(
        *urls.lock().unwrap(),
        ["https://example.test/report/abc".to_string()]
    );
}

#[test]
fn a_browser_that_will_not_open_falls_back_to_the_games_own_screen() {
    let relay = Relay::answering(201);
    let (mut h, urls) = web_settings(&relay, false);
    choose_report(&mut h);
    let spoken = until_spoken(&mut h, NO_BROWSER);
    assert_eq!(h.app.top_name(), "report");
    assert!(
        spoken.contains(&format!("{NO_BROWSER} Report a problem.")),
        "{spoken:?}"
    );
    assert_eq!(urls.lock().unwrap().len(), 1);
}

#[test]
fn a_relay_that_cannot_keep_the_log_falls_back_with_the_reason() {
    let relay = Relay::answering(503);
    let (mut h, urls) = web_settings(&relay, true);
    choose_report(&mut h);
    let spoken = until_spoken(&mut h, "Write the report here");
    assert_eq!(h.app.top_name(), "report");
    assert!(
        spoken.iter().any(|l| l == "The report form could not be opened: the relay answered 503. Write the report here. Report a problem."),
        "{spoken:?}"
    );
    assert!(urls.lock().unwrap().is_empty());
}

#[test]
fn escape_leaves_while_the_log_is_on_its_way() {
    let relay = Relay::answering(201);
    let (mut h, _urls) = web_settings(&relay, true);
    choose_report(&mut h);
    h.press(Key::Escape);
    assert_eq!(h.app.top_name(), "settings");
}

#[test]
fn a_crash_offer_opens_the_web_form_marked_as_a_crash() {
    let relay = Relay::answering(201);
    let mut h = Headless::<WebReporting>::new((), Home);
    h.app.ctx.reports.set_sender(Box::new(relay.clone()));
    let (open, urls) = opener(true);
    h.app.ctx.reports.set_opener(open);
    let marker = h.app.ctx.paths.data_dir.join("session.running");
    drop(Session::begin(&marker));
    h.app
        .ctx
        .reports
        .adopt_session(Session::begin(&marker), Vec::new());
    h.frame(0.0);
    h.frame(0.0);
    assert_eq!(h.app.top_name(), "confirm");
    h.press(Key::Return);
    assert_eq!(h.app.top_name(), "report_link");
    until_spoken(&mut h, OPENED);
    assert_eq!(h.app.top_name(), "home");
    assert!(relay.posted.lock().unwrap()[0].1.crashed);
    assert_eq!(urls.lock().unwrap().len(), 1);
}

#[test]
fn a_report_names_where_the_player_was() {
    set_where(&[("Game", "Tightrope"), ("Mode", "challenge")]);
    let relay = Relay::answering(201);
    let (mut h, _urls) = web_settings(&relay, true);
    choose_report(&mut h);
    until_spoken(&mut h, OPENED);
    set_where(&[]);
    let report = &relay.reports()[0];
    assert_eq!(
        report
            .details
            .iter()
            .map(|d| (d.name.as_str(), d.value.as_str()))
            .collect::<Vec<_>>(),
        [("Game", "Tightrope"), ("Mode", "challenge")]
    );
    assert!(!report.crashed);
    assert_eq!(report.subject(), "[Reporting 1.2.0, Tightrope] ");
}

#[test]
fn where_the_player_is_goes_in_the_session_marker_as_it_changes() {
    set_where(&[("Game", "Belfry")]);
    let mut h = Headless::<WebReporting>::new((), Home);
    let marker = h.app.ctx.paths.data_dir.join("session.running");
    h.app
        .ctx
        .reports
        .adopt_session(Session::begin(&marker), Vec::new());
    h.frame(0.0);
    assert_eq!(
        std::fs::read_to_string(&marker).unwrap(),
        "running\nGame\tBelfry"
    );
    set_where(&[("Game", "Belfry"), ("Round", "3")]);
    h.frame(0.0);
    assert_eq!(
        std::fs::read_to_string(&marker).unwrap(),
        "running\nGame\tBelfry\nRound\t3"
    );
    set_where(&[]);
    h.frame(0.0);
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "running");
}

#[test]
fn a_crash_report_names_where_the_last_run_was_not_where_this_one_is() {
    let relay = Relay::answering(201);
    let mut h = Headless::<WebReporting>::new((), Home);
    h.app.ctx.reports.set_sender(Box::new(relay.clone()));
    let (open, _urls) = opener(true);
    h.app.ctx.reports.set_opener(open);
    let marker = h.app.ctx.paths.data_dir.join("session.running");
    // The last run was in Belfry when it died.
    let mut last = Session::begin(&marker);
    last.note("Game\tBelfry");
    drop(last);
    // This run is in the lobby.
    set_where(&[("Screen", "lobby")]);
    h.app
        .ctx
        .reports
        .adopt_session(Session::begin(&marker), Vec::new());
    h.frame(0.0);
    h.frame(0.0);
    assert_eq!(h.app.top_name(), "confirm");
    h.press(Key::Return);
    until_spoken(&mut h, OPENED);
    set_where(&[]);
    let report = &relay.reports()[0];
    assert!(report.crashed);
    assert_eq!(report.details.len(), 1);
    assert_eq!(report.details[0].name, "Game");
    assert_eq!(report.details[0].value, "Belfry");
    assert_eq!(report.subject(), "[Reporting 1.2.0, Belfry] crash: ");
}
