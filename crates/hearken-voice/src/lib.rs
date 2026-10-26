//! AI voices for Hearken games: Kokoro speech, synthesized offline through
//! [local-tts](https://github.com/GarAlex/local-tts) and ONNX Runtime, with
//! no account, key or network once the voice package is installed.
//!
//! The model is behind the `voice` feature. Without it this crate only finds
//! the `voices` folder, so a game can say why its AI voices are missing.
//!
//! This is the first cut (the voice spike): load the model and turn a line
//! into WAV bytes that `hearken-audio` plays. The worker thread, cache and
//! queue come with the `Voice` service.

use std::path::{Path, PathBuf};

/// The voice package Hearken uses: Kokoro-82M, 8-bit, 28 English voices.
pub const PACKAGE: &str = "kokoro-en-q8";

/// Picks a voices folder: the first of `<env_prefix>_VOICES` (non-empty),
/// the executable's own `voices` folder, or the current directory's
/// `voices` folder, that exists as a directory.
pub fn find_voices(env_prefix: &str) -> Option<PathBuf> {
    let var = format!("{env_prefix}_VOICES");
    if let Some(dir) = std::env::var_os(&var).filter(|d| !d.is_empty()) {
        let path = PathBuf::from(dir);
        if path.is_dir() {
            log::info!("voices: using {var} ({})", path.display());
            return Some(path);
        }
        log::warn!("voices: {var} is set to {path:?} but that is not a directory; ignoring");
    }
    let beside_exe = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.join("voices")));
    let in_cwd = std::env::current_dir().ok().map(|d| d.join("voices"));
    beside_exe.into_iter().chain(in_cwd).find(|p| p.is_dir())
}

/// The installed package's folder under `voices`, when it is there.
pub fn package_dir(voices: &Path) -> Option<PathBuf> {
    let dir = voices.join(PACKAGE);
    dir.join("model.onnx").is_file().then_some(dir)
}

/// What went wrong loading or speaking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// One synthesized line: mono samples in [-1, 1].
#[derive(Debug, Clone, PartialEq)]
pub struct Speech {
    pub sample_rate: u32,
    pub samples: Vec<f32>,
}

impl Speech {
    /// Seconds of audio.
    pub fn duration_s(&self) -> f64 {
        self.samples.len() as f64 / f64::from(self.sample_rate.max(1))
    }

    /// A 16-bit mono WAV file in memory, for `ctx.audio.play_clip`.
    pub fn to_wav(&self) -> Vec<u8> {
        let data_len = (self.samples.len() * 2) as u32;
        let mut out = Vec::with_capacity(44 + data_len as usize);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_len).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes()); // PCM
        out.extend_from_slice(&1u16.to_le_bytes()); // mono
        out.extend_from_slice(&self.sample_rate.to_le_bytes());
        out.extend_from_slice(&(self.sample_rate * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        for s in &self.samples {
            let v = if s.is_finite() {
                s.clamp(-1.0, 1.0)
            } else {
                0.0
            };
            out.extend_from_slice(&((v * 32767.0) as i16).to_le_bytes());
        }
        out
    }
}

#[cfg(feature = "voice")]
pub use kokoro::Kokoro;

#[cfg(feature = "voice")]
mod kokoro {
    use super::{Error, Speech};
    use local_tts_core::{Language, SynthesisRequest, Synthesizer};
    use std::path::Path;

    /// The Kokoro model, loaded once. Synthesis blocks for about a real-time
    /// fraction of the line's length, so call it off the game's frame.
    pub struct Kokoro {
        engine: local_tts_core::kokoro::Kokoro,
    }

    impl Kokoro {
        /// Loads the package folder (`voices/kokoro-en-q8`), checking its
        /// pinned checksums. Words the dictionary lacks, such as team names,
        /// are guessed rather than refused.
        pub fn load(package_dir: &Path) -> Result<Kokoro, Error> {
            let mut engine = local_tts_core::kokoro::Kokoro::load(package_dir)
                .map_err(|e| Error(format!("loading {}: {e}", package_dir.display())))?;
            engine
                .set_fallback(true)
                .map_err(|e| Error(e.to_string()))?;
            Ok(Kokoro { engine })
        }

        /// The voices installed, the default first.
        pub fn speakers(&self) -> Vec<String> {
            self.engine.speakers()
        }

        /// Adds CMU-format pronunciations (`voices/lexicon.txt`); returns
        /// how many were read.
        pub fn add_lexicon(&mut self, path: &Path) -> Result<usize, Error> {
            self.engine
                .add_lexicon(path)
                .map_err(|e| Error(e.to_string()))
        }

        /// Speaks `text` in `speaker` (`bf_emma`, `am_michael`, ...).
        /// `rate` above 1 is faster; it is clamped to 0.5 to 2.
        pub fn synthesize(
            &mut self,
            text: &str,
            speaker: &str,
            rate: f32,
        ) -> Result<Speech, Error> {
            let rate = if rate.is_finite() {
                rate.clamp(0.5, 2.0)
            } else {
                1.0
            };
            let audio = self
                .engine
                .synthesize(&SynthesisRequest {
                    text,
                    language: Language::En,
                    speaker,
                    duration_scale: 1.0 / rate,
                })
                .map_err(|e| Error(e.to_string()))?;
            Ok(Speech {
                sample_rate: audio.sample_rate,
                samples: audio.samples,
            })
        }
    }
}

/// Downloads and verifies the voice package into `voices/kokoro-en-q8`
/// from its pinned Hugging Face URLs (about 92 MB, no account). An installed
/// package is left as it is. `progress` gets (bytes so far, total).
#[cfg(feature = "download")]
pub fn install(
    voices: &Path,
    progress: &mut dyn FnMut(u64, u64),
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<PathBuf, Error> {
    std::fs::create_dir_all(voices).map_err(|e| Error(e.to_string()))?;
    local_tts_core::catalog::install(PACKAGE, voices, None, progress, cancel).map_err(Error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_matches_the_samples() {
        let speech = Speech {
            sample_rate: 24_000,
            samples: vec![0.0, 0.5, -0.5, 1.5, f32::NAN],
        };
        let wav = speech.to_wav();
        assert_eq!(wav.len(), 44 + 10);
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 24_000);
        assert_eq!(i16::from_le_bytes([wav[46], wav[47]]), 16_383);
        assert_eq!(i16::from_le_bytes([wav[50], wav[51]]), 32_767);
        assert_eq!(i16::from_le_bytes([wav[52], wav[53]]), 0);
    }

    #[test]
    fn package_dir_needs_the_model() {
        let dir = std::env::temp_dir().join("hearken-voice-no-model");
        let _ = std::fs::create_dir_all(dir.join(PACKAGE));
        assert_eq!(package_dir(&dir), None);
    }
}
