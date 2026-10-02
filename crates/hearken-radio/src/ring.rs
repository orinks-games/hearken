//! The decoded audio between the worker and the audio thread: stereo
//! frames, each run tagged with its sample rate, holding at most a few
//! seconds.

use std::collections::VecDeque;
use std::time::Duration;

use hearken_audio::FeedRead;

pub(crate) struct Ring {
    cap: Duration,
    /// Runs of frames at one rate, oldest first. A run the reader empties
    /// stays until the next push, so the audio thread never frees memory.
    runs: VecDeque<(u32, VecDeque<[f32; 2]>)>,
}

impl Ring {
    pub(crate) fn new(cap: Duration) -> Ring {
        Ring {
            cap,
            runs: VecDeque::new(),
        }
    }

    /// Adds frames at `rate` (never 0). Nothing is ever dropped: the worker
    /// waits while the ring is [`full`](Ring::full) instead.
    pub(crate) fn push(&mut self, rate: u32, frames: Vec<[f32; 2]>) {
        match self.runs.back_mut() {
            Some((last, run)) if *last == rate => run.extend(frames),
            _ => self.runs.push_back((rate, frames.into())),
        }
        self.runs.retain(|(_, run)| !run.is_empty());
    }

    /// Holding the cap. A push may run past it by its own length, so a ring
    /// that is filling always gets there; an empty ring is never full.
    pub(crate) fn full(&self) -> bool {
        let buffered = self.buffered();
        !buffered.is_zero() && buffered >= self.cap
    }

    /// Copies out the oldest frames, stopping at a rate change.
    pub(crate) fn read(&mut self, out: &mut [[f32; 2]]) -> FeedRead {
        let Some((rate, run)) = self.runs.iter_mut().find(|(_, run)| !run.is_empty()) else {
            return FeedRead {
                frames: 0,
                sample_rate: 0,
            };
        };
        let frames = out.len().min(run.len());
        for (slot, frame) in out.iter_mut().zip(run.drain(..frames)) {
            *slot = frame;
        }
        FeedRead {
            frames,
            sample_rate: *rate,
        }
    }

    /// How much audio is waiting, each run timed at its own rate.
    pub(crate) fn buffered(&self) -> Duration {
        self.runs
            .iter()
            .map(|(rate, run)| {
                Duration::from_nanos(run.len() as u64 * 1_000_000_000 / u64::from(*rate))
            })
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `n` frames at `rate`, each valued by its index from `from`.
    fn frames(from: usize, n: usize) -> Vec<[f32; 2]> {
        (from..from + n).map(|i| [i as f32; 2]).collect()
    }

    #[test]
    fn reads_come_out_in_order() {
        let mut ring = Ring::new(Duration::from_secs(1));
        ring.push(1_000, frames(0, 10));
        ring.push(1_000, frames(10, 10));
        let mut out = [[0.0; 2]; 15];
        assert_eq!(ring.read(&mut out).frames, 15);
        assert_eq!(out.to_vec(), frames(0, 15));
        assert_eq!(ring.read(&mut out).frames, 5);
        assert_eq!(out[..5].to_vec(), frames(15, 5));
        assert_eq!(ring.read(&mut out).frames, 0);
    }

    #[test]
    fn a_read_stops_at_a_rate_change() {
        let mut ring = Ring::new(Duration::from_secs(1));
        ring.push(44_100, frames(0, 10));
        ring.push(48_000, frames(10, 10));
        let mut out = [[0.0; 2]; 32];
        let first = ring.read(&mut out);
        assert_eq!((first.frames, first.sample_rate), (10, 44_100));
        let second = ring.read(&mut out);
        assert_eq!((second.frames, second.sample_rate), (10, 48_000));
        assert_eq!(out[..10].to_vec(), frames(10, 10));
    }

    #[test]
    fn a_full_ring_keeps_all_its_audio() {
        let mut ring = Ring::new(Duration::from_secs(1));
        ring.push(1_000, frames(0, 1_500));
        assert_eq!(ring.buffered(), Duration::from_millis(1_500));
        assert!(ring.full());
        let mut out = [[0.0; 2]; 2_000];
        assert_eq!(ring.read(&mut out).frames, 1_500);
        assert_eq!(out[..1_500].to_vec(), frames(0, 1_500));
        assert!(!Ring::new(Duration::ZERO).full(), "empty is never full");
    }

    #[test]
    fn buffered_counts_seconds_at_each_chunks_rate() {
        let mut ring = Ring::new(Duration::from_secs(4));
        ring.push(44_100, frames(0, 22_050));
        ring.push(48_000, frames(0, 24_000));
        assert_eq!(ring.buffered(), Duration::from_secs(1));
    }
}
