//! The shared reverb every acoustic sound sends to: one feedback delay
//! network tuned from the room the listener stands in.
//!
//! Sounds add their send into a [`ReverbSend`] bus on the audio thread; the
//! backend runs one [`RoomReverbDsp`] after every sound has played its
//! block, which reads the bus, clears it and adds the tail to the mix.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use hearken_acoustics::{RoomRender, BAND_COUNT};

use super::filters::{Biquad, Line, Shelf};
use super::mailbox::Mailbox;

/// Most frames one block can carry through the send bus.
pub(crate) const SEND_FRAMES: usize = 4096;

/// Words in the room's mailbox: on, RT60 per band, wet per band, pre-delay.
const ROOM_WORDS: usize = 2 + 2 * BAND_COUNT;
const ON: usize = 0;
const RT60: usize = 1;
const WET: usize = 1 + BAND_COUNT;
const PREDELAY: usize = 1 + 2 * BAND_COUNT;

/// The network's lines, in ms: lengths that share no factor.
const LINES_MS: [f32; 8] = [29.7, 37.1, 41.1, 43.7, 53.3, 59.9, 67.7, 73.1];
/// Longest pre-delay, seconds.
const MAX_PREDELAY_S: f32 = 0.2;
/// The highest rate the lines are sized for; above it, delays are cut short.
const MAX_RATE: f32 = 96_000.0;
/// Frames between re-tunes.
const BLOCK: u32 = 32;
/// How long the network must be silent, in and out, before it sleeps:
/// longer than the pre-delay and the longest line together.
const SLEEP_S: f32 = 0.5;
/// Shelf corners splitting low, mid and high.
const LOW_HZ: f32 = 300.0;
const HIGH_HZ: f32 = 3_000.0;

struct Inner {
    bus: Box<[AtomicU32]>,
    room: Mailbox<ROOM_WORDS>,
}

/// The shared reverb's input bus and room settings. Cloning shares them.
#[derive(Clone)]
pub struct ReverbSend(Arc<Inner>);

impl std::fmt::Debug for ReverbSend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReverbSend")
    }
}

impl Default for ReverbSend {
    fn default() -> ReverbSend {
        ReverbSend::new()
    }
}

impl ReverbSend {
    pub fn new() -> ReverbSend {
        ReverbSend(Arc::new(Inner {
            bus: (0..SEND_FRAMES).map(|_| AtomicU32::new(0)).collect(),
            room: Mailbox::new(&[0.0; ROOM_WORDS]),
        }))
    }

    /// Sets the room from the game thread; `None` turns the tail off (it
    /// rings out first).
    pub fn set_room(&self, room: Option<&RoomRender>) {
        let mut w = [0.0f32; ROOM_WORDS];
        if let Some(r) = room {
            w[ON] = 1.0;
            for b in 0..BAND_COUNT {
                w[RT60 + b] = finite_or(r.rt60_s[b], 0.0).clamp(0.0, 30.0);
                w[WET + b] = finite_or(r.wet_gains[b], 0.0).clamp(0.0, 4.0);
            }
            w[PREDELAY] = finite_or(r.predelay_s, 0.0).clamp(0.0, MAX_PREDELAY_S);
        }
        self.0.room.write(&w);
    }

    /// Adds `x` to frame `index` of the bus. Audio thread only: every
    /// sound and the reverb run on that one thread, so a plain
    /// load-and-store cannot race.
    #[inline]
    pub(crate) fn add(&self, index: usize, x: f32) {
        if let Some(slot) = self.0.bus.get(index) {
            let v = f32::from_bits(slot.load(Ordering::Relaxed)) + x;
            slot.store(v.to_bits(), Ordering::Relaxed);
        }
    }

    #[inline]
    fn take(&self, index: usize) -> f32 {
        match self.0.bus.get(index) {
            Some(slot) => f32::from_bits(slot.swap(0, Ordering::Relaxed)),
            None => 0.0,
        }
    }

    /// The reverb itself, for the backend to run once per block.
    pub fn processor(&self) -> RoomReverbDsp {
        RoomReverbDsp::new(self.clone())
    }
}

fn finite_or(v: f32, or: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        or
    }
}

/// Mean of `v[range]`.
fn mean(v: &[f32]) -> f32 {
    v.iter().sum::<f32>() / v.len().max(1) as f32
}

/// Low, mid and high from seven octave bands.
fn three(v: &[f32]) -> [f32; 3] {
    [mean(&v[0..2]), mean(&v[2..5]), mean(&v[5..7])]
}

/// The shared reverb on the audio thread. Call [`begin`](Self::begin) once
/// a block, then [`frame`](Self::frame) for each of its frames.
pub struct RoomReverbDsp {
    send: ReverbSend,
    seen: u32,
    target: [f32; ROOM_WORDS],
    /// RT60 low, mid, high as tuned now; smoothed toward the target.
    rt: [f32; 3],
    /// Output tilt low, mid, high.
    tilt: [f32; 3],
    on: f32,
    predelay: f32,
    rate: f32,
    pre: Line,
    lines: [Line; 8],
    len: [usize; 8],
    gain: [f32; 8],
    lows: [Biquad; 8],
    highs: [Biquad; 8],
    low_shelf: Shelf,
    high_shelf: Shelf,
    out_eq: [[Biquad; 2]; 2],
    level: f32,
    retune_in: u32,
    /// Blocks since anything went in or came out; the network sleeps
    /// after [`SLEEP_S`] of that.
    quiet: u32,
    energy: f32,
}

impl RoomReverbDsp {
    fn new(send: ReverbSend) -> RoomReverbDsp {
        let size = |ms: f32| (ms / 1000.0 * MAX_RATE).ceil() as usize + 4;
        RoomReverbDsp {
            send,
            seen: 0,
            target: [0.0; ROOM_WORDS],
            rt: [0.0; 3],
            tilt: [1.0; 3],
            on: 0.0,
            predelay: 0.0,
            rate: 0.0,
            pre: Line::new(size(MAX_PREDELAY_S * 1000.0)),
            lines: LINES_MS.map(|ms| Line::new(size(ms))),
            len: [1; 8],
            gain: [0.0; 8],
            lows: [Biquad::default(); 8],
            highs: [Biquad::default(); 8],
            low_shelf: Shelf::new(LOW_HZ, 48_000.0, false),
            high_shelf: Shelf::new(HIGH_HZ, 48_000.0, true),
            out_eq: [[Biquad::default(); 2]; 2],
            level: 0.0,
            retune_in: 0,
            quiet: u32::MAX,
            energy: 0.0,
        }
    }

    fn tune_rate(&mut self, rate: f32) {
        self.rate = rate;
        for (len, (ms, line)) in self.len.iter_mut().zip(LINES_MS.iter().zip(&self.lines)) {
            *len = ((ms / 1000.0 * rate).round() as usize).clamp(1, line.max_delay() as usize);
        }
        self.low_shelf = Shelf::new(LOW_HZ, rate, false);
        self.high_shelf = Shelf::new(HIGH_HZ, rate, true);
        self.retune();
    }

    /// Line gains and filters for the RT60s now, and the output level and
    /// tilt.
    fn retune(&mut self) {
        let rate = self.rate;
        let [tl, tm, th] = self.rt.map(|t| t.max(0.01));
        let mut mean_len = 0.0;
        for i in 0..8 {
            let secs = self.len[i] as f32 / rate;
            let g = |t: f32| 10f32.powf(-3.0 * secs / t);
            let (gl, gm, gh) = (g(tl), g(tm), g(th));
            self.gain[i] = gm;
            self.lows[i].c = self.low_shelf.coeffs(gl / gm.max(1e-6));
            self.highs[i].c = self.high_shelf.coeffs(gh / gm.max(1e-6));
            mean_len += self.len[i] as f32;
        }
        mean_len /= 8.0;
        // Total energy of the tail, for a unit impulse in, is about
        // rate * T / (6 ln 10 * 8 * mean_len); scale it to 1.
        let total = rate * tm / (13.8155 * 8.0 * mean_len);
        self.level = if total > 0.0 {
            (1.0 / total).sqrt()
        } else {
            0.0
        };
        for side in &mut self.out_eq {
            side[0].c = self.low_shelf.coeffs(self.tilt[0] / self.tilt[1].max(1e-6));
            side[1].c = self
                .high_shelf
                .coeffs(self.tilt[2] / self.tilt[1].max(1e-6));
        }
    }

    /// Takes up the latest room for a block at `sample_rate`. `false`: the
    /// reverb is asleep; skip [`frame`](Self::frame) for this block, and the
    /// bus is cleared for `frames` frames.
    pub fn begin(&mut self, sample_rate: f32, frames: usize) -> bool {
        if !(sample_rate.is_finite() && sample_rate > 0.0) {
            return false;
        }
        if sample_rate != self.rate {
            self.tune_rate(sample_rate);
        }
        if self.send.0.room.read(&mut self.target, &mut self.seen) {
            self.quiet = 0;
        }
        // Anything waiting on the bus wakes the network.
        let mut input = false;
        for i in 0..frames.min(SEND_FRAMES) {
            if self.send.0.bus[i].load(Ordering::Relaxed) != 0 {
                input = true;
                break;
            }
        }
        if input {
            self.quiet = 0;
        }
        if self.asleep() && !input {
            // Asleep: nothing in, tail gone.
            return false;
        }
        true
    }

    /// One frame of the tail, to add to the mix. `index` is the frame's
    /// place in the block, as the sounds wrote it.
    pub fn frame(&mut self, index: usize) -> (f32, f32) {
        if self.retune_in == 0 {
            self.retune_in = BLOCK;
            self.step_params();
        }
        self.retune_in -= 1;
        let x = self.send.take(index) * self.on;
        self.energy += x * x;
        self.pre.write(x);
        let into = self.pre.read(self.predelay * self.rate);
        let o: [f32; 8] = std::array::from_fn(|i| self.lines[i].tap(self.len[i]));
        // An 8 x 8 Hadamard mix, scaled to keep energy.
        let h = hadamard8(o);
        const IN: [f32; 8] = [1.0, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, -1.0];
        let b = into * 0.353_553_4;
        for i in 0..8 {
            let fb = self.highs[i].run(self.lows[i].run(h[i] * self.gain[i]));
            self.lines[i].write(fb + IN[i] * b);
        }
        let l = (o[0] - o[1] + o[2] + o[3] - o[4] + o[5] - o[6] + o[7]) * 0.353_553_4;
        let r = (o[0] + o[1] - o[2] + o[3] + o[4] - o[5] - o[6] - o[7]) * 0.353_553_4;
        let [[l0, l1], [r0, r1]] = &mut self.out_eq;
        let l = l1.run(l0.run(l)) * self.level;
        let r = r1.run(r0.run(r)) * self.level;
        self.energy += l * l + r * r;
        (l, r)
    }

    /// Once a block: smooth toward the target room, and notice when the
    /// tail has died away.
    fn step_params(&mut self) {
        let t = &self.target;
        let on = t[ON] > 0.5;
        let rt = three(&t[RT60..RT60 + BAND_COUNT]);
        let wet = three(&t[WET..WET + BAND_COUNT]);
        let wet_mean = mean(&t[WET..WET + BAND_COUNT]);
        // About 20 ms to settle.
        let a = 1.0 - (-(BLOCK as f32) / (0.02 * self.rate)).exp();
        let mut moved = false;
        for i in 0..3 {
            let target_rt = if on { rt[i] } else { self.rt[i] };
            let next = self.rt[i] + (target_rt - self.rt[i]) * a;
            if (next - self.rt[i]).abs() > 1e-4 {
                moved = true;
            }
            self.rt[i] = next;
            let tilt = if wet_mean > 0.0 {
                wet[i] / wet_mean
            } else {
                1.0
            };
            let next = self.tilt[i] + (tilt - self.tilt[i]) * a;
            if (next - self.tilt[i]).abs() > 1e-4 {
                moved = true;
            }
            self.tilt[i] = next;
        }
        let target_on = if on { 1.0 } else { 0.0 };
        self.on += (target_on - self.on) * a;
        self.predelay = t[PREDELAY].clamp(0.0, MAX_PREDELAY_S);
        if moved {
            self.retune();
        }
        for b in self.lows.iter_mut().chain(self.highs.iter_mut()) {
            b.flush();
        }
        for side in &mut self.out_eq {
            side[0].flush();
            side[1].flush();
        }
        if self.energy < 1e-12 {
            self.quiet = self.quiet.saturating_add(1);
        } else {
            self.quiet = 0;
        }
        self.energy = 0.0;
        if self.asleep() {
            for line in &mut self.lines {
                line.clear();
            }
            self.pre.clear();
        }
    }

    fn asleep(&self) -> bool {
        self.quiet as f32 * BLOCK as f32 > SLEEP_S * self.rate
    }
}

fn hadamard8(o: [f32; 8]) -> [f32; 8] {
    let mut v = o;
    let mut h = 1;
    while h < 8 {
        let mut i = 0;
        while i < 8 {
            for j in i..i + h {
                let (a, b) = (v[j], v[j + h]);
                v[j] = a + b;
                v[j + h] = a - b;
            }
            i += 2 * h;
        }
        h *= 2;
    }
    v.map(|x| x * 0.353_553_4)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room(rt: f32, wet: f32) -> RoomRender {
        RoomRender {
            rt60_s: [rt; BAND_COUNT],
            wet_gains: [wet; BAND_COUNT],
            predelay_s: 0.01,
        }
    }

    /// The tail's energy for a unit impulse, and when it fell 60 dB.
    fn impulse(send: &ReverbSend, dsp: &mut RoomReverbDsp, rate: f32, secs: f32) -> (f32, f32) {
        let n = (rate * secs) as usize;
        let mut energy = 0.0f64;
        let mut env = Vec::new();
        let mut block_e = 0.0f64;
        for start in (0..n).step_by(128) {
            if start == 128 * 10 {
                send.add(0, 1.0);
            }
            if !dsp.begin(rate, 128) {
                env.push(0.0);
                continue;
            }
            for i in 0..128 {
                let (l, r) = dsp.frame(i);
                let e = f64::from(l * l + r * r) * 0.5;
                energy += e;
                block_e += e;
            }
            env.push(block_e);
            block_e = 0.0;
        }
        // Backward integration, for the decay time.
        let mut tail: Vec<f64> = env.clone();
        for i in (0..tail.len().saturating_sub(1)).rev() {
            tail[i] += tail[i + 1];
        }
        let top = tail[11];
        let mut t20 = None;
        let mut t5 = None;
        for (i, &e) in tail.iter().enumerate().skip(11) {
            let db = 10.0 * (e / top).log10();
            if t5.is_none() && db < -5.0 {
                t5 = Some(i);
            }
            if t20.is_none() && db < -25.0 {
                t20 = Some(i);
            }
        }
        let blocks = (t20.unwrap() - t5.unwrap()) as f32;
        let rt60 = blocks * 128.0 / rate * 3.0;
        (energy as f32, rt60)
    }

    #[test]
    fn tail_energy_and_decay_follow_the_room() {
        let rate = 48_000.0;
        for rt in [0.5f32, 1.5] {
            let send = ReverbSend::new();
            send.set_room(Some(&room(rt, 0.5)));
            let mut dsp = send.processor();
            let (energy, measured) = impulse(&send, &mut dsp, rate, rt * 2.5);
            // Energy 0.25 (wet 0.5 squared) is the aim; the send carries the
            // wet level, so here the unit impulse's tail is near 1.
            assert!((0.5..2.0).contains(&energy), "rt {rt}: energy {energy}");
            assert!(
                (measured / rt - 1.0).abs() < 0.25,
                "rt {rt}: measured {measured}"
            );
        }
    }

    #[test]
    fn sleeps_when_silent() {
        let send = ReverbSend::new();
        let mut dsp = send.processor();
        send.set_room(Some(&room(1.0, 0.5)));
        send.add(0, 1.0);
        let mut ran = 0;
        for _ in 0..1000 {
            if dsp.begin(48_000.0, 128) {
                ran += 1;
                for i in 0..128 {
                    dsp.frame(i);
                }
            }
        }
        assert!(ran > 100 && ran < 1000, "{ran}");
        assert!(!dsp.begin(48_000.0, 128));
    }

    #[test]
    fn hostile_room_values_stay_finite() {
        let send = ReverbSend::new();
        send.set_room(Some(&RoomRender {
            rt60_s: [f32::NAN, f32::INFINITY, -1.0, 1e30, 0.0, 5.0, 1.0],
            wet_gains: [f32::NAN, 1e9, -3.0, 0.0, 0.0, 0.0, 0.0],
            predelay_s: f32::INFINITY,
        }));
        let mut dsp = send.processor();
        send.add(0, 1.0);
        for _ in 0..400 {
            if dsp.begin(44_100.0, 128) {
                for i in 0..128 {
                    let (l, r) = dsp.frame(i);
                    assert!(l.is_finite() && r.is_finite());
                }
            }
        }
    }
}
