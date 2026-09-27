//! Built-in earcons and per-category default sounds.
//!
//! A game that ships no audio at all still gets feedback: the seven `UI_*`
//! keys each have their own short earcon, and every other key falls back to
//! a default for its [`SoundKind`] (one sound shared by every key in that
//! category, not per key -- a thousand missing sfx keys hold one clip).

use crate::SAMPLE_RATE;
use fundsp::prelude32::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// `ui/menu_open`.
pub const UI_OPEN: &str = "ui/menu_open";
/// `ui/menu_move`.
pub const UI_MOVE: &str = "ui/menu_move";
/// `ui/menu_select`.
pub const UI_SELECT: &str = "ui/menu_select";
/// `ui/menu_back`.
pub const UI_BACK: &str = "ui/menu_back";
/// `ui/tick`.
pub const UI_TICK: &str = "ui/tick";
/// `ui/error`.
pub const UI_ERROR: &str = "ui/error";
/// `ui/notify`.
pub const UI_NOTIFY: &str = "ui/notify";

/// The category a sound key falls into, read from its first path segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SoundKind {
    Ui,
    Sfx,
    Ambience,
    Music,
    Other,
}

/// Sorts a key into its [`SoundKind`] by the folder before its first `/`
/// (`"ui/"`, `"sfx/"`, `"amb/"`, `"music/"`); anything else is `Other`.
pub fn kind(key: &str) -> SoundKind {
    match key.split('/').next().unwrap_or("") {
        "ui" => SoundKind::Ui,
        "sfx" => SoundKind::Sfx,
        "amb" => SoundKind::Ambience,
        "music" => SoundKind::Music,
        _ => SoundKind::Other,
    }
}

const SR: f32 = SAMPLE_RATE as f32;

/// A short sine tone: linear 3 ms attack, exponential decay to -60 dB by the
/// last sample.
fn tone(hz: f32, seconds: f32, peak: f32) -> Vec<f32> {
    let n = (seconds * SR) as usize;
    // A 3 ms attack is negligible next to most earcons, but on the shortest
    // one (the 12 ms tick) it would still be running when the decay has
    // already cut deep, so the ramp never reaches an audible level. Capping
    // it to a share of the tone's own length keeps every longer tone
    // byte-for-byte unchanged while giving a short tone a short attack.
    let attack = (0.003 * SR).min(n as f32 / 8.0);
    let decay = 1000.0f32.ln() / n as f32;
    (0..n)
        .map(|i| {
            let t = i as f32;
            peak * (t / attack).min(1.0)
                * (-decay * t).exp()
                * (std::f32::consts::TAU * hz * t / SR).sin()
        })
        .collect()
}

/// Concatenates two tones end to end.
fn join(mut a: Vec<f32>, b: Vec<f32>) -> Vec<f32> {
    a.extend(b);
    a
}

/// Renders a fundsp graph to mono samples at [`SAMPLE_RATE`].
fn render(mut unit: Box<dyn AudioUnit>, seconds: f32) -> Vec<f32> {
    unit.set_sample_rate(SAMPLE_RATE as f64);
    let n = (seconds * SR) as usize;
    (0..n).map(|_| unit.get_mono()).collect()
}

/// Blends a render's trailing extra `seconds` into its start with an
/// equal-power crossfade, so looping it never clicks at the seam.
fn seamless(rendered: &[f32], seconds: f32) -> Vec<f32> {
    let n = (seconds * SR) as usize;
    let x = (0.25 * SR) as usize;
    let mut out = rendered[..n].to_vec();
    for (i, out_i) in out.iter_mut().enumerate().take(x) {
        let w = i as f32 / x as f32 * std::f32::consts::FRAC_PI_2;
        *out_i = rendered[i] * w.sin() + rendered[n + i] * w.cos();
    }
    out
}

fn normalize_peak(v: &mut [f32], target: f32) {
    let p = v.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if p > 0.0 {
        let scale = target / p;
        v.iter_mut().for_each(|s| *s *= scale);
    }
}

fn normalize_rms(v: &mut [f32], target: f32) {
    let mean_sq = v.iter().map(|s| s * s).sum::<f32>() / v.len() as f32;
    let rms = mean_sq.sqrt();
    if rms > 0.0 {
        let scale = target / rms;
        v.iter_mut().for_each(|s| *s *= scale);
    }
}

fn sfx_default() -> Vec<f32> {
    let unit: Box<dyn AudioUnit> = Box::new(noise() >> bandpass_hz(1500.0, 1.5));
    let mut v = render(unit, 0.15);
    for (i, s) in v.iter_mut().enumerate() {
        *s *= (-(i as f32) / (0.025 * SR)).exp();
    }
    normalize_peak(&mut v, 0.6);
    v
}

fn ambience_default() -> Vec<f32> {
    let unit: Box<dyn AudioUnit> = Box::new(pink() >> lowpass_hz(900.0, 0.7));
    let rendered = render(unit, 2.25);
    let mut v = seamless(&rendered, 2.0);
    normalize_rms(&mut v, 0.1);
    v
}

fn music_default() -> Vec<f32> {
    let unit: Box<dyn AudioUnit> =
        Box::new(((saw_hz(220.0) + saw_hz(330.0)) * 0.5) >> lowpass_hz(1200.0, 0.7));
    let rendered = render(unit, 4.25);
    let mut v = seamless(&rendered, 4.0);
    normalize_rms(&mut v, 0.1);
    v
}

/// Renders the earcon for a `UI_*` key, or the default sound for the key's
/// [`SoundKind`] otherwise.
pub fn builtin(key: &str) -> Vec<f32> {
    match key {
        UI_OPEN => join(tone(523.0, 0.040, 0.4), tone(784.0, 0.060, 0.4)),
        UI_MOVE => tone(1200.0, 0.025, 0.35),
        UI_SELECT => join(tone(660.0, 0.045, 0.4), tone(990.0, 0.070, 0.4)),
        UI_BACK => join(tone(990.0, 0.045, 0.4), tone(660.0, 0.070, 0.4)),
        UI_TICK => tone(2000.0, 0.012, 0.3),
        UI_ERROR => join(tone(220.0, 0.090, 0.5), tone(165.0, 0.120, 0.5)),
        UI_NOTIFY => join(
            join(tone(784.0, 0.060, 0.4), tone(988.0, 0.060, 0.4)),
            tone(1319.0, 0.120, 0.4),
        ),
        _ => match kind(key) {
            SoundKind::Ui => tone(880.0, 0.060, 0.4),
            SoundKind::Sfx => sfx_default(),
            SoundKind::Ambience => ambience_default(),
            SoundKind::Music => music_default(),
            SoundKind::Other => tone(660.0, 0.040, 0.4),
        },
    }
}

/// The `builtin_wav` cache key: the matching `UI_*` constant itself, or the
/// kind's name for everything else -- one entry per kind, not per key.
fn cache_key(key: &str) -> &'static str {
    match key {
        UI_OPEN => UI_OPEN,
        UI_MOVE => UI_MOVE,
        UI_SELECT => UI_SELECT,
        UI_BACK => UI_BACK,
        UI_TICK => UI_TICK,
        UI_ERROR => UI_ERROR,
        UI_NOTIFY => UI_NOTIFY,
        _ => match kind(key) {
            SoundKind::Ui => "ui",
            SoundKind::Sfx => "sfx",
            SoundKind::Ambience => "amb",
            SoundKind::Music => "music",
            SoundKind::Other => "other",
        },
    }
}

type WavCache = HashMap<&'static str, Arc<[u8]>>;

static CACHE: OnceLock<Mutex<WavCache>> = OnceLock::new();

/// [`builtin`] rendered to a WAV, cached per `UI_*` key and per
/// [`SoundKind`] so a game with a thousand missing keys holds five defaults.
pub fn builtin_wav(key: &str) -> Arc<[u8]> {
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let cache_key = cache_key(key);
    let mut map = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(existing) = map.get(cache_key) {
        return existing.clone();
    }
    let bytes: Arc<[u8]> = Arc::from(crate::wav_bytes(&builtin(key), SAMPLE_RATE));
    map.insert(cache_key, bytes.clone());
    bytes
}
