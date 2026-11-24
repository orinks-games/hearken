//! Two copies of the game play Pebbles through a real rooms server: one
//! creates a game and speaks its code, the other joins by that code, the
//! host starts, and the turns go round until the rules end. Runs only
//! against a dev server, named by `HELLO_HEARKEN_TEST_SERVER`:
//!
//! ```sh
//! cargo build -p hearken-replay --example pebbles --target wasm32-unknown-unknown --release
//! node server/bin/dev-server.js --rules target/wasm32-unknown-unknown/release/examples
//! HELLO_HEARKEN_TEST_SERVER=http://127.0.0.1:8787 cargo test -p hello-hearken --features online --test online
//! ```
#![cfg(feature = "online")]

use std::time::{Duration, Instant};

use hearken::rooms::Phase;
use hearken::{Headless, Key};
use hello_hearken::{HelloGame, Title};

fn server() -> Option<String> {
    std::env::var("HELLO_HEARKEN_TEST_SERVER")
        .ok()
        .filter(|s| !s.trim().is_empty())
}

/// Frames `h` until a spoken line contains `text`, or panics after 20 s.
fn hear(h: &mut Headless<HelloGame>, text: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(line) = h.spoken().iter().find(|l| l.contains(text)) {
            return line.clone();
        }
        assert!(
            Instant::now() < deadline,
            "never heard {text:?}; heard {:?}; screen {:?}",
            h.spoken(),
            h.app.top_lines()
        );
        h.frame(0.05);
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Frames both copies for `seconds`, so polls go out and answers land.
fn tick(a: &mut Headless<HelloGame>, b: &mut Headless<HelloGame>, seconds: f64) {
    let frames = (seconds / 0.05).ceil() as usize;
    for _ in 0..frames {
        a.frame(0.05);
        b.frame(0.05);
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A copy of the game at the title, playing online as a guest.
fn guest(url: &str) -> Headless<HelloGame> {
    // The test server is named through the developer-build environment
    // hook, as a developer would name it.
    std::env::set_var("HELLO_HEARKEN_ONLINE_URL", url);
    let mut h = Headless::<HelloGame>::new((), Title::new());
    h.press(Key::Down); // Play online
    h.press(Key::Return);
    assert_eq!(h.app.top_name(), "online_play");
    h.clear();
    h.press(Key::Return); // Play online: connect an account or play as a guest
    assert_eq!(h.app.top_name(), "online_account");
    h.press(Key::Return); // Play as a guest
    hear(&mut h, "Playing as guest");
    assert_eq!(h.app.top_name(), "online_play");
    h.clear();
    h
}

/// The code the room screen spoke, as "M 4 K, 7 T 8" -> "M4K7T8".
fn code_from(line: &str) -> String {
    let after = line.split("Invite code ").nth(1).unwrap_or("");
    after
        .chars()
        .take_while(|c| *c != '.')
        .filter(|c| c.is_ascii_alphanumeric())
        .collect()
}

#[test]
fn two_copies_play_a_match_by_invite_code() {
    let Some(url) = server() else {
        eprintln!("HELLO_HEARKEN_TEST_SERVER is not set: skipping the two-copy match");
        return;
    };
    let mut ada = guest(&url);
    let mut bo = guest(&url);

    // Ada creates a game and hears its code.
    ada.press(Key::Return); // Create a game
    let line = hear(&mut ada, "Invite code ");
    let code = code_from(&line);
    assert_eq!(code.len(), 6, "{line}");
    assert_eq!(ada.app.top_name(), "room");
    let lines = ada.app.top_lines();
    assert!(
        lines
            .iter()
            .any(|l| l.contains("(you), host. Waiting for 1 more.")),
        "{lines:?}"
    );

    // Bo joins by typing it; the room fills and starts by itself.
    bo.press(Key::Down); // Join with a code
    bo.press(Key::Return);
    assert_eq!(bo.app.top_name(), "text_entry");
    for c in code.chars() {
        bo.send(hearken::InputEvent::typed(c));
    }
    bo.press(Key::Return);
    // The last seat starts the match, so Bo arrives in a room already in
    // play and goes straight to the match.
    hear(&mut bo, "Pebbles with ");
    assert_eq!(bo.app.top_name(), "pebbles_match");
    hear(&mut ada, " joined.");
    hear(&mut ada, "The game starts.");
    assert_eq!(ada.app.top_name(), "pebbles_match");

    // Seat 0 (Ada) moves first. The copies take turns until the rules end:
    // whoever is to move takes 3, or what is left. 10 pebbles: 3, 3, 3, 1.
    let id = ada.app.ctx.rooms.rooms().next().unwrap().id().clone();
    let you_ada = ada.app.ctx.rooms.room(&id).unwrap().info.you();
    let you_bo = bo.app.ctx.rooms.room(&id).unwrap().info.you();
    assert_ne!(you_ada, you_bo);
    assert_eq!(you_ada.map(|s| s.number()), Some(0));
    fn turn(mover: &mut Headless<HelloGame>, waiter: &mut Headless<HelloGame>, take: Key) {
        mover.clear();
        waiter.clear();
        mover.press(take);
        hear(mover, "You take ");
        hear(mover, "You took ");
        hear(waiter, " took ");
    }
    turn(&mut ada, &mut bo, Key::Num3);

    // A line of chat between moves: T, the line, Enter. Bo hears it as
    // "Ada: ..." and Shift+T reads it back.
    ada.clear();
    bo.clear();
    ada.press(Key::T);
    assert_eq!(ada.app.top_name(), "text_entry");
    for c in "good luck".chars() {
        ada.send(hearken::InputEvent::typed(c));
    }
    ada.press(Key::Return);
    assert_eq!(ada.app.top_name(), "pebbles_match");
    let line = format!(
        "{}: good luck",
        ada.app.ctx.online.player_name().unwrap_or_default()
    );
    hear(&mut bo, &line);
    assert!(
        !ada.spoken().iter().any(|l| l.contains(&line)),
        "own line not read back: {:?}",
        ada.spoken()
    );
    bo.clear();
    bo.send(hearken::InputEvent::key_mods(Key::T, hearken::Mods::SHIFT));
    bo.frame(0.0);
    assert_eq!(bo.spoken().last(), Some(&line));

    turn(&mut bo, &mut ada, Key::Num3);
    turn(&mut ada, &mut bo, Key::Num3);
    turn(&mut bo, &mut ada, Key::Num1);
    tick(&mut ada, &mut bo, 1.0);
    hear(&mut ada, "You won.");
    hear(&mut bo, " won.");

    // The server has the same result: it replayed the match itself.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        tick(&mut ada, &mut bo, 0.5);
        let info = &ada.app.ctx.rooms.room(&id).unwrap().info;
        if info.phase == Phase::Finished {
            assert!(
                info.result
                    .as_ref()
                    .and_then(|r| r["winner"].as_str())
                    .is_some(),
                "{:?}",
                info.result
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the server never finished the room: {info:?}"
        );
    }

    // Back out: the room shows the result, then My games lists it as over.
    ada.clear();
    ada.press(Key::Escape);
    assert_eq!(ada.app.top_name(), "room");
    let lines = ada.app.top_lines();
    assert!(
        lines.iter().any(|l| l.contains("Over: You won")),
        "{lines:?}"
    );
    ada.press(Key::Escape);
    assert_eq!(ada.app.top_name(), "online_play");
    ada.clear();
    for _ in 0..3 {
        ada.press(Key::Down);
    }
    ada.press(Key::Return); // My games
    let line = hear(&mut ada, "Pebbles with ");
    assert!(line.contains("over, You won"), "{line}");
}
