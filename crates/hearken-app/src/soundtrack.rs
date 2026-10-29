//! A game's soundtrack: music the player unlocks by playing, then listens to
//! on its own from a menu.
//!
//! A game lists its pieces as [`Track`]s, from [`Game::soundtrack`] or
//! later with [`Soundtrack::set_tracks`]. Each one is locked until the
//! player earns it, by one of two routes:
//!
//! - **Hearing it.** A track names the sound keys that unlock it
//!   ([`Track::unlocked_by`]); the first time any of them starts during
//!   play, the engine unlocks the track and saves. A [`Track::clip`] is
//!   unlocked by its own key unless told otherwise.
//! - **The game says so.** [`Ctx::unlock_track`], for a rule the engine
//!   cannot hear: a night finished, a boss beaten.
//!
//! [`SoundtrackScreen`] lists the tracks. An unlocked one plays on Enter;
//! a locked one reads its hint ([`Track::hint`]); a secret one
//! ([`Track::secret`]) is not listed or counted until it is unlocked, so a
//! surprise stays a surprise.
//!
//! # Every play is a new performance
//!
//! The soundtrack does not play one fixed file. Each play draws a fresh
//! [`Variety`] and performs the piece from it:
//!
//! - [`Piece::Stems`] arranges a piece's looping layers anew every time:
//!   which voices open it, the order they join in, where some drop out and
//!   come back, how it thins out at the end, how long it runs, and each
//!   voice's level and place in the stereo field.
//! - [`Variation`] moves the whole piece up or down a semitone or two (key
//!   and tempo together, as a turntable would) and shades the mix.
//! - [`Piece::Live`] hands the performance to the game's own
//!   [`Performer`], which can compose as it goes.
//!
//! So a recording of one play is that play, not the piece: the stems stay
//! in the game's sound pack and are only ever heard mixed, and the next
//! play differs. What this cannot stop is someone recording what comes out
//! of their speakers, or digging the pack key out of the program (see
//! `hearken-pack`); nothing that plays sound on a player's computer can.
//!
//! # Saving
//!
//! The unlocked ids go to `soundtrack.json` in the game's data folder
//! through [`crate::Vault`], so a release made with `seal = true` seals
//! them like any other save: an edited or swapped file is set aside and the
//! soundtrack starts over. A game that keeps its progress elsewhere
//! overrides [`Game::load_soundtrack`] and [`Game::save_soundtrack`].
//!
//! A developer build (any build `hearken release` did not make) opens every
//! track while `<PREFIX>_ALL_TRACKS=1` is set, to audition them, and saves
//! nothing then.

use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use hearken_audio::{Audio, Bus, Sound, Start};
use serde::{Deserialize, Serialize};

use crate::ctx::Ctx;
use crate::game::Game;
use crate::menu::{Menu, MenuCore, MenuItem};
use crate::seal::{Opened, Vault};
use crate::sound::UiSound;

/// The file [`Game::load_soundtrack`] and [`Game::save_soundtrack`] use by
/// default, in the game's data folder.
pub const FILE: &str = "soundtrack.json";

/// A random source for one performance: a fresh seed every play
/// ([`Variety::fresh`]), or a fixed one for a test ([`Variety::new`]).
#[derive(Clone, Debug)]
pub struct Variety(u64);

impl Variety {
    pub fn new(seed: u64) -> Variety {
        Variety(seed)
    }

    /// A seed no earlier play used: the time, mixed with a count of plays.
    pub fn fresh() -> Variety {
        static PLAYS: AtomicU64 = AtomicU64::new(0);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let n = PLAYS.fetch_add(1, Ordering::Relaxed);
        let mut v = Variety(now ^ n.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        v.next_u64();
        v
    }

    /// splitmix64.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// `0..n`; `0` when `n` is `0`.
    pub fn below(&mut self, n: u32) -> u32 {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as u32
        }
    }

    /// `lo..=hi`, whichever way round they are given.
    pub fn range(&mut self, lo: i32, hi: i32) -> i32 {
        let (lo, hi) = (lo.min(hi), lo.max(hi));
        lo + self.below((hi - lo) as u32 + 1) as i32
    }

    /// `0.0..1.0`.
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// `-spread..spread`.
    pub fn spread(&mut self, spread: f32) -> f32 {
        (self.unit() * 2.0 - 1.0) * spread
    }

    pub fn chance(&mut self, p: f32) -> bool {
        self.unit() < p
    }

    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below(i as u32 + 1) as usize;
            items.swap(i, j);
        }
    }
}

/// How far each play of a recorded piece may stray from the recording.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Variation {
    /// The whole piece moves up or down by up to this many semitones, key
    /// and tempo together. `0` keeps the recorded key.
    pub semitones: u8,
    /// Each voice's level moves by up to this many decibels.
    pub level_db: f32,
    /// Each voice moves up to this far left or right (`0..=1`). A single
    /// recording ([`Piece::Clip`]) stays centred.
    pub pan: f32,
}

impl Variation {
    /// Every play as recorded.
    pub const NONE: Variation = Variation {
        semitones: 0,
        level_db: 0.0,
        pan: 0.0,
    };

    /// Up or down a semitone, levels within 2 dB, voices spread a quarter
    /// of the way out: different each time, and still the same piece.
    pub const GENTLE: Variation = Variation {
        semitones: 1,
        level_db: 2.0,
        pan: 0.25,
    };

    /// The playback rate for this play.
    pub fn pitch(&self, v: &mut Variety) -> f32 {
        let s = self.semitones as i32;
        2f32.powf(v.range(-s, s) as f32 / 12.0)
    }

    fn level(&self, v: &mut Variety) -> f32 {
        10f32.powf(v.spread(self.level_db.max(0.0)) / 20.0)
    }
}

impl Default for Variation {
    fn default() -> Self {
        Variation::GENTLE
    }
}

/// Plays one performance of a piece. The engine calls `start` once, then
/// `update` every frame until it returns false or the player stops it
/// (`stop`). Every sound it starts should go on [`Bus::MUSIC`].
pub trait Performer {
    fn start(&mut self, audio: &mut Audio);
    /// `dt` seconds on; false once the piece has ended.
    fn update(&mut self, audio: &mut Audio, dt: f64) -> bool;
    /// Stops at once, with a short fade.
    fn stop(&mut self, audio: &mut Audio);
}

/// Makes a game's own [`Performer`] for one play, from that play's
/// [`Variety`].
pub type PerformerFactory = Rc<dyn Fn(Variety) -> Box<dyn Performer>>;

/// Layers recorded to loop together, all the same length, arranged anew
/// every play (see the module docs).
#[derive(Clone, Debug, PartialEq)]
pub struct Stems {
    /// Keys that play the whole way through: a pad, a pulse. May be empty.
    pub bed: Vec<String>,
    /// Keys that come and go, in the order they best enter: the arrangement
    /// keeps roughly to it, with neighbours swapping now and then.
    pub voices: Vec<String>,
    /// Seconds in one loop of the stems, at the recorded speed.
    pub loop_s: f64,
    /// About how long a play runs, in seconds. Rounded to whole loops, at
    /// least enough for every voice to join and leave.
    pub length_s: f64,
    /// Each voice's level before [`Variation::level_db`].
    pub level: f32,
}

impl Stems {
    pub fn new(loop_s: f64) -> Stems {
        Stems {
            bed: Vec::new(),
            voices: Vec::new(),
            loop_s,
            length_s: 150.0,
            level: 0.8,
        }
    }

    pub fn bed(mut self, keys: impl IntoIterator<Item = impl Into<String>>) -> Stems {
        self.bed = keys.into_iter().map(Into::into).collect();
        self
    }

    pub fn voices(mut self, keys: impl IntoIterator<Item = impl Into<String>>) -> Stems {
        self.voices = keys.into_iter().map(Into::into).collect();
        self
    }

    pub fn length_s(mut self, seconds: f64) -> Stems {
        self.length_s = seconds;
        self
    }

    pub fn level(mut self, level: f32) -> Stems {
        self.level = level;
        self
    }

    /// Which voices play in each loop of one performance: `plan[pass][voice]`.
    /// Every voice is heard, the last pass is never silent, and the plan
    /// builds up, holds, breaks down and thins out.
    pub fn plan(&self, v: &mut Variety) -> Vec<Vec<bool>> {
        let n = self.voices.len();
        if n == 0 {
            let passes = self.passes(0);
            return vec![Vec::new(); passes];
        }
        // The entry order: the given order, neighbours swapped now and then.
        let mut order: Vec<usize> = (0..n).collect();
        for i in 1..n {
            if v.chance(0.35) {
                order.swap(i - 1, i);
            }
        }
        let passes = self.passes(n);
        let mut plan = Vec::with_capacity(passes);
        let mut on = vec![false; n];
        // Open with one or two voices.
        let opening = if n > 1 && v.chance(0.4) { 2 } else { 1 };
        let mut joined = 0;
        for &i in order.iter().take(opening) {
            on[i] = true;
            joined += 1;
        }
        // Leave room at the end to thin out to one voice.
        let thin_at = passes.saturating_sub(n.clamp(1, 4));
        for pass in 0..passes {
            if pass > 0 && pass < thin_at {
                if joined < n {
                    // Build: a voice joins most loops, sometimes two.
                    let add = if v.chance(0.2) {
                        2
                    } else if v.chance(0.8) {
                        1
                    } else {
                        0
                    };
                    for _ in 0..add {
                        if joined < n {
                            on[order[joined]] = true;
                            joined += 1;
                        }
                    }
                } else {
                    // Everyone is in: now and then a breakdown drops a
                    // voice or two for a loop, and any dropped come back.
                    let dropped: Vec<usize> = (0..n).filter(|&i| !on[i]).collect();
                    for i in dropped {
                        on[i] = true;
                    }
                    if n > 2 && v.chance(0.3) {
                        let k = 1 + v.below(2) as usize;
                        for _ in 0..k {
                            let i = v.below(n as u32) as usize;
                            on[i] = false;
                        }
                    }
                }
            } else if pass >= thin_at && pass > 0 {
                // Thin out: voices leave in a shuffled order, the last
                // pass keeping one.
                let mut playing: Vec<usize> = (0..n).filter(|&i| on[i]).collect();
                let left = passes - pass;
                let keep = if left <= 1 {
                    1
                } else {
                    playing.len().div_ceil(2).max(1)
                };
                v.shuffle(&mut playing);
                for &i in playing.iter().skip(keep) {
                    on[i] = false;
                }
            }
            if !on.iter().any(|&o| o) {
                on[order[0]] = true;
            }
            plan.push(on.clone());
        }
        // A voice the build never reached gets one loop in the fullest pass.
        let fullest = (0..plan.len())
            .max_by_key(|&p| plan[p].iter().filter(|&&o| o).count())
            .unwrap_or(0);
        for i in 0..n {
            if !plan.iter().any(|p| p[i]) {
                plan[fullest][i] = true;
            }
        }
        plan
    }

    fn passes(&self, voices: usize) -> usize {
        let loop_s = self.loop_s.max(0.5);
        let wanted = (self.length_s / loop_s).round() as usize;
        wanted.max(voices + 4).max(2)
    }
}

/// What a track plays.
#[derive(Clone)]
pub enum Piece {
    /// One recording, played `repeats` times through (a loop played once
    /// is just the loop).
    Clip { key: String, repeats: u32 },
    /// Layers arranged anew each play.
    Stems(Stems),
    /// The game's own performer.
    Live(PerformerFactory),
}

impl std::fmt::Debug for Piece {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Piece::Clip { key, repeats } => write!(f, "Clip({key:?} x{repeats})"),
            Piece::Stems(s) => write!(f, "Stems({s:?})"),
            Piece::Live(_) => f.write_str("Live"),
        }
    }
}

/// One piece of a game's soundtrack.
#[derive(Clone, Debug)]
pub struct Track {
    /// Stable save id, unique in the game. Never change one that shipped.
    pub id: String,
    pub title: String,
    /// Read on F1 once it is unlocked: where the piece is heard.
    pub about: String,
    /// Read while it is locked: how to earn it.
    pub hint: String,
    /// The section it is listed under (a game's name in a collection);
    /// [`SoundtrackScreen::album`] lists one section.
    pub album: String,
    /// Not listed or counted until unlocked.
    pub secret: bool,
    /// Unlocked from the start.
    pub open: bool,
    /// Sound keys that unlock it the first time one starts.
    pub unlocked_by: Vec<String>,
    pub piece: Piece,
    pub variation: Variation,
}

impl Track {
    fn new(id: impl Into<String>, title: impl Into<String>, piece: Piece) -> Track {
        Track {
            id: id.into(),
            title: title.into(),
            about: String::new(),
            hint: String::new(),
            album: String::new(),
            secret: false,
            open: false,
            unlocked_by: Vec::new(),
            piece,
            variation: Variation::GENTLE,
        }
    }

    /// One recording, unlocked the first time `key` is heard.
    pub fn clip(id: impl Into<String>, title: impl Into<String>, key: impl Into<String>) -> Track {
        let key = key.into();
        let mut t = Track::new(
            id,
            title,
            Piece::Clip {
                key: key.clone(),
                repeats: 1,
            },
        );
        t.unlocked_by = vec![key];
        t
    }

    /// Layers arranged anew each play. Unlocked by [`Track::unlocked_by`]
    /// or [`Ctx::unlock_track`].
    pub fn stems(id: impl Into<String>, title: impl Into<String>, stems: Stems) -> Track {
        Track::new(id, title, Piece::Stems(stems))
    }

    /// The game's own performer, made fresh for each play.
    pub fn live(
        id: impl Into<String>,
        title: impl Into<String>,
        make: impl Fn(Variety) -> Box<dyn Performer> + 'static,
    ) -> Track {
        Track::new(id, title, Piece::Live(Rc::new(make)))
    }

    pub fn about(mut self, text: impl Into<String>) -> Track {
        self.about = text.into();
        self
    }

    pub fn hint(mut self, text: impl Into<String>) -> Track {
        self.hint = text.into();
        self
    }

    pub fn album(mut self, name: impl Into<String>) -> Track {
        self.album = name.into();
        self
    }

    pub fn secret(mut self) -> Track {
        self.secret = true;
        self
    }

    pub fn open(mut self) -> Track {
        self.open = true;
        self
    }

    /// Replaces the keys that unlock it when heard.
    pub fn unlocked_by(mut self, keys: impl IntoIterator<Item = impl Into<String>>) -> Track {
        self.unlocked_by = keys.into_iter().map(Into::into).collect();
        self
    }

    /// For a clip: how many times through one play goes.
    pub fn repeats(mut self, n: u32) -> Track {
        if let Piece::Clip { repeats, .. } = &mut self.piece {
            *repeats = n.max(1);
        }
        self
    }

    pub fn variation(mut self, variation: Variation) -> Track {
        self.variation = variation;
        self
    }

    /// A performer for one play of this track.
    pub fn perform(&self, mut v: Variety) -> Box<dyn Performer> {
        match &self.piece {
            Piece::Clip { key, repeats } => Box::new(ClipPerformer {
                key: key.clone(),
                repeats: *repeats,
                pitch: self.variation.pitch(&mut v),
                level: self.variation.level(&mut v).min(1.0),
                sound: None,
                left: None,
            }),
            Piece::Stems(stems) => Box::new(Arrangement::new(stems, self.variation, v)),
            Piece::Live(make) => make(v),
        }
    }
}

/// How long a stopped piece fades, and how long a voice takes to come in
/// or leave in an arrangement.
const STOP_FADE_MS: u32 = 600;
const VOICE_FADE_MS: u32 = 350;
const END_FADE_MS: u32 = 2500;

struct ClipPerformer {
    key: String,
    repeats: u32,
    pitch: f32,
    level: f32,
    sound: Option<Sound>,
    /// Seconds left before a repeated clip is let go.
    left: Option<f64>,
}

impl Performer for ClipPerformer {
    fn start(&mut self, audio: &mut Audio) {
        let start = Start::ONCE.pitch(self.pitch).volume(self.level);
        if self.repeats > 1 {
            let s = audio.start_with(
                &self.key,
                Bus::MUSIC,
                Start {
                    looping: true,
                    ..start
                },
            );
            match audio.length_s(s) {
                Some(len) if len > 0.0 => {
                    self.left = Some(len * self.repeats as f64 / self.pitch as f64);
                    self.sound = Some(s);
                    return;
                }
                _ => audio.stop(s, 0),
            }
        }
        self.sound = Some(audio.start_with(&self.key, Bus::MUSIC, start));
    }

    fn update(&mut self, audio: &mut Audio, dt: f64) -> bool {
        let Some(s) = self.sound else {
            return false;
        };
        if let Some(left) = self.left.as_mut() {
            *left -= dt;
            if *left <= 0.0 {
                self.left = None;
                audio.release(s, END_FADE_MS);
            }
        }
        audio.is_playing(s)
    }

    fn stop(&mut self, audio: &mut Audio) {
        if let Some(s) = self.sound.take() {
            audio.stop(s, STOP_FADE_MS);
        }
    }
}

/// One play of [`Stems`]: every stem starts together and keeps looping, so
/// they never drift apart, and the plan fades voices in and out at loop
/// boundaries.
struct Arrangement {
    bed: Vec<String>,
    voices: Vec<String>,
    plan: Vec<Vec<bool>>,
    loop_s: f64,
    pitch: f32,
    levels: Vec<f32>,
    pans: Vec<f32>,
    bed_level: f32,
    bed_sounds: Vec<Sound>,
    voice_sounds: Vec<Sound>,
    clock: f64,
    pass: usize,
    ending: Option<f64>,
}

impl Arrangement {
    fn new(stems: &Stems, variation: Variation, mut v: Variety) -> Arrangement {
        let pitch = variation.pitch(&mut v);
        let plan = stems.plan(&mut v);
        let levels = stems
            .voices
            .iter()
            .map(|_| (stems.level * variation.level(&mut v)).clamp(0.0, 1.0))
            .collect();
        let pans = stems
            .voices
            .iter()
            .map(|_| v.spread(variation.pan.clamp(0.0, 1.0)))
            .collect();
        let bed_level = (stems.level * variation.level(&mut v)).clamp(0.0, 1.0);
        Arrangement {
            bed: stems.bed.clone(),
            voices: stems.voices.clone(),
            plan,
            // The loop runs faster or slower with the pitch.
            loop_s: stems.loop_s.max(0.5) / pitch as f64,
            pitch,
            levels,
            pans,
            bed_level,
            bed_sounds: Vec::new(),
            voice_sounds: Vec::new(),
            clock: 0.0,
            pass: 0,
            ending: None,
        }
    }

    fn apply_pass(&mut self, audio: &mut Audio, fade_ms: u32) {
        let Some(row) = self.plan.get(self.pass) else {
            return;
        };
        for (i, &s) in self.voice_sounds.iter().enumerate() {
            let target = if row.get(i).copied().unwrap_or(false) {
                self.levels[i]
            } else {
                0.0
            };
            audio.set_volume_over(s, target, fade_ms);
        }
    }

    fn all(&self) -> impl Iterator<Item = Sound> + '_ {
        self.bed_sounds
            .iter()
            .chain(self.voice_sounds.iter())
            .copied()
    }
}

impl Performer for Arrangement {
    fn start(&mut self, audio: &mut Audio) {
        let start = Start::LOOP.pitch(self.pitch);
        for key in &self.bed {
            let s = audio.start_with(key, Bus::MUSIC, start.volume(self.bed_level));
            self.bed_sounds.push(s);
        }
        for (i, key) in self.voices.iter().enumerate() {
            let on = self
                .plan
                .first()
                .and_then(|r| r.get(i))
                .copied()
                .unwrap_or(false);
            let level = if on { self.levels[i] } else { 0.0 };
            let s = audio.start_with(key, Bus::MUSIC, start.volume(level).pan(self.pans[i]));
            self.voice_sounds.push(s);
        }
    }

    fn update(&mut self, audio: &mut Audio, dt: f64) -> bool {
        if let Some(left) = self.ending.as_mut() {
            *left -= dt;
            return *left > 0.0;
        }
        self.clock += dt;
        let pass = (self.clock / self.loop_s).floor() as usize;
        if pass != self.pass {
            self.pass = pass;
            if pass >= self.plan.len() {
                let all: Vec<Sound> = self.all().collect();
                for s in all {
                    audio.stop(s, END_FADE_MS);
                }
                self.ending = Some(END_FADE_MS as f64 / 1000.0);
                return true;
            }
            self.apply_pass(audio, VOICE_FADE_MS);
        }
        true
    }

    fn stop(&mut self, audio: &mut Audio) {
        let all: Vec<Sound> = self.all().collect();
        for s in all {
            audio.stop(s, STOP_FADE_MS);
        }
        self.bed_sounds.clear();
        self.voice_sounds.clear();
    }
}

/// The unlocked ids, as [`FILE`] holds them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    #[serde(default)]
    pub unlocked: BTreeSet<String>,
}

/// Reads [`FILE`] through `vault`: the default [`Game::load_soundtrack`].
/// A file that fails its seal is set aside and the soundtrack starts over;
/// a plain file from before sealing is read only when `import_plain`.
pub fn load_from_vault(vault: &mut Vault, import_plain: bool) -> BTreeSet<String> {
    let parse = |bytes: &[u8]| {
        serde_json::from_slice::<Saved>(bytes)
            .map(|s| s.unlocked)
            .unwrap_or_else(|e| {
                log::warn!("{FILE} is unreadable, starting the soundtrack over: {e}");
                BTreeSet::new()
            })
    };
    match vault.read(FILE) {
        Opened::Missing => BTreeSet::new(),
        Opened::Sealed(bytes) | Opened::Plain(bytes) => parse(&bytes),
        Opened::Legacy(bytes) if import_plain => parse(&bytes),
        Opened::Legacy(_) | Opened::Tampered(_) => {
            log::warn!("{FILE} was changed outside the game; the soundtrack starts over");
            if let Err(e) = vault.set_aside(FILE) {
                log::warn!("could not set {FILE} aside: {e}");
            }
            BTreeSet::new()
        }
        Opened::Locked => {
            log::warn!("{FILE} is sealed by a release this build cannot open");
            BTreeSet::new()
        }
        Opened::Failed(e) => {
            log::warn!("could not read {FILE}: {e}");
            BTreeSet::new()
        }
    }
}

/// Writes [`FILE`] through `vault`: the default [`Game::save_soundtrack`].
pub fn save_to_vault(vault: &mut Vault, unlocked: &BTreeSet<String>) {
    let saved = Saved {
        unlocked: unlocked.clone(),
    };
    match serde_json::to_vec_pretty(&saved) {
        Ok(bytes) => {
            if let Err(e) = vault.write(FILE, &bytes) {
                log::warn!("could not save {FILE}: {e}");
            }
        }
        Err(e) => log::warn!("could not encode {FILE}: {e}"),
    }
}

/// The game's tracks and what the player has unlocked, at `ctx.soundtrack`.
pub struct Soundtrack {
    tracks: Vec<Track>,
    unlocked: BTreeSet<String>,
    loaded: bool,
    /// Unlocked since the last `take_new`, by title.
    new: Vec<String>,
    listening: bool,
    /// The watch list must be rebuilt.
    dirty: bool,
    /// A developer build with `<PREFIX>_ALL_TRACKS=1`.
    all: bool,
}

impl Soundtrack {
    pub fn new(tracks: Vec<Track>) -> Soundtrack {
        Soundtrack {
            tracks,
            unlocked: BTreeSet::new(),
            loaded: false,
            new: Vec::new(),
            listening: true,
            dirty: true,
            all: false,
        }
    }

    /// Replaces the track list. What is unlocked stays unlocked.
    pub fn set_tracks(&mut self, tracks: Vec<Track>) {
        self.tracks = tracks;
        self.dirty = true;
    }

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    pub fn track(&self, id: &str) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }

    /// Whether `id` is unlocked. Before the saved unlocks are read (the
    /// first frame), only open tracks are.
    pub fn is_unlocked(&self, id: &str) -> bool {
        self.all || self.unlocked.contains(id) || self.track(id).is_some_and(|t| t.open)
    }

    /// Whether hearing a track's keys unlocks it. Off for a screen that
    /// plays sounds out of play, such as a sound test, so previewing the
    /// title theme there does not unlock it. On by default.
    pub fn listen(&mut self, on: bool) {
        self.listening = on;
    }

    pub fn is_listening(&self) -> bool {
        self.listening
    }

    /// The titles of tracks unlocked since the last call, for the game to
    /// announce when it suits it (on a results screen, say).
    pub fn take_new(&mut self) -> Vec<String> {
        std::mem::take(&mut self.new)
    }

    /// The tracks a menu lists for `album` (every album when `None`): all
    /// but the secret ones still locked.
    pub fn listed(&self, album: Option<&str>) -> Vec<&Track> {
        self.tracks
            .iter()
            .filter(|t| album.is_none_or(|a| t.album == a))
            .filter(|t| !t.secret || self.is_unlocked(&t.id))
            .collect()
    }

    fn mark(&mut self, id: &str) -> bool {
        if self.is_unlocked(id) {
            return false;
        }
        let Some(title) = self.track(id).map(|t| t.title.clone()) else {
            return false;
        };
        self.unlocked.insert(id.to_string());
        self.new.push(title);
        self.dirty = true;
        true
    }

    /// The keys still worth listening for: those of locked tracks.
    fn watch_list(&self) -> Vec<String> {
        let mut keys: Vec<String> = self
            .tracks
            .iter()
            .filter(|t| !self.is_unlocked(&t.id))
            .flat_map(|t| t.unlocked_by.iter().cloned())
            .collect();
        keys.sort();
        keys.dedup();
        keys
    }
}

impl Default for Soundtrack {
    fn default() -> Self {
        Soundtrack::new(Vec::new())
    }
}

/// Whether this is a build `hearken release` made.
fn is_release() -> bool {
    option_env!("HEARKEN_RELEASE") == Some("1")
}

impl<G: Game> Ctx<G> {
    fn load_soundtrack(&mut self) {
        if self.soundtrack.loaded {
            return;
        }
        self.soundtrack.loaded = true;
        let saved = G::load_soundtrack(self);
        self.soundtrack.unlocked.extend(saved);
        if !is_release() {
            let var = format!("{}_ALL_TRACKS", G::ENV_PREFIX);
            self.soundtrack.all = std::env::var(var).is_ok_and(|v| v == "1");
        }
        self.soundtrack.dirty = true;
    }

    fn save_soundtrack(&mut self) {
        if self.soundtrack.all {
            return;
        }
        let unlocked = self.soundtrack.unlocked.clone();
        G::save_soundtrack(self, &unlocked);
    }

    /// Unlocks track `id` and saves: true when it was locked. Its title is
    /// queued for [`Soundtrack::take_new`].
    pub fn unlock_track(&mut self, id: &str) -> bool {
        self.load_soundtrack();
        let fresh = self.soundtrack.mark(id);
        if fresh {
            self.save_soundtrack();
        }
        fresh
    }

    /// Whether track `id` is unlocked, reading the saved unlocks first if
    /// nothing has yet.
    pub fn track_unlocked(&mut self, id: &str) -> bool {
        self.load_soundtrack();
        self.soundtrack.is_unlocked(id)
    }

    /// Once a frame: unlocks the tracks whose keys were heard.
    pub(crate) fn update_soundtrack(&mut self) {
        if self.soundtrack.tracks.is_empty() {
            return;
        }
        self.load_soundtrack();
        let heard = self.audio.take_watched();
        if self.soundtrack.listening && !heard.is_empty() {
            let ids: Vec<String> = self
                .soundtrack
                .tracks
                .iter()
                .filter(|t| t.unlocked_by.iter().any(|k| heard.contains(k)))
                .map(|t| t.id.clone())
                .collect();
            let mut any = false;
            for id in ids {
                any |= self.soundtrack.mark(&id);
            }
            if any {
                self.save_soundtrack();
            }
        }
        if self.soundtrack.dirty {
            self.soundtrack.dirty = false;
            let keys = self.soundtrack.watch_list();
            self.audio.watch_keys(keys);
        }
    }
}

/// The soundtrack menu: Play all (shuffled), then one row per listed
/// track, then Back. Enter plays an unlocked track, or stops the one
/// playing; on a locked track it reads the hint. Whatever the music slot
/// was playing stops while the screen is open and comes back after.
pub struct SoundtrackScreen<G: Game> {
    core: MenuCore<Self, G>,
    album: Option<String>,
    name: &'static str,
    playing: Option<(String, Box<dyn Performer>)>,
    /// Ids still to play after this one, for Play all.
    queue: Vec<String>,
    /// The music slot's track when the screen opened.
    resume_music: Option<(String, bool)>,
}

/// [`SoundtrackScreen`]'s default name.
pub const SCREEN_NAME: &str = "soundtrack";

impl<G: Game> SoundtrackScreen<G> {
    /// Every album, titled "Soundtrack".
    pub fn new() -> Self {
        SoundtrackScreen {
            core: MenuCore::new("Soundtrack").with_intro_help(
                "Up and Down move, Enter plays or stops a track, Escape goes back. \
                 Each play is a new performance.",
            ),
            album: None,
            name: SCREEN_NAME,
            playing: None,
            queue: Vec::new(),
            resume_music: None,
        }
    }

    /// Lists only `album`'s tracks, under the title `"<album> soundtrack"`.
    pub fn album(mut self, album: impl Into<String>) -> Self {
        let album = album.into();
        self.core.title = format!("{album} soundtrack");
        self.album = Some(album);
        self
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.core.title = title.into();
        self
    }

    /// The screen's [`crate::State::name`].
    pub fn named(mut self, name: &'static str) -> Self {
        self.name = name;
        self
    }

    /// The id of the track playing, for tests.
    pub fn playing(&self) -> Option<&str> {
        self.playing.as_ref().map(|(id, _)| id.as_str())
    }

    fn stop(&mut self, ctx: &mut Ctx<G>) {
        if let Some((_, mut p)) = self.playing.take() {
            p.stop(&mut ctx.audio);
        }
    }

    fn play(&mut self, ctx: &mut Ctx<G>, id: &str) {
        self.stop(ctx);
        let Some(track) = ctx.soundtrack.track(id).cloned() else {
            return;
        };
        if !ctx.track_unlocked(id) {
            return;
        }
        let mut p = track.perform(Variety::fresh());
        p.start(&mut ctx.audio);
        self.playing = Some((track.id, p));
    }

    fn play_next(&mut self, ctx: &mut Ctx<G>) -> bool {
        while let Some(id) = self.queue.pop() {
            if ctx.track_unlocked(&id) {
                self.play(ctx, &id);
                return true;
            }
        }
        false
    }

    fn counts(&self, ctx: &Ctx<G>) -> (usize, usize) {
        let listed = ctx.soundtrack.listed(self.album.as_deref());
        let unlocked = listed
            .iter()
            .filter(|t| ctx.soundtrack.is_unlocked(&t.id))
            .count();
        (unlocked, listed.len())
    }

    fn row(track: &Track) -> MenuItem<Self, G> {
        let id = track.id.clone();
        let label_id = id.clone();
        let help_id = id.clone();
        MenuItem::new(
            crate::menu::Label::dynamic(move |s: &Self, ctx: &Ctx<G>| {
                let Some(t) = ctx.soundtrack.track(&label_id) else {
                    return String::new();
                };
                if !ctx.soundtrack.is_unlocked(&label_id) {
                    return "Locked track".to_string();
                }
                if s.playing() == Some(label_id.as_str()) {
                    format!("{}, playing", t.title)
                } else {
                    t.title.clone()
                }
            }),
            move |s: &mut Self, ctx: &mut Ctx<G>| {
                if !ctx.track_unlocked(&id) {
                    let hint = ctx
                        .soundtrack
                        .track(&id)
                        .map(|t| t.hint.clone())
                        .filter(|h| !h.is_empty())
                        .unwrap_or_else(|| "Keep playing to unlock it.".to_string());
                    ctx.say(format!("Locked. {}", crate::text::end_sentence(&hint)));
                    return;
                }
                s.queue.clear();
                if s.playing() == Some(id.as_str()) {
                    s.stop(ctx);
                    ctx.say("Stopped.");
                } else {
                    s.play(ctx, &id);
                    let title = ctx.soundtrack.track(&id).map(|t| t.title.clone());
                    ctx.say(format!("Playing {}.", title.unwrap_or_default()));
                }
            },
        )
        .select_sound(None)
        .help(crate::menu::Label::dynamic(
            move |_s: &Self, ctx: &Ctx<G>| match ctx.soundtrack.track(&help_id) {
                Some(t) if ctx.soundtrack.is_unlocked(&help_id) && !t.about.is_empty() => {
                    t.about.clone()
                }
                Some(t) if !ctx.soundtrack.is_unlocked(&help_id) && !t.hint.is_empty() => {
                    format!("Locked. {}", t.hint)
                }
                _ => "Enter plays it, and Enter again stops it.".to_string(),
            },
        ))
    }
}

impl<G: Game> Default for SoundtrackScreen<G> {
    fn default() -> Self {
        SoundtrackScreen::new()
    }
}

impl<G: Game> Menu for SoundtrackScreen<G> {
    type Game = G;

    fn name(&self) -> &'static str {
        self.name
    }

    fn menu(&self) -> &MenuCore<Self, G> {
        &self.core
    }

    fn menu_mut(&mut self) -> &mut MenuCore<Self, G> {
        &mut self.core
    }

    fn build_items(&mut self, ctx: &mut Ctx<G>) -> Vec<MenuItem<Self, G>> {
        ctx.load_soundtrack();
        let mut items = Vec::new();
        let (unlocked, _) = self.counts(ctx);
        if unlocked > 1 {
            items.push(
                MenuItem::new("Play all, shuffled", |s: &mut Self, ctx: &mut Ctx<G>| {
                    let mut ids: Vec<String> = ctx
                        .soundtrack
                        .listed(s.album.as_deref())
                        .iter()
                        .filter(|t| ctx.soundtrack.is_unlocked(&t.id))
                        .map(|t| t.id.clone())
                        .collect();
                    Variety::fresh().shuffle(&mut ids);
                    s.queue = ids;
                    if s.play_next(ctx) {
                        let title = s
                            .playing()
                            .and_then(|id| ctx.soundtrack.track(id))
                            .map(|t| t.title.clone())
                            .unwrap_or_default();
                        ctx.say(format!("Playing all, shuffled. {title}."));
                    }
                })
                .select_sound(None)
                .help("Plays every unlocked track in a random order."),
            );
        }
        let tracks: Vec<Track> = ctx
            .soundtrack
            .listed(self.album.as_deref())
            .into_iter()
            .cloned()
            .collect();
        for t in &tracks {
            items.push(Self::row(t));
        }
        if tracks.is_empty() {
            items.push(MenuItem::inert("No tracks yet."));
        }
        items.push(
            MenuItem::new("Back", |s: &mut Self, ctx: &mut Ctx<G>| s.on_escape(ctx))
                .select_sound(None),
        );
        items
    }

    fn enter(&mut self, ctx: &mut Ctx<G>) {
        self.resume_music = ctx
            .audio
            .music()
            .map(|k| (k.to_string(), ctx.audio.music_looping()));
        ctx.audio.play_music(None, false);
        crate::menu::menu_enter(self, ctx);
    }

    fn exit(&mut self, ctx: &mut Ctx<G>) {
        self.queue.clear();
        self.stop(ctx);
        if let Some((key, looping)) = self.resume_music.take() {
            ctx.audio.play_music(Some(&key), looping);
        }
    }

    fn announce(&mut self, ctx: &mut Ctx<G>) {
        let (unlocked, listed) = self.counts(ctx);
        let title = crate::text::end_sentence(&self.core.title);
        ctx.say(format!("{title} {unlocked} of {listed} tracks unlocked."));
        let current = self.current_text(ctx);
        ctx.say_queued(current);
    }

    fn update(&mut self, ctx: &mut Ctx<G>, dt: f64) {
        let Some((_, p)) = self.playing.as_mut() else {
            return;
        };
        if !p.update(&mut ctx.audio, dt) {
            self.playing = None;
            self.play_next(ctx);
        }
    }

    fn on_escape(&mut self, ctx: &mut Ctx<G>) {
        ctx.play_ui(UiSound::Back);
        ctx.pop();
    }
}
