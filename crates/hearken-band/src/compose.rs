//! The composer: from a seed and a style, a whole song.
//!
//! A [`Song`] is chosen once: key, scale, tempo, swing, how busy the tune
//! is, and which instrument, bass pattern and groove each part takes. Its
//! sections are written on demand ([`Song::section`]), the same for the
//! same seed every time: an intro, a tune (A), a second tune (B), the first
//! tune again, a break, and round. Each time round the form, B is new, and
//! every second time round A is new too, so a long session keeps changing
//! while its key, tempo and instruments stay the song's own.

use serde::{Deserialize, Serialize};

use crate::rng::Rng;
use crate::style::{Part, Play, Role, Style};
use crate::theory::{ladder, pitch_name, snap, Chord, Scale};
use crate::Bank;

/// Beats in a bar. Every song is in four.
pub const BEATS: f32 = 4.0;

/// A pitched note, in beats from the start of its bar.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub beat: f32,
    pub len: f32,
    pub midi: i32,
    pub vel: f32,
}

/// What a part does in a bar.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Note(Note),
    /// A drum hit: what it does in the groove (`kick`, `hat`...), and the
    /// layer it follows, which for extras such as an open hat or a crash is
    /// [`Role::Lift`] rather than the drums' own.
    Hit {
        beat: f32,
        what: &'static str,
        vel: f32,
        role: Role,
    },
}

impl Event {
    pub fn beat(&self) -> f32 {
        match self {
            Event::Note(n) => n.beat,
            Event::Hit { beat, .. } => *beat,
        }
    }
}

/// The kinds of section in the form, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionKind {
    Intro,
    A,
    B,
    /// A again, ending on the tonic.
    AReturn,
    Break,
}

impl SectionKind {
    pub fn name(self) -> &'static str {
        match self {
            SectionKind::Intro => "intro",
            SectionKind::A => "A",
            SectionKind::B => "B",
            SectionKind::AReturn => "A again",
            SectionKind::Break => "break",
        }
    }
}

/// The form a song goes round.
pub const FORM: [SectionKind; 5] = [
    SectionKind::Intro,
    SectionKind::A,
    SectionKind::B,
    SectionKind::AReturn,
    SectionKind::Break,
];

/// One section, written out.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub index: usize,
    pub kind: SectionKind,
    /// One chord a bar.
    pub chords: Vec<Chord>,
    /// The tune, bar by bar.
    pub tune: Vec<Vec<Note>>,
    /// The counter line, bar by bar.
    pub counter: Vec<Vec<Note>>,
}

impl Section {
    pub fn bars(&self) -> usize {
        self.chords.len()
    }
}

/// What the song chose for one part of the style.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Chosen {
    /// The part's index in the style.
    pub part: usize,
    pub instrument: String,
    /// A bass pattern or a groove, for parts that name one.
    #[serde(default)]
    pub pattern: Option<String>,
    /// Lowest and highest note it plays here.
    pub low: i32,
    pub high: i32,
}

/// A song: everything chosen once, from which every bar is written.
#[derive(Clone, Debug, PartialEq)]
pub struct Song {
    pub seed: u64,
    pub style: Style,
    /// The key's pitch class, 0 C to 11 B.
    pub key: u8,
    pub scale: Scale,
    pub bpm: f32,
    pub swing: f32,
    pub busy: f32,
    /// How often a chord takes its seventh.
    pub sevenths: f32,
    /// The parts in this song (a part with a chance may sit out).
    pub parts: Vec<Chosen>,
    /// The comp's rhythm, beats in a bar.
    comp: Vec<(f32, f32)>,
}

impl Song {
    /// Writes a new song in `style` from `seed`, with instruments `bank`
    /// has. A part whose every instrument is missing from the bank sits out.
    pub fn compose(style: &Style, bank: &Bank, seed: u64) -> Song {
        // The style's name goes into every choice, so one seed (a date)
        // writes a different song in each style.
        let seed = seed ^ crate::rng::hash(&style.name);
        let mut r = Rng::fork(seed, 1);
        let key = if style.keys.is_empty() {
            r.below(12) as u8
        } else {
            *r.pick(&style.keys).unwrap_or(&0) % 12
        };
        let scale = *r.pick(&style.scales).unwrap_or(&Scale::Major);
        let bpm = r
            .range(style.tempo.0, style.tempo.1.max(style.tempo.0))
            .round();
        let swing = r.range(style.swing.0, style.swing.1.max(style.swing.0));
        let busy = r
            .range(style.busy.0, style.busy.1.max(style.busy.0))
            .clamp(0.0, 1.0);
        let sevenths = r.range(0.0, 0.45);
        let mut parts = Vec::new();
        for (i, part) in style.parts.iter().enumerate() {
            let mut pr = Rng::fork(seed, 100 + i as u64);
            if part.chance < 1.0 && !pr.chance(part.chance) {
                continue;
            }
            if let Some(chosen) = choose(part, i, bank, &mut pr) {
                parts.push(chosen);
            }
        }
        let comps: [&[(f32, f32)]; 4] = [
            &[(0.5, 0.4), (1.5, 0.4), (2.5, 0.4), (3.5, 0.4)],
            &[(0.0, 0.9), (1.5, 0.5), (3.0, 0.9)],
            &[(0.0, 1.4), (1.5, 2.4)],
            &[(1.0, 0.45), (3.0, 0.45)],
        ];
        let comp = comps[r.below(comps.len())].to_vec();
        Song {
            seed,
            style: style.clone(),
            key,
            scale,
            bpm,
            swing,
            busy,
            sevenths,
            parts,
            comp,
        }
    }

    /// One line that says what the song is, for logs and the agent:
    /// `"D dorian, 118 a minute: bass fm_bass (rock), lead square_lead"`.
    pub fn describe(&self) -> String {
        let parts: Vec<String> = self
            .parts
            .iter()
            .map(|c| {
                let part = &self.style.parts[c.part];
                match &c.pattern {
                    Some(p) => format!("{} {} ({p})", part.role.name(), c.instrument),
                    None => format!("{} {}", part.role.name(), c.instrument),
                }
            })
            .collect();
        format!(
            "{} {}, {} a minute: {}",
            pitch_name(self.key),
            self.scale.name(),
            self.bpm,
            parts.join(", ")
        )
    }

    /// The style's part behind chosen part `c`.
    pub fn part(&self, c: &Chosen) -> &Part {
        &self.style.parts[c.part]
    }

    /// Section `index` of the song (counting from 0 for ever round the
    /// form), written out.
    pub fn section(&self, index: usize) -> Section {
        let kind = FORM[index % FORM.len()];
        let round = (index / FORM.len()) as u64;
        let phrase = self.style.phrase_bars.max(1) as usize;
        // A's material is new every second round, B's every round.
        let a_seed = round / 2;
        let (chords, tune_seed, bars) = match kind {
            SectionKind::Intro => (self.progression(10 + a_seed, phrase, false), None, phrase),
            SectionKind::A => {
                let p = self.progression(10 + a_seed, phrase, false);
                (twice(&p), Some(20 + a_seed), phrase * 2)
            }
            SectionKind::AReturn => {
                let p = self.progression(10 + a_seed, phrase, false);
                let mut c = p.clone();
                c.extend(self.progression(10 + a_seed, phrase, true));
                (c, Some(20 + a_seed), phrase * 2)
            }
            SectionKind::B => {
                let p = self.progression(1000 + round, phrase, false);
                (twice(&p), Some(2000 + round), phrase * 2)
            }
            SectionKind::Break => (self.progression(3000 + round, 4, false), None, 4),
        };
        let chords: Vec<Chord> = chords.into_iter().take(bars).collect();
        let tune = match tune_seed {
            Some(s) => self.tune(&chords, s, kind),
            None => vec![Vec::new(); chords.len()],
        };
        let counter = self.counter(&chords, &tune);
        Section {
            index,
            kind,
            chords,
            tune,
            counter,
        }
    }

    /// A phrase of `bars` chords from `salt`. A phrase starts home and ends
    /// on a chord that leads back; `home` ends it on the tonic instead.
    fn progression(&self, salt: u64, bars: usize, home: bool) -> Vec<Chord> {
        let mut r = Rng::fork(self.seed, salt.wrapping_mul(7919));
        let minor = self.scale.is_minor();
        let mut degree: u8 = if r.chance(0.8) {
            0
        } else if minor {
            5
        } else {
            3
        };
        let mut out = Vec::with_capacity(bars);
        for bar in 0..bars {
            if bar > 0 {
                degree = next_degree(degree, minor, &mut r);
            }
            if bar + 1 == bars {
                degree = if home {
                    0
                } else {
                    let turns: &[u8] = if minor { &[4, 6, 3] } else { &[4, 3, 1] };
                    turns[r.weighted(&[3.0, 2.0, 1.0])]
                };
            }
            out.push(Chord {
                degree,
                seventh: r.chance(self.sevenths),
            });
        }
        out
    }

    /// The tune over `chords`: a one-bar idea, the idea again fitted to the
    /// next chord, something new, and a cadence, phrase after phrase.
    fn tune(&self, chords: &[Chord], salt: u64, kind: SectionKind) -> Vec<Vec<Note>> {
        let Some(lead) = self.chosen(|p| matches!(p.play, Play::Lead)) else {
            return vec![Vec::new(); chords.len()];
        };
        let mut r = Rng::fork(self.seed, salt.wrapping_mul(104_729));
        let ladder = ladder(self.key, &self.scale.tune(), lead.low, lead.high);
        if ladder.len() < 3 {
            return vec![Vec::new(); chords.len()];
        }
        let idea = rhythm(self.busy, &mut r);
        let idea_steps: Vec<i32> = idea.iter().map(|_| step(&mut r)).collect();
        let answer = rhythm(self.busy, &mut r);
        let answer_steps: Vec<i32> = answer.iter().map(|_| step(&mut r)).collect();
        let phrase = self.style.phrase_bars.max(1) as usize;
        let mut at = ladder.len() / 2;
        let mut out = Vec::with_capacity(chords.len());
        for (bar, chord) in chords.iter().enumerate() {
            let tones = chord.tones(self.scale);
            let in_phrase = bar % phrase;
            let last_bar = bar + 1 == chords.len();
            let (cells, steps): (Vec<(f32, bool)>, Vec<i32>) = if in_phrase + 1 == phrase {
                // The cadence: a step or two, then a long note on the chord.
                (
                    vec![(1.0, false), (1.0, false), (2.0, false)],
                    vec![-1, -1, 0],
                )
            } else if in_phrase.is_multiple_of(2) && in_phrase != 2 {
                (idea.clone(), idea_steps.clone())
            } else if in_phrase == 2 || in_phrase == 6 {
                (answer.clone(), answer_steps.clone())
            } else {
                // The idea again, its first step turned toward the chord.
                (idea.clone(), idea_steps.iter().map(|s| -s).collect())
            };
            let mut beat = 0.0;
            let mut notes = Vec::new();
            for (i, ((len, rest), s)) in cells.iter().zip(steps.iter()).enumerate() {
                at = (at as i32 + s).clamp(0, ladder.len() as i32 - 1) as usize;
                let mut midi = ladder[at];
                let strong = beat == 0.0 || (beat == 2.0 && *len >= 1.0) || *len >= 2.0;
                if strong {
                    let target = if last_bar && i + 1 == cells.len() && kind == SectionKind::AReturn
                    {
                        vec![0]
                    } else {
                        tones.clone()
                    };
                    let chord_pcs: Vec<u8> = target.iter().map(|t| (self.key + t) % 12).collect();
                    let snapped = snap(midi, 0, &chord_pcs);
                    if snapped >= lead.low && snapped <= lead.high {
                        midi = snapped;
                        if let Some(p) = ladder.iter().position(|n| *n == midi) {
                            at = p;
                        }
                    }
                }
                if !rest {
                    let vel = if strong {
                        0.85
                    } else {
                        0.68 + r.range(-0.06, 0.06)
                    };
                    notes.push(Note {
                        beat,
                        len: *len * 0.92,
                        midi,
                        vel,
                    });
                }
                beat += len;
            }
            // A tune that strays to an edge comes back toward the middle.
            if at < 2 || at + 3 > ladder.len() {
                at = ladder.len() / 2;
            }
            out.push(notes);
        }
        out
    }

    /// A counter line under the tune: a long chord tone a third or a sixth
    /// below where the tune starts each half bar.
    fn counter(&self, chords: &[Chord], tune: &[Vec<Note>]) -> Vec<Vec<Note>> {
        let Some(c) = self.chosen(|p| matches!(p.play, Play::Counter)) else {
            return vec![Vec::new(); chords.len()];
        };
        let mut out = Vec::with_capacity(chords.len());
        let mut prev = (c.low + c.high) / 2;
        for (bar, chord) in chords.iter().enumerate() {
            let pcs: Vec<u8> = chord
                .tones(self.scale)
                .iter()
                .map(|t| (self.key + t) % 12)
                .collect();
            let mut notes = Vec::new();
            for half in [0.0f32, 2.0] {
                let above = tune
                    .get(bar)
                    .and_then(|t| t.iter().find(|n| n.beat >= half && n.beat < half + 2.0))
                    .map(|n| n.midi - 4)
                    .unwrap_or(prev);
                let mut midi = snap(above, 0, &pcs);
                while midi > c.high {
                    midi -= 12;
                }
                while midi < c.low {
                    midi += 12;
                }
                prev = midi;
                notes.push(Note {
                    beat: half,
                    len: 1.9,
                    midi,
                    vel: 0.6,
                });
            }
            out.push(notes);
        }
        out
    }

    fn chosen(&self, which: impl Fn(&Part) -> bool) -> Option<&Chosen> {
        self.parts.iter().find(|c| which(&self.style.parts[c.part]))
    }

    /// Every event in bar `bar` of `section`, for each chosen part (by its
    /// index in [`Song::parts`]), with `hurry` and `danger` as play has them
    /// so their parts can be written only when wanted.
    pub fn bar(&self, section: &Section, bar: usize) -> Vec<(usize, Event)> {
        let mut out = Vec::new();
        let Some(chord) = section.chords.get(bar).copied() else {
            return out;
        };
        let next = section.chords.get(bar + 1).copied().unwrap_or(Chord::on(0));
        let phrase = self.style.phrase_bars.max(1) as usize;
        let last_of_phrase = bar % phrase + 1 == phrase;
        let tones = chord.tones(self.scale);
        let mut r = Rng::fork(self.seed, 5_000 + (section.index as u64) * 64 + bar as u64);
        for (pi, c) in self.parts.iter().enumerate() {
            let part = self.part(c);
            let notes: Vec<Note> = match &part.play {
                Play::Bass { .. } => bass(
                    c.pattern.as_deref().unwrap_or("whole"),
                    self.key,
                    self.scale,
                    chord,
                    next,
                    c.low,
                    c.high,
                ),
                Play::Pad => voicing(self.key, &tones, c.low, c.high)
                    .into_iter()
                    .map(|midi| Note {
                        beat: 0.0,
                        len: BEATS,
                        midi,
                        vel: 0.55,
                    })
                    .collect(),
                Play::Comp => {
                    let chord_notes = voicing(self.key, &tones, c.low, c.high);
                    self.comp
                        .iter()
                        .flat_map(|(beat, len)| {
                            chord_notes.iter().map(move |midi| Note {
                                beat: *beat,
                                len: *len,
                                midi: *midi,
                                vel: 0.6,
                            })
                        })
                        .collect()
                }
                Play::Arp => {
                    let base = voicing(self.key, &tones, c.low, c.high);
                    let mut run: Vec<i32> = base.clone();
                    run.extend(base.iter().map(|n| n + 12).filter(|n| *n <= c.high));
                    let down = r.chance(0.4);
                    if down {
                        run.reverse();
                    }
                    (0..8)
                        .map(|i| Note {
                            beat: i as f32 * 0.5,
                            len: 0.45,
                            midi: run[i % run.len().max(1)],
                            vel: if i % 2 == 0 { 0.62 } else { 0.5 },
                        })
                        .collect()
                }
                Play::Lead => section.tune.get(bar).cloned().unwrap_or_default(),
                Play::Counter => section.counter.get(bar).cloned().unwrap_or_default(),
                Play::Drone => {
                    let root = fit((self.key + chord.root(self.scale)) as i32, c.low, c.high);
                    let mut v = vec![Note {
                        beat: 0.0,
                        len: BEATS,
                        midi: fit(self.key as i32, c.low, c.high).min(root),
                        vel: 0.6,
                    }];
                    if bar % 2 == 1 {
                        let tense = if r.chance(0.5) { 1 } else { 6 };
                        v.push(Note {
                            beat: 2.0,
                            len: 2.0,
                            midi: fit(self.key as i32 + tense, c.low + 6, c.high),
                            vel: 0.42,
                        });
                    }
                    v
                }
                Play::Drums { .. } => {
                    for (beat, what, vel, role) in groove(
                        c.pattern.as_deref().unwrap_or("backbeat"),
                        last_of_phrase && r.chance(0.65),
                        bar == 0
                            && matches!(
                                section.kind,
                                SectionKind::A | SectionKind::B | SectionKind::AReturn
                            ),
                        part.role,
                    ) {
                        out.push((
                            pi,
                            Event::Hit {
                                beat,
                                what,
                                vel,
                                role,
                            },
                        ));
                    }
                    continue;
                }
                Play::Rush => {
                    for i in 0..16 {
                        out.push((
                            pi,
                            Event::Hit {
                                beat: i as f32 * 0.25,
                                what: "hat",
                                vel: if i % 4 == 0 { 0.6 } else { 0.38 },
                                role: part.role,
                            },
                        ));
                    }
                    continue;
                }
            };
            out.extend(notes.into_iter().map(|n| (pi, Event::Note(n))));
        }
        out
    }
}

fn twice(p: &[Chord]) -> Vec<Chord> {
    let mut v = p.to_vec();
    v.extend_from_slice(p);
    v
}

fn choose(part: &Part, index: usize, bank: &Bank, r: &mut Rng) -> Option<Chosen> {
    let drums = matches!(part.play, Play::Drums { .. } | Play::Rush);
    let usable: Vec<&String> = part
        .instruments
        .iter()
        .filter(|n| {
            if drums {
                bank.kit(n).is_some()
            } else {
                bank.instrument(n).is_some()
            }
        })
        .collect();
    let name = (*r.pick(&usable)?).clone();
    let (low, high) = if drums {
        (0, 127)
    } else {
        let inst = bank.instrument(&name)?;
        let (lo, hi) = part.range.unwrap_or((0, 127));
        let low = lo.max(inst.low) as i32;
        let high = hi.min(inst.high) as i32;
        if low > high {
            return None;
        }
        (low, high)
    };
    let pattern = match &part.play {
        Play::Bass { patterns } => r.pick(patterns).cloned(),
        Play::Drums { grooves } => r.pick(grooves).cloned(),
        _ => None,
    };
    Some(Chosen {
        part: index,
        instrument: name,
        pattern,
        low,
        high,
    })
}

/// Which chord follows `from`, by how chords usually move in a major or a
/// minor key.
fn next_degree(from: u8, minor: bool, r: &mut Rng) -> u8 {
    let table: &[(u8, f32)] = match (minor, from) {
        (false, 0) => &[(3, 3.0), (4, 2.0), (5, 3.0), (1, 1.5), (2, 0.5)],
        (false, 1) => &[(4, 4.0), (3, 1.0)],
        (false, 2) => &[(5, 3.0), (3, 2.0)],
        (false, 3) => &[(4, 3.0), (0, 2.0), (1, 1.5), (5, 1.0)],
        (false, 4) => &[(0, 4.0), (5, 2.0), (3, 1.0)],
        (false, 5) => &[(3, 3.0), (1, 2.0), (4, 1.5), (2, 0.5)],
        (true, 0) => &[(3, 3.0), (5, 3.0), (6, 2.0), (4, 1.5), (2, 1.0)],
        (true, 2) => &[(5, 2.0), (6, 2.0), (3, 2.0)],
        (true, 3) => &[(0, 2.0), (4, 2.0), (6, 1.5), (5, 1.0)],
        (true, 4) => &[(0, 4.0), (5, 1.0)],
        (true, 5) => &[(6, 3.0), (3, 2.0), (2, 1.5), (4, 1.0)],
        (true, 6) => &[(0, 3.0), (2, 2.0), (5, 1.0)],
        _ => &[(0, 2.0), (4, 1.0)],
    };
    let weights: Vec<f32> = table.iter().map(|(_, w)| *w).collect();
    table[r.weighted(&weights)].0
}

/// A bar's rhythm: note lengths in beats, each perhaps a rest, adding up
/// to four.
fn rhythm(busy: f32, r: &mut Rng) -> Vec<(f32, bool)> {
    const SPARSE: &[&[f32]] = &[
        &[2.0, 2.0],
        &[3.0, 1.0],
        &[1.0, 3.0],
        &[2.0, 1.0, 1.0],
        &[1.5, 2.5],
    ];
    const MIDDLE: &[&[f32]] = &[
        &[1.0, 1.0, 2.0],
        &[1.0, 0.5, 0.5, 2.0],
        &[1.5, 0.5, 2.0],
        &[0.5, 0.5, 1.0, 2.0],
        &[1.0, 1.0, 1.0, 1.0],
        &[2.0, 0.5, 0.5, 1.0],
        &[1.5, 1.5, 1.0],
    ];
    const BUSY: &[&[f32]] = &[
        &[0.5, 0.5, 0.5, 0.5, 1.0, 1.0],
        &[0.5, 0.5, 1.0, 0.5, 0.5, 1.0],
        &[1.0, 0.5, 0.5, 0.5, 0.5, 1.0],
        &[0.25, 0.25, 0.5, 1.0, 1.0, 1.0],
        &[0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 1.0],
        &[0.75, 0.25, 0.5, 0.5, 1.0, 1.0],
    ];
    let set = match r.weighted(&[1.0 - busy, 1.0 - (busy - 0.5).abs(), busy]) {
        0 => SPARSE,
        1 => MIDDLE,
        _ => BUSY,
    };
    let cell = set[r.below(set.len())];
    cell.iter()
        .enumerate()
        .map(|(i, len)| (*len, i > 0 && i + 1 < cell.len() && r.chance(0.12)))
        .collect()
}

/// A tune's next move, in scale steps: mostly steps, sometimes a leap.
fn step(r: &mut Rng) -> i32 {
    [-3, -2, -1, 0, 1, 2, 3, 4][r.weighted(&[0.5, 1.5, 3.0, 0.8, 3.0, 1.5, 0.6, 0.3])]
}

/// MIDI note `n`'s pitch class moved by octaves into `low..=high`, as low
/// as it can go.
fn fit(n: i32, low: i32, high: i32) -> i32 {
    let mut m = low + (n - low).rem_euclid(12);
    if m > high {
        m -= 12;
    }
    m.max(0)
}

/// The chord's notes in a close voicing in the middle of `low..=high`.
fn voicing(key: u8, tones: &[u8], low: i32, high: i32) -> Vec<i32> {
    let mid = (low + high) / 2 - 4;
    let mut out: Vec<i32> = tones
        .iter()
        .map(|t| {
            let pc = (key as i32 + *t as i32).rem_euclid(12);
            let mut n = mid - (mid - pc).rem_euclid(12);
            if n < mid - 2 {
                n += 12;
            }
            while n > high {
                n -= 12;
            }
            while n < low {
                n += 12;
            }
            n
        })
        .filter(|n| *n >= low && *n <= high)
        .collect();
    out.sort();
    out.dedup();
    out
}

/// A bar of bass line in `pattern`.
fn bass(
    pattern: &str,
    key: u8,
    scale: Scale,
    chord: Chord,
    next: Chord,
    low: i32,
    high: i32,
) -> Vec<Note> {
    let root = fit((key + chord.root(scale)) as i32, low, high);
    let fifth = {
        let f = root + (chord.fifth(scale) as i32 - chord.root(scale) as i32);
        if f > high {
            f - 12
        } else {
            f
        }
    };
    let octave = if root + 12 <= high { root + 12 } else { root };
    let n = |beat: f32, len: f32, midi: i32, vel: f32| Note {
        beat,
        len,
        midi,
        vel,
    };
    match pattern {
        "root_fifth" => vec![n(0.0, 1.9, root, 0.8), n(2.0, 1.9, fifth, 0.7)],
        "pulse" => (0..8)
            .map(|i| {
                n(
                    i as f32 * 0.5,
                    0.42,
                    root,
                    if i % 2 == 0 { 0.78 } else { 0.62 },
                )
            })
            .collect(),
        "rock" => vec![
            n(0.0, 1.4, root, 0.82),
            n(1.5, 0.45, root, 0.6),
            n(2.0, 0.9, fifth, 0.7),
            n(3.0, 0.9, octave, 0.66),
        ],
        "walk" => {
            let tones = chord.tones(scale);
            let third = fit((key + tones[1]) as i32, root, high);
            let target = fit((key + next.root(scale)) as i32, low, high);
            let approach = if target > root {
                target - 1
            } else {
                target + 1
            };
            vec![
                n(0.0, 0.95, root, 0.8),
                n(1.0, 0.95, third.min(high), 0.68),
                n(2.0, 0.95, fifth, 0.72),
                n(3.0, 0.95, approach.clamp(low, high), 0.66),
            ]
        }
        "synth" => (0..16)
            .filter(|i| i % 4 != 3)
            .map(|i| {
                n(
                    i as f32 * 0.25,
                    0.2,
                    if i % 4 == 2 { octave } else { root },
                    if i % 4 == 0 { 0.8 } else { 0.55 },
                )
            })
            .collect(),
        "dub" => vec![
            n(0.0, 0.7, root, 0.82),
            n(1.5, 0.45, root, 0.62),
            n(2.5, 0.45, fifth, 0.65),
            n(3.0, 0.7, root, 0.7),
        ],
        _ => vec![n(0.0, BEATS * 0.97, root, 0.8)],
    }
}

/// A bar of `name` groove: when each hit falls, what it is, how hard, and
/// which layer it follows. `fill` turns the last beat into a fill; `crash`
/// opens the bar with a crash.
fn groove(name: &str, fill: bool, crash: bool, role: Role) -> Vec<(f32, &'static str, f32, Role)> {
    let lines: &[(&'static str, &str)] = match name {
        "four" => &[
            ("kick", "x...x...x...x..."),
            ("snare", "....x.......x..."),
            ("hat", "..x...x...x...x."),
            ("open", "..x.......x....."),
        ],
        "half" => &[
            ("kick", "x.........x....."),
            ("snare", "........x......."),
            ("hat", "x.x.x.x.x.x.x.x."),
            ("open", "..............x."),
        ],
        "break" => &[
            ("kick", "x.....x...x....."),
            ("snare", "....x..o.x..x..o"),
            ("hat", "xoxoxoxoxoxoxoxo"),
            ("open", "......x........."),
        ],
        "train" => &[
            ("kick", "x..x..x.x..x..x."),
            ("snare", "....x.......x..."),
            ("hat", "x.xxx.xxx.xxx.xx"),
            ("perc", "..x...x...x...x."),
        ],
        "brush" => &[
            ("snare", "x..ox..ox..ox..o"),
            ("kick", "x.......x......."),
            ("hat", "....x.......x..."),
            ("perc", "..o...o...o...o."),
        ],
        "march" => &[
            ("kick", "x...x...x...x..."),
            ("snare", "x.o.x.oox.o.x.oo"),
            ("perc", "x.......x......."),
        ],
        "bossa" => &[
            ("kick", "x..xx..xx..xx..x"),
            ("rim", "x..x..x...x..x.."),
            ("hat", "xoxoxoxoxoxoxoxo"),
        ],
        _ => &[
            ("kick", "x.....x.x......."),
            ("snare", "....x.......x..."),
            ("hat", "x.x.x.x.x.x.x.x."),
            ("open", "..............x."),
        ],
    };
    let mut out = Vec::new();
    for (what, steps) in lines {
        for (i, ch) in steps.chars().enumerate() {
            if fill && i >= 12 && *what == "snare" {
                continue;
            }
            let vel = match ch {
                'X' => 1.0,
                'x' => 0.8,
                'o' => 0.38,
                _ => continue,
            };
            let vel = if *what == "hat" { vel * 0.7 } else { vel };
            let layer = if matches!(*what, "open" | "perc") {
                Role::Lift
            } else {
                role
            };
            out.push((i as f32 * 0.25, *what, vel, layer));
        }
    }
    if fill {
        for (i, vel) in [(12, 0.55), (13, 0.62), (14, 0.72), (15, 0.85)] {
            out.push((i as f32 * 0.25, "snare", vel, role));
        }
    }
    if crash {
        out.push((0.0, "crash", 0.7, Role::Lift));
    }
    out
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;
    use crate::style::{Instrument, Kit};

    pub(crate) fn bank() -> Bank {
        let mut b = Bank::new("band");
        for (name, low, high) in [("bass", 28, 55), ("keys", 48, 84), ("lead", 60, 88)] {
            b.instruments.push(Instrument {
                name: name.into(),
                low,
                high,
                samples: (low..=high).step_by(3).collect(),
                length_s: 3.0,
                release_ms: 90,
            });
        }
        b.kits.push(Kit {
            name: "kit".into(),
            hits: ["kick", "snare", "hat", "open", "crash", "perc", "rim"]
                .iter()
                .map(|h| (h.to_string(), h.to_string()))
                .collect(),
        });
        b
    }

    pub(crate) fn style() -> Style {
        Style::new(
            "test",
            (96.0, 138.0),
            &[Scale::Minor, Scale::Dorian, Scale::Major],
        )
        .part(Part::new(
            Role::Base,
            Play::Bass {
                patterns: vec!["rock".into(), "walk".into(), "synth".into()],
            },
            &["bass"],
        ))
        .part(Part::new(Role::Base, Play::Pad, &["keys"]).pan(-0.2))
        .part(Part::new(
            Role::Pulse,
            Play::Drums {
                grooves: vec!["train".into(), "four".into()],
            },
            &["kit"],
        ))
        .part(Part::new(Role::Melody, Play::Lead, &["lead"]).pan(0.3))
        .part(Part::new(Role::Lift, Play::Counter, &["keys"]).range(55, 72))
        .part(Part::new(Role::Lift, Play::Arp, &["keys"]).range(60, 84))
        .part(Part::new(Role::Hurry, Play::Rush, &["kit"]))
        .part(Part::new(Role::Danger, Play::Drone, &["bass"]))
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::{bank, style};
    use super::*;

    #[test]
    fn the_same_seed_writes_the_same_song_and_another_seed_another() {
        let a = Song::compose(&style(), &bank(), 7);
        let b = Song::compose(&style(), &bank(), 7);
        assert_eq!(a, b);
        assert_eq!(a.section(3), b.section(3));
        let differ = (8..40)
            .map(|s| Song::compose(&style(), &bank(), s))
            .filter(|s| s.key != a.key || s.bpm != a.bpm || s.scale != a.scale)
            .count();
        assert!(
            differ >= 28,
            "only {differ} of 32 songs differ in key, tempo or scale"
        );
    }

    #[test]
    fn tempos_and_keys_stay_in_the_style() {
        let style = style().keys(&[2, 9]);
        for seed in 0..200 {
            let s = Song::compose(&style, &bank(), seed);
            assert!((96.0..=138.0).contains(&s.bpm), "{}", s.bpm);
            assert!([2, 9].contains(&s.key));
        }
    }

    #[test]
    fn every_bar_keeps_its_notes_in_range_and_inside_the_bar() {
        for seed in 0..40 {
            let s = Song::compose(&style(), &bank(), seed);
            for i in 0..12 {
                let sec = s.section(i);
                for bar in 0..sec.bars() {
                    for (pi, e) in s.bar(&sec, bar) {
                        assert!(e.beat() >= 0.0 && e.beat() < BEATS, "{e:?}");
                        if let Event::Note(n) = e {
                            let c = &s.parts[pi];
                            assert!(
                                n.midi >= c.low && n.midi <= c.high,
                                "{} {n:?} not in {}..{}",
                                c.instrument,
                                c.low,
                                c.high
                            );
                            assert!(n.len > 0.0 && n.beat + n.len <= BEATS + 0.01, "{n:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_tune_bar_fills_its_four_beats_and_a_return_ends_on_the_key() {
        let s = Song::compose(&style(), &bank(), 11);
        let sec = s.section(3);
        assert_eq!(sec.kind, SectionKind::AReturn);
        let last = sec.tune.last().unwrap().last().unwrap();
        assert_eq!((last.midi - s.key as i32).rem_euclid(12), 0, "{last:?}");
        // A again keeps A's opening.
        assert_eq!(s.section(1).tune[0], sec.tune[0]);
        // The next round's B is new.
        assert_ne!(s.section(2).tune, s.section(7).tune);
    }

    #[test]
    fn a_style_names_only_what_the_bank_has() {
        assert!(
            style().problems(&bank()).is_empty(),
            "{:?}",
            style().problems(&bank())
        );
        let bad = style().part(Part::new(
            Role::Base,
            Play::Bass {
                patterns: vec!["polka".into()],
            },
            &["tuba"],
        ));
        let p = bad.problems(&bank());
        assert_eq!(p.len(), 2, "{p:?}");
    }
}
