mod common;

use common::{Probe, TestData, TestGame};
use hearken_app::{Headless, HeldKeys, InputEvent, Key};

fn start() -> Headless<TestGame> {
    let mut h = Headless::new(TestData::default(), Probe::new("root"));
    // A known repeat timing, whatever this machine's keyboard settings say.
    h.app.ctx.held = HeldKeys::with_timing(500, 33);
    h
}

#[test]
fn a_held_key_reads_held_until_it_is_released() {
    let mut h = start();
    h.press(Key::Up);
    assert!(h.app.ctx.held.is_pressed(Key::Up));
    h.frame(0.1);
    assert!(h.app.ctx.held.is_pressed(Key::Up));
    h.send(InputEvent::key_up(Key::Up));
    assert!(!h.app.ctx.held.is_pressed(Key::Up));
}

#[test]
fn leaving_the_window_lets_go_of_every_key() {
    let mut h = start();
    h.press(Key::Up);
    h.send(InputEvent::WindowFocusLost);
    assert!(!h.app.ctx.held.is_pressed(Key::Up));
}

#[test]
fn a_screen_readers_re_sent_key_reads_held_but_a_new_screen_never_inherits_it() {
    let mut h = start();
    // JAWS re-sends an arrow as a press and a release inside one frame.
    h.app.step(
        vec![InputEvent::key(Key::Up), InputEvent::key_up(Key::Up)],
        1.0 / 60.0,
    );
    assert!(
        h.app.ctx.held.is_pressed(Key::Up),
        "a pair in one frame is a hold"
    );
    h.app.ctx.push(Probe::new("next"));
    h.frame(1.0 / 60.0);
    assert!(!h.app.ctx.held.is_pressed(Key::Up));
}
