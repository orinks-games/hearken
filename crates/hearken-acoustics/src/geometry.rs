//! Shapes a scene is built from, and the exact tests on them.

use crate::vec::Vec3;
use serde::{Deserialize, Serialize};

/// A piece of the world. All lengths in metres.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Shape {
    /// A thin flat panel: the parallelogram with corners `origin`,
    /// `origin + u`, `origin + v` and `origin + u + v`. Sound reflects off
    /// both faces.
    Wall { origin: Vec3, u: Vec3, v: Vec3 },
    /// A solid axis-aligned box.
    Box { min: Vec3, max: Vec3 },
    /// A solid upright cylinder (a pole, a tree trunk, a pillar) standing on
    /// `base` (the centre of its bottom).
    Cylinder {
        base: Vec3,
        radius: f64,
        height: f64,
    },
    /// An endless floor at height `z`; sound above it reflects off it.
    Ground { z: f64 },
    /// An endless ceiling at height `z`; sound below it reflects off it.
    Ceiling { z: f64 },
}

/// Which kind of shape, without its numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShapeKind {
    Wall,
    Box,
    Cylinder,
    Ground,
    Ceiling,
}

/// Why a shape was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeError {
    /// A value is NaN or infinite.
    NotFinite,
    /// A coordinate is farther from the origin than the scene allows.
    TooFar,
    /// Zero or negative size, or a wall with no area.
    Degenerate,
}

impl core::fmt::Display for ShapeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ShapeError::NotFinite => write!(f, "shape has a value that is not a number"),
            ShapeError::TooFar => write!(f, "shape is outside the scene's extent"),
            ShapeError::Degenerate => write!(f, "shape has no size"),
        }
    }
}

/// Smallest size along any axis, metres.
pub const MIN_SIZE: f64 = 1e-3;

impl Shape {
    /// An upright wall from (x0, y0) to (x1, y1), from `z` up `height`.
    pub fn wall_between(x0: f64, y0: f64, x1: f64, y1: f64, z: f64, height: f64) -> Shape {
        Shape::Wall {
            origin: Vec3::new(x0, y0, z),
            u: Vec3::new(x1 - x0, y1 - y0, 0.0),
            v: Vec3::new(0.0, 0.0, height),
        }
    }

    /// A box from its two opposite corners, in any order.
    pub fn cuboid(a: Vec3, b: Vec3) -> Shape {
        Shape::Box {
            min: a.min(b),
            max: a.max(b),
        }
    }

    /// An upright cylinder standing at (x, y) from `z`.
    pub fn pole(x: f64, y: f64, z: f64, radius: f64, height: f64) -> Shape {
        Shape::Cylinder {
            base: Vec3::new(x, y, z),
            radius,
            height,
        }
    }

    pub fn kind(&self) -> ShapeKind {
        match self {
            Shape::Wall { .. } => ShapeKind::Wall,
            Shape::Box { .. } => ShapeKind::Box,
            Shape::Cylinder { .. } => ShapeKind::Cylinder,
            Shape::Ground { .. } => ShapeKind::Ground,
            Shape::Ceiling { .. } => ShapeKind::Ceiling,
        }
    }

    /// Whether the shape is endless (a ground or ceiling).
    pub fn is_unbounded(&self) -> bool {
        matches!(self, Shape::Ground { .. } | Shape::Ceiling { .. })
    }

    /// Checks the numbers, refusing anything farther than `extent` metres.
    pub fn validate(&self, extent: f64) -> Result<(), ShapeError> {
        let finite = |v: Vec3| v.is_finite();
        let near = |v: Vec3| v.x.abs() <= extent && v.y.abs() <= extent && v.z.abs() <= extent;
        match *self {
            Shape::Wall { origin, u, v } => {
                if !(finite(origin) && finite(u) && finite(v)) {
                    return Err(ShapeError::NotFinite);
                }
                if !(near(origin) && near(origin + u) && near(origin + v) && near(origin + u + v)) {
                    return Err(ShapeError::TooFar);
                }
                if u.length() < MIN_SIZE
                    || v.length() < MIN_SIZE
                    || u.cross(v).length() < MIN_SIZE * MIN_SIZE
                {
                    return Err(ShapeError::Degenerate);
                }
            }
            Shape::Box { min, max } => {
                if !(finite(min) && finite(max)) {
                    return Err(ShapeError::NotFinite);
                }
                if !(near(min) && near(max)) {
                    return Err(ShapeError::TooFar);
                }
                let s = max - min;
                if s.x < MIN_SIZE || s.y < MIN_SIZE || s.z < MIN_SIZE {
                    return Err(ShapeError::Degenerate);
                }
            }
            Shape::Cylinder {
                base,
                radius,
                height,
            } => {
                if !(finite(base) && radius.is_finite() && height.is_finite()) {
                    return Err(ShapeError::NotFinite);
                }
                if radius < MIN_SIZE || height < MIN_SIZE {
                    return Err(ShapeError::Degenerate);
                }
                let r = Vec3::new(radius, radius, height);
                if !(near(base - r) && near(base + r)) {
                    return Err(ShapeError::TooFar);
                }
            }
            Shape::Ground { z } | Shape::Ceiling { z } => {
                if !z.is_finite() {
                    return Err(ShapeError::NotFinite);
                }
                if z.abs() > extent {
                    return Err(ShapeError::TooFar);
                }
            }
        }
        Ok(())
    }

    /// The bounding box, or `None` for an endless shape.
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        match *self {
            Shape::Wall { origin, u, v } => {
                let a = origin;
                let b = origin + u;
                let c = origin + v;
                let d = origin + u + v;
                Some((a.min(b).min(c).min(d), a.max(b).max(c).max(d)))
            }
            Shape::Box { min, max } => Some((min, max)),
            Shape::Cylinder {
                base,
                radius,
                height,
            } => Some((
                Vec3::new(base.x - radius, base.y - radius, base.z),
                Vec3::new(base.x + radius, base.y + radius, base.z + height),
            )),
            Shape::Ground { .. } | Shape::Ceiling { .. } => None,
        }
    }

    /// Where the segment from `p` to `q` is inside or crosses the shape, as
    /// the parameter range `(enter, exit)` along it (0 at `p`, 1 at `q`),
    /// clipped to 0..1. A thin wall gives `enter == exit`.
    pub fn segment_overlap(&self, p: Vec3, q: Vec3) -> Option<(f64, f64)> {
        let d = q - p;
        match *self {
            Shape::Wall { origin, u, v } => {
                let t = quad_hit(origin, u, v, p, d)?;
                if (0.0..=1.0).contains(&t) {
                    Some((t, t))
                } else {
                    None
                }
            }
            Shape::Box { min, max } => {
                let (t0, t1) = slab(min, max, p, d)?;
                clip01(t0, t1)
            }
            Shape::Cylinder {
                base,
                radius,
                height,
            } => {
                let (t0, t1) = cylinder_span(base, radius, height, p, d)?;
                clip01(t0, t1)
            }
            Shape::Ground { z } | Shape::Ceiling { z } => {
                if d.z == 0.0 {
                    return None;
                }
                let t = (z - p.z) / d.z;
                if (0.0..=1.0).contains(&t) {
                    Some((t, t))
                } else {
                    None
                }
            }
        }
    }

    /// The first surface a ray from `p` along `dir` meets within
    /// `max_t` (in units of `dir`), and the surface normal there, facing
    /// back towards `p`. From inside a solid, the inside of its far face.
    pub fn ray_hit(&self, p: Vec3, dir: Vec3, max_t: f64) -> Option<(f64, Vec3)> {
        let (t, n) = match *self {
            Shape::Wall { origin, u, v } => {
                let t = quad_hit(origin, u, v, p, dir)?;
                (t, u.cross(v))
            }
            Shape::Box { min, max } => {
                let (t0, t1) = slab(min, max, p, dir)?;
                let t = if t0 > 0.0 { t0 } else { t1 };
                let hit = p + dir * t;
                (t, box_normal(min, max, hit))
            }
            Shape::Cylinder {
                base,
                radius,
                height,
            } => {
                let (t0, t1) = cylinder_span(base, radius, height, p, dir)?;
                let t = if t0 > 0.0 { t0 } else { t1 };
                let hit = p + dir * t;
                (t, cylinder_normal(base, radius, height, hit))
            }
            Shape::Ground { z } | Shape::Ceiling { z } => {
                if dir.z == 0.0 {
                    return None;
                }
                ((z - p.z) / dir.z, Vec3::UP)
            }
        };
        if !(t > 0.0 && t <= max_t) {
            return None;
        }
        let n = n.normalized().unwrap_or(Vec3::UP);
        let n = if n.dot(dir) > 0.0 { -n } else { n };
        Some((t, n))
    }

    /// Whether `p` is inside a solid (box or cylinder). Walls and endless
    /// planes have no inside.
    pub fn contains(&self, p: Vec3) -> bool {
        match *self {
            Shape::Box { min, max } => {
                p.x > min.x
                    && p.x < max.x
                    && p.y > min.y
                    && p.y < max.y
                    && p.z > min.z
                    && p.z < max.z
            }
            Shape::Cylinder {
                base,
                radius,
                height,
            } => {
                let dx = p.x - base.x;
                let dy = p.y - base.y;
                p.z > base.z && p.z < base.z + height && dx * dx + dy * dy < radius * radius
            }
            _ => false,
        }
    }

    /// The point of the shape nearest `p`, the surface's width and height
    /// there, in metres. For a point inside a solid, the nearest point on
    /// its surface. Endless planes report `plane_size` for both.
    pub fn closest_point(&self, p: Vec3, plane_size: f64) -> (Vec3, f64, f64) {
        match *self {
            Shape::Wall { origin, u, v } => (quad_closest(origin, u, v, p), u.length(), v.length()),
            Shape::Box { min, max } => {
                let c = p.max(min).min(max);
                let point = if c == p {
                    // Inside: push out through the nearest face.
                    let dists = [
                        p.x - min.x,
                        max.x - p.x,
                        p.y - min.y,
                        max.y - p.y,
                        p.z - min.z,
                        max.z - p.z,
                    ];
                    let mut best = 0;
                    for i in 1..6 {
                        if dists[i] < dists[best] {
                            best = i;
                        }
                    }
                    let mut q = p;
                    match best {
                        0 => q.x = min.x,
                        1 => q.x = max.x,
                        2 => q.y = min.y,
                        3 => q.y = max.y,
                        4 => q.z = min.z,
                        _ => q.z = max.z,
                    }
                    q
                } else {
                    c
                };
                let s = max - min;
                (point, s.x.max(s.y), s.z)
            }
            Shape::Cylinder {
                base,
                radius,
                height,
            } => {
                let dx = p.x - base.x;
                let dy = p.y - base.y;
                let r = (dx * dx + dy * dy).sqrt();
                let top = base.z + height;
                let z = p.z.max(base.z).min(top);
                let point = if r > radius || r < 1e-12 {
                    let (ux, uy) = if r < 1e-12 {
                        (1.0, 0.0)
                    } else {
                        (dx / r, dy / r)
                    };
                    Vec3::new(base.x + ux * radius, base.y + uy * radius, z)
                } else if p.z > top {
                    Vec3::new(p.x, p.y, top)
                } else if p.z < base.z {
                    Vec3::new(p.x, p.y, base.z)
                } else {
                    // Inside: out through the side or the nearer cap.
                    let side = radius - r;
                    let down = p.z - base.z;
                    let up = top - p.z;
                    if side <= down && side <= up {
                        Vec3::new(base.x + dx / r * radius, base.y + dy / r * radius, p.z)
                    } else if down < up {
                        Vec3::new(p.x, p.y, base.z)
                    } else {
                        Vec3::new(p.x, p.y, top)
                    }
                };
                (point, 2.0 * radius, height)
            }
            Shape::Ground { z } | Shape::Ceiling { z } => {
                (Vec3::new(p.x, p.y, z), plane_size, plane_size)
            }
        }
    }
}

fn clip01(t0: f64, t1: f64) -> Option<(f64, f64)> {
    let a = t0.max(0.0);
    let b = t1.min(1.0);
    if a <= b {
        Some((a, b))
    } else {
        None
    }
}

/// Ray parameter where `p + t d` meets the parallelogram, if it does
/// (any sign of t).
pub(crate) fn quad_hit(origin: Vec3, u: Vec3, v: Vec3, p: Vec3, d: Vec3) -> Option<f64> {
    let n = u.cross(v);
    let denom = d.dot(n);
    let nn = n.dot(n);
    if denom.abs() <= 1e-12 * nn.sqrt() * d.length() || nn == 0.0 {
        return None;
    }
    let t = (origin - p).dot(n) / denom;
    let w = p + d * t - origin;
    let a = w.cross(v).dot(n) / nn;
    let b = u.cross(w).dot(n) / nn;
    if (0.0..=1.0).contains(&a) && (0.0..=1.0).contains(&b) {
        Some(t)
    } else {
        None
    }
}

/// Whether point `q` (assumed on the plane) lies within the parallelogram.
pub(crate) fn quad_contains(origin: Vec3, u: Vec3, v: Vec3, q: Vec3) -> bool {
    let n = u.cross(v);
    let nn = n.dot(n);
    if nn == 0.0 {
        return false;
    }
    let w = q - origin;
    let a = w.cross(v).dot(n) / nn;
    let b = u.cross(w).dot(n) / nn;
    (-1e-9..=1.0 + 1e-9).contains(&a) && (-1e-9..=1.0 + 1e-9).contains(&b)
}

fn quad_closest(origin: Vec3, u: Vec3, v: Vec3, p: Vec3) -> Vec3 {
    let n = match u.cross(v).normalized() {
        Some(n) => n,
        None => return origin,
    };
    let proj = p - n * (p - origin).dot(n);
    if quad_contains(origin, u, v, proj) {
        return proj;
    }
    let corners = [origin, origin + u, origin + u + v, origin + v];
    let mut best = origin;
    let mut best_d = f64::INFINITY;
    for i in 0..4 {
        let a = corners[i];
        let b = corners[(i + 1) % 4];
        let c = segment_closest(a, b, p);
        let d = (c - p).length_sq();
        if d < best_d {
            best_d = d;
            best = c;
        }
    }
    best
}

fn segment_closest(a: Vec3, b: Vec3, p: Vec3) -> Vec3 {
    let ab = b - a;
    let l = ab.length_sq();
    if l == 0.0 {
        return a;
    }
    let t = ((p - a).dot(ab) / l).max(0.0).min(1.0);
    a + ab * t
}

/// Ray-box slab test: the (enter, exit) parameters, any sign.
fn slab(min: Vec3, max: Vec3, p: Vec3, d: Vec3) -> Option<(f64, f64)> {
    let mut t0 = f64::NEG_INFINITY;
    let mut t1 = f64::INFINITY;
    for (lo, hi, o, dd) in [
        (min.x, max.x, p.x, d.x),
        (min.y, max.y, p.y, d.y),
        (min.z, max.z, p.z, d.z),
    ] {
        if dd == 0.0 {
            if o < lo || o > hi {
                return None;
            }
        } else {
            let a = (lo - o) / dd;
            let b = (hi - o) / dd;
            t0 = t0.max(a.min(b));
            t1 = t1.min(a.max(b));
        }
    }
    if t0 <= t1 && t0.is_finite() && t1.is_finite() {
        Some((t0, t1))
    } else {
        None
    }
}

fn box_normal(min: Vec3, max: Vec3, q: Vec3) -> Vec3 {
    let d = [
        ((q.x - min.x).abs(), Vec3::new(-1.0, 0.0, 0.0)),
        ((q.x - max.x).abs(), Vec3::new(1.0, 0.0, 0.0)),
        ((q.y - min.y).abs(), Vec3::new(0.0, -1.0, 0.0)),
        ((q.y - max.y).abs(), Vec3::new(0.0, 1.0, 0.0)),
        ((q.z - min.z).abs(), Vec3::new(0.0, 0.0, -1.0)),
        ((q.z - max.z).abs(), Vec3::new(0.0, 0.0, 1.0)),
    ];
    let mut best = d[0];
    for e in &d[1..] {
        if e.0 < best.0 {
            best = *e;
        }
    }
    best.1
}

/// Ray against a closed upright cylinder: the (enter, exit) parameters.
fn cylinder_span(base: Vec3, r: f64, h: f64, p: Vec3, d: Vec3) -> Option<(f64, f64)> {
    let ox = p.x - base.x;
    let oy = p.y - base.y;
    let a = d.x * d.x + d.y * d.y;
    let c = ox * ox + oy * oy - r * r;
    let (mut t0, mut t1) = if a <= 1e-24 {
        if c > 0.0 {
            return None;
        }
        (f64::NEG_INFINITY, f64::INFINITY)
    } else {
        let b = ox * d.x + oy * d.y;
        let disc = b * b - a * c;
        if disc < 0.0 {
            return None;
        }
        let s = disc.sqrt();
        ((-b - s) / a, (-b + s) / a)
    };
    // Clip by the caps.
    let z0 = base.z;
    let z1 = base.z + h;
    if d.z == 0.0 {
        if p.z < z0 || p.z > z1 {
            return None;
        }
    } else {
        let a = (z0 - p.z) / d.z;
        let b = (z1 - p.z) / d.z;
        t0 = t0.max(a.min(b));
        t1 = t1.min(a.max(b));
    }
    if t0 <= t1 && t0.is_finite() && t1.is_finite() {
        Some((t0, t1))
    } else {
        None
    }
}

fn cylinder_normal(base: Vec3, r: f64, h: f64, q: Vec3) -> Vec3 {
    let dx = q.x - base.x;
    let dy = q.y - base.y;
    let side = ((dx * dx + dy * dy).sqrt() - r).abs();
    let bottom = (q.z - base.z).abs();
    let top = (q.z - base.z - h).abs();
    if side <= bottom && side <= top {
        Vec3::new(dx, dy, 0.0)
    } else if bottom < top {
        Vec3::new(0.0, 0.0, -1.0)
    } else {
        Vec3::UP
    }
}

/// A flat reflecting face of a shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Face {
    pub origin: Vec3,
    pub u: Vec3,
    pub v: Vec3,
    /// Unit normal on the side that reflects.
    pub normal: Vec3,
    /// Whether both sides reflect (a thin wall).
    pub two_sided: bool,
    /// Endless plane: no bounds check.
    pub unbounded: bool,
}

impl Face {
    /// Signed distance of `p` in front of the plane.
    pub fn side(&self, p: Vec3) -> f64 {
        (p - self.origin).dot(self.normal)
    }

    pub fn mirror(&self, p: Vec3) -> Vec3 {
        p - self.normal * (2.0 * self.side(p))
    }

    pub fn contains(&self, q: Vec3) -> bool {
        self.unbounded || quad_contains(self.origin, self.u, self.v, q)
    }

    /// Where the segment from `a` to `b` meets the plane (any point on it).
    pub fn plane_point(&self, a: Vec3, b: Vec3) -> Option<Vec3> {
        let da = self.side(a);
        let db = self.side(b);
        let den = da - db;
        if den.abs() < 1e-12 {
            return None;
        }
        Some(a.lerp(b, da / den))
    }
}

/// Flat faces of a shape that can give a mirror reflection. Cylinders give
/// none (a curved surface scatters).
pub(crate) fn faces(shape: &Shape, out: &mut Vec<Face>) {
    match *shape {
        Shape::Wall { origin, u, v } => {
            if let Some(n) = u.cross(v).normalized() {
                out.push(Face {
                    origin,
                    u,
                    v,
                    normal: n,
                    two_sided: true,
                    unbounded: false,
                });
            }
        }
        Shape::Box { min, max } => {
            let s = max - min;
            let ex = Vec3::new(s.x, 0.0, 0.0);
            let ey = Vec3::new(0.0, s.y, 0.0);
            let ez = Vec3::new(0.0, 0.0, s.z);
            let x1 = Vec3::new(max.x, min.y, min.z);
            let y1 = Vec3::new(min.x, max.y, min.z);
            let z1 = Vec3::new(min.x, min.y, max.z);
            let f = |origin, u, v, n| Face {
                origin,
                u,
                v,
                normal: n,
                two_sided: false,
                unbounded: false,
            };
            out.push(f(min, ey, ez, Vec3::new(-1.0, 0.0, 0.0)));
            out.push(f(x1, ey, ez, Vec3::new(1.0, 0.0, 0.0)));
            out.push(f(min, ex, ez, Vec3::new(0.0, -1.0, 0.0)));
            out.push(f(y1, ex, ez, Vec3::new(0.0, 1.0, 0.0)));
            out.push(f(min, ex, ey, Vec3::new(0.0, 0.0, -1.0)));
            out.push(f(z1, ex, ey, Vec3::UP));
        }
        Shape::Cylinder { .. } => {}
        Shape::Ground { z } => out.push(Face {
            origin: Vec3::new(0.0, 0.0, z),
            u: Vec3::ZERO,
            v: Vec3::ZERO,
            normal: Vec3::UP,
            two_sided: false,
            unbounded: true,
        }),
        Shape::Ceiling { z } => out.push(Face {
            origin: Vec3::new(0.0, 0.0, z),
            u: Vec3::ZERO,
            v: Vec3::ZERO,
            normal: Vec3::new(0.0, 0.0, -1.0),
            two_sided: false,
            unbounded: true,
        }),
    }
}

// cos and sin of k * 45 degrees, exact constants so no trig call is needed.
const OCTAGON: [(f64, f64); 8] = [
    (1.0, 0.0),
    (
        core::f64::consts::FRAC_1_SQRT_2,
        core::f64::consts::FRAC_1_SQRT_2,
    ),
    (0.0, 1.0),
    (
        -core::f64::consts::FRAC_1_SQRT_2,
        core::f64::consts::FRAC_1_SQRT_2,
    ),
    (-1.0, 0.0),
    (
        -core::f64::consts::FRAC_1_SQRT_2,
        -core::f64::consts::FRAC_1_SQRT_2,
    ),
    (0.0, -1.0),
    (
        core::f64::consts::FRAC_1_SQRT_2,
        -core::f64::consts::FRAC_1_SQRT_2,
    ),
];
// 1 / cos(22.5 degrees): an octagon with this circumradius encloses the
// unit circle.
const OCTAGON_OUTSET: f64 = 1.082_392_200_292_393_9;

/// Points in the horizontal plane at height `z`, just outside the shape's
/// corners, `offset` metres clear: the places a path can bend round it.
pub(crate) fn corner_points(shape: &Shape, offset: f64, z: f64, out: &mut Vec<Vec3>) {
    match *shape {
        Shape::Wall { origin, u, v } => {
            let corners = [origin, origin + u, origin + u + v, origin + v];
            let n = u.cross(v).normalized().unwrap_or(Vec3::UP);
            if n.z.abs() < 0.5 {
                // Mostly upright: its footprint is a line. Find its two ends
                // along the horizontal direction of the wall.
                let h = Vec3::new(-n.y, n.x, 0.0)
                    .normalized()
                    .unwrap_or(Vec3::new(1.0, 0.0, 0.0));
                let mut lo = corners[0];
                let mut hi = corners[0];
                for c in &corners[1..] {
                    if c.dot(h) < lo.dot(h) {
                        lo = *c;
                    }
                    if c.dot(h) > hi.dot(h) {
                        hi = *c;
                    }
                }
                let nh = Vec3::new(n.x, n.y, 0.0)
                    .normalized()
                    .unwrap_or(Vec3::new(0.0, 1.0, 0.0));
                for (end, out_dir) in [(lo, -h), (hi, h)] {
                    for side in [1.0, -1.0] {
                        let p = end + out_dir * offset + nh * (side * offset);
                        out.push(Vec3::new(p.x, p.y, z));
                    }
                }
            } else {
                // Mostly flat: treat its footprint as a four-sided patch.
                let c = (corners[0] + corners[2]) * 0.5;
                for k in corners {
                    let dir = Vec3::new(k.x - c.x, k.y - c.y, 0.0)
                        .normalized()
                        .unwrap_or(Vec3::new(1.0, 0.0, 0.0));
                    out.push(Vec3::new(
                        k.x + dir.x * offset * core::f64::consts::SQRT_2,
                        k.y + dir.y * offset * core::f64::consts::SQRT_2,
                        z,
                    ));
                }
            }
        }
        Shape::Box { min, max } => {
            for (x, sx) in [(min.x, -1.0), (max.x, 1.0)] {
                for (y, sy) in [(min.y, -1.0), (max.y, 1.0)] {
                    out.push(Vec3::new(x + sx * offset, y + sy * offset, z));
                }
            }
        }
        Shape::Cylinder { base, radius, .. } => {
            let r = (radius + offset) * OCTAGON_OUTSET;
            for (c, s) in OCTAGON {
                out.push(Vec3::new(base.x + c * r, base.y + s * r, z));
            }
        }
        Shape::Ground { .. } | Shape::Ceiling { .. } => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wall() -> Shape {
        Shape::wall_between(0.0, -1.0, 0.0, 1.0, 0.0, 3.0)
    }

    #[test]
    fn wall_segment_crossing() {
        let w = wall();
        let p = Vec3::new(-1.0, 0.0, 1.0);
        let q = Vec3::new(1.0, 0.0, 1.0);
        assert_eq!(w.segment_overlap(p, q), Some((0.5, 0.5)));
        // Past the end of the wall.
        assert_eq!(
            w.segment_overlap(Vec3::new(-1.0, 2.0, 1.0), Vec3::new(1.0, 2.0, 1.0)),
            None
        );
        // Over the top.
        assert_eq!(
            w.segment_overlap(Vec3::new(-1.0, 0.0, 4.0), Vec3::new(1.0, 0.0, 4.0)),
            None
        );
        // Parallel to it.
        assert_eq!(
            w.segment_overlap(Vec3::new(0.0, -5.0, 1.0), Vec3::new(0.0, 5.0, 1.0)),
            None
        );
        // Short of it.
        assert_eq!(
            w.segment_overlap(Vec3::new(-2.0, 0.0, 1.0), Vec3::new(-1.0, 0.0, 1.0)),
            None
        );
    }

    #[test]
    fn box_and_cylinder_spans() {
        let b = Shape::cuboid(Vec3::new(1.0, -1.0, 0.0), Vec3::new(2.0, 1.0, 2.0));
        let (t0, t1) = b
            .segment_overlap(Vec3::new(0.0, 0.0, 1.0), Vec3::new(4.0, 0.0, 1.0))
            .unwrap();
        assert_eq!((t0, t1), (0.25, 0.5));
        let c = Shape::pole(2.0, 0.0, 0.0, 0.5, 2.0);
        let (t0, t1) = c
            .segment_overlap(Vec3::new(0.0, 0.0, 1.0), Vec3::new(4.0, 0.0, 1.0))
            .unwrap();
        assert!((t0 - 0.375).abs() < 1e-12 && (t1 - 0.625).abs() < 1e-12);
        assert!(c
            .segment_overlap(Vec3::new(0.0, 1.0, 1.0), Vec3::new(4.0, 1.0, 1.0))
            .is_none());
        assert!(c.contains(Vec3::new(2.0, 0.1, 1.0)));
        assert!(!c.contains(Vec3::new(2.0, 0.1, 3.0)));
    }

    #[test]
    fn ray_hits_face_towards_origin() {
        let b = Shape::cuboid(Vec3::new(1.0, -1.0, 0.0), Vec3::new(2.0, 1.0, 2.0));
        let (t, n) = b
            .ray_hit(Vec3::new(0.0, 0.0, 1.0), Vec3::new(1.0, 0.0, 0.0), 10.0)
            .unwrap();
        assert_eq!(t, 1.0);
        assert_eq!(n, Vec3::new(-1.0, 0.0, 0.0));
        // From inside, the far face.
        let (t, _) = b
            .ray_hit(Vec3::new(1.5, 0.0, 1.0), Vec3::new(1.0, 0.0, 0.0), 10.0)
            .unwrap();
        assert_eq!(t, 0.5);
        let g = Shape::Ground { z: 0.0 };
        let (t, n) = g
            .ray_hit(Vec3::new(0.0, 0.0, 2.0), Vec3::new(0.0, 0.0, -1.0), 10.0)
            .unwrap();
        assert_eq!((t, n), (2.0, Vec3::UP));
        assert!(g
            .ray_hit(Vec3::new(0.0, 0.0, 2.0), Vec3::new(0.0, 0.0, 1.0), 10.0)
            .is_none());
    }

    #[test]
    fn validation() {
        let e = 1000.0;
        assert_eq!(wall().validate(e), Ok(()));
        assert_eq!(
            Shape::wall_between(0.0, 0.0, 0.0, 0.0, 0.0, 3.0).validate(e),
            Err(ShapeError::Degenerate)
        );
        assert_eq!(
            Shape::pole(f64::NAN, 0.0, 0.0, 1.0, 1.0).validate(e),
            Err(ShapeError::NotFinite)
        );
        assert_eq!(
            Shape::pole(0.0, 0.0, 0.0, -1.0, 1.0).validate(e),
            Err(ShapeError::Degenerate)
        );
        assert_eq!(
            Shape::Box {
                min: Vec3::new(1.0, 0.0, 0.0),
                max: Vec3::new(0.0, 1.0, 1.0)
            }
            .validate(e),
            Err(ShapeError::Degenerate)
        );
        assert_eq!(
            Shape::Ground { z: 1e7 }.validate(e),
            Err(ShapeError::TooFar)
        );
    }

    #[test]
    fn closest_points() {
        let (p, w, h) = wall().closest_point(Vec3::new(-2.0, 5.0, 1.0), 0.0);
        assert_eq!(p, Vec3::new(0.0, 1.0, 1.0));
        assert_eq!((w, h), (2.0, 3.0));
        let c = Shape::pole(0.0, 0.0, 0.0, 1.0, 2.0);
        let (p, _, _) = c.closest_point(Vec3::new(3.0, 0.0, 1.0), 0.0);
        assert_eq!(p, Vec3::new(1.0, 0.0, 1.0));
        let b = Shape::cuboid(Vec3::ZERO, Vec3::new(2.0, 2.0, 2.0));
        let (p, _, _) = b.closest_point(Vec3::new(1.0, 1.9, 1.0), 0.0);
        assert_eq!(p, Vec3::new(1.0, 2.0, 1.0));
    }

    #[test]
    fn face_mirror() {
        let mut f = Vec::new();
        faces(&wall(), &mut f);
        let m = f[0].mirror(Vec3::new(-1.0, 0.0, 1.0));
        assert_eq!(m, Vec3::new(1.0, 0.0, 1.0));
    }
}
