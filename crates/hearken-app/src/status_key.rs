//! The status key, in layers.
//!
//! One press answers the question a player asks most: what is wrong, and
//! the one or two numbers they track. Pressing it again straight away
//! reads the next layer, the fuller picture. A wall of facts on every
//! press makes players wait through what they already know to reach what
//! changed.
//!
//! Build the layers most urgent first, and leave out a danger when there
//! is none rather than saying "no ghost". Keep each layer's order fixed so
//! players learn where to stop listening. Which key is the status key is
//! the game's choice, through its `Action`.
//!
//! Extracted from Wicket's `parlor_core::StatusKey`, which followed Freight
//! Fate's Space key.

use crate::ctx::Ctx;
use crate::game::Game;

/// How long after a status line a second press still counts as "again".
pub const AGAIN_SECONDS: f64 = 6.0;

/// Remembers the last status press, so a quick second press reads the next
/// layer. Keep one on each play screen and pass every status press to
/// [`StatusKey::speak`].
#[derive(Clone, Debug, Default)]
pub struct StatusKey {
    /// `ctx.said` and `ctx.clock` just after the last status line, and
    /// which layer it was.
    last: Option<(u64, f64, usize)>,
}

impl StatusKey {
    pub fn new() -> Self {
        Self::default()
    }

    /// Speak the next layer. The first press, or any press after something
    /// else was said or a few seconds went by, speaks the first layer.
    /// Empty layers are skipped, and after the last layer it starts over.
    /// Returns the line it spoke, for a game that keeps it for a repeat key.
    pub fn speak<G: Game>(&mut self, ctx: &mut Ctx<G>, layers: Vec<String>) -> String {
        let layers: Vec<String> = layers.into_iter().filter(|l| !l.is_empty()).collect();
        if layers.is_empty() {
            return String::new();
        }
        let next = match self.last {
            Some((said, at, layer)) if said == ctx.said && ctx.clock - at <= AGAIN_SECONDS => {
                (layer + 1) % layers.len()
            }
            _ => 0,
        };
        let line = layers[next].clone();
        ctx.say(line.clone());
        self.last = Some((ctx.said, ctx.clock, next));
        line
    }

    /// Forget the last press, so the next one starts at the first layer.
    pub fn reset(&mut self) {
        self.last = None;
    }
}
