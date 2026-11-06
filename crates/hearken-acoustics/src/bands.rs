//! The fixed set of frequency bands every per-band value uses.

use serde::{Deserialize, Serialize};

/// How many bands there are.
pub const BAND_COUNT: usize = 7;

/// Octave band centres, 125 Hz to 8 kHz (the nominal ISO 266 values).
pub const BAND_CENTRES_HZ: [f64; BAND_COUNT] =
    [125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0];

/// Exact octave band centres, 1000 * 10^(k/10) for k = -9, -6, ... 9
/// (ISO 266). Physics that depends on frequency uses these.
pub const BAND_EXACT_HZ: [f64; BAND_COUNT] = [
    125.89254117941672,
    251.188643150958,
    501.1872336272723,
    1000.0,
    1995.2623149688795,
    3981.071705534973,
    7943.282347242815,
];

/// One value per octave band, lowest band first.
///
/// What the values mean depends on where they appear: an absorption
/// coefficient, a loss in decibels, or a linear amplitude gain.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Bands(pub [f64; BAND_COUNT]);

impl Default for Bands {
    fn default() -> Self {
        Bands::splat(0.0)
    }
}

impl Bands {
    /// Unity gain in every band.
    pub const ONE: Bands = Bands([1.0; BAND_COUNT]);
    /// Zero in every band.
    pub const ZERO: Bands = Bands([0.0; BAND_COUNT]);

    pub const fn splat(v: f64) -> Bands {
        Bands([v; BAND_COUNT])
    }

    pub fn map(self, f: impl Fn(f64) -> f64) -> Bands {
        let mut out = self.0;
        for v in out.iter_mut() {
            *v = f(*v);
        }
        Bands(out)
    }

    pub fn zip(self, o: Bands, f: impl Fn(f64, f64) -> f64) -> Bands {
        let mut out = self.0;
        for (v, w) in out.iter_mut().zip(o.0) {
            *v = f(*v, w);
        }
        Bands(out)
    }

    /// Band-by-band product.
    pub fn times(self, o: Bands) -> Bands {
        self.zip(o, |a, b| a * b)
    }

    pub fn scale(self, s: f64) -> Bands {
        self.map(|v| v * s)
    }

    /// Turns a loss in decibels into a linear amplitude gain.
    pub fn loss_db_to_gain(self) -> Bands {
        self.map(db_loss_to_gain)
    }

    /// The plain average of the bands.
    pub fn mean(self) -> f64 {
        let mut s = 0.0;
        for v in self.0 {
            s += v;
        }
        s / BAND_COUNT as f64
    }

    /// The sum of squares, the energy of a set of amplitude gains.
    pub fn energy(self) -> f64 {
        let mut s = 0.0;
        for v in self.0 {
            s += v * v;
        }
        s
    }

    pub fn max_value(self) -> f64 {
        let mut m = f64::NEG_INFINITY;
        for v in self.0 {
            m = m.max(v);
        }
        m
    }

    pub fn is_finite(self) -> bool {
        self.0.iter().all(|v| v.is_finite())
    }

    /// The values as `f32`, for an audio thread.
    pub fn to_f32(self) -> [f32; BAND_COUNT] {
        let mut out = [0.0f32; BAND_COUNT];
        for (o, v) in out.iter_mut().zip(self.0) {
            *o = v as f32;
        }
        out
    }
}

/// A loss in decibels as a linear amplitude gain: 6.02 dB gives 0.5.
pub fn db_loss_to_gain(db: f64) -> f64 {
    libm::pow(10.0, -db / 20.0)
}

/// A linear amplitude gain as decibels (negative for a loss). Zero and
/// below give -200 dB.
pub fn gain_to_db(gain: f64) -> f64 {
    if gain > 1e-10 {
        20.0 * libm::log10(gain)
    } else {
        -200.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decibel_conversions() {
        assert!((db_loss_to_gain(20.0) - 0.1).abs() < 1e-15);
        assert!((db_loss_to_gain(0.0) - 1.0).abs() < 1e-15);
        assert!((gain_to_db(0.5) + 6.0206).abs() < 1e-4);
        assert_eq!(gain_to_db(0.0), -200.0);
    }

    #[test]
    fn bands_maths() {
        let b = Bands([1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0]);
        assert_eq!(b.mean(), 4.0);
        assert_eq!(b.max_value(), 7.0);
        assert_eq!(b.times(Bands::splat(2.0)).0[6], 14.0);
        assert_eq!(Bands::ONE.energy(), 7.0);
    }

    #[test]
    fn exact_centres_match_the_formula() {
        for (k, f) in BAND_EXACT_HZ.iter().enumerate() {
            let want = 1000.0 * libm::pow(10.0, (3 * k as i32 - 9) as f64 / 10.0);
            assert!((f - want).abs() < 1e-9, "{f} {want}");
        }
    }

    #[test]
    fn serializes_as_a_plain_array() {
        let s = serde_json::to_string(&Bands::splat(0.5)).unwrap();
        assert_eq!(s, "[0.5,0.5,0.5,0.5,0.5,0.5,0.5]");
    }
}
