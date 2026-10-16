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
//! Dropping a `KiraBackend` without [`Backend::shutdown`] is safe: Kira tells
//! its output thread to stop on drop, without a join.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hearken_synth::Voice;
use kira::effect::filter::{FilterBuilder, FilterHandle};
use kira::effect::panning_control::{PanningControlBuilder, PanningControlHandle};
use kira::info::Info;
use kira::sound::static_sound::StaticSoundHandle;
use kira::sound::streaming::StreamingSoundHandle;
use kira::sound::{FromFileError, PlaybackPosition, PlaybackState, Region, Sound, SoundData};
use kira::track::{TrackBuilder, TrackHandle};
use kira::{
    AudioManager, AudioManagerSettings, Capacities, Decibels, DefaultBackend, Frame, Panning,
    Tween, Value,
};

use crate::backend::{Backend, BackendError, Channel, Source};
use crate::decode::{self, LoopPoints};
use crate::feed::Feed;
use crate::types::{LoopRegion, Mix};

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
    /// A feed, which never ends on its own either; its pitch, as `f32` bits.
    Feed(Arc<AtomicU32>),
}

struct Entry {
    track: TrackHandle,
    filter: FilterHandle,
    panner: PanningControlHandle,
    playing: Playing,
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

/// Starts a clip on `track`: decoded whole, or streamed if it is big. A
/// looping clip repeats its file's loop points, or all of it.
fn play_clip(
    track: &mut TrackHandle,
    bytes: Arc<[u8]>,
    looping: bool,
    pitch: f32,
) -> Result<Playing, BackendError> {
    if bytes.len() > STREAM_OVER_BYTES {
        let (data, points) = decode::stream(bytes).map_err(backend_error)?;
        let mut data = data.playback_rate(rate(pitch));
        if looping {
            data = data.loop_region(file_region(points));
        }
        track.play(data).map(Playing::Stream).map_err(backend_error)
    } else {
        let (data, points) = decode::decode_whole(bytes).map_err(backend_error)?;
        let mut data = data.playback_rate(rate(pitch));
        if looping {
            data = data.loop_region(file_region(points));
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
            Playing::Voice(pitch) | Playing::Feed(pitch) => {
                pitch.store(mix.pitch.to_bits(), Ordering::Relaxed)
            }
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
