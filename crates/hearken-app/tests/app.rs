mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::{take_log, Probe, TestData, TestGame};
use hearken_app::{App, Ctx, Headless, HeadlessShell, InputEvent, Key, Paths, Settings, State};
use hearken_speech::{CaptureSpeech, SharedCapture};

fn headless(root: Probe) -> Headless<TestGame> {
    let mut h = Headless::new(TestData::default(), root);
    take_log(&mut h.app.ctx);
    h
}

#[test]
fn push_enters_after_the_handler_returns() {
    let mut h = headless(Probe::new("root").on_return(|ctx| ctx.push(Probe::new("b"))));
    h.press(Key::Return);
    assert_eq!(take_log(&mut h.app.ctx), ["root key Return", "b enter"]);
    assert_eq!((h.app.top_name(), h.app.depth()), ("b", 2));
}

#[test]
fn pop_exits_and_resumes_the_screen_below() {
    let mut h = headless(Probe::new("root"));
    h.app.ctx.push(Probe::new("b").on_return(|ctx| ctx.pop()));
    h.frame(0.0);
    take_log(&mut h.app.ctx);
    h.press(Key::Return);
    assert_eq!(
        take_log(&mut h.app.ctx),
        ["b key Return", "b exit", "root resume"]
    );
    assert_eq!(h.app.depth(), 1);
}

#[test]
fn replace_exits_then_enters_without_resuming_the_screen_below() {
    let mut h = headless(Probe::new("root"));
    h.app
        .ctx
        .push(Probe::new("b").on_return(|ctx| ctx.replace(Probe::new("c"))));
    h.frame(0.0);
    take_log(&mut h.app.ctx);
    h.press(Key::Return);
    assert_eq!(
        take_log(&mut h.app.ctx),
        ["b key Return", "b exit", "c enter"]
    );
    assert_eq!((h.app.top_name(), h.app.depth()), ("c", 2));
}

#[test]
fn reset_exits_every_screen_top_down_then_enters() {
    let mut h = headless(Probe::new("root"));
    h.app.ctx.push(Probe::new("b"));
    h.app
        .ctx
        .push(Probe::new("c").on_return(|ctx| ctx.reset_to(Probe::new("d"))));
    h.frame(0.0);
    take_log(&mut h.app.ctx);
    h.press(Key::Return);
    assert_eq!(
        take_log(&mut h.app.ctx),
        ["c key Return", "c exit", "b exit", "root exit", "d enter"]
    );
    assert_eq!(h.app.depth(), 1);
}

#[test]
fn popping_the_root_and_pushing_in_the_same_handler_keeps_running() {
    let mut h = headless(Probe::new("root").on_return(|ctx| {
        ctx.pop();
        ctx.push(Probe::new("next"));
    }));
    h.press(Key::Return);
    assert!(h.app.ctx.running);
    assert_eq!(h.app.top_name(), "next");
}

#[test]
fn popping_the_last_screen_ends_the_run() {
    let mut h = headless(Probe::new("root").on_return(|ctx| ctx.pop()));
    h.press(Key::Return);
    assert!(!h.app.ctx.running);
    assert_eq!(h.app.depth(), 0);
}

struct Runaway;

impl State<TestGame> for Runaway {
    fn name(&self) -> &'static str {
        "runaway"
    }
    fn enter(&mut self, ctx: &mut Ctx<TestGame>) {
        ctx.push(Runaway);
    }
    fn help(&self, _ctx: &Ctx<TestGame>) -> String {
        String::new()
    }
}

#[test]
fn a_screen_that_keeps_pushing_itself_is_cut_off_after_sixteen_rounds() {
    let h = Headless::new(TestData::default(), Runaway);
    assert!(h.app.ctx.running);
    assert_eq!(h.app.depth(), 17, "the first screen plus sixteen rounds");
}

#[test]
fn a_quit_event_ends_the_run() {
    let mut h = headless(Probe::new("root"));
    h.send(InputEvent::Quit);
    assert!(!h.app.ctx.running);
}

#[test]
fn f2_stops_speech_and_the_screen_never_sees_it() {
    let mut h = headless(Probe::new("root"));
    h.press(Key::F2);
    assert_eq!(h.speech.borrow().stop_calls(), 1);
    assert!(take_log(&mut h.app.ctx).is_empty());
}

#[test]
fn f1_speaks_the_top_screen_help_and_the_screen_never_sees_it() {
    let mut h = headless(Probe::new("root"));
    h.clear();
    h.press(Key::F1);
    assert_eq!(h.spoken(), ["root help"]);
    assert!(take_log(&mut h.app.ctx).is_empty());
}

#[test]
fn game_events_are_observed_then_reach_only_the_top_screen() {
    let mut h = headless(Probe::new("root"));
    h.app.ctx.push(Probe::new("b"));
    h.frame(0.0);
    take_log(&mut h.app.ctx);
    h.app.ctx.game.queued_events = vec![7, 8];
    h.frame(0.0);
    assert_eq!(
        take_log(&mut h.app.ctx),
        ["observe 7", "b event 7", "observe 8", "b event 8"]
    );
}

#[test]
fn only_the_top_screen_updates() {
    let mut h = headless(Probe::new("root"));
    h.app.ctx.push(Probe::new("b"));
    h.frame(0.0);
    h.app.ctx.game.log.clear();
    h.frame(0.016);
    assert_eq!(h.app.ctx.game.log, ["b update"]);
}

#[test]
fn speech_is_recorded_in_the_transcript_and_blank_lines_are_dropped() {
    let mut h = headless(Probe::new("root"));
    h.clear();
    h.app.ctx.say("Hello.");
    h.app.ctx.say("   ");
    h.app.ctx.say_event("Watch out.", false);
    assert_eq!(h.spoken(), ["Hello."]);
    assert_eq!(h.speech.borrow().event_lines(), ["Watch out."]);
    let tail: Vec<_> = h
        .app
        .ctx
        .transcript
        .iter()
        .rev()
        .take(2)
        .rev()
        .cloned()
        .collect();
    assert_eq!(tail, ["Hello.", "[event] Watch out."]);
}

#[test]
fn the_transcript_keeps_the_newest_lines() {
    let mut h = headless(Probe::new("root"));
    for i in 0..(hearken_app::TRANSCRIPT_CAP + 5) {
        h.app.ctx.say(format!("line {i}"));
    }
    assert_eq!(h.app.ctx.transcript.len(), hearken_app::TRANSCRIPT_CAP);
    assert_eq!(
        h.app.ctx.transcript.last().map(String::as_str),
        Some(format!("line {}", hearken_app::TRANSCRIPT_CAP + 4).as_str())
    );
}

struct Announcer;

impl State<TestGame> for Announcer {
    fn name(&self) -> &'static str {
        "announcer"
    }
    fn enter(&mut self, ctx: &mut Ctx<TestGame>) {
        ctx.say("Main menu.");
    }
    fn help(&self, _ctx: &Ctx<TestGame>) -> String {
        String::new()
    }
}

#[test]
fn notices_are_spoken_after_the_first_screen_announces_itself() {
    let capture = Rc::new(RefCell::new(CaptureSpeech::new()));
    let dir = tempfile::tempdir().unwrap();
    let mut ctx: Ctx<TestGame> = Ctx::new(
        Box::new(SharedCapture(capture.clone())),
        Settings::default(),
        Paths::at(dir.path()),
        TestData::default(),
    );
    ctx.notices.push("Your settings were reset.".into());
    let _app = App::new(ctx, Box::new(Announcer));
    assert_eq!(
        capture.borrow().calls(hearken_speech::SpeechChannel::Main),
        [
            ("Main menu.".to_string(), true),
            ("Your settings were reset.".to_string(), false)
        ]
    );
}

#[test]
fn run_ends_when_a_screen_quits_then_saves_settings_and_shuts_speech_down() {
    let mut h = headless(Probe::new("root").on_return(|ctx| ctx.quit()));
    let mut shell = HeadlessShell::new();
    shell.push(InputEvent::key(Key::Return));
    h.app.run(&mut shell);
    assert!(!h.app.ctx.running);
    assert!(h.app.ctx.paths.settings_file().is_file());
    assert_eq!(h.speech.borrow().shutdown_calls(), 1);
    assert_eq!(
        shell.rendered.last().map(Vec::as_slice),
        Some(&["root".to_string()][..])
    );
}
