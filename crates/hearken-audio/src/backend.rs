//! The [`Backend`] trait every sound library implements, and what it plays.

use crate::feed::Feed;
use crate::types::{LoopRegion, Mix};
use std::fmt;

/// An opaque, backend-assigned id for one playing sound.
pub type Channel = u64;

/// What to play: loose clip bytes, a live synth voice, or a feed the backend
/// pulls audio from.
pub enum Source {
    Clip(std::sync::Arc<[u8]>),
    Voice(hearken_synth::Voice),
    Feed(std::sync::Arc<dyn Feed>),
}

/// An error a backend reports back through [`Backend::play`].
#[derive(Clone, Debug, PartialEq)]
pub struct BackendError(pub String);

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for BackendError {}

/// A sound backend: Kira, FMOD, or a test double.
///
/// Nothing here may block the game loop: `shutdown` is explicit, idempotent
/// and bounded, never a network call, an IPC round trip or a thread join.
pub trait Backend: Send {
    /// A short, human-readable name for logs and diagnostics.
    fn name(&self) -> String;

    /// Starts playing `source` and returns its channel.
    fn play(
        &mut self,
        key: &str,
        source: Source,
        looping: bool,
        mix: Mix,
    ) -> Result<Channel, BackendError>;

    /// Changes a playing channel's mix, ramping over `ramp_ms`.
    fn set_mix(&mut self, channel: Channel, mix: Mix, ramp_ms: u32);

    /// Fades the channel out over `fade_ms`, then frees it; the backend owns
    /// the fade, not the caller.
    fn stop(&mut self, channel: Channel, fade_ms: u32);

    /// Leaves a looping channel's loop: a clip plays on through the rest of
    /// its file (a horn's release, say) and then ends by itself. A backend
    /// that cannot, and any live voice or feed, which has no tail, stops
    /// with `fade_ms` instead, which is the default.
    fn release(&mut self, channel: Channel, fade_ms: u32) {
        self.stop(channel, fade_ms);
    }

    /// Replaces the region a looping clip repeats, overriding any loop
    /// points the file itself carries. The default ignores it, so the whole
    /// clip (or its own loop points) repeats.
    fn set_loop_region(&mut self, channel: Channel, region: LoopRegion) {
        let _ = (channel, region);
    }

    /// Pauses or resumes a channel.
    fn pause(&mut self, channel: Channel, paused: bool);

    /// Whether the channel is still alive. A paused channel is still
    /// playing; only a channel that has ended or been stopped is not.
    fn is_playing(&mut self, channel: Channel) -> bool;

    /// Called once a frame: reap ended and faded channels.
    fn update(&mut self) {}

    /// Explicit, idempotent teardown. Never blocks.
    fn shutdown(&mut self) {}
}
