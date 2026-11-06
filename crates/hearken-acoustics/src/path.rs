//! Paths round obstacles: the way a sound bends round a corner, and a
//! walking route from one place to another.
//!
//! Both search a visibility graph in the horizontal plane. Its nodes sit
//! just outside the corners of every shape near the two ends; edges join
//! nodes that can see each other. The shortest route wins (A*, ties broken
//! by node order, so the result is the same everywhere). The search runs
//! at the height midway between the ends; the corners of the route found
//! then get heights on a straight slope from one end to the other.

use crate::bands::{Bands, BAND_COUNT, BAND_EXACT_HZ};
use crate::geometry::corner_points;
use crate::options::{
    clamp_f, spreading, QueryOptions, MAX_PATH_NODES, MAX_SHAPE_TESTS, MAX_VISIBILITY_TESTS,
};
use crate::scene::{Probe, Scene};
use crate::vec::Vec3;

/// How far outside a corner a sound path bends, metres.
const SOUND_OFFSET: f64 = 0.05;

/// A sound's way round obstacles when the straight line is blocked.
#[derive(Clone, Debug, PartialEq)]
pub struct DiffractedPath {
    /// The corners the sound bends round, from the source to the listener.
    pub corners: Vec<Vec3>,
    /// Length of the whole bent path, metres.
    pub length: f64,
    /// Seconds at the scene's speed of sound.
    pub delay_s: f64,
    /// Spreading over the whole length (reference over length, at most 1).
    pub spreading: f64,
    /// Amplitude left after air absorption, per band.
    pub air: Bands,
    /// Amplitude left after bending round the corners, per band.
    pub diffraction: Bands,
    /// spreading * air * diffraction.
    pub gain: Bands,
    /// Where the sound seems to come from: the corner nearest the listener.
    pub arrival: Vec3,
}

/// The result of a diffraction search.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct DiffractionSearch {
    /// The path, if the line is blocked and a way round was found.
    pub path: Option<DiffractedPath>,
    /// Whether the search hit a node or test cap, so a path may exist that
    /// was not found or a shorter one was missed.
    pub truncated: bool,
    /// Shape tests the search made.
    pub shape_tests: usize,
}

/// Settings for [`navigate`]. Clamped before use.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NavOptions {
    /// How far the route keeps from obstacles, metres (a body's radius).
    pub clearance: f64,
    /// Most corner points used.
    pub max_nodes: usize,
    /// Most line-of-sight tests.
    pub max_visibility_tests: usize,
    /// How far beyond the box round start and goal to look, metres.
    pub margin: f64,
    /// Most shape tests, as in [`QueryOptions::max_shape_tests`]. A retry
    /// without clearance gets what the first search left.
    pub max_shape_tests: usize,
}

impl Default for NavOptions {
    fn default() -> Self {
        NavOptions {
            clearance: 0.3,
            max_nodes: 512,
            max_visibility_tests: 50_000,
            margin: 30.0,
            max_shape_tests: 1_000_000,
        }
    }
}

impl NavOptions {
    pub fn clamped(self) -> NavOptions {
        let d = NavOptions::default();
        NavOptions {
            clearance: clamp_f(self.clearance, 0.0, 10.0, d.clearance),
            max_nodes: self.max_nodes.min(MAX_PATH_NODES),
            max_visibility_tests: self.max_visibility_tests.min(MAX_VISIBILITY_TESTS),
            margin: clamp_f(self.margin, 0.0, 1000.0, d.margin),
            max_shape_tests: self.max_shape_tests.min(MAX_SHAPE_TESTS),
        }
    }
}

/// A walking route.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct NavPath {
    /// Waypoints after the start, ending at the goal. Empty when no route
    /// was found. A game can put a beacon on the first one.
    pub points: Vec<Vec3>,
    /// Total length, metres.
    pub length: f64,
    /// Whether the goal is reached.
    pub reached: bool,
    /// Whether the route keeps the asked clearance. When no route keeps it,
    /// the search tries again with none.
    pub clearance_kept: bool,
    /// Whether the search hit a cap.
    pub truncated: bool,
}

struct Search {
    /// Corner points between the ends, in order.
    corners: Vec<Vec3>,
    found: bool,
    truncated: bool,
    /// Shape tests made.
    used: usize,
}

struct Graph<'a> {
    scene: &'a Scene,
    clearance: f64,
    probe: Probe,
}

impl Graph<'_> {
    fn clear(&mut self, a: Vec3, b: Vec3) -> bool {
        if self.scene.blocked(a, b, &[], false, &mut self.probe) {
            return false;
        }
        if self.clearance > 0.0 {
            // Two lines either side, `clearance` out.
            let d = Vec3::new(b.x - a.x, b.y - a.y, 0.0);
            if let Some(d) = d.normalized() {
                let side = Vec3::new(-d.y, d.x, 0.0) * self.clearance;
                for s in [side, -side] {
                    if self
                        .scene
                        .blocked(a + s, b + s, &[], false, &mut self.probe)
                    {
                        return false;
                    }
                }
            }
        }
        true
    }
}

/// How a search runs: corner offset, clearance, caps and margin.
#[derive(Clone, Copy)]
struct Budget {
    offset: f64,
    clearance: f64,
    max_nodes: usize,
    max_tests: usize,
    margin: f64,
    max_shape_tests: usize,
}

fn search(scene: &Scene, a: Vec3, b: Vec3, budget: Budget) -> Search {
    let Budget {
        offset,
        clearance,
        max_nodes,
        max_tests,
        margin,
        max_shape_tests,
    } = budget;
    let mut g = Graph {
        scene,
        clearance,
        probe: Probe::new(max_shape_tests),
    };
    let mut out = Search {
        corners: Vec::new(),
        found: false,
        truncated: false,
        used: 0,
    };
    let found = g.clear(a, b);
    out.used = g.probe.used();
    if found {
        out.found = true;
        return out;
    }
    if g.probe.spent() {
        out.truncated = true;
        return out;
    }
    // Corner nodes at the height midway between the ends.
    let z = (a.z + b.z) * 0.5;
    let m = Vec3::new(margin, margin, 0.0);
    let lo = a.min(b) - m;
    let hi = a.max(b) + m;
    let mut ids = Vec::new();
    if !scene.gather_within(lo, hi, &mut g.probe, &mut ids) {
        out.used = g.probe.used();
        out.truncated = true;
        return out;
    }
    let mut nodes = vec![a, b];
    let mut pts = Vec::new();
    // Corner checks get a quarter of the shape tests left.
    let corner_budget = g.probe.used() + g.probe.left() / 4;
    'objects: for &i in &ids {
        let shape = &scene.objects()[i as usize].shape;
        let Some((smin, smax)) = shape.bounds() else {
            continue;
        };
        // A shape wholly above or below the path height does not block it.
        if smax.z < z || smin.z > z {
            continue;
        }
        if smax.x < lo.x || smin.x > hi.x || smax.y < lo.y || smin.y > hi.y {
            continue;
        }
        pts.clear();
        corner_points(shape, offset, z, &mut pts);
        for &p in &pts {
            // Points inside other solids are skipped, but still cost.
            if g.probe.used() >= corner_budget || g.probe.spent() {
                out.truncated = true;
                break 'objects;
            }
            if scene.inside_solid(p, &mut g.probe) {
                continue;
            }
            if nodes.len() >= max_nodes + 2 {
                out.truncated = true;
                break 'objects;
            }
            nodes.push(p);
        }
    }

    // A* with a linear scan for the next node: n is small and the scan
    // keeps the order fixed.
    let n = nodes.len();
    let mut cost = vec![f64::INFINITY; n];
    let mut parent = vec![usize::MAX; n];
    let mut open = vec![false; n];
    let mut closed = vec![false; n];
    cost[0] = 0.0;
    open[0] = true;
    let mut tests = 0usize;
    'search: loop {
        let mut cur = usize::MAX;
        let mut best = f64::INFINITY;
        for i in 0..n {
            if open[i] {
                let f = cost[i] + nodes[i].distance(b);
                if f < best {
                    best = f;
                    cur = i;
                }
            }
        }
        if cur == usize::MAX {
            break;
        }
        if cur == 1 {
            out.found = true;
            break;
        }
        open[cur] = false;
        closed[cur] = true;
        for j in 0..n {
            if closed[j] || j == cur {
                continue;
            }
            let c = cost[cur] + nodes[cur].distance(nodes[j]);
            if c < cost[j] {
                if tests >= max_tests {
                    out.truncated = true;
                    break 'search;
                }
                tests += 1;
                if g.clear(nodes[cur], nodes[j]) {
                    cost[j] = c;
                    parent[j] = cur;
                    open[j] = true;
                } else if g.probe.spent() {
                    out.truncated = true;
                    break 'search;
                }
            }
        }
    }
    out.used = g.probe.used();
    if out.found {
        let mut i = parent[1];
        let mut steps = 0;
        while i != 0 && i != usize::MAX && steps <= n {
            out.corners.push(nodes[i]);
            i = parent[i];
            steps += 1;
        }
        out.corners.reverse();
        slope_heights(a, b, &mut out.corners);
    }
    out
}

/// The search runs at one height; give each corner the height it would
/// have on a straight slope from `a` to `b`, by distance along the route.
fn slope_heights(a: Vec3, b: Vec3, corners: &mut [Vec3]) {
    let flat = |p: Vec3, q: Vec3| {
        let dx = q.x - p.x;
        let dy = q.y - p.y;
        (dx * dx + dy * dy).sqrt()
    };
    let mut total = 0.0;
    let mut prev = a;
    for &c in corners.iter() {
        total += flat(prev, c);
        prev = c;
    }
    total += flat(prev, b);
    if total <= 0.0 {
        return;
    }
    let mut run = 0.0;
    let mut prev = a;
    for c in corners.iter_mut() {
        run += flat(prev, *c);
        prev = *c;
        c.z = a.z + (b.z - a.z) * (run / total);
    }
}

/// ISO 9613-2:1996 eq. (14) barrier attenuation, without the meteorological
/// factor: Dz = 10 log10(3 + (C2 / lambda) C3 z) dB, C2 = 20, z the extra
/// path length, C3 = 1 for one edge, or from eq. (15) with `e` the distance
/// between the first and last edges. Capped at 20 dB for one edge and 25 dB
/// for more.
pub fn barrier_attenuation_db(
    path_difference: f64,
    edges: usize,
    edge_span: f64,
    speed_of_sound: f64,
) -> Bands {
    let z = path_difference.max(0.0);
    let mut out = [0.0; BAND_COUNT];
    for (o, f) in out.iter_mut().zip(BAND_EXACT_HZ) {
        let lambda = speed_of_sound / f;
        let c3 = if edges <= 1 || edge_span <= 0.0 {
            1.0
        } else {
            let k = 5.0 * lambda / edge_span;
            let k2 = k * k;
            (1.0 + k2) / (1.0 / 3.0 + k2)
        };
        let cap = if edges <= 1 { 20.0 } else { 25.0 };
        let dz = 10.0 * libm::log10(3.0 + (20.0 / lambda) * c3 * z);
        *o = dz.max(0.0).min(cap);
    }
    Bands(out)
}

/// The way a sound from `source` bends round obstacles to reach
/// `listener`, when the straight line is blocked. Gives no path when the
/// line is clear or diffraction is off.
pub fn diffracted_path(
    scene: &Scene,
    source: Vec3,
    listener: Vec3,
    opts: &QueryOptions,
) -> DiffractionSearch {
    let opts = opts.clamped();
    if !opts.diffraction || !(source.is_usable() && listener.is_usable()) {
        return DiffractionSearch::default();
    }
    let s = search(
        scene,
        source,
        listener,
        Budget {
            offset: SOUND_OFFSET,
            clearance: 0.0,
            max_nodes: opts.max_path_nodes,
            max_tests: opts.max_visibility_tests,
            margin: opts.path_margin,
            max_shape_tests: opts.max_shape_tests,
        },
    );
    let mut result = DiffractionSearch {
        path: None,
        truncated: s.truncated,
        shape_tests: s.used,
    };
    if !s.found || s.corners.is_empty() {
        return result;
    }
    let mut length = 0.0;
    let mut prev = source;
    for &c in &s.corners {
        length += prev.distance(c);
        prev = c;
    }
    length += prev.distance(listener);
    let mut span = 0.0;
    for w in s.corners.windows(2) {
        span += w[0].distance(w[1]);
    }
    let direct = source.distance(listener);
    let dz = barrier_attenuation_db(
        length - direct,
        s.corners.len(),
        span,
        scene.speed_of_sound(),
    );
    let diffraction = dz.loss_db_to_gain();
    let spread = spreading(length, opts.reference_distance);
    let air = scene.air_gain(length);
    let arrival = *s.corners.last().unwrap_or(&source);
    result.path = Some(DiffractedPath {
        corners: s.corners,
        length,
        delay_s: length / scene.speed_of_sound(),
        spreading: spread,
        air,
        diffraction,
        gain: air.times(diffraction).scale(spread),
        arrival,
    });
    result
}

/// A walking route from `from` to `to` through open space, keeping
/// `clearance` from obstacles where it can.
///
/// The route runs in the horizontal plane at the height midway between the
/// ends. Clearance is checked along the route's centre and two lines either
/// side of it, so a thin spike could still poke into it.
pub fn navigate(scene: &Scene, from: Vec3, to: Vec3, opts: &NavOptions) -> NavPath {
    let opts = opts.clamped();
    if !(from.is_usable() && to.is_usable()) {
        return NavPath::default();
    }
    let mut kept = true;
    let budget = Budget {
        offset: opts.clearance + SOUND_OFFSET,
        clearance: opts.clearance,
        max_nodes: opts.max_nodes,
        max_tests: opts.max_visibility_tests,
        margin: opts.margin,
        max_shape_tests: opts.max_shape_tests,
    };
    let mut s = search(scene, from, to, budget);
    let mut truncated = s.truncated;
    if !s.found && opts.clearance > 0.0 {
        kept = false;
        let loose = Budget {
            offset: SOUND_OFFSET,
            clearance: 0.0,
            max_shape_tests: opts.max_shape_tests.saturating_sub(s.used),
            ..budget
        };
        s = search(scene, from, to, loose);
        truncated |= s.truncated;
    }
    if !s.found {
        return NavPath {
            truncated,
            ..NavPath::default()
        };
    }
    let mut points = s.corners;
    points.push(to);
    let mut length = 0.0;
    let mut prev = from;
    for &p in &points {
        length += prev.distance(p);
        prev = p;
    }
    NavPath {
        points,
        length,
        reached: true,
        clearance_kept: kept,
        truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Shape;
    use crate::material::Builtin;
    use crate::scene::SceneBuilder;

    /// A long wall along x = 5 with a 1 m doorway from y = 2 to y = 3.
    fn doorway() -> Scene {
        let mut b = SceneBuilder::new();
        let brick = Builtin::Brick.id();
        b.add(Shape::wall_between(5.0, -100.0, 5.0, 2.0, 0.0, 3.0), brick)
            .unwrap();
        b.add(Shape::wall_between(5.0, 3.0, 5.0, 100.0, 0.0, 3.0), brick)
            .unwrap();
        b.build()
    }

    #[test]
    fn iso_barrier_values() {
        // z = 0: 10 log10(3) = 4.77 dB in every band.
        let a = barrier_attenuation_db(0.0, 1, 0.0, 343.0);
        for v in a.0 {
            assert!((v - 4.771).abs() < 1e-3);
        }
        // 1 m extra at 1 kHz, c = 343: 10 log10(3 + 20 / 0.343) = 17.9 dB.
        let a = barrier_attenuation_db(1.0, 1, 0.0, 343.0);
        assert!((a.0[3] - 17.88).abs() < 0.01, "{}", a.0[3]);
        // Capped.
        let a = barrier_attenuation_db(100.0, 1, 0.0, 343.0);
        assert_eq!(a.0[6], 20.0);
        let a = barrier_attenuation_db(100.0, 2, 5.0, 343.0);
        assert_eq!(a.0[6], 25.0);
        // Low bands bend more easily.
        let a = barrier_attenuation_db(0.5, 1, 0.0, 343.0);
        assert!(a.0[0] < a.0[6]);
    }

    #[test]
    fn sound_comes_through_the_doorway() {
        let s = doorway();
        let src = Vec3::new(8.0, 0.0, 1.5);
        let lis = Vec3::new(2.0, 0.0, 1.5);
        let r = diffracted_path(&s, src, lis, &QueryOptions::default());
        assert!(!r.truncated);
        let p = r.path.expect("path");
        // It bends at the doorway's lower jamb (y = 2) on both sides.
        assert!(!p.corners.is_empty());
        for c in &p.corners {
            assert!((c.x - 5.0).abs() < 0.1 && (c.y - 2.0).abs() < 0.1, "{c:?}");
        }
        assert!((p.arrival.y - 2.0).abs() < 0.1);
        assert!(p.length > src.distance(lis));
        assert!(p.gain.0[6] < p.gain.0[0]);
    }

    #[test]
    fn clear_line_gives_no_path() {
        let s = doorway();
        let r = diffracted_path(
            &s,
            Vec3::new(8.0, 2.5, 1.5),
            Vec3::new(2.0, 2.5, 1.5),
            &QueryOptions::default(),
        );
        assert!(r.path.is_none() && !r.truncated);
    }

    #[test]
    fn sealed_room_gives_no_path() {
        let mut b = SceneBuilder::new();
        let c = Builtin::Concrete.id();
        b.add(Shape::wall_between(0.0, 0.0, 4.0, 0.0, 0.0, 3.0), c)
            .unwrap();
        b.add(Shape::wall_between(4.0, 0.0, 4.0, 4.0, 0.0, 3.0), c)
            .unwrap();
        b.add(Shape::wall_between(4.0, 4.0, 0.0, 4.0, 0.0, 3.0), c)
            .unwrap();
        b.add(Shape::wall_between(0.0, 4.0, 0.0, 0.0, 0.0, 3.0), c)
            .unwrap();
        let s = b.build();
        let r = diffracted_path(
            &s,
            Vec3::new(2.0, 2.0, 1.5),
            Vec3::new(10.0, 2.0, 1.5),
            &QueryOptions::default(),
        );
        assert!(r.path.is_none());
        let n = navigate(
            &s,
            Vec3::new(2.0, 2.0, 1.5),
            Vec3::new(10.0, 2.0, 1.5),
            &NavOptions::default(),
        );
        assert!(!n.reached && n.points.is_empty());
    }

    #[test]
    fn round_a_pillar() {
        let mut b = SceneBuilder::new();
        b.add(Shape::pole(5.0, 0.0, 0.0, 1.0, 3.0), Builtin::Concrete.id())
            .unwrap();
        let s = b.build();
        let n = navigate(
            &s,
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(10.0, 0.0, 1.0),
            &NavOptions::default(),
        );
        assert!(n.reached && n.clearance_kept);
        assert!(n.points.len() >= 2);
        assert_eq!(*n.points.last().unwrap(), Vec3::new(10.0, 0.0, 1.0));
        // Longer than straight, shorter than a big detour.
        assert!(n.length > 10.0 && n.length < 11.5, "{}", n.length);
        for p in &n.points[..n.points.len() - 1] {
            let r = ((p.x - 5.0) * (p.x - 5.0) + p.y * p.y).sqrt();
            assert!(r >= 1.3, "{p:?}");
        }
    }

    #[test]
    fn navigate_through_doorway_and_tight_gaps() {
        let s = doorway();
        let n = navigate(
            &s,
            Vec3::new(2.0, -5.0, 1.0),
            Vec3::new(8.0, -5.0, 1.0),
            &NavOptions::default(),
        );
        assert!(n.reached && n.clearance_kept);
        // Through the gap between y = 2 and y = 3.
        assert!(n
            .points
            .iter()
            .any(|p| (p.x - 5.0).abs() < 0.5 && p.y > 2.0 && p.y < 3.0));
        assert!(n.length < 20.0, "{}", n.length);
        // A body too wide for the doorway goes the long way round.
        let wide = NavOptions {
            clearance: 0.8,
            ..NavOptions::default()
        };
        let n = navigate(
            &s,
            Vec3::new(2.0, -5.0, 1.0),
            Vec3::new(8.0, -5.0, 1.0),
            &wide,
        );
        assert!(n.reached && n.clearance_kept);
        assert!(n.length > 100.0, "{}", n.length);
        // With no long way round it still gets a route, flagged.
        let mut b = SceneBuilder::new();
        let c = Builtin::Concrete.id();
        b.add(Shape::wall_between(0.0, 0.0, 4.0, 0.0, 0.0, 3.0), c)
            .unwrap();
        b.add(Shape::wall_between(4.0, 0.0, 4.0, 4.0, 0.0, 3.0), c)
            .unwrap();
        b.add(Shape::wall_between(4.0, 4.0, 2.5, 4.0, 0.0, 3.0), c)
            .unwrap();
        b.add(Shape::wall_between(1.5, 4.0, 0.0, 4.0, 0.0, 3.0), c)
            .unwrap();
        b.add(Shape::wall_between(0.0, 4.0, 0.0, 0.0, 0.0, 3.0), c)
            .unwrap();
        let room = b.build();
        let from = Vec3::new(2.0, 1.0, 1.0);
        let to = Vec3::new(2.0, 8.0, 1.0);
        let n = navigate(&room, from, to, &NavOptions::default());
        assert!(n.reached && n.clearance_kept);
        let n = navigate(&room, from, to, &wide);
        assert!(n.reached && !n.clearance_kept);
    }

    #[test]
    fn caps_stop_the_search() {
        let mut b = SceneBuilder::new();
        for i in 0..400 {
            let x = (i % 20) as f64 * 3.0;
            let y = (i / 20) as f64 * 3.0;
            b.add(Shape::pole(x, y, 0.0, 0.5, 3.0), Builtin::Wood.id())
                .unwrap();
        }
        let s = b.build();
        let o = QueryOptions {
            max_path_nodes: 10,
            ..QueryOptions::default()
        };
        let r = diffracted_path(&s, Vec3::new(-2.0, 0.0, 1.0), Vec3::new(60.0, 0.0, 1.0), &o);
        assert!(r.truncated);
        let o = QueryOptions {
            max_visibility_tests: 5,
            ..QueryOptions::default()
        };
        let r = diffracted_path(&s, Vec3::new(-2.0, 0.0, 1.0), Vec3::new(60.0, 0.0, 1.0), &o);
        assert!(r.truncated);
    }
}
