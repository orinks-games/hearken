//! Air: the speed of sound and how much each band fades per metre.

use crate::bands::{Bands, BAND_COUNT, BAND_EXACT_HZ};
use serde::{Deserialize, Serialize};

/// Standard sea-level pressure, kPa.
pub const STANDARD_PRESSURE_KPA: f64 = 101.325;

/// Carbon dioxide mole fraction used for the speed of sound (about 400 ppm,
/// today's outdoor air; Cramer's paper used 314 ppm).
const CO2_MOLE_FRACTION: f64 = 0.0004;

/// The air the sound travels through.
///
/// Values outside the supported range are clamped by [`Atmosphere::clamped`]
/// before use: temperature -40 to 60 C, humidity 0 to 100 %, pressure 50 to
/// 110 kPa. Non-finite values fall back to the default (20 C, 50 %,
/// 101.325 kPa).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Atmosphere {
    /// Air temperature in degrees Celsius.
    pub temperature_c: f64,
    /// Relative humidity in percent, 0 to 100.
    pub relative_humidity: f64,
    /// Atmospheric pressure in kilopascals.
    pub pressure_kpa: f64,
}

impl Default for Atmosphere {
    fn default() -> Self {
        Atmosphere {
            temperature_c: 20.0,
            relative_humidity: 50.0,
            pressure_kpa: STANDARD_PRESSURE_KPA,
        }
    }
}

fn clamp_or(v: f64, lo: f64, hi: f64, fallback: f64) -> f64 {
    if v.is_finite() {
        v.max(lo).min(hi)
    } else {
        fallback
    }
}

impl Atmosphere {
    pub fn new(temperature_c: f64, relative_humidity: f64, pressure_kpa: f64) -> Atmosphere {
        Atmosphere {
            temperature_c,
            relative_humidity,
            pressure_kpa,
        }
    }

    /// This atmosphere with every value made finite and in range.
    pub fn clamped(self) -> Atmosphere {
        let d = Atmosphere::default();
        Atmosphere {
            temperature_c: clamp_or(self.temperature_c, -40.0, 60.0, d.temperature_c),
            relative_humidity: clamp_or(self.relative_humidity, 0.0, 100.0, d.relative_humidity),
            pressure_kpa: clamp_or(self.pressure_kpa, 50.0, 110.0, d.pressure_kpa),
        }
    }

    /// Speed of sound in m/s.
    ///
    /// O. Cramer, "The variation of the specific heat ratio and the speed of
    /// sound in air with temperature, pressure, humidity, and CO2
    /// concentration", J. Acoust. Soc. Am. 93(5), 2510-2516 (1993), eq. (15)
    /// and Table III. Water vapour mole fraction from the saturation vapour
    /// pressure and enhancement factor of R. S. Davis, "Equation for the
    /// determination of the density of moist air", Metrologia 29 (1992).
    /// Cramer states the fit for 0 to 30 C; outside that it extrapolates.
    pub fn speed_of_sound(&self) -> f64 {
        let a = self.clamped();
        let t = a.temperature_c;
        let p = a.pressure_kpa * 1000.0;
        let tk = t + 273.15;
        let psv =
            libm::exp(1.2378847e-5 * tk * tk - 1.9121316e-2 * tk + 33.93711047 - 6.3431645e3 / tk);
        let enhance = 1.00062 + 3.14e-8 * p + 5.6e-7 * t * t;
        let xw = (a.relative_humidity / 100.0) * enhance * psv / p;
        let xc = CO2_MOLE_FRACTION;
        let t2 = t * t;
        331.5024 + 0.603055 * t - 0.000528 * t2
            + (51.471935 + 0.1495874 * t - 0.000782 * t2) * xw
            + (-1.82e-7 + 3.73e-8 * t - 2.93e-10 * t2) * p
            + (-85.20931 - 0.228525 * t + 5.91e-5 * t2) * xc
            - 2.835149 * xw * xw
            - 2.15e-13 * p * p
            + 29.179762 * xc * xc
            + 0.000486 * xw * p * xc
    }

    /// Pure-tone air absorption at `frequency_hz`, in dB per metre.
    ///
    /// ISO 9613-1:1993, "Acoustics - Attenuation of sound during propagation
    /// outdoors - Part 1: Calculation of the absorption of sound by the
    /// atmosphere", equations (3) to (5) and Annex B for the molar
    /// concentration of water vapour. The standard covers -20 to 50 C;
    /// outside that it extrapolates.
    pub fn absorption_db_per_m(&self, frequency_hz: f64) -> f64 {
        let a = self.clamped();
        let f = if frequency_hz.is_finite() {
            frequency_hz.max(1.0).min(100_000.0)
        } else {
            1000.0
        };
        let t = a.temperature_c + 273.15;
        let t0 = 293.15; // reference temperature
        let t01 = 273.16; // triple-point isotherm
        let pr = STANDARD_PRESSURE_KPA;
        let pa = a.pressure_kpa;
        let p_rel = pa / pr;
        // Annex B: molar concentration of water vapour, percent.
        let c = -6.8346 * libm::pow(t01 / t, 1.261) + 4.6151;
        let psat_rel = libm::pow(10.0, c);
        let h = a.relative_humidity * psat_rel / p_rel;
        // Relaxation frequencies of oxygen and nitrogen, eq. (3) and (4).
        let fr_o = p_rel * (24.0 + 4.04e4 * h * (0.02 + h) / (0.391 + h));
        let t_rel = t / t0;
        let fr_n = p_rel
            * (1.0 / t_rel.sqrt())
            * (9.0 + 280.0 * h * libm::exp(-4.170 * (libm::pow(t_rel, -1.0 / 3.0) - 1.0)));
        // Eq. (5).
        let f2 = f * f;
        let classical = 1.84e-11 * (1.0 / p_rel) * t_rel.sqrt();
        let t_rel_m52 = libm::pow(t_rel, -2.5);
        let oxygen = 0.01275 * libm::exp(-2239.1 / t) / (fr_o + f2 / fr_o);
        let nitrogen = 0.1068 * libm::exp(-3352.0 / t) / (fr_n + f2 / fr_n);
        8.686 * f2 * (classical + t_rel_m52 * (oxygen + nitrogen))
    }

    /// Air absorption for each band, in dB per metre, taken at the exact
    /// band centres as ISO 9613-2 Table 2 does.
    pub fn absorption_bands_db_per_m(&self) -> Bands {
        let mut out = [0.0; BAND_COUNT];
        for (o, f) in out.iter_mut().zip(BAND_EXACT_HZ) {
            *o = self.absorption_db_per_m(f);
        }
        Bands(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_of_sound_dry_air_at_20c() {
        let c = Atmosphere::new(20.0, 0.0, STANDARD_PRESSURE_KPA).speed_of_sound();
        // The textbook ideal-gas value is 343.2 m/s; Cramer's real-gas fit
        // sits within a few tenths of that.
        assert!((c - 343.2).abs() < 0.25, "{c}");
    }

    #[test]
    fn speed_of_sound_dry_air_at_0c() {
        let c = Atmosphere::new(0.0, 0.0, STANDARD_PRESSURE_KPA).speed_of_sound();
        // Cramer (1993): 331.45 m/s for dry air at 0 C.
        assert!((c - 331.45).abs() < 0.05, "{c}");
    }

    #[test]
    fn humid_air_is_faster() {
        let dry = Atmosphere::new(20.0, 0.0, STANDARD_PRESSURE_KPA).speed_of_sound();
        let wet = Atmosphere::new(20.0, 100.0, STANDARD_PRESSURE_KPA).speed_of_sound();
        // Saturated air at 20 C is a little over 1 m/s faster.
        assert!(wet - dry > 1.0 && wet - dry < 1.6, "{dry} {wet}");
    }

    #[test]
    fn warm_air_is_faster() {
        let cold = Atmosphere::new(-10.0, 50.0, STANDARD_PRESSURE_KPA).speed_of_sound();
        let hot = Atmosphere::new(35.0, 50.0, STANDARD_PRESSURE_KPA).speed_of_sound();
        assert!(cold < 326.0 && hot > 352.0, "{cold} {hot}");
    }

    fn db_per_km(t: f64, rh: f64) -> [f64; BAND_COUNT] {
        let b = Atmosphere::new(t, rh, STANDARD_PRESSURE_KPA).absorption_bands_db_per_m();
        b.map(|v| v * 1000.0).0
    }

    fn close(got: [f64; BAND_COUNT], want: [f64; BAND_COUNT]) {
        for (i, (g, w)) in got.iter().zip(want).enumerate() {
            // The table rounds to one decimal below 10 dB/km and to 3
            // significant figures above; allow that plus a hair.
            let tol = if w < 10.0 { 0.06 } else { w * 0.006 };
            assert!((g - w).abs() <= tol, "band {i}: got {g}, want {w}");
        }
    }

    // ISO 9613-2:1996 Table 2 gives these attenuation coefficients (dB/km),
    // computed from ISO 9613-1 at the exact octave band centres,
    // 125 Hz to 8 kHz.
    #[test]
    fn iso_9613_table_10c_70rh() {
        close(
            db_per_km(10.0, 70.0),
            [0.4, 1.0, 1.9, 3.7, 9.7, 32.8, 117.0],
        );
    }

    #[test]
    fn iso_9613_table_20c_70rh() {
        close(db_per_km(20.0, 70.0), [0.3, 1.1, 2.8, 5.0, 9.0, 22.9, 76.6]);
    }

    #[test]
    fn iso_9613_table_30c_70rh() {
        close(
            db_per_km(30.0, 70.0),
            [0.3, 1.0, 3.1, 7.4, 12.7, 23.1, 59.3],
        );
    }

    #[test]
    fn iso_9613_table_15c_20rh() {
        close(
            db_per_km(15.0, 20.0),
            [0.6, 1.2, 2.7, 8.2, 28.2, 88.8, 202.0],
        );
    }

    #[test]
    fn iso_9613_table_15c_50rh() {
        close(
            db_per_km(15.0, 50.0),
            [0.5, 1.2, 2.2, 4.2, 10.8, 36.2, 129.0],
        );
    }

    #[test]
    fn iso_9613_table_15c_80rh() {
        close(db_per_km(15.0, 80.0), [0.3, 1.1, 2.4, 4.1, 8.3, 23.7, 82.8]);
    }

    #[test]
    fn bad_values_are_clamped_not_propagated() {
        let a = Atmosphere::new(f64::NAN, f64::INFINITY, -5.0);
        let c = a.speed_of_sound();
        assert!(c.is_finite());
        assert!(a.absorption_bands_db_per_m().is_finite());
        assert!(Atmosphere::new(1e9, -1e9, 1e9).speed_of_sound().is_finite());
        assert!(Atmosphere::default()
            .absorption_db_per_m(f64::NAN)
            .is_finite());
    }
}
