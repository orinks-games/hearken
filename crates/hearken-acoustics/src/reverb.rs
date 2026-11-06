//! A late reverb estimate for the space round the listener.
//!
//! A fixed set of rays (a Fibonacci sphere) goes out from the listener.
//! Each ray's hit distance r gives a cone of the room: volume
//! (omega r^3 / 3) and wall area (omega r^2 / cos theta), with omega the
//! solid angle per ray. Summed over the sphere these give the volume and
//! surface of the space the listener can see. Each ray that meets nothing
//! counts as an opening (absorption 1) the size of an average hit patch.
//! Reverberation time follows from Sabine and Eyring.

use crate::bands::{Bands, BAND_COUNT};
use crate::options::clamp_f;
use crate::scene::Scene;
use crate::vec::Vec3;

/// Most rays per estimate.
pub const MAX_REVERB_RAYS: usize = 4096;
/// Fewest rays per estimate.
pub const MIN_REVERB_RAYS: usize = 16;
/// Longest reverberation time reported, seconds.
pub const MAX_RT60_S: f64 = 30.0;
/// Highest wet gain reported.
pub const MAX_WET_GAIN: f64 = 2.0;
/// Most shape tests one estimate makes. A ray with more shapes near it
/// than its share is left out, and the estimate uses the rays cast.
pub const MAX_REVERB_SHAPE_TESTS: usize = 1_000_000;
/// Most shape tests one ray may use.
const MAX_RAY_SHAPE_TESTS: usize = MAX_REVERB_SHAPE_TESTS / 16;

/// 24 ln(10): RT60 = 24 ln(10) V / (c A).
const SABINE: f64 = 55.262_042_231_857_1;
/// pi (3 - sqrt 5), the golden angle in radians.
const GOLDEN_ANGLE: f64 = 2.399_963_229_728_653_3;
/// 10 log10(e): dB per neper of intensity.
const DB_PER_NEPER: f64 = 4.342_944_819_032_518;
/// Steepest incidence counted for a wall patch (cos 78 degrees); grazing
/// rays would otherwise make the area estimate blow up.
const MIN_COS: f64 = 0.2;

/// Settings for [`estimate_reverb`]. Clamped before use.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReverbOptions {
    /// How many rays, 16 to 4096.
    pub rays: usize,
    /// How far a ray goes before it counts as open air, metres.
    pub max_distance: f64,
    /// Distance at which direct sound is at 0 dB, metres, as in
    /// [`crate::QueryOptions::reference_distance`].
    pub reference_distance: f64,
}

impl Default for ReverbOptions {
    fn default() -> Self {
        ReverbOptions {
            rays: 256,
            max_distance: 200.0,
            reference_distance: 1.0,
        }
    }
}

impl ReverbOptions {
    pub fn clamped(self) -> ReverbOptions {
        let d = ReverbOptions::default();
        ReverbOptions {
            rays: self.rays.clamp(MIN_REVERB_RAYS, MAX_REVERB_RAYS),
            max_distance: clamp_f(self.max_distance, 1.0, 10_000.0, d.max_distance),
            reference_distance: clamp_f(self.reference_distance, 0.01, 100.0, 1.0),
        }
    }
}

/// The reverberant field round a listener.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct ReverbEstimate {
    /// Estimated volume of the space, m3.
    pub volume_m3: f64,
    /// Estimated surface, openings included, m2.
    pub surface_m2: f64,
    /// Share of rays that met nothing, 0 (closed room) to 1 (open field).
    pub open_fraction: f64,
    /// Mean free path 4V/S, metres.
    pub mean_free_path_m: f64,
    /// Area-weighted absorption coefficient, openings counting as 1.
    pub mean_absorption: Bands,
    /// Sabine reverberation time per band, seconds.
    pub rt60_sabine: Bands,
    /// Eyring reverberation time per band, seconds.
    pub rt60_eyring: Bands,
    /// Reverberant level per band as an amplitude, relative to the direct
    /// sound at the reference distance: sqrt(16 pi r0^2 / R), R the room
    /// constant S a / (1 - a). Capped at [`MAX_WET_GAIN`].
    pub wet_gain: Bands,
    /// Distance at which direct and reverberant sound are equal at 1 kHz,
    /// metres.
    pub critical_distance_m: f64,
    /// Rays cast and rays that hit.
    pub rays: usize,
    pub hits: usize,
    /// Whether some rays were left out because the shape-test allowance
    /// ([`MAX_REVERB_SHAPE_TESTS`]) ran short.
    pub truncated: bool,
}

impl ReverbEstimate {
    /// The reverberation time to use: Eyring, which stays right in
    /// absorbent rooms where Sabine runs long.
    pub fn rt60(&self) -> Bands {
        self.rt60_eyring
    }
}

/// The fixed ray directions: `n` points of a Fibonacci sphere, the same on
/// every platform.
pub fn ray_directions(n: usize) -> Vec<Vec3> {
    let n = n.clamp(1, MAX_REVERB_RAYS);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let z = 1.0 - (2.0 * i as f64 + 1.0) / n as f64;
        let r = (1.0 - z * z).max(0.0).sqrt();
        let phi = GOLDEN_ANGLE * i as f64;
        out.push(Vec3::new(r * libm::cos(phi), r * libm::sin(phi), z));
    }
    out
}

/// Estimates the late reverb heard at `listener`.
pub fn estimate_reverb(scene: &Scene, listener: Vec3, opts: &ReverbOptions) -> ReverbEstimate {
    let opts = opts.clamped();
    let mut est = ReverbEstimate {
        rays: opts.rays,
        ..ReverbEstimate::default()
    };
    if !listener.is_usable() {
        return est;
    }
    let dirs = ray_directions(opts.rays);
    let omega = 4.0 * core::f64::consts::PI / opts.rays as f64;
    let mut buf = Vec::new();
    let mut volume = 0.0;
    let mut surface = 0.0;
    let mut absorbed = [0.0; BAND_COUNT];
    let mut left = MAX_REVERB_SHAPE_TESTS;
    let mut cast = 0usize;
    for d in &dirs {
        let limit = left.min(MAX_RAY_SHAPE_TESTS);
        let Some(hit) = scene.ray_cast_with(listener, *d, opts.max_distance, &mut buf, limit)
        else {
            est.truncated = true;
            continue;
        };
        // Every ray costs at least one.
        left -= buf.len().max(1).min(left);
        cast += 1;
        if let Some((t, n, id)) = hit {
            est.hits += 1;
            volume += omega * t * t * t / 3.0;
            let cos = (-n.dot(*d)).max(MIN_COS);
            let area = omega * t * t / cos;
            surface += area;
            let m = scene.objects()[id.0 as usize].material;
            let a = scene
                .material(m)
                .map(|m| m.absorption)
                .unwrap_or(Bands::ONE);
            for (s, a) in absorbed.iter_mut().zip(a.0) {
                *s += area * a;
            }
        }
    }
    est.rays = cast;
    if est.hits == 0 {
        est.open_fraction = 1.0;
        return est;
    }
    let mut hits = est.hits as f64;
    let mut missed = (cast - est.hits) as f64;
    if cast < opts.rays {
        // The rays cast stand for the whole sphere.
        let k = opts.rays as f64 / cast as f64;
        volume *= k;
        surface *= k;
        for s in absorbed.iter_mut() {
            *s *= k;
        }
        hits *= k;
        missed *= k;
    }
    // Each ray that met nothing stands for an opening the size of an
    // average hit patch, enclosing an average cone of volume.
    let open_area = missed * surface / hits;
    volume += missed * volume / hits;
    surface += open_area;
    for s in absorbed.iter_mut() {
        *s += open_area;
    }
    est.open_fraction = missed / opts.rays as f64;
    est.volume_m3 = volume;
    est.surface_m2 = surface;
    est.mean_free_path_m = 4.0 * volume / surface;

    let c = scene.speed_of_sound();
    let air = scene.air_absorption_db_per_m();
    let r0 = opts.reference_distance;
    let mut mean = [0.0; BAND_COUNT];
    let mut sab = [0.0; BAND_COUNT];
    let mut eyr = [0.0; BAND_COUNT];
    let mut wet = [0.0; BAND_COUNT];
    for b in 0..BAND_COUNT {
        let a = (absorbed[b] / surface).max(0.0).min(1.0);
        mean[b] = a;
        // Air: 4 m V, m the intensity attenuation per metre.
        let air_term = 4.0 * (air.0[b] / DB_PER_NEPER) * volume;
        let a_eff = a.min(0.999);
        sab[b] = rt(volume, surface * a + air_term, c);
        eyr[b] = rt(volume, -surface * libm::log(1.0 - a_eff) + air_term, c);
        let room = surface * a_eff / (1.0 - a_eff) + air_term;
        wet[b] = if room > 0.0 {
            (16.0 * core::f64::consts::PI * r0 * r0 / room)
                .sqrt()
                .min(MAX_WET_GAIN)
        } else {
            MAX_WET_GAIN
        };
    }
    est.mean_absorption = Bands(mean);
    est.rt60_sabine = Bands(sab);
    est.rt60_eyring = Bands(eyr);
    est.wet_gain = Bands(wet);
    let a = mean[3].min(0.999);
    let room = surface * a / (1.0 - a);
    est.critical_distance_m = (room / (16.0 * core::f64::consts::PI)).sqrt();
    est
}

fn rt(volume: f64, absorption_area: f64, c: f64) -> f64 {
    if absorption_area > 0.0 {
        (SABINE * volume / (c * absorption_area)).min(MAX_RT60_S)
    } else {
        MAX_RT60_S
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Shape;
    use crate::material::{Builtin, Material};
    use crate::scene::SceneBuilder;

    fn room(material: crate::scene::MaterialId) -> Scene {
        room_in(SceneBuilder::new(), material)
    }

    /// A 10 x 8 x 3 m shoebox: four walls, floor and ceiling.
    fn room_in(mut b: SceneBuilder, material: crate::scene::MaterialId) -> Scene {
        let (x, y, z) = (10.0, 8.0, 3.0);
        b.add(Shape::wall_between(0.0, 0.0, x, 0.0, 0.0, z), material)
            .unwrap();
        b.add(Shape::wall_between(x, 0.0, x, y, 0.0, z), material)
            .unwrap();
        b.add(Shape::wall_between(x, y, 0.0, y, 0.0, z), material)
            .unwrap();
        b.add(Shape::wall_between(0.0, y, 0.0, 0.0, 0.0, z), material)
            .unwrap();
        b.add(Shape::Ground { z: 0.0 }, material).unwrap();
        b.add(Shape::Ceiling { z }, material).unwrap();
        b.build()
    }

    #[test]
    fn directions_are_unit_and_balanced() {
        let d = ray_directions(512);
        let mut sum = Vec3::ZERO;
        for v in &d {
            assert!((v.length() - 1.0).abs() < 1e-12);
            sum = sum + *v;
        }
        assert!(sum.length() < 0.05 * d.len() as f64, "{sum:?}");
    }

    #[test]
    fn shoebox_volume_and_surface() {
        let s = room(Builtin::Plaster.id());
        let e = estimate_reverb(
            &s,
            Vec3::new(5.0, 4.0, 1.5),
            &ReverbOptions {
                rays: 4096,
                ..ReverbOptions::default()
            },
        );
        assert_eq!(e.hits, 4096);
        assert_eq!(e.open_fraction, 0.0);
        assert!((e.volume_m3 / 240.0 - 1.0).abs() < 0.03, "{}", e.volume_m3);
        // True surface 268 m2; the grazing cap trims a little.
        assert!(
            (e.surface_m2 / 268.0 - 1.0).abs() < 0.08,
            "{}",
            e.surface_m2
        );
    }

    #[test]
    fn rt60_matches_sabine_by_hand() {
        let mut b = SceneBuilder::new();
        let m = b
            .add_material(&Material {
                name: "flat".into(),
                absorption: Bands::splat(0.1),
                scattering: Bands::ZERO,
                transmission_loss_db: Bands::ZERO,
            })
            .unwrap();
        let s = room_in(b, m);
        let e = estimate_reverb(
            &s,
            Vec3::new(5.0, 4.0, 1.5),
            &ReverbOptions {
                rays: 4096,
                ..ReverbOptions::default()
            },
        );
        // Sabine by hand, no air: 0.161 * 240 / (268 * 0.1) = 1.44 s.
        let by_hand = SABINE * 240.0 / (s.speed_of_sound() * 26.8);
        assert!(
            (e.rt60_sabine.0[0] / by_hand - 1.0).abs() < 0.08,
            "{} {by_hand}",
            e.rt60_sabine.0[0]
        );
        // Eyring is shorter than Sabine.
        assert!(e.rt60_eyring.0[0] < e.rt60_sabine.0[0]);
        // Air takes the top band down.
        assert!(e.rt60_sabine.0[6] < e.rt60_sabine.0[0]);
        assert!(e.critical_distance_m > 0.5 && e.critical_distance_m < 3.0);
    }

    #[test]
    fn carpet_room_is_drier_than_tile_room() {
        let p = Vec3::new(5.0, 4.0, 1.5);
        let o = ReverbOptions::default();
        let tile = estimate_reverb(&room(Builtin::Tile.id()), p, &o);
        let carpet = estimate_reverb(&room(Builtin::Carpet.id()), p, &o);
        assert!(carpet.rt60().0[4] < tile.rt60().0[4] / 5.0);
        assert!(carpet.wet_gain.0[4] < tile.wet_gain.0[4]);
    }

    #[test]
    fn open_field_has_no_reverb() {
        let e = estimate_reverb(&Scene::default(), Vec3::ZERO, &ReverbOptions::default());
        assert_eq!(e.hits, 0);
        assert_eq!(e.open_fraction, 1.0);
        assert_eq!(e.rt60(), Bands::ZERO);
        // Just a ground: half the rays hit and the opening dominates. The
        // statistical model still gives a tail (grazing hits on the ground
        // make the "room" look large), but its level is tiny.
        let mut b = SceneBuilder::new();
        b.add(Shape::Ground { z: 0.0 }, Builtin::Grass.id())
            .unwrap();
        let e = estimate_reverb(
            &b.build(),
            Vec3::new(0.0, 0.0, 1.5),
            &ReverbOptions::default(),
        );
        assert!((e.open_fraction - 0.5).abs() < 0.01);
        assert!(e.rt60().0[3] < 0.6, "{:?}", e.rt60());
        assert!(e.wet_gain.max_value() < 0.06, "{:?}", e.wet_gain);
    }

    #[test]
    fn bad_options_are_clamped() {
        let o = ReverbOptions {
            rays: usize::MAX,
            max_distance: f64::NAN,
            reference_distance: -1.0,
        }
        .clamped();
        assert_eq!(o.rays, MAX_REVERB_RAYS);
        assert_eq!(o.max_distance, 200.0);
        assert_eq!(o.reference_distance, 0.01);
        let e = estimate_reverb(&Scene::default(), Vec3::new(f64::NAN, 0.0, 0.0), &o);
        assert_eq!(e.hits, 0);
    }
}
