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
//! One unbroken hold lasts at most `max_hold_s` of real frame time, so a
//! voice that never reports silence, or a game that never stops talking,
//! cannot freeze the clock. After that the clock runs until speech has gone
//! quiet once, then holds again.

use crate::ctx::Ctx;
use crate::game::Game;

/// The longest one unbroken hold lasts by default, in seconds.
pub const MAX_HOLD_S: f64 = 20.0;

/// Where a [`SpeechHold`] stands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SpeechHoldState {
    /// Speech is quiet; the clock runs.
    Flowing,
    /// Speech is playing; the clock has waited `for_s` seconds of frame
    /// time so far.
    Holding { for_s: f64 },
    /// A hold reached `max_hold_s`; the clock runs until speech is quiet.
    Expired,
}

impl SpeechHoldState {
    /// The state's name, for logs and tests.
    pub fn name(self) -> &'static str {
        match self {
            SpeechHoldState::Flowing => "flowing",
            SpeechHoldState::Holding { .. } => "holding",
            SpeechHoldState::Expired => "expired",
        }
    }
}

/// A game clock that stands still while speech plays. See the module docs.
#[derive(Clone, Debug)]
pub struct SpeechHold {
    /// The longest one unbroken hold lasts, in seconds of frame time.
    pub max_hold_s: f64,
    state: SpeechHoldState,
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
            state: SpeechHoldState::Flowing,
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
        let dt = dt.max(0.0);
        let (next, flowed) = match (self.state, busy) {
            (SpeechHoldState::Expired, true) => (SpeechHoldState::Expired, dt),
            (_, false) => (SpeechHoldState::Flowing, dt),
            (SpeechHoldState::Flowing, true) => self.hold(0.0, dt),
            (SpeechHoldState::Holding { for_s }, true) => self.hold(for_s, dt),
        };
        self.state = next;
        flowed
    }

    /// Hold `dt` more after `for_s`; past `max_hold_s` the rest of the
    /// frame flows.
    fn hold(&self, for_s: f64, dt: f64) -> (SpeechHoldState, f64) {
        let held = for_s + dt;
        if held >= self.max_hold_s {
            (SpeechHoldState::Expired, held - self.max_hold_s.max(for_s))
        } else {
            (SpeechHoldState::Holding { for_s: held }, 0.0)
        }
    }

    /// Whether the last call held the clock.
    pub fn held(&self) -> bool {
        matches!(self.state, SpeechHoldState::Holding { .. })
    }

    /// Where the hold stands.
    pub fn state(&self) -> SpeechHoldState {
        self.state
    }

    /// Back to flowing, as if speech had gone quiet.
    pub fn reset(&mut self) {
        self.state = SpeechHoldState::Flowing;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_holds_while_busy_and_flows_once_quiet() {
        let mut hold = SpeechHold::new();
        assert_eq!(hold.step(false, 0.1), 0.1);
        assert!(!hold.held());
        assert_eq!(hold.step(true, 0.1), 0.0);
        assert!(hold.held());
        assert_eq!(hold.state(), SpeechHoldState::Holding { for_s: 0.1 });
        assert_eq!(hold.step(false, 0.1), 0.1);
        assert_eq!(hold.state(), SpeechHoldState::Flowing);
    }

    #[test]
    fn a_hold_ends_at_the_maximum_and_the_rest_of_the_frame_flows() {
        let mut hold = SpeechHold::with_max_hold(1.0);
        assert_eq!(hold.step(true, 0.75), 0.0);
        assert_eq!(hold.step(true, 0.5), 0.25);
        assert_eq!(hold.state(), SpeechHoldState::Expired);
        assert!(!hold.held());
        assert_eq!(hold.step(true, 0.5), 0.5, "still talking: time flows");
    }

    #[test]
    fn it_holds_again_only_after_speech_was_quiet() {
        let mut hold = SpeechHold::with_max_hold(1.0);
        hold.step(true, 2.0);
        assert_eq!(hold.state(), SpeechHoldState::Expired);
        assert_eq!(hold.step(true, 0.1), 0.1);
        assert_eq!(hold.step(false, 0.1), 0.1);
        assert_eq!(hold.step(true, 0.1), 0.0);
        assert!(hold.held());
    }

    #[test]
    fn a_frame_of_no_time_holds_without_counting() {
        let mut hold = SpeechHold::with_max_hold(1.0);
        assert_eq!(hold.step(true, 0.0), 0.0);
        assert_eq!(hold.state(), SpeechHoldState::Holding { for_s: 0.0 });
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
