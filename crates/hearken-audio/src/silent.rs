//! A backend that plays nothing.

use crate::backend::{Backend, BackendError, Channel, Source};
use crate::types::Mix;
use std::collections::HashSet;

/// Plays nothing. Channel ids increase forever; a looping sound stays
/// "playing" until [`Backend::stop`], while a one-shot is never reported as
/// playing -- matching a real backend, which will already have finished by
/// the time anyone could ask.
#[derive(Default)]
pub struct SilentBackend {
    next: Channel,
    looping: HashSet<Channel>,
}

impl Backend for SilentBackend {
    fn name(&self) -> String {
        "silent".to_string()
    }

    fn play(
        &mut self,
        _key: &str,
        _source: Source,
        looping: bool,
        _mix: Mix,
    ) -> Result<Channel, BackendError> {
        let channel = self.next;
        self.next += 1;
        if looping {
            self.looping.insert(channel);
        }
        Ok(channel)
    }

    fn set_mix(&mut self, _channel: Channel, _mix: Mix, _ramp_ms: u32) {}

    fn stop(&mut self, channel: Channel, _fade_ms: u32) {
        self.looping.remove(&channel);
    }

    /// A released loop has its tail to play out, which takes no time here.
    fn release(&mut self, channel: Channel, _fade_ms: u32) {
        self.looping.remove(&channel);
    }

    fn pause(&mut self, _channel: Channel, _paused: bool) {}

    fn is_playing(&mut self, channel: Channel) -> bool {
        self.looping.contains(&channel)
    }
}
