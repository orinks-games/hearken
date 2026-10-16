mod common;

use common::{Probe, TestData, TestGame};
use hearken_app::menu::{Label, Menu, MenuCore, MenuItem, DEFAULT_INTRO_HELP};
use hearken_app::{Ctx, Headless, InputEvent, Key};

/// Alpha adjusts, Beta cannot, Gamma rebuilds the rows with a fourth.
struct TestMenu {
    menu: MenuCore<Self, TestGame>,
    activated: u32,
    volume: i32,
    grown: bool,
}

impl TestMenu {
    fn new() -> Self {
        TestMenu {
            menu: MenuCore::new("Test menu"),
            activated: 0,
            volume: 70,
            grown: false,
        }
    }
}

impl Menu for TestMenu {
    type Game = TestGame;

    fn name(&self) -> &'static str {
        "test_menu"
    }
    fn menu(&self) -> &MenuCore<Self, TestGame> {
        &self.menu
    }
    fn menu_mut(&mut self) -> &mut MenuCore<Self, TestGame> {
        &mut self.menu
    }
    fn build_items(&mut self, _ctx: &mut Ctx<TestGame>) -> Vec<MenuItem<Self, TestGame>> {
        let mut items = vec![
            MenuItem::new(
                Label::dynamic(|s: &TestMenu, _| {
                    if s.volume == 70 {
                        "Alpha".to_string()
                    } else {
                        format!("Alpha: {}", s.volume)
                    }
                }),
                |s: &mut TestMenu, _| s.activated += 1,
            )
            .help("Alpha help")
            .adjustable(|s: &mut TestMenu, _, delta| s.volume += delta * 10),
            MenuItem::inert("Beta").help("Beta help"),
            MenuItem::new("Gamma", |s: &mut TestMenu, ctx| {
                s.grown = true;
                Menu::refresh(s, ctx, true);
            }),
        ];
        if self.grown {
            items.push(MenuItem::inert("Delta"));
        }
        items
    }
}

struct Empty {
    menu: MenuCore<Self, TestGame>,
}

impl Menu for Empty {
    type Game = TestGame;

    fn name(&self) -> &'static str {
        "empty"
    }
    fn menu(&self) -> &MenuCore<Self, TestGame> {
        &self.menu
    }
    fn menu_mut(&mut self) -> &mut MenuCore<Self, TestGame> {
        &mut self.menu
    }
    fn build_items(&mut self, _ctx: &mut Ctx<TestGame>) -> Vec<MenuItem<Self, TestGame>> {
        Vec::new()
    }
}

/// The test menu open over a probe root, with its announcement flushed.
fn open() -> Headless<TestGame> {
    let mut h = Headless::new(TestData::default(), Probe::new("root"));
    h.app.ctx.push(TestMenu::new());
    h.frame(0.0);
    h
}

#[test]
fn opening_announces_the_title_then_the_first_row_and_where_it_is() {
    let h = open();
    assert_eq!(h.spoken(), ["Test menu.", "Alpha. 1 of 3."]);
}

#[test]
fn up_and_down_speak_the_row_and_wrap() {
    let mut h = open();
    h.clear();
    for key in [Key::Down, Key::Down, Key::Down, Key::Up] {
        h.press(key);
    }
    assert_eq!(
        h.spoken(),
        [
            "Beta. 2 of 3.",
            "Gamma. 3 of 3.",
            "Alpha. 1 of 3.",
            "Gamma. 3 of 3."
        ]
    );
}

#[test]
fn with_positions_off_a_row_is_read_without_where_it_is() {
    let mut h = open();
    h.app.ctx.settings.menus.announce_position = false;
    h.clear();
    h.press(Key::Down);
    assert_eq!(h.spoken(), ["Beta."]);
}

#[test]
fn home_and_end_jump_to_the_edges() {
    let mut h = open();
    h.clear();
    h.press(Key::End);
    h.press(Key::Home);
    assert_eq!(h.spoken(), ["Gamma. 3 of 3.", "Alpha. 1 of 3."]);
}

#[test]
fn enter_runs_the_row_and_an_action_that_rebuilds_keeps_its_rows() {
    let mut h = open();
    h.press(Key::End);
    h.press(Key::Return);
    assert!(h.app.top_lines().contains(&"  Delta".to_string()));
}

#[test]
fn left_and_right_adjust_and_read_back_and_a_row_without_an_adjuster_is_silent() {
    let mut h = open();
    h.clear();
    h.press(Key::Right);
    assert_eq!(h.spoken(), ["Alpha: 80. 1 of 3."]);
    h.press(Key::Down);
    h.clear();
    h.press(Key::Right);
    assert!(h.spoken().is_empty());
}

#[test]
fn a_letter_jumps_to_the_next_matching_row_and_a_miss_is_silent() {
    let mut h = open();
    h.clear();
    h.send(InputEvent::typed('g'));
    assert_eq!(h.spoken(), ["Gamma. 3 of 3."]);
    h.clear();
    h.send(InputEvent::typed('z'));
    assert!(h.spoken().is_empty());
}

#[test]
fn f1_reads_the_row_help_or_the_menu_help_when_the_row_has_none() {
    let mut h = open();
    h.clear();
    h.press(Key::F1);
    h.press(Key::End);
    h.clear();
    h.press(Key::F1);
    assert_eq!(h.spoken(), [DEFAULT_INTRO_HELP]);
}

#[test]
fn f1_on_the_first_row_reads_its_help() {
    let mut h = open();
    h.clear();
    h.press(Key::F1);
    assert_eq!(h.spoken(), ["Alpha help"]);
}

#[test]
fn escape_closes_the_menu_and_the_screen_below_resumes() {
    let mut h = open();
    h.press(Key::Escape);
    assert_eq!((h.app.top_name(), h.app.depth()), ("root", 1));
}

#[test]
fn coming_back_keeps_the_cursor() {
    let mut h = open();
    h.press(Key::Down);
    h.app.ctx.push(Probe::new("above"));
    h.frame(0.0);
    h.app.ctx.pop();
    h.clear();
    h.frame(0.0);
    assert_eq!(h.spoken(), ["Test menu.", "Beta. 2 of 3."]);
}

#[test]
fn an_empty_menu_says_so_and_ignores_keys() {
    let mut h = Headless::new(TestData::default(), Probe::new("root"));
    h.app.ctx.push(Empty {
        menu: MenuCore::new("Nothing here"),
    });
    h.frame(0.0);
    assert_eq!(h.spoken(), ["Nothing here.", "No options available."]);
    h.clear();
    for key in [
        Key::Down,
        Key::Up,
        Key::Home,
        Key::End,
        Key::Return,
        Key::Right,
    ] {
        h.press(key);
    }
    h.send(InputEvent::typed('a'));
    assert!(h.spoken().is_empty());
    h.press(Key::F1);
    assert_eq!(h.spoken(), [DEFAULT_INTRO_HELP]);
}

struct Stepper {
    menu: MenuCore<Self, TestGame>,
    level: i32,
}

impl Menu for Stepper {
    type Game = TestGame;
    fn name(&self) -> &'static str {
        "stepper"
    }
    fn menu(&self) -> &MenuCore<Self, TestGame> {
        &self.menu
    }
    fn menu_mut(&mut self) -> &mut MenuCore<Self, TestGame> {
        &mut self.menu
    }
    fn build_items(&mut self, _ctx: &mut Ctx<TestGame>) -> Vec<MenuItem<Self, TestGame>> {
        vec![MenuItem::inert(Label::dynamic(|s: &Stepper, _| {
            format!("Level: {}", s.level)
        }))
        .adjustable(|s: &mut Stepper, _, delta| s.level += delta)
        .adjusted_text(Label::dynamic(|s: &Stepper, _| {
            format!("{}, level", s.level)
        }))]
    }
}

#[test]
fn a_row_with_adjusted_text_reads_that_back_on_left_and_right() {
    let mut h = Headless::new(
        TestData::default(),
        Stepper {
            menu: MenuCore::new("Steps"),
            level: 1,
        },
    );
    assert_eq!(h.spoken(), ["Steps.", "Level: 1. 1 of 1."]);
    h.clear();
    h.press(Key::Right);
    h.press(Key::Right);
    assert_eq!(h.spoken(), ["2, level.", "3, level."]);
}
