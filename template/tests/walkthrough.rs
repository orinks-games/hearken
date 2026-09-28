use hearken_app::{Headless, InputEvent, Key, Mods};
use hello_hearken::{HelloGame, Title};

fn start() -> Headless<HelloGame> {
    Headless::new((), Title::new())
}

fn into_practice() -> Headless<HelloGame> {
    let mut h = start();
    h.press(Key::Return); // Play
    h.clear();
    h
}

#[test]
fn the_title_announces_itself() {
    let h = start();
    assert_eq!(h.spoken(), ["Hello Hearken.", "Play. 1 of 4."]);
}

#[test]
fn settings_open_and_the_title_comes_back_where_it_was() {
    let mut h = start();
    h.press(Key::Down);
    h.clear();
    h.press(Key::Return);
    assert_eq!(
        h.spoken(),
        ["Settings.", "Speech rate: 50 percent. 1 of 12."]
    );
    h.clear();
    h.press(Key::Escape);
    assert_eq!(h.spoken(), ["Hello Hearken.", "Settings. 2 of 4."]);
}

#[test]
fn practice_counts_and_pauses_and_returns_to_the_title() {
    let mut h = start();
    h.clear();
    h.press(Key::Return);
    assert_eq!(h.app.top_name(), "practice");
    assert_eq!(
        h.spoken(),
        ["The practice room. A beacon circles you. Space counts, hold the Up arrow to rev the engine, Escape pauses."]
    );
    h.clear();
    h.press(Key::Space);
    h.press(Key::Space);
    assert_eq!(h.spoken(), ["1", "2"]);
    h.press(Key::Escape);
    assert_eq!(h.app.top_name(), "pause");
    for _ in 0..3 {
        h.press(Key::Down);
    }
    h.press(Key::Return);
    assert_eq!((h.app.top_name(), h.app.depth()), ("title", 1));
}

#[test]
fn escape_on_the_title_says_how_to_leave_and_quit_leaves() {
    let mut h = start();
    h.clear();
    h.press(Key::Escape);
    assert_eq!(h.spoken(), ["Press Enter on Quit to leave."]);
    assert!(h.app.ctx.running);
    h.press(Key::End);
    h.press(Key::Return);
    assert!(!h.app.ctx.running);
}

#[test]
fn holding_a_key_in_the_practice_room_counts_once() {
    let mut h = start();
    h.press(Key::Return);
    h.clear();
    h.press(Key::Space);
    for _ in 0..5 {
        h.send(InputEvent::KeyDown {
            key: Key::Space,
            mods: Mods::NONE,
            text: Some(' '),
            repeat: true,
        });
    }
    assert_eq!(h.spoken(), ["1"]);
}

#[test]
fn count_can_move_to_another_key() {
    let mut h = start();
    h.press(Key::Down);
    h.press(Key::Return); // Settings
    h.press(Key::K); // Keyboard shortcuts
    h.press(Key::Return); // Keyboard shortcuts
    h.press(Key::Return); // Count
    h.press(Key::C);
    assert_eq!(
        h.spoken().last().map(String::as_str),
        Some("Count is now C.")
    );
    h.press(Key::Escape);
    h.press(Key::Escape);
    h.press(Key::Home);
    h.clear();
    h.press(Key::Return); // Play
    assert_eq!(
        h.spoken(),
        ["The practice room. A beacon circles you. C counts, hold the Up arrow to rev the engine, Escape pauses."]
    );
    h.clear();
    h.press(Key::C);
    h.press(Key::Space);
    assert_eq!(h.spoken(), ["1"]);
}

#[test]
fn the_title_ticks_as_you_move() {
    let mut h = start();
    h.clear();
    h.press(Key::Down);
    assert_eq!(h.played(), ["ui/menu_move"]);
}

#[test]
fn the_practice_room_runs_an_engine_and_a_beacon_pings_as_it_circles() {
    let mut h = into_practice();
    assert!(
        h.app.ctx.audio.live().iter().any(|s| s.params.is_some()),
        "the engine voice runs"
    );
    let mut places = Vec::new();
    for _ in 0..40 {
        h.frame(0.05);
    }
    for call in h.sounds.calls() {
        if let hearken_app::Call::Play { key, mix, .. } = call {
            if key == "sfx/beacon" {
                places.push(mix.relative.expect("a placed sound"));
            }
        }
    }
    assert!(places.len() >= 2, "a ping every 0.8 seconds: {places:?}");
    assert!(
        (places[0] - places[1]).length() > 1.0,
        "the beacon moved between pings: {places:?}"
    );
}

#[test]
fn holding_the_engine_key_revs_it_and_letting_go_idles_it() {
    let mut h = into_practice();
    h.send(InputEvent::key(Key::Up));
    for _ in 0..30 {
        h.frame(0.05);
    }
    let throttle = |h: &Headless<HelloGame>| {
        h.app
            .ctx
            .audio
            .live()
            .iter()
            .find_map(|s| s.params.as_ref()?.value("throttle"))
            .unwrap()
    };
    assert!(throttle(&h) > 0.5);
    h.send(InputEvent::key_up(Key::Up));
    for _ in 0..30 {
        h.frame(0.05);
    }
    assert!(throttle(&h) < 0.2);
}

#[test]
fn leaving_the_practice_room_stops_its_engine() {
    let mut h = into_practice();
    h.press(Key::Escape);
    h.press(Key::End);
    h.press(Key::Up); // Return to the title
    h.press(Key::Return);
    assert!(h.app.ctx.audio.live().iter().all(|s| s.params.is_none()));
}
