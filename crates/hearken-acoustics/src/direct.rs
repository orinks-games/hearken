//! The straight path from a source to the listener.

use crate::bands::Bands;
use crate::options::{spreading, QueryOptions, MAX_CROSSINGS};
use crate::scene::Scene;
use crate::vec::Vec3;

/// The straight line from source to listener and what it loses on the way.
/// Gains are linear amplitudes relative to the source heard at the
/// reference distance in open air.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DirectPath {
    /// Metres.
    pub distance: f64,
    /// Seconds at the scene's speed of sound.
    pub delay_s: f64,
    /// Spreading loss: reference distance over distance, at most 1.
    pub spreading: f64,
    /// Amplitude left after air absorption, per band.
    pub air: Bands,
    /// Amplitude left after passing through every shape crossed, per band.
    pub transmission: Bands,
    /// spreading * air * transmission.
    pub gain: Bands,
    /// How many shapes the line passes through.
    pub obstacles: u32,
    /// Whether more than [`MAX_CROSSINGS`] shapes were crossed, so the line
    /// counts as fully blocked.
    pub truncated: bool,
}

impl DirectPath {
    /// A path with no sound on it, for invalid positions.
    pub fn silent() -> DirectPath {
        DirectPath {
            distance: 0.0,
            delay_s: 0.0,
            spreading: 0.0,
            air: Bands::ZERO,
            transmission: Bands::ZERO,
            gain: Bands::ZERO,
            obstacles: 0,
            truncated: false,
        }
    }

    /// Whether anything stands in the way.
    pub fn is_blocked(&self) -> bool {
        self.obstacles > 0
    }
}

/// The straight path from `source` to `listener`.
pub fn direct_path(scene: &Scene, source: Vec3, listener: Vec3, opts: &QueryOptions) -> DirectPath {
    let opts = opts.clamped();
    let mut buf = Vec::new();
    direct_with(scene, source, listener, &opts, &mut buf)
}

pub(crate) fn direct_with(
    scene: &Scene,
    source: Vec3,
    listener: Vec3,
    opts: &QueryOptions,
    buf: &mut Vec<u32>,
) -> DirectPath {
    if !(source.is_usable() && listener.is_usable()) {
        return DirectPath::silent();
    }
    let distance = source.distance(listener);
    if !distance.is_finite() {
        return DirectPath::silent();
    }
    let mut transmission = Bands::ONE;
    let mut obstacles = 0u32;
    let mut truncated = false;
    scene.for_each_crossing(source, listener, buf, |_, o| {
        if obstacles >= MAX_CROSSINGS {
            truncated = true;
            return false;
        }
        obstacles += 1;
        transmission = transmission.times(scene.transmission_gain(o.material));
        true
    });
    if truncated {
        transmission = Bands::ZERO;
    }
    let spread = spreading(distance, opts.reference_distance);
    let air = scene.air_gain(distance);
    DirectPath {
        distance,
        delay_s: distance / scene.speed_of_sound(),
        spreading: spread,
        air,
        transmission,
        gain: air.times(transmission).scale(spread),
        obstacles,
        truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bands::db_loss_to_gain;
    use crate::geometry::Shape;
    use crate::material::Builtin;
    use crate::scene::SceneBuilder;

    #[test]
    fn open_air() {
        let s = Scene::default();
        let p = direct_path(
            &s,
            Vec3::new(0.0, 100.0, 1.5),
            Vec3::new(0.0, 0.0, 1.5),
            &QueryOptions::default(),
        );
        assert_eq!(p.distance, 100.0);
        assert!((p.delay_s - 100.0 / s.speed_of_sound()).abs() < 1e-15);
        assert!((p.delay_s - 0.2915).abs() < 0.001, "{}", p.delay_s);
        assert_eq!(p.spreading, 0.01);
        assert_eq!(p.obstacles, 0);
        assert_eq!(p.transmission, Bands::ONE);
        // 8 kHz loses more than 125 Hz.
        assert!(p.air.0[6] < p.air.0[0]);
        let want = db_loss_to_gain(s.air_absorption_db_per_m().0[6] * 100.0) * 0.01;
        assert!((p.gain.0[6] - want).abs() < 1e-15);
    }

    #[test]
    fn through_walls() {
        let mut b = SceneBuilder::new();
        b.add(
            Shape::wall_between(5.0, -5.0, 5.0, 5.0, 0.0, 3.0),
            Builtin::Drywall.id(),
        )
        .unwrap();
        b.add(
            Shape::wall_between(7.0, -5.0, 7.0, 5.0, 0.0, 3.0),
            Builtin::Concrete.id(),
        )
        .unwrap();
        let s = b.build();
        let o = QueryOptions::default();
        let a = Vec3::new(0.0, 0.0, 1.5);
        let one = direct_path(&s, a, Vec3::new(6.0, 0.0, 1.5), &o);
        assert_eq!(one.obstacles, 1);
        let dw = Builtin::Drywall.material().transmission_gain();
        assert_eq!(one.transmission, dw);
        let two = direct_path(&s, a, Vec3::new(10.0, 0.0, 1.5), &o);
        assert_eq!(two.obstacles, 2);
        assert!(two.transmission.0[3] < one.transmission.0[3]);
        // Heavy walls cut high bands more than low ones.
        assert!(two.transmission.0[5] < two.transmission.0[0]);
    }

    #[test]
    fn solids_count_once() {
        let mut b = SceneBuilder::new();
        b.add(
            Shape::cuboid(Vec3::new(2.0, -1.0, 0.0), Vec3::new(3.0, 1.0, 3.0)),
            Builtin::Wood.id(),
        )
        .unwrap();
        let s = b.build();
        let p = direct_path(
            &s,
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(5.0, 0.0, 1.0),
            &QueryOptions::default(),
        );
        assert_eq!(p.obstacles, 1);
    }

    #[test]
    fn source_on_a_wall_face_is_not_behind_it() {
        let mut b = SceneBuilder::new();
        b.add(
            Shape::wall_between(5.0, -5.0, 5.0, 5.0, 0.0, 3.0),
            Builtin::Brick.id(),
        )
        .unwrap();
        let s = b.build();
        let p = direct_path(
            &s,
            Vec3::new(5.0, 0.0, 1.5),
            Vec3::new(0.0, 0.0, 1.5),
            &QueryOptions::default(),
        );
        assert_eq!(p.obstacles, 0);
    }

    #[test]
    fn bad_positions_are_silent() {
        let s = Scene::default();
        let p = direct_path(
            &s,
            Vec3::new(f64::NAN, 0.0, 0.0),
            Vec3::ZERO,
            &QueryOptions::default(),
        );
        assert_eq!(p, DirectPath::silent());
        let p = direct_path(
            &s,
            Vec3::new(1e308, 0.0, 0.0),
            Vec3::new(-1e308, 0.0, 0.0),
            &QueryOptions::default(),
        );
        assert_eq!(p, DirectPath::silent());
    }

    #[test]
    fn many_walls_are_capped() {
        let mut b = SceneBuilder::new();
        for i in 0..100 {
            let x = 1.0 + i as f64 * 0.1;
            b.add(
                Shape::wall_between(x, -1.0, x, 1.0, 0.0, 3.0),
                Builtin::Curtain.id(),
            )
            .unwrap();
        }
        let s = b.build();
        let p = direct_path(
            &s,
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(20.0, 0.0, 1.0),
            &QueryOptions::default(),
        );
        assert!(p.truncated);
        assert_eq!(p.obstacles, MAX_CROSSINGS);
        assert_eq!(p.gain, Bands::ZERO);
    }
}
