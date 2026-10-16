mod common;

use common::{take_log, Probe, TestData, TestGame};
use hearken_app::menu::{Menu, MenuCore, MenuItem};
use hearken_app::screens::{ConfirmOption, ConfirmScreen};
use hearken_app::{Ctx, Headless, Key, State};
use hearken_speech::SpeechChannel;

/// Three rows; Enter pushes whatever `child` builds.
struct List {
    menu: MenuCore<Self, TestGame>,
    rebuilt: u32,
}

impl List {
    fn new() -> Self {
        List {
            menu: MenuCore::new("My routes"),
            rebuilt: 0,
        }
    }
}

impl Menu for List {
    type Game = TestGame;
    fn name(&self) -> &'static str {
        "list"
    }
    fn menu(&self) -> &MenuCore<Self, TestGame> {
        &self.menu
    }
    fn menu_mut(&mut self) -> &mut MenuCore<Self, TestGame> {
        &mut self.menu
    }
    fn build_items(&mut self, _ctx: &mut Ctx<TestGame>) -> Vec<MenuItem<Self, TestGame>> {
        self.rebuilt += 1;
        let rebuilt = self.rebuilt;
        vec![
            MenuItem::inert("Alpha"),
            MenuItem::inert("Beta"),
            MenuItem::inert(format!("Gamma, built {rebuilt} times")),
        ]
    }
    fn on_escape(&mut self, _ctx: &mut Ctx<TestGame>) {}
}

/// A screen that announces itself on enter and resume, as most do.
struct Speaker;

impl State<TestGame> for Speaker {
    fn name(&self) -> &'static str {
        "speaker"
    }
    fn enter(&mut self, ctx: &mut Ctx<TestGame>) {
        ctx.say("Speaker screen.");
    }
    fn help(&self, _ctx: &Ctx<TestGame>) -> String {
        "help".to_string()
    }
}

/// The list with its cursor on Beta, and `top` pushed over it.
fn list_under(top: impl State<TestGame> + 'static) -> Headless<TestGame> {
    let mut h = Headless::new(TestData::default(), List::new());
    h.press(Key::Down);
    h.app.ctx.push(top);
    h.frame(0.0);
    take_log(&mut h.app.ctx);
    h.clear();
    h
}

fn closer(name: &'static str, f: impl FnMut(&mut Ctx<TestGame>) + 'static) -> Probe {
    Probe::new(name).on_return(f)
}

#[test]
fn a_quiet_pop_reveals_the_screen_below_without_a_word() {
    let mut h = list_under(closer("b", |ctx| ctx.pop_quiet()));
    h.press(Key::Return);
    assert_eq!(h.app.top_name(), "list");
    assert!(h.spoken().is_empty(), "{:?}", h.spoken());
    // The rows were rebuilt underneath, cursor kept.
    h.press(Key::Down);
    assert_eq!(h.spoken(), ["Gamma, built 2 times. 3 of 3."]);
}

#[test]
fn a_quiet_pop_does_not_resume_a_plain_screen() {
    let mut h = Headless::new(TestData::default(), Probe::new("root"));
    h.app.ctx.push(closer("b", |ctx| ctx.pop_quiet()));
    h.frame(0.0);
    take_log(&mut h.app.ctx);
    h.press(Key::Return);
    assert_eq!(take_log(&mut h.app.ctx), ["b key Return", "b exit"]);
}

#[test]
fn a_result_is_spoken_first_then_the_menus_current_row_without_its_title() {
    let mut h = list_under(closer("b", |ctx| ctx.pop_with("Via Ames added")));
    h.press(Key::Return);
    assert_eq!(h.spoken(), ["Via Ames added.", "Beta. 2 of 3."]);
    assert_eq!(
        h.speech.borrow().calls(SpeechChannel::Main),
        [
            ("Via Ames added.".to_string(), true),
            ("Beta. 2 of 3.".to_string(), false)
        ],
        "the result interrupts; the row queues behind it"
    );
}

#[test]
fn a_plain_screen_speaks_the_result_then_announces_itself_queued() {
    let mut h = Headless::new(TestData::default(), Speaker);
    h.app.ctx.push(closer("b", |ctx| ctx.pop_with("Saved.")));
    h.frame(0.0);
    h.clear();
    h.press(Key::Return);
    assert_eq!(
        h.speech.borrow().calls(SpeechChannel::Main),
        [
            ("Saved.".to_string(), true),
            ("Speaker screen.".to_string(), false)
        ]
    );
}

#[test]
fn a_blank_result_is_an_ordinary_pop() {
    let mut h = list_under(closer("b", |ctx| ctx.pop_with("  ")));
    h.press(Key::Return);
    assert_eq!(h.spoken(), ["My routes.", "Beta. 2 of 3."]);
}

#[test]
fn pop_to_closes_every_screen_above_the_named_one_and_it_announces() {
    let mut h = list_under(Probe::new("b"));
    h.app.ctx.push(closer("c", |ctx| ctx.pop_to("list")));
    h.frame(0.0);
    take_log(&mut h.app.ctx);
    h.clear();
    h.press(Key::Return);
    assert_eq!(
        take_log(&mut h.app.ctx),
        ["c key Return", "c exit", "b exit"]
    );
    assert_eq!((h.app.top_name(), h.app.depth()), ("list", 1));
    assert_eq!(h.spoken(), ["My routes.", "Beta. 2 of 3."]);
}

#[test]
fn pop_to_with_a_result_and_quietly() {
    let mut h = list_under(Probe::new("b"));
    h.app.ctx.push(closer("c", |ctx| {
        ctx.pop_to_with("list", "Gamma deleted. My routes.")
    }));
    h.frame(0.0);
    h.clear();
    h.press(Key::Return);
    assert_eq!(h.spoken(), ["Gamma deleted. My routes.", "Beta. 2 of 3."]);

    let mut h = list_under(Probe::new("b"));
    h.app.ctx.push(closer("c", |ctx| ctx.pop_to_quiet("list")));
    h.frame(0.0);
    h.clear();
    h.press(Key::Return);
    assert_eq!(h.app.depth(), 1);
    assert!(h.spoken().is_empty());
}

#[test]
fn pop_to_a_screen_not_on_the_stack_changes_nothing() {
    let mut h = list_under(closer("b", |ctx| ctx.pop_to("nowhere")));
    h.press(Key::Return);
    assert_eq!((h.app.top_name(), h.app.depth()), ("b", 2));
    assert_eq!(take_log(&mut h.app.ctx), ["b key Return"]);
}

#[test]
fn pop_to_the_top_screen_closes_nothing_but_still_speaks_a_result() {
    let mut h = Headless::new(TestData::default(), List::new());
    h.clear();
    h.app.ctx.pop_to("list");
    h.frame(0.0);
    assert!(h.spoken().is_empty());
    h.app.ctx.pop_to_with("list", "Nothing to undo");
    h.frame(0.0);
    assert_eq!(h.app.depth(), 1);
    assert_eq!(h.spoken(), ["Nothing to undo.", "Alpha. 1 of 3."]);
}

// -- the confirm screen ------------------------------------------------------

fn confirm_over_list(screen: ConfirmScreen<TestGame>) -> Headless<TestGame> {
    list_under(screen)
}

#[test]
fn the_question_comes_before_the_options() {
    let mut h = Headless::new(TestData::default(), List::new());
    h.app.ctx.push(ConfirmScreen::<TestGame>::new(
        "Delete Gamma? This cannot be undone.",
        |_ctx| {},
    ));
    h.clear();
    h.frame(0.0);
    assert_eq!(
        h.spoken(),
        ["Delete Gamma? This cannot be undone.", "Yes. 1 of 2."]
    );
    h.clear();
    h.press(Key::Down);
    assert_eq!(h.spoken(), ["No. 2 of 2."]);
    h.clear();
    h.press(Key::F1);
    assert_eq!(
        h.spoken(),
        ["Up and Down move, Enter chooses, Escape chooses No."]
    );
}

#[test]
fn yes_runs_its_closure_and_its_own_stack_change_decides_what_is_heard() {
    let mut h = confirm_over_list(ConfirmScreen::new(
        "Delete Gamma?",
        |ctx: &mut Ctx<TestGame>| {
            ctx.game.log.push("deleted".to_string());
            ctx.pop_with("Gamma deleted. My routes.");
        },
    ));
    h.press(Key::Return);
    assert_eq!(h.app.top_name(), "list");
    assert!(h.app.ctx.game.log.contains(&"deleted".to_string()));
    assert_eq!(h.spoken(), ["Gamma deleted. My routes.", "Beta. 2 of 3."]);
}

#[test]
fn an_option_that_leaves_the_stack_alone_closes_the_screen() {
    let mut h = confirm_over_list(ConfirmScreen::new("Sure?", |ctx: &mut Ctx<TestGame>| {
        ctx.game.log.push("yes".to_string());
    }));
    h.press(Key::Return);
    assert_eq!(h.app.top_name(), "list");
    assert_eq!(h.spoken(), ["My routes.", "Beta. 2 of 3."]);
}

#[test]
fn escape_picks_the_cancel_option() {
    let mut h = confirm_over_list(
        ConfirmScreen::new("Discard the plan?", |ctx: &mut Ctx<TestGame>| {
            ctx.game.log.push("discarded".to_string())
        })
        .yes_label("Discard")
        .no_label("Keep editing"),
    );
    h.press(Key::Escape);
    assert_eq!(h.app.top_name(), "list");
    assert!(!h.app.ctx.game.log.contains(&"discarded".to_string()));
    assert_eq!(h.spoken(), ["My routes.", "Beta. 2 of 3."]);
}

#[test]
fn custom_options_a_chosen_cancel_and_focus() {
    let mut h = Headless::new(TestData::default(), List::new());
    h.app.ctx.push(
        ConfirmScreen::<TestGame>::with_options(
            "Leave the run?",
            vec![
                ConfirmOption::close("Stay").help("Back to the cab."),
                ConfirmOption::new("Save and leave", |ctx: &mut Ctx<TestGame>| {
                    ctx.game.log.push("saved".to_string());
                    ctx.pop_to_quiet("list");
                }),
                ConfirmOption::close("Leave without saving"),
            ],
        )
        .cancel(0)
        .focus(1),
    );
    h.frame(0.0);
    h.clear();
    h.press(Key::Up);
    h.press(Key::F1);
    assert_eq!(h.spoken(), ["Stay. 1 of 3.", "Back to the cab."]);
    h.press(Key::Down);
    h.clear();
    h.press(Key::Return);
    assert!(h.app.ctx.game.log.contains(&"saved".to_string()));
    assert_eq!(h.app.top_name(), "list");
    assert!(h.spoken().is_empty());
}

#[test]
fn focus_cancel_starts_on_no() {
    let mut h = Headless::new(TestData::default(), List::new());
    h.app
        .ctx
        .push(ConfirmScreen::<TestGame>::new("Delete?", |_ctx| {}).focus_cancel());
    h.clear();
    h.frame(0.0);
    assert_eq!(h.spoken(), ["Delete?", "No. 2 of 2."]);
}

#[test]
fn an_ending_option_quits_without_the_screen_beneath_speaking() {
    let mut h = list_under(ConfirmScreen::<TestGame>::with_options(
        "Quit the game?",
        vec![
            ConfirmOption::new("Quit", |ctx: &mut Ctx<TestGame>| ctx.say("Goodbye.")).ends_game(),
            ConfirmOption::close("Keep playing"),
        ],
    ));
    h.frame(0.0);
    h.clear();
    h.press(Key::Return);
    assert!(!h.app.ctx.running);
    assert_eq!(h.spoken(), ["Goodbye."]);
    assert_eq!(h.app.depth(), 2, "nothing closed, so nothing beneath spoke");
}
