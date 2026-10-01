//! Audio a backend pulls as it plays: a live stream, a call, a video's
//! soundtrack. The pool does not know where the audio comes from, only that
//! it keeps arriving.

/// A source of audio that a backend pulls from, on its own audio thread.
///
/// Never blocks: what is not there yet is silence, and a feed that cannot
/// keep up must say so by returning fewer frames, not by waiting.
pub trait Feed: Send + Sync {
    /// Copies up to `out.len()` stereo frames available now into `out`, all
    /// at one sample rate, and returns how many and that rate. Stops early at
    /// a rate change; the next call starts at the new rate.
    fn read(&self, out: &mut [[f32; 2]]) -> FeedRead;
}

/// What one [`Feed::read`] produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeedRead {
    /// How many frames were written to the front of `out`.
    pub frames: usize,
    /// The sample rate of those frames; meaningless when `frames` is 0.
    pub sample_rate: u32,
}
