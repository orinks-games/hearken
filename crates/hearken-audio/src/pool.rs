//! The pool of positioned sounds: generation-checked handles over a [`Backend`].

use crate::backend::{Backend, Channel, Source};
use crate::clips::{ClipSource, LooseClips};
use crate::silent::SilentBackend;
use crate::spatial::{place, SpatialConfig};
use crate::types::{Bus, Listener, Mix, Vec3};
use hearken_synth::{builtin_wav, Params, Recipe, Voice};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::mpsc::Receiver;

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
    pub(crate) pitch: f32,
    pub(crate) looping: bool,
    pub(crate) voice: Option<(Recipe, Params)>,
    pub(crate) paused: bool,
    pub(crate) paused_by_all: bool,
    pub(crate) occluded: bool,
    pub(crate) sent: Mix,
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
    pub(crate) duck_level: f32,
    pub(crate) duck_buses: HashSet<Bus>,
}

impl Default for Mixing {
    fn default() -> Mixing {
        Mixing {
            bus_volumes: HashMap::new(),
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
    let (mut pan, mut lowpass_hz, mut relative) = (0.0, None, None);
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
    pub(crate) bed: Option<(String, Sound)>,
    pub(crate) music: Option<(String, bool, Sound)>,
    pub(crate) opening: Option<Receiver<Result<Box<dyn Backend>, String>>>,
    pub(crate) notice: Option<String>,
    pub(crate) shut_down: bool,
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
            bed: None,
            music: None,
            opening: None,
            notice: None,
            shut_down: false,
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

    /// `key`'s clip bytes, falling back to its built-in default (and noting
    /// it missing) the same way `start` does -- so a sound replayed on a
    /// newly opened backend gets the same clip it would if started fresh.
    pub(crate) fn clip_or_default(&mut self, key: &str) -> std::sync::Arc<[u8]> {
        // Already known bad (missing, or rejected by the backend last time):
        // go straight to the default without asking the clip source again.
        if self.missing.contains(key) {
            return builtin_wav(key);
        }
        match self.clips.load(key) {
            Some(bytes) => bytes,
            None => {
                self.note_missing(key);
                builtin_wav(key)
            }
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
            None => Source::Clip(self.clip_or_default(key)),
        };

        let mut live = Live {
            key: key.to_string(),
            channel: 0,
            bus,
            pos,
            volume,
            pitch: 1.0,
            looping,
            voice,
            paused: false,
            paused_by_all: false,
            occluded: false,
            sent: Mix::UNITY,
        };
        let mix = self.mix_of(&live);

        match self.backend.play(key, source, looping, mix) {
            Ok(channel) => {
                live.channel = channel;
                live.sent = mix;
                self.alloc(live)
            }
            Err(err) => {
                if live.voice.is_some() {
                    // A live voice has no clip to fall back to: falling
                    // back to the key's default clip would start an
                    // unrequested, unstoppable loop (nobody holds a handle
                    // to stop it) named after a key that was never missing.
                    log::error!("backend failed to play voice {key:?}: {err}");
                    return Sound::NONE;
                }
                if self.note_missing(key) {
                    log::error!("backend failed to play {key:?}: {err}");
                }
                match self
                    .backend
                    .play(key, Source::Clip(builtin_wav(key)), looping, mix)
                {
                    Ok(channel) => {
                        live.channel = channel;
                        live.sent = mix;
                        self.alloc(live)
                    }
                    Err(err) => {
                        log::error!("backend failed to play {key:?}'s default too: {err}");
                        Sound::NONE
                    }
                }
            }
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

    /// Pauses `sound`.
    pub fn pause(&mut self, sound: Sound) {
        let Some(live) = slot_mut(&mut self.slots, sound) else {
            return;
        };
        live.paused = true;
        let channel = live.channel;
        self.backend.pause(channel, true);
    }

    /// Resumes a paused `sound`.
    pub fn resume(&mut self, sound: Sound) {
        let Some(live) = slot_mut(&mut self.slots, sound) else {
            return;
        };
        live.paused = false;
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
        let affected: Vec<Sound> = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(i, slot)| {
                let live = slot.live.as_ref()?;
                (bus == Bus::MASTER || live.bus == bus).then_some(Sound {
                    index: i as u32,
                    generation: slot.generation,
                })
            })
            .collect();
        for sound in affected {
            self.resend(sound, self.fades.volume_ms);
        }
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

    /// Every key that has fallen back to its default at least once, sorted.
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

    /// Records `key` as missing; returns whether this is the first time.
    fn note_missing(&mut self, key: &str) -> bool {
        let newly_missing = self.missing.insert(key.to_string());
        if newly_missing {
            log::warn!("sound {key} is missing; playing its default");
        }
        newly_missing
    }

    fn alloc(&mut self, live: Live) -> Sound {
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
