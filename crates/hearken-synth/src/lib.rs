//! Synthesised sound for audio games.
//!
//! `hearken-synth` renders every sound the engine ships with -- the seven
//! `UI_*` earcons, a default sound per [`SoundKind`] for everything else,
//! and live-voice recipes a game writes itself -- from [`fundsp`] graphs.
//! Nothing here reaches a file, a network, or a synthesis provider: it is
//! fundsp and nothing else external.

mod builtin;
mod voice;
mod wav;

pub use fundsp;

/// The sample rate every built-in and recipe renders at.
pub const SAMPLE_RATE: u32 = 44_100;

pub use builtin::{
    builtin, builtin_wav, kind, SoundKind, UI_BACK, UI_ERROR, UI_MOVE, UI_NOTIFY, UI_OPEN,
    UI_SELECT, UI_TICK,
};
pub use voice::{Params, Recipe, Voice};
pub use wav::wav_bytes;
