//! The formant speech synthesizer from genny in Rust: a Klatt-style cascade
//! with an English text front-end, for a game to speak by itself on a device
//! with no screen reader or speech engine.
//!
//! Ported from `genny/speech.py` in genny by Oriol Gómez
//! (<https://github.com/ogomez92/genny>), with his permission; see LICENSE.
//! No models and no downloads: text becomes phonemes (genny's built-in
//! dictionary, the bundled CMU Pronouncing Dictionary with the default
//! `cmudict` feature, suffix stripping, number words, letter names and a
//! letter-to-sound fallback, then weak forms and flaps across the phrase),
//! then durations and a pitch contour, then parameter tracks that drive a
//! glottal source, aspiration and frication noise through time-varying
//! resonators. The British voices and their accent rules are Hearken's own,
//! added by Joshua Tubbs (Orinks).
//!
//! ```
//! let voice = hearken_genny::voice("US female").unwrap();
//! let samples = hearken_genny::speak("Level up!", &voice, 44_100.0, 1.0);
//! assert!(!samples.is_empty());
//! ```

mod accent;
mod cmudict;
mod phonemes;
mod synth;
mod text;

pub use text::to_phonemes;

/// The voices a player chooses from, in Settings order.
pub const VOICE_NAMES: [&str; 6] = [
    "US female",
    "US male",
    "British female",
    "British male",
    "US child",
    "Robot",
];

/// A voice preset: the source and tract settings the synth reads. The US
/// and Robot presets are genny's male, female, child and robot voices.
#[derive(Clone, Debug, PartialEq)]
pub struct Voice {
    /// Base F0 in Hz.
    pub pitch: f64,
    /// Scales every formant: tract size and timbre.
    pub formant_shift: f64,
    /// Aspiration mixed into the voicing.
    pub breath: f64,
    /// Glottal open quotient.
    pub oq: f64,
    /// Spectral tilt corner, Hz.
    pub tilt: f64,
    /// Slow random wander of the pitch, as a fraction of F0.
    pub jitter: f64,
    /// Slow random wander of the source level, as a fraction.
    pub shimmer: f64,
    /// Scales every formant bandwidth.
    pub bw: f64,
    /// Monotone.
    pub flat: bool,
    /// Ring modulation depth and rate (the robot).
    pub ring: f64,
    pub ring_hz: f64,
    /// The light British accent.
    pub british: bool,
}

const MALE: Voice = Voice {
    pitch: 115.0,
    formant_shift: 1.0,
    breath: 0.08,
    oq: 0.62,
    tilt: 5000.0,
    jitter: 0.004,
    shimmer: 0.035,
    bw: 1.0,
    flat: false,
    ring: 0.0,
    ring_hz: 55.0,
    british: false,
};

const FEMALE: Voice = Voice {
    pitch: 205.0,
    formant_shift: 1.16,
    breath: 0.18,
    oq: 0.70,
    tilt: 3600.0,
    jitter: 0.004,
    shimmer: 0.03,
    bw: 1.15,
    ..MALE
};

/// A preset by its name in [`VOICE_NAMES`] (any case).
pub fn voice(name: &str) -> Option<Voice> {
    let v = match name.to_ascii_lowercase().as_str() {
        "us female" => FEMALE,
        "us male" => MALE,
        "british female" => Voice {
            pitch: 195.0,
            formant_shift: 1.14,
            british: true,
            ..FEMALE
        },
        "british male" => Voice {
            pitch: 108.0,
            formant_shift: 0.98,
            british: true,
            ..MALE
        },
        "us child" => Voice {
            pitch: 280.0,
            formant_shift: 1.28,
            breath: 0.14,
            oq: 0.68,
            tilt: 4200.0,
            jitter: 0.005,
            shimmer: 0.03,
            bw: 1.25,
            ..MALE
        },
        "robot" => Voice {
            pitch: 100.0,
            breath: 0.0,
            oq: 0.5,
            tilt: 7000.0,
            jitter: 0.0,
            shimmer: 0.0,
            bw: 0.9,
            flat: true,
            ring: 0.35,
            ..MALE
        },
        _ => return None,
    };
    Some(v)
}

/// Render `text` as mono samples at `sr` Hz. `rate` is a speed multiplier
/// (1.0 is genny's own pace). Empty when there is nothing to say.
pub fn speak(text: &str, voice: &Voice, sr: f64, rate: f64) -> Vec<f32> {
    let mut phrases = text::text_to_phrases(text);
    phrases.retain(|p| p.words.iter().any(|w| !w.ph.is_empty()));
    if phrases.is_empty() {
        return Vec::new();
    }
    if voice.british {
        accent::british(&mut phrases);
    }
    synth::render(&phrases, voice, sr, rate)
}

/// Split text into sentences, so a long line can start playing as soon as
/// its first sentence is rendered. Each keeps its closing punctuation.
pub fn sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let bytes: Vec<(usize, char)> = text.char_indices().collect();
    for (k, &(i, c)) in bytes.iter().enumerate() {
        let end_of_sentence = matches!(c, '.' | '!' | '?' | ';')
            && bytes.get(k + 1).is_none_or(|&(_, n)| n.is_whitespace());
        if end_of_sentence {
            let piece = text[start..i + c.len_utf8()].trim();
            if !piece.is_empty() {
                out.push(piece);
            }
            start = i + c.len_utf8();
        }
    }
    let rest = text[start..].trim();
    if !rest.is_empty() {
        out.push(rest);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `(text, transcription)` pairs, as genny by Oriol Gómez (13258e8)
    /// prints them with `genny say TEXT --show`.
    fn check(cases: &[(&str, &str)]) {
        for &(text, want) in cases {
            assert_eq!(to_phonemes(text), want, "{text}");
        }
    }

    #[test]
    #[cfg(feature = "cmudict")]
    fn transcribes_like_genny() {
        check(&[
            ("Play, 1 of 6.", "P L EY1 ,  W AH1 N | AH0 V | S IH1 K S ."),
            (
                "Nevermore Manor. Rooms cleared: 4.",
                "N EH1 V ER0 M AO2 R | M AE1 N ER0 .  R UW1 M Z | K L IH1 R D :  F AO1 R .",
            ),
            (
                "Train approaching, platform two.",
                "T R EY1 N | AH0 P R OW1 CH IH0 NG ,  P L AE1 T F AO2 R M | T UW1 .",
            ),
            (
                "Wicket's. Every game speaks through your screen reader.",
                "W IH1 K AH0 T S .  EH1 V ER0 IY0 | G EY1 M | S P IY1 K S | TH R UW1 | Y ER0 | S K R IY1 N | R IY1 DX ER0 .",
            ),
            // A function word stranded at the end keeps its full form.
            ("What is it for?", "W AH1 T | IH0 Z | IH0 T | F AO1 R ?"),
            // "the" before a vowel.
            (
                "Go to the end of the hall, then wait.",
                "G OW1 | T AH0 | DH IY0 | EH1 N D | AH0 V | DH AH0 | HH AO1 L ,  DH EH1 N | W EY1 T .",
            ),
            (
                "Better water is ready.",
                "B EH1 DX ER0 | W AO1 DX ER0 | IH0 Z | R EH1 DX IY0 .",
            ),
            // Suffixes stripped to a CMUdict stem.
            (
                "Respawned, respawning, kindness.",
                "R IY2 S P AA1 N D ,  R IY2 S P AA1 N IH0 NG ,  K AY1 N D N AH0 S .",
            ),
            // Suffixes stripped to a built-in stem.
            (
                "Lost lives, finishing, stopped, timers, fastly.",
                "L AO1 S T | L AY1 V Z ,  F IH1 N IH0 SH IH0 NG ,  S T AA1 P T ,  T AY1 M ER0 Z ,  F AE1 S T L IY0 .",
            ),
            // Zyxor and sneakily are in neither dictionary; HUD reads as a word.
            (
                "Zyxor sneakily unlocked the HUD and a key.",
                "Z IH1 K S AO0 R | S N IY1 K IH0 L IY0 | AH0 N L AA1 K T | DH AH0 | HH AH1 D | AH0 N D | AH0 | K IY1 .",
            ),
            ("A", "EY1"),
        ]);
    }

    #[test]
    #[cfg(not(feature = "cmudict"))]
    fn transcribes_like_genny_without_cmudict() {
        // genny with its CMUdict lookups emptied.
        check(&[
            ("Play, 1 of 6.", "P L EY1 ,  W AH1 N | AH0 V | S IH1 K S ."),
            (
                "Nevermore Manor. Rooms cleared: 4.",
                "N EH1 V ER0 M OW0 R | M AE1 N AO0 R .  R UW1 M Z | K L IH1 R D :  F AO1 R .",
            ),
            (
                "Train approaching, platform two.",
                "T R EY1 N | AE1 P R OW0 CH IH0 NG ,  P L AE1 T F AO0 R M | T UW1 .",
            ),
            (
                "Wicket's. Every game speaks through your screen reader.",
                "W IH1 K AH0 T S .  EH1 V ER0 IY0 | G EY1 M | S P IY1 K S | TH R AO1 | Y ER0 | S K R IY1 N | R IY1 DX ER0 .",
            ),
            ("What is it for?", "W AE1 T | IH0 Z | IH0 T | F ER0 ?"),
            (
                "Go to the end of the hall, then wait.",
                "G OW1 | T AH0 | DH IY0 | EH1 N D | AH0 V | DH AH0 | HH AE1 L ,  TH EH1 N | W EY1 T .",
            ),
            (
                "Better water is ready.",
                "B EH1 DX ER0 | W AE1 DX ER0 | IH0 Z | R EH1 DX IY0 .",
            ),
            (
                "Lost lives, finishing, stopped, timers, fastly.",
                "L AO1 S T | L AY1 V Z ,  F IH1 N IH0 SH IH0 NG ,  S T AA1 P T ,  T AY1 M ER0 Z ,  F AE1 S T L IY0 .",
            ),
            (
                "Zyxor sneakily unlocked the HUD and a key.",
                "Z IH1 K S AO0 R | S N IY1 K IH0 L IY0 | AH1 N L AH0 K AH0 D | DH IY0 | EY1 CH | Y UW1 | D IY1 | AH0 N D | AH0 | K IY1 .",
            ),
        ]);
    }

    #[test]
    fn every_voice_speaks() {
        for name in VOICE_NAMES {
            let v = voice(name).unwrap();
            let s = speak("Settings. Speech voice, ready?", &v, 22_050.0, 1.0);
            assert!(s.len() > 22_050, "{name} rendered {} samples", s.len());
            let peak = s.iter().fold(0.0f32, |m, x| m.max(x.abs()));
            assert!((0.8..=0.9).contains(&peak), "{name} peak {peak}");
            assert!(s.iter().all(|x| x.is_finite()));
        }
    }

    #[test]
    fn nothing_to_say_is_silent() {
        assert!(speak("  ... ", &voice("US male").unwrap(), 44_100.0, 1.0).is_empty());
    }

    #[test]
    fn faster_is_shorter() {
        let v = voice("US male").unwrap();
        let slow = speak("Rooms cleared: four.", &v, 16_000.0, 1.0).len();
        let fast = speak("Rooms cleared: four.", &v, 16_000.0, 2.0).len();
        assert!(fast < slow * 2 / 3, "{fast} vs {slow}");
    }

    #[test]
    fn british_drops_r_and_rounds_lot() {
        use crate::text::text_to_phrases;
        let mut p = text_to_phrases("car stop");
        accent::british(&mut p);
        let names: Vec<&str> = p[0]
            .words
            .iter()
            .flat_map(|w| w.ph.iter().map(|&(q, _)| phonemes::ph(q).name))
            .collect();
        assert!(!names.contains(&"R"), "{names:?}");
        assert!(names.contains(&"OH"), "{names:?}");
    }

    #[test]
    fn splits_sentences() {
        assert_eq!(
            sentences("Play. 1 of 6! Ready? Go"),
            vec!["Play.", "1 of 6!", "Ready?", "Go"]
        );
        assert_eq!(sentences("2.5 points."), vec!["2.5 points."]);
    }
}
