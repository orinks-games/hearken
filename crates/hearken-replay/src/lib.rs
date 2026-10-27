//! Verified runs for Hearken games.
//!
//! A game records a run as its seed and inputs ([`Recording`]), not as its
//! result. A server plays the run again with its own copy of the game's
//! rules, built for WebAssembly ([`export_rules!`]), and only the result it
//! computes counts. What that result is for (a leaderboard, a shared goal,
//! a ghost to race) is up to the game's own server handler: nothing here
//! knows.
//!
//! This crate has no audio, speech or window code, so a game's rules crate
//! that depends on it (and not on `hearken`) builds for
//! `wasm32-unknown-unknown`. `docs/online.md` in the Hearken repository
//! covers the whole service.

pub mod format;
pub mod rules;
pub mod wasm;

pub use format::{
    build_of, canonical_json, run_id, BuildId, FormatError, Input, Run, MAX_RUN_BYTES,
};
pub use rules::{
    assert_replays, replay, verify, Expected, InputCode, Recording, Replayable, Verified,
    VerifyError,
};
