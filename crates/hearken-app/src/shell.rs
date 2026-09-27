//! The window and event pump, behind a trait so tests need no display.

use std::collections::VecDeque;

use crate::input::InputEvent;

pub const FRAME_SECONDS: f64 = 1.0 / 60.0;

pub trait Shell {
    fn poll(&mut self) -> Vec<InputEvent>;
    fn render(&mut self, lines: &[String]);
    /// Sleep out the rest of the frame and return the seconds since the
    /// previous call.
    fn frame(&mut self) -> f64;
}

/// No window: events come from a queue, one per frame, and frames take no
/// time.
pub struct HeadlessShell {
    pub queued: VecDeque<InputEvent>,
    pub dt: f64,
    pub rendered: Vec<Vec<String>>,
}

impl HeadlessShell {
    pub fn new() -> Self {
        HeadlessShell {
            queued: VecDeque::new(),
            dt: FRAME_SECONDS,
            rendered: Vec::new(),
        }
    }

    pub fn push(&mut self, event: InputEvent) {
        self.queued.push_back(event);
    }
}

impl Default for HeadlessShell {
    fn default() -> Self {
        Self::new()
    }
}

impl Shell for HeadlessShell {
    fn poll(&mut self) -> Vec<InputEvent> {
        self.queued.pop_front().into_iter().collect()
    }

    fn render(&mut self, lines: &[String]) {
        self.rendered.push(lines.to_vec());
    }

    fn frame(&mut self) -> f64 {
        self.dt
    }
}
