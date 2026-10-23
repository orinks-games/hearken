//! The pool of positioned sounds: generation-checked handles over a [`Backend`].

use crate::backend::{Backend, BackendError, Channel, Source};
use crate::clips::{ClipSource, LooseClips};
use crate::feed::Feed;
use crate::silent::SilentBackend;
use crate::spatial::{place, SpatialConfig};
use crate::types::{Bus, Listener, LoopRegion, Mix, Ramps, Start, Vec3};
use crate::voicing::Insert;
use hearken_synth::{builtin_wav, Params, Recipe, Voice};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::mpsc::Receiver;
use std::sync::Arc;

/// A handle to a sound in the pool. Stale once its slot is reused: lookups
/// compare the slot's generation, so an old handle finds nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Sound {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

impl Sound {
    /// A handle that never matches a live sound.
    pub const NONE: Sound = Sound {
        index: u32::MAX,
        generation: 0,
    };
}

/// A snapshot of one live sound, from [`Audio::live`].
#[derive(Clone)]
pub struct LiveSound {
    pub sound: Sound,
    pub key: String,
    pub bus: Bus,
    pub pos: Option<Vec3>,
    pub looping: bool,
    pub paused: bool,
    pub params: Option<Params>,
}

impl std::fmt::Debug for LiveSound {
    // `hearken_synth::Params` does not implement `Debug` (it holds fundsp's
    // `Shared`, which does not either), so this is written by hand instead
    // of derived.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveSound")
            .field("sound", &self.sound)
            .field("key", &self.key)
            .field("bus", &self.bus)
            .field("pos", &self.pos)
            .field("looping", &self.looping)
            .field("paused", &self.paused)
            .field("has_params", &self.params.is_some())
            .finish()
    }
}

/// How long a change ramps before it reaches the backend. Every value is a
/// default a game can change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fades {
    pub bed_ms: u32,
    pub music_ms: u32,
    pub duck_ms: u32,
    pub volume_ms: u32,
    pub move_ms: u32,
}

impl Default for Fades {
    fn default() -> Fades {
        Fades {
            bed_ms: 800,
            music_ms: 500,
            duck_ms: 150,
            volume_ms: 50,
            move_ms: 30,
        }
    }
}

pub(crate) struct Slot {
    pub(crate) generation: u32,
    pub(crate) live: Option<Live>,
}

pub(crate) struct Live {
    pub(crate) key: String,
    pub(crate) channel: Channel,
    pub(crate) bus: Bus,
    pub(crate) pos: Option<Vec3>,
    pub(crate) volume: f32,
    /// The pan of a sound with no position; a positioned sound takes its
    /// pan from where it is instead.
    pub(crate) pan: f32,
    pub(crate) pitch: f32,
    pub(crate) looping: bool,
    /// The region a looping clip repeats, when the game chose one; `None`
    /// repeats the file's own loop points, or the whole file.
    pub(crate) region: Option<LoopRegion>,
    pub(crate) voice: Option<(Recipe, Params)>,
    /// The bytes of a clip the game handed over itself
    /// ([`Audio::play_clip`]), kept so a deferred backend's replay plays the
    /// same clip instead of asking the clip source for it.
    pub(crate) clip: Option<Arc<[u8]>>,
    /// Kept so a deferred backend's replay can hand the new backend the same
    /// feed.
    pub(crate) feed: Option<Arc<dyn Feed>>,
    pub(crate) paused: bool,
    pub(crate) paused_by_all: bool,
    pub(crate) occluded: bool,
    pub(crate) sent: Mix,
}

impl Live {
    /// A sound at unity pitch, not yet on a backend: `channel` and `sent` are
    /// filled in once one takes it.
    fn new(key: &str, pos: Option<Vec3>, bus: Bus, looping: bool, volume: f32) -> Live {
        Live {
            key: key.to_string(),
            channel: 0,
            bus,
            pos,
            volume,
            pan: 0.0,
            pitch: 1.0,
            looping,
            region: None,
            voice: None,
            clip: None,
            feed: None,
            paused: false,
            paused_by_all: false,
            occluded: false,
            sent: Mix::UNITY,
        }
    }
}

fn slot_ref(slots: &[Slot], sound: Sound) -> Option<&Live> {
    let slot = slots.get(sound.index as usize)?;
    (slot.generation == sound.generation)
        .then_some(slot.live.as_ref())
        .flatten()
}

pub(crate) fn slot_mut(slots: &mut [Slot], sound: Sound) -> Option<&mut Live> {
    let slot = slots.get_mut(sound.index as usize)?;
    if slot.generation != sound.generation {
        return None;
    }
    slot.live.as_mut()
}

/// Every bus's own volume and the duck, folded into one path -- `start`,
/// `resend` and `update` all compute a sound's gain through
/// [`bus_gain_of`], so a positioned sound on a ducked bus never flips
/// between ducked and unducked depending on which of them last touched it.
pub(crate) struct Mixing {
    pub(crate) bus_volumes: HashMap<Bus, f32>,
    /// The insert each sound started on a bus carries.
    pub(crate) inserts: HashMap<Bus, Insert>,
    pub(crate) duck_level: f32,
    pub(crate) duck_buses: HashSet<Bus>,
}

impl Default for Mixing {
    fn default() -> Mixing {
        Mixing {
            bus_volumes: HashMap::new(),
            inserts: HashMap::new(),
            duck_level: 1.0, // 1.0 is no duck
            duck_buses: HashSet::new(),
        }
    }
}

fn bus_gain_of(mixing: &Mixing, bus: Bus) -> f32 {
    let master = *mixing.bus_volumes.get(&Bus::MASTER).unwrap_or(&1.0);
    let mut gain = if bus == Bus::MASTER {
        master
    } else {
        master * *mixing.bus_volumes.get(&bus).unwrap_or(&1.0)
    };
    if mixing.duck_buses.contains(&bus) {
        gain *= mixing.duck_level;
    }
    gain
}

/// Clamps `value` into `min..=max`, or keeps `current` unchanged when
/// `value` is NaN or infinite. A non-finite position or facing already
/// cannot reach a backend (Task 2's `place`); this is the same guarantee for
/// volume, pitch and bus volume, so a bad value from game code cannot poison
/// a sound (or, on `Bus::MASTER`, every sound) and cannot defeat the
/// send-only-on-change check below (`NaN != NaN` always).
pub(crate) fn clamped_or(current: f32, value: f32, min: f32, max: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        current
    }
}

fn compute_mix(listener: &Listener, spatial: &SpatialConfig, bus_gain: f32, live: &Live) -> Mix {
    let mut gain = live.volume * bus_gain;
    let (mut pan, mut lowpass_hz, mut relative) = (live.pan, None, None);
    if let Some(pos) = live.pos {
        let p = place(listener, pos, live.occluded, spatial);
        gain *= p.gain;
        (pan, lowpass_hz, relative) = (p.pan, p.lowpass_hz, Some(p.relative));
    }
    Mix {
        gain,
        pan,
        pitch: live.pitch,
        lowpass_hz,
        relative,
    }
}

/// Where a [`Start`] begins in its clip: a NaN, infinite or negative offset
/// is the top.
fn offset(start: Start) -> f64 {
    if start.from_s.is_finite() {
        start.from_s.max(0.0)
    } else {
        0.0
    }
}

/// A pool of positioned sounds, buses and live voices over a [`Backend`].
pub struct Audio {
    pub spatial: SpatialConfig,
    pub fades: Fades,
    pub(crate) backend: Box<dyn Backend>,
    clips: Box<dyn ClipSource>,
    pub(crate) slots: Vec<Slot>,
    free: Vec<u32>,
    listener: Listener,
    pub(crate) mixing: Mixing,
    missing: BTreeSet<String>,
    /// Whether a missing or refused clip plays its built-in default.
    fallback: bool,
    pub(crate) bed: Option<(String, Sound)>,
    pub(crate) music: Option<(String, bool, Sound)>,
    pub(crate) opening: Option<Receiver<Result<Box<dyn Backend>, String>>>,
    pub(crate) notice: Option<String>,
    pub(crate) shut_down: bool,
    /// Keys of sounds started, kept only for an agent (`keep_heard`).
    heard: Option<Vec<String>>,
}

impl Audio {
    /// A pool over `backend`, loading clips from `clips`.
    pub fn new(backend: Box<dyn Backend>, clips: Box<dyn ClipSource>) -> Audio {
        Audio {
            spatial: SpatialConfig::default(),
            fades: Fades::default(),
            backend,
            clips,
            slots: Vec::new(),
            free: Vec::new(),
            listener: Listener::default(),
            mixing: Mixing::default(),
            missing: BTreeSet::new(),
            fallback: true,
            bed: None,
            music: None,
            opening: None,
            notice: None,
            shut_down: false,
            heard: None,
        }
    }

    /// A pool that plays nothing and loads no clips.
    pub fn silent() -> Audio {
        Audio::new(
            Box::new(SilentBackend::default()),
            Box::new(LooseClips::none()),
        )
    }

    /// Plays `key` once on `bus`.
    pub fn play(&mut self, key: &str, bus: Bus) -> Sound {
        self.start(key, None, bus, false, None, 1.0)
    }

    /// Plays `key` once on `bus`, positioned at `pos`.
    pub fn play_at(&mut self, key: &str, pos: Vec3, bus: Bus) -> Sound {
        self.start(key, Some(pos), bus, false, None, 1.0)
    }

    /// Starts `key` looping on `bus`, optionally positioned.
    pub fn start_loop(&mut self, key: &str, pos: Option<Vec3>, bus: Bus) -> Sound {
        self.start(key, pos, bus, true, None, 1.0)
    }

    /// Starts `key` looping only `region` of its clip: the clip plays from its
    /// start, repeats the region until [`Audio::release`] or
    /// [`Audio::stop`], and on release plays on through the rest of the
    /// file. Overrides any loop points the file carries; an invalid region
    /// is ignored and the clip loops as [`Audio::start_loop`] would.
    pub fn start_loop_region(
        &mut self,
        key: &str,
        region: LoopRegion,
        pos: Option<Vec3>,
        bus: Bus,
    ) -> Sound {
        let sound = self.start(key, pos, bus, true, None, 1.0);
        self.apply_region(sound, region);
        sound
    }

    /// Sets the region a looping `sound` repeats; an invalid one is ignored
    /// with a warning.
    fn apply_region(&mut self, sound: Sound, region: LoopRegion) {
        let Some(live) = slot_mut(&mut self.slots, sound) else {
            return;
        };
        if !region.is_valid() {
            log::warn!("sound {}: ignoring loop region {region:?}", live.key);
            return;
        }
        live.region = Some(region);
        let channel = live.channel;
        self.backend.set_loop_region(channel, region);
    }

    /// Starts a live synth voice from `recipe`, looping until stopped.
    pub fn start_voice(
        &mut self,
        recipe: Recipe,
        params: &[(&'static str, f32)],
        pos: Option<Vec3>,
        bus: Bus,
    ) -> Sound {
        self.start(
            "voice",
            pos,
            bus,
            true,
            Some((recipe, Params::new(params))),
            1.0,
        )
    }

    /// Starts a feed -- audio the backend pulls, such as a radio stream --
    /// looping until stopped, as an ordinary sound: bus volume, the duck,
    /// pause, fades, position and occlusion all apply.
    pub fn start_feed(&mut self, feed: Arc<dyn Feed>, pos: Option<Vec3>, bus: Bus) -> Sound {
        self.start_feed_at_volume(feed, pos, bus, 1.0)
    }

    /// [`start_feed`](Self::start_feed) at a starting `volume` (0..1)
    /// instead of `1.0`: a station that should fade in from silence starts
    /// silent, rather than being turned down after it has started.
    pub fn start_feed_at_volume(
        &mut self,
        feed: Arc<dyn Feed>,
        pos: Option<Vec3>,
        bus: Bus,
        volume: f32,
    ) -> Sound {
        let live = Live {
            feed: Some(feed.clone()),
            ..Live::new("feed", pos, bus, true, volume.clamp(0.0, 1.0))
        };
        self.launch(Source::Feed(feed), live)
    }

    /// Starts `key` on `bus` at a given starting `volume`, instead of the
    /// usual `1.0` -- for a bed that should fade in from silence.
    pub(crate) fn start_at_volume(
        &mut self,
        key: &str,
        bus: Bus,
        looping: bool,
        volume: f32,
    ) -> Sound {
        self.start(key, None, bus, looping, None, volume)
    }

    /// Whether `key` has a clip of its own, without playing it or noting it
    /// missing: so a game can play a recording when one is there and fall
    /// back to a live synth voice of its own, not the category default, when
    /// it is not.
    pub fn has_clip(&mut self, key: &str) -> bool {
        self.clips.load(key).is_some()
    }

    /// Whether a clip that is missing, or that the backend refuses (it
    /// will not decode), plays the key's built-in default instead (`true`,
    /// the default). Off, such a sound plays nothing: it is logged the first
    /// time, listed in [`Audio::missing_clips`], and -- unlike with the
    /// default on -- not remembered, so the clip source is asked again next
    /// time and a clip that turns up later (a sound the game generates and
    /// registers as it runs) is heard.
    pub fn set_default_fallback(&mut self, on: bool) {
        self.fallback = on;
    }

    /// `key`'s clip bytes, falling back to its built-in default (and noting
    /// it missing) the same way `start` does -- so a sound replayed on a
    /// newly opened backend gets the same clip it would if started fresh.
    /// `None` when the clip is missing and the fallback is off.
    pub(crate) fn clip_or_default(&mut self, key: &str) -> Option<Arc<[u8]>> {
        // Already known bad (missing, or rejected by the backend last time):
        // go straight to the default without asking the clip source again.
        if self.fallback && self.missing.contains(key) {
            return Some(builtin_wav(key));
        }
        match self.clips.load(key) {
            Some(bytes) => Some(bytes),
            None => {
                self.note_missing(key);
                self.fallback.then(|| builtin_wav(key))
            }
        }
    }

    /// Plays `source` on the backend, falling back to `key`'s built-in
    /// default clip (and noting `key` missing) if the backend refuses it --
    /// unless `source` is already a live voice, which has no clip to fall
    /// back to: retrying would start an unrequested, unstoppable loop
    /// (nobody holds a handle to stop it) named after a key that was never
    /// missing. A feed is the same: it has no clip either. Shared by
    /// `launch` and, for the replay after a deferred backend opens, `open`'s
    /// `swap_backend`.
    pub(crate) fn play_with_fallback(
        &mut self,
        key: &str,
        source: Source,
        looping: bool,
        mix: Mix,
        from_s: f64,
        bus: Bus,
    ) -> Option<Channel> {
        let is_live = matches!(source, Source::Voice(_) | Source::Feed(_));
        match self.play_on_backend(key, source, looping, mix, from_s, bus) {
            Ok(channel) => Some(channel),
            Err(err) => {
                if is_live {
                    log::error!("backend failed to play live source {key:?}: {err}");
                    return None;
                }
                if self.note_missing(key) {
                    log::error!("backend failed to play {key:?}: {err}");
                }
                if !self.fallback {
                    return None;
                }
                match self.play_on_backend(
                    key,
                    Source::Clip(builtin_wav(key)),
                    looping,
                    mix,
                    0.0,
                    bus,
                ) {
                    Ok(channel) => Some(channel),
                    Err(err) => {
                        log::error!("backend failed to play {key:?}'s default too: {err}");
                        None
                    }
                }
            }
        }
    }

    /// A sound on a bus with an insert goes through
    /// [`Backend::play_inserted`]; otherwise `from_s` past the top goes
    /// through [`Backend::play_from`], and from the top through
    /// [`Backend::play`], as it always has.
    fn play_on_backend(
        &mut self,
        key: &str,
        source: Source,
        looping: bool,
        mix: Mix,
        from_s: f64,
        bus: Bus,
    ) -> Result<Channel, BackendError> {
        if let Some(insert) = self.mixing.inserts.get(&bus) {
            self.backend
                .play_inserted(key, source, looping, mix, from_s.max(0.0), insert)
        } else if from_s > 0.0 {
            self.backend.play_from(key, source, looping, mix, from_s)
        } else {
            self.backend.play(key, source, looping, mix)
        }
    }

    fn start(
        &mut self,
        key: &str,
        pos: Option<Vec3>,
        bus: Bus,
        looping: bool,
        voice: Option<(Recipe, Params)>,
        volume: f32,
    ) -> Sound {
        let source = match &voice {
            Some((recipe, params)) => Source::Voice(Voice::with_params(*recipe, params.clone())),
            None => match self.clip_or_default(key) {
                Some(bytes) => Source::Clip(bytes),
                None => return Sound::NONE,
            },
        };

        let live = Live {
            voice,
            ..Live::new(key, pos, bus, looping, volume)
        };
        self.launch(source, live)
    }

    /// Starts `key` on `bus` as `start` says: its own volume, pan and pitch
    /// from the first sample, looping or not, a loop region, and an offset
    /// into the clip. The clip comes from the pool's clip source, the same
    /// as [`Audio::play`].
    pub fn start_with(&mut self, key: &str, bus: Bus, start: Start) -> Sound {
        let Some(bytes) = self.clip_or_default(key) else {
            return Sound::NONE;
        };
        self.start_clip(key, bytes, false, bus, start)
    }

    /// Plays clip `bytes` the game found itself (its own packs, a file),
    /// named `key` for logs and [`Audio::live`], as `start` says. The clip
    /// source is not asked. A clip the backend refuses falls back the way a
    /// refused clip from the source does (see
    /// [`Audio::set_default_fallback`]).
    pub fn play_clip(&mut self, key: &str, bytes: Arc<[u8]>, bus: Bus, start: Start) -> Sound {
        self.start_clip(key, bytes, true, bus, start)
    }

    /// [`Audio::play_clip`], but a clip the backend refuses (one that will
    /// not decode) is an error, never a default and never silence the
    /// caller cannot see: for a player's own music file, which a playlist
    /// skips when it cannot play.
    pub fn try_play_clip(
        &mut self,
        key: &str,
        bytes: Arc<[u8]>,
        bus: Bus,
        start: Start,
    ) -> Result<Sound, BackendError> {
        let live = self.live_for(key, Some(bytes.clone()), bus, start);
        let mix = self.mix_of(&live);
        let from_s = offset(start);
        match self.play_on_backend(key, Source::Clip(bytes), live.looping, mix, from_s, bus) {
            Ok(channel) => Ok(self.adopt(live, channel, mix, start)),
            Err(err) => {
                if self.note_missing(key) {
                    log::error!("backend failed to play {key:?}: {err}");
                }
                Err(err)
            }
        }
    }

    /// Reads the file at `path` and plays it as [`Audio::try_play_clip`]
    /// does, keyed by its path.
    pub fn try_play_file(
        &mut self,
        path: &std::path::Path,
        bus: Bus,
        start: Start,
    ) -> Result<Sound, PlayError> {
        let bytes: Arc<[u8]> = std::fs::read(path).map_err(PlayError::Read)?.into();
        let key = path.to_string_lossy().into_owned();
        self.try_play_clip(&key, bytes, bus, start)
            .map_err(PlayError::Refused)
    }

    /// The slot-to-be for a clip started as `start` says.
    fn live_for(&self, key: &str, clip: Option<Arc<[u8]>>, bus: Bus, start: Start) -> Live {
        let mut live = Live::new(
            key,
            None,
            bus,
            start.looping,
            clamped_or(1.0, start.volume, 0.0, 1.0),
        );
        live.pan = clamped_or(0.0, start.pan, -1.0, 1.0);
        live.pitch = clamped_or(1.0, start.pitch, 0.05, 4.0);
        live.clip = clip;
        live
    }

    /// Gives a clip the backend took its slot, and its loop region.
    fn adopt(&mut self, mut live: Live, channel: Channel, mix: Mix, start: Start) -> Sound {
        live.channel = channel;
        live.sent = mix;
        let sound = self.alloc(live);
        if let (true, Some(region)) = (start.looping, start.region) {
            self.apply_region(sound, region);
        }
        sound
    }

    fn start_clip(
        &mut self,
        key: &str,
        bytes: Arc<[u8]>,
        keep: bool,
        bus: Bus,
        start: Start,
    ) -> Sound {
        let live = self.live_for(key, keep.then(|| bytes.clone()), bus, start);
        let mix = self.mix_of(&live);
        let from_s = offset(start);
        match self.play_with_fallback(key, Source::Clip(bytes), live.looping, mix, from_s, bus) {
            Some(channel) => self.adopt(live, channel, mix, start),
            None => Sound::NONE,
        }
    }

    /// Plays `source` for `live` and gives it a slot, or returns
    /// [`Sound::NONE`] if the backend would not take it.
    fn launch(&mut self, source: Source, mut live: Live) -> Sound {
        let mix = self.mix_of(&live);

        match self.play_with_fallback(&live.key, source, live.looping, mix, 0.0, live.bus) {
            Some(channel) => {
                live.channel = channel;
                live.sent = mix;
                self.alloc(live)
            }
            None => Sound::NONE,
        }
    }

    /// Stops `sound`, fading over `fade_ms`; the handle is freed at once
    /// since the backend owns the fade.
    pub fn stop(&mut self, sound: Sound, fade_ms: u32) {
        let Some(channel) = slot_ref(&self.slots, sound).map(|live| live.channel) else {
            return;
        };
        self.backend.stop(channel, fade_ms);
        self.free_slot(sound.index);
    }

    /// Lets a looping `sound` go: a clip leaves its loop and plays on to the
    /// end of its file -- the release a horn or a bell recorded after its
    /// loop region, or after the loop points the file carries -- then ends
    /// by itself; the handle stays valid until it does. A live voice or a
    /// feed, which has no tail, stops over `fade_ms`, as does a clip on a
    /// backend that cannot play one out. A one-shot is left alone.
    pub fn release(&mut self, sound: Sound, fade_ms: u32) {
        let Some(live) = slot_mut(&mut self.slots, sound) else {
            return;
        };
        if !live.looping {
            return;
        }
        live.looping = false;
        let channel = live.channel;
        self.backend.release(channel, fade_ms);
    }

    /// Pauses `sound`. Clears `paused_by_all`: an explicit pause is the
    /// game's own doing, not `pause_all`'s, so a later `resume_all` must not
    /// touch it. `pause_all` itself restores the flag right after calling
    /// this.
    pub fn pause(&mut self, sound: Sound) {
        let Some(live) = slot_mut(&mut self.slots, sound) else {
            return;
        };
        live.paused = true;
        live.paused_by_all = false;
        let channel = live.channel;
        self.backend.pause(channel, true);
    }

    /// Resumes a paused `sound`, and clears `paused_by_all` the same way
    /// `pause` does.
    pub fn resume(&mut self, sound: Sound) {
        let Some(live) = slot_mut(&mut self.slots, sound) else {
            return;
        };
        live.paused = false;
        live.paused_by_all = false;
        let channel = live.channel;
        self.backend.pause(channel, false);
    }

    /// Sets `sound`'s volume, clamped to `0..=1`.
    pub fn set_volume(&mut self, sound: Sound, volume: f32) {
        if let Some(live) = slot_mut(&mut self.slots, sound) {
            live.volume = clamped_or(live.volume, volume, 0.0, 1.0);
        }
        self.resend(sound, self.fades.move_ms);
    }

    /// Sets `sound`'s pitch, clamped to `0.05..=4`.
    pub fn set_pitch(&mut self, sound: Sound, pitch: f32) {
        if let Some(live) = slot_mut(&mut self.slots, sound) {
            live.pitch = clamped_or(live.pitch, pitch, 0.05, 4.0);
        }
        self.resend(sound, self.fades.move_ms);
    }

    /// Sets `sound`'s volume, clamped to `0..=1`, ramping over `ramp_ms`
    /// (`0` is at once). Anything else about the sound that was due to
    /// change moves over the usual `fades.move_ms`.
    pub fn set_volume_over(&mut self, sound: Sound, volume: f32, ramp_ms: u32) {
        if let Some(live) = slot_mut(&mut self.slots, sound) {
            live.volume = clamped_or(live.volume, volume, 0.0, 1.0);
        }
        let ramps = Ramps {
            gain_ms: ramp_ms,
            ..Ramps::all(self.fades.move_ms)
        };
        self.resend_ramped(sound, ramps);
    }

    /// Sets `sound`'s pitch, clamped to `0.05..=4`, sliding over `ramp_ms`
    /// (`0` is at once).
    pub fn set_pitch_over(&mut self, sound: Sound, pitch: f32, ramp_ms: u32) {
        if let Some(live) = slot_mut(&mut self.slots, sound) {
            live.pitch = clamped_or(live.pitch, pitch, 0.05, 4.0);
        }
        let ramps = Ramps {
            pitch_ms: ramp_ms,
            ..Ramps::all(self.fades.move_ms)
        };
        self.resend_ramped(sound, ramps);
    }

    /// Sets the pan of a `sound` with no position, clamped to `-1..=1`,
    /// over `fades.move_ms`. A positioned sound keeps the pan its position
    /// gives it; this applies if its position is dropped.
    pub fn set_pan(&mut self, sound: Sound, pan: f32) {
        self.set_pan_over(sound, pan, self.fades.move_ms);
    }

    /// [`Audio::set_pan`] over `ramp_ms` (`0` is at once).
    pub fn set_pan_over(&mut self, sound: Sound, pan: f32, ramp_ms: u32) {
        if let Some(live) = slot_mut(&mut self.slots, sound) {
            live.pan = clamped_or(live.pan, pan, -1.0, 1.0);
        }
        let ramps = Ramps {
            pan_ms: ramp_ms,
            ..Ramps::all(self.fades.move_ms)
        };
        self.resend_ramped(sound, ramps);
    }

    /// How long `sound`'s clip runs, in seconds, when the backend can tell.
    pub fn length_s(&mut self, sound: Sound) -> Option<f64> {
        let channel = slot_ref(&self.slots, sound)?.channel;
        self.backend.length_s(channel)
    }

    /// Moves `sound`, or drops its position entirely.
    pub fn set_position(&mut self, sound: Sound, pos: Option<Vec3>) {
        if let Some(live) = slot_mut(&mut self.slots, sound) {
            live.pos = pos;
        }
        self.resend(sound, self.fades.move_ms);
    }

    /// Sets a named parameter on `sound`'s live voice; `false` if `sound`
    /// has no voice or the name is unknown.
    pub fn set_voice_param(&mut self, sound: Sound, name: &str, value: f32) -> bool {
        slot_ref(&self.slots, sound)
            .and_then(|live| live.voice.as_ref())
            .is_some_and(|(_, params)| params.set(name, value))
    }

    /// Whether `sound` is still alive in the pool.
    pub fn is_playing(&self, sound: Sound) -> bool {
        slot_ref(&self.slots, sound).is_some()
    }

    /// Moves the listener.
    pub fn set_listener(&mut self, pos: Vec3, facing: f32) {
        self.listener = Listener { pos, facing };
    }

    /// The listener's current position and facing.
    pub fn listener(&self) -> Listener {
        self.listener
    }

    /// Sets a bus's volume, clamped to `0..=1`; `Bus::MASTER` scales every
    /// bus.
    pub fn set_bus_volume(&mut self, bus: Bus, volume: f32) {
        let volume = clamped_or(self.bus_volume(bus), volume, 0.0, 1.0);
        self.mixing.bus_volumes.insert(bus, volume);
        let affected = self.live_sounds(|live| bus == Bus::MASTER || live.bus == bus);
        for sound in affected {
            self.resend(sound, self.fades.volume_ms);
        }
    }

    /// Every live sound whose [`Live`] satisfies `pred`, snapshotted as
    /// handles before the caller mutates the pool through them.
    pub(crate) fn live_sounds(&self, pred: impl Fn(&Live) -> bool) -> Vec<Sound> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, slot)| {
                let live = slot.live.as_ref()?;
                pred(live).then_some(Sound {
                    index: i as u32,
                    generation: slot.generation,
                })
            })
            .collect()
    }

    /// Gives every sound started on `bus` from now on `insert` (a radio's
    /// cab speakers, say), run on its own track before its mix; `None`
    /// takes it off. Sounds already playing keep what they started with,
    /// so a game sets this before it starts them. The insert is a shared
    /// handle: changing it (its amount, a window) reaches every sound that
    /// carries it at once. A backend that cannot run inserts plays the
    /// sounds as they are.
    pub fn set_bus_insert(&mut self, bus: Bus, insert: Option<Insert>) {
        match insert {
            Some(insert) => self.mixing.inserts.insert(bus, insert),
            None => self.mixing.inserts.remove(&bus),
        };
    }

    /// The insert sounds started on `bus` carry, if any.
    pub fn bus_insert(&self, bus: Bus) -> Option<&Insert> {
        self.mixing.inserts.get(&bus)
    }

    /// A bus's volume; `1.0` until set.
    pub fn bus_volume(&self, bus: Bus) -> f32 {
        *self.mixing.bus_volumes.get(&bus).unwrap_or(&1.0)
    }

    /// Runs one frame: checks whether a deferred backend has opened, reaps
    /// ended channels and re-mixes positioned sounds against the listener
    /// and `occluded`.
    pub fn update(&mut self, occluded: &dyn Fn(Vec3) -> bool) {
        self.poll_open();
        self.backend.update();
        let listener = self.listener;
        let spatial = self.spatial;
        let mixing = &self.mixing;
        let backend = &mut self.backend;
        let mut freed = Vec::new();
        let mut sends: Vec<(Channel, Mix)> = Vec::new();

        for (i, slot) in self.slots.iter_mut().enumerate() {
            let Some(live) = slot.live.as_mut() else {
                continue;
            };
            if !live.paused && !backend.is_playing(live.channel) {
                freed.push(i as u32);
                continue;
            }
            if let Some(pos) = live.pos {
                live.occluded = occluded(pos);
                let gain = bus_gain_of(mixing, live.bus);
                let mix = compute_mix(&listener, &spatial, gain, live);
                if mix != live.sent {
                    live.sent = mix;
                    sends.push((live.channel, mix));
                }
            }
        }

        for index in freed {
            self.free_slot(index);
        }
        let move_ms = self.fades.move_ms;
        for (channel, mix) in sends {
            self.backend.set_mix(channel, mix, move_ms);
        }
    }

    /// Every key that has fallen back to its default at least once (or, with
    /// the fallback off, played nothing), sorted.
    pub fn missing_clips(&self) -> Vec<String> {
        self.missing.iter().cloned().collect()
    }

    /// A snapshot of every live sound.
    pub fn live(&self) -> Vec<LiveSound> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, slot)| {
                let live = slot.live.as_ref()?;
                Some(LiveSound {
                    sound: Sound {
                        index: i as u32,
                        generation: slot.generation,
                    },
                    key: live.key.clone(),
                    bus: live.bus,
                    pos: live.pos,
                    looping: live.looping,
                    paused: live.paused,
                    params: live.voice.as_ref().map(|(_, params)| params.clone()),
                })
            })
            .collect()
    }

    /// The backend's name.
    pub fn backend_name(&self) -> String {
        self.backend.name()
    }

    fn mix_of(&self, live: &Live) -> Mix {
        compute_mix(&self.listener, &self.spatial, self.bus_gain(live.bus), live)
    }

    fn bus_gain(&self, bus: Bus) -> f32 {
        bus_gain_of(&self.mixing, bus)
    }

    pub(crate) fn resend(&mut self, sound: Sound, ramp_ms: u32) {
        let Some(bus) = slot_ref(&self.slots, sound).map(|live| live.bus) else {
            return;
        };
        let gain = self.bus_gain(bus);
        let listener = self.listener;
        let spatial = self.spatial;
        let Some(live) = slot_mut(&mut self.slots, sound) else {
            return;
        };
        let mix = compute_mix(&listener, &spatial, gain, live);
        if mix == live.sent {
            return;
        }
        live.sent = mix;
        let channel = live.channel;
        self.backend.set_mix(channel, mix, ramp_ms);
    }

    /// [`Audio::resend`] with a ramp of its own for each part of the mix.
    fn resend_ramped(&mut self, sound: Sound, ramps: Ramps) {
        let Some(bus) = slot_ref(&self.slots, sound).map(|live| live.bus) else {
            return;
        };
        let gain = self.bus_gain(bus);
        let listener = self.listener;
        let spatial = self.spatial;
        let Some(live) = slot_mut(&mut self.slots, sound) else {
            return;
        };
        let mix = compute_mix(&listener, &spatial, gain, live);
        if mix == live.sent {
            return;
        }
        live.sent = mix;
        let channel = live.channel;
        self.backend.set_mix_ramped(channel, mix, ramps);
    }

    /// Records `key` as missing; returns whether this is the first time.
    fn note_missing(&mut self, key: &str) -> bool {
        let newly_missing = self.missing.insert(key.to_string());
        if newly_missing {
            if self.fallback {
                log::warn!("sound {key} is missing; playing its default");
            } else {
                log::warn!("sound {key} is missing or will not play; playing nothing");
            }
        }
        newly_missing
    }

    /// Starts keeping the key of every sound that starts, for an agent
    /// listening to the game (`take_heard`). Off until asked, so a player's
    /// game keeps nothing.
    pub fn keep_heard(&mut self) {
        self.heard.get_or_insert_with(Vec::new);
    }

    /// The keys of the sounds started since the last call, a loop marked
    /// "(loop)". Empty unless `keep_heard` was called.
    pub fn take_heard(&mut self) -> Vec<String> {
        self.heard.as_mut().map(std::mem::take).unwrap_or_default()
    }

    fn alloc(&mut self, live: Live) -> Sound {
        if let Some(heard) = &mut self.heard {
            // ponytail: unbounded until taken; an agent drains it every call.
            heard.push(if live.looping {
                format!("{} (loop)", live.key)
            } else {
                live.key.clone()
            });
        }
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.live = Some(live);
            Sound {
                index,
                generation: slot.generation,
            }
        } else {
            let index = self.slots.len() as u32;
            self.slots.push(Slot {
                generation: 0,
                live: Some(live),
            });
            Sound {
                index,
                generation: 0,
            }
        }
    }

    pub(crate) fn free_slot(&mut self, index: u32) {
        let slot = &mut self.slots[index as usize];
        slot.live = None;
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(index);
    }
}

/// Why [`Audio::try_play_file`] played nothing.
#[derive(Debug)]
pub enum PlayError {
    /// The file could not be read.
    Read(std::io::Error),
    /// The backend would not play it: it is not audio it can decode.
    Refused(BackendError),
}

impl std::fmt::Display for PlayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlayError::Read(err) => write!(f, "could not read it ({err})"),
            PlayError::Refused(err) => write!(f, "could not play it ({err})"),
        }
    }
}

impl std::error::Error for PlayError {}
