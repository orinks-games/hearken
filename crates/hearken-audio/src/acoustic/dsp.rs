//! One positioned sound through the acoustic model, on the audio thread:
//! its propagation delay, per-band gains, early reflections as panned taps,
//! a send to the shared reverb, and pan or HRTF output.
//!
//! The game thread works everything out ([`super::voice`]) and posts plain
//! numbers through a [`Mailbox`]; this side only filters and mixes. Nothing
//! here allocates, locks or blocks once made. Every change ramps.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use hearken_acoustics::{BAND_COUNT, MAX_TAPS};

use super::filters::{Biquad, Line, Shelf};
use super::mailbox::Mailbox;
use super::room::ReverbSend;

/// Where each number sits in a voice's mailbox.
pub(crate) mod word {
    use super::{BAND_COUNT, MAX_TAPS};
    /// 1 to run the model, 0 to pass the sound through untouched.
    pub const ENGAGED: usize = 0;
    /// 1 to glide the delay (Doppler), 0 to crossfade to a new delay.
    pub const DOPPLER: usize = 1;
    /// 1 to render the main arrival through the HRTF.
    pub const HRTF: usize = 2;
    /// The main arrival's delay, seconds.
    pub const DELAY: usize = 3;
    /// Its gain per band.
    pub const GAINS: usize = 4;
    /// Its left and right gains when panned.
    pub const PAN: usize = GAINS + BAND_COUNT;
    /// Send to the shared reverb.
    pub const SEND: usize = PAN + 2;
    /// How long changes ramp, ms.
    pub const RAMP_MS: usize = SEND + 1;
    pub const TAPS: usize = RAMP_MS + 1;
    /// Per tap: delay s, low, mid, high gains, left, right.
    pub const TAP_WORDS: usize = 6;
    pub const COUNT: usize = TAPS + TAP_WORDS * MAX_TAPS;
}

pub(crate) const WORDS: usize = word::COUNT;

/// The highest rate the delay line is sized for.
pub(crate) const MAX_RATE: f32 = 96_000.0;
/// Frames between parameter steps.
const BLOCK: u32 = 32;
/// Corners of the direct path's shelf cascade: between the octave bands.
const DIRECT_SHELF_HZ: [f32; BAND_COUNT - 1] = [177.0, 354.0, 707.0, 1414.0, 2828.0, 5657.0];
/// A tap's shelves: low bands below, high bands above.
const TAP_LOW_HZ: f32 = 350.0;
const TAP_HIGH_HZ: f32 = 2828.0;
/// Below this a path is treated as silent.
const SILENT: f32 = 1e-6;
/// How long a crossfade to a new delay takes, seconds.
const FADE_S: f32 = 0.012;
/// Jumps longer than this crossfade instead of gliding, seconds.
const MAX_GLIDE_S: f32 = 0.02;
/// Without Doppler, the delay is held until it is this far out, seconds,
/// then a crossfade takes it to the new one. Smaller steps would come back
/// as a pitch shift of their own.
const HOLD_S: f32 = 0.03;
/// Fastest glide: half a sample per sample, a pitch shift of at most 1.5.
const MAX_SLOPE: f32 = 0.5;

pub(crate) fn pack(value: f32, ms: u32) -> u64 {
    (u64::from(value.to_bits()) << 32) | u64::from(ms)
}

fn unpack(word: u64) -> (f32, u32) {
    (f32::from_bits((word >> 32) as u32), word as u32)
}

/// What the game thread and a voice's processor share.
pub(crate) struct VoiceShared {
    pub(crate) mailbox: Mailbox<WORDS>,
    /// The channel's own gain (volume, bus, fades), packed with its ramp,
    /// so the reverb send follows a fade the backend runs after the
    /// processor.
    pub(crate) track_gain: AtomicU64,
    /// Longest delay the line holds, seconds.
    pub(crate) max_delay_s: f32,
}

/// The channel gain a backend keeps in step with its own, for the reverb
/// send.
#[derive(Clone)]
pub struct AcousticGain(Arc<VoiceShared>);

impl AcousticGain {
    /// The channel's gain is moving to `gain` over `ms`.
    pub fn set(&self, gain: f32, ms: u32) {
        let gain = if gain.is_finite() { gain.max(0.0) } else { 0.0 };
        self.0.track_gain.store(pack(gain, ms), Ordering::Relaxed);
    }
}

/// A delay read head: glides for Doppler, or crossfades to a new delay.
#[derive(Clone, Copy)]
struct Head {
    cur: f32,
    step: f32,
    left: u32,
    from: f32,
    fade: f32,
}

impl Head {
    const fn new() -> Head {
        Head {
            cur: 0.0,
            step: 0.0,
            left: 0,
            from: 0.0,
            fade: 0.0,
        }
    }

    /// Moves toward `target` samples. `silent` paths jump there at once.
    fn retarget(&mut self, target: f32, glide: bool, ramp: u32, rate: f32, silent: bool) {
        let diff = target - self.cur;
        if silent {
            *self = Head {
                cur: target,
                ..Head::new()
            };
            return;
        }
        if glide && diff.abs() <= MAX_GLIDE_S * rate {
            if diff == 0.0 {
                self.left = 0;
                return;
            }
            let ramp = ramp.max(1) as f32;
            let need = (diff.abs() / MAX_SLOPE).ceil();
            let frames = ramp.max(need);
            self.step = diff / frames;
            self.left = frames as u32;
        } else if diff.abs() > HOLD_S * rate || (glide && diff != 0.0) {
            self.from = self.read_pos();
            self.cur = target;
            self.left = 0;
            self.fade = 1.0;
        }
    }

    fn read_pos(&self) -> f32 {
        if self.fade > 0.0 {
            // Mid-fade: start the next from whichever is louder.
            if self.fade > 0.5 {
                self.from
            } else {
                self.cur
            }
        } else {
            self.cur
        }
    }

    #[inline]
    fn read(&mut self, line: &Line, fade_step: f32) -> f32 {
        if self.left > 0 {
            self.cur += self.step;
            self.left -= 1;
        }
        let y = line.read(self.cur);
        if self.fade > 0.0 {
            let old = line.read(self.from);
            let y = y + (old - y) * self.fade;
            self.fade = (self.fade - fade_step).max(0.0);
            y
        } else {
            y
        }
    }
}

/// A value stepping in straight lines from block to block.
#[derive(Clone, Copy, Default)]
struct Glide {
    now: f32,
    step: f32,
}

impl Glide {
    fn aim(&mut self, target: f32) {
        self.step = (target - self.now) / BLOCK as f32;
    }

    #[inline]
    fn next(&mut self) -> f32 {
        self.now += self.step;
        self.now
    }

    fn settle(&mut self, target: f32) {
        self.now = target;
        self.step = 0.0;
    }
}

/// Lowest band level the shelf cascade shapes, relative to the loudest
/// band (-60 dB); quieter bands are held here.
const MIN_RELATIVE: f32 = 1e-3;

/// The direct path's overall gain and each band's level relative to it.
/// The shelves start from the lowest band, so the gain is that band's
/// level, but the levels are taken relative to the loudest band and floored
/// there. So a nearly silent low band cannot drag the other bands down.
fn direct_shape(bands: &[f32; BAND_COUNT]) -> (f32, [f32; BAND_COUNT]) {
    let top = bands.iter().fold(0.0f32, |m, &g| m.max(g));
    if !(top > 0.0 && top.is_finite()) {
        return (0.0, [1.0; BAND_COUNT]);
    }
    let rel = bands.map(|g| (g / top).max(MIN_RELATIVE));
    (top * rel[0], rel)
}

struct Direct {
    head: Head,
    /// Band gains as filtered now.
    bands: [f32; BAND_COUNT],
    gain: Glide,
    pan: [Glide; 2],
    shelves: [Biquad; BAND_COUNT - 1],
    shapes: [Shelf; BAND_COUNT - 1],
    on: bool,
}

struct TapPath {
    head: Head,
    low_mid_high: [f32; 3],
    gain: Glide,
    pan: [Glide; 2],
    low: Biquad,
    high: Biquad,
    on: bool,
}

/// A linear ramp toward a packed target.
struct Ramp {
    now: f32,
    target: f32,
    step: f32,
    left: u32,
    seen: u64,
}

impl Ramp {
    fn new(word: u64) -> Ramp {
        let (v, _) = unpack(word);
        Ramp {
            now: v,
            target: v,
            step: 0.0,
            left: 0,
            seen: word,
        }
    }

    fn poll(&mut self, word: u64, rate: f32) {
        if word == self.seen {
            return;
        }
        self.seen = word;
        let (target, ms) = unpack(word);
        let frames = (ms as f32 / 1000.0 * rate).round() as u32;
        self.target = target;
        if frames == 0 {
            self.now = target;
            self.left = 0;
        } else {
            self.step = (target - self.now) / frames as f32;
            self.left = frames;
        }
    }

    fn advance(&mut self, frames: u32) -> f32 {
        if self.left > 0 {
            if frames >= self.left {
                self.left = 0;
                self.now = self.target;
            } else {
                self.left -= frames;
                self.now += self.step * frames as f32;
            }
        }
        self.now
    }
}

/// One sound's acoustic processor, made on the game thread and run on the
/// audio thread: call [`begin`](Self::begin) once a block, then
/// [`frame`](Self::frame) for each frame while `begin` says so.
pub struct AcousticDsp {
    shared: Arc<VoiceShared>,
    seen: u32,
    p: [f32; WORDS],
    rate: f32,
    line: Line,
    direct: Direct,
    taps: [TapPath; MAX_TAPS],
    engaged: f32,
    engaged_step: f32,
    send: Glide,
    track: Ramp,
    send_bus: Option<ReverbSend>,
    step_in: u32,
    fade_step: f32,
    /// Smoothing of band gains per block, 0..1.
    smooth: f32,
    /// Whether a block has run yet: a sound rendered before it starts
    /// begins in the model, not fading into it.
    started: bool,
    #[cfg(feature = "hrtf")]
    pub(crate) hrtf: Option<super::hrtf::HrtfDsp>,
}

impl AcousticDsp {
    pub(crate) fn new(shared: Arc<VoiceShared>) -> AcousticDsp {
        let len = (shared.max_delay_s * MAX_RATE).ceil() as usize + 8;
        let track = Ramp::new(shared.track_gain.load(Ordering::Relaxed));
        let shapes = DIRECT_SHELF_HZ.map(|hz| Shelf::new(hz, 48_000.0, true));
        AcousticDsp {
            seen: 0,
            p: [0.0; WORDS],
            rate: 0.0,
            line: Line::new(len),
            direct: Direct {
                head: Head::new(),
                bands: [0.0; BAND_COUNT],
                gain: Glide::default(),
                pan: [Glide::default(); 2],
                shelves: [Biquad::default(); BAND_COUNT - 1],
                shapes,
                on: false,
            },
            taps: std::array::from_fn(|_| TapPath {
                head: Head::new(),
                low_mid_high: [0.0; 3],
                gain: Glide::default(),
                pan: [Glide::default(); 2],
                low: Biquad::default(),
                high: Biquad::default(),
                on: false,
            }),
            engaged: 0.0,
            engaged_step: 0.0,
            send: Glide::default(),
            track,
            send_bus: None,
            step_in: 0,
            fade_step: 0.0,
            smooth: 1.0,
            started: false,
            #[cfg(feature = "hrtf")]
            hrtf: None,
            shared,
        }
    }

    /// Where the reverb send goes. A backend with a shared reverb attaches
    /// its bus before it starts the sound.
    pub fn attach_send(&mut self, send: ReverbSend) {
        self.send_bus = Some(send);
    }

    /// A handle for keeping the reverb send in step with the channel's gain.
    pub fn gain_control(&self) -> AcousticGain {
        AcousticGain(self.shared.clone())
    }

    fn tune_rate(&mut self, rate: f32) {
        self.rate = rate;
        self.direct.shapes = DIRECT_SHELF_HZ.map(|hz| Shelf::new(hz, rate, true));
        self.fade_step = 1.0 / (FADE_S * rate);
        // Force a fresh read of the mailbox and a fresh step.
        self.seen = 0;
        self.step_in = 0;
    }

    /// Takes up the latest settings for a block at `sample_rate`. `false`:
    /// the model is off and settled, so leave the block untouched (the
    /// delay line still needs [`idle`](Self::idle) for each frame).
    pub fn begin(&mut self, sample_rate: f32) -> bool {
        if !(sample_rate.is_finite() && sample_rate > 0.0) {
            return false;
        }
        if sample_rate != self.rate {
            self.tune_rate(sample_rate);
        }
        self.track
            .poll(self.shared.track_gain.load(Ordering::Relaxed), sample_rate);
        if self.shared.mailbox.read(&mut self.p, &mut self.seen) {
            self.step_in = 0;
        }
        let want = self.p[word::ENGAGED] > 0.5;
        if !self.started {
            self.started = true;
            if want {
                self.settle();
                self.engaged = 1.0;
            }
        }
        want || self.engaged > 0.0
    }

    /// Keeps the delay line fed while the model is off, so it has the
    /// sound's past when it comes on.
    #[inline]
    pub fn idle(&mut self, left: f32, right: f32) {
        self.line.write(0.5 * (left + right));
    }

    /// Settles every path on the posted values at once: for the first
    /// block, so a sound starts where it is instead of ramping in.
    fn settle(&mut self) {
        let rate = self.rate;
        let p = self.p;
        self.direct.head = Head {
            cur: (p[word::DELAY] * rate).clamp(
                0.0,
                self.line.max_delay().min(self.shared.max_delay_s * rate),
            ),
            ..Head::new()
        };
        self.direct
            .bands
            .copy_from_slice(&p[word::GAINS..word::GAINS + BAND_COUNT]);
        self.direct.gain.settle(direct_shape(&self.direct.bands).0);
        for s in 0..2 {
            self.direct.pan[s].settle(p[word::PAN + s]);
        }
        for (i, tap) in self.taps.iter_mut().enumerate() {
            let o = word::TAPS + i * word::TAP_WORDS;
            tap.head = Head {
                cur: (p[o] * rate).max(0.0),
                ..Head::new()
            };
            tap.low_mid_high = [p[o + 1], p[o + 2], p[o + 3]];
            tap.gain.settle(p[o + 2]);
            tap.pan[0].settle(p[o + 4]);
            tap.pan[1].settle(p[o + 5]);
        }
        self.send.settle(p[word::SEND]);
    }

    /// Once every [`BLOCK`] frames: aim every glide at the posted values.
    fn step(&mut self) {
        let rate = self.rate;
        let p = self.p;
        let engaged = p[word::ENGAGED] > 0.5;
        let target = if engaged { 1.0 } else { 0.0 };
        if self.engaged == 0.0 && engaged {
            self.settle();
        }
        self.engaged_step = if target > self.engaged {
            self.fade_step
        } else if target < self.engaged {
            -self.fade_step
        } else {
            0.0
        };
        let ramp_ms = if p[word::RAMP_MS].is_finite() {
            p[word::RAMP_MS].clamp(0.0, 1000.0)
        } else {
            50.0
        };
        let ramp = (ramp_ms / 1000.0 * rate) as u32;
        self.smooth = if ramp_ms <= 0.0 {
            1.0
        } else {
            1.0 - (-(BLOCK as f32) / (ramp_ms / 1000.0 * rate / 3.0)).exp()
        };
        let glide = p[word::DOPPLER] > 0.5;
        let a = self.smooth;
        // The line is sized for the highest rate; hold the reach to the
        // asked seconds so it is the same at every rate.
        let max_delay = self.line.max_delay().min(self.shared.max_delay_s * rate);
        let delay = |s: f32| {
            if s.is_finite() {
                (s * rate).clamp(0.0, max_delay)
            } else {
                0.0
            }
        };

        // The main arrival.
        let hrtf_latency = self.hrtf_latency();
        let d = &mut self.direct;
        let silent = d.bands.iter().all(|&g| g < SILENT);
        d.head.retarget(
            (delay(p[word::DELAY]) - hrtf_latency).max(0.0),
            glide,
            ramp,
            rate,
            silent,
        );
        for b in 0..BAND_COUNT {
            let t = p[word::GAINS + b];
            d.bands[b] += (t - d.bands[b]) * a;
            if (d.bands[b] - t).abs() < 1e-7 {
                d.bands[b] = t;
            }
        }
        d.on = d.bands.iter().any(|&g| g >= SILENT);
        let (gain, rel) = direct_shape(&d.bands);
        if d.on {
            for k in 0..BAND_COUNT - 1 {
                d.shelves[k].c = d.shapes[k].coeffs(rel[k + 1] / rel[k]);
                d.shelves[k].flush();
            }
        } else {
            for s in &mut d.shelves {
                s.clear();
            }
        }
        d.gain.aim(gain);
        d.pan[0].aim(p[word::PAN]);
        d.pan[1].aim(p[word::PAN + 1]);

        // The reflections.
        let low = Shelf::new(TAP_LOW_HZ, rate, false);
        let high = Shelf::new(TAP_HIGH_HZ, rate, true);
        for (i, tap) in self.taps.iter_mut().enumerate() {
            let o = word::TAPS + i * word::TAP_WORDS;
            let silent = tap.low_mid_high.iter().all(|&g| g < SILENT);
            tap.head.retarget(delay(p[o]), glide, ramp, rate, silent);
            for k in 0..3 {
                let t = p[o + 1 + k];
                tap.low_mid_high[k] += (t - tap.low_mid_high[k]) * a;
                if (tap.low_mid_high[k] - t).abs() < 1e-7 {
                    tap.low_mid_high[k] = t;
                }
            }
            tap.on = tap.low_mid_high.iter().any(|&g| g >= SILENT);
            if tap.on {
                let [l, m, h] = tap.low_mid_high.map(|g| g.max(1e-6));
                tap.low.c = low.coeffs(l / m);
                tap.high.c = high.coeffs(h / m);
                tap.low.flush();
                tap.high.flush();
            } else {
                tap.low.clear();
                tap.high.clear();
            }
            tap.gain.aim(tap.low_mid_high[1]);
            tap.pan[0].aim(p[o + 4]);
            tap.pan[1].aim(p[o + 5]);
        }
        self.send.aim(p[word::SEND]);
    }

    #[cfg(feature = "hrtf")]
    fn hrtf_latency(&self) -> f32 {
        match &self.hrtf {
            Some(h) if self.p[word::HRTF] > 0.5 && h.rate() == self.rate => h.latency() as f32,
            _ => 0.0,
        }
    }

    #[cfg(not(feature = "hrtf"))]
    fn hrtf_latency(&self) -> f32 {
        0.0
    }

    /// One stereo frame through the model. `index` is the frame's place in
    /// the block, for the reverb send.
    pub fn frame(&mut self, index: usize, left: f32, right: f32) -> (f32, f32) {
        if self.step_in == 0 {
            self.step_in = BLOCK;
            self.step();
        }
        self.step_in -= 1;
        let x = 0.5 * (left + right);
        self.line.write(x);
        let fade = self.fade_step;
        let (mut l, mut r) = (0.0, 0.0);

        let d = &mut self.direct;
        let g = d.gain.next();
        let pl = d.pan[0].next();
        let pr = d.pan[1].next();
        let mut arrival = 0.0;
        if d.on {
            let mut y = d.head.read(&self.line, fade);
            arrival = y;
            for s in &mut d.shelves {
                y = s.run(y);
            }
            y *= g;
            #[cfg(feature = "hrtf")]
            let panned = match &mut self.hrtf {
                Some(h) if self.p[word::HRTF] > 0.5 && h.rate() == self.rate => {
                    let (hl, hr) = h.push(y);
                    l += hl;
                    r += hr;
                    false
                }
                _ => true,
            };
            #[cfg(not(feature = "hrtf"))]
            let panned = true;
            if panned {
                l += y * pl;
                r += y * pr;
            }
        } else {
            #[cfg(feature = "hrtf")]
            if let Some(h) = &mut self.hrtf {
                let (hl, hr) = h.push(0.0);
                l += hl;
                r += hr;
            }
        }

        for tap in &mut self.taps {
            let g = tap.gain.next();
            let pl = tap.pan[0].next();
            let pr = tap.pan[1].next();
            if !tap.on {
                continue;
            }
            let y = tap.head.read(&self.line, fade);
            let y = tap.high.run(tap.low.run(y)) * g;
            l += y * pl;
            r += y * pr;
        }

        let send = self.send.next();
        let track = self.track.advance(1);
        if let Some(bus) = &self.send_bus {
            let s = arrival * send * track * self.engaged;
            if s != 0.0 {
                bus.add(index, s);
            }
        }

        self.engaged = (self.engaged + self.engaged_step).clamp(0.0, 1.0);
        let e = self.engaged;
        if e >= 1.0 {
            (l, r)
        } else {
            (left + (l - left) * e, right + (r - right) * e)
        }
    }

    /// Runs a whole block in place, as a backend would.
    pub fn process(&mut self, frames: &mut [[f32; 2]], sample_rate: f32) {
        if !self.begin(sample_rate) {
            for f in frames.iter() {
                self.idle(f[0], f[1]);
            }
            return;
        }
        for (i, f) in frames.iter_mut().enumerate() {
            let (l, r) = self.frame(i, f[0], f[1]);
            *f = [l, r];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::voice::{AcousticVoice, Post};
    use super::{direct_shape, BAND_COUNT, MIN_RELATIVE};
    use crate::spatial::SpatialConfig;
    use hearken_acoustics::{SoundRender, Tap};

    const POST: Post = Post {
        doppler: true,
        delay: true,
        reflections: true,
        reverb: true,
        hrtf: false,
        ramp_ms: 50.0,
    };

    #[test]
    fn garbage_renders_stay_finite_and_bounded() {
        let mut voice = AcousticVoice::new(0.5);
        let mut dsp = voice.processor();
        let cfg = SpatialConfig::default();
        let nasty = [f32::NAN, f32::INFINITY, -1e30, 1e30, 0.0, -0.0, 1e-40];
        let mut block = vec![[0.0f32; 2]; 256];
        for (k, &v) in nasty.iter().cycle().take(60).enumerate() {
            let mut r = SoundRender {
                delay_s: v,
                distance_m: v,
                gains: [v; 7],
                direction: [v, -v, v],
                reverb_send: v,
                tap_count: 255,
                ..SoundRender::default()
            };
            r.taps = [Tap {
                delay_s: v,
                gains: [v; 7],
                direction: [v; 3],
            }; 8];
            voice.post(&r, POST, &cfg);
            for (i, f) in block.iter_mut().enumerate() {
                let x = if (i + k) % 37 == 0 { 1.0 } else { 0.0 };
                *f = [x, x];
            }
            dsp.process(&mut block, 48_000.0);
            for f in &block {
                assert!(f[0].is_finite() && f[1].is_finite(), "{v}: {f:?}");
                assert!(f[0].abs() < 1e4 && f[1].abs() < 1e4, "{v}: {f:?}");
            }
        }
    }

    #[test]
    fn bypass_passes_the_sound_through() {
        let voice = AcousticVoice::new(0.5);
        let mut dsp = voice.processor();
        let input: Vec<[f32; 2]> = (0..512).map(|i| [i as f32 * 0.001, -0.3]).collect();
        let mut out = input.clone();
        dsp.process(&mut out, 44_100.0);
        assert_eq!(out, input);
    }

    #[test]
    fn direct_shape_is_relative_to_the_loudest_band() {
        let (g, rel) = direct_shape(&[1e-9, 0.5, 0.5, 0.5, 0.5, 0.25, 0.0]);
        assert_eq!(rel[1], 1.0);
        assert_eq!(rel[0], MIN_RELATIVE);
        assert_eq!(rel[6], MIN_RELATIVE);
        assert_eq!(rel[5], 0.5);
        assert_eq!(g, 0.5 * MIN_RELATIVE);
        assert_eq!(direct_shape(&[0.0; BAND_COUNT]).0, 0.0);
        assert_eq!(direct_shape(&[f32::NAN; BAND_COUNT]).0, 0.0);
    }

    #[test]
    fn delay_and_gain_land_where_posted() {
        let mut voice = AcousticVoice::new(0.5);
        let r = SoundRender {
            delay_s: 0.01,
            gains: [0.5; 7],
            direction: [0.0, 1.0, 0.0],
            ..SoundRender::default()
        };
        voice.post(&r, POST, &SpatialConfig::default());
        let mut dsp = voice.processor();
        let mut block = vec![[0.0f32; 2]; 1024];
        block[0] = [1.0, 1.0];
        dsp.process(&mut block, 48_000.0);
        let peak = block
            .iter()
            .enumerate()
            .max_by(|a, b| a.1[0].abs().total_cmp(&b.1[0].abs()))
            .unwrap();
        assert_eq!(peak.0, 480);
        assert!((peak.1[0] - 0.5).abs() < 0.02, "{:?}", peak.1);
    }
}
