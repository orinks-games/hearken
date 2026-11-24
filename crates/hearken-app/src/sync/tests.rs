use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::testing::FakeSaves;
use super::*;
use crate::online::{OnlineConfig, OnlineSettings, Secret};
use hearken_seal::Key;

const URL: &str = "https://example.test/api/games/g";

/// One computer: its account connection, its data folder, and a toy save
/// with a best score and a set of unlocks.
struct Computer {
    online: Online,
    vault: Vault,
    _dir: tempfile::TempDir,
    save: Value,
    sync: FileSync,
    now: f64,
    events: Vec<SyncEvent>,
    /// The base each merge came with.
    bases: Vec<Option<Value>>,
}

fn settings(token: &str) -> OnlineSettings {
    OnlineSettings {
        token: Some(Secret(token.into())),
        name: Some("Ada".into()),
        guest: false,
    }
}

fn computer(server: &Arc<FakeSaves>, key: &Key, token: Option<&str>) -> Computer {
    let saved = token.map(settings).unwrap_or_default();
    let mut online = Online::new(Some(OnlineConfig::new(URL, "example.test")), &saved);
    online.set_transport(server.clone());
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::with_key(dir.path(), Some(key.clone()));
    Computer {
        online,
        vault,
        _dir: dir,
        save: json!({"best": 0, "unlocks": []}),
        sync: FileSync::new(SyncConfig::new("records", "records.json", 1)),
        now: 0.0,
        events: Vec::new(),
        bases: Vec::new(),
    }
}

fn merge(local: &mut Value, cloud: &Value) -> bool {
    let before = local.clone();
    let best = local["best"]
        .as_u64()
        .unwrap()
        .max(cloud["best"].as_u64().unwrap());
    let mut unlocks: Vec<String> = local["unlocks"]
        .as_array()
        .unwrap()
        .iter()
        .chain(cloud["unlocks"].as_array().unwrap())
        .map(|u| u.as_str().unwrap().to_string())
        .collect();
    unlocks.sort();
    unlocks.dedup();
    *local = json!({"best": best, "unlocks": unlocks});
    *local != before
}

impl Computer {
    fn play(&mut self, best: u64, unlock: &str) {
        let b = self.save["best"].as_u64().unwrap().max(best);
        self.save["best"] = json!(b);
        if !unlock.is_empty() {
            let mut u: Vec<Value> = self.save["unlocks"].as_array().unwrap().clone();
            u.push(json!(unlock));
            self.save["unlocks"] = Value::Array(u);
        }
        self.sync.touch();
    }

    fn progress(&self) -> bool {
        self.save["best"] != json!(0) || !self.save["unlocks"].as_array().unwrap().is_empty()
    }

    /// Steps the sync until it settles: no request on its way and the same
    /// state for a while.
    fn settle(&mut self) -> String {
        let until = Instant::now() + Duration::from_secs(5);
        let mut quiet = 0;
        while Instant::now() < until {
            self.online.update();
            let before = self.sync.state().clone();
            let need = self
                .sync
                .step(&mut self.online, &mut self.vault, self.now, true);
            match need {
                Some(Need::Export) => {
                    let bytes = serde_json::to_vec(&self.save).unwrap();
                    let progress = self.progress();
                    let card = json!({"best": self.save["best"]});
                    self.sync.exported(
                        &mut self.online,
                        &mut self.vault,
                        &bytes,
                        progress,
                        Some(card),
                        self.now,
                    );
                    quiet = 0;
                }
                Some(Need::Merge { theirs, base }) => {
                    let cloud: Value = serde_json::from_slice(&theirs).unwrap();
                    self.bases
                        .push(base.map(|b| serde_json::from_slice(&b).unwrap()));
                    let changed = merge(&mut self.save, &cloud);
                    self.sync.merged(&mut self.vault, changed);
                    quiet = 0;
                }
                None if *self.sync.state() == before && self.sync.ticket.is_none() => {
                    quiet += 1;
                    if quiet > 5 {
                        break;
                    }
                }
                None => quiet = 0,
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        self.events.extend(self.sync.take_events());
        self.sync.state().name()
    }

    /// Time passes, past the upload gap and the recheck.
    fn later(&mut self, seconds: f64) {
        self.now += seconds;
    }
}

fn setup() -> (Arc<FakeSaves>, Key) {
    let server = FakeSaves::new();
    server.token("token-for-ada-on-one-0000", "ada", false);
    server.token("token-for-ada-on-two-0000", "ada", false);
    server.token("token-for-bea-on-two-0000", "bea", false);
    server.token("token-for-guest-00000000", "g1", true);
    (server, Key::generate().unwrap())
}

const ONE: &str = "token-for-ada-on-one-0000";
const TWO: &str = "token-for-ada-on-two-0000";

#[test]
fn progress_goes_up_and_comes_down_on_another_computer() {
    let (server, key) = setup();
    let mut one = computer(&server, &key, Some(ONE));
    assert_eq!(one.settle(), "up-to-date", "nothing to send yet");
    assert!(server.kept("ada", "records").is_empty());

    one.play(40, "diary-1");
    one.later(31.0);
    assert_eq!(one.settle(), "up-to-date");
    let kept = server.kept("ada", "records");
    assert_eq!(kept.len(), 1);
    assert!(kept[0].content.starts_with(b"HKSL"), "the copy is sealed");
    assert_eq!(kept[0].card, Some(json!({"best": 40})));
    assert_eq!(one.events, [SyncEvent::FirstSaved]);

    // A fresh computer takes it without asking.
    let mut two = computer(&server, &key, Some(TWO));
    assert_eq!(two.settle(), "up-to-date");
    assert_eq!(two.save, json!({"best": 40, "unlocks": ["diary-1"]}));
    assert_eq!(two.events, [SyncEvent::Added]);
    assert_eq!(
        server.kept("ada", "records").len(),
        1,
        "nothing new to send back"
    );
}

#[test]
fn two_computers_that_played_apart_end_with_both() {
    let (server, key) = setup();
    let mut one = computer(&server, &key, Some(ONE));
    one.play(40, "diary-1");
    one.settle();
    let mut two = computer(&server, &key, Some(TWO));
    two.settle();

    // Both play without hearing from each other.
    one.play(55, "");
    two.play(10, "diary-2");
    one.later(31.0);
    two.later(31.0);
    assert_eq!(one.settle(), "up-to-date");
    // Two's upload is stale; it fetches one's, merges, and sends both.
    assert_eq!(two.settle(), "up-to-date");
    assert_eq!(
        two.save,
        json!({"best": 55, "unlocks": ["diary-1", "diary-2"]})
    );
    // One finds the merged copy at its next look.
    one.later(601.0);
    assert_eq!(one.settle(), "up-to-date");
    assert_eq!(one.save, two.save);
    assert_eq!(server.kept("ada", "records").len(), 3);
}

#[test]
fn a_merge_comes_with_the_copy_both_sides_last_agreed_on() {
    let (server, key) = setup();
    let mut one = computer(&server, &key, Some(ONE));
    one.play(40, "diary-1");
    one.settle();
    let mut two = computer(&server, &key, Some(TWO));
    two.settle();
    // Meeting the account, two has nothing in common with it yet.
    assert_eq!(two.bases, vec![None]);

    one.play(55, "");
    two.play(10, "diary-2");
    one.later(31.0);
    two.later(31.0);
    one.settle();
    two.settle();
    // Two's stale upload brings one's copy, with the copy both last had.
    let agreed = json!({"best": 40, "unlocks": ["diary-1"]});
    assert_eq!(two.bases.last(), Some(&Some(agreed)));
    // One's next merge starts from what two sent up.
    one.later(601.0);
    one.settle();
    assert_eq!(
        one.bases.last(),
        Some(&Some(json!({"best": 55, "unlocks": ["diary-1"]})))
    );
}

#[test]
fn a_computer_with_its_own_progress_asks_before_combining() {
    let (server, key) = setup();
    let mut one = computer(&server, &key, Some(ONE));
    one.play(40, "diary-1");
    one.settle();

    let mut two = computer(&server, &key, Some(TWO));
    two.play(70, "");
    assert_eq!(two.settle(), "needs-choice");
    assert_eq!(two.events, [SyncEvent::NeedsChoice]);
    assert_eq!(
        server.kept("ada", "records").len(),
        1,
        "nothing sent before the answer"
    );

    two.sync.choose(&mut two.vault, Choice::Combine);
    assert_eq!(two.settle(), "up-to-date");
    assert_eq!(two.save, json!({"best": 70, "unlocks": ["diary-1"]}));
    assert_eq!(server.kept("ada", "records").len(), 2);
}

#[test]
fn keeping_them_apart_turns_syncing_off_here_and_it_stays_off() {
    let (server, key) = setup();
    let mut one = computer(&server, &key, Some(ONE));
    one.play(40, "");
    one.settle();
    let mut two = computer(&server, &key, Some(TWO));
    two.play(70, "");
    assert_eq!(two.settle(), "needs-choice");
    two.sync.choose(&mut two.vault, Choice::KeepApart);
    assert_eq!(two.settle(), "off");
    assert_eq!(two.save["best"], json!(70));

    // The next start of the game remembers.
    two.sync = FileSync::new(SyncConfig::new("records", "records.json", 1));
    assert_eq!(two.settle(), "off");
    assert!(!two.sync.enabled(&mut two.vault));
    two.sync.set_enabled(&mut two.vault, true);
    assert_eq!(
        two.settle(),
        "needs-choice",
        "turned back on, it asks again"
    );
}

#[test]
fn another_account_on_a_synced_computer_is_asked_too() {
    let (server, key) = setup();
    let mut two = computer(&server, &key, Some(TWO));
    two.play(40, "");
    two.settle();
    assert_eq!(server.kept("ada", "records").len(), 1);
    // Bea already has progress of her own on her account.
    server.token("bea-elsewhere-000000000", "bea", false);
    let mut bea = computer(&server, &key, Some("bea-elsewhere-000000000"));
    bea.play(5, "");
    bea.settle();

    two.online.hold_account("token-for-bea-on-two-0000", "Bea");
    assert_eq!(
        two.settle(),
        "needs-choice",
        "Ada's progress is never folded into Bea's unasked"
    );
}

#[test]
fn guests_and_signed_out_players_do_not_sync() {
    let (server, key) = setup();
    let mut none = computer(&server, &key, None);
    none.play(5, "");
    assert_eq!(none.settle(), "needs-account");
    let mut guest = computer(&server, &key, None);
    guest.online = Online::new(
        Some(OnlineConfig::new(URL, "example.test")),
        &OnlineSettings {
            guest: true,
            ..settings("token-for-guest-00000000")
        },
    );
    guest.online.set_transport(server.clone());
    assert_eq!(guest.settle(), "needs-account");
    assert!(server.requests().is_empty(), "nothing asked of the server");
}

#[test]
fn a_token_the_server_forgot_signs_out_and_says_so() {
    let (server, key) = setup();
    let mut one = computer(&server, &key, Some("token-nobody-knows-00000"));
    assert_eq!(one.settle(), "needs-account");
    assert_eq!(one.events, vec![SyncEvent::SignedOut]);
    assert!(!one.online.is_connected());
}

#[test]
fn nothing_goes_up_until_there_is_progress() {
    let (server, key) = setup();
    let mut one = computer(&server, &key, Some(ONE));
    assert_eq!(one.settle(), "up-to-date");
    // A file that changed but holds no progress yet.
    one.save["seen"] = json!(true);
    one.sync.touch();
    one.later(31.0);
    assert_eq!(one.settle(), "up-to-date");
    assert!(server.kept("ada", "records").is_empty());
    assert!(one.events.is_empty());
    one.play(5, "");
    one.later(31.0);
    one.settle();
    assert_eq!(server.kept("ada", "records").len(), 1);
    assert_eq!(one.events, vec![SyncEvent::FirstSaved]);
}

#[test]
fn offline_waits_and_tries_again() {
    let (server, key) = setup();
    let mut one = computer(&server, &key, Some(ONE));
    server.set_offline(true);
    one.play(40, "");
    assert_eq!(one.settle(), "offline");
    server.set_offline(false);
    one.later(10.0);
    assert_eq!(one.settle(), "offline", "not before the retry");
    one.later(61.0);
    assert_eq!(one.settle(), "up-to-date");
    assert_eq!(server.kept("ada", "records").len(), 1);
}

#[test]
fn any_answer_from_the_server_ends_the_wait() {
    let (server, key) = setup();
    let mut one = computer(&server, &key, Some(ONE));
    server.set_offline(true);
    one.play(40, "");
    assert_eq!(one.settle(), "offline");
    server.set_offline(false);
    // Something else the game sends gets an answer, well before the retry.
    let ticket = one.online.save_head("other").unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    while one.online.result(ticket).is_none() && Instant::now() < until {
        one.online.update();
        std::thread::sleep(Duration::from_millis(5));
    }
    one.later(1.0);
    assert_eq!(one.settle(), "up-to-date");
    assert_eq!(server.kept("ada", "records").len(), 1);
}

#[test]
fn a_copy_from_a_newer_version_is_left_alone() {
    let (server, key) = setup();
    let mut one = computer(&server, &key, Some(ONE));
    one.sync = FileSync::new(SyncConfig::new("records", "records.json", 2));
    one.play(40, "");
    one.settle();
    let mut two = computer(&server, &key, Some(TWO));
    two.play(1, "");
    assert_eq!(two.settle(), "blocked-newer-format");
    assert_eq!(two.events, [SyncEvent::NewerFormat]);
    assert_eq!(two.save["best"], json!(1));
}

#[test]
fn a_copy_another_key_sealed_does_not_open() {
    let (server, key) = setup();
    let mut one = computer(&server, &key, Some(ONE));
    one.play(40, "");
    one.settle();
    let mut two = computer(&server, &Key::generate().unwrap(), Some(TWO));
    assert_eq!(two.settle(), "blocked-unreadable");
    assert_eq!(two.save["best"], json!(0));
}

#[test]
fn no_merge_until_the_game_says_it_is_safe() {
    let (server, key) = setup();
    let mut one = computer(&server, &key, Some(ONE));
    one.play(40, "");
    one.settle();
    let mut two = computer(&server, &key, Some(TWO));
    let until = Instant::now() + Duration::from_secs(3);
    while Instant::now() < until && *two.sync.state() != SyncState::Merging {
        two.online.update();
        if let Some(Need::Export) = two.sync.step(&mut two.online, &mut two.vault, 0.0, false) {
            two.sync
                .exported(&mut two.online, &mut two.vault, b"{}", false, None, 0.0);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(*two.sync.state(), SyncState::Merging);
    assert_eq!(
        two.sync.step(&mut two.online, &mut two.vault, 0.0, false),
        None
    );
    assert!(matches!(
        two.sync.step(&mut two.online, &mut two.vault, 0.0, true),
        Some(Need::Merge { .. })
    ));
}

#[test]
fn flush_sends_the_last_change_as_the_game_closes() {
    let (server, key) = setup();
    let mut one = computer(&server, &key, Some(ONE));
    one.play(40, "");
    one.settle();
    one.play(90, "");
    // Within the gap: nothing would go up yet.
    assert_eq!(one.settle(), "up-to-date");
    assert_eq!(server.kept("ada", "records").len(), 1);
    let bytes = serde_json::to_vec(&one.save).unwrap();
    one.sync.flush(
        &mut one.online,
        &mut one.vault,
        &bytes,
        true,
        None,
        Duration::from_secs(2),
    );
    assert_eq!(server.kept("ada", "records").len(), 2);
}

#[test]
fn base64_round_trips() {
    for n in 0..20 {
        let bytes: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
        assert_eq!(unbase64(&base64(&bytes)).unwrap(), bytes);
    }
    assert_eq!(base64(b"hearken"), "aGVhcmtlbg==");
    assert_eq!(unbase64("aGVhcmtlbg=="), Some(b"hearken".to_vec()));
    assert_eq!(unbase64("aGVh=mtl"), None);
    assert_eq!(unbase64("abc"), None);
    assert_eq!(unbase64("ab!c"), None);
}
