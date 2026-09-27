mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::{take_log, Probe, TestData, TestGame};
use hearken_app::controller::fakes::{fake_factory, FakePad, FakeSdlLog};
use hearken_app::controller::{
    deadzone, ControllerManager, InputDevice, RepeatTimer, CONTROLLER_DISCONNECTED, REPEAT_DELAY_S,
};
use hearken_app::menu::{Label, Menu, MenuCore, MenuItem};
use hearken_app::{ControllerAxis, ControllerButton as B, Ctx, Headless, InputEvent};

fn down(button: B, instance_id: u32) -> InputEvent {
    InputEvent::ControllerButtonDown {
        button,
        instance_id,
        repeat: false,
    }
}

fn up(button: B, instance_id: u32) -> InputEvent {
    InputEvent::ControllerButtonUp {
        button,
        instance_id,
    }
}

fn axis(axis: ControllerAxis, value: i16) -> InputEvent {
    InputEvent::ControllerAxis {
        axis,
        value,
        instance_id: 0,
    }
}

/// A manager with a pad bound as `instance_id`, its id latched.
fn with_pad(instance_id: u32) -> ControllerManager {
    let mut m = ControllerManager::detached(true);
    m.bind_device(Box::new(FakePad::new(instance_id)), "test pad");
    m.set_id_pending(false);
    m
}

fn with_factory(log: &Rc<RefCell<FakeSdlLog>>, enabled: bool) -> ControllerManager {
    ControllerManager::new(
        enabled,
        Some(fake_factory(vec![("Pad".to_string(), 4)], log.clone())),
    )
}

#[test]
fn the_deadzone_is_silent_inside_and_reaches_full_range_at_the_edge() {
    assert_eq!(deadzone(0.05, 0.10), 0.0);
    assert_eq!(deadzone(1.0, 0.10), 1.0);
    assert_eq!(deadzone(-1.0, 0.10), -1.0);
    assert!((deadzone(0.55, 0.10) - 0.5).abs() < 1e-9);
}

#[test]
fn a_held_dpad_side_repeats_after_a_pause_then_faster() {
    let mut t = RepeatTimer::default();
    t.press(B::A);
    assert_eq!(t.tick(1.0), 0, "only the D-pad sides repeat");
    t.press(B::DPadRight);
    assert_eq!(t.tick(REPEAT_DELAY_S - 0.01), 0);
    assert_eq!(t.tick(0.02), 1);
    // Held long enough, the repeats come every REPEAT_FAST_S.
    t.tick(2.0);
    let fast = t.tick(0.3);
    assert!((9..=11).contains(&fast), "{fast}");
    t.release(B::DPadRight);
    assert_eq!(t.tick(1.0), 0);
}

#[test]
fn only_the_bound_pads_presses_get_through_and_a_doubled_press_counts_once() {
    let mut m = with_pad(3);
    assert_eq!(m.device(), InputDevice::Keyboard);
    assert!(m.process_event(&down(B::A, 3)));
    assert!(
        !m.process_event(&down(B::A, 3)),
        "a pad that sends a press twice"
    );
    assert!(m.process_event(&up(B::A, 3)));
    assert!(!m.process_event(&up(B::A, 3)), "a release with no press");
    assert!(!m.process_event(&down(B::A, 9)), "another pad");
    assert_eq!(m.device(), InputDevice::Controller);
    m.note_keyboard();
    assert_eq!(m.device(), InputDevice::Keyboard);
}

#[test]
fn a_freshly_opened_pad_takes_the_id_of_its_first_event() {
    let mut m = ControllerManager::detached(true);
    m.bind_device(Box::new(FakePad::new(1)), "test pad");
    assert!(m.id_pending());
    assert!(
        m.process_event(&down(B::A, 7)),
        "a hot-plug left the opened id stale"
    );
    assert_eq!(m.instance_id(), Some(7));
    assert!(
        !m.process_event(&down(B::B, 1)),
        "the stale id is foreign now"
    );
}

#[test]
fn unplugging_the_bound_pad_is_reported_once_and_lets_go_of_everything() {
    let mut m = with_pad(2);
    m.process_event(&down(B::RightShoulder, 2));
    assert!(m.modifier() && m.is_down(B::RightShoulder));
    assert!(m.process_event(&InputEvent::ControllerRemoved { instance_id: 2 }));
    assert!(m.take_disconnect());
    assert!(!m.take_disconnect());
    assert!(!m.connected());
    assert!(!m.modifier());
    assert!(
        !m.process_event(&InputEvent::ControllerRemoved { instance_id: 5 }),
        "some other pad"
    );
    assert!(!m.take_disconnect());
}

#[test]
fn a_new_pad_is_opened_only_when_none_is_bound() {
    let log = Rc::new(RefCell::new(FakeSdlLog::default()));
    let mut m = with_factory(&log, true);
    assert_eq!(log.borrow().opens, [0], "the pad plugged in at start");
    assert_eq!(m.name(), "Pad");
    m.process_event(&InputEvent::ControllerAdded { device_index: 0 });
    assert_eq!(log.borrow().opens, [0], "already bound and attached");
    m.process_event(&InputEvent::ControllerRemoved { instance_id: 4 });
    m.process_event(&InputEvent::ControllerAdded { device_index: 0 });
    assert_eq!(log.borrow().opens, [0, 0]);
    assert!(m.connected());
}

#[test]
fn a_pad_that_went_away_without_a_word_is_replaced_on_the_next_add() {
    let log = Rc::new(RefCell::new(FakeSdlLog::default()));
    let mut m = with_factory(&log, true);
    let pad = FakePad::new(4);
    let attached = pad.attached.clone();
    m.bind_device(Box::new(pad), "Pad");
    attached.set(false);
    m.process_event(&InputEvent::ControllerAdded { device_index: 0 });
    assert_eq!(log.borrow().opens, [0, 0]);
}

#[test]
fn controllers_off_never_touch_the_subsystem_and_on_brings_it_back() {
    let log = Rc::new(RefCell::new(FakeSdlLog::default()));
    let mut m = with_factory(&log, false);
    assert_eq!(log.borrow().inits, 0);
    assert!(!m.subsystem_up());
    m.set_enabled(true);
    assert_eq!(log.borrow().inits, 1);
    assert!(m.connected());
    m.set_id_pending(false);
    m.set_enabled(false);
    assert!(!m.subsystem_up() && !m.connected());
    assert_eq!(log.borrow().quits, 1);
    assert!(!m.process_event(&down(B::A, 4)));
    m.set_enabled(true);
    assert_eq!(log.borrow().inits, 2);
}

#[test]
fn sticks_and_triggers_read_past_their_deadzones_and_only_while_active() {
    let mut m = with_pad(0);
    m.process_event(&axis(ControllerAxis::LeftX, 1000));
    assert_eq!(
        m.axis(ControllerAxis::LeftX),
        0.0,
        "inside the stick deadzone"
    );
    assert_eq!(m.device(), InputDevice::Keyboard, "drift is not use");
    m.process_event(&axis(ControllerAxis::LeftX, -32768));
    assert_eq!(m.axis(ControllerAxis::LeftX), -1.0);
    assert_eq!(m.device(), InputDevice::Controller);
    m.process_event(&axis(ControllerAxis::TriggerRight, 32767));
    assert_eq!(m.axis(ControllerAxis::TriggerRight), 1.0);
    m.set_enabled(false);
    assert_eq!(m.axis(ControllerAxis::LeftX), 0.0);
}

/// A level that Left and Right adjust, and a second row.
struct Rows {
    menu: MenuCore<Self, TestGame>,
    level: i32,
}

impl Rows {
    fn new() -> Self {
        Rows {
            menu: MenuCore::new("Rows"),
            level: 0,
        }
    }
}

impl Menu<TestGame> for Rows {
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
            MenuItem::new(
                Label::dynamic(|s: &Rows, _ctx: &Ctx<TestGame>| format!("Level {}", s.level)),
                |_s: &mut Rows, ctx: &mut Ctx<TestGame>| ctx.say("chosen"),
            )
            .adjustable(|s: &mut Rows, _ctx: &mut Ctx<TestGame>, delta| s.level += delta),
            MenuItem::inert("Other"),
        ]
    }
}

/// `screen` over a root screen, a pad bound, nothing spoken yet.
fn with_pad_over_root(screen: Rows) -> Headless<TestGame> {
    let mut h = Headless::new(TestData::default(), Probe::new("root"));
    h.connect_pad();
    h.app.ctx.push(screen);
    h.frame(0.0);
    h.clear();
    h
}

#[test]
fn a_controller_drives_a_menu_like_the_arrow_keys() {
    let mut h = with_pad_over_root(Rows::new());
    h.send(InputEvent::button(B::DPadDown));
    h.send(InputEvent::button(B::DPadUp));
    h.send(InputEvent::button(B::DPadRight));
    h.send(InputEvent::button(B::A));
    assert_eq!(
        h.spoken(),
        [
            "Other. 2 of 2.",
            "Level 0. 1 of 2.",
            "Level 1. 1 of 2.",
            "chosen"
        ]
    );
    h.send(InputEvent::button(B::B));
    assert_eq!(h.app.top_name(), "root");
}

#[test]
fn the_controllers_back_button_reads_the_help() {
    let mut h = Headless::new(TestData::default(), Probe::new("root"));
    h.connect_pad();
    h.send(InputEvent::button(B::Back));
    assert_eq!(h.spoken(), ["root help"]);
}

#[test]
fn without_a_bound_pad_no_button_reaches_a_screen() {
    let mut h = Headless::new(TestData::default(), Probe::new("root"));
    take_log(&mut h.app.ctx);
    h.send(InputEvent::button(B::A));
    assert!(take_log(&mut h.app.ctx).is_empty());
}

#[test]
fn unplugging_the_pad_says_so_and_the_screen_hears_it_too() {
    let mut h = Headless::new(TestData::default(), Probe::new("root"));
    h.connect_pad();
    take_log(&mut h.app.ctx);
    h.send(InputEvent::ControllerRemoved { instance_id: 0 });
    assert_eq!(h.spoken(), [CONTROLLER_DISCONNECTED]);
    assert_eq!(
        take_log(&mut h.app.ctx),
        ["root input ControllerRemoved { instance_id: 0 }"]
    );
    h.clear();
    h.send(InputEvent::ControllerRemoved { instance_id: 0 });
    assert!(h.spoken().is_empty(), "said once");
}

#[test]
fn holding_the_dpad_right_keeps_adjusting_until_it_is_let_go() {
    let mut h = with_pad_over_root(Rows::new());
    h.send(InputEvent::button(B::DPadRight));
    h.frame(0.5);
    h.send(InputEvent::button_up(B::DPadRight));
    h.frame(1.0);
    assert_eq!(h.spoken(), ["Level 1. 1 of 2.", "Level 2. 1 of 2."]);
}

#[test]
fn leaving_the_window_stops_a_held_dpad_and_the_next_press_still_works() {
    let mut h = with_pad_over_root(Rows::new());
    h.send(InputEvent::button(B::DPadRight));
    // SDL drops the release while the window is in the background.
    h.send(InputEvent::WindowFocusLost);
    h.frame(2.0);
    assert_eq!(h.spoken(), ["Level 1. 1 of 2."], "no repeats while away");
    h.send(InputEvent::button(B::DPadRight));
    assert_eq!(
        h.spoken().last().map(String::as_str),
        Some("Level 2. 1 of 2."),
        "the press after coming back is not taken for a duplicate"
    );
}
