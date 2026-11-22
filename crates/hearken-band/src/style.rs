//! A style sheet: what a game's band sounds like. The composer writes a new
//! song inside it each time; the style stays the game's own.
//!
//! A style can be built in code or read from JSON (it is `serde`), so a
//! game can keep its sheets next to its sound specs.

use serde::{Deserialize, Serialize};

use crate::theory::Scale;

/// Which layer of the music a part belongs to: how it follows play.
///
/// These are the six layers a dynamic score has always had: `Base` always
/// plays, `Pulse`, `Melody` and `Lift` join one after another as play
/// heats up, and `Hurry` and `Danger` follow flags of their own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Base,
    Pulse,
    Melody,
    Lift,
    Hurry,
    Danger,
}

impl Role {
    pub const ALL: [Role; 6] = [
        Role::Base,
        Role::Pulse,
        Role::Melody,
        Role::Lift,
        Role::Hurry,
        Role::Danger,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Role::Base => "base",
            Role::Pulse => "pulse",
            Role::Melody => "melody",
            Role::Lift => "lift",
            Role::Hurry => "hurry",
            Role::Danger => "danger",
        }
    }
}

/// What a part plays.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Play {
    /// A bass line, in one of the named patterns (see [`BASS_PATTERNS`]).
    Bass { patterns: Vec<String> },
    /// The chord held through each bar.
    Pad,
    /// The chord struck in a rhythm: off beats, pushes, a comp.
    Comp,
    /// The chord broken into a running figure.
    Arp,
    /// The song's tune.
    Lead,
    /// A second line under the tune: long notes in thirds and sixths.
    Counter,
    /// The drums, in one of the named grooves (see [`GROOVES`]).
    Drums { grooves: Vec<String> },
    /// The hats twice as fast: time running out.
    Rush,
    /// A low held note and a tense one over it.
    Drone,
}

/// The bass patterns a [`Play::Bass`] can name.
pub const BASS_PATTERNS: &[&str] = &[
    "whole",
    "root_fifth",
    "pulse",
    "rock",
    "walk",
    "synth",
    "dub",
];

/// The grooves a [`Play::Drums`] can name.
pub const GROOVES: &[&str] = &[
    "four", "backbeat", "half", "break", "train", "brush", "march", "bossa",
];

/// One part of the band.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Part {
    pub role: Role,
    pub play: Play,
    /// Instruments it may be played on, one chosen per song: names from the
    /// game's [`crate::Bank`]. Drums name a kit.
    pub instruments: Vec<String>,
    /// `-1` left to `1` right.
    #[serde(default)]
    pub pan: f32,
    /// Its level at full, `0..=1`, before the note's own.
    #[serde(default = "one")]
    pub volume: f32,
    /// Lowest and highest MIDI notes it plays (the instrument's own range
    /// narrows it further). Ignored for drums.
    #[serde(default)]
    pub range: Option<(u8, u8)>,
    /// How often the part is in a song at all, `0..=1`; most are always in.
    #[serde(default = "one")]
    pub chance: f32,
}

fn one() -> f32 {
    1.0
}

impl Part {
    pub fn new(role: Role, play: Play, instruments: &[&str]) -> Part {
        Part {
            role,
            play,
            instruments: instruments.iter().map(|s| s.to_string()).collect(),
            pan: 0.0,
            volume: 1.0,
            range: None,
            chance: 1.0,
        }
    }

    pub fn pan(mut self, pan: f32) -> Part {
        self.pan = pan;
        self
    }

    pub fn volume(mut self, volume: f32) -> Part {
        self.volume = volume;
        self
    }

    pub fn range(mut self, low: u8, high: u8) -> Part {
        self.range = Some((low, high));
        self
    }

    pub fn chance(mut self, chance: f32) -> Part {
        self.chance = chance;
        self
    }
}

/// A game's style sheet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Style {
    /// A short name, for logs and saved songs: `"neon-rail/run"`.
    pub name: String,
    /// Slowest and fastest tempo, beats a minute; each song picks one.
    pub tempo: (f32, f32),
    /// Scales a song may be in.
    pub scales: Vec<Scale>,
    /// Keys a song may be in, as pitch classes (0 C to 11 B); empty is any.
    #[serde(default)]
    pub keys: Vec<u8>,
    /// How far off beats lean late, `0` straight to `1` a full triplet
    /// swing. A song picks between the two numbers.
    #[serde(default)]
    pub swing: (f32, f32),
    /// How busy the tune is, `0` long notes to `1` running ones. A song
    /// picks between the two numbers.
    #[serde(default = "middling")]
    pub busy: (f32, f32),
    /// How many bars a phrase is: 4 or 8.
    #[serde(default = "four")]
    pub phrase_bars: u8,
    /// How much hurry speeds the song up, as a fraction: `0.06` is 6 percent.
    #[serde(default = "hurry")]
    pub hurry_speedup: f32,
    /// The whole band's level, `0..=1`, to sit the music at the game's
    /// loudness.
    #[serde(default = "one")]
    pub volume: f32,
    pub parts: Vec<Part>,
}

fn middling() -> (f32, f32) {
    (0.35, 0.65)
}

fn four() -> u8 {
    4
}

fn hurry() -> f32 {
    0.06
}

impl Style {
    pub fn new(name: &str, tempo: (f32, f32), scales: &[Scale]) -> Style {
        Style {
            name: name.to_string(),
            tempo,
            scales: scales.to_vec(),
            keys: Vec::new(),
            swing: (0.0, 0.0),
            busy: middling(),
            phrase_bars: 4,
            hurry_speedup: hurry(),
            volume: 1.0,
            parts: Vec::new(),
        }
    }

    pub fn part(mut self, part: Part) -> Style {
        self.parts.push(part);
        self
    }

    pub fn keys(mut self, keys: &[u8]) -> Style {
        self.keys = keys.to_vec();
        self
    }

    pub fn swing(mut self, lo: f32, hi: f32) -> Style {
        self.swing = (lo, hi);
        self
    }

    pub fn busy(mut self, lo: f32, hi: f32) -> Style {
        self.busy = (lo, hi);
        self
    }

    pub fn phrase_bars(mut self, bars: u8) -> Style {
        self.phrase_bars = bars;
        self
    }

    /// The same style at another tempo range and name: a mode of a game
    /// with its own pace.
    pub fn retimed(&self, name: &str, tempo: (f32, f32)) -> Style {
        Style {
            name: name.to_string(),
            tempo,
            ..self.clone()
        }
    }

    /// Everything wrong with the sheet against `bank`, empty when it is
    /// sound: an unknown instrument, pattern or groove, or a range with no
    /// notes. A game's tests check its sheets with this.
    pub fn problems(&self, bank: &crate::Bank) -> Vec<String> {
        let mut out = Vec::new();
        if !(self.tempo.0 > 20.0 && self.tempo.1 >= self.tempo.0 && self.tempo.1 < 300.0) {
            out.push(format!(
                "{}: tempo {:?} is not a range of beats a minute",
                self.name, self.tempo
            ));
        }
        if self.scales.is_empty() {
            out.push(format!("{}: no scales", self.name));
        }
        if !matches!(self.phrase_bars, 4 | 8) {
            out.push(format!("{}: phrase_bars must be 4 or 8", self.name));
        }
        for (i, part) in self.parts.iter().enumerate() {
            let at = format!("{} part {i} ({})", self.name, part.role.name());
            if part.instruments.is_empty() {
                out.push(format!("{at}: no instruments"));
            }
            let drums = matches!(part.play, Play::Drums { .. } | Play::Rush);
            for name in &part.instruments {
                if drums {
                    if bank.kit(name).is_none() {
                        out.push(format!("{at}: no kit {name:?} in the bank"));
                    }
                } else {
                    match bank.instrument(name) {
                        None => out.push(format!("{at}: no instrument {name:?} in the bank")),
                        Some(inst) => {
                            let (lo, hi) = part.range.unwrap_or((0, 127));
                            if lo.max(inst.low) > hi.min(inst.high) {
                                out.push(format!("{at}: {name} cannot play in {lo}..{hi}"));
                            }
                        }
                    }
                }
            }
            match &part.play {
                Play::Bass { patterns } => {
                    for p in patterns {
                        if !BASS_PATTERNS.contains(&p.as_str()) {
                            out.push(format!("{at}: no bass pattern {p:?}"));
                        }
                    }
                    if patterns.is_empty() {
                        out.push(format!("{at}: no bass patterns"));
                    }
                }
                Play::Drums { grooves } => {
                    for g in grooves {
                        if !GROOVES.contains(&g.as_str()) {
                            out.push(format!("{at}: no groove {g:?}"));
                        }
                    }
                    if grooves.is_empty() {
                        out.push(format!("{at}: no grooves"));
                    }
                }
                _ => {}
            }
        }
        out
    }
}

/// One sampled instrument: notes recorded every few semitones, each a
/// clip under `<prefix>/<name>/<midi>`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Instrument {
    pub name: String,
    /// Lowest and highest MIDI note it may play.
    pub low: u8,
    pub high: u8,
    /// The MIDI notes recorded, lowest first.
    pub samples: Vec<u8>,
    /// How long each recorded note lasts before its release, in seconds.
    /// A note held longer is held only this long.
    pub length_s: f32,
    /// How long a note takes to fade when cut short, in milliseconds.
    #[serde(default = "release")]
    pub release_ms: u32,
}

fn release() -> u32 {
    90
}

impl Instrument {
    /// The recorded note nearest `midi`, and the pitch to play it at.
    pub fn sample_for(&self, midi: i32) -> Option<(u8, f32)> {
        let best = self
            .samples
            .iter()
            .min_by_key(|s| ((**s as i32 - midi).abs(), **s as i32 > midi))?;
        let pitch = 2f32.powf((midi - *best as i32) as f32 / 12.0);
        Some((*best, pitch))
    }
}

/// A drum kit: which recorded hit plays each part of a groove. Hits are
/// clips under `<prefix>/drums/<hit>`, shared by every kit that uses them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Kit {
    pub name: String,
    /// The hits it has, by what they do in a groove: `kick`, `snare`,
    /// `hat`, `open`, `crash`, `perc`, `rim`. A groove leaves out what the
    /// kit lacks.
    pub hits: Vec<(String, String)>,
}

impl Kit {
    /// The clip name of the hit that does `what`.
    pub fn hit(&self, what: &str) -> Option<&str> {
        self.hits
            .iter()
            .find(|(w, _)| w == what)
            .map(|(_, h)| h.as_str())
    }
}

/// Every instrument and kit a game has recorded for its bands.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Bank {
    /// The clip key every sample sits under: `"parlor/band"`.
    pub prefix: String,
    pub instruments: Vec<Instrument>,
    pub kits: Vec<Kit>,
}

impl Bank {
    pub fn new(prefix: &str) -> Bank {
        Bank {
            prefix: prefix.to_string(),
            ..Bank::default()
        }
    }

    pub fn instrument(&self, name: &str) -> Option<&Instrument> {
        self.instruments.iter().find(|i| i.name == name)
    }

    pub fn kit(&self, name: &str) -> Option<&Kit> {
        self.kits.iter().find(|k| k.name == name)
    }

    /// The clip key of instrument `name`'s recorded note `midi`.
    pub fn note_key(&self, name: &str, midi: u8) -> String {
        format!("{}/{name}/{midi}", self.prefix)
    }

    /// The clip key of drum hit `hit`.
    pub fn hit_key(&self, hit: &str) -> String {
        format!("{}/drums/{hit}", self.prefix)
    }

    /// Every clip key the bank names, for a game's check that each is on
    /// disk.
    pub fn keys(&self) -> Vec<String> {
        let mut out = Vec::new();
        for i in &self.instruments {
            for s in &i.samples {
                out.push(self.note_key(&i.name, *s));
            }
        }
        for k in &self.kits {
            for (_, h) in &k.hits {
                out.push(self.hit_key(h));
            }
        }
        out.sort();
        out.dedup();
        out
    }
}
