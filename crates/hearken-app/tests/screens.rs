mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::{Probe, TestData, TestGame};
use hearken_app::screens::help::sections;
use hearken_app::screens::{HelpScreen, PauseScreen, SettingsScreen};
use hearken_app::{App, Ctx, Game, Headless, Key, NoAction, Paths, Settings};
use hearken_speech::{CaptureSpeech, SharedCapture};

const DOC: &str = "# Test Game\n\nIgnored preamble.\n\n## Getting around\n\nUp and Down move.\nEnter chooses.\n\n## Quitting\n\nPick Quit.\n";

fn over_root(screen: impl hearken_app::State<TestGame> + 'static) -> Headless<TestGame> {
    let mut h = Headless::new(TestData::default(), Probe::new("root"));
    h.app.ctx.push(screen);
    h.frame(0.0);
    h
}

#[test]
fn help_sections_split_on_level_two_headings() {
    let s = sections(DOC);
    assert_eq!(s.len(), 2);
    assert_eq!(s[0].title, "Getting around");
    assert_eq!(s[0].body, "Up and Down move. Enter chooses.");
    assert_eq!(s[1].body, "Pick Quit.");
}

#[test]
fn smaller_headings_inside_a_section_lose_their_marks() {
    let s = sections("## Keys\n\n### Moving\n\nUp and Down move.\n####Choosing\nEnter chooses.\n");
    assert_eq!(s.len(), 1);
    assert_eq!(
        s[0].body,
        "Moving Up and Down move. Choosing Enter chooses."
    );
}

#[test]
fn the_help_screen_lists_topics_and_reads_one_on_enter() {
    let mut h = over_root(HelpScreen::<TestGame>::new(DOC));
    assert_eq!(h.spoken(), ["Help.", "Getting around. 1 of 2."]);
    h.clear();
    h.press(Key::Return);
    assert_eq!(h.spoken(), ["Up and Down move. Enter chooses."]);
}

#[test]
fn speech_rate_moves_by_five_percent_clamps_and_reaches_the_voice() {
    let mut h = over_root(SettingsScreen::<TestGame>::new());
    assert_eq!(
        h.spoken(),
        ["Settings.", "Speech rate: 50 percent. 1 of 14."]
    );
    h.press(Key::Right);
    assert_eq!(h.app.ctx.settings.speech.rate, 0.55);
    assert_eq!(
        h.speech.borrow().configure_calls().last().and_then(|c| c.0),
        Some(0.55),
        "the new rate reached the voice"
    );
    for _ in 0..20 {
        h.press(Key::Left);
    }
    assert_eq!(h.app.ctx.settings.speech.rate, 0.0);
}

#[test]
fn the_new_rate_is_read_back_through_the_voice_it_changed() {
    let mut h = over_root(SettingsScreen::<TestGame>::new());
    h.clear();
    h.press(Key::Right);
    let line = "Speech rate: 55 percent. 1 of 14.";
    assert_eq!(
        h.speech.borrow().previews(),
        [("speech_rate".to_string(), line.to_string(), true)]
    );
    assert!(h.spoken().is_empty(), "read once, by the previewing voice");
    assert_eq!(h.app.ctx.transcript.last().map(String::as_str), Some(line));
}

#[test]
fn without_a_voice_that_can_change_rate_there_is_no_rate_row() {
    let capture = Rc::new(RefCell::new(CaptureSpeech::new()));
    let dir = tempfile::tempdir().unwrap();
    let ctx: Ctx<TestGame> = Ctx::new(
        Box::new(SharedCapture(capture.clone())),
        Settings::default(),
        Paths::at(dir.path()),
        TestData::default(),
    );
    let _app = App::new(ctx, Box::new(SettingsScreen::<TestGame>::new()));
    assert_eq!(
        capture.borrow().main_lines(),
        ["Settings.", "Speech detail: Full. 1 of 13."]
    );
}

#[test]
fn the_menu_positions_row_toggles_on_enter_left_and_right_and_reads_back_once() {
    let mut h = over_root(SettingsScreen::<TestGame>::new());
    // M jumps through Master volume, Music volume, then Menu positions.
    for _ in 0..3 {
        h.press(Key::M);
    }
    h.clear();
    h.press(Key::Right);
    assert!(!h.app.ctx.settings.menus.announce_position);
    h.press(Key::Left);
    assert!(h.app.ctx.settings.menus.announce_position);
    h.press(Key::Return);
    assert!(!h.app.ctx.settings.menus.announce_position);
    assert_eq!(
        h.spoken(),
        [
            "Menu positions: off.",
            "Menu positions: on. 11 of 14.",
            "Menu positions: off."
        ]
    );
}

#[test]
fn enter_on_an_inert_row_plays_nothing() {
    let mut h = over_root(SettingsScreen::<TestGame>::new());
    h.clear();
    h.press(Key::Return); // Speech rate: adjustable, but does nothing on Enter
    assert!(h.played().is_empty());
    h.press(Key::M); // Master volume: also inert
    h.clear();
    h.press(Key::Return);
    assert!(h.played().is_empty());
}

#[test]
fn escape_on_settings_saves_them_and_goes_back() {
    let mut h = over_root(SettingsScreen::<TestGame>::new());
    h.press(Key::Right);
    h.press(Key::Escape);
    assert_eq!(h.app.top_name(), "root");
    let saved = std::fs::read_to_string(h.app.ctx.paths.settings_file()).unwrap();
    assert!(saved.contains("0.55"), "saved: {saved}");
}

fn back_to_title(ctx: &mut Ctx<TestGame>) {
    ctx.reset_to(Probe::new("title"));
}

#[test]
fn pause_lists_its_rows_and_each_one_works() {
    let mut h = over_root(PauseScreen::<TestGame>::new(DOC, back_to_title));
    assert_eq!(h.spoken(), ["Paused.", "Resume. 1 of 5."]);
    h.press(Key::Return);
    assert_eq!(h.app.top_name(), "root", "Resume closes the pause menu");

    h.app
        .ctx
        .push(PauseScreen::<TestGame>::new(DOC, back_to_title));
    h.frame(0.0);
    h.press(Key::Down);
    h.press(Key::Return);
    assert_eq!(h.app.top_name(), "settings");
    h.press(Key::Escape);
    h.press(Key::Down);
    h.press(Key::Return);
    assert_eq!(h.app.top_name(), "help");
    h.press(Key::Escape);
    h.press(Key::Down);
    h.press(Key::Return);
    assert_eq!((h.app.top_name(), h.app.depth()), ("title", 1));

    h.app
        .ctx
        .push(PauseScreen::<TestGame>::new(DOC, back_to_title));
    h.frame(0.0);
    h.press(Key::End);
    h.press(Key::Return);
    assert!(!h.app.ctx.running, "Quit ends the run");
}

#[test]
fn settings_open_the_shortcut_screens_and_turn_the_controller_off_and_on() {
    let mut h = over_root(SettingsScreen::<TestGame>::new());
    h.press(Key::K); // Keyboard shortcuts
    h.clear();
    h.press(Key::Return);
    assert_eq!(h.spoken(), ["Keyboard shortcuts.", "Jump: Space. 1 of 4."]);
    h.press(Key::Escape);
    h.press(Key::Down); // Controller buttons, right after Keyboard shortcuts
    h.press(Key::Return);
    assert_eq!(h.app.top_name(), "bindings");
    h.press(Key::Escape);
    h.press(Key::Down); // Controller, right after Controller buttons
    h.clear();
    h.press(Key::Return);
    assert_eq!(h.spoken(), ["Controller: off. 14 of 14."]);
    assert!(!h.app.ctx.settings.controller.enabled);
    assert!(!h.app.ctx.controller.enabled());
    h.press(Key::Right);
    assert!(h.app.ctx.controller.enabled());
}

struct Bare;

impl Game for Bare {
    const TITLE: &'static str = "Bare";
    const DATA_DIR: &'static str = "HearkenBareTest";
    const ENV_PREFIX: &'static str = "HEARKEN_BARE_TEST";
    type Event = ();
    type Data = ();
    type Settings = ();
    type Action = NoAction;
    type Category = hearken_app::NoCategory;
}

#[test]
fn a_game_with_nothing_to_rebind_gets_no_shortcut_rows() {
    let h = Headless::<Bare>::new((), SettingsScreen::<Bare>::new());
    assert_eq!(
        h.spoken(),
        ["Settings.", "Speech rate: 50 percent. 1 of 10."]
    );
}
