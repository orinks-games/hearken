//! The BASS [`Backend`]: plays clips and live voices through
//! `hearken-bass-sys`.
//!
//! The low-pass filter -- a `BASS_FX_BFX_BQF` biquad, with its centre
//! frequency and a fixed Q of `0.707` -- is the same shape Lucky Break Casino
//! used for occlusion in `bass3d.rs` (commit a101970); falling back to a
//! quieter, unfiltered channel when `bass_fx` is not vendored is this
//! crate's own choice, not carried from anywhere. Keeping a memory stream's
//! buffer alive until it is freed -- pinned inside [`bass_sys::safe::Stream`]
//! itself -- is what Freight Fate's `audio/bass.rs` (commit c5d3ef8d) did by
//! hand; here it comes for free from the safe layer, so this module just
//! holds the `Stream` (or [`CallbackStream`] for a live voice) until the
//! channel is reaped.
//!
//! # Dropping without shutting down
//!
//! `BassBackend` has no `Drop` impl. If it is ever dropped without a call to
//! [`Backend::shutdown`] first -- the case carried over from Task 4's
//! review: the game quits while the device is still opening, the pool
//! abandons the open, and the `BassBackend` that finishes building on the
//! detached `audio-open` thread is simply dropped there -- every open
//! [`Stream`]/[`CallbackStream`] still frees its own handle (a quick,
//! non-blocking FFI call that is safe on any thread), but `BASS_Free` is
//! never called. The constraints forbid anything in `Drop` that can block or
//! join a thread, and freeing the whole device is exactly the kind of thing
//! that belongs to an explicit, bounded `shutdown`, not an implicit one run
//! from whatever thread happens to drop the value. So a `BassBackend`
//! dropped without `shutdown` leaves BASS initialised for the rest of the
//! process: the device sits open and idle -- no channels, no memory pinned
//! by them -- until the process exits, rather than either leaking channels
//! or blocking the thread that dropped it.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use bass_sys::safe::{
    callback_stream, channel_frequency, channel_is_active, channel_pause, channel_play,
    channel_remove_fx, channel_set_attribute, channel_set_fx, channel_slide_attribute,
    channel_stop, fx_set_bqf, init, stream_create_mem_shared, version_string, CallbackStream,
    Stream,
};
use bass_sys::{
    Api, BASS_ACTIVE_STOPPED, BASS_ATTRIB_FREQ, BASS_ATTRIB_PAN, BASS_ATTRIB_VOL, BASS_BFX_BQF,
    BASS_BFX_BQF_LOWPASS, BASS_BFX_CHANALL, BASS_DEFAULT_DEVICE, BASS_ERROR_ALREADY,
    BASS_FX_BFX_BQF, BASS_SAMPLE_LOOP, HFX,
};

use crate::backend::{Backend, BackendError, Channel, Source};
use crate::types::Mix;

/// A clip's decoded WAV/OGG/MP3 stream, or a live voice's callback stream --
/// whichever [`Source`] a channel was started from.
enum Owned {
    Clip(Stream),
    Voice(CallbackStream),
}

impl Owned {
    fn handle(&self) -> u32 {
        match self {
            Owned::Clip(s) => s.handle(),
            Owned::Voice(s) => s.handle(),
        }
    }
}

struct Entry {
    stream: Owned,
    /// The channel's native playback rate, so a pitch of `1.0` always means
    /// "as recorded" no matter what BASS decoded it at.
    base_freq: f32,
    /// The attached low-pass filter, if the mix currently wants one.
    fx: Option<HFX>,
    looping: bool,
}

/// Plays sound through the BASS audio library.
///
/// Channels are the underlying BASS stream handle widened to a `u64`; there
/// is one [`Entry`] per channel, tracking the stream that owns its BASS
/// handle, its native frequency and its attached filter (if any).
pub struct BassBackend {
    channels: HashMap<Channel, Entry>,
    /// Channels mid fade-out (`stop` with a non-zero fade), and when each one
    /// is due to be reaped.
    fading: Vec<(Channel, Instant)>,
    bass_fx_loaded: bool,
}

impl BassBackend {
    /// Opens the default sound device.
    pub fn open(env_prefix: &str) -> Result<BassBackend, String> {
        BassBackend::open_device(env_prefix, BASS_DEFAULT_DEVICE)
    }

    /// Opens a specific device (`0` is BASS's "no sound" device, used by
    /// tests; `-1` is the system default, what [`BassBackend::open`] uses).
    pub fn open_device(env_prefix: &str, device: i32) -> Result<BassBackend, String> {
        bass_sys::set_game_library_env(&format!("{env_prefix}_BASS_PATH"));
        let api = Api::get().map_err(|e| format!("BASS could not be loaded: {e}"))?;
        match init(device, 44_100, 0) {
            Ok(()) => {}
            Err(e) if e.is(BASS_ERROR_ALREADY) => {}
            Err(e) => return Err(format!("no sound device ({})", e.name())),
        }
        let bass_fx_loaded = bass_sys::fx::load_bass_fx(api.library_dir());
        let version = version_string().unwrap_or_else(|_| "unknown".to_string());
        log::info!(
            "bass: device {device} open (BASS {version}, bass_fx {})",
            if bass_fx_loaded {
                "loaded"
            } else {
                "unavailable"
            }
        );
        Ok(BassBackend {
            channels: HashMap::new(),
            fading: Vec::new(),
            bass_fx_loaded,
        })
    }

    /// Whether `channel` has a low-pass filter attached right now. For tests
    /// and diagnostics.
    pub fn has_filter(&self, channel: Channel) -> bool {
        self.channels
            .get(&channel)
            .is_some_and(|entry| entry.fx.is_some())
    }

    fn create_stream(source: Source, looping: bool) -> Result<Owned, BackendError> {
        match source {
            Source::Clip(bytes) => {
                let flags = if looping { BASS_SAMPLE_LOOP } else { 0 };
                stream_create_mem_shared(bytes, flags)
                    .map(Owned::Clip)
                    .map_err(|e| BackendError(e.name().into()))
            }
            Source::Voice(mut voice) => {
                voice.set_sample_rate(44_100.0);
                callback_stream(44_100, 1, 0, move |out| {
                    voice.fill(out);
                    out.len()
                })
                .map(Owned::Voice)
                .map_err(|e| BackendError(e.name().into()))
            }
        }
    }

    /// Sends `mix` to `entry`'s channel: volume, pan and frequency always;
    /// the low-pass filter attached or removed as `mix.lowpass_hz` asks.
    /// `ramp_ms` zero sets each attribute at once, otherwise slides it.
    fn apply_mix(bass_fx_loaded: bool, entry: &mut Entry, mix: Mix, ramp_ms: u32) {
        let handle = entry.stream.handle();
        let wants_lowpass = mix.lowpass_hz.is_some();
        let mut volume = mix.gain.max(0.0);
        if wants_lowpass && !bass_fx_loaded {
            // No filter available to muffle the sound: duck it by -3 dB
            // instead of leaving it at full, unfiltered volume.
            volume *= 0.708;
        }
        let pan = mix.pan.clamp(-1.0, 1.0);
        let freq = entry.base_freq * mix.pitch;

        set_attr(handle, BASS_ATTRIB_VOL, volume, ramp_ms);
        set_attr(handle, BASS_ATTRIB_PAN, pan, ramp_ms);
        set_attr(handle, BASS_ATTRIB_FREQ, freq, ramp_ms);

        match mix.lowpass_hz {
            Some(hz) if bass_fx_loaded => {
                let fx = match entry.fx {
                    Some(fx) => fx,
                    None => match channel_set_fx(handle, BASS_FX_BFX_BQF, 0) {
                        Ok(fx) => {
                            entry.fx = Some(fx);
                            fx
                        }
                        Err(e) => {
                            log::warn!("bass: could not attach the low-pass filter: {e}");
                            return;
                        }
                    },
                };
                let params = BASS_BFX_BQF {
                    l_filter: BASS_BFX_BQF_LOWPASS,
                    f_center: hz.clamp(20.0, 20_000.0),
                    f_gain: 0.0,
                    f_bandwidth: 0.0,
                    f_q: 0.707,
                    f_s: 0.0,
                    l_channel: BASS_BFX_CHANALL,
                };
                if let Err(e) = fx_set_bqf(fx, &params) {
                    log::warn!("bass: could not set the low-pass filter's parameters: {e}");
                }
            }
            _ => {
                if let Some(fx) = entry.fx.take() {
                    let _ = channel_remove_fx(handle, fx);
                }
            }
        }
    }
}

/// Sets or slides one channel attribute, ignoring the error a stale handle
/// (already reaped, or freed by BASS itself) would report -- the same
/// "calls on an old handle do nothing" contract the rest of the crate keeps.
fn set_attr(handle: u32, attrib: u32, value: f32, ramp_ms: u32) {
    let result = if ramp_ms == 0 {
        channel_set_attribute(handle, attrib, value)
    } else {
        channel_slide_attribute(handle, attrib, value, ramp_ms)
    };
    let _ = result;
}

impl Backend for BassBackend {
    fn name(&self) -> String {
        "BASS".to_string()
    }

    fn play(
        &mut self,
        _key: &str,
        source: Source,
        looping: bool,
        mix: Mix,
    ) -> Result<Channel, BackendError> {
        let owned = BassBackend::create_stream(source, looping)?;
        let handle = owned.handle();
        let base_freq = channel_frequency(handle).unwrap_or(44_100) as f32;
        let mut entry = Entry {
            stream: owned,
            base_freq,
            fx: None,
            looping,
        };
        BassBackend::apply_mix(self.bass_fx_loaded, &mut entry, mix, 0);
        if let Err(e) = channel_play(handle, false) {
            // `entry` (and the stream/callback stream it owns) drops here.
            return Err(BackendError(e.name().into()));
        }
        let channel = handle as Channel;
        self.channels.insert(channel, entry);
        Ok(channel)
    }

    fn set_mix(&mut self, channel: Channel, mix: Mix, ramp_ms: u32) {
        let bass_fx_loaded = self.bass_fx_loaded;
        if let Some(entry) = self.channels.get_mut(&channel) {
            BassBackend::apply_mix(bass_fx_loaded, entry, mix, ramp_ms);
        }
    }

    fn stop(&mut self, channel: Channel, fade_ms: u32) {
        let Some(entry) = self.channels.get(&channel) else {
            return;
        };
        let handle = entry.stream.handle();
        if fade_ms == 0 {
            let _ = channel_stop(handle);
            self.channels.remove(&channel);
        } else {
            let _ = channel_slide_attribute(handle, BASS_ATTRIB_VOL, 0.0, fade_ms);
            self.fading.push((
                channel,
                Instant::now() + Duration::from_millis(fade_ms as u64),
            ));
        }
    }

    fn pause(&mut self, channel: Channel, paused: bool) {
        let Some(entry) = self.channels.get(&channel) else {
            return;
        };
        let handle = entry.stream.handle();
        let result = if paused {
            channel_pause(handle)
        } else {
            channel_play(handle, false)
        };
        let _ = result;
    }

    fn is_playing(&mut self, channel: Channel) -> bool {
        self.channels
            .get(&channel)
            .is_some_and(|entry| channel_is_active(entry.stream.handle()) != BASS_ACTIVE_STOPPED)
    }

    fn update(&mut self) {
        let now = Instant::now();
        let mut expired = Vec::new();
        self.fading.retain(|&(channel, deadline)| {
            if now >= deadline {
                expired.push(channel);
                false
            } else {
                true
            }
        });
        for channel in expired {
            self.channels.remove(&channel);
        }

        let ended: Vec<Channel> = self
            .channels
            .iter()
            .filter(|(_, entry)| {
                !entry.looping && channel_is_active(entry.stream.handle()) == BASS_ACTIVE_STOPPED
            })
            .map(|(&channel, _)| channel)
            .collect();
        for channel in ended {
            self.channels.remove(&channel);
        }
    }

    fn shutdown(&mut self) {
        let dropped = self.channels.len();
        self.channels.clear();
        self.fading.clear();
        log::info!("bass: dropped {dropped} channel(s)");
        match bass_sys::safe::free() {
            Ok(()) => log::info!("bass: device freed"),
            Err(e) => log::warn!("bass: BASS_Free failed: {e}"),
        }
    }
}
