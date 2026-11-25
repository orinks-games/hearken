//! Scales, chords and note names.

use serde::{Deserialize, Serialize};

/// The scales a song can be in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scale {
    Major,
    Minor,
    Dorian,
    Mixolydian,
    Lydian,
    Phrygian,
    HarmonicMinor,
    /// Major pentatonic: the tune keeps to five notes, the chords to major.
    Pentatonic,
    /// Minor pentatonic: the tune keeps to five notes, the chords to minor.
    MinorPentatonic,
    /// The blues scale over dorian chords.
    Blues,
}

impl Scale {
    pub const ALL: [Scale; 10] = [
        Scale::Major,
        Scale::Minor,
        Scale::Dorian,
        Scale::Mixolydian,
        Scale::Lydian,
        Scale::Phrygian,
        Scale::HarmonicMinor,
        Scale::Pentatonic,
        Scale::MinorPentatonic,
        Scale::Blues,
    ];

    /// The seven notes the chords are built from, in semitones over the root.
    pub fn harmony(self) -> [u8; 7] {
        match self {
            Scale::Major | Scale::Pentatonic => [0, 2, 4, 5, 7, 9, 11],
            Scale::Minor | Scale::MinorPentatonic => [0, 2, 3, 5, 7, 8, 10],
            Scale::Dorian | Scale::Blues => [0, 2, 3, 5, 7, 9, 10],
            Scale::Mixolydian => [0, 2, 4, 5, 7, 9, 10],
            Scale::Lydian => [0, 2, 4, 6, 7, 9, 11],
            Scale::Phrygian => [0, 1, 3, 5, 7, 8, 10],
            Scale::HarmonicMinor => [0, 2, 3, 5, 7, 8, 11],
        }
    }

    /// The notes a tune may use, in semitones over the root.
    pub fn tune(self) -> Vec<u8> {
        match self {
            Scale::Pentatonic => vec![0, 2, 4, 7, 9],
            Scale::MinorPentatonic => vec![0, 3, 5, 7, 10],
            Scale::Blues => vec![0, 3, 5, 6, 7, 10],
            other => other.harmony().to_vec(),
        }
    }

    /// Whether the scale sounds minor: its third is flat.
    pub fn is_minor(self) -> bool {
        self.harmony()[2] == 3
    }

    pub fn name(self) -> &'static str {
        match self {
            Scale::Major => "major",
            Scale::Minor => "minor",
            Scale::Dorian => "dorian",
            Scale::Mixolydian => "mixolydian",
            Scale::Lydian => "lydian",
            Scale::Phrygian => "phrygian",
            Scale::HarmonicMinor => "harmonic minor",
            Scale::Pentatonic => "pentatonic",
            Scale::MinorPentatonic => "minor pentatonic",
            Scale::Blues => "blues",
        }
    }
}

/// A chord: the scale degree it is built on (0 is the tonic), in a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chord {
    pub degree: u8,
    /// Add the seventh.
    pub seventh: bool,
}

impl Chord {
    pub const fn on(degree: u8) -> Chord {
        Chord {
            degree,
            seventh: false,
        }
    }

    /// The chord's notes as pitch classes over the key's root: root, third,
    /// fifth, and the seventh if it has one.
    pub fn tones(self, scale: Scale) -> Vec<u8> {
        let h = scale.harmony();
        let d = self.degree as usize % 7;
        let mut out = vec![
            h[d],
            h[(d + 2) % 7] + if d + 2 >= 7 { 12 } else { 0 },
            h[(d + 4) % 7] + if d + 4 >= 7 { 12 } else { 0 },
        ];
        if self.seventh {
            out.push(h[(d + 6) % 7] + if d + 6 >= 7 { 12 } else { 0 });
        }
        out
    }

    /// The chord's root over the key's root, 0 to 11.
    pub fn root(self, scale: Scale) -> u8 {
        scale.harmony()[self.degree as usize % 7]
    }

    /// The chord's fifth over the key's root (may pass 12).
    pub fn fifth(self, scale: Scale) -> u8 {
        self.tones(scale)[2]
    }
}

/// The name of a pitch class, 0 C to 11 B.
pub fn pitch_name(pc: u8) -> &'static str {
    [
        "C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B",
    ][pc as usize % 12]
}

/// The nearest of `allowed` (pitch classes over `root`) to MIDI note `n`,
/// preferring below on a tie.
pub fn snap(n: i32, root: u8, allowed: &[u8]) -> i32 {
    let mut best = n;
    let mut gap = i32::MAX;
    for d in 0..=12 {
        for cand in [n - d, n + d] {
            let pc = (cand - root as i32).rem_euclid(12) as u8;
            if allowed.iter().any(|a| a % 12 == pc) && d < gap {
                best = cand;
                gap = d;
            }
        }
        if gap != i32::MAX {
            break;
        }
    }
    best
}

/// The scale notes (as MIDI) from `low` to `high` inclusive.
pub fn ladder(root: u8, allowed: &[u8], low: i32, high: i32) -> Vec<i32> {
    (low..=high)
        .filter(|n| {
            let pc = (n - root as i32).rem_euclid(12) as u8;
            allowed.iter().any(|a| a % 12 == pc)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords_are_built_from_the_scale() {
        assert_eq!(Chord::on(0).tones(Scale::Major), vec![0, 4, 7]);
        assert_eq!(Chord::on(5).tones(Scale::Major), vec![9, 12, 16]);
        assert_eq!(Chord::on(4).tones(Scale::HarmonicMinor), vec![7, 11, 14]);
        let c = Chord {
            degree: 1,
            seventh: true,
        };
        assert_eq!(c.tones(Scale::Dorian), vec![2, 5, 9, 12]);
    }

    #[test]
    fn snapping_finds_the_nearest_allowed_note() {
        // C major from C: 61 (C#) snaps down to 60.
        assert_eq!(snap(61, 0, &[0, 2, 4, 5, 7, 9, 11]), 60);
        assert_eq!(snap(66, 0, &[0, 4, 7]), 67);
        assert_eq!(snap(62, 0, &[0, 4, 7]), 60);
        assert_eq!(ladder(0, &[0, 4, 7], 60, 72), vec![60, 64, 67, 72]);
    }
}
