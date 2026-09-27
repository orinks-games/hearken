//! Sound for audio games: a pool of positioned sounds, buses and live voices over any audio library.
//!
//! Hearken manages audio as a handle-and-pool model. The engine positions sounds in 3D space using
//! the listener's heading (degrees clockwise from north, NVGT convention) and provides per-sound
//! and per-bus controls (gain, pan, pitch, low-pass filter cutoff). A backend — BASS, FMOD, or
//! your own — plays them over speakers or a headset.

pub mod spatial;
pub mod types;

pub use spatial::{place, Placement, SpatialConfig};
pub use types::{Bus, Listener, Mix, Vec3};
