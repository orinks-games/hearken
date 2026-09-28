mod common;

use common::*;
use hearken_app::menu::{Menu, MenuCore, MenuItem};
use hearken_app::screens::{PauseScreen, TextEntryScreen, TimedMessage};
use hearken_app::{Bus, Ctx, Headless, InputEvent, Key};

struct Rows {
    menu: MenuCore<Self, TestGame>,
    level: i32,
}

impl Menu for Rows {
    type Game = TestGame;
    fn name(&self) -> &'static str {
        "rows"
    }
    fn menu(&self) -> &MenuCore<Self, TestGame> {
        &self.menu
    }
    fn menu_mut(&mut self) -> &mut MenuCore<Self, TestGame> {
        &mut self.menu
    }
    fn build_items(&mut self, _ctx: &mut Ctx<TestGame>) -> Vec<MenuItem<Self, TestGame>> {
        vec![
            MenuItem::new("Alpha", |_s: &mut Self, _c: &mut Ctx<TestGame>| {}),
            MenuItem::new("Door", |_s: &mut Self, _c: &mut Ctx<TestGame>| {})
                .select_sound(Some("sfx/door")),
            MenuItem::new("Hush", |_s: &mut Self, _c: &mut Ctx<TestGame>| {}).select_sound(None),
            MenuItem::inert("Level")
                .adjustable(|s: &mut Self, _c: &mut Ctx<TestGame>, d| s.level += d),
            MenuItem::inert("Quiet level")
                .adjustable(|s: &mut Self, _c: &mut Ctx<TestGame>, d| s.level += d)
                .silent_adjust(),
        ]
    }
}

fn rows() -> Headless<TestGame> {
    let mut h = Headless::new(TestData::default(), Probe::new("root"));
    h.app.ctx.push(Rows {
        menu: MenuCore::new("Rows"),
        level: 0,
    });
    h.frame(0.0);
    h
}

#[test]
fn a_menu_opens_moves_selects_and_goes_back_with_its_sounds() {
    let mut h = rows();
    assert_eq!(h.played(), ["ui/menu_open"]);
    h.clear();
    h.press(Key::Down);
    h.press(Key::Up);
    h.press(Key::Return);
    h.press(Key::Escape);
    assert_eq!(
        h.played(),
        [
            "ui/menu_move",
            "ui/menu_move",
            "ui/menu_select",
            "ui/menu_back"
        ]
    );
}

#[test]
fn a_row_can_have_its_own_select_sound_or_none() {
    let mut h = rows();
    h.clear();
    h.press(Key::Down);
    h.press(Key::Return);
    h.press(Key::Down);
    h.press(Key::Return);
    assert_eq!(h.played(), ["ui/menu_move", "sfx/door", "ui/menu_move"]);
}

#[test]
fn left_and_right_tick_unless_the_row_plays_its_own() {
    let mut h = rows();
    h.press(Key::End);
    h.press(Key::Up); // Level
    h.clear();
    h.press(Key::Right);
    assert_eq!(h.played(), ["ui/menu_move"]);
    h.press(Key::Down); // Quiet level
    h.clear();
    h.press(Key::Right);
    assert!(h.played().is_empty());
}

#[test]
fn a_first_letter_that_matches_nothing_is_silent() {
    let mut h = rows();
    h.clear();
    h.press(Key::Z);
    assert!(h.played().is_empty());
}

#[test]
fn typing_ticks_and_every_edge_is_an_error_sound() {
    let mut h = Headless::new(TestData::default(), Probe::new("root"));
    h.app.ctx.push(
        TextEntryScreen::<TestGame>::new("Name", "Name", |_c: &mut Ctx<TestGame>, _t: &str| {})
            .max_len(1),
    );
    h.frame(0.0);
    h.clear();
    h.press(Key::Backspace);
    h.press(Key::Left);
    h.press(Key::Home);
    assert_eq!(h.played(), ["ui/error", "ui/error", "ui/error"]);
    assert!(
        h.spoken().is_empty(),
        "the sound says it; words would repeat it"
    );
    h.clear();
    h.send(InputEvent::key_text(Key::A, 'a'));
    h.send(InputEvent::key_text(Key::B, 'b'));
    assert_eq!(h.played(), ["ui/tick", "ui/error"]);
    assert_eq!(
        h.spoken().last().map(String::as_str),
        Some("The field is full.")
    );
    h.clear();
    h.press(Key::Right);
    h.press(Key::End);
    h.press(Key::Escape);
    assert_eq!(h.played(), ["ui/error", "ui/error", "ui/menu_back"]);
}

#[test]
fn a_timed_message_arrives_with_its_notify_sound() {
    let mut h = Headless::new(TestData::default(), Probe::new("root"));
    h.clear();
    h.app.ctx.push(TimedMessage::<TestGame>::new(
        "Ready",
        "Get ready.",
        "Starting soon.",
        1.0,
        |_c: &mut Ctx<TestGame>| {},
    ));
    h.frame(0.0);
    assert_eq!(h.played(), ["ui/notify"]);
}

#[test]
fn pause_holds_the_games_sounds_and_event_speech_and_resume_brings_the_sounds_back() {
    let mut h = Headless::new(TestData::default(), Probe::new("root"));
    h.app.ctx.audio.start_loop("amb/hum", None, Bus::AMBIENCE);
    let hum = h.sounds.channel_of("amb/hum").unwrap();
    let stops = h.speech.borrow().stop_event_calls();
    h.app.ctx.push(PauseScreen::<TestGame>::new(
        "# Help",
        |_c: &mut Ctx<TestGame>| {},
    ));
    h.frame(0.0);
    assert!(h.sounds.calls().contains(&hearken_app::Call::Pause {
        channel: hum,
        paused: true
    }));
    assert_eq!(h.speech.borrow().stop_event_calls(), stops + 1);
    h.press(Key::Return); // Resume
    assert!(h.sounds.calls().contains(&hearken_app::Call::Pause {
        channel: hum,
        paused: false
    }));
}
