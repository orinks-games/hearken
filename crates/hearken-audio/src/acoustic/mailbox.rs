//! A fixed block of numbers the game thread writes and the audio thread
//! reads, without locks: a sequence lock over atomics. The reader never
//! waits; when it catches a write half done it keeps what it had.

use std::sync::atomic::{fence, AtomicU32, Ordering};

pub(crate) struct Mailbox<const N: usize> {
    seq: AtomicU32,
    words: [AtomicU32; N],
}

impl<const N: usize> Mailbox<N> {
    pub(crate) fn new(values: &[f32; N]) -> Mailbox<N> {
        Mailbox {
            // Even: nothing being written. Starts at 2 so a fresh reader
            // (which has seen 0) takes the first values.
            seq: AtomicU32::new(2),
            words: std::array::from_fn(|i| AtomicU32::new(values[i].to_bits())),
        }
    }

    /// Writes all `N` values. One writer only: the game thread.
    pub(crate) fn write(&self, values: &[f32; N]) {
        let s = self.seq.load(Ordering::Relaxed);
        self.seq.store(s.wrapping_add(1), Ordering::Relaxed);
        fence(Ordering::Release);
        for (word, v) in self.words.iter().zip(values) {
            word.store(v.to_bits(), Ordering::Relaxed);
        }
        self.seq.store(s.wrapping_add(2), Ordering::Release);
    }

    /// Copies the values into `out` if they changed since `seen` and no
    /// write was under way. Never waits.
    pub(crate) fn read(&self, out: &mut [f32; N], seen: &mut u32) -> bool {
        let s1 = self.seq.load(Ordering::Acquire);
        if s1 & 1 == 1 || s1 == *seen {
            return false;
        }
        let mut tmp = [0.0f32; N];
        for (t, word) in tmp.iter_mut().zip(&self.words) {
            *t = f32::from_bits(word.load(Ordering::Relaxed));
        }
        fence(Ordering::Acquire);
        if self.seq.load(Ordering::Relaxed) != s1 {
            return false;
        }
        *out = tmp;
        *seen = s1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_once_per_write() {
        let m = Mailbox::new(&[1.0, 2.0]);
        let mut out = [0.0; 2];
        let mut seen = 0;
        assert!(m.read(&mut out, &mut seen));
        assert_eq!(out, [1.0, 2.0]);
        assert!(!m.read(&mut out, &mut seen));
        m.write(&[3.0, 4.0]);
        assert!(m.read(&mut out, &mut seen));
        assert_eq!(out, [3.0, 4.0]);
    }

    #[test]
    fn torn_reads_are_never_seen() {
        let m = std::sync::Arc::new(Mailbox::new(&[0.0; 16]));
        let w = m.clone();
        let writer = std::thread::spawn(move || {
            for i in 0..20_000 {
                w.write(&[i as f32; 16]);
            }
        });
        let mut out = [0.0; 16];
        let mut seen = 0;
        for _ in 0..200_000 {
            if m.read(&mut out, &mut seen) {
                assert!(out.iter().all(|&v| v == out[0]), "{out:?}");
            }
        }
        writer.join().unwrap();
    }
}
