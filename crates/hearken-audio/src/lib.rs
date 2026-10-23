//! Sound for audio games: a pool of positioned sounds, buses and live voices over any audio library.
//!
//! Hearken manages audio as a handle-and-pool model. The engine positions sounds in 3D space using
//! the listener's heading (degrees clockwise from north, NVGT convention) and provides per-sound
//! and per-bus controls (gain, pan, pitch, low-pass filter cutoff). A backend — Kira, FMOD, or
//! your own — plays them over speakers or a headset.

mod backend;
mod clips;
#[cfg(feature = "kira")]
mod decode;
mod feed;
#[cfg(feature = "kira")]
mod kira;
mod open;
mod pool;
mod recording;
mod silent;
mod slots;
mod spatial;
mod types;
mod voicing;

#[cfg(feature = "kira")]
pub use self::kira::{DeviceBackend, DeviceError, GainCurve, KiraBackend, PanLaw};
pub use backend::{Backend, BackendError, Channel, Source};
pub use clips::{
    discover_clips, ClipSource, LayeredClips, LooseClips, MadeClips, MemoryClips, PackedClips,
};
#[cfg(feature = "kira")]
pub use decode::{probe_clip, ClipInfo};
pub use feed::{Feed, FeedRead};
pub use open::{NO_SOUND_NOTICE, SHUTDOWN_WAIT};
pub use pool::{Audio, Fades, Heard, LiveSound, PlayError, Sound};
pub use recording::{Call, Recorder, RecordingBackend};
pub use silent::SilentBackend;
pub use spatial::{place, Placement, SpatialConfig};
pub use types::{Bus, Listener, LoopRegion, Mix, Ramps, Start, Vec3};
pub use voicing::{CabSpeakers, CabSpeakersDsp, CabSpeakersParams, CabTone, Insert, Reflection};
