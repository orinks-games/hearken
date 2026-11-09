//! A take: what a stretch of play asked the pool to do on chosen buses, with
//! the time of each call, so the same music can be performed again.
//!
//! A game whose music is built as it plays (parts starting a bar at a time,
//! voices fading with what the player does) has nothing to keep once the
//! run is over: the sound is gone, and a seed replays the composition but
//! not what happened to it. A take keeps the calls instead. While one is
//! recording ([`Audio::begin_take`](crate::Audio::begin_take)), every sound
//! started on one of its buses is followed: its start, each change to its
//! volume, pitch, pan or position, its pause and its stop, plus every move
//! of the listener, each stamped with the take's own clock. The take is a
//! few numbers an event, so an hour of play is under a megabyte, and
//! nothing is written anywhere until the game asks for it.
//!
//! Performing a take is the pool's caller's business (Hearken's soundtrack
//! performs one live from the menu, or on an offline mixer into a file):
//! every event names what to call, on which recorded sound, and when.
//!
//! # Holding
//!
//! A take is held ([`Audio::hold_take`](crate::Audio::hold_take)) while
//! something covers the play it belongs to: a pause menu, a settings
//! screen. Its clock stops, a sound started while held is not followed, and
//! a call on a followed sound (a game stopping its music as a menu opens)
//! lands at the frozen time. The held span is not in the take at all, so a
//! pause leaves no gap and a menu's own music leaves no trace.
//!
//! # Coalescing
//!
//! A game that eases a volume a little every frame would fill a take with
//! sixty entries a second per sound. A change to the same part of the same
//! sound inside [`COALESCE_S`] replaces the one waiting, and a change that
//! lands within [`SAME`] of the value last recorded is dropped, so an ease
//! becomes a few short ramps and a value set to what it already is costs
//! nothing.
//!
//! # States
//!
//! The recorder is in one of [`TakeState`]'s states, changed in one place
//! ([`Recorder::set_state`]) and logged under [`TARGET`]: idle, recording,
//! held, or ended. Ended is a moment, not a resting state: the take is
//! handed back and the recorder is idle again.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::pool::Sound;
use crate::types::{Bus, LoopRegion, Vec3};

/// The log target the recorder's state changes go to.
pub const TARGET: &str = "hearken::take";

/// How long a change to one part of a sound waits for a later one to
/// replace it before it is recorded, in seconds.
pub const COALESCE_S: f64 = 0.05;

/// How near a value must be to the one last recorded for the same part of
/// the same sound to be dropped as no change.
pub const SAME: f32 = 0.01;

/// The longest a take records, in seconds: an hour. Past it the take is
/// cut ([`Take::cut`]) and records nothing more, though it stays recording
/// until ended so the game needs no special case.
pub const MAX_S: f64 = 60.0 * 60.0;

/// The most events a take holds before it is cut.
pub const MAX_EVENTS: usize = 250_000;

/// Where the recorder is. Changed only through [`Recorder::set_state`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TakeState {
    /// Nothing is recording.
    Idle,
    /// Following sounds on the take's buses, clock running.
    Recording,
    /// Covered: clock stopped, new sounds not followed.
    Held,
    /// The take was just handed back; the recorder goes idle at once.
    Ended,
}

impl TakeState {
    /// A stable name for logs and tests: `"idle"`, `"recording"`,
    /// `"held"`, `"ended"`.
    pub fn name(self) -> &'static str {
        match self {
            TakeState::Idle => "idle",
            TakeState::Recording => "recording",
            TakeState::Held => "held",
            TakeState::Ended => "ended",
        }
    }
}

/// One call in a take. `t` is seconds into the take; `id` names the
/// recorded sound, counted from 0 in the order the sounds started.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "e", rename_all = "snake_case")]
pub enum TakeEvent {
    /// A clip started, with everything its start carried.
    Start {
        t: f64,
        id: u32,
        key: String,
        volume: f32,
        pan: f32,
        pitch: f32,
        looping: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        region: Option<LoopRegion>,
        #[serde(default, skip_serializing_if = "is_zero")]
        from_s: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pos: Option<Vec3>,
    },
    Stop {
        t: f64,
        id: u32,
        fade_ms: u32,
    },
    Release {
        t: f64,
        id: u32,
        fade_ms: u32,
    },
    Pause {
        t: f64,
        id: u32,
        paused: bool,
    },
    Volume {
        t: f64,
        id: u32,
        value: f32,
        ramp_ms: u32,
    },
    Pitch {
        t: f64,
        id: u32,
        value: f32,
        ramp_ms: u32,
    },
    Pan {
        t: f64,
        id: u32,
        value: f32,
        ramp_ms: u32,
    },
    Position {
        t: f64,
        id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pos: Option<Vec3>,
    },
    Listener {
        t: f64,
        pos: Vec3,
        facing: f32,
    },
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

impl TakeEvent {
    /// Seconds into the take.
    pub fn t(&self) -> f64 {
        match self {
            TakeEvent::Start { t, .. }
            | TakeEvent::Stop { t, .. }
            | TakeEvent::Release { t, .. }
            | TakeEvent::Pause { t, .. }
            | TakeEvent::Volume { t, .. }
            | TakeEvent::Pitch { t, .. }
            | TakeEvent::Pan { t, .. }
            | TakeEvent::Position { t, .. }
            | TakeEvent::Listener { t, .. } => *t,
        }
    }
}

/// A finished take.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Take {
    /// The format, for a reader that meets an older or newer file.
    #[serde(default = "version_now")]
    pub version: u32,
    /// The buses the take followed, by name.
    pub buses: Vec<String>,
    /// How long it ran, in recorded seconds (held time left out).
    pub length_s: f64,
    /// Whether it hit [`MAX_S`] or [`MAX_EVENTS`] and stopped recording
    /// there.
    #[serde(default)]
    pub cut: bool,
    /// Every call, in time order.
    pub events: Vec<TakeEvent>,
}

/// The format written now.
pub const VERSION: u32 = 1;

fn version_now() -> u32 {
    VERSION
}

impl Take {
    /// Whether nothing was started.
    pub fn is_empty(&self) -> bool {
        !self
            .events
            .iter()
            .any(|e| matches!(e, TakeEvent::Start { .. }))
    }

    /// How many sounds started.
    pub fn starts(&self) -> usize {
        self.events
            .iter()
            .filter(|e| matches!(e, TakeEvent::Start { .. }))
            .count()
    }

    /// The take as JSON, one line.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// A take read back from [`Take::to_json`].
    pub fn from_json(text: &str) -> Result<Take, serde_json::Error> {
        serde_json::from_str(text)
    }
}

/// Which part of a sound a coalesced change is to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Part {
    Volume,
    Pitch,
    Pan,
}

/// A change waiting to be recorded, so a later one inside [`COALESCE_S`]
/// can replace it.
#[derive(Clone, Copy, Debug)]
struct Pending {
    t: f64,
    id: u32,
    part: Part,
    value: f32,
    ramp_ms: u32,
}

/// Records a take for [`crate::Audio`].
pub(crate) struct Recorder {
    buses: HashSet<Bus>,
    state: TakeState,
    clock: f64,
    events: Vec<TakeEvent>,
    /// The recorded id of each followed sound.
    ids: HashMap<Sound, u32>,
    next_id: u32,
    pending: Vec<Pending>,
    /// The value last recorded for each part of each sound.
    last: HashMap<(u32, Part), f32>,
    /// The listener as last recorded, and a move waiting to be.
    listener: Option<(Vec3, f32)>,
    listener_pending: Option<(f64, Vec3, f32)>,
    cut: bool,
}

impl Recorder {
    /// A recorder on `buses`, recording from now, with the listener where
    /// it stands: the first event, so a performance hears the first sounds
    /// from the same place.
    pub(crate) fn new(buses: &[Bus], pos: Vec3, facing: f32) -> Recorder {
        let mut r = Recorder {
            buses: buses.iter().copied().collect(),
            state: TakeState::Idle,
            clock: 0.0,
            events: Vec::new(),
            ids: HashMap::new(),
            next_id: 0,
            pending: Vec::new(),
            last: HashMap::new(),
            listener: None,
            listener_pending: None,
            cut: false,
        };
        r.set_state(TakeState::Recording);
        if pos.is_finite() {
            r.listener = Some((pos, facing));
            r.push(TakeEvent::Listener {
                t: 0.0,
                pos,
                facing,
            });
        }
        r
    }

    pub(crate) fn state(&self) -> TakeState {
        self.state
    }

    /// The one place the state changes: logged, and the same state again
    /// is nothing.
    pub(crate) fn set_state(&mut self, next: TakeState) -> bool {
        if next == self.state {
            return false;
        }
        log::info!(target: TARGET, "{} -> {}", self.state.name(), next.name());
        self.state = next;
        true
    }

    pub(crate) fn seconds(&self) -> f64 {
        self.clock
    }

    pub(crate) fn len(&self) -> usize {
        self.events.len() + self.pending.len()
    }

    pub(crate) fn hold(&mut self, held: bool) {
        if held {
            self.flush_all();
            self.set_state(TakeState::Held);
        } else {
            self.set_state(TakeState::Recording);
        }
    }

    /// Moves the clock on while recording, and records every change that
    /// has waited long enough.
    pub(crate) fn advance(&mut self, dt: f64) {
        if self.state != TakeState::Recording || !dt.is_finite() || dt <= 0.0 {
            return;
        }
        self.clock += dt;
        if self.clock >= MAX_S {
            self.cut_here();
        }
        self.flush_due();
    }

    /// Ends the take and hands it back; the recorder is idle after.
    pub(crate) fn end(&mut self) -> Take {
        self.flush_all();
        self.set_state(TakeState::Ended);
        self.set_state(TakeState::Idle);
        let mut buses: Vec<String> = self.buses.iter().map(|b| b.0.to_string()).collect();
        buses.sort();
        Take {
            version: VERSION,
            buses,
            length_s: self.clock,
            cut: self.cut,
            events: std::mem::take(&mut self.events),
        }
    }

    /// Whether the recorder follows sounds on `bus` right now: recording,
    /// not held, not cut.
    fn follows(&self, bus: Bus) -> bool {
        self.state == TakeState::Recording && !self.cut && self.buses.contains(&bus)
    }

    /// Stops recording from here, keeping what there is.
    fn cut_here(&mut self) {
        if !self.cut {
            self.cut = true;
            self.flush_all();
            log::info!(target: TARGET, "cut at {:.1} s, {} events", self.clock, self.events.len());
        }
    }

    fn push(&mut self, event: TakeEvent) {
        if self.cut {
            return;
        }
        self.events.push(event);
        if self.events.len() >= MAX_EVENTS {
            self.cut_here();
        }
    }

    /// A sound started on `bus`: followed from now on if the take is
    /// recording on that bus.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn started(
        &mut self,
        sound: Sound,
        bus: Bus,
        key: &str,
        volume: f32,
        pan: f32,
        pitch: f32,
        looping: bool,
        region: Option<LoopRegion>,
        from_s: f64,
        pos: Option<Vec3>,
    ) {
        if !self.follows(bus) || sound == Sound::NONE {
            return;
        }
        // Where the listener stands sets the first mix, so a move still
        // settling goes in first.
        if let Some((t, pos, facing)) = self.listener_pending.take() {
            self.listener = Some((pos, facing));
            self.push(TakeEvent::Listener { t, pos, facing });
        }
        let id = self.next_id;
        self.next_id += 1;
        self.ids.insert(sound, id);
        self.last.insert((id, Part::Volume), volume);
        self.last.insert((id, Part::Pitch), pitch);
        self.last.insert((id, Part::Pan), pan);
        let t = self.clock;
        self.push(TakeEvent::Start {
            t,
            id,
            key: key.to_string(),
            volume,
            pan,
            pitch,
            looping,
            region,
            from_s,
            pos,
        });
    }

    /// A followed sound's loop region, set after it started.
    pub(crate) fn region_set(&mut self, sound: Sound, new: LoopRegion) {
        let Some(&id) = self.ids.get(&sound) else {
            return;
        };
        if let Some(TakeEvent::Start { region, .. }) = self
            .events
            .iter_mut()
            .rev()
            .find(|e| matches!(e, TakeEvent::Start { id: i, .. } if *i == id))
        {
            *region = Some(new);
        }
    }

    pub(crate) fn stopped(&mut self, sound: Sound, fade_ms: u32) {
        let Some(id) = self.ids.remove(&sound) else {
            return;
        };
        self.flush_for(id);
        let t = self.clock;
        self.push(TakeEvent::Stop { t, id, fade_ms });
    }

    pub(crate) fn released(&mut self, sound: Sound, fade_ms: u32) {
        let Some(&id) = self.ids.get(&sound) else {
            return;
        };
        self.flush_for(id);
        let t = self.clock;
        self.push(TakeEvent::Release { t, id, fade_ms });
    }

    pub(crate) fn paused(&mut self, sound: Sound, paused: bool) {
        let Some(&id) = self.ids.get(&sound) else {
            return;
        };
        self.flush_for(id);
        let t = self.clock;
        self.push(TakeEvent::Pause { t, id, paused });
    }

    /// A followed sound ended by itself, or its slot was freed: no event,
    /// its clip ends the same way when performed.
    pub(crate) fn freed(&mut self, sound: Sound) {
        if let Some(id) = self.ids.remove(&sound) {
            self.flush_for(id);
        }
    }

    pub(crate) fn volume(&mut self, sound: Sound, value: f32, ramp_ms: u32) {
        self.change(sound, Part::Volume, value, ramp_ms);
    }

    pub(crate) fn pitch(&mut self, sound: Sound, value: f32, ramp_ms: u32) {
        self.change(sound, Part::Pitch, value, ramp_ms);
    }

    pub(crate) fn pan(&mut self, sound: Sound, value: f32, ramp_ms: u32) {
        self.change(sound, Part::Pan, value, ramp_ms);
    }

    pub(crate) fn moved(&mut self, sound: Sound, pos: Option<Vec3>) {
        let Some(&id) = self.ids.get(&sound) else {
            return;
        };
        if !pos.is_none_or(|p| p.is_finite()) {
            return;
        }
        let t = self.clock;
        self.push(TakeEvent::Position { t, id, pos });
    }

    /// The listener moved: recorded once it has sat still for
    /// [`COALESCE_S`], and only when it is somewhere new.
    pub(crate) fn listener_moved(&mut self, pos: Vec3, facing: f32) {
        if self.state != TakeState::Recording || self.cut || !pos.is_finite() {
            return;
        }
        if self.listener == Some((pos, facing)) {
            self.listener_pending = None;
            return;
        }
        let t = self.listener_pending.map_or(self.clock, |(t, ..)| t);
        self.listener_pending = Some((t, pos, facing));
    }

    fn change(&mut self, sound: Sound, part: Part, value: f32, ramp_ms: u32) {
        let Some(&id) = self.ids.get(&sound) else {
            return;
        };
        if !value.is_finite() {
            return;
        }
        if let Some(p) = self
            .pending
            .iter_mut()
            .find(|p| p.id == id && p.part == part)
        {
            p.value = value;
            p.ramp_ms = ramp_ms;
            return;
        }
        if self
            .last
            .get(&(id, part))
            .is_some_and(|last| (last - value).abs() < SAME)
        {
            return;
        }
        self.pending.push(Pending {
            t: self.clock,
            id,
            part,
            value,
            ramp_ms,
        });
    }

    fn record(&mut self, p: Pending) {
        if self
            .last
            .get(&(p.id, p.part))
            .is_some_and(|last| (last - p.value).abs() < SAME)
        {
            return;
        }
        self.last.insert((p.id, p.part), p.value);
        let (t, id, value, ramp_ms) = (p.t, p.id, p.value, p.ramp_ms);
        self.push(match p.part {
            Part::Volume => TakeEvent::Volume {
                t,
                id,
                value,
                ramp_ms,
            },
            Part::Pitch => TakeEvent::Pitch {
                t,
                id,
                value,
                ramp_ms,
            },
            Part::Pan => TakeEvent::Pan {
                t,
                id,
                value,
                ramp_ms,
            },
        });
    }

    fn flush_due(&mut self) {
        let clock = self.clock;
        let due: Vec<Pending> = self
            .pending
            .iter()
            .copied()
            .filter(|p| clock - p.t >= COALESCE_S)
            .collect();
        if !due.is_empty() {
            self.pending.retain(|p| clock - p.t < COALESCE_S);
            for p in due {
                self.record(p);
            }
        }
        if let Some((t, pos, facing)) = self.listener_pending {
            if clock - t >= COALESCE_S {
                self.listener_pending = None;
                self.listener = Some((pos, facing));
                self.push(TakeEvent::Listener { t, pos, facing });
            }
        }
    }

    fn flush_for(&mut self, id: u32) {
        let mine: Vec<Pending> = self
            .pending
            .iter()
            .copied()
            .filter(|p| p.id == id)
            .collect();
        if mine.is_empty() {
            return;
        }
        self.pending.retain(|p| p.id != id);
        for p in mine {
            self.record(p);
        }
    }

    fn flush_all(&mut self) {
        let all = std::mem::take(&mut self.pending);
        for p in all {
            self.record(p);
        }
        if let Some((t, pos, facing)) = self.listener_pending.take() {
            self.listener = Some((pos, facing));
            self.push(TakeEvent::Listener { t, pos, facing });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sound(i: u32) -> Sound {
        Sound {
            index: i,
            generation: 0,
        }
    }

    fn take_events(r: &Recorder) -> &[TakeEvent] {
        &r.events
    }

    fn start(r: &mut Recorder, i: u32, bus: Bus) {
        r.started(
            sound(i),
            bus,
            "song/a",
            0.8,
            0.0,
            1.0,
            true,
            None,
            0.0,
            None,
        );
    }

    #[test]
    fn follows_only_its_buses_and_stamps_the_clock() {
        let mut r = Recorder::new(&[Bus::MUSIC], Vec3::ZERO, 0.0);
        start(&mut r, 1, Bus::MUSIC);
        start(&mut r, 2, Bus::EFFECTS);
        r.advance(1.5);
        r.stopped(sound(1), 300);
        r.stopped(sound(2), 300);
        let take = r.end();
        assert_eq!(take.length_s, 1.5);
        assert_eq!(take.starts(), 1);
        assert!(matches!(
            take.events[0],
            TakeEvent::Listener { t, .. } if t == 0.0
        ));
        assert_eq!(
            take.events[2],
            TakeEvent::Stop {
                t: 1.5,
                id: 0,
                fade_ms: 300
            }
        );
        assert_eq!(take.buses, vec!["music".to_string()]);
    }

    #[test]
    fn an_ease_becomes_a_few_ramps_and_no_change_is_nothing() {
        let mut r = Recorder::new(&[Bus::MUSIC], Vec3::ZERO, 0.0);
        start(&mut r, 1, Bus::MUSIC);
        // Sixty frames of easing from 0.8 down to 0.1 over a second.
        for i in 1..=60 {
            r.volume(sound(1), 0.8 - 0.7 * i as f32 / 60.0, 30);
            r.advance(1.0 / 60.0);
        }
        // Set to what it already is: nothing.
        r.volume(sound(1), 0.1, 30);
        r.advance(0.1);
        let take = r.end();
        let volumes: Vec<&TakeEvent> = take
            .events
            .iter()
            .filter(|e| matches!(e, TakeEvent::Volume { .. }))
            .collect();
        assert!(
            (15..=22).contains(&volumes.len()),
            "{} volume events",
            volumes.len()
        );
        let last = volumes.last().unwrap();
        assert!(matches!(last, TakeEvent::Volume { value, .. } if (*value - 0.1).abs() < 0.02));
    }

    #[test]
    fn held_time_is_left_out_and_new_sounds_are_not_followed() {
        let mut r = Recorder::new(&[Bus::MUSIC], Vec3::ZERO, 0.0);
        start(&mut r, 1, Bus::MUSIC);
        r.advance(2.0);
        r.hold(true);
        assert_eq!(r.state(), TakeState::Held);
        r.advance(10.0);
        start(&mut r, 2, Bus::MUSIC);
        r.stopped(sound(1), 500);
        r.hold(false);
        r.advance(1.0);
        r.stopped(sound(2), 500);
        let take = r.end();
        assert_eq!(take.length_s, 3.0);
        assert_eq!(take.starts(), 1);
        assert!(take.events.iter().any(|e| matches!(
            e,
            TakeEvent::Stop {
                t, id: 0, fade_ms: 500
            } if *t == 2.0
        )));
    }

    #[test]
    fn a_take_round_trips_through_json() {
        let mut r = Recorder::new(&[Bus::MUSIC], Vec3::ZERO, 0.0);
        r.started(
            sound(1),
            Bus::MUSIC,
            "song/a",
            0.5,
            -0.3,
            1.1,
            false,
            Some(LoopRegion::new(1.0, 2.0)),
            0.25,
            Some(Vec3::new(1.0, 2.0, 0.0)),
        );
        r.listener_moved(Vec3::new(0.0, 1.0, 0.0), 90.0);
        r.advance(0.1);
        r.listener_moved(Vec3::new(0.0, 3.0, 0.0), 90.0);
        start(&mut r, 2, Bus::MUSIC);
        assert!(matches!(
            (
                &take_events(&r)[take_events(&r).len() - 2],
                &take_events(&r)[take_events(&r).len() - 1]
            ),
            (TakeEvent::Listener { .. }, TakeEvent::Start { id: 1, .. })
        ));
        r.pitch(sound(1), 0.94, 0);
        r.advance(0.1);
        r.moved(sound(1), None);
        r.paused(sound(1), true);
        r.released(sound(1), 200);
        let take = r.end();
        let back = Take::from_json(&take.to_json()).unwrap();
        assert_eq!(back, take);
        assert_eq!(back.version, VERSION);
    }

    #[test]
    fn a_long_take_is_cut_at_the_limit() {
        let mut r = Recorder::new(&[Bus::MUSIC], Vec3::ZERO, 0.0);
        start(&mut r, 1, Bus::MUSIC);
        r.advance(MAX_S + 5.0);
        start(&mut r, 2, Bus::MUSIC);
        let take = r.end();
        assert!(take.cut);
        assert_eq!(take.starts(), 1);
    }

    #[test]
    fn the_state_changes_in_one_place() {
        let mut r = Recorder::new(&[Bus::MUSIC], Vec3::ZERO, 0.0);
        assert_eq!(r.state(), TakeState::Recording);
        assert!(!r.set_state(TakeState::Recording));
        assert!(r.set_state(TakeState::Held));
        r.end();
        assert_eq!(r.state(), TakeState::Idle);
        assert_eq!(TakeState::Ended.name(), "ended");
    }
}
