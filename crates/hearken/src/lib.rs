//! Hearken, as one dependency: a game lists `hearken` and nothing else.
//! Everything in `hearken-app` is re-exported at the root, with its audio,
//! synth and speech crates under `audio`, `synth` and `speech`. Extras are
//! features of this crate, not further dependencies.

pub use hearken_app::*;
pub use hearken_replay as replay;
pub use hearken_speech as speech;

#[cfg(feature = "radio")]
pub use hearken_radio as radio;

#[cfg(feature = "voice")]
pub use hearken_voice as voice;
