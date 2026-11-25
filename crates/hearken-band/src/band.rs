//! The band: plays a [`Song`] note by note on the music bus, following
//! play the way a dynamic score's stems do.
//!
//! Notes are handed to the pool [`LOOKAHEAD_S`] early, each with its exact
//! time on the audio clock ([`Audio::clock_s`]), so they land on the beat
//! whatever the frame rate. A run's take keeps them at those times, so a
//! saved song and its export sound as the run did.
//!
//! # States
//!
//! The band is in one of [`BandState`]'s states, changed only in
//! [`Band::set_state`] and logged under [`TARGET`]: silent, counting in
//! (started, first bar not yet sounding), playing, or stopped. A pause
//! needs nothing: the band notices the audio clock ran on without it and
//! moves every note still to come by the time it was away.

use hearken_audio::{Audio, Bus, Sound, Start};

use crate::compose::{Event, Section, Song, BEATS};
use crate::style::{Play, Role};
use crate::Bank;

/// The log target the band's state changes go to.
pub const TARGET: &str = "hearken::band";

/// How far ahead notes are handed over, in seconds.
pub const LOOKAHEAD_S: f64 = 0.2;

/// How long a layer takes to swell from silence to full, or fade, in
/// seconds: as a score's stems do.
pub const RAMP_S: f32 = 1.5;

/// How long the first bar waits after a start, in seconds.
const COUNT_IN_S: f64 = 0.12;

/// Past this much more audio clock than frame time between two calls, the
/// band was away (a pause) and its notes still to come move on.
const AWAY_S: f64 = 0.25;

/// The most notes sounding at once; past it, new short notes are skipped.
const MAX_LIVE: usize = 64;

/// What play is doing, as the band hears it: as a score's mood.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Feel {
    /// 0 calm to 1 at its most intense.
    pub intensity: f32,
    pub hurry: bool,
    pub danger: bool,
}

impl Feel {
    pub fn new(intensity: f32) -> Feel {
        Feel {
            intensity: intensity.clamp(0.0, 1.0),
            hurry: false,
            danger: false,
        }
    }
}

/// When each layer comes in, as intensity: silent at or below the first
/// number, full at the second. As a score's stems were.
pub fn layer_level(role: Role, feel: Feel) -> f32 {
    let between = |from: f32, full: f32| {
        if feel.intensity <= from {
            0.0
        } else if feel.intensity >= full {
            1.0
        } else {
            (feel.intensity - from) / (full - from)
        }
    };
    match role {
        Role::Base => 1.0,
        Role::Pulse => between(0.1, 0.35),
        Role::Melody => between(0.35, 0.6),
        Role::Lift => between(0.65, 0.95),
        Role::Hurry => f32::from(u8::from(feel.hurry)),
        Role::Danger => f32::from(u8::from(feel.danger)),
    }
}

/// Where the band is. Changed only through [`Band::set_state`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BandState {
    Silent,
    /// Started; the first bar is on its way.
    CountingIn,
    Playing,
    /// Stopped: notes fading, nothing more to come.
    Stopped,
}

impl BandState {
    pub fn name(self) -> &'static str {
        match self {
            BandState::Silent => "silent",
            BandState::CountingIn => "counting in",
            BandState::Playing => "playing",
            BandState::Stopped => "stopped",
        }
    }
}

/// A note handed over or about to be.
#[derive(Clone, Debug)]
struct Queued {
    at: f64,
    part: usize,
    key: String,
    pitch: f32,
    vel: f32,
    /// When to let it go, on the audio clock.
    end: f64,
    held: bool,
    release_ms: u32,
    role: Role,
}

#[derive(Clone, Debug)]
struct Live {
    sound: Sound,
    part: usize,
    starts: f64,
    end: f64,
    held: bool,
    release_ms: u32,
    vel: f32,
    role: Role,
    /// The level it was last set to.
    level: f32,
    /// How it was started, so a note moved after a pause can start again.
    key: String,
    pitch: f32,
}

/// Plays a song.
pub struct Band {
    song: Song,
    bank: Bank,
    state: BandState,
    section: Section,
    bar: usize,
    /// Bars played since the start, all sections.
    bars_played: u64,
    bar_start: f64,
    bar_len: f64,
    queue: Vec<Queued>,
    live: Vec<Live>,
    /// Each layer's level now, eased toward what play asks.
    levels: [f32; 6],
    feel: Feel,
    last_clock: Option<f64>,
    /// A game asked to move on to the next section at the next bar line.
    move_on: bool,
    /// The game's own speed on top of the song's tempo (a course that
    /// quickens sector by sector).
    speed: f32,
}

fn role_index(role: Role) -> usize {
    Role::ALL.iter().position(|r| *r == role).unwrap_or(0)
}

impl Band {
    /// A band ready to play `song` from `bank`'s samples. Silent until
    /// [`Band::start`].
    pub fn new(song: Song, bank: Bank) -> Band {
        let section = song.section(0);
        Band {
            song,
            bank,
            state: BandState::Silent,
            section,
            bar: 0,
            bars_played: 0,
            bar_start: 0.0,
            bar_len: 2.0,
            queue: Vec::new(),
            live: Vec::new(),
            levels: [0.0; 6],
            feel: Feel::default(),
            last_clock: None,
            move_on: false,
            speed: 1.0,
        }
    }

    pub fn song(&self) -> &Song {
        &self.song
    }

    pub fn state(&self) -> BandState {
        self.state
    }

    /// The section playing now.
    pub fn section(&self) -> &Section {
        &self.section
    }

    /// The bar of the section playing now, from 0.
    pub fn bar(&self) -> usize {
        self.bar
    }

    pub fn bars_played(&self) -> u64 {
        self.bars_played
    }

    /// Each layer's level now, in [`Role::ALL`] order.
    pub fn levels(&self) -> [f32; 6] {
        self.levels
    }

    /// The tempo now, beats a minute: the song's, sped up while hurrying.
    pub fn bpm(&self) -> f32 {
        let speed = if self.feel.hurry {
            1.0 + self.song.style.hurry_speedup.clamp(0.0, 0.5)
        } else {
            1.0
        };
        self.song.bpm * speed * self.speed
    }

    /// The one place the state changes; a change is logged.
    fn set_state(&mut self, next: BandState) {
        if next != self.state {
            log::debug!(target: TARGET, "{}: {} -> {}", self.song.style.name, self.state.name(), next.name());
            self.state = next;
        }
    }

    /// Starts the song from its first bar at `feel`'s levels. Playing
    /// already, it starts over.
    pub fn start(&mut self, audio: &mut Audio, feel: Feel) {
        self.start_at(audio, feel, 0);
    }

    /// Starts the song at the top of section `section` (counting round the
    /// form for ever): a run that comes back after a break between its
    /// sectors carries on where its song had got to.
    pub fn start_at(&mut self, audio: &mut Audio, feel: Feel, section: usize) {
        self.silence(audio, 0);
        log::info!(target: TARGET, "{} song, section {section}: {}", self.song.style.name, self.song.describe());
        self.feel = feel;
        for role in Role::ALL {
            self.levels[role_index(role)] = layer_level(role, feel);
        }
        self.section = self.song.section(section);
        self.bar = 0;
        self.bars_played = 0;
        let now = audio.clock_s();
        self.last_clock = Some(now);
        self.bar_start = now + COUNT_IN_S;
        self.bar_len = self.bar_seconds();
        self.queue_bar();
        self.set_state(BandState::CountingIn);
        self.dispatch(audio, now);
    }

    /// Moves to the next section at the next bar line: a game's sector or a
    /// new day.
    pub fn next_section(&mut self) {
        self.move_on = true;
    }

    /// Sets the game's own speed on top of the song's tempo, from the next
    /// bar: `1.0` is the song's, `1.1` ten percent faster. Kept between a
    /// half and twice.
    pub fn set_speed(&mut self, speed: f32) {
        if speed.is_finite() {
            self.speed = speed.clamp(0.5, 2.0);
        }
    }

    /// Jumps every layer straight to its level for `feel`, for a change
    /// play makes all at once.
    pub fn jump(&mut self, audio: &mut Audio, feel: Feel) {
        self.feel = feel;
        for role in Role::ALL {
            self.levels[role_index(role)] = layer_level(role, feel);
        }
        for i in 0..self.live.len() {
            let level = self.levels[role_index(self.live[i].role)];
            let part_volume = self.part_volume(self.live[i].part);
            let live = &mut self.live[i];
            if live.held {
                live.level = level;
                audio.set_volume(live.sound, (live.vel * part_volume * level).clamp(0.0, 1.0));
            }
        }
    }

    /// Follows play: layers ease toward their levels for `feel`, notes due
    /// within [`LOOKAHEAD_S`] are handed over, and notes due to end are let
    /// go. `dt` is the frame's time.
    pub fn follow(&mut self, audio: &mut Audio, feel: Feel, dt: f64) {
        if !matches!(self.state, BandState::CountingIn | BandState::Playing) {
            return;
        }
        let now = audio.clock_s();
        if let Some(last) = self.last_clock {
            let away = now - last - dt.max(0.0);
            if away > AWAY_S {
                self.move_on_by(audio, last, away);
            }
        }
        self.last_clock = Some(now);
        self.feel = feel;
        let step = (dt as f32 / RAMP_S).max(0.0);
        for role in Role::ALL {
            let i = role_index(role);
            let want = layer_level(role, feel);
            let have = self.levels[i];
            self.levels[i] = if want > have {
                (have + step).min(want)
            } else {
                (have - step).max(want)
            };
        }
        // Held notes follow their layer.
        for i in 0..self.live.len() {
            let level = self.levels[role_index(self.live[i].role)];
            let part_volume = self.part_volume(self.live[i].part);
            let live = &mut self.live[i];
            if live.held && (level - live.level).abs() > 0.02 {
                live.level = level;
                let volume = (live.vel * part_volume * level).clamp(0.0, 1.0);
                audio.set_volume_over(live.sound, volume, 120);
            }
        }
        if self.state == BandState::CountingIn && now >= self.bar_start {
            self.set_state(BandState::Playing);
        }
        self.dispatch(audio, now);
        self.release_due(audio, now);
    }

    /// Stops every note over `fade_ms`; nothing more is played.
    pub fn stop(&mut self, audio: &mut Audio, fade_ms: u32) {
        if self.state == BandState::Silent {
            return;
        }
        self.silence(audio, fade_ms);
        self.set_state(BandState::Stopped);
    }

    fn silence(&mut self, audio: &mut Audio, fade_ms: u32) {
        for live in self.live.drain(..) {
            audio.stop(live.sound, fade_ms);
        }
        self.queue.clear();
        self.move_on = false;
    }

    fn part_volume(&self, part: usize) -> f32 {
        self.song
            .parts
            .get(part)
            .map_or(0.0, |c| self.song.part(c).volume * self.song.style.volume)
    }

    fn bar_seconds(&self) -> f64 {
        f64::from(BEATS) * 60.0 / f64::from(self.bpm().max(20.0))
    }

    /// Where `beat` of a bar falls, in beats, with the song's swing.
    fn swung(&self, beat: f32) -> f32 {
        let whole = beat.floor();
        let f = beat - whole;
        let s = self.song.swing;
        let lean = if (f - 0.5).abs() < 1e-3 {
            s / 6.0
        } else if (f - 0.25).abs() < 1e-3 || (f - 0.75).abs() < 1e-3 {
            s / 12.0
        } else {
            0.0
        };
        whole + f + lean
    }

    /// Writes the current bar's notes into the queue.
    fn queue_bar(&mut self) {
        let spb = self.bar_len / f64::from(BEATS);
        let events = self.song.bar(&self.section, self.bar);
        for (part, event) in events {
            let Some(chosen) = self.song.parts.get(part) else {
                continue;
            };
            let style_part = self.song.part(chosen);
            let held = matches!(style_part.play, Play::Pad | Play::Drone);
            match event {
                Event::Note(n) => {
                    let Some(inst) = self.bank.instrument(&chosen.instrument) else {
                        continue;
                    };
                    let Some((sample, pitch)) = inst.sample_for(n.midi) else {
                        continue;
                    };
                    let at = self.bar_start + f64::from(self.swung(n.beat)) * spb;
                    let len = (f64::from(n.len) * spb).min(f64::from(inst.length_s));
                    self.queue.push(Queued {
                        at,
                        part,
                        key: self.bank.note_key(&inst.name, sample),
                        pitch,
                        vel: n.vel,
                        end: at + len,
                        held,
                        release_ms: inst.release_ms,
                        role: style_part.role,
                    });
                }
                Event::Hit {
                    beat,
                    what,
                    vel,
                    role,
                } => {
                    let Some(kit) = self.bank.kit(&chosen.instrument) else {
                        continue;
                    };
                    let Some(hit) = kit.hit(what) else {
                        continue;
                    };
                    let at = self.bar_start + f64::from(self.swung(beat)) * spb;
                    self.queue.push(Queued {
                        at,
                        part,
                        key: self.bank.hit_key(hit),
                        pitch: 1.0,
                        vel,
                        // A hit rings out by itself.
                        end: f64::INFINITY,
                        held: false,
                        release_ms: 0,
                        role,
                    });
                }
            }
        }
        self.queue.sort_by(|a, b| a.at.total_cmp(&b.at));
    }

    /// Moves on a bar, and a section when this one is done or a game asked.
    fn next_bar(&mut self) {
        self.bar_start += self.bar_len;
        self.bar += 1;
        self.bars_played += 1;
        if self.bar >= self.section.bars() || (self.move_on && self.bar > 0) {
            self.move_on = false;
            let next = self.section.index + 1;
            self.section = self.song.section(next);
            self.bar = 0;
            log::debug!(target: TARGET, "{}: section {} ({})", self.song.style.name, next, self.section.kind.name());
        }
        self.bar_len = self.bar_seconds();
        self.queue_bar();
    }

    /// Hands over every note due within the lookahead, a bar at a time.
    fn dispatch(&mut self, audio: &mut Audio, now: f64) {
        let horizon = now + LOOKAHEAD_S;
        loop {
            while let Some(q) = self.queue.first() {
                if q.at > horizon {
                    return;
                }
                let q = self.queue.remove(0);
                self.play(audio, q);
            }
            if self.bar_start + self.bar_len > horizon {
                return;
            }
            self.next_bar();
        }
    }

    fn play(&mut self, audio: &mut Audio, q: Queued) {
        let level = self.levels[role_index(q.role)];
        if level < 0.01 && !q.held {
            return;
        }
        if self.live.len() >= MAX_LIVE && !q.held {
            return;
        }
        let part = self.song.parts.get(q.part);
        let pan = part.map_or(0.0, |c| self.song.part(c).pan);
        let volume = self.part_volume(q.part);
        let start = Start::ONCE
            .volume((q.vel * volume * level).clamp(0.0, 1.0))
            .pan(pan.clamp(-1.0, 1.0))
            .pitch(q.pitch)
            .at(q.at);
        let sound = audio.start_with(&q.key, Bus::MUSIC, start);
        if sound == Sound::NONE {
            return;
        }
        self.live.push(Live {
            sound,
            part: q.part,
            starts: q.at,
            end: q.end,
            held: q.held,
            release_ms: q.release_ms,
            vel: q.vel,
            role: q.role,
            level,
            key: q.key,
            pitch: q.pitch,
        });
    }

    /// Lets go every note whose time is up, and forgets ones that ended.
    fn release_due(&mut self, audio: &mut Audio, now: f64) {
        let mut i = 0;
        while i < self.live.len() {
            let l = &self.live[i];
            let done = if l.end.is_finite() {
                if now >= l.end - 0.01 {
                    audio.stop(l.sound, l.release_ms);
                    true
                } else {
                    false
                }
            } else {
                now > l.starts + 0.05 && !audio.is_playing(l.sound)
            };
            if done {
                self.live.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }

    /// The band was away for `away` seconds after `last` (a pause): every
    /// note still to come moves on by that much, and notes handed over but
    /// not yet sounding start again at their new times.
    fn move_on_by(&mut self, audio: &mut Audio, last: f64, away: f64) {
        log::debug!(target: TARGET, "{}: away {away:.2} s", self.song.style.name);
        self.bar_start += away;
        for q in &mut self.queue {
            q.at += away;
            q.end += away;
        }
        let mut again = Vec::new();
        for l in &mut self.live {
            l.end += away;
            if l.starts > last {
                audio.stop(l.sound, 0);
                again.push(Queued {
                    at: l.starts + away,
                    part: l.part,
                    key: l.key.clone(),
                    pitch: l.pitch,
                    vel: l.vel,
                    end: l.end,
                    held: l.held,
                    release_ms: l.release_ms,
                    role: l.role,
                });
            }
        }
        self.live.retain(|l| l.starts <= last);
        self.queue.extend(again);
        self.queue.sort_by(|a, b| a.at.total_cmp(&b.at));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compose::tests_support::{bank, style};

    fn audio() -> Audio {
        Audio::silent()
    }

    #[test]
    fn layers_come_in_as_play_heats_up() {
        assert_eq!(layer_level(Role::Base, Feel::new(0.0)), 1.0);
        assert_eq!(layer_level(Role::Pulse, Feel::new(0.05)), 0.0);
        assert_eq!(layer_level(Role::Melody, Feel::new(0.6)), 1.0);
        assert_eq!(layer_level(Role::Lift, Feel::new(0.5)), 0.0);
        let f = Feel {
            hurry: true,
            ..Feel::new(0.0)
        };
        assert_eq!(layer_level(Role::Hurry, f), 1.0);
    }

    #[test]
    fn a_band_counts_in_plays_bars_on_time_and_stops() {
        let song = Song::compose(&style(), &bank(), 3);
        let bar_s = 240.0 / f64::from(song.bpm);
        let mut a = audio();
        let mut band = Band::new(song, bank());
        band.start(&mut a, Feel::new(1.0));
        assert_eq!(band.state(), BandState::CountingIn);
        let mut t = 0.0;
        while t < bar_s * 6.0 {
            a.advance_take(1.0 / 60.0);
            band.follow(&mut a, Feel::new(1.0), 1.0 / 60.0);
            t += 1.0 / 60.0;
        }
        assert_eq!(band.state(), BandState::Playing);
        // Six bars of time, with the bars handed over just ahead.
        assert!(
            (5..=7).contains(&band.bars_played()),
            "{}",
            band.bars_played()
        );
        band.stop(&mut a, 300);
        assert_eq!(band.state(), BandState::Stopped);
    }

    #[test]
    fn hurry_speeds_the_tempo_and_a_pause_moves_the_notes_on() {
        let song = Song::compose(&style(), &bank(), 5);
        let bpm = song.bpm;
        let mut a = audio();
        let mut band = Band::new(song, bank());
        band.start(&mut a, Feel::new(0.5));
        let hurry = Feel {
            hurry: true,
            ..Feel::new(0.5)
        };
        a.advance_take(0.1);
        band.follow(&mut a, hurry, 0.1);
        assert!((band.bpm() - bpm * 1.06).abs() < 0.01);
        let before = band.bar_start;
        // Ten seconds go by with the band not asked (a pause menu).
        a.advance_take(10.0);
        band.follow(&mut a, hurry, 1.0 / 60.0);
        assert!(
            band.bar_start >= before + 9.9,
            "{} then {}",
            before,
            band.bar_start
        );
    }
}
