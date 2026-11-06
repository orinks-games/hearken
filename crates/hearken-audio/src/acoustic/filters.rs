//! Small DSP parts for the acoustic path: biquads, shelves and a delay line
//! with fractional reads. Nothing here allocates once made.

use std::f32::consts::PI;

/// Values this small are flushed to zero, so filter state never decays into
/// denormals.
const TINY: f32 = 1e-20;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Coeffs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Coeffs {
    pub(crate) const PASS: Coeffs = Coeffs {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    fn norm(b: [f32; 3], a: [f32; 3]) -> Coeffs {
        Coeffs {
            b0: b[0] / a[0],
            b1: b[1] / a[0],
            b2: b[2] / a[0],
            a1: a[1] / a[0],
            a2: a[2] / a[0],
        }
    }

    /// A second-order low-pass (RBJ cookbook).
    pub(crate) fn lowpass(hz: f32, q: f32, rate: f32) -> Coeffs {
        let w = 2.0 * PI * hz.clamp(10.0, rate * 0.49) / rate;
        let (c, al) = (w.cos(), w.sin() / (2.0 * q));
        Coeffs::norm(
            [(1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0],
            [1.0 + al, -2.0 * c, 1.0 - al],
        )
    }
}

/// A shelf at a fixed frequency and sample rate: only its gain moves, so
/// re-tuning it costs one square root, no trigonometry.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Shelf {
    cos: f32,
    /// 2 alpha for slope 1: sin(w) * sqrt(2).
    k: f32,
    high: bool,
}

impl Shelf {
    pub(crate) fn new(hz: f32, rate: f32, high: bool) -> Shelf {
        let w = 2.0 * PI * hz.clamp(10.0, rate * 0.45) / rate;
        Shelf {
            cos: w.cos(),
            k: w.sin() * std::f32::consts::SQRT_2,
            high,
        }
    }

    /// The coefficients for a shelf that scales its band by `ratio` (a
    /// linear amplitude, held to -80..80 dB).
    pub(crate) fn coeffs(&self, ratio: f32) -> Coeffs {
        let ratio = if ratio.is_finite() {
            ratio.clamp(1e-4, 1e4)
        } else {
            1.0
        };
        if (ratio - 1.0).abs() < 1e-5 {
            return Coeffs::PASS;
        }
        let a = ratio.sqrt();
        let c = self.cos;
        // 2 sqrt(A) alpha.
        let k = self.k * a.sqrt();
        if self.high {
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
#[derive(Clone, Copy, Debug)]
pub(crate) struct Biquad {
    pub(crate) c: Coeffs,
    s1: f32,
    s2: f32,
}

impl Default for Biquad {
    fn default() -> Biquad {
        Biquad {
            c: Coeffs::PASS,
            s1: 0.0,
            s2: 0.0,
        }
    }
}

impl Biquad {
    #[inline]
    pub(crate) fn run(&mut self, x: f32) -> f32 {
        let c = self.c;
        let y = c.b0 * x + self.s1;
        self.s1 = c.b1 * x - c.a1 * y + self.s2;
        self.s2 = c.b2 * x - c.a2 * y;
        y
    }

    /// Flushes tiny state to zero; once a block is enough.
    pub(crate) fn flush(&mut self) {
        if self.s1.is_nan() || self.s1.abs() <= TINY {
            self.s1 = 0.0;
        }
        if self.s2.is_nan() || self.s2.abs() <= TINY {
            self.s2 = 0.0;
        }
    }

    pub(crate) fn clear(&mut self) {
        self.s1 = 0.0;
        self.s2 = 0.0;
    }
}

/// A mono delay line. `write` then `read(d)`: `read(0.0)` is the sample just
/// written.
pub(crate) struct Line {
    buf: Box<[f32]>,
    pos: usize,
}

impl Line {
    pub(crate) fn new(len: usize) -> Line {
        Line {
            buf: vec![0.0; len.max(4)].into_boxed_slice(),
            pos: 0,
        }
    }

    /// The longest delay that can be read, in samples.
    pub(crate) fn max_delay(&self) -> f32 {
        (self.buf.len() - 2) as f32
    }

    #[inline]
    pub(crate) fn write(&mut self, x: f32) {
        self.pos += 1;
        if self.pos == self.buf.len() {
            self.pos = 0;
        }
        self.buf[self.pos] = x;
    }

    /// `d` samples ago, linearly interpolated; held to what the line holds.
    #[inline]
    pub(crate) fn read(&self, d: f32) -> f32 {
        let d = d.clamp(0.0, self.max_delay());
        let whole = d as usize;
        let frac = d - whole as f32;
        let len = self.buf.len();
        let i = (self.pos + len - whole) % len;
        let j = if i == 0 { len - 1 } else { i - 1 };
        let a = self.buf[i];
        a + (self.buf[j] - a) * frac
    }

    /// `d` whole samples ago.
    #[inline]
    pub(crate) fn tap(&self, d: usize) -> f32 {
        let len = self.buf.len();
        let d = d.min(len - 1);
        self.buf[(self.pos + len - d) % len]
    }

    pub(crate) fn clear(&mut self) {
        self.buf.fill(0.0);
    }
}

/// A small, fast noise source: xorshift32, uniform in -1..1.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Noise(u32);

impl Noise {
    pub(crate) fn new(seed: u32) -> Noise {
        Noise(seed.max(1))
    }

    #[inline]
    pub(crate) fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gain_at(c: Coeffs, hz: f32, rate: f32) -> f32 {
        let mut b = Biquad {
            c,
            ..Biquad::default()
        };
        let n = (rate * 0.5) as usize;
        let mut acc = 0.0f64;
        let mut acc_in = 0.0f64;
        for i in 0..n {
            let x = (2.0 * PI * hz * i as f32 / rate).sin();
            let y = b.run(x);
            if i > n / 2 {
                acc += f64::from(y * y);
                acc_in += f64::from(x * x);
            }
        }
        (acc / acc_in).sqrt() as f32
    }

    #[test]
    fn shelves_reach_their_gain() {
        let rate = 48_000.0;
        let high = Shelf::new(1000.0, rate, true).coeffs(0.1);
        assert!((gain_at(high, 100.0, rate) - 1.0).abs() < 0.03);
        assert!((gain_at(high, 12_000.0, rate) - 0.1).abs() < 0.01);
        let low = Shelf::new(1000.0, rate, false).coeffs(2.0);
        assert!((gain_at(low, 50.0, rate) - 2.0).abs() < 0.05);
        assert!((gain_at(low, 15_000.0, rate) - 1.0).abs() < 0.03);
        let half = gain_at(high, 1000.0, rate);
        assert!((half - 0.1f32.sqrt()).abs() < 0.05, "{half}");
    }

    #[test]
    fn line_reads_fractions() {
        let mut l = Line::new(16);
        for i in 0..10 {
            l.write(i as f32);
        }
        assert_eq!(l.read(0.0), 9.0);
        assert_eq!(l.read(2.0), 7.0);
        assert!((l.read(2.5) - 6.5).abs() < 1e-6);
        assert_eq!(l.tap(3), 6.0);
    }
}
