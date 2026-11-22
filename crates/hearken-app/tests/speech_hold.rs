mod common;

use common::*;
use hearken_app::{Ctx, Headless, PacerConfig, SpeechHold, SpeechHoldState, State};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const LINE: &str = "Three customers wait at the counter, the first one in a hurry.";

fn start() -> Headless<TestGame> {
    Headless::new(TestData::default(), Probe::new("root"))
}

/// A screen whose countdown runs through a `SpeechHold`, as a game's does.
struct Countdown {
    hold: Rc<RefCell<SpeechHold>>,
    /// Game time the hold let through, in total and on the last frame.
    total: Rc<Cell<f64>>,
    last: Rc<Cell<f64>>,
}

impl State<TestGame> for Countdown {
    fn name(&self) -> &'static str {
        "countdown"
    }
    fn update(&mut self, ctx: &mut Ctx<TestGame>, dt: f64) {
        let flowed = self.hold.borrow_mut().dt(ctx, dt);
        self.last.set(flowed);
        self.total.set(self.total.get() + flowed);
    }
    fn lines(&self, _ctx: &Ctx<TestGame>) -> Vec<String> {
        Vec::new()
    }
    fn help(&self, _ctx: &Ctx<TestGame>) -> String {
        String::new()
    }
}

struct Timed {
    h: Headless<TestGame>,
    hold: Rc<RefCell<SpeechHold>>,
    total: Rc<Cell<f64>>,
    last: Rc<Cell<f64>>,
}

fn countdown() -> Timed {
    let hold = Rc::new(RefCell::new(SpeechHold::new()));
    let total = Rc::new(Cell::new(0.0));
    let last = Rc::new(Cell::new(0.0));
    let screen = Countdown {
        hold: hold.clone(),
        total: total.clone(),
        last: last.clone(),
    };
    Timed {
        h: Headless::new(TestData::default(), screen),
        hold,
        total,
        last,
    }
}

fn duration(text: &str) -> f64 {
    PacerConfig::DEFAULT.duration_s(text)
}

fn busy(h: &mut Headless<TestGame>) -> bool {
    h.app.ctx.speech_busy()
}

/// Frames of `step` seconds until `seconds` have passed.
fn run(h: &mut Headless<TestGame>, seconds: f64, step: f64) {
    let end = h.app.ctx.clock + seconds;
    while h.app.ctx.clock + 1e-9 < end {
        h.frame(step);
    }
}

#[test]
fn nothing_said_is_not_busy() {
    let mut h = start();
    assert!(!busy(&mut h));
}

#[test]
fn a_line_on_the_main_channel_is_busy_for_its_estimate() {
    let mut h = start();
    h.app.ctx.say(LINE);
    assert!(busy(&mut h));
    h.frame(duration(LINE) - 0.05);
    assert!(busy(&mut h));
    h.frame(0.1);
    assert!(!busy(&mut h));
}

#[test]
fn an_interrupting_line_starts_over_and_a_queued_line_waits_behind() {
    let mut h = start();
    h.app.ctx.say(LINE);
    h.app.ctx.say("Next.");
    h.frame(duration("Next.") + 0.05);
    assert!(!busy(&mut h), "the interrupt cut the long line");

    h.app.ctx.say(LINE);
    h.app.ctx.say_queued("Next.");
    h.frame(duration(LINE) + 0.05);
    assert!(busy(&mut h), "the queued line still plays");
    h.frame(duration("Next."));
    assert!(!busy(&mut h));
}

#[test]
fn stop_speech_clears_it() {
    let mut h = start();
    h.app.ctx.say(LINE);
    h.app.ctx.say_event(LINE, false);
    h.app.ctx.stop_speech();
    assert!(!busy(&mut h));
}

#[test]
fn a_preview_counts_and_a_blank_line_does_not() {
    let mut h = start();
    h.app.ctx.say("   ");
    h.app.ctx.say_queued("");
    assert!(!busy(&mut h));
    h.app.ctx.say_preview("speech_rate", "Rate 50.");
    assert!(busy(&mut h));
}

#[test]
fn event_speech_is_busy_too() {
    let mut t = countdown();
    t.h.app.ctx.say_event(LINE, false);
    assert!(
        t.h.spoken().is_empty(),
        "on the event voice, not the main one"
    );
    t.h.frame(0.1);
    assert_eq!(t.last.get(), 0.0);
    run(&mut t.h, duration(LINE), 0.1);
    t.h.frame(0.1);
    assert_eq!(t.last.get(), 0.1);
}

#[test]
fn a_silent_report_after_the_start_grace_ends_it_early() {
    let mut h = start();
    h.speech.borrow_mut().set_voice_speaking(Some(false));
    busy(&mut h);
    h.app.ctx.say(LINE);
    h.frame(0.1);
    h.frame(0.1);
    assert!(
        busy(&mut h),
        "inside the grace the voice may not have started"
    );
    run(&mut h, 0.5, 0.1);
    assert!(!busy(&mut h));
    assert!(h.app.ctx.clock < duration(LINE) / 2.0);
}

#[test]
fn a_speaking_report_holds_past_the_estimate_until_it_falls_silent() {
    let mut h = start();
    h.speech.borrow_mut().set_voice_speaking(Some(true));
    busy(&mut h);
    h.app.ctx.say("Hi.");
    run(&mut h, duration("Hi.") + 2.0, 0.1);
    assert!(busy(&mut h), "the voice says it is still speaking");
    h.speech.borrow_mut().set_voice_speaking(Some(false));
    h.frame(0.1);
    assert!(!busy(&mut h));
}

#[test]
fn a_voice_that_cannot_report_leaves_the_estimate() {
    let mut h = start();
    busy(&mut h);
    h.app.ctx.say(LINE);
    run(&mut h, duration(LINE) - 0.2, 0.1);
    assert!(busy(&mut h));
    run(&mut h, 0.3, 0.1);
    assert!(!busy(&mut h));
    assert!(h.speech.borrow().voice_reports() > 0, "the voice was asked");
}

#[test]
fn the_voice_is_asked_only_by_a_game_that_waits_and_only_after_speech() {
    let mut h = start();
    h.app.ctx.say(LINE);
    run(&mut h, 1.0, 0.1);
    assert_eq!(h.speech.borrow().voice_reports(), 0, "no game asked");

    let mut h = start();
    busy(&mut h);
    run(&mut h, 1.0, 0.1);
    assert_eq!(h.speech.borrow().voice_reports(), 0, "nothing was said");
    h.app.ctx.say("Hi.");
    run(&mut h, 1.0, 0.1);
    let asked = h.speech.borrow().voice_reports();
    assert!(asked > 0);
    run(&mut h, 10.0, 0.1);
    let later = h.speech.borrow().voice_reports();
    run(&mut h, 1.0, 0.1);
    assert_eq!(
        h.speech.borrow().voice_reports(),
        later,
        "long quiet: not asked"
    );
}

#[test]
fn a_speech_hold_stops_the_clock_then_lets_it_run() {
    let mut t = countdown();
    t.h.frame(0.1);
    assert_eq!(t.last.get(), 0.1, "nothing said: time runs");
    t.h.app.ctx.say(LINE);
    let before = t.total.get();
    t.h.frame(0.1);
    assert_eq!(t.last.get(), 0.0);
    assert!(t.hold.borrow().held());
    run(&mut t.h, duration(LINE) + 0.5, 0.1);
    let ran = t.total.get() - before;
    assert!(ran > 0.0 && ran < 0.7, "held, then ran: {ran}");
    assert!(!t.hold.borrow().held());
    assert_eq!(t.hold.borrow().state(), SpeechHoldState::Flowing);
}

#[test]
fn the_longest_hold_lets_time_run_and_holds_again_only_after_quiet() {
    let mut t = countdown();
    t.h.speech.borrow_mut().set_voice_speaking(Some(true));
    t.h.app.ctx.say("Hi.");
    let frames = 300;
    run(&mut t.h, frames as f64 * 0.1, 0.1);
    let held = frames as f64 * 0.1 - t.total.get();
    let max = t.hold.borrow().max_hold_s;
    assert!((held - max).abs() < 1e-6, "held {held} s");
    assert_eq!(t.hold.borrow().state().name(), "expired");
    assert!(busy(&mut t.h), "still talking");
    t.h.frame(0.1);
    assert_eq!(t.last.get(), 0.1);

    t.h.speech.borrow_mut().set_voice_speaking(Some(false));
    t.h.frame(0.1);
    assert_eq!(t.last.get(), 0.1);
    assert_eq!(
        t.hold.borrow().state().name(),
        "expired",
        "not quiet long enough"
    );
    let rearm = t.hold.borrow().rearm_quiet_s;
    run(&mut t.h, rearm, 0.1);
    assert_eq!(t.hold.borrow().state(), SpeechHoldState::Flowing);
    t.h.app.ctx.say("Again.");
    t.h.frame(0.1);
    assert_eq!(t.last.get(), 0.0);
    assert!(t.hold.borrow().held());
}

/// A voice that keeps saying it is speaking, in reports `age` old, with the
/// countdown's hold counting how often it let time through.
fn never_quiet(t: &mut Timed, frames: usize) -> usize {
    let mut leaks = 0;
    for _ in 0..frames {
        t.h.frame(0.1);
        if t.last.get() > 0.0 {
            leaks += 1;
        }
        assert!(
            busy(&mut t.h),
            "read not busy between frames at {}",
            t.h.app.ctx.clock
        );
    }
    leaks
}

#[test]
fn old_speaking_reports_never_read_as_quiet_on_the_main_channel() {
    let mut t = countdown();
    t.hold.borrow_mut().max_hold_s = 1000.0;
    {
        let mut speech = t.h.speech.borrow_mut();
        speech.set_voice_speaking(Some(true));
        speech.set_voice_report_age(std::time::Duration::from_millis(300));
    }
    t.h.app.ctx.say("Hi.");
    assert_eq!(never_quiet(&mut t, 100), 0);
    t.h.speech.borrow_mut().set_voice_speaking(Some(false));
    t.h.frame(0.1);
    assert!(!busy(&mut t.h));
}

#[test]
fn old_speaking_reports_never_read_as_quiet_on_the_event_channel() {
    let mut t = countdown();
    t.hold.borrow_mut().max_hold_s = 1000.0;
    {
        let mut speech = t.h.speech.borrow_mut();
        speech.set_voice_speaking(Some(true));
        speech.set_voice_report_age(std::time::Duration::from_millis(300));
    }
    t.h.app.ctx.say_event("Hi.", false);
    assert_eq!(never_quiet(&mut t, 100), 0);
}

#[test]
fn a_voice_that_stops_reporting_falls_back_to_the_estimate() {
    let mut t = countdown();
    t.h.speech.borrow_mut().set_voice_speaking(Some(true));
    t.h.app.ctx.say("Hi.");
    run(&mut t.h, 5.0, 0.1);
    assert!(busy(&mut t.h));
    t.h.speech.borrow_mut().set_voice_speaking(None);
    run(&mut t.h, 1.5, 0.1);
    assert!(!busy(&mut t.h), "no fresh report: the estimate, long over");
}

#[test]
fn a_blip_of_quiet_does_not_restart_the_longest_hold() {
    let mut t = countdown();
    t.h.speech.borrow_mut().set_voice_speaking(Some(true));
    t.h.app.ctx.say("Hi.");
    run(&mut t.h, 10.0, 0.1);
    // One frame of silence, then the voice is reading again.
    t.h.speech.borrow_mut().set_voice_speaking(Some(false));
    t.h.frame(0.1);
    t.h.speech.borrow_mut().set_voice_speaking(Some(true));
    t.h.app.ctx.say("Hi.");
    let before = t.total.get();
    run(&mut t.h, 20.0, 0.1);
    let ran = t.total.get() - before;
    assert!(ran > 9.0, "the cap counted on from 10 s, ran {ran}");
}
