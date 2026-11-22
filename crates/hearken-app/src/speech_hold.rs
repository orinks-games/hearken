//! A game clock that waits for speech.
//!
//! A countdown the player must hear a line to beat (a customer's patience,
//! a fuse, a deadline) runs out sooner for a player with a slow speech rate:
//! the same line takes them longer to hear. [`SpeechHold`] stops such a
//! clock while speech is playing ([`Ctx::speech_busy`]) and lets it run
//! once the voice is quiet:
//!
//! ```ignore
//! // On the screen: `hold: SpeechHold`, made with `SpeechHold::new()`.
//! fn update(&mut self, ctx: &mut Ctx<G>, dt: f64) {
//!     self.patience -= self.hold.dt(ctx, dt);
//! }
//! ```
//!
//! A hold lasts at most `max_hold_s` of real frame time, so a voice that
//! never reports silence, or a game that never stops talking, cannot freeze
//! the clock. After that the clock runs until speech has been quiet for
//! `rearm_quiet_s`, then holds again. A shorter quiet gap lets time run but
//! does not start the count over, so a voice that is nearly always talking
//! still lets the clock go after `max_hold_s` in all.

use crate::ctx::Ctx;
use crate::game::Game;

/// The longest a hold lasts by default, in seconds.
pub const MAX_HOLD_S: f64 = 20.0;

/// How long speech must be quiet, by default, before a hold starts its
/// count over, in seconds.
pub const REARM_QUIET_S: f64 = 0.5;

/// Where a [`SpeechHold`] stands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SpeechHoldState {
    /// Speech has been quiet long enough; the next speech starts a fresh
    /// hold.
    Flowing,
    /// A hold has kept the clock still for `for_s` seconds of frame time.
    /// While speech plays the clock waits; in a quiet gap it runs, and
    /// `quiet_s` counts the gap until it is long enough to end the hold.
    Holding { for_s: f64, quiet_s: f64 },
    /// A hold reached `max_hold_s`. The clock runs, and `quiet_s` counts
    /// quiet until it is long enough to hold again.
    Expired { quiet_s: f64 },
}

impl SpeechHoldState {
    /// The state's name, for logs and tests.
    pub fn name(self) -> &'static str {
        match self {
            SpeechHoldState::Flowing => "flowing",
            SpeechHoldState::Holding { .. } => "holding",
            SpeechHoldState::Expired { .. } => "expired",
        }
    }
}

/// A game clock that stands still while speech plays. See the module docs.
#[derive(Clone, Debug)]
pub struct SpeechHold {
    /// The longest a hold lasts, in seconds of frame time.
    pub max_hold_s: f64,
    /// How long speech must be quiet, in seconds of frame time, before a
    /// hold or an expired hold starts over.
    pub rearm_quiet_s: f64,
    state: SpeechHoldState,
    /// Whether the last call held the clock.
    held: bool,
}

impl Default for SpeechHold {
    fn default() -> Self {
        Self::new()
    }
}

impl SpeechHold {
    /// A hold bounded at [`MAX_HOLD_S`].
    pub fn new() -> Self {
        Self::with_max_hold(MAX_HOLD_S)
    }

    /// A hold bounded at `max_hold_s` seconds.
    pub fn with_max_hold(max_hold_s: f64) -> Self {
        SpeechHold {
            max_hold_s,
            rearm_quiet_s: REARM_QUIET_S,
            state: SpeechHoldState::Flowing,
            held: false,
        }
    }

    /// This frame's game time: `0.0` while speech is playing, else `dt`.
    pub fn dt<G: Game>(&mut self, ctx: &mut Ctx<G>, dt: f64) -> f64 {
        let busy = ctx.speech_busy();
        self.step(busy, dt)
    }

    /// The same, with the answer to "is speech playing?" given: every
    /// transition is here.
    pub fn step(&mut self, busy: bool, dt: f64) -> f64 {
        use SpeechHoldState::*;
        let dt = dt.max(0.0);
        let rearmed = |quiet_s: f64| quiet_s + dt >= self.rearm_quiet_s;
        let (next, flowed) = match (self.state, busy) {
            (Flowing, false) => (Flowing, dt),
            (Flowing, true) => self.hold(0.0, dt),
            (Holding { for_s, .. }, true) => self.hold(for_s, dt),
            (Holding { quiet_s, .. }, false) if rearmed(quiet_s) => (Flowing, dt),
            (Holding { for_s, quiet_s }, false) => (
                Holding {
                    for_s,
                    quiet_s: quiet_s + dt,
                },
                dt,
            ),
            (Expired { .. }, true) => (Expired { quiet_s: 0.0 }, dt),
            (Expired { quiet_s }, false) if rearmed(quiet_s) => (Flowing, dt),
            (Expired { quiet_s }, false) => (
                Expired {
                    quiet_s: quiet_s + dt,
                },
                dt,
            ),
        };
        self.state = next;
        self.held = busy && matches!(next, Holding { .. });
        flowed
    }

    /// Hold `dt` more after `for_s`; past `max_hold_s` the rest of the
    /// frame flows.
    fn hold(&self, for_s: f64, dt: f64) -> (SpeechHoldState, f64) {
        let held = for_s + dt;
        if held >= self.max_hold_s {
            (
                SpeechHoldState::Expired { quiet_s: 0.0 },
                held - self.max_hold_s.max(for_s),
            )
        } else {
            (
                SpeechHoldState::Holding {
                    for_s: held,
                    quiet_s: 0.0,
                },
                0.0,
            )
        }
    }

    /// Whether the last call held the clock.
    pub fn held(&self) -> bool {
        self.held
    }

    /// Where the hold stands.
    pub fn state(&self) -> SpeechHoldState {
        self.state
    }

    /// Back to flowing, as if speech had been quiet a while.
    pub fn reset(&mut self) {
        self.state = SpeechHoldState::Flowing;
        self.held = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holding(for_s: f64) -> SpeechHoldState {
        SpeechHoldState::Holding {
            for_s,
            quiet_s: 0.0,
        }
    }

    #[test]
    fn it_holds_while_busy_and_flows_once_quiet() {
        let mut hold = SpeechHold::new();
        assert_eq!(hold.step(false, 0.1), 0.1);
        assert!(!hold.held());
        assert_eq!(hold.step(true, 0.1), 0.0);
        assert!(hold.held());
        assert_eq!(hold.state(), holding(0.1));
        assert_eq!(hold.step(false, 0.25), 0.25);
        assert!(!hold.held());
        assert_eq!(hold.step(false, 0.25), 0.25);
        assert_eq!(hold.state(), SpeechHoldState::Flowing);
    }

    #[test]
    fn a_hold_ends_at_the_maximum_and_the_rest_of_the_frame_flows() {
        let mut hold = SpeechHold::with_max_hold(1.0);
        assert_eq!(hold.step(true, 0.75), 0.0);
        assert_eq!(hold.step(true, 0.5), 0.25);
        assert_eq!(hold.state(), SpeechHoldState::Expired { quiet_s: 0.0 });
        assert!(!hold.held());
        assert_eq!(hold.step(true, 0.5), 0.5, "still talking: time flows");
    }

    #[test]
    fn a_short_quiet_gap_flows_but_keeps_the_count() {
        let mut hold = SpeechHold::with_max_hold(1.0);
        assert_eq!(hold.step(true, 0.75), 0.0);
        assert_eq!(hold.step(false, 0.25), 0.25, "the gap flows");
        assert_eq!(
            hold.state(),
            SpeechHoldState::Holding {
                for_s: 0.75,
                quiet_s: 0.25
            }
        );
        assert_eq!(hold.step(true, 0.5), 0.25, "the count went on from 0.75");
        assert_eq!(hold.state().name(), "expired");
    }

    #[test]
    fn blips_never_let_a_hold_run_past_the_maximum() {
        let mut hold = SpeechHold::with_max_hold(1.0);
        let mut held = 0.0;
        for frame in 0..100 {
            let busy = frame % 5 != 4;
            held += 0.125 - hold.step(busy, 0.125);
        }
        assert_eq!(held, 1.0);
    }

    #[test]
    fn sustained_quiet_starts_the_count_over() {
        let mut hold = SpeechHold::with_max_hold(1.0);
        hold.step(true, 0.75);
        hold.step(false, 0.25);
        hold.step(false, 0.25);
        assert_eq!(hold.state(), SpeechHoldState::Flowing);
        assert_eq!(hold.step(true, 0.75), 0.0, "a fresh hold");
    }

    #[test]
    fn it_holds_again_only_after_sustained_quiet() {
        let mut hold = SpeechHold::with_max_hold(1.0);
        hold.step(true, 2.0);
        assert_eq!(hold.state().name(), "expired");
        assert_eq!(hold.step(true, 0.25), 0.25);
        assert_eq!(hold.step(false, 0.25), 0.25);
        assert_eq!(hold.step(true, 0.25), 0.25, "a blip of quiet is not enough");
        assert_eq!(hold.step(false, 0.25), 0.25);
        assert_eq!(hold.step(false, 0.25), 0.25);
        assert_eq!(hold.state(), SpeechHoldState::Flowing);
        assert_eq!(hold.step(true, 0.25), 0.0);
        assert!(hold.held());
    }

    #[test]
    fn a_frame_of_no_time_holds_without_counting() {
        let mut hold = SpeechHold::with_max_hold(1.0);
        assert_eq!(hold.step(true, 0.0), 0.0);
        assert_eq!(hold.state(), holding(0.0));
        assert!(hold.held());
    }

    #[test]
    fn reset_flows_again() {
        let mut hold = SpeechHold::with_max_hold(1.0);
        hold.step(true, 5.0);
        hold.reset();
        assert_eq!(hold.state(), SpeechHoldState::Flowing);
        assert_eq!(hold.step(true, 0.5), 0.0);
    }
}
