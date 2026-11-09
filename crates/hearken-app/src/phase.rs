//! Named phases for a screen, changed in one place.
//!
//! A play screen passes through phases: waiting for the first key, playing,
//! a pause before the results, finished. Kept as booleans and timers
//! (`started`, `over`, `ending: Option<f64>`) they are rebuilt from flags in
//! `update`, `handle_input`, `resume` and the status key, and nothing
//! records when one became another. A [`Phases`] holds the current phase
//! and is the one place it changes: [`Phases::set`] logs the change as
//! `[phase] play: ready -> playing`, keeps it in [`Ctx::changes`] for the
//! agent server, Headless tests and play-test recordings, and tells the
//! caller the phase changed, so the line for the new phase is spoken there,
//! once, and nowhere else.
//!
//! ```ignore
//! #[derive(Clone, Copy, Debug, PartialEq, Eq)]
//! enum Hall { Dark, Lit, Ending, Done }
//!
//! impl Phase for Hall {
//!     fn name(self) -> &'static str {
//!         match self {
//!             Hall::Dark => "dark",
//!             Hall::Lit => "lit",
//!             Hall::Ending => "ending",
//!             Hall::Done => "done",
//!         }
//!     }
//! }
//!
//! // In the screen: `phases: Phases<Hall>`, made with `Phases::new(Hall::Dark)`.
//! if self.phases.set(ctx, Hall::Lit) {
//!     ctx.say("Taper lit.");
//! }
//! // A phase that ends by itself: two seconds, then the results.
//! self.phases.set_for(ctx, Hall::Ending, 2.0);
//! // Once a frame:
//! if self.phases.tick(dt) {
//!     self.finish(ctx);
//! }
//! ```
//!
//! The screen reports its phase through [`crate::State::phase`], which is
//! how the agent server's `screen` tool and `Headless::phase` read it. The
//! tracker never picks a phase itself: what follows a timed phase is the
//! screen's decision, in `update`, where it can speak.

use crate::ctx::Ctx;
use crate::game::Game;

/// A named phase of a screen or a run. The name is a stable id for logs,
/// tests and the agent server: lowercase, such as `"ready"` or `"ending"`.
pub trait Phase: Copy + Eq + std::fmt::Debug + 'static {
    fn name(self) -> &'static str;
}

/// The current phase, and the one place it changes.
#[derive(Clone, Debug)]
pub struct Phases<P: Phase> {
    current: P,
    /// `ctx.clock` when the current phase began; 0 for the first.
    since: f64,
    /// Seconds left of a timed phase, from [`Phases::set_for`].
    timer: Option<f64>,
}

impl<P: Phase> Phases<P> {
    /// Starts in `start`, which is not logged: a screen is made before it
    /// is entered, and its first phase is where it begins.
    pub fn new(start: P) -> Self {
        Phases {
            current: start,
            since: 0.0,
            timer: None,
        }
    }

    pub fn get(&self) -> P {
        self.current
    }

    pub fn is(&self, phase: P) -> bool {
        self.current == phase
    }

    /// The current phase's name, for [`crate::State::phase`].
    pub fn name(&self) -> &'static str {
        self.current.name()
    }

    /// Moves to `next`. The same phase again does nothing and returns
    /// false. Otherwise the change is logged and noted on `ctx`, any timer
    /// is dropped, and true comes back so the caller speaks the new phase's
    /// line exactly once, where the change is.
    pub fn set<G: Game>(&mut self, ctx: &mut Ctx<G>, next: P) -> bool {
        if next == self.current {
            return false;
        }
        let from = self.current.name();
        self.current = next;
        self.since = ctx.clock;
        self.timer = None;
        ctx.note_phase(from, next.name());
        true
    }

    /// `set`, and the phase ends by itself after `seconds` of game time:
    /// [`Phases::tick`] reports the frame it runs out. Called again on the
    /// same phase it only restarts the timer. Replaces a screen's own
    /// `ending: Option<f64>`.
    pub fn set_for<G: Game>(&mut self, ctx: &mut Ctx<G>, next: P, seconds: f64) -> bool {
        let changed = self.set(ctx, next);
        self.timer = Some(seconds.max(0.0));
        changed
    }

    /// Once a frame, from `update`: true on the frame a timed phase runs
    /// out, once. The screen then decides what comes next.
    pub fn tick(&mut self, dt: f64) -> bool {
        let Some(left) = self.timer.as_mut() else {
            return false;
        };
        *left -= dt;
        if *left > 0.0 {
            return false;
        }
        self.timer = None;
        true
    }

    /// Seconds left of a timed phase, or `None` when the current phase has
    /// no timer.
    pub fn time_left(&self) -> Option<f64> {
        self.timer.map(|t| t.max(0.0))
    }

    /// Seconds spent in the current phase, on `ctx.clock`.
    pub fn seconds_in<G: Game>(&self, ctx: &Ctx<G>) -> f64 {
        (ctx.clock - self.since).max(0.0)
    }
}
