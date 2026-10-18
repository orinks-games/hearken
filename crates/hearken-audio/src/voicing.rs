//! Inserts: processing a backend runs on a sound's own track, chosen by the
//! bus the sound starts on (see [`Audio::set_bus_insert`](crate::Audio::set_bus_insert)).
//!
//! The one insert so far is [`CabSpeakers`]: a radio heard through a
//! vehicle's door speakers in a small cab. In signal order:
//!
//! 1. the speaker: a high-pass, a low-pass and a midrange bell;
//! 2. the cabin's tone, which moves between a sealed cab and an open
//!    window: a low shelf and a high shelf;
//! 3. stereo width, by scaling the side signal;
//! 4. the far speaker a little later and quieter than the near one;
//! 5. a tiny room: six early reflections a side under 10 ms and a short
//!    decaying tail, mixed in under the direct sound;
//! 6. a fixed make-up gain, so the voiced sound is as loud as the clean one.
//!
//! Every filter is an RBJ cookbook biquad. Everything is linear, so voicing
//! each sound on a bus is the same as voicing the bus's sum.
//!
//! The DSP itself ([`CabSpeakersDsp`]) knows nothing of any audio library:
//! a backend makes one per sound from the shared [`CabSpeakers`] handle and
//! runs it on its audio thread. Nothing in it allocates once made.

use std::f32::consts::{FRAC_1_SQRT_2, PI};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Processing a backend inserts on every sound started on a bus.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Insert {
    /// A radio through a cab's speakers: see [`CabSpeakers`].
    CabSpeakers(CabSpeakers),
}

/// One shelf pair and its make-up gain: the cab's tone at one window state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CabTone {
    pub low_shelf_hz: f32,
    pub low_shelf_db: f32,
    pub high_shelf_hz: f32,
    pub high_shelf_db: f32,
    /// Gain after everything, so the voiced sound matches the clean one in
    /// loudness.
    pub makeup_db: f32,
}

/// One early reflection: its delay after the direct sound, and its gain
/// (negative flips its polarity).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reflection {
    pub ms: f32,
    pub gain: f32,
}

/// What [`CabSpeakers`] does, in numbers. The default is a truck stereo with
/// the driver on the left.
#[derive(Clone, Debug, PartialEq)]
pub struct CabSpeakersParams {
    pub highpass_hz: f32,
    pub lowpass_hz: f32,
    pub mid_hz: f32,
    pub mid_db: f32,
    pub mid_q: f32,
    /// The tone with the windows up.
    pub sealed: CabTone,
    /// The tone with the window down.
    pub open: CabTone,
    /// Stereo width: `1` as recorded, `0` mono.
    pub width: f32,
    /// How much later the far speaker arrives, in ms.
    pub far_delay_ms: f32,
    /// How much quieter the far speaker is, in dB (negative).
    pub far_db: f32,
    /// Whether the far speaker is the right one (the driver sits on the
    /// left).
    pub far_is_right: bool,
    pub reflections_left: Vec<Reflection>,
    pub reflections_right: Vec<Reflection>,
    /// When the room's tail starts after the direct sound, in ms.
    pub tail_start_ms: f32,
    /// The tail's decay to -60 dB, in seconds.
    pub tail_rt60_s: f32,
    pub tail_lowpass_hz: f32,
    /// The room under the direct sound: `out = direct + room_wet * room`.
    pub room_wet: f32,
}

const fn reflection(ms: f32, gain: f32) -> Reflection {
    Reflection { ms, gain }
}

impl Default for CabSpeakersParams {
    fn default() -> CabSpeakersParams {
        CabSpeakersParams {
            highpass_hz: 90.0,
            lowpass_hz: 11_000.0,
            mid_hz: 2_000.0,
            mid_db: 3.0,
            mid_q: 0.9,
            // Make-up gains: the level the chain costs, measured as
            // BS.1770 loudness over three stations (music, jazz, talk);
            // every one lands within 0.35 dB of the clean signal.
            sealed: CabTone {
                low_shelf_hz: 120.0,
                low_shelf_db: 3.0,
                high_shelf_hz: 6_000.0,
                high_shelf_db: -2.0,
                makeup_db: 0.13,
            },
            open: CabTone {
                low_shelf_hz: 120.0,
                low_shelf_db: -1.0,
                high_shelf_hz: 5_000.0,
                high_shelf_db: 1.5,
                makeup_db: 0.26,
            },
            width: 0.7,
            far_delay_ms: 0.75,
            far_db: -2.0,
            far_is_right: true,
            reflections_left: vec![
                reflection(1.3, 0.32),
                reflection(2.4, -0.24),
                reflection(3.6, 0.20),
                reflection(5.1, -0.15),
                reflection(6.7, 0.11),
                reflection(8.6, -0.08),
            ],
            reflections_right: vec![
                reflection(1.6, 0.30),
                reflection(2.9, -0.22),
                reflection(4.0, 0.18),
                reflection(5.6, -0.14),
                reflection(7.3, 0.10),
                reflection(9.4, -0.07),
            ],
            tail_start_ms: 8.0,
            tail_rt60_s: 0.10,
            tail_lowpass_hz: 4_000.0,
            room_wet: 0.30,
        }
    }
}

/// A target and its ramp in one word, so the audio thread never reads one
/// without the other.
fn pack(value: f32, ms: u32) -> u64 {
    (u64::from(value.to_bits()) << 32) | u64::from(ms)
}

fn unpack(word: u64) -> (f32, u32) {
    (f32::from_bits((word >> 32) as u32), word as u32)
}

fn unit(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

struct Shared {
    params: CabSpeakersParams,
    amount: AtomicU64,
    window: AtomicU64,
}

/// The cab-speaker voicing, shared by every sound that carries it: the game
/// thread sets how much of it plays and how far the window is down, and
/// every sound's [`CabSpeakersDsp`] follows on the audio thread.
#[derive(Clone)]
pub struct CabSpeakers {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for CabSpeakers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CabSpeakers")
            .field("amount", &self.amount())
            .field("window", &self.window())
            .finish()
    }
}

impl CabSpeakers {
    /// Fully on, windows up.
    pub fn new(params: CabSpeakersParams) -> CabSpeakers {
        CabSpeakers {
            shared: Arc::new(Shared {
                params,
                amount: AtomicU64::new(pack(1.0, 0)),
                window: AtomicU64::new(pack(0.0, 0)),
            }),
        }
    }

    pub fn params(&self) -> &CabSpeakersParams {
        &self.shared.params
    }

    /// How much of the voicing plays, `0` (clean, the sound untouched) to
    /// `1`, crossfading over `ramp_ms`.
    pub fn set_amount(&self, amount: f32, ramp_ms: u32) {
        self.shared
            .amount
            .store(pack(unit(amount), ramp_ms), Ordering::Relaxed);
    }

    /// How far the window is down, `0` (sealed) to `1` (open), moving the
    /// tone over `ramp_ms`.
    pub fn set_window(&self, open: f32, ramp_ms: u32) {
        self.shared
            .window
            .store(pack(unit(open), ramp_ms), Ordering::Relaxed);
    }

    /// The amount last asked for.
    pub fn amount(&self) -> f32 {
        unpack(self.shared.amount.load(Ordering::Relaxed)).0
    }

    /// The window position last asked for.
    pub fn window(&self) -> f32 {
        unpack(self.shared.window.load(Ordering::Relaxed)).0
    }

    /// A processor for one sound, starting where the handle stands now.
    pub fn processor(&self) -> CabSpeakersDsp {
        CabSpeakersDsp::new(self.clone())
    }
}

/// One value moving in a straight line toward its target.
struct Ramp {
    current: f32,
    target: f32,
    step: f32,
    left: u64,
    seen: u64,
}

impl Ramp {
    fn new(word: u64) -> Ramp {
        let (value, _) = unpack(word);
        Ramp {
            current: value,
            target: value,
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
        let frames = (ms as f32 / 1000.0 * rate).round() as u64;
        self.target = target;
        if frames == 0 {
            self.current = target;
            self.left = 0;
        } else {
            self.step = (target - self.current) / frames as f32;
            self.left = frames;
        }
    }

    /// Moves `frames` along.
    fn advance(&mut self, frames: u64) -> f32 {
        if self.left > 0 {
            if frames >= self.left {
                self.left = 0;
                self.current = self.target;
            } else {
                self.left -= frames;
                self.current += self.step * frames as f32;
            }
        }
        self.current
    }

    fn settled_at(&self, value: f32) -> bool {
        self.left == 0 && self.current == value
    }
}

#[derive(Clone, Copy, Default)]
struct Coeffs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Coeffs {
    fn norm(b: [f32; 3], a: [f32; 3]) -> Coeffs {
        Coeffs {
            b0: b[0] / a[0],
            b1: b[1] / a[0],
            b2: b[2] / a[0],
            a1: a[1] / a[0],
            a2: a[2] / a[0],
        }
    }

    fn w0(hz: f32, rate: f32) -> f32 {
        2.0 * PI * hz.clamp(10.0, rate * 0.49) / rate
    }

    fn highpass(hz: f32, q: f32, rate: f32) -> Coeffs {
        let w = Coeffs::w0(hz, rate);
        let (c, al) = (w.cos(), w.sin() / (2.0 * q));
        Coeffs::norm(
            [(1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0],
            [1.0 + al, -2.0 * c, 1.0 - al],
        )
    }

    fn lowpass(hz: f32, q: f32, rate: f32) -> Coeffs {
        let w = Coeffs::w0(hz, rate);
        let (c, al) = (w.cos(), w.sin() / (2.0 * q));
        Coeffs::norm(
            [(1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0],
            [1.0 + al, -2.0 * c, 1.0 - al],
        )
    }

    fn bell(hz: f32, db: f32, q: f32, rate: f32) -> Coeffs {
        let a = 10f32.powf(db / 40.0);
        let w = Coeffs::w0(hz, rate);
        let (c, al) = (w.cos(), w.sin() / (2.0 * q));
        Coeffs::norm(
            [1.0 + al * a, -2.0 * c, 1.0 - al * a],
            [1.0 + al / a, -2.0 * c, 1.0 - al / a],
        )
    }

    /// A shelf with slope 1.
    fn shelf(hz: f32, db: f32, high: bool, rate: f32) -> Coeffs {
        let a = 10f32.powf(db / 40.0);
        let w = Coeffs::w0(hz, rate);
        let c = w.cos();
        let k = 2.0 * a.sqrt() * (w.sin() / 2.0 * 2f32.sqrt());
        if high {
            Coeffs::norm(
                [
                    a * ((a + 1.0) + (a - 1.0) * c + k),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
                    a * ((a + 1.0) + (a - 1.0) * c - k),
                ],
                [
                    (a + 1.0) - (a - 1.0) * c + k,
                    2.0 * ((a - 1.0) - (a + 1.0) * c),
                    (a + 1.0) - (a - 1.0) * c - k,
                ],
            )
        } else {
            Coeffs::norm(
                [
                    a * ((a + 1.0) - (a - 1.0) * c + k),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * c),
                    a * ((a + 1.0) - (a - 1.0) * c - k),
                ],
                [
                    (a + 1.0) + (a - 1.0) * c + k,
                    -2.0 * ((a - 1.0) + (a + 1.0) * c),
                    (a + 1.0) + (a - 1.0) * c - k,
                ],
            )
        }
    }
}

/// A biquad, transposed direct form II.
#[derive(Clone, Copy, Default)]
struct Biquad {
    c: Coeffs,
    s1: f32,
    s2: f32,
}

impl Biquad {
    fn run(&mut self, x: f32) -> f32 {
        let c = self.c;
        let y = c.b0 * x + self.s1;
        self.s1 = c.b1 * x - c.a1 * y + self.s2;
        self.s2 = c.b2 * x - c.a2 * y;
        y
    }

    fn clear(&mut self) {
        self.s1 = 0.0;
        self.s2 = 0.0;
    }
}

/// A delay line read before it is written: `read(d)` is the sample written
/// `d` writes ago.
struct Delay {
    buf: Box<[f32]>,
    pos: usize,
}

impl Delay {
    fn new(len: usize) -> Delay {
        Delay {
            buf: vec![0.0; len.max(1)].into_boxed_slice(),
            pos: 0,
        }
    }

    fn read(&self, d: usize) -> f32 {
        let len = self.buf.len();
        let d = d.clamp(1, len);
        self.buf[(self.pos + len - d) % len]
    }

    fn write(&mut self, x: f32) {
        self.buf[self.pos] = x;
        self.pos = (self.pos + 1) % self.buf.len();
    }

    fn clear(&mut self) {
        self.buf.fill(0.0);
        self.pos = 0;
    }
}

/// A Butterworth biquad's Q.
const BUTTERWORTH: f32 = FRAC_1_SQRT_2;

/// The highest sample rate the delay lines are sized for; above it, the
/// longest delays are held to what fits.
const MAX_RATE: f32 = 192_000.0;

/// How often a moving window re-tunes the shelves, in frames.
const RETUNE_FRAMES: u32 = 32;

/// The room tail's lines, in ms: a four-line feedback delay network a
/// side, at lengths that share no factor.
const TAIL_LINES_LEFT_MS: [f32; 4] = [2.9, 3.7, 4.8, 6.1];
const TAIL_LINES_RIGHT_MS: [f32; 4] = [3.1, 3.9, 5.1, 6.4];

/// The tail's level, so its energy matches a decaying noise tail starting
/// at a peak of 0.05 (the audition's impulse response) at 44.1 kHz.
const TAIL_LEVEL: f32 = 0.225;

/// One side's room: the reflections off the line the direct sound runs
/// through, and the tail's network behind a pre-delay.
struct Room {
    taps: Vec<(usize, f32)>,
    line: Delay,
    tail: [Delay; 4],
    tail_len: [usize; 4],
    tail_gain: [f32; 4],
    pre: usize,
    tail_lp: Biquad,
    tail_level: f32,
}

impl Room {
    fn new(reflections: &[Reflection], lines_ms: [f32; 4]) -> Room {
        let max_ms = reflections
            .iter()
            .map(|r| r.ms)
            .fold(20.0f32, f32::max)
            .min(100.0);
        let size = |ms: f32| (ms / 1000.0 * MAX_RATE).ceil() as usize + 2;
        Room {
            taps: Vec::with_capacity(reflections.len()),
            line: Delay::new(size(max_ms)),
            tail: lines_ms.map(|ms| Delay::new(size(ms))),
            tail_len: [1; 4],
            tail_gain: [0.0; 4],
            pre: 1,
            tail_lp: Biquad::default(),
            tail_level: 0.0,
        }
    }

    fn tune(
        &mut self,
        p: &CabSpeakersParams,
        reflections: &[Reflection],
        lines_ms: [f32; 4],
        rate: f32,
    ) {
        let samples = |ms: f32| (ms.max(0.0) / 1000.0 * rate).round() as usize;
        self.taps.clear();
        for r in reflections {
            self.taps.push((samples(r.ms).max(1), r.gain));
        }
        let rt60 = p.tail_rt60_s.max(0.01);
        for ((len, gain), ms) in self
            .tail_len
            .iter_mut()
            .zip(self.tail_gain.iter_mut())
            .zip(lines_ms)
        {
            *len = samples(ms).max(1);
            *gain = 10f32.powf(-3.0 * (*len as f32 / rate) / rt60);
        }
        let shortest = lines_ms.iter().copied().fold(f32::MAX, f32::min);
        self.pre = samples(p.tail_start_ms - shortest).max(1);
        self.tail_lp.c = Coeffs::lowpass(p.tail_lowpass_hz, BUTTERWORTH, rate);
        // The same decay holds more samples at a higher rate.
        self.tail_level = TAIL_LEVEL * (44_100.0 / rate).sqrt();
    }

    /// The room's answer to `x`, before the wet level.
    fn run(&mut self, x: f32) -> f32 {
        let mut wet = 0.0;
        for &(d, g) in &self.taps {
            wet += g * self.line.read(d);
        }
        let into_tail = self.line.read(self.pre);
        self.line.write(x);
        let o = [
            self.tail[0].read(self.tail_len[0]),
            self.tail[1].read(self.tail_len[1]),
            self.tail[2].read(self.tail_len[2]),
            self.tail[3].read(self.tail_len[3]),
        ];
        // A Hadamard mix, which keeps the network's energy.
        let m = [
            0.5 * (o[0] + o[1] + o[2] + o[3]),
            0.5 * (o[0] - o[1] + o[2] - o[3]),
            0.5 * (o[0] + o[1] - o[2] - o[3]),
            0.5 * (o[0] - o[1] - o[2] + o[3]),
        ];
        let signs = [1.0, -1.0, 1.0, -1.0];
        for i in 0..4 {
            self.tail[i].write(signs[i] * into_tail + self.tail_gain[i] * m[i]);
        }
        let tail = o[0] - o[1] + o[2] - o[3];
        wet + self.tail_level * self.tail_lp.run(tail)
    }

    fn clear(&mut self) {
        self.line.clear();
        for line in &mut self.tail {
            line.clear();
        }
        self.tail_lp.clear();
    }
}

/// One side's speaker filters.
#[derive(Clone, Copy, Default)]
struct Side {
    hp: Biquad,
    lp: Biquad,
    mid: Biquad,
    low: Biquad,
    high: Biquad,
}

impl Side {
    fn run(&mut self, x: f32) -> f32 {
        let x = self.hp.run(x);
        let x = self.lp.run(x);
        let x = self.mid.run(x);
        let x = self.low.run(x);
        self.high.run(x)
    }

    fn clear(&mut self) {
        for b in [
            &mut self.hp,
            &mut self.lp,
            &mut self.mid,
            &mut self.low,
            &mut self.high,
        ] {
            b.clear();
        }
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// [`CabSpeakers`] for one sound, on the audio thread. Call
/// [`begin`](Self::begin) once a block, then [`frame`](Self::frame) for each
/// frame of it while `begin` says the voicing is on.
pub struct CabSpeakersDsp {
    handle: CabSpeakers,
    rate: f32,
    amount: Ramp,
    window: Ramp,
    /// The window position the shelves are tuned to.
    tuned: f32,
    retune_in: u32,
    sides: [Side; 2],
    far: Delay,
    far_len: usize,
    far_gain: f32,
    rooms: [Room; 2],
    makeup: f32,
    /// Whether any state is left over from processing, to clear once the
    /// voicing is off.
    dirty: bool,
}

impl CabSpeakersDsp {
    fn new(handle: CabSpeakers) -> CabSpeakersDsp {
        let p = handle.params().clone();
        let far_ms = p.far_delay_ms.clamp(0.0, 50.0);
        CabSpeakersDsp {
            amount: Ramp::new(handle.shared.amount.load(Ordering::Relaxed)),
            window: Ramp::new(handle.shared.window.load(Ordering::Relaxed)),
            tuned: f32::NAN,
            retune_in: 0,
            rate: 0.0,
            sides: [Side::default(); 2],
            far: Delay::new((far_ms / 1000.0 * MAX_RATE).ceil() as usize + 2),
            far_len: 0,
            far_gain: 10f32.powf(p.far_db / 20.0),
            rooms: [
                Room::new(&p.reflections_left, TAIL_LINES_LEFT_MS),
                Room::new(&p.reflections_right, TAIL_LINES_RIGHT_MS),
            ],
            makeup: 1.0,
            dirty: false,
            handle,
        }
    }

    /// The fixed filters and delays, for `rate`.
    fn tune_rate(&mut self, rate: f32) {
        self.rate = rate;
        let p = self.handle.shared.params.clone();
        for side in &mut self.sides {
            side.hp.c = Coeffs::highpass(p.highpass_hz, BUTTERWORTH, rate);
            side.lp.c = Coeffs::lowpass(p.lowpass_hz, BUTTERWORTH, rate);
            side.mid.c = Coeffs::bell(p.mid_hz, p.mid_db, p.mid_q, rate);
        }
        self.far_len = (p.far_delay_ms.max(0.0) / 1000.0 * rate).round() as usize;
        self.rooms[0].tune(&p, &p.reflections_left, TAIL_LINES_LEFT_MS, rate);
        self.rooms[1].tune(&p, &p.reflections_right, TAIL_LINES_RIGHT_MS, rate);
        self.tuned = f32::NAN;
    }

    /// The shelves and make-up gain for window position `w`.
    fn tune_window(&mut self, w: f32) {
        let p = &self.handle.shared.params;
        let (s, o) = (p.sealed, p.open);
        let low = Coeffs::shelf(
            lerp(s.low_shelf_hz, o.low_shelf_hz, w),
            lerp(s.low_shelf_db, o.low_shelf_db, w),
            false,
            self.rate,
        );
        let high = Coeffs::shelf(
            lerp(s.high_shelf_hz, o.high_shelf_hz, w),
            lerp(s.high_shelf_db, o.high_shelf_db, w),
            true,
            self.rate,
        );
        for side in &mut self.sides {
            side.low.c = low;
            side.high.c = high;
        }
        self.makeup = 10f32.powf(lerp(s.makeup_db, o.makeup_db, w) / 20.0);
        self.tuned = w;
    }

    /// Takes up the handle's latest settings for a block at `sample_rate`.
    /// `false` means the voicing is off and settled: leave the block as it
    /// is, untouched.
    pub fn begin(&mut self, sample_rate: f32) -> bool {
        if !(sample_rate.is_finite() && sample_rate > 0.0) {
            return false;
        }
        if sample_rate != self.rate {
            self.tune_rate(sample_rate);
        }
        let shared = &self.handle.shared;
        self.amount
            .poll(shared.amount.load(Ordering::Relaxed), sample_rate);
        self.window
            .poll(shared.window.load(Ordering::Relaxed), sample_rate);
        if self.amount.settled_at(0.0) {
            if self.dirty {
                self.clear();
            }
            return false;
        }
        true
    }

    fn clear(&mut self) {
        for side in &mut self.sides {
            side.clear();
        }
        self.far.clear();
        for room in &mut self.rooms {
            room.clear();
        }
        self.dirty = false;
    }

    /// One stereo frame through the voicing.
    pub fn frame(&mut self, left: f32, right: f32) -> (f32, f32) {
        self.dirty = true;
        if self.retune_in == 0 {
            self.retune_in = RETUNE_FRAMES;
            let w = self.window.advance(u64::from(RETUNE_FRAMES));
            if w != self.tuned {
                self.tune_window(w);
            }
        }
        self.retune_in -= 1;
        let amount = self.amount.advance(1);
        let p = &self.handle.shared.params;

        let l = self.sides[0].run(left);
        let r = self.sides[1].run(right);
        let mid = 0.5 * (l + r);
        let side = 0.5 * (l - r) * p.width;
        let (near, far) = if p.far_is_right {
            (mid + side, mid - side)
        } else {
            (mid - side, mid + side)
        };
        let late = if self.far_len == 0 {
            far
        } else {
            let out = self.far.read(self.far_len);
            self.far.write(far);
            out
        };
        let far = late * self.far_gain;
        let (l, r) = if p.far_is_right {
            (near, far)
        } else {
            (far, near)
        };
        let wet = p.room_wet;
        let l = l + wet * self.rooms[0].run(l);
        let r = r + wet * self.rooms[1].run(r);
        let (l, r) = (l * self.makeup, r * self.makeup);
        if amount >= 1.0 {
            (l, r)
        } else {
            (lerp(left, l, amount), lerp(right, r, amount))
        }
    }

    /// Runs a whole block of frames in place, as a backend would.
    pub fn process(&mut self, frames: &mut [[f32; 2]], sample_rate: f32) {
        if !self.begin(sample_rate) {
            return;
        }
        for frame in frames {
            let (l, r) = self.frame(frame[0], frame[1]);
            *frame = [l, r];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 44_100.0;

    /// Only the speaker and the cabin's tone: no width, no far speaker, no
    /// room, no make-up, so one side's response is the filters alone.
    fn filters_only() -> CabSpeakersParams {
        let mut p = CabSpeakersParams {
            width: 1.0,
            far_delay_ms: 0.0,
            far_db: 0.0,
            room_wet: 0.0,
            ..CabSpeakersParams::default()
        };
        p.sealed.makeup_db = 0.0;
        p.open.makeup_db = 0.0;
        p
    }

    fn run(dsp: &mut CabSpeakersDsp, input: &[[f32; 2]]) -> Vec<[f32; 2]> {
        let mut out = input.to_vec();
        for block in out.chunks_mut(512) {
            dsp.process(block, RATE);
        }
        out
    }

    fn sine(hz: f32, seconds: f32) -> Vec<[f32; 2]> {
        (0..(seconds * RATE) as usize)
            .map(|n| {
                let x = (2.0 * PI * hz * n as f32 / RATE).sin() * 0.5;
                [x, x]
            })
            .collect()
    }

    fn rms(x: &[[f32; 2]], side: usize) -> f32 {
        (x.iter().map(|f| f[side] * f[side]).sum::<f32>() / x.len() as f32).sqrt()
    }

    /// A sine's gain through the left side, in dB, past the filters'
    /// settling.
    fn gain_db(dsp: &mut CabSpeakersDsp, hz: f32) -> f32 {
        let input = sine(hz, 1.0);
        let out = run(dsp, &input);
        let skip = input.len() / 2;
        20.0 * (rms(&out[skip..], 0) / rms(&input[skip..], 0)).log10()
    }

    /// The audition's measured response of the speaker and cabin filters
    /// (SETTINGS.md), windows up and window down.
    #[test]
    fn the_filters_match_the_audition_response() {
        let up = [
            (40.0, -11.3),
            (90.0, -0.7),
            (120.0, 0.3),
            (1_000.0, 1.1),
            (2_000.0, 3.0),
            (6_000.0, -0.8),
            (11_000.0, -4.8),
        ];
        let down = [
            (60.0, -8.8),
            (120.0, -1.7),
            (2_000.0, 3.0),
            (6_000.0, 1.3),
            (11_000.0, -1.5),
        ];
        let cab = CabSpeakers::new(filters_only());
        for (hz, want) in up {
            let got = gain_db(&mut cab.processor(), hz);
            assert!(
                (got - want).abs() < 0.3,
                "up {hz} Hz: {got:.2} dB, want {want}"
            );
        }
        cab.set_window(1.0, 0);
        for (hz, want) in down {
            let got = gain_db(&mut cab.processor(), hz);
            assert!(
                (got - want).abs() < 0.3,
                "down {hz} Hz: {got:.2} dB, want {want}"
            );
        }
    }

    #[test]
    fn clean_leaves_every_sample_as_it_was() {
        let cab = CabSpeakers::new(CabSpeakersParams::default());
        cab.set_amount(0.0, 0);
        let input: Vec<[f32; 2]> = sine(440.0, 0.2)
            .into_iter()
            .enumerate()
            .map(|(n, [l, _])| [l, (n as f32 * 0.01).cos() * 0.3])
            .collect();
        let out = run(&mut cab.processor(), &input);
        assert_eq!(out, input);
    }

    #[test]
    fn turning_it_off_crossfades_then_leaves_the_sound_untouched() {
        let cab = CabSpeakers::new(CabSpeakersParams::default());
        let mut dsp = cab.processor();
        let warm = sine(500.0, 0.5);
        let _ = run(&mut dsp, &warm);
        cab.set_amount(0.0, 300);
        let fading = sine(500.0, 0.3);
        let out = run(&mut dsp, &fading);
        assert_ne!(out, fading, "the crossfade is still under way");
        let after = sine(700.0, 0.2);
        assert_eq!(run(&mut dsp, &after), after);
    }

    #[test]
    fn the_far_speaker_is_later_and_quieter() {
        let mut p = filters_only();
        p.far_delay_ms = 0.75;
        p.far_db = -2.0;
        let cab = CabSpeakers::new(p);
        // Mono in: both sides filter alike, so the right is the left moved
        // 33 samples (0.75 ms at 44.1 kHz) and 2 dB down.
        let input: Vec<[f32; 2]> = (0..4_000)
            .map(|n| {
                let x = ((n * 7919) % 1000) as f32 / 1000.0 - 0.5;
                [x, x]
            })
            .collect();
        let out = run(&mut cab.processor(), &input);
        let gain = 10f32.powf(-2.0 / 20.0);
        for n in 33..out.len() {
            let want = out[n - 33][0] * gain;
            assert!((out[n][1] - want).abs() < 1e-5, "frame {n}");
        }
        assert!(out[..33].iter().all(|f| f[1] == 0.0));
    }

    #[test]
    fn width_scales_the_side_signal() {
        let mut p = filters_only();
        p.width = 0.7;
        let cab = CabSpeakers::new(p.clone());
        let side: Vec<[f32; 2]> = sine(1_000.0, 0.5)
            .into_iter()
            .map(|[l, _]| [l, -l])
            .collect();
        let narrowed = run(&mut cab.processor(), &side);
        p.width = 1.0;
        let full = run(&mut CabSpeakers::new(p).processor(), &side);
        let ratio = rms(&narrowed[10_000..], 0) / rms(&full[10_000..], 0);
        assert!((ratio - 0.7).abs() < 1e-3, "{ratio}");
    }

    /// The room's impulse response: the six reflections where the audition
    /// put them, then a tail with the audition's energy that has died away
    /// 60 dB within its RT60 of starting.
    #[test]
    fn the_room_has_the_audition_reflections_and_tail() {
        let p = CabSpeakersParams::default();
        let mut room = Room::new(&p.reflections_left, TAIL_LINES_LEFT_MS);
        room.tune(&p, &p.reflections_left, TAIL_LINES_LEFT_MS, RATE);
        let ir: Vec<f32> = (0..(0.3 * RATE) as usize)
            .map(|n| room.run(if n == 0 { 1.0 } else { 0.0 }))
            .collect();
        for r in &p.reflections_left {
            let at = (r.ms / 1000.0 * RATE).round() as usize;
            assert!((ir[at] - r.gain).abs() < 0.03, "{} ms: {}", r.ms, ir[at]);
        }
        let taps: f32 = p.reflections_left.iter().map(|r| r.gain * r.gain).sum();
        let total: f32 = ir.iter().map(|x| x * x).sum();
        let tail = total - taps;
        eprintln!("room tail energy {tail:.3}");
        // The audition's tails held 0.114 (left) and 0.141 (right).
        assert!((0.09..0.17).contains(&tail), "tail energy {tail}");
        let start = (p.tail_start_ms / 1000.0 * RATE) as usize;
        let window =
            |from: usize| -> f32 { ir[from..from + 441].iter().map(|x| x * x).sum::<f32>() };
        let first = window(start + 441);
        let late = window(start + 441 + (0.1 * RATE) as usize);
        let drop_db = 10.0 * (late / first).log10();
        assert!((-75.0..-45.0).contains(&drop_db), "dropped {drop_db:.1} dB");
    }

    /// Both sides' level through BS.1770's K-weighting (ungated): what the
    /// make-up gains were measured with.
    fn loudness(x: &[[f32; 2]]) -> f32 {
        let mut level = 0.0;
        for side in 0..2 {
            let mut shelf = Biquad {
                c: Coeffs::shelf(1_681.97, 4.0, true, RATE),
                ..Biquad::default()
            };
            let mut hp = Biquad {
                c: Coeffs::highpass(38.0, 0.5, RATE),
                ..Biquad::default()
            };
            let k: Vec<[f32; 2]> = x
                .iter()
                .map(|f| {
                    let y = hp.run(shelf.run(f[side]));
                    [y, y]
                })
                .collect();
            level += rms(&k, 0).powi(2);
        }
        level.sqrt()
    }

    /// Broadband noise comes out within a dB and a half of where it went
    /// in, windows up or down: the make-up gain keeps a volume setting
    /// meaning the same.
    #[test]
    fn loudness_stays_close_to_clean() {
        let mut seed = 1u32;
        let mut noise = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        };
        // Pink, by Paul Kellet's economy filter: the spectrum of most music
        // and speech, near enough.
        let mut b = [0.0f32; 3];
        let input: Vec<[f32; 2]> = (0..(4.0 * RATE) as usize)
            .map(|_| {
                let white = noise();
                b[0] = 0.99765 * b[0] + white * 0.099_046;
                b[1] = 0.963 * b[1] + white * 0.296_516_4;
                b[2] = 0.57 * b[2] + white * 1.052_691_3;
                let x = 0.2 * (b[0] + b[1] + b[2] + white * 0.1848);
                [x, 0.9 * x + 0.02 * noise()]
            })
            .collect();
        for window in [0.0, 1.0] {
            let cab = CabSpeakers::new(CabSpeakersParams::default());
            cab.set_window(window, 0);
            let out = run(&mut cab.processor(), &input);
            let before = loudness(&input);
            let after = loudness(&out);
            let db = 20.0 * (after / before).log10();
            eprintln!("pink noise, window {window}: {db:+.2} dB");
            assert!(db.abs() < 1.5, "window {window}: {db:.2} dB");
        }
    }

    /// The window glides the tone over its ramp, with no jump on the way.
    #[test]
    fn the_window_moves_the_tone_smoothly_over_its_ramp() {
        let cab = CabSpeakers::new(CabSpeakersParams::default());
        let mut dsp = cab.processor();
        let _ = run(&mut dsp, &sine(150.0, 0.5));
        cab.set_window(1.0, 2_000);
        let input = sine(150.0, 2.01);
        let out = run(&mut dsp, &input);
        let worst_in = input
            .windows(2)
            .map(|w| (w[1][0] - w[0][0]).abs())
            .fold(0.0, f32::max);
        let worst_out = out
            .windows(2)
            .map(|w| (w[1][0] - w[0][0]).abs())
            .fold(0.0, f32::max);
        assert!(worst_out < worst_in * 1.6, "{worst_out} vs {worst_in}");
        assert!((dsp.tuned - 1.0).abs() < 1e-6);
        // Half way through, half way between.
        let mut half = cab.processor();
        cab.set_window(0.0, 0);
        let _ = run(&mut half, &sine(150.0, 0.1));
        cab.set_window(1.0, 2_000);
        let _ = run(&mut half, &sine(150.0, 1.0));
        assert!((half.tuned - 0.5).abs() < 0.03, "{}", half.tuned);
    }

    #[test]
    fn it_runs_at_any_common_rate() {
        for rate in [22_050.0, 48_000.0, 96_000.0, 192_000.0] {
            let cab = CabSpeakers::new(CabSpeakersParams::default());
            let mut dsp = cab.processor();
            let mut block = vec![[0.25f32, -0.25]; 1_024];
            for _ in 0..50 {
                dsp.process(&mut block, rate);
            }
            assert!(block.iter().all(|f| f[0].is_finite() && f[1].is_finite()));
        }
    }
}
