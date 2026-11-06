//! Early reflections by the image-source method, order 1 and 2.
//!
//! Each flat face near the listener mirrors the source; a mirror image
//! whose path really bounces off the face (inside its bounds, on the right
//! side, with nothing in the way of either leg) is a reflection. Order 2
//! mirrors the first image in a second face.

use crate::bands::Bands;
use crate::geometry::{faces, quad_contains, Face};
use crate::options::{spreading, QueryOptions};
use crate::scene::{MaterialId, ObjectId, Probe, Scene};
use crate::vec::Vec3;
use core::cmp::Ordering;
use std::collections::BinaryHeap;

/// One early reflection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reflection {
    /// 1 or 2 bounces.
    pub order: u8,
    /// Bounce points, in order from the source; only the first `order` are
    /// used.
    pub points: [Vec3; 2],
    /// The shapes bounced off.
    pub objects: [ObjectId; 2],
    /// Their materials.
    pub materials: [MaterialId; 2],
    /// Unit vector from the listener towards the last bounce point: where
    /// the reflection is heard from.
    pub direction: Vec3,
    /// Length of the whole path, metres.
    pub distance: f64,
    /// Seconds at the scene's speed of sound.
    pub delay_s: f64,
    /// Spreading * air * each surface's specular reflection, per band.
    pub gain: Bands,
}

/// The reflections found, strongest first.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Reflections {
    pub list: Vec<Reflection>,
    /// Whether some were dropped by the face or reflection caps.
    pub truncated: bool,
    /// Shape tests the search made.
    pub shape_tests: usize,
}

/// A face in the nearest-faces heap, ordered by distance then by when it
/// was found: a total order, so the faces kept are the same everywhere.
struct Near {
    seq: u32,
    cand: Candidate,
}

impl Ord for Near {
    fn cmp(&self, other: &Self) -> Ordering {
        self.cand
            .distance
            .total_cmp(&other.cand.distance)
            .then(self.seq.cmp(&other.seq))
    }
}

impl PartialOrd for Near {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Near {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Near {}

#[derive(Clone, Copy)]
struct Candidate {
    face: Face,
    object: u32,
    material: MaterialId,
    distance: f64,
}

/// Which side of `f` a point is on: +1, -1, or 0 when on the plane (or
/// behind a one-sided face).
fn side(f: &Face, p: Vec3) -> f64 {
    let d = f.side(p);
    if d > 1e-6 {
        1.0
    } else if d < -1e-6 && f.two_sided {
        -1.0
    } else {
        0.0
    }
}

/// The early reflections from `source` to `listener`.
pub fn early_reflections(
    scene: &Scene,
    source: Vec3,
    listener: Vec3,
    opts: &QueryOptions,
) -> Reflections {
    let opts = opts.clamped();
    let mut out = Reflections::default();
    if opts.reflection_order == 0
        || opts.max_reflections == 0
        || !(source.is_usable() && listener.is_usable())
    {
        return out;
    }
    let r = opts.reflection_radius;
    let rv = Vec3::new(r, r, r);
    let mut probe = Probe::new(opts.max_shape_tests);
    let mut ids = Vec::new();
    if !scene.gather_within(listener - rv, listener + rv, &mut probe, &mut ids) {
        out.truncated = true;
        out.shape_tests = probe.used();
        return out;
    }
    // The nearest faces, kept in a heap of at most `max_reflection_faces`
    // so a crowd of shapes costs no memory. Ties go to the earlier face
    // (id order, then face order).
    let keep = opts.max_reflection_faces;
    let mut heap: BinaryHeap<Near> = BinaryHeap::with_capacity(keep.min(64));
    let mut seq = 0u32;
    let mut fs = Vec::new();
    for &i in &ids {
        let o = &scene.objects()[i as usize];
        fs.clear();
        faces(&o.shape, &mut fs);
        // Each face looked at costs a test.
        if !probe.charge(fs.len()) {
            out.truncated = true;
            break;
        }
        for f in &fs {
            let d = if f.unbounded {
                f.side(listener).abs()
            } else {
                face_distance(f, listener)
            };
            if d > r {
                continue;
            }
            let near = Near {
                seq,
                cand: Candidate {
                    face: *f,
                    object: i,
                    material: o.material,
                    distance: d,
                },
            };
            seq = seq.saturating_add(1);
            if heap.len() < keep {
                heap.push(near);
            } else {
                out.truncated = true;
                if let Some(mut top) = heap.peek_mut() {
                    if near.cmp(&top) == Ordering::Less {
                        *top = near;
                    }
                }
            }
        }
    }
    let cands: Vec<Candidate> = heap.into_sorted_vec().into_iter().map(|n| n.cand).collect();

    let mut found: Vec<(Reflection, usize, usize)> = Vec::new();
    for (ai, a) in cands.iter().enumerate() {
        let sa = side(&a.face, source);
        if sa == 0.0 || side(&a.face, listener) != sa {
            continue;
        }
        let img = a.face.mirror(source);
        // First order.
        if let Some(p) = a.face.plane_point(img, listener) {
            if a.face.contains(p)
                && !scene.blocked(source, p, &[a.object], true, &mut probe)
                && !scene.blocked(p, listener, &[a.object], true, &mut probe)
            {
                let len = img.distance(listener);
                let gain = path_gain(scene, len, &opts).times(scene.specular_gain(a.material));
                found.push((make(1, [p, p], [a, a], listener, len, gain, scene), ai, ai));
            }
        }
        if opts.reflection_order < 2 {
            continue;
        }
        // Second order: source -> a -> b -> listener.
        for (bi, b) in cands.iter().enumerate() {
            if bi == ai {
                continue;
            }
            let sb = side(&b.face, img);
            if sb == 0.0 || side(&b.face, listener) != sb {
                continue;
            }
            let img2 = b.face.mirror(img);
            let Some(p2) = b.face.plane_point(img2, listener) else {
                continue;
            };
            if !b.face.contains(p2) || side(&a.face, p2) != sa {
                continue;
            }
            let Some(p1) = a.face.plane_point(img, p2) else {
                continue;
            };
            if !a.face.contains(p1) || side(&b.face, p1) != sb {
                continue;
            }
            if scene.blocked(source, p1, &[a.object], true, &mut probe)
                || scene.blocked(p1, p2, &[a.object, b.object], true, &mut probe)
                || scene.blocked(p2, listener, &[b.object], true, &mut probe)
            {
                continue;
            }
            let len = img2.distance(listener);
            let gain = path_gain(scene, len, &opts)
                .times(scene.specular_gain(a.material))
                .times(scene.specular_gain(b.material));
            found.push((
                make(2, [p1, p2], [a, b], listener, len, gain, scene),
                ai,
                bi,
            ));
        }
    }
    out.truncated |= probe.spent();
    out.shape_tests = probe.used();
    // Strongest first; ties by order then faces, so the order is fixed.
    found.sort_by(|x, y| {
        y.0.gain
            .energy()
            .total_cmp(&x.0.gain.energy())
            .then(x.0.order.cmp(&y.0.order))
            .then(x.1.cmp(&y.1))
            .then(x.2.cmp(&y.2))
    });
    if found.len() > opts.max_reflections {
        found.truncate(opts.max_reflections);
        out.truncated = true;
    }
    out.list = found.into_iter().map(|f| f.0).collect();
    out
}

fn path_gain(scene: &Scene, len: f64, opts: &QueryOptions) -> Bands {
    scene
        .air_gain(len)
        .scale(spreading(len, opts.reference_distance))
}

fn make(
    order: u8,
    points: [Vec3; 2],
    by: [&Candidate; 2],
    listener: Vec3,
    len: f64,
    gain: Bands,
    scene: &Scene,
) -> Reflection {
    let last = points[(order - 1) as usize];
    Reflection {
        order,
        points,
        objects: [ObjectId(by[0].object), ObjectId(by[1].object)],
        materials: [by[0].material, by[1].material],
        direction: (last - listener).normalized().unwrap_or(Vec3::ZERO),
        distance: len,
        delay_s: len / scene.speed_of_sound(),
        gain,
    }
}

/// Distance from `p` to the nearest point of a bounded face.
fn face_distance(f: &Face, p: Vec3) -> f64 {
    let proj = p - f.normal * f.side(p);
    if quad_contains(f.origin, f.u, f.v, proj) {
        return f.side(p).abs();
    }
    let c = [
        f.origin,
        f.origin + f.u,
        f.origin + f.u + f.v,
        f.origin + f.v,
    ];
    let mut best = f64::INFINITY;
    for i in 0..4 {
        let a = c[i];
        let ab = c[(i + 1) % 4] - a;
        let l = ab.length_sq();
        let t = if l > 0.0 {
            ((p - a).dot(ab) / l).max(0.0).min(1.0)
        } else {
            0.0
        };
        best = best.min((a + ab * t).distance(p));
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Shape;
    use crate::material::Builtin;
    use crate::scene::SceneBuilder;

    fn opts(order: u8) -> QueryOptions {
        QueryOptions {
            reflection_order: order,
            ..QueryOptions::default()
        }
    }

    #[test]
    fn one_wall_one_reflection() {
        let mut b = SceneBuilder::new();
        b.add(
            Shape::wall_between(-10.0, 2.0, 10.0, 2.0, 0.0, 3.0),
            Builtin::Concrete.id(),
        )
        .unwrap();
        let s = b.build();
        let src = Vec3::new(-1.0, 0.0, 1.5);
        let lis = Vec3::new(1.0, 0.0, 1.5);
        let r = early_reflections(&s, src, lis, &opts(1));
        assert_eq!(r.list.len(), 1);
        let f = r.list[0];
        assert_eq!(f.order, 1);
        // Image at (-1, 4): path length sqrt(4 + 16).
        assert!((f.distance - 20f64.sqrt()).abs() < 1e-12);
        assert!((f.points[0].x).abs() < 1e-12 && (f.points[0].y - 2.0).abs() < 1e-12);
        // Heard from the wall's side, ahead-left of the listener.
        assert!(f.direction.y > 0.0 && f.direction.x < 0.0);
        let spec = Builtin::Concrete.material().specular_gain();
        let want = spec.0[3] * s.air_gain(f.distance).0[3] / f.distance;
        assert!((f.gain.0[3] - want).abs() < 1e-15);
    }

    #[test]
    fn short_wall_misses() {
        let mut b = SceneBuilder::new();
        b.add(
            Shape::wall_between(5.0, 2.0, 6.0, 2.0, 0.0, 3.0),
            Builtin::Concrete.id(),
        )
        .unwrap();
        let s = b.build();
        let r = early_reflections(
            &s,
            Vec3::new(-1.0, 0.0, 1.5),
            Vec3::new(1.0, 0.0, 1.5),
            &opts(1),
        );
        assert!(r.list.is_empty());
    }

    #[test]
    fn corridor_second_order() {
        let mut b = SceneBuilder::new();
        let c = Builtin::Concrete.id();
        b.add(Shape::wall_between(-20.0, 2.0, 20.0, 2.0, 0.0, 3.0), c)
            .unwrap();
        b.add(Shape::wall_between(-20.0, -2.0, 20.0, -2.0, 0.0, 3.0), c)
            .unwrap();
        b.add(Shape::Ground { z: 0.0 }, c).unwrap();
        let s = b.build();
        let src = Vec3::new(-3.0, 0.5, 1.5);
        let lis = Vec3::new(3.0, -0.5, 1.5);
        let r1 = early_reflections(&s, src, lis, &opts(1));
        assert_eq!(r1.list.len(), 3, "two walls and the floor");
        let r2 = early_reflections(&s, src, lis, &opts(2));
        // Plus wall-wall both ways and wall-floor / floor-wall pairs.
        assert!(r2.list.len() > 3);
        assert!(r2.list.iter().any(|r| r.order == 2));
        // Strongest first.
        assert!(r2
            .list
            .windows(2)
            .all(|w| w[0].gain.energy() >= w[1].gain.energy()));
        // Second-order paths are longer than the first-order ones off the
        // same walls.
        for r in r2.list.iter().filter(|r| r.order == 2) {
            assert!(r.distance > src.distance(lis));
        }
    }

    #[test]
    fn blocked_leg_is_dropped() {
        let mut b = SceneBuilder::new();
        let c = Builtin::Concrete.id();
        b.add(Shape::wall_between(-10.0, 2.0, 10.0, 2.0, 0.0, 3.0), c)
            .unwrap();
        // A screen between the listener and the bounce point.
        b.add(Shape::wall_between(0.5, 0.5, 1.5, 0.5, 0.0, 3.0), c)
            .unwrap();
        let s = b.build();
        let r = early_reflections(
            &s,
            Vec3::new(-1.0, 0.0, 1.5),
            Vec3::new(1.0, 0.0, 1.5),
            &opts(1),
        );
        assert!(r.list.iter().all(|f| f.objects[0] != ObjectId(0)), "{r:?}");
    }

    #[test]
    fn box_faces_reflect_only_outward() {
        let mut b = SceneBuilder::new();
        b.add(
            Shape::cuboid(Vec3::new(-5.0, 2.0, 0.0), Vec3::new(5.0, 4.0, 3.0)),
            Builtin::Brick.id(),
        )
        .unwrap();
        let s = b.build();
        let r = early_reflections(
            &s,
            Vec3::new(-1.0, 0.0, 1.5),
            Vec3::new(1.0, 0.0, 1.5),
            &opts(2),
        );
        assert_eq!(r.list.len(), 1);
    }

    #[test]
    fn caps_apply() {
        let mut b = SceneBuilder::new();
        for i in 0..100 {
            let y = 2.0 + i as f64 * 0.01;
            b.add(
                Shape::wall_between(-10.0 - i as f64, y, 10.0, y, 0.0, 3.0),
                Builtin::Glass.id(),
            )
            .unwrap();
        }
        let s = b.build();
        let o = QueryOptions {
            max_reflection_faces: 4,
            max_reflections: 2,
            ..QueryOptions::default()
        };
        let r = early_reflections(&s, Vec3::new(-1.0, 0.0, 1.5), Vec3::new(1.0, 0.0, 1.5), &o);
        assert!(r.truncated);
        assert!(r.list.len() <= 2);
    }
}
