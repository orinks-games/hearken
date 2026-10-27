//! What the agent heard since its last reply, kept small.
//!
//! The buffer fills every frame whether or not a call is waiting, so an
//! agent that thinks for a minute between calls comes back to a minute of
//! game. A game that starts a sound every frame (footsteps, a ticking
//! clock, a crowd) put tens of thousands of lines into one reply, and every
//! `wait_for` frame lowercased all of them again. Now a line that repeats
//! the one before it is counted rather than stored, the buffer keeps at most
//! [`MAX_LINES`] lines and says how many older ones it let go, and a
//! `wait_for` looks at each line once, as it arrives.

use std::collections::VecDeque;

/// Distinct lines one reply carries at most; older ones are counted.
pub(super) const MAX_LINES: usize = 1000;

#[derive(Default)]
pub(super) struct Heard {
    /// Each line and how many times in a row it was heard.
    lines: VecDeque<(String, u32)>,
    /// Lines let go to stay under [`MAX_LINES`].
    dropped: u64,
}

impl Heard {
    pub(super) fn push(&mut self, line: String) {
        if let Some((last, n)) = self.lines.back_mut() {
            if *last == line {
                *n = n.saturating_add(1);
                return;
            }
        }
        self.lines.push_back((line, 1));
        if self.lines.len() > MAX_LINES {
            self.lines.pop_front();
            self.dropped += 1;
        }
    }

    /// Whether a kept line contains `text`, which is already lowercase.
    pub(super) fn contains(&self, text: &str) -> bool {
        self.lines
            .iter()
            .any(|(line, _)| line.to_lowercase().contains(text))
    }

    /// Everything kept, one line each, and empties the buffer.
    pub(super) fn take(&mut self) -> String {
        let mut out = Vec::with_capacity(self.lines.len() + 1);
        if self.dropped > 0 {
            out.push(format!("({} earlier lines not kept)", self.dropped));
        }
        for (line, n) in self.lines.drain(..) {
            if n > 1 {
                out.push(format!("{line} (x{n})"));
            } else {
                out.push(line);
            }
        }
        self.dropped = 0;
        out.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repeated_line_is_counted_once() {
        let mut h = Heard::default();
        for _ in 0..40 {
            h.push("sound: step".into());
        }
        h.push("said: Gate open.".into());
        h.push("sound: step".into());
        assert_eq!(h.take(), "sound: step (x40)\nsaid: Gate open.\nsound: step");
        assert_eq!(h.take(), "");
    }

    #[test]
    fn the_buffer_keeps_the_newest_lines_and_counts_the_rest() {
        let mut h = Heard::default();
        for i in 0..MAX_LINES + 5 {
            h.push(format!("line {i}"));
        }
        let text = h.take();
        let mut lines = text.lines();
        assert_eq!(lines.next(), Some("(5 earlier lines not kept)"));
        assert_eq!(lines.next(), Some("line 5"));
        assert_eq!(text.lines().count(), MAX_LINES + 1);
    }

    #[test]
    fn contains_matches_without_case() {
        let mut h = Heard::default();
        h.push("said: Night One begins.".into());
        assert!(h.contains("night one"));
        assert!(!h.contains("night two"));
    }
}
