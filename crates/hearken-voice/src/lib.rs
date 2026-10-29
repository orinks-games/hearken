//! AI voices for Hearken games: Kokoro speech, synthesized offline through
//! [local-tts](https://github.com/GarAlex/local-tts) and ONNX Runtime, with
//! no account, key or network once the voice package is installed.
//!
//! The model is behind the `voice` feature. Without it this crate only finds
//! the `voices` folder, so a game can say why its AI voices are missing.
//!
//! [`Voice`] is the service a game installs with `ctx.set_commentary`: the
//! model loads once on a worker thread, lines are synthesized there and
//! cached in memory and under `voice-cache/` in the game's data folder, and
//! they play one at a time on `Bus::VOICE` with the pacer's rules. With no
//! model, or with the player's setting on the screen reader, lines go to the
//! event voice instead. [`FakeVoice`] records lines for `Headless` tests.

use std::path::{Path, PathBuf};

mod cache;
mod fake;
#[cfg(feature = "download")]
mod install_screen;
mod rules;
mod service;
mod settings;
mod synth;
mod worker;

pub use fake::{FakeVoice, Voiced, VoicedLog};
#[cfg(feature = "download")]
pub use install_screen::{install_confirm, InstallScreen, INSTALL_QUESTION};
pub use service::{Voice, VoiceConfig, NOT_INSTALLED, NO_MODEL_IN_BUILD};
pub use settings::{settings_rows, HasVoiceSettings, Role, VoiceMode, VoiceSettings};
pub use synth::{Loader, Synth};

/// The 28 English voices of `kokoro-en-q8`, for Settings before the model
/// has loaded. American (`a`) and British (`b`), female (`f`) and male (`m`).
pub const KOKORO_VOICES: [&str; 28] = [
    "af_heart",
    "af_alloy",
    "af_aoede",
    "af_bella",
    "af_jessica",
    "af_kore",
    "af_nicole",
    "af_nova",
    "af_river",
    "af_sarah",
    "af_sky",
    "am_adam",
    "am_echo",
    "am_eric",
    "am_fenrir",
    "am_liam",
    "am_michael",
    "am_onyx",
    "am_puck",
    "am_santa",
    "bf_alice",
    "bf_emma",
    "bf_isabella",
    "bf_lily",
    "bm_daniel",
    "bm_fable",
    "bm_george",
    "bm_lewis",
];

/// The voice package Hearken uses: Kokoro-82M, 8-bit, 28 English voices.
pub const PACKAGE: &str = "kokoro-en-q8";

/// Picks a voices folder: the first of `<env_prefix>_VOICES` (non-empty),
/// or else the first of the executable's own `voices` folder and the current
/// directory's `voices` folder that holds the installed package.
pub fn find_voices(env_prefix: &str) -> Option<PathBuf> {
    find_voices_in(env_prefix, None)
}

/// [`find_voices`], also looking in `data_dir/voices` (where the in-game
/// installer puts the package) after the executable's folder.
pub fn find_voices_in(env_prefix: &str, data_dir: Option<&Path>) -> Option<PathBuf> {
    let var = format!("{env_prefix}_VOICES");
    if let Some(dir) = std::env::var_os(&var).filter(|d| !d.is_empty()) {
        let path = PathBuf::from(dir);
        if path.is_dir() {
            log::info!("voices: using {var} ({})", path.display());
            return Some(path);
        }
        log::warn!("voices: {var} is set to {path:?} but that is not a directory; ignoring");
    }
    // Beside the program, or in a Mac app's Resources.
    let beside_exe: Vec<PathBuf> = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(game_dirs))
        .unwrap_or_default()
        .into_iter()
        .map(|d| d.join("voices"))
        .collect();
    let in_data = data_dir.map(|d| d.join("voices"));
    let in_cwd = std::env::current_dir().ok().map(|d| d.join("voices"));
    beside_exe
        .into_iter()
        .chain(in_data)
        .chain(in_cwd)
        .find(|p| package_dir(p).is_some())
}

/// The program's folder and, in a Mac app (`Game.app/Contents/MacOS`), the
/// app's `Contents/Resources` before it (as `hearken_audio::game_dirs`).
fn game_dirs(exe_dir: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if exe_dir.ends_with("Contents/MacOS") {
        if let Some(contents) = exe_dir.parent() {
            dirs.push(contents.join("Resources"));
        }
    }
    dirs.push(exe_dir.to_path_buf());
    dirs
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
impl Synth for Kokoro {
    fn synthesize(&mut self, text: &str, voice: &str, rate: f32) -> Result<Speech, Error> {
        Kokoro::synthesize(self, text, voice, rate)
    }

    fn speakers(&self) -> Vec<String> {
        Kokoro::speakers(self)
    }
}

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
