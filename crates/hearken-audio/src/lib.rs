//! Sound for audio games: a pool of positioned sounds, buses and live voices over any audio library.
//!
//! Hearken manages audio as a handle-and-pool model. The engine positions sounds in 3D space using
//! the listener's heading (degrees clockwise from north, NVGT convention) and provides per-sound
//! and per-bus controls (gain, pan, pitch, low-pass filter cutoff). A backend — Kira, FMOD, or
//! your own — plays them over speakers or a headset.

mod backend;
mod clips;
#[cfg(feature = "kira")]
mod kira;
mod open;
mod pool;
mod recording;
mod silent;
mod slots;
mod spatial;
mod types;

#[cfg(feature = "kira")]
pub use self::kira::KiraBackend;
pub use backend::{Backend, BackendError, Channel, Source};
pub use clips::{ClipSource, LooseClips, MemoryClips};
pub use open::{NO_SOUND_NOTICE, SHUTDOWN_WAIT};
pub use pool::{Audio, Fades, LiveSound, Sound};
pub use recording::{Call, Recorder, RecordingBackend};
pub use silent::SilentBackend;
pub use spatial::{place, Placement, SpatialConfig};
pub use types::{Bus, Listener, Mix, Vec3};
