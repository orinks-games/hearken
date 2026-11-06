//! Connecting an online account from Settings, against a server that
//! answers from a script.
#![cfg(feature = "online")]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hearken_app::online::{Body, OnlineConfig, Reply, Secret, Transport};
use hearken_app::screens::SettingsScreen;
use hearken_app::{Game, Headless, Key, NoAction, NoCategory};
use serde_json::{json, Value};

struct Linked;

impl Game for Linked {
    const TITLE: &'static str = "Linked";
    const DATA_DIR: &'static str = "HearkenLinkedTest";
    const ENV_PREFIX: &'static str = "HEARKEN_LINKED_TEST";
    type Event = ();
    type Data = ();
    type Settings = ();
    type Action = NoAction;
    type Category = NoCategory;

    fn online() -> Option<OnlineConfig> {
        Some(OnlineConfig::new(
            "https://example.test/api/games/linked",
            "example.test",
        ))
    }
}

struct Script(Mutex<VecDeque<(u16, Value)>>);

impl Transport for Script {
    fn post(&self, _url: &str, _token: Option<&str>, _body: Body) -> Result<Reply, String> {
        let (status, body) = self.0.lock().unwrap().pop_front().ok_or("no reply")?;
        Ok(Reply { status, body })
    }
}

fn frames_until(h: &mut Headless<Linked>, line: &str) {
    for _ in 0..600 {
        h.frame(0.016);
        if h.spoken().iter().any(|l| l.contains(line)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("never heard {line:?}: {:?}", h.spoken());
}

#[test]
fn a_player_connects_and_disconnects_an_account() {
    let mut h = Headless::<Linked>::new((), SettingsScreen::<Linked>::new());
    h.app.ctx.online.set_transport(Arc::new(Script(Mutex::new(
        vec![
            (
                200,
                json!({ "device_code": "d".repeat(64), "user_code": "ABCD-EF34",
                        "verify_url": "https://example.test/activate", "interval": 2 }),
            ),
            (
                200,
                json!({ "status": "ready", "token": "k".repeat(40), "name": "Ada" }),
            ),
        ]
        .into(),
    ))));
    // The account row is last; End goes there.
    h.press(Key::End);
    assert!(
        h.spoken()
            .iter()
            .any(|l| l.starts_with("Online account: not connected")),
        "{:?}",
        h.spoken()
    );
    h.clear();
    h.press(Key::Return);
    assert_eq!(h.spoken(), ["Asking example.test for a code."]);
    frames_until(
        &mut h,
        "Code A, B, C, D, then E, F, 3, 4. Go to example.test slash activate and enter it.",
    );
    frames_until(&mut h, "Connected to example.test as Ada.");
    assert_eq!(
        h.app.ctx.settings.online.token,
        Some(Secret("k".repeat(40)))
    );
    let saved = std::fs::read_to_string(h.app.ctx.paths.settings_file()).unwrap();
    assert!(
        saved.contains(&"k".repeat(40)),
        "the account is saved at once"
    );

    h.clear();
    h.press(Key::Return);
    assert_eq!(
        h.spoken(),
        ["Connected to example.test as Ada. Enter disconnects this computer, Escape keeps it."]
    );
    h.clear();
    h.press(Key::Return);
    h.frame(0.016);
    assert!(
        h.spoken()[0].starts_with("Disconnected from example.test."),
        "{:?}",
        h.spoken()
    );
    assert!(!h.app.ctx.online.is_connected());
    assert_eq!(h.app.ctx.settings.online.token, None);
}

#[test]
fn an_expired_code_offers_a_new_one() {
    let mut h = Headless::<Linked>::new((), SettingsScreen::<Linked>::new());
    h.app.ctx.online.set_transport(Arc::new(Script(Mutex::new(
        vec![
            (
                200,
                json!({ "device_code": "d".repeat(64), "user_code": "ABCD-EF34", "interval": 2 }),
            ),
            (410, json!({ "status": "expired" })),
        ]
        .into(),
    ))));
    h.press(Key::End);
    h.press(Key::Return);
    frames_until(&mut h, "Go to example.test and enter it.");
    frames_until(
        &mut h,
        "The code expired. Enter asks for a new one, Escape goes back.",
    );
}

struct Guests;

impl Game for Guests {
    const TITLE: &'static str = "Guests";
    const DATA_DIR: &'static str = "HearkenGuestsTest";
    const ENV_PREFIX: &'static str = "HEARKEN_GUESTS_TEST";
    type Event = ();
    type Data = ();
    type Settings = ();
    type Action = NoAction;
    type Category = NoCategory;

    fn online() -> Option<OnlineConfig> {
        Some(
            OnlineConfig::new("https://example.test/api/games/guests", "example.test")
                .with_guests(),
        )
    }
}

/// Each request's URL and token.
type Sent = Arc<Mutex<Vec<(String, Option<String>)>>>;

/// Answers from a script, and remembers each request's URL and token.
struct Recorded {
    replies: Mutex<VecDeque<(u16, Value)>>,
    sent: Sent,
}

impl Transport for Recorded {
    fn post(&self, url: &str, token: Option<&str>, _body: Body) -> Result<Reply, String> {
        self.sent
            .lock()
            .unwrap()
            .push((url.to_string(), token.map(String::from)));
        let (status, body) = self.replies.lock().unwrap().pop_front().ok_or("no reply")?;
        Ok(Reply { status, body })
    }
}

fn guest_frames_until(h: &mut Headless<Guests>, line: &str) {
    for _ in 0..600 {
        h.frame(0.016);
        if h.spoken().iter().any(|l| l.contains(line)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("never heard {line:?}: {:?}", h.spoken());
}

#[test]
fn a_guest_plays_renames_connects_an_account_and_stops() {
    let mut h = Headless::<Guests>::new((), SettingsScreen::<Guests>::new());
    let sent = Arc::new(Mutex::new(Vec::new()));
    h.app.ctx.online.set_transport(Arc::new(Recorded {
        replies: Mutex::new(
            vec![
                (200, json!({ "status": "ready", "token": "g".repeat(40), "name": "Copper Fox 12", "guest": true })),
                (200, json!({ "ok": true, "name": "Sunny Pine 3" })),
                (
                    200,
                    json!({ "device_code": "d".repeat(64), "user_code": "ABCD-EF34",
                            "verify_url": "https://example.test/activate", "interval": 2 }),
                ),
                (200, json!({ "status": "ready", "token": "k".repeat(40), "name": "Ada" })),
            ]
            .into(),
        ),
        sent: sent.clone(),
    }));
    h.press(Key::End);
    h.clear();
    h.press(Key::Return);
    assert_eq!(h.spoken(), ["Not playing online. Play as a guest, 1 of 2"]);
    h.clear();
    h.press(Key::Return);
    guest_frames_until(&mut h, "Playing as guest Copper Fox 12.");
    assert!(h.app.ctx.online.is_guest());
    assert!(h.app.ctx.settings.online.guest, "a guest is saved as one");

    // Back on the row, the screen offers a new name first.
    h.clear();
    h.press(Key::Return);
    assert_eq!(
        h.spoken(),
        ["Playing as guest Copper Fox 12. New guest name, 1 of 3"]
    );
    h.clear();
    h.press(Key::Return);
    guest_frames_until(&mut h, "You are now Sunny Pine 3.");
    assert_eq!(h.app.ctx.online.player_name(), Some("Sunny Pine 3"));

    // Connecting an account sends the guest's token, so its scores follow.
    h.press(Key::Down);
    h.clear();
    h.press(Key::Return);
    guest_frames_until(&mut h, "Code A, B, C, D, then E, F, 3, 4.");
    guest_frames_until(&mut h, "Connected to example.test as Ada.");
    let sent = sent.lock().unwrap();
    let start = sent
        .iter()
        .find(|(url, _)| url.ends_with("activate/start"))
        .unwrap();
    assert_eq!(start.1.as_deref(), Some("g".repeat(40).as_str()));
    assert!(!h.app.ctx.online.is_guest());
    assert!(!h.app.ctx.settings.online.guest);
}

#[test]
fn a_guest_can_stop_playing_online() {
    let mut h = Headless::<Guests>::new((), SettingsScreen::<Guests>::new());
    h.app.ctx.online.set_transport(Arc::new(Script(Mutex::new(
        vec![(
            200,
            json!({ "status": "ready", "token": "g".repeat(40), "name": "Copper Fox 12" }),
        )]
        .into(),
    ))));
    h.press(Key::End);
    h.press(Key::Return);
    h.press(Key::Return);
    guest_frames_until(&mut h, "Playing as guest Copper Fox 12.");
    h.press(Key::Return);
    h.press(Key::Up);
    h.clear();
    h.press(Key::Return);
    h.frame(0.016);
    assert!(
        h.spoken()[0].starts_with("Stopped playing online on this computer."),
        "{:?}",
        h.spoken()
    );
    assert!(!h.app.ctx.online.is_connected());
    assert!(!h.app.ctx.settings.online.guest);
}
