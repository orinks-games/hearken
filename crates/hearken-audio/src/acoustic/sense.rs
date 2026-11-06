//! Obstacle sense: silent walls and poles heard through a quiet noise
//! floor round the listener.
//!
//! The floor is eight independent noise sources, one per 45-degree sector
//! round the head. A wall in a sector blocks that sector's noise and sends
//! back the opposite sector's, a little later (twice its distance over the
//! speed of sound). The two copies of the same noise comb-filter, which
//! colours the sound and lifts the bass as the wall gets close. A pole
//! shadows the high frequencies of its sector. It plays as a [`Feed`], so
//! any backend that plays feeds plays it, under the ambience bus.

use std::f32::consts::PI;
use std::sync::{Arc, Mutex};

use crate::feed::{Feed, FeedRead};

use super::filters::{Biquad, Coeffs, Line, Noise};
use super::mailbox::Mailbox;

pub(crate) const SECTORS: usize = 8;
const SECTOR_WORDS: usize = 6;
const LEVEL: usize = SECTORS * SECTOR_WORDS;
const WORDS: usize = LEVEL + 1;
/// The rate the floor is made at; the backend resamples.
const RATE: f32 = 48_000.0;
/// Longest reflection delay, seconds (a wall 10 m away).
const MAX_DELAY_S: f32 = 0.06;
const BLOCK: usize = 64;
/// The floor's level at full strength, per sector: about -45 dB RMS in
/// each ear.
pub(crate) const FULL_LEVEL: f32 = 0.01;

/// What the floor does in one sector.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Sector {
    /// How much of the sector's own noise gets through, 0..1.
    pub open: f32,
    /// How much of what gets through is low-passed (a pole's shadow).
    pub cover: f32,
    pub cover_hz: f32,
    /// The opposite sector's noise reflected back, as a gain.
    pub reflect: f32,
    pub delay_s: f32,
    /// The reflection's low-pass, from the surface's absorption.
    pub reflect_hz: f32,
}

impl Sector {
    pub(crate) const CLEAR: Sector = Sector {
        open: 1.0,
        cover: 0.0,
        cover_hz: 20_000.0,
        reflect: 0.0,
        delay_s: 0.0,
        reflect_hz: 20_000.0,
    };
}

struct SectorDsp {
    noise: Noise,
    pink: [f32; 3],
    line: Line,
    cover: Biquad,
    reflect: Biquad,
    /// Smoothed open, cover and reflect gains.
    gains: [f32; 3],
    delay: f32,
    pan: [f32; 2],
}

struct State {
    seen: u32,
    p: [f32; WORDS],
    sectors: [SectorDsp; SECTORS],
    level: f32,
    left: usize,
}

/// The obstacle-sense noise floor: a [`Feed`] the pool starts on the
/// ambience bus, and the game thread re-aims each frame.
pub struct ObstacleSense {
    mailbox: Mailbox<WORDS>,
    state: Mutex<State>,
}

impl std::fmt::Debug for ObstacleSense {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ObstacleSense")
    }
}

impl ObstacleSense {
    pub(crate) fn new() -> Arc<ObstacleSense> {
        let mut p = [0.0; WORDS];
        for s in 0..SECTORS {
            write_sector(&mut p, s, Sector::CLEAR);
        }
        let size = (MAX_DELAY_S * RATE) as usize + 4;
        let sectors = std::array::from_fn(|i| {
            // Sector 0 is straight ahead, then clockwise.
            let az = i as f32 * PI / 4.0;
            // Soft pans: each ear hears every sector a little, as a head
            // does, so a reflection and the noise it copies meet in both
            // ears and comb-filter there.
            let pan = super::voice::pan_gains(0.6 * az.sin());
            SectorDsp {
                noise: Noise::new(0x9E37_79B9u32.wrapping_mul(i as u32 + 1)),
                pink: [0.0; 3],
                line: Line::new(size),
                cover: Biquad::default(),
                reflect: Biquad::default(),
                gains: [1.0, 0.0, 0.0],
                delay: 0.0,
                pan,
            }
        });
        Arc::new(ObstacleSense {
            mailbox: Mailbox::new(&p),
            state: Mutex::new(State {
                seen: 0,
                p,
                sectors,
                level: 0.0,
                left: 0,
            }),
        })
    }

    /// Sets every sector and the overall level (0 is silent).
    pub(crate) fn set(&self, sectors: &[Sector; SECTORS], level: f32) {
        let mut p = [0.0; WORDS];
        for (i, s) in sectors.iter().enumerate() {
            write_sector(&mut p, i, *s);
        }
        p[LEVEL] = if level.is_finite() {
            level.clamp(0.0, 0.1)
        } else {
            0.0
        };
        self.mailbox.write(&p);
    }
}

fn write_sector(p: &mut [f32; WORDS], i: usize, s: Sector) {
    let o = i * SECTOR_WORDS;
    let f = |v: f32, lo: f32, hi: f32, or: f32| if v.is_finite() { v.clamp(lo, hi) } else { or };
    p[o] = f(s.open, 0.0, 1.0, 1.0);
    p[o + 1] = f(s.cover, 0.0, 1.0, 0.0);
    p[o + 2] = f(s.cover_hz, 50.0, 20_000.0, 20_000.0);
    p[o + 3] = f(s.reflect, 0.0, 1.0, 0.0);
    p[o + 4] = f(s.delay_s, 0.0, MAX_DELAY_S, 0.0);
    p[o + 5] = f(s.reflect_hz, 50.0, 20_000.0, 20_000.0);
}

impl State {
    fn step(&mut self) {
        let a = 0.1;
        let target_level = self.p[LEVEL];
        self.level += (target_level - self.level) * a;
        for (i, s) in self.sectors.iter_mut().enumerate() {
            let o = i * SECTOR_WORDS;
            let t = [self.p[o], self.p[o + 1], self.p[o + 3]];
            for k in 0..3 {
                s.gains[k] += (t[k] - s.gains[k]) * a;
            }
            let d = self.p[o + 4] * RATE;
            // A big jump (a new wall) is taken at once, small ones glide.
            s.delay = if (d - s.delay).abs() > 48.0 {
                d
            } else {
                s.delay + (d - s.delay) * a
            };
            s.cover.c = Coeffs::lowpass(self.p[o + 2], std::f32::consts::FRAC_1_SQRT_2, RATE);
            s.reflect.c = Coeffs::lowpass(self.p[o + 5], std::f32::consts::FRAC_1_SQRT_2, RATE);
            s.cover.flush();
            s.reflect.flush();
        }
    }

    fn frame(&mut self) -> [f32; 2] {
        // Every sector's noise first, so reflections can read the
        // opposite sector's line.
        for s in &mut self.sectors {
            let w = s.noise.next();
            // A cheap pink-ish tilt: three one-poles.
            s.pink[1] = 0.985 * s.pink[1] + 0.032 * w;
            s.pink[2] = 0.950 * s.pink[2] + 0.048 * w;
            let n = s.pink[1] + s.pink[2] + 0.1 * w;
            s.line.write(n);
        }
        let mut out = [0.0f32; 2];
        for i in 0..SECTORS {
            let opposite = (i + SECTORS / 2) % SECTORS;
            let refl_in = self.sectors[opposite].line.read(self.sectors[i].delay);
            let s = &mut self.sectors[i];
            let n = s.line.read(0.0);
            let [open, cover, reflect] = s.gains;
            let direct = open * (n * (1.0 - cover) + cover * s.cover.run(n));
            let back = reflect * s.reflect.run(refl_in);
            let y = (direct + back) * self.level;
            out[0] += y * s.pan[0];
            out[1] += y * s.pan[1];
        }
        out
    }
}

impl Feed for ObstacleSense {
    fn read(&self, out: &mut [[f32; 2]]) -> FeedRead {
        // Only the audio thread reads, so this lock is never contended;
        // if it ever were, the block is silence rather than a wait.
        let Ok(mut st) = self.state.try_lock() else {
            out.fill([0.0; 2]);
            return FeedRead {
                frames: out.len(),
                sample_rate: RATE as u32,
            };
        };
        let st = &mut *st;
        let mut p = st.p;
        if self.mailbox.read(&mut p, &mut st.seen) {
            st.p = p;
        }
        for frame in out.iter_mut() {
            if st.left == 0 {
                st.left = BLOCK;
                st.step();
            }
            st.left -= 1;
            *frame = st.frame();
        }
        FeedRead {
            frames: out.len(),
            sample_rate: RATE as u32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(sense: &ObstacleSense, frames: usize) -> Vec<[f32; 2]> {
        let mut out = vec![[0.0; 2]; frames];
        for chunk in out.chunks_mut(256) {
            sense.read(chunk);
        }
        out
    }

    fn energy(x: &[[f32; 2]], side: usize) -> f64 {
        x.iter().map(|f| f64::from(f[side]).powi(2)).sum()
    }

    #[test]
    fn a_pole_on_the_left_darkens_the_left() {
        let sense = ObstacleSense::new();
        let mut sectors = [Sector::CLEAR; SECTORS];
        // Sector 6 is due left.
        sectors[6].cover = 1.0;
        sectors[6].cover_hz = 800.0;
        sense.set(&sectors, FULL_LEVEL);
        let out = render(&sense, 96_000);
        let out = &out[24_000..];
        // High-pass both ears crudely by differencing.
        let hi = |side: usize| -> f64 {
            out.windows(2)
                .map(|w| f64::from(w[1][side] - w[0][side]).powi(2))
                .sum()
        };
        assert!(hi(0) < hi(1) * 0.9, "left {} right {}", hi(0), hi(1));
    }

    #[test]
    fn silent_at_zero_level_and_bounded() {
        let sense = ObstacleSense::new();
        let out = render(&sense, 4800);
        assert!(energy(&out, 0) == 0.0);
        let mut sectors = [Sector::CLEAR; SECTORS];
        sectors[2] = Sector {
            open: f32::NAN,
            cover: 9.0,
            cover_hz: -1.0,
            reflect: 1e9,
            delay_s: f32::INFINITY,
            reflect_hz: 0.0,
        };
        sense.set(&sectors, 1e9);
        let out = render(&sense, 48_000);
        assert!(out.iter().all(|f| f[0].is_finite() && f[0].abs() < 1.0));
    }
}
