//! Pebbles: the smallest two-player rules worth putting online, for the
//! replay crate's tests of seated runs and as a pattern for a game's
//! matches.
//!
//! A pile of pebbles sits between two seats. Seat 1 goes first; a turn
//! takes one, two or three. Whoever takes the last pebble loses. A seat
//! that leaves the match hands the win to the other.
//!
//! The outcome carries `next_seat` and a `summary` line per seat, which
//! Hearken's My games screen reads when a server replays the match after
//! each turn (`hearken_play`); the rest is the game's own.
//!
//! Build it for a server with
//! `cargo build -p hearken-replay --example pebbles --target wasm32-unknown-unknown --release`.

use hearken_replay::{InputCode, Replayable, Seat, Seated};
use serde::{Deserialize, Serialize};

/// The one move: take this many pebbles, 1 to 3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Take(pub u8);

impl InputCode for Take {
    fn to_code(&self) -> (u16, i32) {
        (1, i32::from(self.0))
    }

    fn from_code(code: u16, value: i32) -> Option<Self> {
        (code == 1 && (1..=3).contains(&value)).then_some(Take(value as u8))
    }
}

/// What a match starts from besides its seed: the pile, 5 to 60.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Setup {
    pub pile: u32,
}

/// What a match has come to so far.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Standing {
    pub pile: u32,
    /// Whose move it is, as the engine's My games screen reads it. None
    /// once the match is over.
    pub next_seat: Option<u8>,
    /// The seat that won, once the match is over.
    pub winner: Option<u8>,
    /// One line for each seat, as the engine's My games screen reads it.
    pub summary: Vec<String>,
}

/// The rules.
pub struct Pebbles {
    pile: u32,
    turn: Seat,
    winner: Option<Seat>,
    turns: u32,
}

impl Pebbles {
    pub fn pile(&self) -> u32 {
        self.pile
    }

    pub fn turn(&self) -> Seat {
        self.turn
    }

    fn other(seat: Seat) -> Seat {
        seat.next_of(2)
    }
}

impl Replayable for Pebbles {
    const RULES: &'static str = "pebbles";
    const MAX_TICKS: u64 = 128;
    type Input = Seated<Take>;
    type Context = Setup;
    type Outcome = Standing;

    fn new(_seed: u64, setup: &Setup) -> Self {
        Pebbles {
            pile: setup.pile.clamp(5, 60),
            turn: Seat::FIRST,
            winner: None,
            turns: 0,
        }
    }

    fn step(&mut self, inputs: &[Seated<Take>]) {
        for input in inputs {
            if self.winner.is_some() {
                return;
            }
            match *input {
                // A seat that leaves loses, whoever's turn it was.
                Seated::Left { seat } => self.winner = Some(Self::other(seat)),
                // Only the seat whose turn it is may move; the rest is
                // ignored, as a hand-written run file could say anything.
                Seated::Input {
                    seat,
                    input: Take(n),
                } if seat == self.turn => {
                    let n = u32::from(n).min(self.pile);
                    self.pile -= n;
                    self.turns += 1;
                    if self.pile == 0 {
                        self.winner = Some(Self::other(seat));
                    } else {
                        self.turn = Self::other(seat);
                    }
                }
                Seated::Input { .. } => {}
            }
        }
    }

    fn finished(&self) -> bool {
        self.winner.is_some()
    }

    fn outcome(&self) -> Standing {
        let summary = (0..2u8)
            .map(|seat| match self.winner {
                Some(w) if w.number() == seat => "You won".to_string(),
                Some(_) => "You lost".to_string(),
                None if self.turn.number() == seat => {
                    format!("Your move, {} left", self.pile)
                }
                None => format!("Their move, {} left", self.pile),
            })
            .collect();
        Standing {
            pile: self.pile,
            next_seat: self.winner.is_none().then(|| self.turn.number()),
            winner: self.winner.map(Seat::number),
            summary,
        }
    }
}

hearken_replay::export_rules!(Pebbles);

#[allow(dead_code)]
fn main() {}
