//! genny's formant speech synthesizer in Rust: a Klatt-style cascade with a
//! small English text front-end, for a game to speak by itself on a device
//! with no screen reader or speech engine.
//!
//! Ported from `genny/speech.py` (genny by Oriol Gómez, and blal1's fork),
//! with the authors' permission; see LICENSE. No models and no downloads:
//! text becomes phonemes (built-in dictionary, number words, letter names,
//! a letter-to-sound fallback), then durations and a pitch contour, then
//! parameter tracks that drive a glottal source, aspiration and frication
//! noise through time-varying resonators.
//!
//! ```
//! let voice = hearken_genny::voice("US female").unwrap();
//! let samples = hearken_genny::speak("Level up!", &voice, 44_100.0, 1.0);
//! assert!(!samples.is_empty());
//! ```

mod accent;
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

/// A voice preset: the source and tract settings the synth reads.
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
    pub jitter: f64,
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
    phrases.retain(|p| {
        p.words
            .iter()
            .any(|text::Word::Spoken { ph, .. }| !ph.is_empty())
    });
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

    #[test]
    fn transcribes_like_genny() {
        // The same transcriptions genny 1.2.1 prints with `genny say --show`.
        assert_eq!(
            to_phonemes("Play, 1 of 6."),
            "P L EY1 ,  W AH1 N | AH0 V | S IH1 K S ."
        );
        assert_eq!(
            to_phonemes("Nevermore Manor. Rooms cleared: 4."),
            "N EH1 V ER0 M OW0 R | M AE1 N AO0 R .  R UW1 M Z | K L IH1 R D :  F AO1 R ."
        );
        assert_eq!(
            to_phonemes("Train approaching, platform two."),
            "T R EY1 N | AE1 P R OW0 CH IH0 NG ,  P L AE1 T F AO0 R M | T UW1 ."
        );
        assert_eq!(
            to_phonemes("Wicket's. Every game speaks through your screen reader."),
            "W IH1 K AH0 T S .  EH1 V ER0 IY0 | G EY1 M | S P IY1 K S | TH R AO1 | Y AO1 R | S K R IY1 N | R IY1 D ER0 ."
        );
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
            .flat_map(|text::Word::Spoken { ph, .. }| ph.iter().map(|&(q, _)| phonemes::ph(q).name))
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
