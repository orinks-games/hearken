//! A bounding box tree over the shapes, so a query only tests the shapes
//! near its line or region.
//!
//! Each node holds a box round its shapes. A node splits its shapes in two
//! halves at the median along its widest axis, so the tree stays balanced
//! however the shapes are spread: a far-off shape or a crowded spot does
//! not change how finely the rest of the scene is divided.

use crate::vec::Vec3;

/// Most shapes in a leaf.
const LEAF: usize = 4;
/// Deepest node. Median splits halve the count at each level, so a tree
/// of up to 2^32 shapes stays far above this; it bounds the query stack.
const MAX_DEPTH: usize = 48;
/// Query stack size: one pending node per level, and a spare.
const STACK: usize = MAX_DEPTH + 2;

#[derive(Clone, Copy, Debug)]
struct Node {
    lo: Vec3,
    hi: Vec3,
    /// A leaf holds `items[first..first + count]`; an inner node (count 0)
    /// has its children at `first` and `first + 1`.
    first: u32,
    count: u32,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Tree {
    nodes: Vec<Node>,
    items: Vec<u32>,
    /// Shapes checked by every query: the endless planes.
    always: Vec<u32>,
}

/// A box grown a hair on every side, so rounding in the box tests never
/// misses a shape the exact tests would find.
fn padded(lo: Vec3, hi: Vec3) -> (Vec3, Vec3) {
    let pad = |v: f64| 1e-6 + v.abs() * 1e-12;
    (
        Vec3::new(lo.x - pad(lo.x), lo.y - pad(lo.y), lo.z - pad(lo.z)),
        Vec3::new(hi.x + pad(hi.x), hi.y + pad(hi.y), hi.z + pad(hi.z)),
    )
}

fn axis(v: Vec3, a: usize) -> f64 {
    match a {
        0 => v.x,
        1 => v.y,
        _ => v.z,
    }
}

impl Tree {
    /// Builds the tree. `bounds[i]` is shape `i`'s box, `None` for an
    /// endless one.
    pub fn build(bounds: &[Option<(Vec3, Vec3)>]) -> Tree {
        let mut g = Tree::default();
        // (id, padded box, centre)
        let mut work: Vec<(u32, Vec3, Vec3, Vec3)> = Vec::new();
        for (i, b) in bounds.iter().enumerate() {
            match b {
                None => g.always.push(i as u32),
                Some((a, b)) => {
                    let (lo, hi) = padded(*a, *b);
                    work.push((i as u32, lo, hi, (lo + hi) * 0.5));
                }
            }
        }
        if work.is_empty() {
            return g;
        }
        // Nodes are made breadth first from a queue of (node, start, end,
        // depth) ranges into `work`.
        g.nodes.push(Node {
            lo: Vec3::ZERO,
            hi: Vec3::ZERO,
            first: 0,
            count: 0,
        });
        let mut queue = vec![(0usize, 0usize, work.len(), 0usize)];
        let mut q = 0;
        while q < queue.len() {
            let (node, start, end, depth) = queue[q];
            q += 1;
            let part = &mut work[start..end];
            let mut lo = part[0].1;
            let mut hi = part[0].2;
            let mut clo = part[0].3;
            let mut chi = part[0].3;
            for w in part.iter() {
                lo = lo.min(w.1);
                hi = hi.max(w.2);
                clo = clo.min(w.3);
                chi = chi.max(w.3);
            }
            g.nodes[node].lo = lo;
            g.nodes[node].hi = hi;
            let n = end - start;
            let spread = chi - clo;
            let flat = spread.x <= 0.0 && spread.y <= 0.0 && spread.z <= 0.0;
            if n <= LEAF || depth >= MAX_DEPTH || flat {
                // Shapes at one spot gain nothing from a split.
                g.nodes[node].first = g.items.len() as u32;
                g.nodes[node].count = n as u32;
                g.items.extend(part.iter().map(|w| w.0));
                continue;
            }
            let a = if spread.x >= spread.y && spread.x >= spread.z {
                0
            } else if spread.y >= spread.z {
                1
            } else {
                2
            };
            // A total order (centre, then id), so the split is the same
            // everywhere.
            part.sort_unstable_by(|p, q| axis(p.3, a).total_cmp(&axis(q.3, a)).then(p.0.cmp(&q.0)));
            let mid = start + n / 2;
            let left = g.nodes.len();
            let blank = Node {
                lo: Vec3::ZERO,
                hi: Vec3::ZERO,
                first: 0,
                count: 0,
            };
            g.nodes.push(blank);
            g.nodes.push(blank);
            g.nodes[node].first = left as u32;
            queue.push((left, start, mid, depth + 1));
            queue.push((left + 1, mid, end, depth + 1));
        }
        g
    }

    fn finish(&self, out: &mut Vec<u32>) {
        out.extend_from_slice(&self.always);
        out.sort_unstable();
        out.dedup();
    }

    /// Walks the tree, entering nodes whose box `hit` accepts, and adds the
    /// shapes of every leaf reached. Gives up, returning false with `out`
    /// cleared, once more than `limit` shapes are found.
    fn walk(&self, hit: impl Fn(Vec3, Vec3) -> bool, limit: usize, out: &mut Vec<u32>) -> bool {
        out.clear();
        if !self.nodes.is_empty() {
            let mut stack = [0u32; STACK];
            let mut sp = 1;
            while sp > 0 {
                sp -= 1;
                let n = self.nodes[stack[sp] as usize];
                if !hit(n.lo, n.hi) {
                    continue;
                }
                if n.count > 0 {
                    if n.count as usize > limit.saturating_sub(out.len()) {
                        out.clear();
                        return false;
                    }
                    let a = n.first as usize;
                    out.extend_from_slice(&self.items[a..a + n.count as usize]);
                } else {
                    // Depth is at most MAX_DEPTH, so the stack never fills.
                    stack[sp] = n.first + 1;
                    stack[sp + 1] = n.first;
                    sp += 2;
                }
            }
        }
        self.finish(out);
        true
    }

    /// Shapes that might touch the segment from `p` to `q`, in id order.
    /// Clears `out` first.
    #[cfg(test)]
    pub fn along(&self, p: Vec3, q: Vec3, out: &mut Vec<u32>) {
        self.along_capped(p, q, usize::MAX, out);
    }

    /// As [`Tree::along`], but stops and returns false, with `out` empty,
    /// once more than `limit` bounded shapes are found. The endless planes
    /// do not count.
    pub fn along_capped(&self, p: Vec3, q: Vec3, limit: usize, out: &mut Vec<u32>) -> bool {
        let d = q - p;
        self.walk(|lo, hi| segment_meets_box(p, d, lo, hi), limit, out)
    }

    /// Shapes that might touch the horizontal box from (x0, y0) to (x1, y1)
    /// at any height, in id order. Clears `out` first. As with
    /// [`Tree::along_capped`], stops and returns false past `limit`.
    pub fn within(
        &self,
        (x0, y0): (f64, f64),
        (x1, y1): (f64, f64),
        limit: usize,
        out: &mut Vec<u32>,
    ) -> bool {
        let (ax, bx) = (x0.min(x1), x0.max(x1));
        let (ay, by) = (y0.min(y1), y0.max(y1));
        self.walk(
            |lo, hi| !(hi.x < ax || lo.x > bx || hi.y < ay || lo.y > by),
            limit,
            out,
        )
    }
}

/// Whether the segment `p + t d`, t in 0..1, meets the box. NaN input
/// counts as a hit, so nothing is missed.
fn segment_meets_box(p: Vec3, d: Vec3, lo: Vec3, hi: Vec3) -> bool {
    let mut t0: f64 = 0.0;
    let mut t1: f64 = 1.0;
    for a in 0..3 {
        let (pa, da, la, ha) = (axis(p, a), axis(d, a), axis(lo, a), axis(hi, a));
        if da == 0.0 {
            if pa < la || pa > ha {
                return false;
            }
            continue;
        }
        let u = (la - pa) / da;
        let v = (ha - pa) / da;
        let (u, v) = if u <= v { (u, v) } else { (v, u) };
        // Written so NaN keeps the range open.
        if u > t0 {
            t0 = u;
        }
        if v < t1 {
            t1 = v;
        }
        if t0 > t1 {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(x: f64, y: f64) -> Option<(Vec3, Vec3)> {
        Some((Vec3::new(x, y, 0.0), Vec3::new(x + 1.0, y + 1.0, 1.0)))
    }

    #[test]
    fn segment_finds_shapes_on_its_line_only() {
        let mut b = Vec::new();
        for i in 0..50 {
            for j in 0..50 {
                b.push(bounds(i as f64 * 4.0, j as f64 * 4.0));
            }
        }
        b.push(None);
        let g = Tree::build(&b);
        let mut out = Vec::new();
        g.along(
            Vec3::new(0.5, 0.5, 0.5),
            Vec3::new(196.5, 196.5, 0.5),
            &mut out,
        );
        // Every shape on the diagonal must be found.
        for i in 0..50 {
            assert!(out.contains(&((i * 50 + i) as u32)), "{i}");
        }
        assert!(out.contains(&2500), "endless shapes always come back");
        assert!(out.len() < 400, "{}", out.len());
        // Sorted and unique.
        assert!(out.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn region_query_and_out_of_grid_points() {
        let b = vec![bounds(0.0, 0.0), bounds(10.0, 10.0)];
        let g = Tree::build(&b);
        let mut out = Vec::new();
        g.within((-1.0, -1.0), (2.0, 2.0), usize::MAX, &mut out);
        assert!(out.contains(&0));
        g.along(
            Vec3::new(-100.0, 10.5, 0.5),
            Vec3::new(100.0, 10.5, 0.5),
            &mut out,
        );
        assert!(out.contains(&1));
        g.along(
            Vec3::new(10.5, -1e6, 0.5),
            Vec3::new(10.5, 1e6, 0.5),
            &mut out,
        );
        assert!(out.contains(&1));
    }

    #[test]
    fn long_shapes_stay_out_of_far_queries() {
        // Long walls far north, small boxes near the origin.
        let mut b = Vec::new();
        for i in 0..1000 {
            let y = 5000.0 + i as f64;
            b.push(Some((
                Vec3::new(-1500.0, y, 0.0),
                Vec3::new(1500.0, y, 3.0),
            )));
        }
        b.push(bounds(0.0, 0.0));
        let g = Tree::build(&b);
        let mut out = Vec::new();
        g.along(
            Vec3::new(-2.0, 0.5, 0.5),
            Vec3::new(60.0, 0.5, 0.5),
            &mut out,
        );
        // A leaf may pair the box with a wall or two, no more.
        assert!(out.contains(&1000) && out.len() <= LEAF, "{out:?}");
        g.within((-1.0, -1.0), (2.0, 2.0), usize::MAX, &mut out);
        assert!(out.contains(&1000) && out.len() <= LEAF, "{out:?}");
        // Near the walls they are all found.
        g.along(
            Vec3::new(0.0, 4990.0, 1.0),
            Vec3::new(0.0, 6010.0, 1.0),
            &mut out,
        );
        assert!((0..1000).all(|i| out.contains(&i)));
    }

    #[test]
    fn far_shape_leaves_the_rest_finely_split() {
        let mut b = Vec::new();
        for i in 0..100 {
            for j in 0..100 {
                b.push(bounds(i as f64 * 2.0, j as f64 * 2.0));
            }
        }
        b.push(bounds(99_999.0, 99_999.0));
        let g = Tree::build(&b);
        let mut out = Vec::new();
        g.along(
            Vec3::new(0.5, 0.5, 0.5),
            Vec3::new(0.5, 10.5, 0.5),
            &mut out,
        );
        assert!(out.len() < 20, "{}", out.len());
        g.along(
            Vec3::new(99_999.5, 0.0, 0.5),
            Vec3::new(99_999.5, 1e6, 0.5),
            &mut out,
        );
        assert!(out.contains(&10_000));
    }

    #[test]
    fn heights_count() {
        let b = vec![bounds(0.0, 0.0)];
        let g = Tree::build(&b);
        let mut out = Vec::new();
        g.along(
            Vec3::new(-1.0, 0.5, 5.0),
            Vec3::new(2.0, 0.5, 5.0),
            &mut out,
        );
        assert!(out.is_empty());
        // Exactly on the top face still counts.
        g.along(
            Vec3::new(-1.0, 0.5, 1.0),
            Vec3::new(2.0, 0.5, 1.0),
            &mut out,
        );
        assert_eq!(out, vec![0]);
    }

    #[test]
    fn empty_or_plane_only_scenes() {
        let g = Tree::build(&[]);
        let mut out = vec![7];
        g.along(Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0), &mut out);
        assert!(out.is_empty());
        let g = Tree::build(&[None, None]);
        g.within((0.0, 0.0), (1.0, 1.0), usize::MAX, &mut out);
        assert_eq!(out, vec![0, 1]);
    }
}
