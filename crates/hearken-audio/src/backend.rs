//! The [`Backend`] trait every sound library implements, and what it plays.

use crate::feed::Feed;
use crate::types::{LoopRegion, Mix, Ramps};
use crate::voicing::Insert;
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

    /// Starts playing `source` `from_s` seconds in. A live voice or a feed
    /// has no position and starts as [`Backend::play`] would; so does a clip
    /// on a backend that cannot seek, which is the default.
    fn play_from(
        &mut self,
        key: &str,
        source: Source,
        looping: bool,
        mix: Mix,
        from_s: f64,
    ) -> Result<Channel, BackendError> {
        let _ = from_s;
        self.play(key, source, looping, mix)
    }

    /// Starts `source` as [`Backend::play_from`] does, with `insert`
    /// processing it before its mix. A backend that cannot run inserts
    /// plays it unprocessed, which is the default.
    fn play_inserted(
        &mut self,
        key: &str,
        source: Source,
        looping: bool,
        mix: Mix,
        from_s: f64,
        insert: &Insert,
    ) -> Result<Channel, BackendError> {
        let _ = insert;
        self.play_from(key, source, looping, mix, from_s)
    }

    /// Changes a playing channel's mix, ramping over `ramp_ms`.
    fn set_mix(&mut self, channel: Channel, mix: Mix, ramp_ms: u32);

    /// Changes a playing channel's mix with a ramp of its own for each part,
    /// touching only the parts that changed, so a ramp already under way on
    /// one part (a pitch slide) is not restarted by a change to another (a
    /// volume set at once). The default sends the whole mix over the gain's
    /// ramp.
    fn set_mix_ramped(&mut self, channel: Channel, mix: Mix, ramps: Ramps) {
        self.set_mix(channel, mix, ramps.gain_ms);
    }

    /// How long the clip on `channel` runs, in seconds, when the backend
    /// knows; `None` for a live voice, a feed, or a backend that cannot tell.
    fn length_s(&mut self, channel: Channel) -> Option<f64> {
        let _ = channel;
        None
    }

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

    /// While `true`, every channel started from now on is made ready but
    /// stays silent at its start; `false` starts all of them at once. For
    /// layers that must play in time with each other when each `play`
    /// takes a while (it decodes the clip first). The default does nothing,
    /// which suits a backend whose `play` returns at once.
    fn hold(&mut self, held: bool) {
        let _ = held;
    }

    /// Whether this backend mixes only when asked ([`Backend::render`])
    /// rather than in real time to a device.
    fn is_offline(&self) -> bool {
        false
    }

    /// Mixes the next `frames` frames (stereo, `-1..=1`) and hands them
    /// back, for a backend with no device ([`crate::KiraBackend::offline`]);
    /// it may hand back a few more. `None` for one that plays to a device.
    fn render(&mut self, frames: usize) -> Option<Vec<[f32; 2]>> {
        let _ = frames;
        None
    }

    /// Whether the channel is still alive. A paused channel is still
    /// playing; only a channel that has ended or been stopped is not.
    fn is_playing(&mut self, channel: Channel) -> bool;

    /// Called once a frame: reap ended and faded channels.
    fn update(&mut self) {}

    /// Starts writing the final mix, what the player hears, to a WAV file
    /// at `path`. A backend that cannot record says so, which is the
    /// default.
    fn start_capture(&mut self, path: &std::path::Path) -> Result<(), BackendError> {
        let _ = path;
        Err(BackendError(format!("{} cannot record", self.name())))
    }

    /// Stops a recording and finishes its file: the file's path, or why it
    /// could not be finished. `None` when nothing was recording.
    fn stop_capture(&mut self) -> Option<Result<std::path::PathBuf, BackendError>> {
        None
    }

    /// Explicit, idempotent teardown. Never blocks.
    fn shutdown(&mut self) {}
}
