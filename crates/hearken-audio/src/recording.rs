//! A backend that records every call made to it, for tests.

use crate::backend::{Backend, BackendError, Channel, Source};
use crate::types::{LoopRegion, Mix, Ramps};
use std::collections::{HashMap, HashSet};
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
        /// Seconds into the clip it started from; `0.0` from the top.
        from_s: f64,
    },
    SetMix {
        channel: Channel,
        mix: Mix,
        ramp_ms: u32,
    },
    /// A [`Backend::set_mix_ramped`] call: the whole mix, and each part's
    /// own ramp.
    SetMixRamped {
        channel: Channel,
        mix: Mix,
        ramps: Ramps,
    },
    Stop {
        channel: Channel,
        fade_ms: u32,
    },
    Pause {
        channel: Channel,
        paused: bool,
    },
    Release {
        channel: Channel,
        fade_ms: u32,
    },
    SetLoopRegion {
        channel: Channel,
        region: LoopRegion,
    },
}

#[derive(Default)]
struct Inner {
    calls: Vec<Call>,
    playing: HashSet<Channel>,
    next: Channel,
    fail_next: Option<String>,
    /// Keys whose clips refuse to play, as an undecodable file would.
    refused: HashSet<String>,
    /// Clip lengths by key, for [`Backend::length_s`].
    lengths: HashMap<String, f64>,
    /// The key each channel was started with.
    keys: HashMap<Channel, String>,
    /// The channels started with an insert.
    inserted: HashSet<Channel>,
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

    /// The most recent mix sent for `channel`, from a `Play`, `SetMix` or
    /// `SetMixRamped` call.
    pub fn last_mix(&self, channel: Channel) -> Option<Mix> {
        self.calls().into_iter().rev().find_map(|call| match call {
            Call::Play {
                channel: c, mix, ..
            } if c == channel => Some(mix),
            Call::SetMix {
                channel: c, mix, ..
            } if c == channel => Some(mix),
            Call::SetMixRamped {
                channel: c, mix, ..
            } if c == channel => Some(mix),
            _ => None,
        })
    }

    /// Whether `channel` was started with an insert
    /// ([`Backend::play_inserted`]).
    pub fn inserted(&self, channel: Channel) -> bool {
        self.with_inner(|inner| inner.inserted.contains(&channel))
    }

    /// Whether `channel` is still playing, as the backend would answer.
    pub fn is_playing(&self, channel: Channel) -> bool {
        self.with_inner(|inner| inner.playing.contains(&channel))
    }

    /// Makes every later `play` of `key` fail, as an undecodable clip does.
    pub fn refuse(&self, key: &str) {
        self.with_inner(|inner| inner.refused.insert(key.to_string()));
    }

    /// Gives `key`'s clip a length, which [`Backend::length_s`] reports for
    /// any channel playing it.
    pub fn set_length(&self, key: &str, seconds: f64) {
        self.with_inner(|inner| inner.lengths.insert(key.to_string(), seconds));
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
        self.play_from(key, source, looping, mix, 0.0)
    }

    fn play_inserted(
        &mut self,
        key: &str,
        source: Source,
        looping: bool,
        mix: Mix,
        from_s: f64,
        _insert: &crate::voicing::Insert,
    ) -> Result<Channel, BackendError> {
        let channel = self.play_from(key, source, looping, mix, from_s)?;
        self.recorder
            .with_inner(|inner| inner.inserted.insert(channel));
        Ok(channel)
    }

    fn play_from(
        &mut self,
        key: &str,
        source: Source,
        looping: bool,
        mix: Mix,
        from_s: f64,
    ) -> Result<Channel, BackendError> {
        self.recorder.with_inner(|inner| {
            if let Some(reason) = inner.fail_next.take() {
                return Err(BackendError(reason));
            }
            if matches!(source, Source::Clip(_)) && inner.refused.contains(key) {
                return Err(BackendError(format!("{key}: refused")));
            }
            let channel = inner.next;
            inner.next += 1;
            inner.playing.insert(channel);
            inner.keys.insert(channel, key.to_string());
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
                from_s,
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

    fn set_mix_ramped(&mut self, channel: Channel, mix: Mix, ramps: Ramps) {
        self.recorder.with_inner(|inner| {
            inner.calls.push(Call::SetMixRamped {
                channel,
                mix,
                ramps,
            });
        });
    }

    fn length_s(&mut self, channel: Channel) -> Option<f64> {
        self.recorder.with_inner(|inner| {
            let key = inner.keys.get(&channel)?;
            inner.lengths.get(key).copied()
        })
    }

    fn stop(&mut self, channel: Channel, fade_ms: u32) {
        self.recorder.with_inner(|inner| {
            inner.playing.remove(&channel);
            inner.calls.push(Call::Stop { channel, fade_ms });
        });
    }

    /// Recorded, and the channel keeps playing its tail until
    /// [`Recorder::finish`] ends it.
    fn release(&mut self, channel: Channel, fade_ms: u32) {
        self.recorder.with_inner(|inner| {
            inner.calls.push(Call::Release { channel, fade_ms });
        });
    }

    fn set_loop_region(&mut self, channel: Channel, region: LoopRegion) {
        self.recorder.with_inner(|inner| {
            inner.calls.push(Call::SetLoopRegion { channel, region });
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
