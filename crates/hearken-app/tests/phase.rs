//! Named phases (`Phases`) and the record of state changes every run
//! keeps: what a test, a recording and the agent server's `screen` tool
//! can read that a player never hears.

mod common;

use common::{TestData, TestGame};
use hearken_app::screens::PauseScreen;
use hearken_app::{Ctx, Headless, InputEvent, Key, Phase, Phases, State, StatusKey};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Round {
    Ready,
    Playing,
    Ending,
    Done,
}

impl Phase for Round {
    fn name(self) -> &'static str {
        match self {
            Round::Ready => "ready",
            Round::Playing => "playing",
            Round::Ending => "ending",
            Round::Done => "done",
        }
    }
}

/// Space starts the round, a second Space ends it after a two-second
/// pause, and S reads the status in layers.
struct Play {
    phases: Phases<Round>,
    status: StatusKey,
    hits: u32,
}

impl Play {
    fn new() -> Self {
        Play {
            phases: Phases::new(Round::Ready),
            status: StatusKey::new(),
            hits: 0,
        }
    }
}

fn to_title(_ctx: &mut Ctx<TestGame>) {}

impl State<TestGame> for Play {
    fn name(&self) -> &'static str {
        "play"
    }
    fn phase(&self) -> Option<&'static str> {
        Some(self.phases.name())
    }
    fn enter(&mut self, ctx: &mut Ctx<TestGame>) {
        ctx.say("Ready.");
    }
    fn handle_input(&mut self, ctx: &mut Ctx<TestGame>, event: &InputEvent) {
        match event.pressed() {
            Some(Key::Space) if self.phases.is(Round::Ready) => {
                if self.phases.set(ctx, Round::Playing) {
                    ctx.say("Go.");
                }
            }
            Some(Key::Space) if self.phases.is(Round::Playing) => {
                self.hits += 1;
                if self.hits == 2 && self.phases.set_for(ctx, Round::Ending, 2.0) {
                    ctx.say("Last one.");
                }
            }
            Some(Key::S) => {
                let layers = vec![
                    format!("{} hits.", self.hits),
                    String::new(),
                    format!("Phase {}.", self.phases.name()),
                ];
                self.status.speak(ctx, layers);
            }
            Some(Key::Escape) => ctx.push(PauseScreen::new("", to_title)),
            _ => {}
        }
    }
    fn update(&mut self, ctx: &mut Ctx<TestGame>, dt: f64) {
        if self.phases.tick(dt) && self.phases.set(ctx, Round::Done) {
            ctx.say("Done.");
        }
    }
    fn help(&self, _ctx: &Ctx<TestGame>) -> String {
        String::new()
    }
}

fn start() -> Headless<TestGame> {
    let mut h = Headless::new(TestData::default(), Play::new());
    h.clear();
    h
}

#[test]
fn a_phase_changes_once_and_is_logged_where_it_changes() {
    let mut h = start();
    assert_eq!(h.phase(), Some("ready"));
    h.press(Key::Space);
    assert_eq!(h.phase(), Some("playing"));
    assert_eq!(h.spoken(), ["Go."]);
    assert_eq!(h.transitions(), ["play: ready -> playing"]);
    // Setting the phase it is already in says and logs nothing.
    assert!(!h.app.ctx.changes.is_empty());
    let before = h.app.ctx.changed;
    h.press(Key::Space);
    assert_eq!(h.app.ctx.changed, before);
}

#[test]
fn a_timed_phase_runs_out_on_game_time_and_the_screen_decides_what_follows() {
    let mut h = start();
    h.press(Key::Space);
    h.press(Key::Space);
    h.press(Key::Space);
    assert_eq!(h.phase(), Some("ending"));
    h.frame(1.5);
    assert_eq!(h.phase(), Some("ending"));
    h.frame(0.6);
    assert_eq!(h.phase(), Some("done"));
    assert_eq!(h.spoken(), ["Go.", "Last one.", "Done."]);
    assert_eq!(
        h.transitions(),
        [
            "play: ready -> playing",
            "play: playing -> ending",
            "play: ending -> done"
        ]
    );
    // Done for good: more frames change nothing.
    h.clear();
    h.frame(5.0);
    assert!(h.transitions().is_empty());
    assert!(h.spoken().is_empty());
}

#[test]
fn seconds_in_a_phase_follow_the_clock() {
    let mut h = start();
    h.frame(1.0);
    h.press(Key::Space);
    let mut probe = Phases::new(Round::Ready);
    assert_eq!(probe.seconds_in(&h.app.ctx), 1.0);
    probe.set(&mut h.app.ctx, Round::Playing);
    h.frame(0.5);
    assert!((probe.seconds_in(&h.app.ctx) - 0.5).abs() < 1e-9);
    assert_eq!(probe.time_left(), None);
    probe.set_for(&mut h.app.ctx, Round::Ending, 3.0);
    assert_eq!(probe.time_left(), Some(3.0));
    assert!(!probe.tick(1.0));
    assert!(probe.tick(2.0));
    assert!(!probe.tick(1.0), "a timer reports once");
}

#[test]
fn screen_changes_are_logged_and_the_stack_is_readable() {
    let mut h = Headless::new(TestData::default(), Play::new());
    assert_eq!(h.screens(), ["start on play"]);
    assert_eq!(h.app.stack_names(), ["play"]);
    h.clear();
    h.press(Key::Escape);
    assert_eq!(h.screens(), ["push pause over play"]);
    assert_eq!(h.app.stack_names(), ["play", "pause"]);
    assert_eq!(h.phase(), None, "a menu has no phase");
    assert_eq!(h.app.ctx.top_screen(), "pause");
    h.clear();
    h.press(Key::Return); // Resume
    assert_eq!(h.screens(), ["pop pause", "play resumes"]);
    assert_eq!(h.app.ctx.top_screen(), "play");
    // The record is kept in order, both kinds together.
    h.clear();
    h.press(Key::Space);
    h.press(Key::Escape);
    assert_eq!(
        h.changes(),
        [
            "[phase] play: ready -> playing",
            "[screen] push pause over play"
        ]
    );
}

#[test]
fn nothing_in_the_record_is_spoken() {
    let mut h = start();
    h.press(Key::Space);
    h.press(Key::Escape);
    h.press(Key::Escape);
    for line in h.app.ctx.transcript.iter() {
        assert!(
            !line.contains("[phase]") && !line.contains("[screen]"),
            "{line}"
        );
    }
}

#[test]
fn the_status_key_reads_layers_then_starts_over() {
    let mut h = start();
    h.press(Key::Space);
    h.clear();
    h.press(Key::S);
    h.press(Key::S);
    h.press(Key::S);
    assert_eq!(h.spoken(), ["0 hits.", "Phase playing.", "0 hits."]);
    h.clear();
    h.frame(hearken_app::status_key::AGAIN_SECONDS + 1.0);
    h.press(Key::S);
    h.app.ctx.say("Something else.");
    h.press(Key::S);
    assert_eq!(h.spoken(), ["0 hits.", "Something else.", "0 hits."]);
    h.clear();
    h.frame(hearken_app::status_key::AGAIN_SECONDS + 1.0);
    h.press(Key::S);
    h.frame(hearken_app::status_key::AGAIN_SECONDS + 1.0);
    h.press(Key::S);
    assert_eq!(h.spoken(), ["0 hits.", "0 hits."]);
}

#[test]
fn sentences_skip_empty_parts() {
    assert_eq!(hearken_app::text::sentences(["A.", "", "B."]), "A. B.");
}
