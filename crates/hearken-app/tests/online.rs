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
