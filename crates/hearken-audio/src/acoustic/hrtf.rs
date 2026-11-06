//! HRTF rendering for headphones, through the `sofar` crate: a SOFA set
//! (the bundled MIT KEMAR one or the game's own), and per sound a
//! partitioned convolution the game thread re-aims as the sound moves.

use std::sync::Arc;

use sofar::reader::{Filter, OpenOptions, Sofar};
use sofar::render::{FilterPublisher, FilterTransition, RealtimeRenderer, Renderer, RendererPlan};

/// The bundled set: MIT KEMAR, normal pinna (see `hrtf/README.md`).
pub const BUILTIN_SOFA: &[u8] = include_bytes!("../../hrtf/mit_kemar_normal_pinna.sofa");

/// Largest SOFA file accepted, bytes.
const MAX_SOFA_BYTES: usize = 64 * 1024 * 1024;
/// Longest filter accepted, taps.
const MAX_FILTER_LEN: usize = 8192;
/// Most filter samples after resampling (64 MB of `f32`).
const MAX_SOFA_VALUES: u64 = 16 * 1024 * 1024;
/// Largest rate over the common divisor of the file's rate and the
/// output's (44.1 kHz to 48 kHz is 147 and 160).
const MAX_RATE_STEP: u64 = 4096;
/// Convolution block, frames: also the HRTF path's latency.
const PARTITION: usize = 64;
/// Re-aim only when the direction moves more than this, degrees.
const REAIM_DEG: f32 = 2.0;

/// Where an HRTF set comes from.
#[derive(Clone, Debug, Default)]
pub enum HrtfSource {
    /// The bundled MIT KEMAR set.
    #[default]
    Builtin,
    /// A SOFA file's bytes, from the game.
    Sofa(Arc<[u8]>),
}

/// A loaded HRTF set at one sample rate.
pub struct HrtfSet {
    sofa: Sofar,
    plan: RendererPlan,
    rate: u32,
}

impl std::fmt::Debug for HrtfSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HrtfSet")
            .field("rate", &self.rate)
            .field("filter_len", &self.sofa.filter_len())
            .finish()
    }
}

impl HrtfSet {
    /// Loads `source` resampled to `rate`. A file that is too big, will
    /// not parse, or makes the reader panic is an error, never a crash.
    pub fn load(source: &HrtfSource, rate: u32) -> Result<HrtfSet, String> {
        let bytes: &[u8] = match source {
            HrtfSource::Builtin => BUILTIN_SOFA,
            HrtfSource::Sofa(b) => b,
        };
        if bytes.len() > MAX_SOFA_BYTES {
            return Err(format!("SOFA file over {MAX_SOFA_BYTES} bytes"));
        }
        if !(8_000..=384_000).contains(&rate) {
            return Err(format!("unusable sample rate {rate}"));
        }
        // The reader trusts the file's own rate and sizes when it resamples,
        // and a failed allocation aborts. So read the file once as it is
        // (a NaN target rate never differs by more than 0.1 Hz, so nothing
        // is resampled) and check it before resampling.
        let raw = open(bytes, f32::NAN, false)?;
        check_resample(&raw, rate)?;
        drop(raw);
        let sofa = open(bytes, rate as f32, true)?;
        if sofa.sample_rate() != rate as f32 {
            return Err("SOFA file not resampled".into());
        }
        let len = sofa.filter_len();
        if len == 0 || len > MAX_FILTER_LEN {
            return Err(format!("SOFA filter length {len} out of range"));
        }
        let plan = RendererPlan::builder(len)
            .with_sample_rate(rate as f32)
            .with_partition_len(PARTITION)
            .build()
            .map_err(|e| e.to_string())?;
        let set = HrtfSet { sofa, plan, rate };
        // A filter straight ahead must come out finite and not silent.
        let mut f = Filter::new(len);
        set.lookup([0.0, 1.0, 0.0], &mut f)?;
        Ok(set)
    }

    pub fn sample_rate(&self) -> u32 {
        self.rate
    }

    /// The filter for a direction in the listener's frame (x right, y
    /// ahead, z up).
    fn lookup(&self, dir: [f32; 3], f: &mut Filter) -> Result<(), String> {
        let [x, y, z] = dir;
        // SOFA: x ahead, y left, z up.
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.sofa.filter(y, -x, z, f);
        }));
        if r.is_err() {
            return Err("SOFA lookup failed".into());
        }
        let ok = f.left.iter().chain(f.right.iter()).all(|v| v.is_finite());
        let loud = f.left.iter().chain(f.right.iter()).any(|v| *v != 0.0);
        if ok && loud {
            Ok(())
        } else {
            Err("SOFA filter empty or not finite".into())
        }
    }

    /// One sound's pair: the game-side link and the audio-side renderer.
    pub(crate) fn voice(&self) -> Result<(HrtfLink, HrtfDsp), String> {
        let renderer = Renderer::builder(&self.plan)
            .with_filter_transition(FilterTransition::crossfade(PARTITION * 2))
            .build()
            .map_err(|e| e.to_string())?;
        let (publisher, rt) = renderer.into_realtime();
        let mut link = HrtfLink {
            publisher,
            filter: Filter::new(self.sofa.filter_len()),
            aimed: None,
        };
        link.aim(self, [0.0, 1.0, 0.0]);
        let dsp = HrtfDsp {
            renderer: rt,
            input: vec![0.0; PARTITION].into_boxed_slice(),
            left: vec![0.0; PARTITION].into_boxed_slice(),
            right: vec![0.0; PARTITION].into_boxed_slice(),
            at: 0,
            rate: self.rate as f32,
        };
        Ok((link, dsp))
    }
}

fn open(bytes: &[u8], rate: f32, normalized: bool) -> Result<Sofar, String> {
    let opened = std::panic::catch_unwind(|| {
        OpenOptions::new()
            .sample_rate(rate)
            .normalized(normalized)
            .open_data(bytes)
    });
    match opened {
        Ok(Ok(s)) => Ok(s),
        Ok(Err(e)) => Err(format!("SOFA file not read: {e}")),
        Err(_) => Err("SOFA file not read: the reader failed".into()),
    }
}

/// Refuses a file whose own rate, or whose size after resampling to
/// `rate`, the reader cannot handle safely.
fn check_resample(raw: &Sofar, rate: u32) -> Result<(), String> {
    let from = raw.sample_rate();
    if !(from.is_finite() && from.fract() == 0.0 && (8_000.0..=384_000.0).contains(&from)) {
        return Err(format!("SOFA sample rate {from} out of range"));
    }
    let from = from as u64;
    let to = u64::from(rate);
    // The resampler's FFT is as long as the rates over their common
    // divisor; coprime rates would make it huge and slow.
    let g = gcd(from, to);
    if from / g > MAX_RATE_STEP || to / g > MAX_RATE_STEP {
        return Err(format!(
            "SOFA sample rate {from} cannot be resampled to {to}"
        ));
    }
    let d = raw.hrtf().dimensions();
    let (m, r, n) = (u64::from(d.m), u64::from(d.r), u64::from(d.n));
    let new_n = (n * to).div_ceil(from);
    if m == 0 || r == 0 || n == 0 || new_n > MAX_FILTER_LEN as u64 {
        return Err(format!("SOFA filter length {new_n} out of range"));
    }
    if m.saturating_mul(r).saturating_mul(new_n) > MAX_SOFA_VALUES {
        return Err("SOFA file has too many filters".into());
    }
    Ok(())
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}

/// The game side of one sound's HRTF: publishes a new filter as the sound
/// moves round the listener.
pub(crate) struct HrtfLink {
    publisher: FilterPublisher,
    filter: Filter,
    aimed: Option<[f32; 3]>,
}

impl HrtfLink {
    /// Aims at `dir` (listener frame), when it moved enough to matter.
    pub(crate) fn aim(&mut self, set: &HrtfSet, dir: [f32; 3]) {
        let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
        if !(len > 1e-6 && len.is_finite()) {
            return;
        }
        let dir = dir.map(|v| v / len);
        if let Some(prev) = self.aimed {
            let dot = prev[0] * dir[0] + prev[1] * dir[1] + prev[2] * dir[2];
            if dot > REAIM_DEG.to_radians().cos() {
                self.publisher.reclaim();
                return;
            }
        }
        if set.lookup(dir, &mut self.filter).is_ok()
            && self.publisher.publish_filter(&self.filter).is_ok()
        {
            self.aimed = Some(dir);
        }
    }
}

/// The audio side of one sound's HRTF: a block-sized buffer round the
/// convolution, so any host block size works. Adds [`PARTITION`] frames of
/// latency, which the main arrival's delay takes back.
pub(crate) struct HrtfDsp {
    renderer: RealtimeRenderer,
    input: Box<[f32]>,
    left: Box<[f32]>,
    right: Box<[f32]>,
    at: usize,
    rate: f32,
}

impl HrtfDsp {
    pub(crate) fn rate(&self) -> f32 {
        self.rate
    }

    pub(crate) fn latency(&self) -> usize {
        PARTITION
    }

    #[inline]
    pub(crate) fn push(&mut self, x: f32) -> (f32, f32) {
        let out = (self.left[self.at], self.right[self.at]);
        self.input[self.at] = x;
        self.at += 1;
        if self.at == PARTITION {
            self.at = 0;
            if self
                .renderer
                .process_block(&self.input, &mut self.left, &mut self.right)
                .is_err()
            {
                self.left.fill(0.0);
                self.right.fill(0.0);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_set_loads_and_lateralises() {
        let set = HrtfSet::load(&HrtfSource::Builtin, 48_000).unwrap();
        let (mut link, mut dsp) = set.voice().unwrap();
        link.aim(&set, [1.0, 0.0, 0.0]);
        let (mut el, mut er) = (0.0f64, 0.0f64);
        let mut noise = super::super::filters::Noise::new(7);
        for _ in 0..48_000 {
            let (l, r) = dsp.push(noise.next() * 0.3);
            el += f64::from(l * l);
            er += f64::from(r * r);
        }
        // A sound on the right is far louder in the right ear.
        assert!(er > el * 4.0, "left {el} right {er}");
    }

    #[test]
    fn garbage_sofa_is_an_error() {
        let junk: Arc<[u8]> = vec![0x89u8; 4096].into();
        assert!(HrtfSet::load(&HrtfSource::Sofa(junk), 48_000).is_err());
        let half: Arc<[u8]> = BUILTIN_SOFA[..BUILTIN_SOFA.len() / 3].into();
        assert!(HrtfSet::load(&HrtfSource::Sofa(half), 48_000).is_err());
        assert!(HrtfSet::load(&HrtfSource::Builtin, 0).is_err());
        // The bundled set is 44.1 kHz: a rate with no common step is
        // refused, common ones resample.
        assert!(HrtfSet::load(&HrtfSource::Builtin, 44_101).is_err());
        assert!(HrtfSet::load(&HrtfSource::Builtin, 96_000).is_ok());
    }
}
