//! Opening the backend off the game loop, the player-facing notice when that
//! fails, and a bounded shutdown.

use std::sync::mpsc::{self, RecvTimeoutError, TryRecvError};
use std::time::Duration;

use crate::backend::{Backend, Source};
use crate::clips::ClipSource;
use crate::pool::Audio;
use crate::silent::SilentBackend;
use hearken_synth::Voice;

/// Shown to the player once when no sound device could be opened.
pub const NO_SOUND_NOTICE: &str =
    "No sound device could be opened, so the game is playing without sound.";

/// How long [`Audio::shutdown`] waits for a still-opening backend before
/// giving up on it.
pub const SHUTDOWN_WAIT: Duration = Duration::from_secs(2);

impl Audio {
    /// Starts silent and opens `open` on its own thread, so a slow or hung
    /// driver never blocks the game loop; `update` swaps the real backend in
    /// once it is ready. The thread is detached: nothing ever joins it. If
    /// the thread itself cannot be spawned (a resource failure, not an
    /// invariant), this stays silent and behaves as if the device could not
    /// be opened, the same as `open` returning `Err`.
    pub fn deferred(
        open: impl FnOnce() -> Result<Box<dyn Backend>, String> + Send + 'static,
        clips: Box<dyn ClipSource>,
    ) -> Audio {
        let mut audio = Audio::new(Box::new(SilentBackend::default()), clips);
        let (tx, rx) = mpsc::sync_channel(1);
        let spawned = std::thread::Builder::new()
            .name("audio-open".to_string())
            .spawn(move || {
                let _ = tx.send(open());
            });
        match spawned {
            Ok(_handle) => audio.opening = Some(rx),
            Err(err) => {
                log::error!("audio: could not spawn the audio-open thread: {err}");
                audio.notice = Some(NO_SOUND_NOTICE.to_string());
            }
        }
        audio
    }

    /// Whether the real backend is still being opened.
    pub fn opening(&self) -> bool {
        self.opening.is_some()
    }

    /// A one-time message for the player, such as [`NO_SOUND_NOTICE`].
    pub fn take_notice(&mut self) -> Option<String> {
        self.notice.take()
    }

    /// Checks the opener thread without blocking. Called first thing in
    /// `update`, so nothing else in a frame ever mixes against a channel id
    /// from a backend that is about to be swapped out.
    pub(crate) fn poll_open(&mut self) {
        let outcome = match &self.opening {
            Some(rx) => rx.try_recv(),
            None => return,
        };
        match outcome {
            Ok(Ok(backend)) => {
                self.opening = None;
                self.swap_backend(backend);
            }
            Ok(Err(reason)) => {
                self.opening = None;
                log::error!("audio: could not open a sound device: {reason}");
                self.notice = Some(NO_SOUND_NOTICE.to_string());
            }
            Err(TryRecvError::Disconnected) => {
                self.opening = None;
                log::error!("audio: the audio-open thread ended without a result");
                self.notice = Some(NO_SOUND_NOTICE.to_string());
            }
            Err(TryRecvError::Empty) => {}
        }
    }

    /// Replays every looping slot (clip, voice or feed) on `backend` with its
    /// current mix, keeping its slot and generation so handles stay valid,
    /// re-applying `paused` and falling back to a default clip the same way
    /// `start` would if the new backend refuses one; frees every one-shot
    /// slot, since a one-shot played while silent was never meant to be
    /// heard again. The silent backend it replaces is dropped.
    fn swap_backend(&mut self, backend: Box<dyn Backend>) {
        log::info!("audio: opened backend {}", backend.name());
        // Assigned up front so the replays below (through
        // `play_with_fallback`, which plays on `self.backend`) land on the
        // new backend, not the silent one it replaces -- which is dropped
        // right here.
        self.backend = backend;
        let mut one_shots = Vec::new();
        let mut loops = Vec::new();
        for (i, slot) in self.slots.iter().enumerate() {
            let Some(live) = &slot.live else { continue };
            if live.looping {
                loops.push(i as u32);
            } else {
                one_shots.push(i as u32);
            }
        }
        for index in one_shots {
            self.free_slot(index);
        }
        for index in loops {
            let Some(live) = self.slots[index as usize].live.as_ref() else {
                continue;
            };
            let (key, mix, voice, feed, paused, region) = (
                live.key.clone(),
                live.sent,
                live.voice.clone(),
                live.feed.clone(),
                live.paused,
                live.region,
            );
            let source = match (&voice, feed) {
                (Some((recipe, params)), _) => {
                    Source::Voice(Voice::with_params(*recipe, params.clone()))
                }
                (None, Some(feed)) => Source::Feed(feed),
                (None, None) => Source::Clip(self.clip_or_default(&key)),
            };
            match self.play_with_fallback(&key, source, true, mix) {
                Some(channel) => {
                    if let Some(region) = region {
                        self.backend.set_loop_region(channel, region);
                    }
                    if paused {
                        self.backend.pause(channel, true);
                    }
                    if let Some(live) = self.slots[index as usize].live.as_mut() {
                        live.channel = channel;
                    }
                }
                None => {
                    self.free_slot(index);
                }
            }
        }
    }

    /// Explicit, idempotent, bounded teardown: never waits longer than
    /// [`SHUTDOWN_WAIT`] for a backend that is still opening, and never
    /// joins the opener thread.
    pub fn shutdown(&mut self) {
        if self.shut_down {
            return;
        }
        self.shut_down = true;
        // An idle placeholder pool -- nothing opening, nothing ever played --
        // has nothing worth a log line; `install_audio` shuts one of these
        // down every time it swaps a pool in, which would otherwise write two
        // lines to every session log before the game had made a sound.
        let had_work = self.opening.is_some() || self.slots.iter().any(|s| s.live.is_some());
        if had_work {
            log::info!("audio: shutting down");
        }
        if let Some(rx) = self.opening.take() {
            match rx.recv_timeout(SHUTDOWN_WAIT) {
                Ok(Ok(mut backend)) => {
                    log::info!(
                        "audio: the device finished opening during shutdown; closing it too"
                    );
                    backend.shutdown();
                }
                Ok(Err(reason)) => {
                    log::info!("audio: the pending open had already failed ({reason}); nothing to shut down");
                }
                Err(RecvTimeoutError::Disconnected) => {
                    log::info!(
                        "audio: the audio-open thread ended without a result; nothing to shut down"
                    );
                }
                Err(RecvTimeoutError::Timeout) => {
                    log::info!(
                        "audio: gave up waiting for the device to open after {SHUTDOWN_WAIT:?}; abandoning it"
                    );
                }
            }
        }
        let live: Vec<u32> = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(i, slot)| slot.live.is_some().then_some(i as u32))
            .collect();
        for index in live {
            if let Some(channel) = self.slots[index as usize].live.as_ref().map(|l| l.channel) {
                self.backend.stop(channel, 0);
            }
            self.free_slot(index);
        }
        self.backend.shutdown();
        self.backend = Box::new(SilentBackend::default());
        if had_work {
            log::info!("audio: shut down");
        }
    }
}
