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
//! `BassBackend` has no `Drop` impl. Dropping one without first calling
//! [`Backend::shutdown`] -- for instance because the game quit while the
//! device was still opening -- still frees every open channel: each
//! [`Stream`]/[`CallbackStream`] frees its own BASS handle on drop, a quick,
//! non-blocking call that is safe from any thread. `BASS_Free` itself,
//! though, only ever runs from `shutdown`, so a `BassBackend` dropped
//! without one leaves BASS initialised, with no channels open, for the rest
//! of the process -- the device sits idle rather than leaking memory, and
//! goes away when the process exits.

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
    /// Set by the first [`Backend::shutdown`] call, so a second one is a
    /// silent no-op instead of a `BASS_Free` on an already-freed device.
    shut_down: bool,
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
            shut_down: false,
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
                    f_center: lowpass_cutoff(hz, entry.base_freq),
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

/// Clamps a requested low-pass cutoff to what `bass_fx`'s biquad filter will
/// accept for a channel whose native rate is `native_rate`: `bass_fx`
/// refuses a centre frequency at or past the channel's Nyquist limit
/// (`native_rate / 2`), so the ceiling here stays 1 Hz under that. The
/// ceiling is never let below the 20 Hz floor either -- on a very low native
/// rate that would invert the range, which would make `f32::clamp` panic --
/// so the floor wins there instead.
fn lowpass_cutoff(hz: f32, native_rate: f32) -> f32 {
    const FLOOR: f32 = 20.0;
    const CEILING: f32 = 20_000.0;
    let ceiling = CEILING.min((native_rate / 2.0 - 1.0).max(FLOOR));
    hz.clamp(FLOOR, ceiling)
}

/// Sets or slides one channel attribute. The handle always comes from a live
/// [`Entry`], so a failure here is not a stale handle -- it is BASS itself
/// refusing the value. Logged at debug for diagnostics; there is nothing to
/// recover, so the mix just carries on with whatever BASS already had.
fn set_attr(handle: u32, attrib: u32, value: f32, ramp_ms: u32) {
    let result = if ramp_ms == 0 {
        channel_set_attribute(handle, attrib, value)
    } else {
        channel_slide_attribute(handle, attrib, value, ramp_ms)
    };
    if let Err(e) = result {
        log::debug!("bass: could not set attribute {attrib}: {e}");
    }
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
        if paused {
            let _ = channel_pause(handle);
        } else if channel_is_active(handle) != BASS_ACTIVE_STOPPED {
            // A channel that has already ended (a one-shot that finished
            // while paused) must stay ended: BASS_ChannelPlay would restart
            // it from the top instead of resuming nothing.
            let _ = channel_play(handle, false);
        }
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
        if self.shut_down {
            return;
        }
        self.shut_down = true;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowpass_cutoff_stays_under_the_channels_nyquist_limit() {
        // 22.05 kHz native rate: bass_fx's own Nyquist check refuses a 15 kHz
        // request, so it is pulled down to 1 Hz under half the native rate.
        assert_eq!(lowpass_cutoff(15_000.0, 22_050.0), 11_024.0);
        // 44.1 kHz native rate: 20 kHz is already under Nyquist, so it is
        // left alone.
        assert_eq!(lowpass_cutoff(20_000.0, 44_100.0), 20_000.0);
        // A native rate so low that "1 Hz under Nyquist" would fall under
        // the 20 Hz floor and invert the clamp's range -- the floor wins
        // instead of panicking.
        assert_eq!(lowpass_cutoff(15_000.0, 30.0), 20.0);
    }
}
