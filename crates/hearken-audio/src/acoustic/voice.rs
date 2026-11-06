//! The game-thread half of a positioned sound's acoustics: turns a
//! [`SoundRender`] into the plain numbers its [`AcousticDsp`] reads.

use std::f32::consts::{FRAC_PI_4, SQRT_2};
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use hearken_acoustics::{SoundRender, BAND_CENTRES_HZ, BAND_COUNT, MAX_TAPS};

use super::dsp::{pack, word, AcousticDsp, VoiceShared, WORDS};
use super::mailbox::Mailbox;
use crate::spatial::SpatialConfig;

/// A reflection keeps its slot while its delay moves by less than this,
/// seconds, so a moving source glides its taps instead of swapping them.
const SAME_TAP_S: f32 = 0.003;

/// How a render is turned into numbers for the processor.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Post {
    pub doppler: bool,
    /// Keep the propagation delay; off, everything is shifted so the main
    /// arrival is at once and only the reflections lag behind it.
    pub delay: bool,
    pub reflections: bool,
    pub reverb: bool,
    /// The main arrival goes through the HRTF.
    pub hrtf: bool,
    pub ramp_ms: f32,
}

/// Constant-power pan with unity in each ear at the centre, as Kira's
/// panner: `pan` -1 (left) to 1 (right).
pub(crate) fn pan_gains(pan: f32) -> [f32; 2] {
    let pan = if pan.is_finite() {
        pan.clamp(-1.0, 1.0)
    } else {
        0.0
    };
    let t = (pan + 1.0) * FRAC_PI_4;
    [t.cos() * SQRT_2, t.sin() * SQRT_2]
}

/// The speaker cue for a direction in the listener's frame (x right, y
/// ahead, z up): its pan, and per-band gains for being behind (the same
/// gain and low-pass `place` gives a sound straight behind).
pub(crate) fn speaker_cue(dir: [f32; 3], cfg: &SpatialConfig) -> (f32, [f32; BAND_COUNT]) {
    let [x, y, _] = dir.map(|v| if v.is_finite() { v } else { 0.0 });
    let flat = x.hypot(y);
    let (pan, behind) = if flat > 1e-4 {
        ((x / flat).clamp(-1.0, 1.0), (-y / flat).max(0.0))
    } else {
        (0.0, 0.0)
    };
    let mut bands = [1.0; BAND_COUNT];
    if behind > 0.0 {
        let gain = 1.0 - (1.0 - cfg.behind_gain.clamp(0.0, 1.0)) * behind;
        let lp = cfg.behind_lowpass_hz.clamp(20.0, 20_000.0);
        let fc = 20_000.0 * (lp / 20_000.0).powf(behind);
        for (b, g) in bands.iter_mut().enumerate() {
            let f = BAND_CENTRES_HZ[b] as f32 / fc;
            *g = gain / (1.0 + f * f * f * f).sqrt();
        }
    }
    (pan, bands)
}

/// The game side of one sound's acoustics.
pub struct AcousticVoice {
    pub(crate) shared: Arc<VoiceShared>,
    /// The delay each tap slot last carried, seconds; negative is empty.
    slots: [f32; MAX_TAPS],
    words: [f32; WORDS],
    #[cfg(feature = "hrtf")]
    pub(crate) hrtf: Option<super::hrtf::HrtfLink>,
}

impl std::fmt::Debug for AcousticVoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcousticVoice")
            .field("engaged", &(self.words[word::ENGAGED] > 0.5))
            .finish()
    }
}

impl AcousticVoice {
    /// A voice that passes its sound through until a render is posted;
    /// its delay line holds up to `max_delay_s`.
    pub(crate) fn new(max_delay_s: f32) -> AcousticVoice {
        let words = [0.0; WORDS];
        AcousticVoice {
            shared: Arc::new(VoiceShared {
                mailbox: Mailbox::new(&words),
                track_gain: AtomicU64::new(pack(1.0, 0)),
                max_delay_s: if max_delay_s.is_finite() {
                    max_delay_s.clamp(0.01, 2.0)
                } else {
                    0.5
                },
            }),
            slots: [-1.0; MAX_TAPS],
            words,
            #[cfg(feature = "hrtf")]
            hrtf: None,
        }
    }

    /// A processor for the backend, starting where this voice stands.
    pub(crate) fn processor(&self) -> AcousticDsp {
        AcousticDsp::new(self.shared.clone())
    }

    /// Passes the sound through untouched (ramping out of the model).
    pub(crate) fn bypass(&mut self) {
        if self.words[word::ENGAGED] == 0.0 {
            return;
        }
        self.words[word::ENGAGED] = 0.0;
        self.shared.mailbox.write(&self.words);
    }

    /// Posts `render`; `cfg` gives the speaker cues for sounds behind.
    pub(crate) fn post(&mut self, r: &SoundRender, post: Post, cfg: &SpatialConfig) {
        let mut w = [0.0f32; WORDS];
        w[word::ENGAGED] = 1.0;
        w[word::DOPPLER] = if post.doppler { 1.0 } else { 0.0 };
        w[word::HRTF] = if post.hrtf { 1.0 } else { 0.0 };
        w[word::RAMP_MS] = post.ramp_ms;
        let shift = if post.delay { 0.0 } else { finite(r.delay_s) };
        w[word::DELAY] = (finite(r.delay_s) - shift).max(0.0);
        let (pan, behind) = speaker_cue(r.direction, cfg);
        for b in 0..BAND_COUNT {
            let cue = if post.hrtf { 1.0 } else { behind[b] };
            w[word::GAINS + b] = gain(r.gains[b]) * cue;
        }
        let [pl, pr] = pan_gains(pan);
        w[word::PAN] = pl;
        w[word::PAN + 1] = pr;
        w[word::SEND] = if post.reverb {
            gain(r.reverb_send)
        } else {
            0.0
        };

        let count = if post.reflections {
            (r.tap_count as usize).min(MAX_TAPS)
        } else {
            0
        };
        let taps = &r.taps[..count];
        // Keep each reflection in the slot that had a delay near its own.
        let mut slot_of = [usize::MAX; MAX_TAPS];
        let mut taken = [false; MAX_TAPS];
        for (t, tap) in taps.iter().enumerate() {
            let d = finite(tap.delay_s);
            let mut best = None;
            for s in 0..MAX_TAPS {
                let gap = (self.slots[s] - d).abs();
                if !taken[s]
                    && self.slots[s] >= 0.0
                    && gap < SAME_TAP_S
                    && best.is_none_or(|(_, g)| gap < g)
                {
                    best = Some((s, gap));
                }
            }
            if let Some((s, _)) = best {
                slot_of[t] = s;
                taken[s] = true;
            }
        }
        for t in 0..taps.len() {
            if slot_of[t] == usize::MAX {
                // An empty slot first, then any free one.
                let free = (0..MAX_TAPS)
                    .find(|&s| !taken[s] && self.slots[s] < 0.0)
                    .or_else(|| (0..MAX_TAPS).find(|&s| !taken[s]));
                if let Some(s) = free {
                    slot_of[t] = s;
                    taken[s] = true;
                }
            }
        }
        for s in 0..MAX_TAPS {
            let o = word::TAPS + s * word::TAP_WORDS;
            // An unused slot fades out where it was.
            w[o] = self.slots[s].max(0.0) - shift;
            w[o] = w[o].max(0.0);
        }
        let mut next = [-1.0f32; MAX_TAPS];
        for (t, tap) in taps.iter().enumerate() {
            let s = slot_of[t];
            if s == usize::MAX {
                continue;
            }
            let o = word::TAPS + s * word::TAP_WORDS;
            let d = finite(tap.delay_s);
            next[s] = d;
            w[o] = (d - shift).max(0.0);
            let (pan, behind) = speaker_cue(tap.direction, cfg);
            let g: [f32; BAND_COUNT] = std::array::from_fn(|b| gain(tap.gains[b]) * behind[b]);
            w[o + 1] = (g[0] + g[1]) / 2.0;
            w[o + 2] = (g[2] + g[3] + g[4]) / 3.0;
            w[o + 3] = (g[5] + g[6]) / 2.0;
            let [pl, pr] = pan_gains(pan);
            w[o + 4] = pl;
            w[o + 5] = pr;
        }
        self.slots = next;
        if w != self.words {
            self.words = w;
            self.shared.mailbox.write(&self.words);
        }
    }
}

fn finite(v: f32) -> f32 {
    if v.is_finite() {
        v.max(0.0)
    } else {
        0.0
    }
}

fn gain(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(0.0, 16.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centre_is_unity_and_sides_keep_power() {
        let [l, r] = pan_gains(0.0);
        assert!((l - 1.0).abs() < 1e-6 && (r - 1.0).abs() < 1e-6);
        let [l, r] = pan_gains(1.0);
        assert!(l.abs() < 1e-6 && (r - SQRT_2).abs() < 1e-6);
        let [l, r] = pan_gains(f32::NAN);
        assert_eq!(l, r);
    }

    #[test]
    fn behind_darkens_highs() {
        let cfg = SpatialConfig::default();
        let (pan, front) = speaker_cue([0.0, 1.0, 0.0], &cfg);
        assert_eq!(pan, 0.0);
        assert_eq!(front, [1.0; BAND_COUNT]);
        let (_, back) = speaker_cue([0.0, -1.0, 0.0], &cfg);
        assert!(back[0] > 0.75 && back[6] < 0.15, "{back:?}");
    }
}
