//! Surfaces close to the listener, so a game can make silent walls and
//! poles audible.

use crate::geometry::ShapeKind;
use crate::options::clamp_f;
use crate::scene::{MaterialId, ObjectId, Probe, Scene};
use crate::vec::Vec3;

/// Most surfaces one query returns.
pub const MAX_NEARBY: usize = 256;
/// Most shape tests one query makes, finding the shapes in range and
/// checking which are visible. Once spent, the surfaces left count as
/// hidden and the result is marked truncated.
pub const MAX_NEARBY_SHAPE_TESTS: usize = 200_000;

/// Settings for [`nearby_surfaces`]. Clamped before use.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NearbyOptions {
    /// How far to look, metres (0 to 100).
    pub radius: f64,
    /// Most surfaces returned, nearest first.
    pub max: usize,
    /// Whether grounds and ceilings count.
    pub include_planes: bool,
}

impl Default for NearbyOptions {
    fn default() -> Self {
        NearbyOptions {
            radius: 5.0,
            max: 16,
            include_planes: false,
        }
    }
}

impl NearbyOptions {
    pub fn clamped(self) -> NearbyOptions {
        NearbyOptions {
            radius: clamp_f(self.radius, 0.0, 100.0, 5.0),
            max: self.max.min(MAX_NEARBY),
            include_planes: self.include_planes,
        }
    }
}

/// One surface near the listener.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NearbySurface {
    pub object: ObjectId,
    pub material: MaterialId,
    pub kind: ShapeKind,
    /// The surface's nearest point.
    pub point: Vec3,
    /// Unit vector from the listener to that point; zero when touching it.
    pub direction: Vec3,
    /// Metres.
    pub distance: f64,
    /// The surface's size, metres: a wall's length and height, a box's
    /// longer side and height, a cylinder's diameter and height. Grounds and
    /// ceilings give twice the search radius.
    pub width_m: f64,
    pub height_m: f64,
    /// Whether the listener can see the nearest point.
    pub visible: bool,
}

/// The surfaces found, nearest first.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Nearby {
    pub surfaces: Vec<NearbySurface>,
    /// Whether more were in range than `max`, or the visibility checks
    /// ran out of shape tests.
    pub truncated: bool,
}

/// The surfaces within `opts.radius` of `listener`, nearest first (ties by
/// id).
pub fn nearby_surfaces(scene: &Scene, listener: Vec3, opts: &NearbyOptions) -> Nearby {
    let opts = opts.clamped();
    let mut out = Nearby::default();
    if !listener.is_usable() || opts.max == 0 {
        return out;
    }
    let r = opts.radius;
    let rv = Vec3::new(r, r, r);
    // The gather is charged too: one test per shape in range.
    let mut probe = Probe::new(MAX_NEARBY_SHAPE_TESTS);
    let mut ids = Vec::new();
    if !scene.gather_within(listener - rv, listener + rv, &mut probe, &mut ids) {
        out.truncated = true;
        return out;
    }
    let mut found = Vec::new();
    for &i in &ids {
        let o = &scene.objects()[i as usize];
        let kind = o.shape.kind();
        match kind {
            ShapeKind::Ground | ShapeKind::Ceiling if !opts.include_planes => continue,
            _ => {}
        }
        let (point, width_m, height_m) = o.shape.closest_point(listener, 2.0 * r);
        // A ground is only near from above, a ceiling from below.
        if (kind == ShapeKind::Ground && listener.z < point.z)
            || (kind == ShapeKind::Ceiling && listener.z > point.z)
        {
            continue;
        }
        let distance = listener.distance(point);
        if distance > r {
            continue;
        }
        found.push(NearbySurface {
            object: crate::scene::ObjectId(i),
            material: o.material,
            kind,
            point,
            direction: (point - listener).normalized().unwrap_or(Vec3::ZERO),
            distance,
            width_m,
            height_m,
            visible: false,
        });
    }
    // Nearest first, ties by id: a total order, so picking the nearest
    // `max` before sorting gives the same list as sorting everything.
    let order = |a: &NearbySurface, b: &NearbySurface| {
        a.distance
            .total_cmp(&b.distance)
            .then(a.object.cmp(&b.object))
    };
    if found.len() > opts.max {
        found.select_nth_unstable_by(opts.max, order);
        found.truncate(opts.max);
        out.truncated = true;
    }
    found.sort_unstable_by(order);
    for s in found.iter_mut() {
        s.visible = !scene.blocked(listener, s.point, &[s.object.0], true, &mut probe);
    }
    out.truncated |= probe.spent();
    out.surfaces = found;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Shape;
    use crate::material::Builtin;
    use crate::scene::SceneBuilder;

    #[test]
    fn finds_wall_and_pole_in_order() {
        let mut b = SceneBuilder::new();
        b.add(
            Shape::wall_between(-5.0, 2.0, 5.0, 2.0, 0.0, 3.0),
            Builtin::Brick.id(),
        )
        .unwrap();
        b.add(Shape::pole(1.0, 0.0, 0.0, 0.1, 3.0), Builtin::Metal.id())
            .unwrap();
        b.add(Shape::pole(50.0, 0.0, 0.0, 0.1, 3.0), Builtin::Metal.id())
            .unwrap();
        b.add(Shape::Ground { z: 0.0 }, Builtin::Grass.id())
            .unwrap();
        let s = b.build();
        let lis = Vec3::new(0.0, 0.0, 1.5);
        let n = nearby_surfaces(&s, lis, &NearbyOptions::default());
        assert_eq!(n.surfaces.len(), 2);
        let pole = n.surfaces[0];
        assert_eq!(pole.kind, ShapeKind::Cylinder);
        assert!((pole.distance - 0.9).abs() < 1e-12);
        assert_eq!(pole.direction, Vec3::new(1.0, 0.0, 0.0));
        assert!((pole.width_m - 0.2).abs() < 1e-12);
        let wall = n.surfaces[1];
        assert_eq!(wall.kind, ShapeKind::Wall);
        assert_eq!(wall.distance, 2.0);
        assert_eq!(wall.material, Builtin::Brick.id());
        assert!(wall.visible && pole.visible);

        let n = nearby_surfaces(
            &s,
            lis,
            &NearbyOptions {
                include_planes: true,
                ..NearbyOptions::default()
            },
        );
        assert_eq!(n.surfaces.len(), 3);
        let g = n.surfaces[1];
        assert_eq!(g.kind, ShapeKind::Ground);
        assert_eq!(g.distance, 1.5);
        assert_eq!(g.direction, Vec3::new(0.0, 0.0, -1.0));
    }

    #[test]
    fn hidden_surfaces_are_marked() {
        let mut b = SceneBuilder::new();
        b.add(
            Shape::wall_between(-5.0, 1.0, 5.0, 1.0, 0.0, 3.0),
            Builtin::Brick.id(),
        )
        .unwrap();
        b.add(Shape::pole(0.0, 2.0, 0.0, 0.2, 3.0), Builtin::Wood.id())
            .unwrap();
        let s = b.build();
        let n = nearby_surfaces(&s, Vec3::new(0.0, 0.0, 1.5), &NearbyOptions::default());
        assert_eq!(n.surfaces.len(), 2);
        assert!(n.surfaces[0].visible);
        assert!(!n.surfaces[1].visible);
    }

    #[test]
    fn capped() {
        let mut b = SceneBuilder::new();
        for i in 0..50 {
            b.add(
                Shape::pole(i as f64 * 0.1, 1.0, 0.0, 0.01, 3.0),
                Builtin::Wood.id(),
            )
            .unwrap();
        }
        let s = b.build();
        let n = nearby_surfaces(
            &s,
            Vec3::new(0.0, 0.0, 1.5),
            &NearbyOptions {
                max: 5,
                ..NearbyOptions::default()
            },
        );
        assert_eq!(n.surfaces.len(), 5);
        assert!(n.truncated);
        assert!(n
            .surfaces
            .windows(2)
            .all(|w| w[0].distance <= w[1].distance));
    }
}
