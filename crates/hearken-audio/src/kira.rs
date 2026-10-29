//! The Kira [`Backend`]: plays clips, live voices and feeds through the
//! `kira` crate, which compiles into the game -- no library to fetch, ship
//! or license.
//!
//! Each channel is a Kira sub-track of its own, carrying a low-pass filter
//! and a panner, with its one sound playing on it. So a [`Mix`] lands the
//! same way on a clip, a live voice and a feed: the gain is the track's
//! volume, the pan its panner, the low-pass its filter (left in, all dry,
//! when the mix wants none, so muffling ramps in and out instead of
//! switching). Pitch is a clip's playback rate, or a voice's or feed's own
//! resampling step. Every stop ends with the channel's track handle dropped,
//! which removes the track and the sound on it.
//!
//! Dropping a `KiraBackend` without [`Backend::shutdown`] is safe: the device
//! watcher is told to stop on drop, without a join. The default device is
//! reopened when it changes or goes away (see [`DeviceBackend`]).

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hearken_synth::Voice;
use kira::effect::filter::{FilterBuilder, FilterHandle};
use kira::effect::panning_control::{PanningControlBuilder, PanningControlHandle};
use kira::effect::Effect;
use kira::info::Info;
use kira::sound::static_sound::StaticSoundHandle;
use kira::sound::streaming::StreamingSoundHandle;
use kira::sound::{FromFileError, PlaybackPosition, PlaybackState, Region, Sound, SoundData};
use kira::track::{TrackBuilder, TrackHandle};
use kira::{
    AudioManager, AudioManagerSettings, Capacities, Decibels, Frame, Panning, Tween, Value,
};

use crate::backend::{Backend, BackendError, Channel, Source};
use crate::decode::{self, LoopPoints};
use crate::feed::Feed;
use crate::types::{LoopRegion, Mix, Ramps};
use crate::voicing::{CabSpeakersDsp, Insert};

mod capture;
mod device;

pub use device::{DeviceBackend, DeviceError};

/// How many sounds can play at once: one Kira sub-track each.
const CHANNELS: usize = 512;

/// Clips up to this size decode whole when they start; bigger ones (music,
/// long beds) stream from a decoder thread, so `play` never stalls the game
/// loop decoding minutes of audio.
// ponytail: one cut by encoded size; key it to decoded length if a long WAV
// or a dense OGG under the cut is ever heard to stall a frame.
const STREAM_OVER_BYTES: usize = 256 * 1024;

/// A looping clip decodes whole up to this size, so the loops a game keeps
/// running for minutes (an engine, road noise, a bed) wrap in memory, sample
/// exact, instead of re-opening a decoder at every wrap; past it, it streams
/// like any long clip.
const LOOP_STREAM_OVER_BYTES: usize = 2 * 1024 * 1024;

/// How a channel's gain moves when it changes, and the shape of a fade-out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GainCurve {
    /// Kira's own: tweened in decibels, so a fade spends its first half
    /// barely quieter and drops off at the end.
    #[default]
    Decibels,
    /// Straight lines in amplitude, as BASS slides its volume: a 1 s fade to
    /// silence is at half volume half way through.
    Linear,
}

/// How a channel's pan splits a sound between the ears.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PanLaw {
    /// Kira's: constant power, unity in each ear at the centre and both
    /// ears' energy kept as it moves, so hard to one side is 3 dB louder in
    /// that ear than the centre.
    #[default]
    ConstantPower,
    /// A balance control, as BASS pans: unity in both ears at the centre,
    /// and moving toward one side only lowers the other ear, linearly, to
    /// silence at the edge. `left = min(1, 1 - pan)`, `right = min(1, 1 + pan)`.
    Balance,
}

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
    /// A feed, which never ends on its own either; its pitch, as `f32` bits.
    Feed(Arc<AtomicU32>),
}

struct Entry {
    track: TrackHandle,
    filter: FilterHandle,
    /// Kira's panner, under [`PanLaw::ConstantPower`].
    panner: Option<PanningControlHandle>,
    /// The linear gain, the balance, or both, when the backend uses either.
    amp: Option<Arc<AmpControl>>,
    curve: GainCurve,
    law: PanLaw,
    /// The mix last sent, so a ramped change touches only what moved.
    sent: Mix,
    /// The clip's length, when it is a clip.
    length_s: Option<f64>,
    playing: Playing,
}

impl Entry {
    fn set_gain(&mut self, gain: f32, ms: u32) {
        match (&self.amp, self.curve) {
            (Some(amp), GainCurve::Linear) => amp.gain.store(pack(gain, ms), Ordering::Relaxed),
            _ => self.track.set_volume(decibels(gain), ramp(ms)),
        }
    }

    fn set_pan(&mut self, pan: f32, ms: u32) {
        let pan = panning(pan);
        match (&self.amp, &mut self.panner) {
            (Some(amp), _) if self.law == PanLaw::Balance => {
                amp.pan.store(pack(pan.0, ms), Ordering::Relaxed)
            }
            (_, Some(panner)) => panner.set_panning(pan, ramp(ms)),
            _ => {}
        }
    }

    fn set_filter(&mut self, hz: Option<f32>, ms: u32) {
        let (cutoff, wet) = lowpass(hz);
        self.filter.set_cutoff(cutoff, ramp(ms));
        self.filter.set_mix(wet, ramp(ms));
    }

    fn set_pitch(&mut self, pitch: f32, ms: u32) {
        match &mut self.playing {
            Playing::Clip(h) => h.set_playback_rate(rate(pitch), ramp(ms)),
            Playing::Stream(h) => h.set_playback_rate(rate(pitch), ramp(ms)),
            Playing::Voice(p) | Playing::Feed(p) => p.store(pitch.to_bits(), Ordering::Relaxed),
        }
    }
}

impl Entry {
    /// A clip is alive until Kira reports it stopped; a voice or a feed
    /// until its channel is stopped.
    fn alive(&self) -> bool {
        match &self.playing {
            Playing::Clip(h) => h.state() != PlaybackState::Stopped,
            Playing::Stream(h) => h.state() != PlaybackState::Stopped,
            Playing::Voice(_) | Playing::Feed(_) => true,
        }
    }
}

/// Plays sound through Kira: the default device, or any Kira output.
pub struct KiraBackend<B: kira::backend::Backend = DeviceBackend> {
    /// `None` once shut down.
    manager: Option<AudioManager<B>>,
    /// Records the final mix to a file on request.
    capture: capture::Capture,
    channels: HashMap<Channel, Entry>,
    /// Channels mid fade-out, and when each one is due to be dropped.
    fading: Vec<(Channel, Instant)>,
    /// While [`Backend::hold`] is on, the channels started since, paused at
    /// their first sample.
    held: Option<Vec<Channel>>,
    /// Decode every clip whole, never stream: an offline render runs faster
    /// than a stream's decoder thread can keep up with.
    whole: bool,
    /// An offline render's mix, collected from the main track.
    collected: Option<Arc<std::sync::Mutex<Vec<[f32; 2]>>>>,
    next: Channel,
    curve: GainCurve,
    law: PanLaw,
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
    pub fn with_settings(mut settings: AudioManagerSettings<B>) -> Result<Self, B::Error> {
        let (tap, capture) = capture::tap();
        settings.main_track_builder.add_effect(tap);
        Ok(KiraBackend {
            manager: Some(AudioManager::new(settings)?),
            capture,
            channels: HashMap::new(),
            fading: Vec::new(),
            held: None,
            whole: false,
            collected: None,
            next: 1,
            curve: GainCurve::default(),
            law: PanLaw::default(),
        })
    }

    /// How gain changes and fades move on channels started from now on.
    pub fn with_gain_curve(mut self, curve: GainCurve) -> Self {
        self.curve = curve;
        self
    }

    /// The pan law of channels started from now on.
    pub fn with_pan_law(mut self, law: PanLaw) -> Self {
        self.law = law;
        self
    }

    /// Kira's manager, to drive a mock output in tests or for diagnostics.
    /// `None` once shut down.
    pub fn manager_mut(&mut self) -> Option<&mut AudioManager<B>> {
        self.manager.as_mut()
    }
}

impl KiraBackend<kira::backend::mock::MockBackend> {
    /// A backend with no device that mixes only when asked
    /// ([`Backend::render`]), faster than real time: for writing a piece of
    /// music to a file. Every clip decodes whole.
    pub fn offline(sample_rate: u32) -> Result<Self, BackendError> {
        let collected = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut settings = AudioManagerSettings::<kira::backend::mock::MockBackend>::default();
        settings.backend_settings.sample_rate = sample_rate.max(8_000);
        settings
            .main_track_builder
            .add_effect(Collector(collected.clone()));
        let mut backend = KiraBackend::with_settings(settings)
            .map_err(|()| BackendError("the offline mixer did not start".into()))?;
        backend.whole = true;
        backend.collected = Some(collected);
        Ok(backend)
    }
}

/// Collects an offline render's final mix.
struct Collector(Arc<std::sync::Mutex<Vec<[f32; 2]>>>);

impl kira::effect::EffectBuilder for Collector {
    type Handle = ();

    fn build(self) -> (Box<dyn Effect>, ()) {
        (Box::new(self), ())
    }
}

impl Effect for Collector {
    fn process(&mut self, input: &mut [Frame], _dt: f64, _info: &Info) {
        if let Ok(mut out) = self.0.lock() {
            out.extend(input.iter().map(|f| [f.left, f.right]));
        }
    }
}

fn backend_error(e: impl std::fmt::Display) -> BackendError {
    BackendError(e.to_string())
}

/// No fade at all: for a hold, which must not move a sound off its start.
const INSTANT: Tween = Tween {
    start_time: kira::StartTime::Immediate,
    duration: Duration::ZERO,
    easing: kira::Easing::Linear,
};

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
    Panning(if pan.is_finite() {
        pan.clamp(-1.0, 1.0)
    } else {
        0.0
    })
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

/// The region a looping clip repeats: its file's own loop points, or all of
/// it.
fn file_region(points: Option<LoopPoints>) -> Region {
    match points {
        Some(LoopPoints {
            start,
            end: Some(end),
        }) => (PlaybackPosition::Samples(start)..PlaybackPosition::Samples(end)).into(),
        Some(LoopPoints { start, end: None }) => (PlaybackPosition::Samples(start)..).into(),
        None => (..).into(),
    }
}

/// A game's loop region, in seconds, as Kira's.
fn region_of(region: LoopRegion) -> Region {
    let start = PlaybackPosition::Seconds(region.start);
    match region.end {
        Some(end) => (start..PlaybackPosition::Seconds(end)).into(),
        None => (start..).into(),
    }
}

/// Starts a clip on `track` `from_s` seconds in: decoded whole, or
/// streamed if it is big. A looping clip repeats its file's loop points, or
/// all of it, and decodes whole up to a bigger size than a one-shot does.
/// Returns what plays and the clip's length in seconds.
fn play_clip(
    track: &mut TrackHandle,
    bytes: Arc<[u8]>,
    looping: bool,
    pitch: f32,
    from_s: f64,
    whole: bool,
) -> Result<(Playing, f64), BackendError> {
    let stream_over = if whole {
        usize::MAX
    } else if looping {
        LOOP_STREAM_OVER_BYTES
    } else {
        STREAM_OVER_BYTES
    };
    let from = PlaybackPosition::Seconds(from_s.max(0.0));
    if bytes.len() > stream_over {
        let (data, points) = decode::stream(bytes).map_err(backend_error)?;
        let mut data = data.playback_rate(rate(pitch)).start_position(from);
        if looping {
            data = data.loop_region(file_region(points));
        }
        let length = data.duration().as_secs_f64();
        track
            .play(data)
            .map(|h| (Playing::Stream(h), length))
            .map_err(backend_error)
    } else {
        let (data, points) = decode::decode_whole(bytes).map_err(backend_error)?;
        let mut data = data.playback_rate(rate(pitch)).start_position(from);
        if looping {
            data = data.loop_region(file_region(points));
        }
        let length = data.duration().as_secs_f64();
        track
            .play(data)
            .map(|h| (Playing::Clip(h), length))
            .map_err(backend_error)
    }
}

/// A target value and its ramp in milliseconds, in one word, so the audio
/// thread never reads one without the other.
fn pack(value: f32, ms: u32) -> u64 {
    (u64::from(value.to_bits()) << 32) | u64::from(ms)
}

fn unpack(word: u64) -> (f32, u32) {
    (f32::from_bits((word >> 32) as u32), word as u32)
}

/// What the game thread asks of a channel's [`Amp`]: a gain and a pan,
/// each with the ramp to reach it over.
struct AmpControl {
    gain: AtomicU64,
    pan: AtomicU64,
}

/// One value moving in a straight line toward its target.
struct LinearRamp {
    current: f32,
    target: f32,
    step: f32,
    frames_left: u64,
    /// The request last taken up, so an unchanged one is not restarted.
    seen: u64,
}

impl LinearRamp {
    fn new(word: u64) -> LinearRamp {
        let (value, _) = unpack(word);
        LinearRamp {
            current: value,
            target: value,
            step: 0.0,
            frames_left: 0,
            seen: word,
        }
    }

    /// Takes up a new request, if there is one.
    fn poll(&mut self, word: u64, dt: f64) {
        if word == self.seen {
            return;
        }
        self.seen = word;
        let (target, ms) = unpack(word);
        let frames = (f64::from(ms) / 1000.0 / dt).round() as u64;
        self.target = target;
        if frames == 0 {
            self.current = target;
            self.frames_left = 0;
        } else {
            self.step = (target - self.current) / frames as f32;
            self.frames_left = frames;
        }
    }

    fn next(&mut self) -> f32 {
        if self.frames_left > 0 {
            self.frames_left -= 1;
            self.current = if self.frames_left == 0 {
                self.target
            } else {
                self.current + self.step
            };
        }
        self.current
    }
}

/// A channel's gain in straight lines ([`GainCurve::Linear`]) and its pan
/// as a balance ([`PanLaw::Balance`]), whichever of the two the backend
/// uses; the other is left to Kira.
struct Amp {
    control: Arc<AmpControl>,
    gain: LinearRamp,
    pan: LinearRamp,
    linear: bool,
    balance: bool,
}

impl Effect for Amp {
    fn process(&mut self, input: &mut [Frame], dt: f64, _info: &Info) {
        self.gain
            .poll(self.control.gain.load(Ordering::Relaxed), dt);
        self.pan.poll(self.control.pan.load(Ordering::Relaxed), dt);
        for frame in input {
            let gain = if self.linear { self.gain.next() } else { 1.0 };
            let (left, right) = if self.balance {
                let pan = self.pan.next();
                ((1.0 - pan).min(1.0), (1.0 + pan).min(1.0))
            } else {
                (1.0, 1.0)
            };
            frame.left *= gain * left;
            frame.right *= gain * right;
        }
    }
}

impl<B: kira::backend::Backend> KiraBackend<B> {
    /// A channel of its own for `source`: a sub-track carrying `insert`
    /// first, if there is one, then the low-pass, the panner and the gain.
    fn start_channel(
        &mut self,
        _key: &str,
        source: Source,
        looping: bool,
        mix: Mix,
        from_s: f64,
        insert: Option<&Insert>,
    ) -> Result<Channel, BackendError> {
        let (curve, law) = (self.curve, self.law);
        let manager = self
            .manager
            .as_mut()
            .ok_or_else(|| BackendError("the sound device is shut down".into()))?;
        let (cutoff, wet) = lowpass(mix.lowpass_hz);
        let linear = curve == GainCurve::Linear;
        let balance = law == PanLaw::Balance;
        let mut builder = TrackBuilder::new()
            .sound_capacity(1)
            .sub_track_capacity(0)
            .volume(if linear {
                Decibels::IDENTITY
            } else {
                decibels(mix.gain)
            });
        if let Some(insert) = insert {
            builder.add_built_effect(insert_effect(insert));
        }
        let filter = builder.add_effect(
            FilterBuilder::new()
                .cutoff(cutoff)
                .resonance(RESONANCE)
                .mix(wet),
        );
        let panner = (!balance)
            .then(|| builder.add_effect(PanningControlBuilder(Value::Fixed(panning(mix.pan)))));
        let amp = (linear || balance).then(|| {
            let control = Arc::new(AmpControl {
                gain: AtomicU64::new(pack(mix.gain.max(0.0), 0)),
                pan: AtomicU64::new(pack(panning(mix.pan).0, 0)),
            });
            builder.add_built_effect(Box::new(Amp {
                gain: LinearRamp::new(control.gain.load(Ordering::Relaxed)),
                pan: LinearRamp::new(control.pan.load(Ordering::Relaxed)),
                control: control.clone(),
                linear,
                balance,
            }));
            control
        });
        let mut track = manager.add_sub_track(builder).map_err(backend_error)?;
        if self.held.is_some() {
            // Paused before the sound is on it, so not one frame plays
            // until the hold is let go.
            track.pause(INSTANT);
        }
        let mut length_s = None;
        let playing = match source {
            Source::Clip(bytes) => {
                let (playing, length) =
                    play_clip(&mut track, bytes, looping, mix.pitch, from_s, self.whole)?;
                length_s = Some(length);
                playing
            }
            Source::Voice(voice) => {
                let pitch = Arc::new(AtomicU32::new(mix.pitch.to_bits()));
                track
                    .play(LiveVoice::new(voice, pitch.clone()))
                    .map_err(backend_error)?;
                Playing::Voice(pitch)
            }
            Source::Feed(feed) => {
                let pitch = Arc::new(AtomicU32::new(mix.pitch.to_bits()));
                track
                    .play(FeedSound::new(feed, pitch.clone()))
                    .map_err(backend_error)?;
                Playing::Feed(pitch)
            }
        };
        let channel = self.next;
        self.next += 1;
        if let Some(held) = &mut self.held {
            held.push(channel);
        }
        self.channels.insert(
            channel,
            Entry {
                track,
                filter,
                panner,
                amp,
                curve,
                law,
                sent: mix,
                length_s,
                playing,
            },
        );
        Ok(channel)
    }
}

/// An [`Insert`] as a Kira effect, first on the channel's track: the
/// sound's own character, before the game's filter, pan and gain.
struct InsertEffect(CabSpeakersDsp);

impl Effect for InsertEffect {
    fn process(&mut self, input: &mut [Frame], dt: f64, _info: &Info) {
        if !self.0.begin((1.0 / dt) as f32) {
            return;
        }
        for frame in input {
            let (left, right) = self.0.frame(frame.left, frame.right);
            frame.left = left;
            frame.right = right;
        }
    }
}

fn insert_effect(insert: &Insert) -> Box<dyn Effect> {
    match insert {
        Insert::CabSpeakers(cab) => Box::new(InsertEffect(cab.processor())),
    }
}

impl<B: kira::backend::Backend + Send + 'static> Backend for KiraBackend<B> {
    fn name(&self) -> String {
        "Kira".to_string()
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

    fn play_from(
        &mut self,
        key: &str,
        source: Source,
        looping: bool,
        mix: Mix,
        from_s: f64,
    ) -> Result<Channel, BackendError> {
        self.start_channel(key, source, looping, mix, from_s, None)
    }

    fn play_inserted(
        &mut self,
        key: &str,
        source: Source,
        looping: bool,
        mix: Mix,
        from_s: f64,
        insert: &Insert,
    ) -> Result<Channel, BackendError> {
        self.start_channel(key, source, looping, mix, from_s, Some(insert))
    }

    fn set_mix(&mut self, channel: Channel, mix: Mix, ramp_ms: u32) {
        let Some(entry) = self.channels.get_mut(&channel) else {
            return;
        };
        entry.set_gain(mix.gain, ramp_ms);
        entry.set_pan(mix.pan, ramp_ms);
        entry.set_filter(mix.lowpass_hz, ramp_ms);
        entry.set_pitch(mix.pitch, ramp_ms);
        entry.sent = mix;
    }

    fn set_mix_ramped(&mut self, channel: Channel, mix: Mix, ramps: Ramps) {
        let Some(entry) = self.channels.get_mut(&channel) else {
            return;
        };
        let sent = entry.sent;
        if mix.gain != sent.gain {
            entry.set_gain(mix.gain, ramps.gain_ms);
        }
        if mix.pan != sent.pan {
            entry.set_pan(mix.pan, ramps.pan_ms);
        }
        if mix.lowpass_hz != sent.lowpass_hz {
            entry.set_filter(mix.lowpass_hz, ramps.filter_ms);
        }
        if mix.pitch != sent.pitch {
            entry.set_pitch(mix.pitch, ramps.pitch_ms);
        }
        entry.sent = mix;
    }

    fn length_s(&mut self, channel: Channel) -> Option<f64> {
        self.channels.get(&channel)?.length_s
    }

    fn stop(&mut self, channel: Channel, fade_ms: u32) {
        if fade_ms == 0 {
            self.channels.remove(&channel);
        } else if let Some(entry) = self.channels.get_mut(&channel) {
            entry.set_gain(0.0, fade_ms);
            let due = Instant::now() + Duration::from_millis(fade_ms.into());
            self.fading.push((channel, due));
        }
    }

    /// A clip leaves its loop and plays out the rest of its file; a voice or
    /// a feed, which has no rest, fades out.
    fn release(&mut self, channel: Channel, fade_ms: u32) {
        let Some(entry) = self.channels.get_mut(&channel) else {
            return;
        };
        match &mut entry.playing {
            Playing::Clip(h) => h.set_loop_region(None),
            Playing::Stream(h) => h.set_loop_region(None),
            Playing::Voice(_) | Playing::Feed(_) => self.stop(channel, fade_ms),
        }
    }

    fn set_loop_region(&mut self, channel: Channel, region: LoopRegion) {
        let Some(entry) = self.channels.get_mut(&channel) else {
            return;
        };
        match &mut entry.playing {
            Playing::Clip(h) => h.set_loop_region(region_of(region)),
            Playing::Stream(h) => h.set_loop_region(region_of(region)),
            Playing::Voice(_) | Playing::Feed(_) => {}
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

    fn hold(&mut self, held: bool) {
        if held {
            self.held.get_or_insert_with(Vec::new);
            return;
        }
        // One resume after another, with nothing slow between them, so
        // they reach the mixer in the same block.
        for channel in self.held.take().unwrap_or_default() {
            if let Some(entry) = self.channels.get_mut(&channel) {
                entry.track.resume(INSTANT);
            }
        }
    }

    fn is_playing(&mut self, channel: Channel) -> bool {
        self.channels.get(&channel).is_some_and(Entry::alive)
    }

    fn is_offline(&self) -> bool {
        self.collected.is_some()
    }

    fn render(&mut self, frames: usize) -> Option<Vec<[f32; 2]>> {
        let collected = self.collected.clone()?;
        let manager = self.manager.as_mut()?;
        let mock = (manager.backend_mut() as &mut dyn std::any::Any)
            .downcast_mut::<kira::backend::mock::MockBackend>()?;
        // One call mixes one internal buffer, 128 frames by default.
        let mut have = 0;
        while have < frames {
            mock.on_start_processing();
            mock.process();
            let now = collected.lock().map(|c| c.len()).unwrap_or(frames);
            if now == have {
                break;
            }
            have = now;
        }
        collected.lock().ok().map(|mut c| std::mem::take(&mut *c))
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

    fn start_capture(&mut self, path: &std::path::Path) -> Result<(), BackendError> {
        if self.manager.is_none() {
            return Err(BackendError("the sound device is closed".into()));
        }
        self.capture.start(path).map_err(BackendError)
    }

    fn stop_capture(&mut self) -> Option<Result<std::path::PathBuf, BackendError>> {
        self.capture.stop().map(|r| r.map_err(BackendError))
    }

    fn shutdown(&mut self) {
        // A recording in progress keeps what it has.
        if let Some(Err(e)) = self.capture.stop() {
            log::warn!("kira: {e}");
        }
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
        LiveVoice {
            voice,
            pitch,
            rate: 0.0,
            phase: 1.0,
            prev: 0.0,
            next: 0.0,
        }
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

/// How many frames a [`FeedSound`] pulls from its feed at a time: the size of
/// the buffer it makes up front, so nothing allocates on the audio thread. A
/// feed hands over what it has, so this only bounds one pull.
const FEED_BLOCK: usize = 1_024;

/// The sample rates a [`FeedSound`] believes: up to 768 kHz, the top of
/// what audio hardware commonly runs at.
const FEED_RATES: std::ops::RangeInclusive<u32> = 1..=768_000;

/// A feed as a Kira sound: pulled a block at a time into a buffer made
/// up front, then read back `feed_rate * pitch / output_rate` frames per
/// output frame through the same linear resampler as [`LiveVoice`], so the
/// feed plays at its own pitch whatever rate it arrives at, and a pitch
/// change never clicks. Nothing here allocates on the audio thread.
struct FeedSound {
    feed: Arc<dyn Feed>,
    pitch: Arc<AtomicU32>,
    /// The last pull: `len` frames, the first `pos` of them already read.
    block: Box<[[f32; 2]]>,
    len: usize,
    pos: usize,
    /// The sample rate of the block in hand; zero until a block arrives at
    /// one of [`FEED_RATES`].
    rate: f64,
    /// Where the next output frame falls between `prev` and `next`.
    phase: f64,
    prev: Frame,
    next: Frame,
}

impl FeedSound {
    fn new(feed: Arc<dyn Feed>, pitch: Arc<AtomicU32>) -> FeedSound {
        FeedSound {
            feed,
            pitch,
            block: vec![[0.0; 2]; FEED_BLOCK].into_boxed_slice(),
            len: 0,
            pos: 0,
            rate: 0.0,
            phase: 1.0,
            prev: Frame::ZERO,
            next: Frame::ZERO,
        }
    }

    /// The feed's next frame, pulling a new block when the last is used up;
    /// `None` when the feed has nothing more for now. A new block's rate
    /// applies from its first frame on.
    fn pull(&mut self) -> Option<Frame> {
        if self.pos == self.len {
            let read = self.feed.read(&mut self.block);
            self.len = read.frames.min(self.block.len());
            self.pos = 0;
            if self.len == 0 {
                return None;
            }
            // A feed is anyone's code: a rate of 0 would freeze the sound on
            // one frame for good, and a huge one would pull the feed in a
            // tight loop on the shared audio thread. Carrying on at the old
            // rate beats both; with no old rate, the block is dropped.
            if FEED_RATES.contains(&read.sample_rate) {
                self.rate = f64::from(read.sample_rate);
            } else if self.rate == 0.0 {
                self.len = 0;
                return None;
            }
        }
        let [left, right] = self.block[self.pos];
        self.pos += 1;
        Some(Frame::new(left, right))
    }
}

impl Sound for FeedSound {
    fn process(&mut self, out: &mut [Frame], dt: f64, _info: &Info) {
        let per_frame = rate(f32::from_bits(self.pitch.load(Ordering::Relaxed))) * dt;
        // Asking a dry feed again every frame would only repeat the answer:
        // once it has nothing, the rest of this block is silence.
        let mut dry = false;
        for frame in out {
            while !dry && self.phase >= 1.0 {
                match self.pull() {
                    Some(next) => {
                        self.prev = self.next;
                        self.next = next;
                        self.phase -= 1.0;
                    }
                    None => dry = true,
                }
            }
            if dry {
                // Zeroed, so silence carries no DC and the audio that comes
                // back fades in from nothing.
                (self.prev, self.next) = (Frame::ZERO, Frame::ZERO);
                *frame = Frame::ZERO;
            } else {
                *frame = self.prev + (self.next - self.prev) * self.phase as f32;
                self.phase += self.rate * per_frame;
            }
        }
    }

    fn finished(&self) -> bool {
        false
    }
}

impl SoundData for FeedSound {
    type Error = Infallible;
    type Handle = ();

    fn into_sound(self) -> Result<(Box<dyn Sound>, ()), Infallible> {
        Ok((Box::new(self), ()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kira::backend::mock::MockBackend;

    /// A clip between the one-shot and the loop cut: 8 s of mono 16-bit
    /// silence at 44.1 kHz, about 700 KiB.
    fn mid_sized_clip() -> Arc<[u8]> {
        let bytes: Arc<[u8]> = hearken_synth::wav_bytes(&vec![0.0; 8 * 44_100], 44_100).into();
        assert!(bytes.len() > STREAM_OVER_BYTES && bytes.len() < LOOP_STREAM_OVER_BYTES);
        bytes
    }

    #[test]
    fn a_loop_up_to_the_loop_cut_decodes_whole_and_a_one_shot_that_size_streams() {
        let mut b = KiraBackend::<MockBackend>::with_settings(AudioManagerSettings::default())
            .expect("the mock output always opens");
        let looped = b
            .play(
                "engine/idle",
                Source::Clip(mid_sized_clip()),
                true,
                Mix::UNITY,
            )
            .unwrap();
        let once = b
            .play("music/x", Source::Clip(mid_sized_clip()), false, Mix::UNITY)
            .unwrap();
        assert!(matches!(b.channels[&looped].playing, Playing::Clip(_)));
        assert!(matches!(b.channels[&once].playing, Playing::Stream(_)));
    }
}
