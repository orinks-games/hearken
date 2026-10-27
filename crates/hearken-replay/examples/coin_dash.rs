//! Coin Dash: the smallest rules worth putting online, for the replay
//! crate's own tests and as a pattern for a game's.
//!
//! The player stands in one of seven lanes and holds Left or Right to move
//! a lane every quarter second. A coin falls into a random lane every
//! second; standing under it when it lands catches it. The run lasts the
//! context's `seconds`, or until the player quits.
//!
//! Build it for a server with
//! `cargo build -p hearken-replay --example coin-dash --target wasm32-unknown-unknown --release`.

use hearken_replay::{InputCode, Replayable};
use serde::{Deserialize, Serialize};

/// Steps a second.
pub const HZ: u64 = 120;
const LANES: i32 = 7;
/// Steps between moves while a direction is held.
const MOVE_EVERY: u64 = HZ / 4;

/// The game's inputs: a direction pressed or released, or quitting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Left(bool),
    Right(bool),
    Quit,
}

impl InputCode for Control {
    fn to_code(&self) -> (u16, i32) {
        match *self {
            Control::Left(down) => (1, i32::from(down)),
            Control::Right(down) => (2, i32::from(down)),
            Control::Quit => (3, 1),
        }
    }

    fn from_code(code: u16, value: i32) -> Option<Self> {
        match (code, value) {
            (1, 0 | 1) => Some(Control::Left(value == 1)),
            (2, 0 | 1) => Some(Control::Right(value == 1)),
            (3, 1) => Some(Control::Quit),
            _ => None,
        }
    }
}

/// What a run starts from besides its seed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Setup {
    /// How long the run lasts, 1 to 600.
    pub seconds: u32,
}

/// What a run comes to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Score {
    pub coins: u32,
    pub missed: u32,
}

/// The rules.
pub struct Dash {
    rng: u64,
    tick: u64,
    end: u64,
    lane: i32,
    left: bool,
    right: bool,
    held_for: u64,
    coin: i32,
    score: Score,
    quit: bool,
}

impl Dash {
    fn next(&mut self) -> u64 {
        // xorshift64*: integers only, the same everywhere.
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn lane(&self) -> i32 {
        self.lane
    }

    pub fn coin(&self) -> i32 {
        self.coin
    }
}

impl Replayable for Dash {
    const RULES: &'static str = "coin-dash";
    const MAX_TICKS: u64 = 600 * HZ;
    type Input = Control;
    type Context = Setup;
    type Outcome = Score;

    fn new(seed: u64, setup: &Setup) -> Self {
        let mut dash = Dash {
            rng: seed | 1,
            tick: 0,
            end: u64::from(setup.seconds.clamp(1, 600)) * HZ,
            lane: LANES / 2,
            left: false,
            right: false,
            held_for: 0,
            coin: 0,
            score: Score {
                coins: 0,
                missed: 0,
            },
            quit: false,
        };
        dash.coin = (dash.next() % LANES as u64) as i32;
        dash
    }

    fn step(&mut self, inputs: &[Control]) {
        for input in inputs {
            match *input {
                Control::Left(down) => self.left = down,
                Control::Right(down) => self.right = down,
                Control::Quit => self.quit = true,
            }
        }
        if self.quit {
            return;
        }
        let dir = i32::from(self.right) - i32::from(self.left);
        if dir == 0 {
            self.held_for = 0;
        } else {
            // A move on the press, then one every quarter second held.
            if self.held_for.is_multiple_of(MOVE_EVERY) {
                self.lane = (self.lane + dir).clamp(0, LANES - 1);
            }
            self.held_for += 1;
        }
        self.tick += 1;
        if self.tick.is_multiple_of(HZ) {
            if self.lane == self.coin {
                self.score.coins += 1;
            } else {
                self.score.missed += 1;
            }
            self.coin = (self.next() % LANES as u64) as i32;
        }
    }

    fn finished(&self) -> bool {
        self.quit || self.tick >= self.end
    }

    fn outcome(&self) -> Score {
        self.score.clone()
    }
}

hearken_replay::export_rules!(Dash);

#[allow(dead_code)]
fn main() {}
