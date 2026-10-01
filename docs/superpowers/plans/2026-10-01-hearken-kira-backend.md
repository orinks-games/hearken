# Kira Backend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace BASS with Kira as Hearken's built-in audio backend, so a game ships no audio DLL and needs no commercial audio licence.

**Architecture:** A new `KiraBackend` implements the existing `hearken_audio::Backend` trait. Positioning stays engine-side (`spatial.rs`); the backend only turns a `Mix` into sound. Each channel is its own Kira sub-track with a low-pass filter and a panner, playing one sound: a decoded clip, a streamed clip (large files), or a live voice through a small custom Kira `Sound`. `hearken-bass-sys`, `tools/fetch_bass.py` and the `bass` feature are deleted.

**Tech Stack:** Rust 1.98, `kira` 0.12 (cpal output, Symphonia decoding: WAV, OGG Vorbis, MP3, FLAC), fundsp voices from `hearken-synth`.

**Spec:** `docs/superpowers/specs/2026-09-26-hearken-engine-design.md` (audio sections), amended by this plan's decision row: "Kira replaces BASS as the built-in backend" (2026-10-01).

## Global Constraints

- `rust-version = "1.95"`, toolchain `1.98.0`; edition 2021.
- Licence MIT OR Apache-2.0; every new dependency must allow closed-source commercial games (Kira MIT/Apache, cpal Apache, Symphonia MPL-2.0).
- The backend is a public `Send` trait; a backend never sees handles, buses or fades.
- Nothing may block the game loop: `play`/`set_mix`/`update` never decode minutes of audio, `shutdown` is explicit, idempotent, bounded, never a thread join.
- No device means silence and one spoken notice (`NO_SOUND_NOTICE`), unchanged.
- CI: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`, and a build with the default backend off. `actionlint` after any workflow edit.

## Review Focus

1. A long music file started on the game loop must not stall a frame: clips over 256 KiB stream instead of decoding whole (test: a 5 s WAV plays and is heard).
2. More sounds than the backend can hold returns an error, never a panic (test: capacity 2, third `play` is `Err`).
3. Calls after `shutdown` (a second `shutdown`, `play`, `set_mix`, `stop`, `pause`) are quiet no-ops or errors, never panics (test).
4. A gain of zero is silence, and non-finite pan/pitch/gain from a game's direct backend call cannot poison the audio thread (test: gain 0 renders zeros; NaN pitch on a voice still renders finite samples).
5. Pausing silences a sound without ending it, and resuming brings it back (test).

---

### Task 1: `KiraBackend` for clips, the mix, and the lifecycle

**Files:**
- Modify: `Cargo.toml` (workspace dep `kira`)
- Modify: `crates/hearken-audio/Cargo.toml` (feature `kira`, default)
- Create: `crates/hearken-audio/src/kira.rs`
- Modify: `crates/hearken-audio/src/lib.rs`
- Test: `crates/hearken-audio/tests/kira.rs`

**Interfaces:**
- Produces: `hearken_audio::KiraBackend<B: kira::backend::Backend = kira::DefaultBackend>` with
  `KiraBackend::open() -> Result<KiraBackend, String>`,
  `KiraBackend::<B>::with_settings(AudioManagerSettings<B>) -> Result<Self, B::Error>`,
  `manager_mut(&mut self) -> Option<&mut AudioManager<B>>`, and `impl Backend`.

- [ ] **Step 1: Dependencies.** Workspace `Cargo.toml`:

```toml
kira = { version = "0.12", default-features = false, features = ["cpal", "mp3", "ogg", "vorbis", "flac", "wav", "pcm"] }
```

`crates/hearken-audio/Cargo.toml`:

```toml
[features]
default = ["kira"]
kira = ["dep:kira"]

[dependencies]
kira = { workspace = true, optional = true }
```

- [ ] **Step 2: Write the failing tests** in `crates/hearken-audio/tests/kira.rs`. They run on Kira's mock output, so they need no device and run in CI. A `Tap` effect on the main track records every frame the backend makes.

```rust
#![cfg(feature = "kira")]
use std::f32::consts::TAU;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hearken_audio::*;
use kira::backend::mock::{MockBackend, MockBackendSettings};
use kira::effect::Effect;
use kira::info::Info;
use kira::track::MainTrackBuilder;
use kira::{AudioManagerSettings, Capacities, Frame};

const RATE: u32 = 44_100;

/// Copies everything reaching the main track, so a test hears what the
/// backend made.
struct Tap(Arc<Mutex<Vec<Frame>>>);

impl Effect for Tap {
    fn process(&mut self, input: &mut [Frame], _dt: f64, _info: &Info) {
        self.0.lock().unwrap().extend_from_slice(input);
    }
}

struct Rig {
    b: KiraBackend<MockBackend>,
    heard: Arc<Mutex<Vec<Frame>>>,
}

fn rig_with(capacities: Capacities) -> Rig {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let settings = AudioManagerSettings {
        capacities,
        main_track_builder: MainTrackBuilder::new().with_built_effect(Box::new(Tap(heard.clone()))),
        backend_settings: MockBackendSettings { sample_rate: RATE },
        ..AudioManagerSettings::default()
    };
    let b = KiraBackend::with_settings(settings).expect("the mock output always opens");
    Rig { b, heard }
}

fn rig() -> Rig {
    rig_with(Capacities::default())
}

impl Rig {
    /// Renders at least `frames` frames of output and returns them.
    fn render(&mut self, frames: usize) -> Vec<Frame> {
        self.heard.lock().unwrap().clear();
        let mock = self.b.manager_mut().expect("open").backend_mut();
        while self.heard.lock().unwrap().len() < frames {
            mock.on_start_processing();
            mock.process();
        }
        std::mem::take(&mut *self.heard.lock().unwrap())
    }

    fn sub_tracks(&mut self) -> usize {
        self.b.manager_mut().expect("open").num_sub_tracks()
    }
}

fn clip(samples: &[f32]) -> Source {
    Source::Clip(hearken_synth::wav_bytes(samples, RATE).into())
}

fn tone(hz: f32, seconds: f32) -> Vec<f32> {
    let n = (seconds * RATE as f32) as usize;
    (0..n)
        .map(|i| (i as f32 * hz * TAU / RATE as f32).sin() * 0.5)
        .collect()
}

fn rms(frames: &[Frame]) -> f32 {
    (frames.iter().map(|f| f.left * f.left).sum::<f32>() / frames.len() as f32).sqrt()
}

fn mix(gain: f32, pan: f32, pitch: f32, lowpass_hz: Option<f32>) -> Mix {
    Mix { gain, pan, pitch, lowpass_hz, relative: None }
}

#[test]
fn a_one_shot_plays_and_is_reaped_after_it_ends() {
    let mut r = rig();
    let ch = r.b.play("sfx/test", clip(&tone(440.0, 0.05)), false, Mix::UNITY).expect("play");
    assert!(r.b.is_playing(ch));
    assert!(rms(&r.render(1_000)) > 0.1, "it makes sound");
    r.render(4_410);
    r.b.update();
    assert!(!r.b.is_playing(ch));
    r.render(128);
    assert_eq!(r.sub_tracks(), 0, "its track was removed");
}

#[test]
fn a_loop_outlasts_its_clip_until_stopped_and_a_fade_ends_it_after_the_fade() {
    let mut r = rig();
    let ch = r.b.play("amb/test", clip(&tone(440.0, 0.05)), true, Mix::UNITY).expect("play");
    r.render(8_820);
    r.b.update();
    assert!(r.b.is_playing(ch), "a loop outlasts its clip");
    r.b.stop(ch, 100);
    r.b.update();
    assert!(r.b.is_playing(ch), "still fading");
    let faded = r.render(8_820);
    assert!(rms(&faded[faded.len() - 1_000..]) < 1e-4, "the fade reached silence");
    std::thread::sleep(Duration::from_millis(150));
    r.b.update();
    assert!(!r.b.is_playing(ch));
}

#[test]
fn gain_and_pan_reach_the_output() {
    let mut r = rig();
    let ch = r.b.play("sfx/dc", clip(&[0.5; 44_100]), false, mix(0.5, 0.0, 1.0, None)).expect("play");
    let f = r.render(1_000)[500];
    assert!((f.left - 0.25).abs() < 1e-3 && (f.right - 0.25).abs() < 1e-3, "{f:?}");
    r.b.set_mix(ch, mix(0.5, 1.0, 1.0, None), 0);
    let f = r.render(1_000)[500];
    assert!(f.left.abs() < 1e-3, "hard right leaves the left ear silent: {f:?}");
    assert!((f.right - 0.25 * std::f32::consts::SQRT_2).abs() < 1e-3, "{f:?}");
}

#[test]
fn a_gain_of_zero_is_silence() {
    let mut r = rig();
    r.b.play("sfx/dc", clip(&[0.5; 4_410]), false, mix(0.0, 0.0, 1.0, None)).expect("play");
    assert!(r.render(2_000).iter().all(|f| f.left == 0.0 && f.right == 0.0));
}

#[test]
fn pitch_two_plays_a_clip_in_half_the_time() {
    let mut r = rig();
    let fast = r.b.play("sfx/a", clip(&tone(440.0, 0.1)), false, mix(1.0, 0.0, 2.0, None)).expect("play");
    let slow = r.b.play("sfx/b", clip(&tone(440.0, 0.1)), false, Mix::UNITY).expect("play");
    r.render(3_000);
    r.b.update();
    assert!(!r.b.is_playing(fast), "0.1 s at double speed is over by 0.068 s");
    assert!(r.b.is_playing(slow));
}

#[test]
fn a_low_pass_muffles_high_frequencies_and_lifts_cleanly() {
    let mut r = rig();
    let ch = r.b.play("amb/hiss", clip(&tone(8_000.0, 0.5)), true, Mix::UNITY).expect("play");
    let open = rms(&r.render(4_410)[500..]);
    r.b.set_mix(ch, mix(1.0, 0.0, 1.0, Some(800.0)), 0);
    let muffled = rms(&r.render(4_410)[500..]);
    r.b.set_mix(ch, Mix::UNITY, 0);
    let lifted = rms(&r.render(4_410)[500..]);
    assert!(muffled < open * 0.1, "open {open}, muffled {muffled}");
    assert!(lifted > open * 0.9, "open {open}, lifted {lifted}");
}

#[test]
fn pause_silences_a_sound_without_ending_it() {
    let mut r = rig();
    let ch = r.b.play("amb/test", clip(&tone(440.0, 0.5)), true, Mix::UNITY).expect("play");
    r.render(1_000);
    r.b.pause(ch, true);
    let paused = r.render(4_410);
    assert!(rms(&paused[1_000..]) < 1e-4);
    r.b.update();
    assert!(r.b.is_playing(ch), "a paused sound is still alive");
    r.b.pause(ch, false);
    assert!(rms(&r.render(4_410)[1_000..]) > 0.1, "resume brings it back");
}

#[test]
fn a_long_clip_streams_and_is_heard() {
    let mut r = rig();
    // 5 s of 16-bit mono is about 441 KB: over the streaming cut.
    let ch = r.b.play("music/long", clip(&tone(440.0, 5.0)), false, Mix::UNITY).expect("play");
    assert!(r.b.is_playing(ch));
    let mut heard = false;
    for _ in 0..200 {
        if rms(&r.render(1_024)) > 0.1 {
            heard = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(heard, "the decoder thread fed the stream");
}

#[test]
fn bytes_that_are_not_sound_are_an_error_not_a_panic() {
    let mut r = rig();
    let err = r.b.play("sfx/bad", Source::Clip((&b"not audio"[..]).into()), false, Mix::UNITY);
    assert!(err.is_err());
    // Two audio cycles: Kira adds the track in one and removes it in the next.
    r.render(256);
    assert_eq!(r.sub_tracks(), 0, "the failed sound's track did not leak");
}

#[test]
fn more_sounds_than_capacity_is_an_error_not_a_panic() {
    let mut r = rig_with(Capacities { sub_track_capacity: 2, ..Capacities::default() });
    for _ in 0..2 {
        r.b.play("sfx/test", clip(&tone(440.0, 0.1)), false, Mix::UNITY).expect("play");
    }
    assert!(r.b.play("sfx/test", clip(&tone(440.0, 0.1)), false, Mix::UNITY).is_err());
}

#[test]
fn shutdown_is_idempotent_and_later_calls_are_harmless() {
    let mut r = rig();
    let ch = r.b.play("amb/test", clip(&tone(440.0, 0.1)), true, Mix::UNITY).expect("play");
    r.b.shutdown();
    r.b.shutdown();
    assert!(r.b.manager_mut().is_none());
    assert!(!r.b.is_playing(ch));
    r.b.set_mix(ch, Mix::UNITY, 0);
    r.b.pause(ch, true);
    r.b.stop(ch, 100);
    r.b.update();
    assert!(r.b.play("sfx/test", clip(&tone(440.0, 0.1)), false, Mix::UNITY).is_err());
}

#[test]
fn the_default_device_opens_and_plays_or_the_test_says_why_not() {
    let mut b = match KiraBackend::open() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("hearken-audio: no sound device here ({e}); skipping");
            return;
        }
    };
    // Silent, so a test run makes no noise; this checks the device path.
    let ch = b.play("sfx/test", clip(&tone(440.0, 0.05)), false, mix(0.0, 0.0, 1.0, None)).expect("play");
    assert!(b.is_playing(ch));
    b.shutdown();
}
```

- [ ] **Step 3: Run them to see them fail.** `cargo test -p hearken-audio --test kira` → compile error, `KiraBackend` not found.

- [ ] **Step 4: Implement `crates/hearken-audio/src/kira.rs`.** `play` already routes `Source::Voice` to Task 2's `LiveVoice`, so the module compiles once both tasks' code is in; they are split only so each gets its own review. Full module:

```rust
//! The Kira [`Backend`]: plays clips and live voices through the `kira`
//! crate, which compiles into the game -- no library to fetch, ship or
//! license.
//!
//! Each channel is a Kira sub-track of its own, carrying a low-pass filter
//! and a panner, with its one sound playing on it. So a [`Mix`] lands the
//! same way on a clip and a live voice: the gain is the track's volume, the
//! pan its panner, the low-pass its filter (left in, all dry, when the mix
//! wants none, so muffling ramps in and out instead of switching). Pitch is
//! a clip's playback rate, or a live voice's own resampling step. Every stop
//! ends with the channel's track handle dropped, which removes the track and
//! the sound on it.
//!
//! Dropping a `KiraBackend` without [`Backend::shutdown`] is safe: Kira tells
//! its output thread to stop on drop, without a join.

use std::collections::HashMap;
use std::convert::Infallible;
use std::io::Cursor;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hearken_synth::Voice;
use kira::effect::filter::{FilterBuilder, FilterHandle};
use kira::effect::panning_control::{PanningControlBuilder, PanningControlHandle};
use kira::info::Info;
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle};
use kira::sound::streaming::{StreamingSoundData, StreamingSoundHandle};
use kira::sound::{FromFileError, PlaybackState, Sound, SoundData};
use kira::track::{TrackBuilder, TrackHandle};
use kira::{
    AudioManager, AudioManagerSettings, Capacities, Decibels, DefaultBackend, Frame, Panning,
    Tween, Value,
};

use crate::backend::{Backend, BackendError, Channel, Source};
use crate::types::Mix;

/// How many sounds can play at once: one Kira sub-track each.
const CHANNELS: usize = 512;

/// Clips up to this size decode whole when they start; bigger ones (music,
/// long beds) stream from a decoder thread, so `play` never stalls the game
/// loop decoding minutes of audio.
// ponytail: one cut by encoded size; key it to decoded length if a long WAV
// or a dense OGG under the cut is ever heard to stall a frame.
const STREAM_OVER_BYTES: usize = 256 * 1024;

/// The low-pass filter's resonance: Kira's state-variable filter at this
/// setting has a Q of about 0.707, the Butterworth shape Lucky Break's BASS
/// biquad used for occlusion.
const RESONANCE: f64 = 0.308;

/// What plays on a channel's track.
enum Playing {
    Clip(StaticSoundHandle),
    Stream(StreamingSoundHandle<FromFileError>),
    /// A live voice, which never ends on its own; its pitch, as `f32` bits.
    Voice(Arc<AtomicU32>),
}

struct Entry {
    track: TrackHandle,
    filter: FilterHandle,
    panner: PanningControlHandle,
    playing: Playing,
}

impl Entry {
    /// A clip is alive until Kira reports it stopped; a voice until its
    /// channel is stopped.
    fn alive(&self) -> bool {
        match &self.playing {
            Playing::Clip(h) => h.state() != PlaybackState::Stopped,
            Playing::Stream(h) => h.state() != PlaybackState::Stopped,
            Playing::Voice(_) => true,
        }
    }
}

/// Plays sound through Kira: the default device, or any Kira output.
pub struct KiraBackend<B: kira::backend::Backend = DefaultBackend> {
    /// `None` once shut down.
    manager: Option<AudioManager<B>>,
    channels: HashMap<Channel, Entry>,
    /// Channels mid fade-out, and when each one is due to be dropped.
    fading: Vec<(Channel, Instant)>,
    next: Channel,
}

impl KiraBackend {
    /// Opens the default sound device.
    pub fn open() -> Result<KiraBackend, String> {
        let settings = AudioManagerSettings {
            capacities: Capacities {
                sub_track_capacity: CHANNELS,
                ..Capacities::default()
            },
            ..AudioManagerSettings::default()
        };
        let backend =
            KiraBackend::with_settings(settings).map_err(|e| format!("no sound device ({e})"))?;
        log::info!("kira: default sound device open");
        Ok(backend)
    }
}

impl<B: kira::backend::Backend> KiraBackend<B> {
    /// Starts Kira with explicit settings: another device, other capacities,
    /// or Kira's mock output in tests.
    pub fn with_settings(settings: AudioManagerSettings<B>) -> Result<Self, B::Error> {
        Ok(KiraBackend {
            manager: Some(AudioManager::new(settings)?),
            channels: HashMap::new(),
            fading: Vec::new(),
            next: 1,
        })
    }

    /// Kira's manager, to drive a mock output in tests or for diagnostics.
    /// `None` once shut down.
    pub fn manager_mut(&mut self) -> Option<&mut AudioManager<B>> {
        self.manager.as_mut()
    }
}

fn backend_error(e: impl std::fmt::Display) -> BackendError {
    BackendError(e.to_string())
}

/// A linear ramp over `ms`; zero is instant.
fn ramp(ms: u32) -> Tween {
    Tween {
        duration: Duration::from_millis(ms.into()),
        ..Tween::default()
    }
}

/// A gain (1.0 is as recorded) in Kira's decibels; zero, negative or
/// non-finite is silence.
fn decibels(gain: f32) -> Decibels {
    if gain > 0.0 && gain.is_finite() {
        Decibels(20.0 * gain.log10())
    } else {
        Decibels::SILENCE
    }
}

fn panning(pan: f32) -> Panning {
    Panning(if pan.is_finite() { pan.clamp(-1.0, 1.0) } else { 0.0 })
}

/// A pitch as a playback rate, held to the pool's own `0.05..=4` so a bad
/// value from a game's direct call can neither stall nor flood the audio
/// thread.
fn rate(pitch: f32) -> f64 {
    if pitch.is_finite() {
        f64::from(pitch.clamp(0.05, 4.0))
    } else {
        1.0
    }
}

/// The filter's cutoff and wet/dry mix for a mix's low-pass: 20 kHz and all
/// dry when there is none, so turning one on sweeps down to it.
fn lowpass(hz: Option<f32>) -> (f64, kira::Mix) {
    match hz {
        Some(hz) if hz.is_finite() => (f64::from(hz.clamp(20.0, 20_000.0)), kira::Mix::WET),
        _ => (20_000.0, kira::Mix::DRY),
    }
}

/// Starts a clip on `track`: decoded whole, or streamed if it is big.
fn play_clip(
    track: &mut TrackHandle,
    bytes: Arc<[u8]>,
    looping: bool,
    pitch: f32,
) -> Result<Playing, BackendError> {
    if bytes.len() > STREAM_OVER_BYTES {
        let mut data = StreamingSoundData::from_cursor(Cursor::new(bytes))
            .map_err(backend_error)?
            .playback_rate(rate(pitch));
        if looping {
            data = data.loop_region(..);
        }
        track.play(data).map(Playing::Stream).map_err(backend_error)
    } else {
        let mut data = StaticSoundData::from_cursor(Cursor::new(bytes))
            .map_err(backend_error)?
            .playback_rate(rate(pitch));
        if looping {
            data = data.loop_region(..);
        }
        track.play(data).map(Playing::Clip).map_err(backend_error)
    }
}

impl<B: kira::backend::Backend + Send> Backend for KiraBackend<B> {
    fn name(&self) -> String {
        "Kira".to_string()
    }

    fn play(
        &mut self,
        _key: &str,
        source: Source,
        looping: bool,
        mix: Mix,
    ) -> Result<Channel, BackendError> {
        let manager = self
            .manager
            .as_mut()
            .ok_or_else(|| BackendError("the sound device is shut down".into()))?;
        let (cutoff, wet) = lowpass(mix.lowpass_hz);
        let mut builder = TrackBuilder::new()
            .sound_capacity(1)
            .sub_track_capacity(0)
            .volume(decibels(mix.gain));
        let filter = builder.add_effect(
            FilterBuilder::new()
                .cutoff(cutoff)
                .resonance(RESONANCE)
                .mix(wet),
        );
        let panner = builder.add_effect(PanningControlBuilder(Value::Fixed(panning(mix.pan))));
        let mut track = manager.add_sub_track(builder).map_err(backend_error)?;
        let playing = match source {
            Source::Clip(bytes) => play_clip(&mut track, bytes, looping, mix.pitch)?,
            Source::Voice(voice) => {
                let pitch = Arc::new(AtomicU32::new(mix.pitch.to_bits()));
                track
                    .play(LiveVoice::new(voice, pitch.clone()))
                    .map_err(backend_error)?;
                Playing::Voice(pitch)
            }
        };
        let channel = self.next;
        self.next += 1;
        self.channels.insert(
            channel,
            Entry {
                track,
                filter,
                panner,
                playing,
            },
        );
        Ok(channel)
    }

    fn set_mix(&mut self, channel: Channel, mix: Mix, ramp_ms: u32) {
        let Some(entry) = self.channels.get_mut(&channel) else {
            return;
        };
        let tween = ramp(ramp_ms);
        let (cutoff, wet) = lowpass(mix.lowpass_hz);
        entry.track.set_volume(decibels(mix.gain), tween);
        entry.panner.set_panning(panning(mix.pan), tween);
        entry.filter.set_cutoff(cutoff, tween);
        entry.filter.set_mix(wet, tween);
        match &mut entry.playing {
            Playing::Clip(h) => h.set_playback_rate(rate(mix.pitch), tween),
            Playing::Stream(h) => h.set_playback_rate(rate(mix.pitch), tween),
            Playing::Voice(pitch) => pitch.store(mix.pitch.to_bits(), Ordering::Relaxed),
        }
    }

    fn stop(&mut self, channel: Channel, fade_ms: u32) {
        if fade_ms == 0 {
            self.channels.remove(&channel);
        } else if let Some(entry) = self.channels.get_mut(&channel) {
            entry.track.set_volume(Decibels::SILENCE, ramp(fade_ms));
            let due = Instant::now() + Duration::from_millis(fade_ms.into());
            self.fading.push((channel, due));
        }
    }

    fn pause(&mut self, channel: Channel, paused: bool) {
        let Some(entry) = self.channels.get_mut(&channel) else {
            return;
        };
        // Kira's default 10 ms tween, so a pause never clicks.
        if paused {
            entry.track.pause(Tween::default());
        } else {
            entry.track.resume(Tween::default());
        }
    }

    fn is_playing(&mut self, channel: Channel) -> bool {
        self.channels.get(&channel).is_some_and(Entry::alive)
    }

    fn update(&mut self) {
        let now = Instant::now();
        let channels = &mut self.channels;
        self.fading.retain(|&(channel, due)| {
            let done = now >= due;
            if done {
                channels.remove(&channel);
            }
            !done
        });
        self.channels.retain(|_, entry| entry.alive());
    }

    fn shutdown(&mut self) {
        let Some(manager) = self.manager.take() else {
            return;
        };
        let dropped = self.channels.len();
        self.channels.clear();
        self.fading.clear();
        drop(manager);
        log::info!("kira: dropped {dropped} channel(s); device closed");
    }
}
```

`lib.rs` gains:

```rust
#[cfg(feature = "kira")]
mod kira;
#[cfg(feature = "kira")]
pub use self::kira::KiraBackend;
```

and its crate docs name Kira where they named BASS.

- [ ] **Step 5: Run the tests.** `cargo test -p hearken-audio --test kira` → all pass (the device test may print its skip line).

- [ ] **Step 6: Commit.** `feat(audio): a Kira backend, compiled in`.

### Task 2: Live voices on Kira

**Files:**
- Modify: `crates/hearken-audio/src/kira.rs` (add `LiveVoice`)
- Test: `crates/hearken-audio/tests/kira.rs`

**Interfaces:**
- Consumes: `hearken_synth::Voice::{set_sample_rate, fill}`; Task 1's `rate(f32) -> f64`.
- Produces: `struct LiveVoice` implementing `kira::sound::{Sound, SoundData}`, used by `play` for `Source::Voice`.

- [ ] **Step 1: Write the failing tests.**

```rust
fn sine_voice(hz: f32) -> hearken_audio::Source {
    use hearken_synth::fundsp::prelude32::*;
    fn tone(p: &hearken_synth::Params) -> Box<dyn AudioUnit> {
        Box::new(var(&p.get("hz")) >> sine())
    }
    // Fully qualified: fundsp's prelude brings its own `Source`.
    hearken_audio::Source::Voice(hearken_synth::Voice::new(tone, &[("hz", hz)]))
}

fn crossings(frames: &[Frame]) -> usize {
    frames.windows(2).filter(|w| (w[0].left < 0.0) != (w[1].left < 0.0)).count()
}

#[test]
fn a_live_voice_sounds_follows_its_pitch_and_stops() {
    let mut r = rig();
    let ch = r.b.play("voice", sine_voice(441.0), true, Mix::UNITY).expect("play");
    let at_one = crossings(&r.render(4_410)[..4_410]);
    assert!((80..=96).contains(&at_one), "441 Hz for 0.1 s crosses zero ~88 times: {at_one}");
    r.b.set_mix(ch, mix(1.0, 0.0, 2.0, None), 0);
    r.render(512);
    let at_two = crossings(&r.render(4_410)[..4_410]);
    assert!((168..=184).contains(&at_two), "pitch 2 doubles it: {at_two}");
    r.b.stop(ch, 0);
    assert!(!r.b.is_playing(ch));
    assert!(rms(&r.render(1_000)[500..]) < 1e-4);
}

#[test]
fn a_nonsense_pitch_on_a_voice_still_renders_sound() {
    let mut r = rig();
    let ch = r.b.play("voice", sine_voice(441.0), true, Mix::UNITY).expect("play");
    r.b.set_mix(ch, mix(1.0, 0.0, f32::NAN, None), 0);
    let frames = r.render(4_410);
    assert!(frames.iter().all(|f| f.left.is_finite()));
    assert!(rms(&frames[1_000..]) > 0.1);
}
```

- [ ] **Step 2: Run them to see them fail** (`LiveVoice` missing).

- [ ] **Step 3: Implement**, appended to `kira.rs`:

```rust
/// A live voice as a Kira sound: rendered mono at the output's rate, then
/// read back `pitch` times as fast through a linear resampler, so a pitch
/// change never re-targets the voice's graph (which would click).
struct LiveVoice {
    voice: Voice,
    pitch: Arc<AtomicU32>,
    /// The rate the voice's graph renders at; set from the first block.
    rate: f64,
    /// Where the next output frame falls between `prev` and `next`.
    phase: f64,
    prev: f32,
    next: f32,
}

impl LiveVoice {
    fn new(voice: Voice, pitch: Arc<AtomicU32>) -> LiveVoice {
        LiveVoice { voice, pitch, rate: 0.0, phase: 1.0, prev: 0.0, next: 0.0 }
    }
}

impl Sound for LiveVoice {
    fn process(&mut self, out: &mut [Frame], dt: f64, _info: &Info) {
        let output_rate = 1.0 / dt;
        if output_rate != self.rate {
            self.voice.set_sample_rate(output_rate);
            self.rate = output_rate;
        }
        let step = rate(f32::from_bits(self.pitch.load(Ordering::Relaxed)));
        for frame in out {
            while self.phase >= 1.0 {
                self.prev = self.next;
                let mut sample = [0.0];
                self.voice.fill(&mut sample);
                self.next = sample[0];
                self.phase -= 1.0;
            }
            *frame = Frame::from_mono(self.prev + (self.next - self.prev) * self.phase as f32);
            self.phase += step;
        }
    }

    fn finished(&self) -> bool {
        false
    }
}

impl SoundData for LiveVoice {
    type Error = Infallible;
    type Handle = ();

    fn into_sound(self) -> Result<(Box<dyn Sound>, ()), Infallible> {
        Ok((Box::new(self), ()))
    }
}
```

- [ ] **Step 4: Run** `cargo test -p hearken-audio --test kira` → pass.

- [ ] **Step 5: Commit** with Task 1 if done in one sitting.

### Task 3: Kira becomes the default; BASS goes

**Files:**
- Modify: `crates/hearken-app/Cargo.toml` (`default = ["kira"]`, `kira = ["hearken-audio/kira"]`)
- Modify: `crates/hearken-app/src/run.rs` (`default_backend` opens `KiraBackend::open()`, logs `audio: using Kira`)
- Modify: `crates/hearken-app/src/lib.rs` (re-export `KiraBackend` under `feature = "kira"`)
- Modify: `crates/hearken-app/src/game.rs` (drop `<PREFIX>_BASS_PATH` from the env list)
- Modify: `crates/hearken-audio/src/backend.rs` (doc: "Kira, FMOD, or a test double")
- Delete: `crates/hearken-bass-sys/`, `crates/hearken-audio/src/bass.rs`, `crates/hearken-audio/tests/bass.rs`, `tools/fetch_bass.py`
- Modify: `Cargo.toml` (drop `hearken-bass-sys` and `libloading`, only bass-sys used it), `crates/hearken-audio/Cargo.toml` (drop `bass` feature and both bass-sys deps), `.gitignore` (drop the BASS vendor lines), `Cargo.lock`
- Modify: `.github/workflows/ci.yml` (drop "Cache BASS" and "Fetch BASS"; "Build without BASS" becomes "Build without Kira")
- Modify: `README.md` (crate list, build steps without BASS, ALSA headers on Linux, licence note for Symphonia)
- Modify: `docs/superpowers/specs/2026-09-26-hearken-engine-design.md` (summary, non-goals, decisions table row, crates diagram, the bass-sys section, backend bullets, failure handling, testing, template README bullet, risks)

**Interfaces:**
- Consumes: `hearken_audio::KiraBackend::open() -> Result<KiraBackend, String>`.

- [ ] **Step 1: Swap the default** in `run.rs`:

```rust
#[cfg(feature = "kira")]
fn default_backend(_prefix: &str) -> Result<Box<dyn Backend>, String> {
    let backend = KiraBackend::open()?;
    log::info!("audio: using Kira");
    Ok(Box::new(backend))
}

#[cfg(not(feature = "kira"))]
fn default_backend(_prefix: &str) -> Result<Box<dyn Backend>, String> {
    log::info!("audio: using the silent backend (no backend built in)");
    Ok(Box::new(SilentBackend::default()))
}
```

- [ ] **Step 2: Delete BASS** (files above), update manifests, `.gitignore`, CI, README, spec.
- [ ] **Step 3: Verify the build both ways.** `cargo check -p hearken-app --no-default-features --locked`; `cargo clippy --workspace --all-targets -- -D warnings`; `actionlint`.
- [ ] **Step 4: Commit.** `feat!: Kira replaces BASS as the built-in backend`.

### Task 4: Verify end to end

- [ ] `cargo fmt --all --check`
- [ ] `cargo test --workspace --locked` (full sweep, once)
- [ ] Run `cargo run -p hello-hearken` on a machine with a sound device; the session log shows `audio: opened backend Kira` and no audio errors; the practice room's engine voice and circling beacon are audible and move.
