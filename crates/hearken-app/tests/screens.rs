mod common;

use common::{Probe, TestData, TestGame};
use hearken_app::screens::help::sections;
use hearken_app::screens::{HelpScreen, PauseScreen, SettingsScreen};
use hearken_app::{Ctx, Headless, Key};

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
fn the_help_screen_lists_topics_and_reads_one_on_enter() {
    let mut h = over_root(HelpScreen::<TestGame>::new(DOC));
    assert_eq!(
        h.spoken().last().map(String::as_str),
        Some("Help. 2 items. Getting around")
    );
    h.clear();
    h.press(Key::Return);
    assert_eq!(h.spoken(), ["Up and Down move. Enter chooses."]);
}

#[test]
fn speech_rate_moves_by_five_percent_clamps_and_reaches_the_voice() {
    let mut h = over_root(SettingsScreen::<TestGame>::new());
    assert_eq!(
        h.spoken().last().map(String::as_str),
        Some("Settings. 1 item. Speech rate: 50 percent")
    );
    h.clear();
    h.press(Key::Right);
    assert_eq!(h.spoken(), ["Speech rate: 55 percent"]);
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
    assert_eq!(
        h.spoken().last().map(String::as_str),
        Some("Paused. 5 items. Resume")
    );
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
