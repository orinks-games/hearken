mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::{Probe, TestData, TestGame};
use hearken_app::screens::text_entry::INSTRUCTIONS;
use hearken_app::screens::{TextEntryScreen, TimedMessage};
use hearken_app::{ControllerButton, Ctx, Headless, InputEvent, Key};
use hearken_speech::SpeechChannel;

/// A name field over a root screen. What Enter hands back lands in the
/// returned list.
fn field() -> (Headless<TestGame>, Rc<RefCell<Vec<String>>>) {
    let confirmed = Rc::new(RefCell::new(Vec::new()));
    let seen = confirmed.clone();
    let mut h = Headless::new(TestData::default(), Probe::new("root"));
    h.app.ctx.push(TextEntryScreen::<TestGame>::new(
        "New player",
        "Name",
        move |_ctx: &mut Ctx<TestGame>, text: &str| seen.borrow_mut().push(text.to_string()),
    ));
    h.frame(0.0);
    (h, confirmed)
}

fn type_text(h: &mut Headless<TestGame>, text: &str) {
    for ch in text.chars() {
        h.send(InputEvent::typed(ch));
    }
}

#[test]
fn the_field_says_what_it_is_and_how_it_works() {
    let (mut h, _) = field();
    assert_eq!(h.spoken(), [format!("New player. Name. {INSTRUCTIONS}")]);
    h.clear();
    h.press(Key::F1);
    assert_eq!(h.spoken(), [INSTRUCTIONS]);
}

#[test]
fn typed_characters_are_echoed_including_space_and_capitals() {
    let (mut h, _) = field();
    h.clear();
    type_text(&mut h, "Jo Ann");
    assert_eq!(h.spoken(), ["cap j", "o", "space", "cap a", "n", "n"]);
}

#[test]
fn left_reviews_back_to_front_and_right_forward_without_changing_the_text() {
    let (mut h, confirmed) = field();
    type_text(&mut h, "Cary");
    h.clear();
    for _ in 0..4 {
        h.press(Key::Left);
    }
    h.press(Key::Right);
    assert_eq!(h.spoken(), ["y", "r", "a", "cap c", "cap c"]);
    h.press(Key::Return);
    assert_eq!(*confirmed.borrow(), ["Cary"]);
}

#[test]
fn home_end_and_up_read_the_ends_and_the_whole_text() {
    let (mut h, _) = field();
    h.clear();
    h.press(Key::Up);
    type_text(&mut h, "Bo");
    h.press(Key::Home);
    h.press(Key::End);
    h.press(Key::Down);
    assert_eq!(
        h.spoken(),
        ["Empty.", "cap b", "o", "Start. cap b", "End. o", "Bo"]
    );
}

#[test]
fn typing_and_backspace_act_where_the_cursor_is() {
    let (mut h, confirmed) = field();
    type_text(&mut h, "Cay");
    h.press(Key::Left);
    type_text(&mut h, "r");
    h.press(Key::Home);
    h.press(Key::Right);
    h.press(Key::Right);
    h.clear();
    h.press(Key::Backspace);
    assert_eq!(h.spoken(), ["Deleted a. Cry"]);
    h.press(Key::Return);
    assert_eq!(*confirmed.borrow(), ["Cry"]);
}

#[test]
fn the_edges_are_quiet_control_characters_are_not_typed_and_a_full_field_says_so() {
    let (mut h, _) = field();
    h.clear();
    h.press(Key::Backspace);
    h.press(Key::Left);
    h.press(Key::Home);
    h.send(InputEvent::key_text(Key::Tab, '\t'));
    assert!(
        h.spoken().is_empty(),
        "nothing to delete, nowhere to go, nothing typed"
    );
    type_text(&mut h, &"x".repeat(24));
    h.clear();
    type_text(&mut h, "y");
    assert_eq!(h.spoken(), ["The field is full."]);
}

#[test]
fn a_controller_confirms_with_a_and_escape_cancels() {
    let (mut h, confirmed) = field();
    h.connect_pad();
    type_text(&mut h, "Al");
    h.send(InputEvent::button(ControllerButton::A));
    assert_eq!(*confirmed.borrow(), ["Al"]);
    assert_eq!(
        h.app.top_name(),
        "text_entry",
        "confirming leaves closing to the game"
    );
    h.press(Key::Escape);
    assert_eq!(h.app.top_name(), "root");
}

fn timed(seconds: f64) -> Headless<TestGame> {
    let mut h = Headless::new(TestData::default(), Probe::new("root"));
    h.app.ctx.push(
        TimedMessage::<TestGame>::new(
            "Loading",
            "Loading the course.",
            "Still loading.",
            seconds,
            |ctx: &mut Ctx<TestGame>| ctx.replace(Probe::new("course")),
        )
        .complete_text("Ready."),
    );
    h.frame(0.0);
    h
}

#[test]
fn a_timed_message_answers_keys_with_its_status_and_moves_on_once() {
    let mut h = timed(1.0);
    assert_eq!(h.spoken(), ["Loading the course."]);
    h.clear();
    h.press(Key::Return);
    h.press(Key::F1);
    h.press(Key::Z);
    assert_eq!(
        h.spoken(),
        ["Still loading.", "Still loading."],
        "a stray key does nothing"
    );
    assert_eq!(h.app.top_name(), "timed_message");
    h.clear();
    h.frame(0.6);
    assert_eq!(h.app.top_lines()[3], "Ready in 0.4 seconds.");
    h.frame(0.6);
    assert_eq!(h.app.top_name(), "course");
    assert_eq!(
        h.speech.borrow().calls(SpeechChannel::Main),
        [("Ready.".to_string(), false)]
    );
}

#[test]
fn a_zero_second_message_moves_on_at_the_first_frame() {
    let h = timed(0.0);
    assert_eq!(h.app.top_name(), "course");
}
