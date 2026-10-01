//! A backend that records every call made to it, for tests.

use crate::backend::{Backend, BackendError, Channel, Source};
use crate::types::Mix;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

/// One call made to a [`RecordingBackend`].
#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    Play {
        channel: Channel,
        key: String,
        looping: bool,
        clip: Option<Arc<[u8]>>,
        mix: Mix,
    },
    SetMix {
        channel: Channel,
        mix: Mix,
        ramp_ms: u32,
    },
    Stop {
        channel: Channel,
        fade_ms: u32,
    },
    Pause {
        channel: Channel,
        paused: bool,
    },
}

#[derive(Default)]
struct Inner {
    calls: Vec<Call>,
    playing: HashSet<Channel>,
    next: Channel,
    fail_next: Option<String>,
}

/// A handle to a [`RecordingBackend`]'s recorded calls, for assertions.
#[derive(Clone, Default)]
pub struct Recorder(Arc<Mutex<Inner>>);

impl Recorder {
    fn with_inner<T>(&self, f: impl FnOnce(&mut Inner) -> T) -> T {
        let mut guard = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f(&mut guard)
    }

    /// Every call recorded so far, in order.
    pub fn calls(&self) -> Vec<Call> {
        self.with_inner(|inner| inner.calls.clone())
    }

    /// The keys of every `Play` call, in order.
    pub fn played(&self) -> Vec<String> {
        self.calls()
            .into_iter()
            .filter_map(|call| match call {
                Call::Play { key, .. } => Some(key),
                _ => None,
            })
            .collect()
    }

    /// The most recent mix sent for `channel`, from a `Play` or `SetMix` call.
    pub fn last_mix(&self, channel: Channel) -> Option<Mix> {
        self.calls().into_iter().rev().find_map(|call| match call {
            Call::Play {
                channel: c, mix, ..
            } if c == channel => Some(mix),
            Call::SetMix {
                channel: c, mix, ..
            } if c == channel => Some(mix),
            _ => None,
        })
    }

    /// The channel of the latest `Play` call for `key`.
    pub fn channel_of(&self, key: &str) -> Option<Channel> {
        self.calls().into_iter().rev().find_map(|call| match call {
            Call::Play {
                channel, key: k, ..
            } if k == key => Some(channel),
            _ => None,
        })
    }

    /// Marks a channel as no longer playing.
    pub fn finish(&self, channel: Channel) {
        self.with_inner(|inner| {
            inner.playing.remove(&channel);
        });
    }

    /// Makes the next `play` call fail with `reason`.
    pub fn fail_next_play(&self, reason: &str) {
        self.with_inner(|inner| inner.fail_next = Some(reason.to_string()));
    }

    /// Forgets recorded calls; channel playing state is untouched.
    pub fn clear(&self) {
        self.with_inner(|inner| inner.calls.clear());
    }
}

/// Records every call made to it, for pool tests.
pub struct RecordingBackend {
    recorder: Recorder,
}

impl RecordingBackend {
    /// A fresh backend and the recorder that observes it.
    pub fn new() -> (RecordingBackend, Recorder) {
        let recorder = Recorder::default();
        (
            RecordingBackend {
                recorder: recorder.clone(),
            },
            recorder,
        )
    }
}

impl Backend for RecordingBackend {
    fn name(&self) -> String {
        "recording".to_string()
    }

    fn play(
        &mut self,
        key: &str,
        source: Source,
        looping: bool,
        mix: Mix,
    ) -> Result<Channel, BackendError> {
        self.recorder.with_inner(|inner| {
            if let Some(reason) = inner.fail_next.take() {
                return Err(BackendError(reason));
            }
            let channel = inner.next;
            inner.next += 1;
            inner.playing.insert(channel);
            let clip = match source {
                Source::Clip(bytes) => Some(bytes),
                Source::Voice(_) | Source::Feed(_) => None,
            };
            inner.calls.push(Call::Play {
                channel,
                key: key.to_string(),
                looping,
                clip,
                mix,
            });
            Ok(channel)
        })
    }

    fn set_mix(&mut self, channel: Channel, mix: Mix, ramp_ms: u32) {
        self.recorder.with_inner(|inner| {
            inner.calls.push(Call::SetMix {
                channel,
                mix,
                ramp_ms,
            });
        });
    }

    fn stop(&mut self, channel: Channel, fade_ms: u32) {
        self.recorder.with_inner(|inner| {
            inner.playing.remove(&channel);
            inner.calls.push(Call::Stop { channel, fade_ms });
        });
    }

    fn pause(&mut self, channel: Channel, paused: bool) {
        self.recorder.with_inner(|inner| {
            inner.calls.push(Call::Pause { channel, paused });
        });
    }

    fn is_playing(&mut self, channel: Channel) -> bool {
        self.recorder
            .with_inner(|inner| inner.playing.contains(&channel))
    }
}
