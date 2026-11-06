//! A stack of uniform grids over the ground plane, so a query only tests
//! the shapes near its line or region.
//!
//! Level 0 has the finest cells; each level above has cells twice as wide.
//! A shape goes in the finest level where it spans at most
//! `MAX_CELLS_PER_SHAPE` cells, so a long wall sits in a few coarse cells
//! instead of in a list every query must check.

use crate::vec::Vec3;

/// Most grid cells per side.
const MAX_CELLS_PER_SIDE: usize = 256;
/// Most cells one shape is listed in.
const MAX_CELLS_PER_SHAPE: usize = 16;
/// Smallest cell edge, metres.
const MIN_CELL: f64 = 0.5;

#[derive(Clone, Debug, Default)]
struct Level {
    cell: f64,
    nx: usize,
    ny: usize,
    /// Cell `i` holds `items[starts[i]..starts[i + 1]]`.
    starts: Vec<u32>,
    items: Vec<u32>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Grid {
    min_x: f64,
    min_y: f64,
    levels: Vec<Level>,
    /// Shapes checked by every query: the endless planes.
    always: Vec<u32>,
}

impl Level {
    fn new(cell: f64, w: f64, h: f64) -> Level {
        Level {
            cell,
            nx: ((w / cell) as usize + 1).min(MAX_CELLS_PER_SIDE),
            ny: ((h / cell) as usize + 1).min(MAX_CELLS_PER_SIDE),
            starts: Vec::new(),
            items: Vec::new(),
        }
    }
}

impl Grid {
    /// Builds the grid. `bounds[i]` is shape `i`'s box, `None` for an endless
    /// one.
    pub fn build(bounds: &[Option<(Vec3, Vec3)>]) -> Grid {
        let mut lo = Vec3::new(f64::INFINITY, f64::INFINITY, 0.0);
        let mut hi = Vec3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, 0.0);
        for (a, b) in bounds.iter().flatten() {
            lo = lo.min(*a);
            hi = hi.max(*b);
        }
        let mut g = Grid::default();
        for (i, b) in bounds.iter().enumerate() {
            if b.is_none() {
                g.always.push(i as u32);
            }
        }
        if !(lo.x <= hi.x && lo.y <= hi.y) {
            return g;
        }
        let (w, h) = (hi.x - lo.x, hi.y - lo.y);
        let span = w.max(h).max(MIN_CELL);
        let mut cell = (span / MAX_CELLS_PER_SIDE as f64).max(MIN_CELL);
        // Aim for a few shapes per cell when the scene is dense.
        let n = bounds.len().max(1) as f64;
        let area = (w + cell) * (h + cell);
        cell = cell.max((area / (n * 4.0)).sqrt().min(span));
        g.min_x = lo.x;
        g.min_y = lo.y;
        // Levels up to one where every shape fits.
        loop {
            let l = Level::new(cell, w, h);
            let top = l.nx * l.ny <= MAX_CELLS_PER_SHAPE;
            g.levels.push(l);
            if top {
                break;
            }
            cell *= 2.0;
        }

        // Pick each shape's level, then count and fill (compressed rows).
        let mut placed = Vec::with_capacity(bounds.len());
        for b in bounds {
            placed.push(b.map(|(a, b)| {
                let last = g.levels.len() - 1;
                for (k, l) in g.levels.iter().enumerate() {
                    let r = g.cell_range(l, a.x, a.y, b.x, b.y);
                    let n = (r.1 - r.0 + 1) * (r.3 - r.2 + 1);
                    if n <= MAX_CELLS_PER_SHAPE || k == last {
                        return (k, r);
                    }
                }
                (last, (0, 0, 0, 0))
            }));
        }
        for k in 0..g.levels.len() {
            let nx = g.levels[k].nx;
            let cells = nx * g.levels[k].ny;
            let mut starts = vec![0u32; cells + 1];
            for &(lk, (x0, x1, y0, y1)) in placed.iter().flatten() {
                if lk == k {
                    for y in y0..=y1 {
                        for x in x0..=x1 {
                            starts[y * nx + x + 1] += 1;
                        }
                    }
                }
            }
            for i in 0..cells {
                starts[i + 1] += starts[i];
            }
            let mut fill = starts.clone();
            let mut items = vec![0u32; starts[cells] as usize];
            for (i, p) in placed.iter().enumerate() {
                if let Some((lk, (x0, x1, y0, y1))) = *p {
                    if lk == k {
                        for y in y0..=y1 {
                            for x in x0..=x1 {
                                let c = y * nx + x;
                                items[fill[c] as usize] = i as u32;
                                fill[c] += 1;
                            }
                        }
                    }
                }
            }
            g.levels[k].starts = starts;
            g.levels[k].items = items;
        }
        g
    }

    fn col(&self, l: &Level, x: f64) -> usize {
        let c = (x - self.min_x) / l.cell;
        if c > 0.0 {
            (c as usize).min(l.nx - 1)
        } else {
            0
        }
    }

    fn row(&self, l: &Level, y: f64) -> usize {
        let c = (y - self.min_y) / l.cell;
        if c > 0.0 {
            (c as usize).min(l.ny - 1)
        } else {
            0
        }
    }

    fn cell_range(
        &self,
        l: &Level,
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
    ) -> (usize, usize, usize, usize) {
        (
            self.col(l, x0.min(x1)),
            self.col(l, x0.max(x1)),
            self.row(l, y0.min(y1)),
            self.row(l, y0.max(y1)),
        )
    }

    fn push_cell(l: &Level, x: usize, y: usize, out: &mut Vec<u32>) {
        let c = y * l.nx + x;
        let (a, b) = (l.starts[c] as usize, l.starts[c + 1] as usize);
        out.extend_from_slice(&l.items[a..b]);
    }

    fn finish(&self, out: &mut Vec<u32>) {
        out.extend_from_slice(&self.always);
        out.sort_unstable();
        out.dedup();
    }

    /// Shapes that might touch the segment from `p` to `q`, in id order.
    /// Clears `out` first.
    pub fn along(&self, p: Vec3, q: Vec3, out: &mut Vec<u32>) {
        out.clear();
        let (a, b) = if p.x <= q.x { (p, q) } else { (q, p) };
        for l in &self.levels {
            if l.items.is_empty() {
                continue;
            }
            // Walk the columns the segment spans; in each, add the rows the
            // segment covers within that column.
            let c0 = self.col(l, a.x);
            let c1 = self.col(l, b.x);
            let dx = b.x - a.x;
            for c in c0..=c1 {
                let (ya, yb) = if dx > 0.0 {
                    let xl = (self.min_x + c as f64 * l.cell).max(a.x);
                    let xr = (self.min_x + (c + 1) as f64 * l.cell).min(b.x);
                    let ta = ((xl - a.x) / dx).max(0.0).min(1.0);
                    let tb = ((xr - a.x) / dx).max(0.0).min(1.0);
                    // Outermost columns hold the clamped ends too.
                    let ta = if c == c0 { 0.0 } else { ta };
                    let tb = if c == c1 { 1.0 } else { tb };
                    (a.y + (b.y - a.y) * ta, a.y + (b.y - a.y) * tb)
                } else {
                    (a.y, b.y)
                };
                for r in self.row(l, ya.min(yb))..=self.row(l, ya.max(yb)) {
                    Grid::push_cell(l, c, r, out);
                }
            }
        }
        self.finish(out);
    }

    /// Shapes that might touch the horizontal box from (x0, y0) to (x1, y1),
    /// in id order. Clears `out` first.
    pub fn within(&self, x0: f64, y0: f64, x1: f64, y1: f64, out: &mut Vec<u32>) {
        out.clear();
        for l in &self.levels {
            if l.items.is_empty() {
                continue;
            }
            let (c0, c1, r0, r1) = self.cell_range(l, x0, y0, x1, y1);
            for r in r0..=r1 {
                for c in c0..=c1 {
                    Grid::push_cell(l, c, r, out);
                }
            }
        }
        self.finish(out);
    }
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
        let g = Grid::build(&b);
        let mut out = Vec::new();
        g.along(
            Vec3::new(0.5, 0.5, 0.0),
            Vec3::new(196.5, 196.5, 0.0),
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
        let g = Grid::build(&b);
        let mut out = Vec::new();
        g.within(-1.0, -1.0, 2.0, 2.0, &mut out);
        assert!(out.contains(&0));
        g.along(
            Vec3::new(-100.0, 10.5, 0.0),
            Vec3::new(100.0, 10.5, 0.0),
            &mut out,
        );
        assert!(out.contains(&1));
        g.along(
            Vec3::new(10.5, -1e6, 0.0),
            Vec3::new(10.5, 1e6, 0.0),
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
        let g = Grid::build(&b);
        let mut out = Vec::new();
        g.along(
            Vec3::new(-2.0, 0.5, 0.0),
            Vec3::new(60.0, 0.5, 0.0),
            &mut out,
        );
        assert_eq!(out, vec![1000]);
        g.within(-1.0, -1.0, 2.0, 2.0, &mut out);
        assert_eq!(out, vec![1000]);
        // Near the walls they are all found.
        g.along(
            Vec3::new(0.0, 4990.0, 0.0),
            Vec3::new(0.0, 6010.0, 0.0),
            &mut out,
        );
        assert_eq!(out.len(), 1000);
    }

    #[test]
    fn empty_or_plane_only_scenes() {
        let g = Grid::build(&[]);
        let mut out = vec![7];
        g.along(Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0), &mut out);
        assert!(out.is_empty());
        let g = Grid::build(&[None, None]);
        g.within(0.0, 0.0, 1.0, 1.0, &mut out);
        assert_eq!(out, vec![0, 1]);
    }
}
