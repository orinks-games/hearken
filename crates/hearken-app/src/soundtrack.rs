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
//! # As heard, or varied
//!
//! The menu plays a track as the player heard it in the game
//! ([`Track::perform_as_heard`]): a clip as recorded, stems joining one
//! loop at a time in their listed order. A track marked [`Track::varied`]
//! is performed afresh every time instead, in the menu too, from a new
//! [`Variety`]:
//!
//! - [`Piece::Stems`] arranges a piece's looping layers anew every time:
//!   which voices open it, the order they join in, where some drop out and
//!   come back, how it thins out at the end, how long it runs, and each
//!   voice's level and place in the stereo field.
//! - [`Variation`] moves the whole piece up or down a semitone by default (key
//!   and tempo together, as a turntable would; never left at the recorded
//!   pitch) and shades the mix. With no bed, no loop holds fewer than two
//!   voices.
//! - [`Piece::Live`] hands the performance to the game's own
//!   [`Performer`], which can compose as it goes.
//!
//! The stems stay in the game's sound pack and are only ever heard mixed.
//! A piece whose stems will not all load (a damaged pack entry) does not
//! play at all, rather than play the ones that did.
//!
//! # Exporting an album
//!
//! An album the game opens with [`Soundtrack::set_album`] and
//! [`AlbumOptions::export`] gets an "Export" row in the menu. It is locked
//! until every track listed in the album is unlocked (a secret track counts
//! once heard) and says how many are. Each track is then played on a
//! mixer with no device, faster than real time, and its finished mix
//! written as a WAV file to `Music/<game>/<album>/` (the game's data
//! folder when there is no Music folder; see [`export`]). With
//! [`AlbumOptions::vary_export`] each export is a fresh variation;
//! otherwise it is the track as heard.
//!
//! Arranging needs layers to move: at least three voices with no bed, or
//! two over a bed ([`Stems::arranges`]). With fewer, every loop holds every
//! voice and only the length, key and mix change from play to play.
//!
//! What this cannot stop, and nothing that plays sound on a player's
//! computer can:
//!
//! - Recording what comes out of the speakers. A voice's level and place
//!   hold for a whole play and the stems stay sample-aligned, so someone
//!   patient can subtract one loop from the next to pull out the voices
//!   that changed between them, and undo the pitch change, which is a
//!   plain resample by a known step. Any loop where every voice plays is
//!   the full mix of that loop.
//! - Digging the pack key out of the program (see `hearken-pack`).
//!
//! # Saving
//!
//! The unlocked ids go to `soundtrack.json` in the game's data folder
//! through [`crate::Vault`], so a release made with `seal = true` seals
//! them like any other save: an edited or swapped file is set aside and the
//! soundtrack starts over. A save keeps what the file already holds and
//! writes nothing while the file cannot be read, so a read that failed
//! once never costs the player their tracks; a save that could not be
//! made is tried again every few seconds, and as the game closes. A game that keeps its
//! progress elsewhere overrides [`Game::load_soundtrack`] and
//! [`Game::save_soundtrack`].
//!
//! The seal ties the file to the game, not to the player: a sealed
//! `soundtrack.json` copied from another player of the same game, or an
//! old copy of one's own, is taken as it is. No offline game can stop
//! that.
//!
//! A developer build (any build `hearken release` did not make) opens every
//! track while `<PREFIX>_ALL_TRACKS=1` is set, to audition them, and saves
//! nothing then.

use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

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
    /// The furthest a play moves from the recorded key: an octave.
    pub const MAX_SEMITONES: u8 = 12;

    /// Every play as recorded.
    pub const NONE: Variation = Variation {
        semitones: 0,
        level_db: 0.0,
        pan: 0.0,
    };

    /// A semitone up or down, levels within 2 dB, voices spread a quarter
    /// of the way out: different each time, and still the same piece.
    pub const GENTLE: Variation = Variation {
        semitones: 1,
        level_db: 2.0,
        pan: 0.25,
    };

    /// The playback rate for this play. With any `semitones` at all, a
    /// play is never at the recorded pitch, so no play is the shipped file
    /// as it is. More than [`Variation::MAX_SEMITONES`] counts as that many,
    /// which keeps the rate where the mixer can play it (`0.5..=2`).
    pub fn pitch(&self, v: &mut Variety) -> f32 {
        let s = self.semitones.min(Variation::MAX_SEMITONES) as i32;
        if s == 0 {
            return 1.0;
        }
        let step = v.range(1, s);
        let step = if v.chance(0.5) { step } else { -step };
        2f32.powf(step as f32 / 12.0)
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
/// `update` with no time passed (false there means nothing could play, and
/// the player is told so), then `update` every frame until it returns
/// false or the player stops it (`stop`). Every sound it starts should go
/// on [`Bus::MUSIC`].
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
    /// About how long a play runs, in seconds: each play runs up to a fifth
    /// longer or shorter, in whole loops, and always long enough for every
    /// voice to join and leave. At most an hour; anything not a number of
    /// seconds is the default, 150.
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
            length_s: DEFAULT_LENGTH_S,
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

    /// Whether a play can change which layers sound: at least three voices
    /// with no bed, or two over a bed. With fewer, every loop holds every
    /// voice, so each play is the full mix throughout, changed only in
    /// length, key and level.
    pub fn arranges(&self) -> bool {
        let n = self.voices.len();
        if self.bed.is_empty() {
            n >= 3
        } else {
            n >= 2
        }
    }

    /// Whether `loop_s` is a length a piece can loop on: a number of
    /// seconds, at most [`Stems::MAX_LOOP_S`].
    pub fn loop_is_valid(&self) -> bool {
        self.loop_s.is_finite() && self.loop_s <= Stems::MAX_LOOP_S
    }

    /// The longest loop a piece may have, in seconds: ten minutes.
    pub const MAX_LOOP_S: f64 = 600.0;

    /// Which voices play in each loop of one performance: `plan[pass][voice]`.
    /// Every voice is heard, the last pass is never silent, and the plan
    /// builds up, holds, breaks down and thins out. With no bed, no loop
    /// holds fewer than two voices, so no stem is ever heard alone.
    pub fn plan(&self, v: &mut Variety) -> Vec<Vec<bool>> {
        let n = self.voices.len();
        let passes = self.passes(n, v);
        if n == 0 {
            return vec![Vec::new(); passes];
        }
        // The fewest voices a loop holds: one over a bed, else two.
        let floor = if self.bed.is_empty() { n.min(2) } else { 1 };
        // The entry order: the given order, neighbours swapped now and then.
        let mut order: Vec<usize> = (0..n).collect();
        for i in 1..n {
            if v.chance(0.35) {
                order.swap(i - 1, i);
            }
        }
        let mut plan = Vec::with_capacity(passes);
        let mut on = vec![false; n];
        // Open with one or two voices.
        let opening = if n > 1 && v.chance(0.4) { 2 } else { 1 }.max(floor);
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
                // pass keeping as few as it may.
                let mut playing: Vec<usize> = (0..n).filter(|&i| on[i]).collect();
                let left = passes - pass;
                let keep = if left <= 1 {
                    floor
                } else {
                    playing.len().div_ceil(2).max(floor)
                };
                v.shuffle(&mut playing);
                for &i in playing.iter().skip(keep) {
                    on[i] = false;
                }
            }
            let mut count = on.iter().filter(|&&o| o).count();
            for &i in &order {
                if count >= floor {
                    break;
                }
                if !on[i] {
                    on[i] = true;
                    count += 1;
                }
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

    /// The plan of a play as the player hears it in the game: the voices
    /// join one loop at a time in their given order, then all play on to
    /// about `length_s`.
    pub fn plan_as_heard(&self) -> Vec<Vec<bool>> {
        let n = self.voices.len();
        let passes = self.passes(n, &mut Variety::new(0)).max(n + 2);
        (0..passes)
            .map(|pass| (0..n).map(|i| i <= pass).collect())
            .collect()
    }

    /// How many loops this play runs: about `length_s`, a fifth either way.
    fn passes(&self, voices: usize, v: &mut Variety) -> usize {
        let loop_s = self.loop_s.clamp(0.5, Stems::MAX_LOOP_S);
        let length_s = if self.length_s.is_finite() {
            self.length_s.clamp(0.0, MAX_LENGTH_S)
        } else {
            DEFAULT_LENGTH_S
        };
        let wanted = (length_s / loop_s).round() as i64;
        let spread = (wanted / 5).max(1) as i32;
        let wanted = (wanted + v.range(-spread, spread) as i64).max(0) as usize;
        wanted.max(voices + 4).max(2)
    }
}

/// How long a play of [`Stems`] runs by default, and at most, in seconds.
const DEFAULT_LENGTH_S: f64 = 150.0;
const MAX_LENGTH_S: f64 = 3600.0;

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
    /// Every play varies, in the menu too ([`Track::varied`]).
    pub varied: bool,
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
            varied: false,
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

    /// How far a varied play strays ([`Track::varied`], or an export from
    /// an album with [`AlbumOptions::vary_export`]).
    pub fn variation(mut self, variation: Variation) -> Track {
        self.variation = variation;
        self
    }

    /// Every play of this track varies, in the soundtrack menu too: for
    /// music the game itself plays differently each time, such as a tune it
    /// makes up. Without it the menu plays a track as the player heard it.
    pub fn varied(mut self) -> Track {
        self.varied = true;
        self
    }

    /// A performer for one play as the player hears it in the game: a clip
    /// as recorded, stems joining one loop at a time in their given order
    /// then all playing to the end, a live piece from a fixed seed. A
    /// [`Track::varied`] track varies anyway, as it does in play.
    pub fn perform_as_heard(&self) -> Box<dyn Performer> {
        if self.varied {
            return self.perform(Variety::fresh());
        }
        match &self.piece {
            Piece::Clip { .. } => Track {
                variation: Variation::NONE,
                ..self.clone()
            }
            .perform(Variety::new(0)),
            Piece::Stems(stems) => Box::new(Arrangement::as_heard(stems)),
            Piece::Live(make) => make(Variety::new(0)),
        }
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
    /// Seconds left before a repeated clip is let go: halfway through its
    /// last time round, so it plays out to its end from there.
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
                    let rounds = self.repeats as f64 - 0.5;
                    self.left = Some(len * rounds / self.pitch as f64);
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

/// One play of [`Stems`]: every stem starts on the same sample and keeps
/// looping, so they never drift apart, and the plan fades voices in and out
/// at loop boundaries.
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
    /// Seconds played, as the frames tell it and as the wall clock does.
    /// The music keeps time by the later of the two: a frame that stalled
    /// is reported short, but the stems played on through it.
    frames_s: f64,
    real_s: f64,
    last: Option<Instant>,
    pass: usize,
    /// When the closing fade ends, on the clock above.
    ending_at: Option<f64>,
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
        let valid = stems.loop_is_valid();
        if !valid {
            log::warn!(
                "stems {:?}: a loop of {} s cannot play; give loop_s in seconds, at most {}",
                stems.voices,
                stems.loop_s,
                Stems::MAX_LOOP_S
            );
        } else if !stems.arranges() {
            log::warn!(
                "stems {:?}: too few layers to arrange, so every loop is the full mix; \
                 give three voices, or two over a bed",
                stems.voices
            );
        }
        let keys = |k: &Vec<String>| if valid { k.clone() } else { Vec::new() };
        Arrangement {
            bed: keys(&stems.bed),
            voices: keys(&stems.voices),
            plan,
            // The loop runs faster or slower with the pitch.
            loop_s: stems.loop_s.clamp(0.5, Stems::MAX_LOOP_S) / pitch as f64,
            pitch,
            levels,
            pans,
            bed_level,
            bed_sounds: Vec::new(),
            voice_sounds: Vec::new(),
            frames_s: 0.0,
            real_s: 0.0,
            last: None,
            pass: 0,
            ending_at: None,
        }
    }

    /// One play as the player hears it: recorded key and levels, centred,
    /// voices joining in order ([`Stems::plan_as_heard`]).
    fn as_heard(stems: &Stems) -> Arrangement {
        let mut a = Arrangement::new(stems, Variation::NONE, Variety::new(0));
        a.plan = stems.plan_as_heard();
        a.levels = vec![stems.level.clamp(0.0, 1.0); stems.voices.len()];
        a.pans = vec![0.0; stems.voices.len()];
        a.bed_level = stems.level.clamp(0.0, 1.0);
        a
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

    fn stop_all(&mut self, audio: &mut Audio, fade_ms: u32) {
        let all: Vec<Sound> = self.all().collect();
        for s in all {
            audio.stop(s, fade_ms);
        }
        self.bed_sounds.clear();
        self.voice_sounds.clear();
    }
}

impl Performer for Arrangement {
    fn start(&mut self, audio: &mut Audio) {
        let start = Start::LOOP.pitch(self.pitch);
        let mut starts: Vec<(&str, Bus, Start)> = Vec::new();
        for key in &self.bed {
            starts.push((key, Bus::MUSIC, start.volume(self.bed_level)));
        }
        for (i, key) in self.voices.iter().enumerate() {
            let on = self
                .plan
                .first()
                .and_then(|r| r.get(i))
                .copied()
                .unwrap_or(false);
            let level = if on { self.levels[i] } else { 0.0 };
            starts.push((key, Bus::MUSIC, start.volume(level).pan(self.pans[i])));
        }
        // All on one sample, however long each takes to decode.
        let mut sounds = audio.start_together(&starts);
        // A stem that will not play, or plays its default instead, while
        // others play: the piece is damaged, and what is left would lay
        // the rest bare. Nothing plays. (With every stem missing, as in a
        // build without its sounds yet, the defaults play as placeholders.)
        let missing = audio.missing_clips();
        let bad = starts
            .iter()
            .zip(&sounds)
            .filter(|((key, _, _), &s)| s == Sound::NONE || missing.iter().any(|m| m == key))
            .count();
        if bad > 0 && bad < starts.len() {
            log::warn!("a piece's stems did not all load, so it does not play");
            for s in sounds.drain(..) {
                audio.stop(s, 0);
            }
            return;
        }
        self.voice_sounds = sounds.split_off(self.bed.len());
        self.bed_sounds = sounds;
        self.last = Some(Instant::now());
    }

    fn update(&mut self, audio: &mut Audio, dt: f64) -> bool {
        // Nothing would play (every stem missing or refused): done, rather
        // than hold silence for the whole piece.
        if !self.all().any(|s| audio.is_playing(s)) {
            return false;
        }
        let now = Instant::now();
        let real = self
            .last
            .replace(now)
            .map_or(0.0, |t| now.duration_since(t).as_secs_f64());
        // Offline, time is only the frames rendered.
        let real = if audio.is_offline() { 0.0 } else { real };
        // Paused stems do not move on.
        if !self.all().any(|s| audio.is_paused(s)) {
            self.frames_s += dt.max(0.0);
            self.real_s += real;
        }
        let clock = self.frames_s.max(self.real_s);
        if let Some(end) = self.ending_at {
            if clock >= end {
                self.stop_all(audio, 0);
                return false;
            }
            return true;
        }
        let pass = (clock / self.loop_s).floor() as usize;
        if pass != self.pass {
            self.pass = pass;
            if pass >= self.plan.len() {
                // Faded down but still held, so a stop now still cuts it
                // short.
                let all: Vec<Sound> = self.all().collect();
                for s in all {
                    audio.set_volume_over(s, 0.0, END_FADE_MS);
                }
                self.ending_at = Some(clock + END_FADE_MS as f64 / 1000.0);
                return true;
            }
            self.apply_pass(audio, VOICE_FADE_MS);
        }
        true
    }

    fn stop(&mut self, audio: &mut Audio) {
        self.stop_all(audio, STOP_FADE_MS);
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
/// It keeps whatever the file already holds, and writes nothing while the
/// file cannot be read, so a read that failed never costs the player the
/// tracks they earned. True once the file holds them.
pub fn save_to_vault(vault: &mut Vault, unlocked: &BTreeSet<String>) -> bool {
    let mut unlocked = unlocked.clone();
    match vault.read(FILE) {
        Opened::Sealed(bytes) | Opened::Plain(bytes) => {
            if let Ok(saved) = serde_json::from_slice::<Saved>(&bytes) {
                unlocked.extend(saved.unlocked);
            }
        }
        Opened::Failed(e) => {
            log::warn!("could not read {FILE}, so not saving over it yet: {e}");
            return false;
        }
        Opened::Locked => {
            log::warn!("{FILE} is sealed by a release this build cannot open; not saving");
            // Nothing this build does will change that: do not retry.
            return true;
        }
        Opened::Missing | Opened::Legacy(_) | Opened::Tampered(_) => {}
    }
    let saved = Saved { unlocked };
    match serde_json::to_vec_pretty(&saved) {
        Ok(bytes) => match vault.write(FILE, &bytes) {
            Ok(()) => true,
            Err(e) => {
                log::warn!("could not save {FILE} yet: {e}");
                false
            }
        },
        Err(e) => {
            log::warn!("could not encode {FILE}: {e}");
            true
        }
    }
}

/// How long after a save that could not be made it is tried again.
const SAVE_RETRY_S: f64 = 3.0;

/// What an album of the soundtrack offers beyond listening. Both are off
/// unless the game turns them on with [`Soundtrack::set_album`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AlbumOptions {
    /// The soundtrack menu offers to write the album to sound files, once
    /// every track listed in it is unlocked (see [`export`]).
    pub export: bool,
    /// Each exported track is a fresh variation ([`Track::perform`]), not
    /// the track as the player hears it.
    pub vary_export: bool,
}

/// The game's tracks and what the player has unlocked, at `ctx.soundtrack`.
pub struct Soundtrack {
    tracks: Vec<Track>,
    albums: std::collections::BTreeMap<String, AlbumOptions>,
    /// Where exports go, in place of the player's Music folder.
    export_dir: Option<std::path::PathBuf>,
    unlocked: BTreeSet<String>,
    loaded: bool,
    /// Unlocked since the last `take_new`, by title.
    new: Vec<String>,
    listening: bool,
    /// Listening was off at some point since the last frame's check: keys
    /// noted in that time were heard out of play and never count.
    deaf: bool,
    /// An unlock not yet saved, and when (on `ctx.clock`) to try again.
    unsaved: Option<f64>,
    /// The watch list must be rebuilt.
    dirty: bool,
    /// The track list changed: music already playing may belong to a track
    /// it added, and that start came before its key was watched.
    scan: bool,
    /// A developer build with `<PREFIX>_ALL_TRACKS=1`.
    all: bool,
}

impl Soundtrack {
    pub fn new(tracks: Vec<Track>) -> Soundtrack {
        Soundtrack {
            tracks,
            albums: Default::default(),
            export_dir: None,
            unlocked: BTreeSet::new(),
            loaded: false,
            new: Vec::new(),
            listening: true,
            deaf: false,
            unsaved: None,
            dirty: true,
            scan: true,
            all: false,
        }
    }

    /// Replaces the track list. What is unlocked stays unlocked.
    pub fn set_tracks(&mut self, tracks: Vec<Track>) {
        self.tracks = tracks;
        self.dirty = true;
        self.scan = true;
    }

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    /// Sets what `album` offers (`""` is a game whose tracks have no
    /// album). Every album starts with [`AlbumOptions::default`]: listen
    /// only.
    pub fn set_album(&mut self, album: impl Into<String>, options: AlbumOptions) {
        self.albums.insert(album.into(), options);
    }

    pub fn album_options(&self, album: &str) -> AlbumOptions {
        self.albums.get(album).copied().unwrap_or_default()
    }

    /// The albums, in the order their first track is listed.
    pub fn albums(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for t in &self.tracks {
            if !out.contains(&t.album) {
                out.push(t.album.clone());
            }
        }
        out
    }

    /// Writes exports under `dir` instead of the player's Music folder.
    pub fn set_export_dir(&mut self, dir: Option<std::path::PathBuf>) {
        self.export_dir = dir;
    }

    /// How many of `album`'s listed tracks are unlocked, and how many are
    /// listed (a secret one counts once it is unlocked). The album can be
    /// exported once the two match.
    pub fn album_progress(&self, album: &str) -> (usize, usize) {
        let listed = self.listed(Some(album));
        let unlocked = listed.iter().filter(|t| self.is_unlocked(&t.id)).count();
        (unlocked, listed.len())
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
    /// A key heard at any moment listening was off never counts, even if
    /// it is back on by the end of the frame.
    pub fn listen(&mut self, on: bool) {
        self.listening = on;
        if !on {
            self.deaf = true;
        }
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
        self.soundtrack.scan = true;
    }

    fn save_soundtrack(&mut self) {
        if self.soundtrack.all {
            return;
        }
        // Read again first: if the first read failed (a file held by a
        // virus scanner, say), what it missed comes back now instead of
        // being saved over.
        let saved = G::load_soundtrack(self);
        let before = self.soundtrack.unlocked.len();
        self.soundtrack.unlocked.extend(saved);
        if self.soundtrack.unlocked.len() != before {
            self.soundtrack.dirty = true;
        }
        let unlocked = self.soundtrack.unlocked.clone();
        self.soundtrack.unsaved = if G::save_soundtrack(self, &unlocked) {
            None
        } else {
            Some(self.clock + SAVE_RETRY_S)
        };
    }

    /// A last try at an unlock that could not be saved, as the game closes.
    pub(crate) fn flush_soundtrack(&mut self) {
        if self.soundtrack.unsaved.is_some() {
            self.save_soundtrack();
        }
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
        // Drained every frame, even with no tracks, so nothing heard now
        // is held over to count later.
        let heard = self.audio.take_watched();
        let deaf = !self.soundtrack.listening || self.soundtrack.deaf;
        self.soundtrack.deaf = !self.soundtrack.listening;
        if self.soundtrack.tracks.is_empty() {
            if self.soundtrack.dirty {
                self.soundtrack.dirty = false;
                self.audio.watch_keys(Vec::new());
            }
            return;
        }
        self.load_soundtrack();
        if self.soundtrack.unsaved.is_some_and(|at| self.clock >= at) {
            self.save_soundtrack();
        }
        let mut heard = if deaf { Vec::new() } else { heard };
        if self.soundtrack.dirty {
            self.soundtrack.dirty = false;
            let keys = self.soundtrack.watch_list();
            self.audio.watch_keys(keys);
        }
        if !deaf && self.soundtrack.scan {
            // Music that started before its key was watched (the first
            // screen's theme, a level's music started with its new list)
            // and is still playing counts as heard: a loop starts only once.
            self.soundtrack.scan = false;
            let watched: BTreeSet<String> = self.soundtrack.watch_list().into_iter().collect();
            heard.extend(
                self.audio
                    .live()
                    .into_iter()
                    .map(|l| l.key)
                    .filter(|k| watched.contains(k)),
            );
        }
        if !heard.is_empty() {
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
/// was playing is paused while the screen is open and goes on from the
/// same place after. Nothing played here unlocks a track, and playback
/// stops when another screen covers this one.
pub struct SoundtrackScreen<G: Game> {
    core: MenuCore<Self, G>,
    album: Option<String>,
    name: &'static str,
    playing: Option<(String, Box<dyn Performer>)>,
    /// Ids still to play after this one, for Play all.
    queue: Vec<String>,
    /// The music slot's sound this screen paused, to resume on leaving.
    held_music: Option<Sound>,
    /// Whether the soundtrack was listening when the screen opened.
    was_listening: bool,
    /// An album being written to files.
    #[cfg(feature = "kira")]
    export: Option<Export>,
}

/// An album being written, a slice a frame.
#[cfg(feature = "kira")]
struct Export {
    album: String,
    dir: std::path::PathBuf,
    vary: bool,
    tracks: Vec<Track>,
    at: usize,
    render: Option<export::Render>,
    written: usize,
    failed: usize,
}

/// How long an export may work each frame, so the game keeps answering.
#[cfg(feature = "kira")]
const EXPORT_SLICE: std::time::Duration = std::time::Duration::from_millis(30);

/// Said when a track has nothing that will play.
const CANT_PLAY: &str = "Can't play that track.";

/// [`SoundtrackScreen`]'s default name.
pub const SCREEN_NAME: &str = "soundtrack";

impl<G: Game> SoundtrackScreen<G> {
    /// Every album, titled "Soundtrack".
    pub fn new() -> Self {
        SoundtrackScreen {
            core: MenuCore::new("Soundtrack").with_intro_help(
                "Up and Down move, Enter plays or stops a track, Escape goes back.",
            ),
            album: None,
            name: SCREEN_NAME,
            playing: None,
            queue: Vec::new(),
            held_music: None,
            was_listening: true,
            #[cfg(feature = "kira")]
            export: None,
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

    /// Starts track `id`: false when it is locked or nothing of it plays.
    fn play(&mut self, ctx: &mut Ctx<G>, id: &str) -> bool {
        self.stop(ctx);
        let Some(track) = ctx.soundtrack.track(id).cloned() else {
            return false;
        };
        if !ctx.track_unlocked(id) {
            return false;
        }
        let mut p = track.perform_as_heard();
        p.start(&mut ctx.audio);
        if !p.update(&mut ctx.audio, 0.0) {
            p.stop(&mut ctx.audio);
            return false;
        }
        self.playing = Some((track.id, p));
        true
    }

    /// The playing track's title, for speech.
    fn playing_title(&self, ctx: &Ctx<G>) -> String {
        self.playing()
            .and_then(|id| ctx.soundtrack.track(id))
            .map(|t| crate::text::end_sentence(&t.title))
            .unwrap_or_default()
    }

    fn play_next(&mut self, ctx: &mut Ctx<G>) -> bool {
        while let Some(id) = self.queue.pop() {
            if self.play(ctx, &id) {
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

    /// The rows that export albums: one for the album this screen lists,
    /// or one per album that offers it.
    #[cfg(feature = "kira")]
    fn export_rows(&self, ctx: &Ctx<G>) -> Vec<MenuItem<Self, G>> {
        let albums = match &self.album {
            Some(a) => vec![a.clone()],
            None => ctx.soundtrack.albums(),
        };
        let one = self.album.is_some();
        albums
            .into_iter()
            .filter(|a| ctx.soundtrack.album_options(a).export)
            .map(|album| {
                let name = if one || album.is_empty() {
                    "album".to_string()
                } else {
                    album.clone()
                };
                let label_album = album.clone();
                let label_name = name.clone();
                MenuItem::new(
                    crate::menu::Label::dynamic(move |s: &Self, ctx: &Ctx<G>| {
                        if s.export.as_ref().is_some_and(|e| e.album == label_album) {
                            return format!("Export {label_name}, working");
                        }
                        let (heard, all) = ctx.soundtrack.album_progress(&label_album);
                        if all > 0 && heard == all {
                            format!("Export {label_name}")
                        } else {
                            format!("Export {label_name}, locked: {heard} of {all} heard")
                        }
                    }),
                    move |s: &mut Self, ctx: &mut Ctx<G>| s.start_export(ctx, &album),
                )
                .select_sound(None)
                .help(
                    "Writes every track of the album to sound files in your Music folder, \
                     once you have heard them all.",
                )
            })
            .collect()
    }

    #[cfg(feature = "kira")]
    fn start_export(&mut self, ctx: &mut Ctx<G>, album: &str) {
        if let Some(e) = &self.export {
            let n = e.tracks.len();
            ctx.say(format!("Exporting, track {} of {n}.", (e.at + 1).min(n)));
            return;
        }
        ctx.load_soundtrack();
        let (heard, all) = ctx.soundtrack.album_progress(album);
        if all == 0 || heard < all {
            ctx.say(format!(
                "Locked. Hear every track first: {heard} of {all} so far."
            ));
            return;
        }
        self.queue.clear();
        self.stop(ctx);
        let tracks: Vec<Track> = ctx
            .soundtrack
            .listed(Some(album))
            .into_iter()
            .cloned()
            .collect();
        let base = ctx
            .soundtrack
            .export_dir
            .clone()
            .unwrap_or_else(|| export::base(&ctx.paths.data_dir));
        let dir = export::folder(&base, G::TITLE, album);
        let n = tracks.len();
        let tracks_word = if n == 1 { "track" } else { "tracks" };
        ctx.say(format!(
            "Exporting {n} {tracks_word}. This takes a moment; Escape stops it."
        ));
        self.export = Some(Export {
            album: album.to_string(),
            dir,
            vary: ctx.soundtrack.album_options(album).vary_export,
            tracks,
            at: 0,
            render: None,
            written: 0,
            failed: 0,
        });
    }

    /// Works on the export for up to [`EXPORT_SLICE`].
    #[cfg(feature = "kira")]
    fn step_export(&mut self, ctx: &mut Ctx<G>) {
        let Some(e) = self.export.as_mut() else {
            return;
        };
        let began = Instant::now();
        while began.elapsed() < EXPORT_SLICE {
            if e.render.is_none() {
                let Some(track) = e.tracks.get(e.at) else {
                    break;
                };
                let performer = if e.vary {
                    track.perform(Variety::fresh())
                } else {
                    track.perform_as_heard()
                };
                let name = format!("{:02} {}.wav", e.at + 1, export::file_name(&track.title));
                let clips = crate::sound::clip_source::<G>(
                    hearken_audio::discover_clips(G::ENV_PREFIX).clips,
                );
                match export::Render::start(performer, clips, e.dir.join(name)) {
                    Ok(r) => e.render = Some(r),
                    Err(err) => {
                        log::warn!("export of {:?} failed: {err}", track.id);
                        e.failed += 1;
                        e.at += 1;
                        continue;
                    }
                }
            }
            let Some(r) = e.render.as_mut() else {
                break;
            };
            match r.step(export::RATE as usize / 4) {
                Ok(false) => {}
                Ok(true) => {
                    e.render = None;
                    e.written += 1;
                    e.at += 1;
                }
                Err(err) => {
                    log::warn!("export to {} failed: {err}", r.path().display());
                    if let Some(r) = e.render.take() {
                        r.cancel();
                    }
                    e.failed += 1;
                    e.at += 1;
                }
            }
        }
        if e.at >= e.tracks.len() {
            let e = self.export.take().expect("export in progress");
            let tracks_word = if e.written == 1 { "track" } else { "tracks" };
            let mut line = format!(
                "Exported {} {tracks_word} to {}.",
                e.written,
                e.dir.display()
            );
            if e.failed > 0 {
                line.push_str(&format!(" {} could not be written.", e.failed));
            }
            if e.written == 0 {
                line = "Could not export the album.".to_string();
            }
            ctx.say(line);
        }
    }

    /// Stops an export, removing its unfinished file: true when one ran.
    #[cfg(feature = "kira")]
    fn cancel_export(&mut self) -> bool {
        let Some(mut e) = self.export.take() else {
            return false;
        };
        if let Some(r) = e.render.take() {
            r.cancel();
        }
        true
    }

    #[cfg(not(feature = "kira"))]
    fn cancel_export(&mut self) -> bool {
        false
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
                } else if s.play(ctx, &id) {
                    let title = s.playing_title(ctx);
                    ctx.say(format!("Playing {title}"));
                } else {
                    ctx.say(CANT_PLAY);
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
                        let title = s.playing_title(ctx);
                        ctx.say(format!("Playing all, shuffled. {title}"));
                    } else {
                        ctx.say("Can't play these tracks.");
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
        #[cfg(feature = "kira")]
        items.extend(self.export_rows(ctx));
        items.push(
            MenuItem::new("Back", |s: &mut Self, ctx: &mut Ctx<G>| s.on_escape(ctx))
                .select_sound(None),
        );
        items
    }

    fn enter(&mut self, ctx: &mut Ctx<G>) {
        self.was_listening = ctx.soundtrack.is_listening();
        ctx.soundtrack.listen(false);
        // Hold the game's music where it is. Music already paused (from a
        // pause menu) is left as it is, for that menu to resume.
        self.held_music = ctx.audio.music_sound().filter(|&s| !ctx.audio.is_paused(s));
        if let Some(s) = self.held_music {
            ctx.audio.pause(s);
        }
        crate::menu::menu_enter(self, ctx);
    }

    fn exit(&mut self, ctx: &mut Ctx<G>) {
        self.cancel_export();
        self.queue.clear();
        self.stop(ctx);
        // What this screen started is not heard in play.
        ctx.audio.take_watched();
        ctx.soundtrack.listen(self.was_listening);
        if let Some(s) = self.held_music.take() {
            ctx.audio.resume(s);
        }
    }

    fn cover(&mut self, ctx: &mut Ctx<G>) {
        self.queue.clear();
        self.stop(ctx);
    }

    fn announce(&mut self, ctx: &mut Ctx<G>) {
        let (unlocked, listed) = self.counts(ctx);
        let title = crate::text::end_sentence(&self.core.title);
        let tracks = if listed == 1 { "track" } else { "tracks" };
        ctx.say(format!("{title} {unlocked} of {listed} {tracks} unlocked."));
        let current = self.current_text(ctx);
        ctx.say_queued(current);
    }

    fn update(&mut self, ctx: &mut Ctx<G>, dt: f64) {
        #[cfg(feature = "kira")]
        self.step_export(ctx);
        let Some((_, p)) = self.playing.as_mut() else {
            return;
        };
        if !p.update(&mut ctx.audio, dt) {
            self.playing = None;
            if self.play_next(ctx) {
                let title = self.playing_title(ctx);
                ctx.say_queued(format!("Playing {title}"));
            }
        }
    }

    fn on_escape(&mut self, ctx: &mut Ctx<G>) {
        ctx.play_ui(UiSound::Back);
        if self.cancel_export() {
            ctx.say("Export stopped.");
            return;
        }
        ctx.pop();
    }
}

/// Writing an album of the soundtrack to sound files, for a game that
/// offers it ([`AlbumOptions::export`]).
///
/// A track is played once on a mixer with no device, faster than real
/// time, and its finished mix written as a 16-bit stereo WAV file at
/// [`RATE`]: only the mix, never the stems. The soundtrack menu does this
/// a slice at a time, so the game keeps answering while it works.
#[cfg(feature = "kira")]
pub mod export {
    use std::fs::File;
    use std::io::{self, BufWriter, Seek, SeekFrom, Write};
    use std::path::{Path, PathBuf};

    use hearken_audio::{Audio, ClipSource, KiraBackend};

    use super::Performer;

    /// Frames a second in an exported file.
    pub const RATE: u32 = 48_000;
    /// The longest a track plays when exported: one that would go on longer
    /// (a live piece that never ends) is faded there.
    pub const MAX_S: f64 = 15.0 * 60.0;
    /// Frames mixed at a time.
    const CHUNK: usize = 1024;
    /// Silence kept after a piece ends, so its last fade is whole.
    const TAIL_S: f64 = 1.0;

    /// `text` as a file or folder name on any system: no path separators
    /// or characters Windows refuses, no trailing dots or spaces, at most 80
    /// characters. Empty becomes `"Untitled"`.
    pub fn file_name(text: &str) -> String {
        let cleaned: String = text
            .chars()
            .map(|c| match c {
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => ' ',
                c if c.is_control() => ' ',
                c => c,
            })
            .collect();
        let words: Vec<&str> = cleaned.split_whitespace().collect();
        let mut name: String = words.join(" ").chars().take(80).collect();
        while name.ends_with('.') || name.ends_with(' ') {
            name.pop();
        }
        // Windows refuses these as a name, with or without an extension.
        let stem = name
            .split('.')
            .next()
            .unwrap_or("")
            .trim_end()
            .to_uppercase();
        let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
            || ((stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.len() == 4
                && stem.as_bytes()[3].is_ascii_digit());
        if name.is_empty() || name.starts_with('.') || reserved {
            name = format!("Untitled {name}").trim().to_string();
        }
        name
    }

    /// Where an album goes: `<base>/<game>/<album>`, or `<base>/<game>` for
    /// a game whose tracks have no album.
    pub fn folder(base: &Path, game: &str, album: &str) -> PathBuf {
        let dir = base.join(file_name(game));
        if album.is_empty() {
            dir
        } else {
            dir.join(file_name(album))
        }
    }

    /// The folder exports go under: the player's Music folder, else
    /// `fallback` (the game's data folder).
    pub fn base(fallback: &Path) -> PathBuf {
        dirs::audio_dir().unwrap_or_else(|| fallback.join("Soundtrack"))
    }

    /// One track being written.
    pub struct Render {
        audio: Audio,
        performer: Box<dyn Performer>,
        out: Wav,
        frames: usize,
        tail: Option<usize>,
        path: PathBuf,
    }

    impl Render {
        /// Starts playing `performer` on a mixer of its own, loading clips
        /// from `clips`, to be written to `path`.
        pub fn start(
            mut performer: Box<dyn Performer>,
            clips: Box<dyn ClipSource>,
            path: PathBuf,
        ) -> io::Result<Render> {
            let backend = KiraBackend::offline(RATE).map_err(|e| io::Error::other(e.0))?;
            let mut audio = Audio::new(Box::new(backend), clips);
            performer.start(&mut audio);
            if !performer.update(&mut audio, 0.0) {
                performer.stop(&mut audio);
                return Err(io::Error::other("nothing of the track would play"));
            }
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let out = Wav::create(&part_of(&path))?;
            Ok(Render {
                audio,
                performer,
                out,
                frames: 0,
                tail: None,
                path,
            })
        }

        /// Seconds written so far.
        pub fn seconds(&self) -> f64 {
            self.frames as f64 / RATE as f64
        }

        /// Writes about `frames` more frames: true once the track has ended
        /// and its file is complete.
        pub fn step(&mut self, frames: usize) -> io::Result<bool> {
            let mut done = 0;
            while done < frames.max(1) {
                let dt = CHUNK as f64 / RATE as f64;
                if self.tail.is_none() {
                    let alive = self.performer.update(&mut self.audio, dt);
                    if !alive || self.seconds() >= MAX_S {
                        if alive {
                            self.performer.stop(&mut self.audio);
                        }
                        self.tail = Some((TAIL_S * RATE as f64) as usize);
                    }
                }
                self.audio.update(&|_| false);
                let mix = self
                    .audio
                    .render(CHUNK)
                    .ok_or_else(|| io::Error::other("the offline mixer is gone"))?;
                if mix.is_empty() {
                    return Err(io::Error::other("the offline mixer stopped"));
                }
                self.out.write(&mix)?;
                self.frames += mix.len();
                done += mix.len();
                if let Some(left) = self.tail.as_mut() {
                    *left = left.saturating_sub(mix.len());
                    if *left == 0 {
                        self.out.finish()?;
                        std::fs::rename(part_of(&self.path), &self.path)?;
                        return Ok(true);
                    }
                }
            }
            Ok(false)
        }

        /// Where the finished file goes.
        pub fn path(&self) -> &Path {
            &self.path
        }

        /// Stops, and removes the unfinished file.
        pub fn cancel(mut self) {
            self.performer.stop(&mut self.audio);
            let _ = std::fs::remove_file(part_of(&self.path));
        }
    }

    impl Drop for Render {
        fn drop(&mut self) {
            self.audio.shutdown();
        }
    }

    /// Writes one whole play of `performer` to `path` at once: for tests
    /// and tools.
    pub fn render_to(
        performer: Box<dyn Performer>,
        clips: Box<dyn ClipSource>,
        path: PathBuf,
    ) -> io::Result<PathBuf> {
        let mut r = Render::start(performer, clips, path)?;
        while !r.step(RATE as usize)? {}
        Ok(r.path.clone())
    }

    /// The file a render writes until it is complete.
    fn part_of(path: &Path) -> PathBuf {
        let mut name = path.file_name().unwrap_or_default().to_os_string();
        name.push(".part");
        path.with_file_name(name)
    }

    /// A 16-bit stereo WAV file, its sizes filled in at the end.
    struct Wav {
        file: BufWriter<File>,
        bytes: u32,
    }

    impl Wav {
        fn create(path: &Path) -> io::Result<Wav> {
            let mut file = BufWriter::new(File::create(path)?);
            let rate = RATE;
            file.write_all(b"RIFF\0\0\0\0WAVEfmt ")?;
            file.write_all(&16u32.to_le_bytes())?;
            file.write_all(&1u16.to_le_bytes())?; // PCM
            file.write_all(&2u16.to_le_bytes())?; // stereo
            file.write_all(&rate.to_le_bytes())?;
            file.write_all(&(rate * 4).to_le_bytes())?;
            file.write_all(&4u16.to_le_bytes())?;
            file.write_all(&16u16.to_le_bytes())?;
            file.write_all(b"data\0\0\0\0")?;
            Ok(Wav { file, bytes: 0 })
        }

        fn write(&mut self, frames: &[[f32; 2]]) -> io::Result<()> {
            let mut buf = Vec::with_capacity(frames.len() * 4);
            for [l, r] in frames {
                for s in [l, r] {
                    let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                    buf.extend_from_slice(&v.to_le_bytes());
                }
            }
            self.bytes = self.bytes.saturating_add(buf.len() as u32);
            self.file.write_all(&buf)
        }

        fn finish(&mut self) -> io::Result<()> {
            self.file.flush()?;
            let file = self.file.get_mut();
            file.seek(SeekFrom::Start(4))?;
            file.write_all(&(36 + self.bytes).to_le_bytes())?;
            file.seek(SeekFrom::Start(40))?;
            file.write_all(&self.bytes.to_le_bytes())?;
            file.sync_all()
        }
    }
}
