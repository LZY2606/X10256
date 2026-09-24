//! Geometric primitives and the ordered-polarity raster model.
//!
//! The copper image starts empty. Each placed draw is then applied in source
//! operation order:
//! * `dark` paints copper,
//! * `clear` erases copper.
//! Replaying the same draws with a different order can therefore change the
//! final copper, which is exactly the polarity-order semantics we must keep as
//! structured, re-playable state.
//!
//! Contact detection is exact by default. A versioned tolerance (>= 0) creates
//! a *separate* set of tolerant contacts; proximity is never reported as a
//! conductive short unless the operator accepts that candidate.

use crate::identity::Id;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

impl Vec2 {
    pub fn new(x: f64, y: f64) -> Self {
        Vec2 { x, y }
    }
    pub fn sub(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x - o.x, self.y - o.y)
    }
    pub fn add(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x + o.x, self.y + o.y)
    }
    pub fn scale(self, s: f64) -> Vec2 {
        Vec2::new(self.x * s, self.y * s)
    }
    pub fn length(self) -> f64 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
    pub fn distance(self, o: Vec2) -> f64 {
        self.sub(o).length()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    Circle {
        cx: f64,
        cy: f64,
        r: f64,
    },
    /// Axis-aligned rectangle stored as (min, max).
    Rect {
        minx: f64,
        miny: f64,
        maxx: f64,
        maxy: f64,
    },
    /// Rounded segment ("obround"): capsule from p1 to p2 with radius r.
    Capsule {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        r: f64,
    },
    /// Filled simple polygon (convex or concave, no self-intersection).
    Polygon {
        pts: Vec<(f64, f64)>,
    },
}

impl Shape {
    pub fn bounds(&self) -> (f64, f64, f64, f64) {
        match self {
            Shape::Circle { cx, cy, r } => (cx - r, cy - r, cx + r, cy + r),
            Shape::Rect {
                minx,
                miny,
                maxx,
                maxy,
            } => (*minx, *miny, *maxx, *maxy),
            Shape::Capsule { x1, y1, x2, y2, r } => {
                let (a, b) = (x1.min(*x2) - r, x1.max(*x2) + r);
                let (c, d) = (y1.min(*y2) - r, y1.max(*y2) + r);
                (a, c, b, d)
            }
            Shape::Polygon { pts } => {
                let mut minx = f64::MAX;
                let mut miny = f64::MAX;
                let mut maxx = f64::MIN;
                let mut maxy = f64::MIN;
                for (x, y) in pts {
                    minx = minx.min(*x);
                    miny = miny.min(*y);
                    maxx = maxx.max(*x);
                    maxy = maxy.max(*y);
                }
                (minx, miny, maxx, maxy)
            }
        }
    }

    /// Signed distance: negative inside, zero on boundary, positive outside.
    pub fn signed_distance(&self, p: Vec2) -> f64 {
        match self {
            Shape::Circle { cx, cy, r } => p.distance(Vec2::new(*cx, *cy)) - r,
            Shape::Rect {
                minx,
                miny,
                maxx,
                maxy,
            } => {
                let dx = (minx - p.x).max(0.0).max(p.x - maxx);
                let dy = (miny - p.y).max(0.0).max(p.y - maxy);
                let outside = (dx * dx + dy * dy).sqrt();
                let inside = (p.x - minx).min(maxx - p.x).min(p.y - miny).min(maxy - p.y);
                if outside > 0.0 {
                    outside
                } else {
                    -inside.max(0.0)
                }
            }
            Shape::Capsule { x1, y1, x2, y2, r } => {
                segment_distance(p, Vec2::new(*x1, *y1), Vec2::new(*x2, *y2)) - r
            }
            Shape::Polygon { pts } => polygon_signed_distance(p, pts),
        }
    }

    pub fn contains(&self, p: Vec2) -> bool {
        self.signed_distance(p) <= f64::EPSILON
    }

    /// Geometric (boundary) gap between two shapes; 0.0 when they overlap or
    /// touch tangentially.
    pub fn gap_to(&self, other: &Shape) -> f64 {
        match (self, other) {
            (Shape::Circle { cx: ax, cy: ay, r: ra }, Shape::Circle { cx: bx, cy: by, r: rb }) => {
                let d = Vec2::new(*ax, *ay).distance(Vec2::new(*bx, *by));
                (d - ra - rb).max(0.0)
            }
            (
                Shape::Rect {
                    minx: ax0,
                    miny: ay0,
                    maxx: ax1,
                    maxy: ay1,
                },
                Shape::Rect {
                    minx: bx0,
                    miny: by0,
                    maxx: bx1,
                    maxy: by1,
                },
            ) => {
                let dx = (bx0 - ax1).max(0.0).max(ax0 - bx1);
                let dy = (by0 - ay1).max(0.0).max(ay0 - by1);
                (dx * dx + dy * dy).sqrt()
            }
            _ => generic_gap(self, other),
        }
    }

    /// Midpoint of the closest approach to `other`, used as a witness point.
    pub fn contact_witness(&self, other: &Shape) -> Vec2 {
        match (self, other) {
            (Shape::Circle { cx: ax, cy: ay, r: ra }, Shape::Circle { cx: bx, cy: by, r: rb }) => {
                let a = Vec2::new(*ax, *ay);
                let b = Vec2::new(*bx, *by);
                let dir = b.sub(a);
                let len = dir.length();
                if len <= f64::EPSILON {
                    return Vec2::new(*ax, *ay);
                }
                let u = dir.scale(1.0 / len);
                a.add(u.scale(ra + (len - ra - rb).max(0.0) / 2.0))
            }
            _ => {
                let c = self.centroid();
                let d = other.centroid();
                c.add(d.sub(c).scale(0.5))
            }
        }
    }

    pub fn centroid(&self) -> Vec2 {
        let (minx, miny, maxx, maxy) = self.bounds();
        Vec2::new((minx + maxx) / 2.0, (miny + maxy) / 2.0)
    }
}

fn segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f64 {
    let ab = b.sub(a);
    let len2 = ab.x * ab.x + ab.y * ab.y;
    if len2 <= f64::EPSILON {
        return p.distance(a);
    }
    let t = ((p.sub(a).x * ab.x + p.sub(a).y * ab.y) / len2).clamp(0.0, 1.0);
    p.distance(a.add(ab.scale(t)))
}

fn polygon_signed_distance(p: Vec2, pts: &[(f64, f64)]) -> f64 {
    let mut dmin = f64::MAX;
    let n = pts.len();
    for i in 0..n {
        let a = Vec2::new(pts[i].0, pts[i].1);
        let b = Vec2::new(pts[(i + 1) % n].0, pts[(i + 1) % n].1);
        dmin = dmin.min(segment_distance(p, a, b));
    }
    if point_in_polygon(p, pts) {
        -dmin
    } else {
        dmin
    }
}

pub fn point_in_polygon(p: Vec2, pts: &[(f64, f64)]) -> bool {
    let mut inside = false;
    let n = pts.len();
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = pts[i];
        let (xj, yj) = pts[j];
        if (yi > p.y) != (yj > p.y) {
            let hit_x = xi + (xj - xi) * (p.y - yi) / (yj - yi);
            if hit_x > p.x {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

fn generic_gap(a: &Shape, b: &Shape) -> f64 {
    // Sample boundary points; adequate for the axis/segment primitives used by
    // the bundled fixture and exact for circle/rect/capsule interactions after
    // a containment short-circuit.
    if let (Some(c), Some(r)) = (try_circle(a), try_circle(b)) {
        return (c.0.distance(c.1) - r.0 - r.1).max(0.0);
    }
    let (aminx, aminy, amaxx, amaxy) = a.bounds();
    let (bminx, bminy, bmaxx, bmaxy) = b.bounds();
    let dx = (bminx - amaxx).max(0.0).max(aminx - bmaxx);
    let dy = (bminy - amaxy).max(0.0).max(aminy - bmaxy);
    if dx == 0.0 && dy == 0.0 {
        return 0.0;
    }
    let mut best = (dx * dx + dy * dy).sqrt();
    for p in sample_boundary(a) {
        if b.signed_distance(p) <= f64::EPSILON {
            return 0.0;
        }
        best = best.min(b.signed_distance(p).max(0.0));
    }
    for p in sample_boundary(b) {
        if a.signed_distance(p) <= f64::EPSILON {
            return 0.0;
        }
        best = best.min(a.signed_distance(p).max(0.0));
    }
    best
}

fn try_circle(s: &Shape) -> Option<(Vec2, f64)> {
    match s {
        Shape::Circle { cx, cy, r } => Some((Vec2::new(*cx, *cy), *r)),
        _ => None,
    }
}

fn sample_boundary(s: &Shape) -> Vec<Vec2> {
    match s {
        Shape::Circle { cx, cy, r } => {
            let mut v = Vec::new();
            for k in 0..48 {
                let a = std::f64::consts::TAU * k as f64 / 48.0;
                v.push(Vec2::new(cx + r * a.cos(), cy + r * a.sin()));
            }
            v
        }
        Shape::Rect {
            minx,
            miny,
            maxx,
            maxy,
        } => vec![
            Vec2::new(*minx, *miny),
            Vec2::new(*maxx, *miny),
            Vec2::new(*maxx, *maxy),
            Vec2::new(*minx, *maxy),
            Vec2::new((*minx + *maxx) / 2.0, *miny),
            Vec2::new((*minx + *maxx) / 2.0, *maxy),
        ],
        Shape::Capsule { x1, y1, x2, y2, r } => {
            let mut v = vec![Vec2::new(*x1, *y1), Vec2::new(*x2, *y2)];
            for k in 0..16 {
                let a = std::f64::consts::TAU * k as f64 / 16.0;
                v.push(Vec2::new(x1 + r * a.cos(), y1 + r * a.sin()));
                v.push(Vec2::new(x2 + r * a.cos(), y2 + r * a.sin()));
            }
            v
        }
        Shape::Polygon { pts } => pts.iter().map(|(x, y)| Vec2::new(*x, *y)).collect(),
    }
}

/* ------------------------------ ordered raster ----------------------------- */

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pol {
    Dark,
    Clear,
}

/// One geometric instance placed in millimetres, carrying stable provenance.
#[derive(Debug, Clone)]
pub struct Placement {
    /// Stable id of the source draw/flash (content-derived).
    pub draw_id: Id,
    pub material_id: Id,
    pub layer_key: String,
    pub seq: usize,
    /// Replay sequence including step/repeat expansion (global op order).
    pub order: usize,
    pub pol: Pol,
    /// Step/repeat cell coordinates, (0,0) for the base instance.
    pub sr_col: u32,
    pub sr_row: u32,
    pub shape: Shape,
}

#[derive(Debug, Clone)]
pub struct Bounds {
    pub minx: f64,
    pub miny: f64,
    pub maxx: f64,
    pub maxy: f64,
}

impl Bounds {
    pub fn empty() -> Self {
        Bounds {
            minx: f64::MAX,
            miny: f64::MAX,
            maxx: f64::MIN,
            maxy: f64::MIN,
        }
    }
    pub fn union_shape(&mut self, s: &Shape) {
        let (a, b, c, d) = s.bounds();
        self.minx = self.minx.min(a);
        self.miny = self.miny.min(b);
        self.maxx = self.maxx.max(c);
        self.maxy = self.maxy.max(d);
    }
    pub fn valid(&self) -> bool {
        self.maxx >= self.minx && self.maxy >= self.miny
    }
}

/// Result of the ordered polarity raster + component labelling.
pub struct RasterModel {
    pub cols: usize,
    pub rows: usize,
    pub cell_mm: f64,
    pub origin: Vec2,
    /// 0 = clear, 1 = copper after full ordered replay.
    pub copper: Vec<u8>,
    /// Component label per copper cell, 0 = unlabelled.
    pub labels: Vec<u32>,
    pub component_count: u32,
}

pub struct RasterBudget {
    pub max_cells: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RasterError {
    /// Resource budget exhausted (resolution cannot fit the cell ceiling).
    BudgetExhausted { needed_cells: u64, ceiling: u64 },
    Empty,
}

impl RasterModel {
    /// Rasterize placements in order, applying dark then clear as they appear.
    /// `resolution_mm` is the target cell size; it is grown (never silently
    /// dropped) only enough to fit the cell ceiling; if it cannot fit even at
    /// the coarsest allowed cell (still able to resolve tolerance), this is
    /// `BudgetExhausted`.
    pub fn build(
        placements: &[Placement],
        resolution_mm: f64,
        budget: RasterBudget,
    ) -> Result<RasterModel, RasterError> {
        let mut bounds = Bounds::empty();
        for p in placements {
            bounds.union_shape(&p.shape);
        }
        if !bounds.valid() {
            return Err(RasterError::Empty);
        }
        let pad = resolution_mm.max(1e-6);
        bounds.minx -= pad;
        bounds.miny -= pad;
        bounds.maxx += pad;
        bounds.maxy += pad;

        let width = bounds.maxx - bounds.minx;
        let height = bounds.maxy - bounds.miny;

        // Coarsest cell that still resolves exact tangency: use a fixed cap so
        // budget exhaustion remains a distinct, reachable outcome.
        let max_cell = 0.5;
        let mut cell = resolution_mm.max(1e-6);
        let dims = |c: f64| -> (usize, usize, u64) {
            let cols = ((width / c).ceil() as usize).max(1);
            let rows = ((height / c).ceil() as usize).max(1);
            (cols, rows, (cols as u64) * (rows as u64))
        };
        let (mut cols, mut rows, mut cells) = dims(cell);
        while cells > budget.max_cells && cell < max_cell {
            cell *= 2.0;
            let (c, r, n) = dims(cell);
            cols = c;
            rows = r;
            cells = n;
        }
        if cells > budget.max_cells {
            return Err(RasterError::BudgetExhausted {
                needed_cells: cells,
                ceiling: budget.max_cells,
            });
        }

        let mut copper = vec![0u8; cols * rows];

        // Ordered paint: later ops overwrite earlier ones on covered cells.
        for p in placements {
            paint_shape(&mut copper, cols, rows, cell, &bounds, &p.shape, p.pol);
        }

        let mut labels = vec![0u32; cols * rows];
        let component_count = label_components(&copper, &mut labels, cols, rows);

        Ok(RasterModel {
            cols,
            rows,
            cell_mm: cell,
            origin: Vec2::new(bounds.minx, bounds.miny),
            copper,
            labels,
            component_count,
        })
    }

    pub fn label_at(&self, x: f64, y: f64) -> Option<u32> {
        let c = ((x - self.origin.x) / self.cell_mm).floor() as isize;
        let r = ((y - self.origin.y) / self.cell_mm).floor() as isize;
        if c < 0 || r < 0 || c >= self.cols as isize || r >= self.rows as isize {
            return None;
        }
        let lab = self.labels[r as usize * self.cols + c as usize];
        if lab == 0 {
            None
        } else {
            Some(lab)
        }
    }
}

fn paint_shape(
    copper: &mut [u8],
    cols: usize,
    rows: usize,
    cell: f64,
    bounds: &Bounds,
    shape: &Shape,
    pol: Pol,
) {
    let (minx, miny, maxx, maxy) = shape.bounds();
    let c0 = (((minx - bounds.minx) / cell).floor() as isize).max(0);
    let r0 = (((miny - bounds.miny) / cell).floor() as isize).max(0);
    let c1 = (((maxx - bounds.minx) / cell).ceil() as isize).min(cols as isize - 1);
    let r1 = (((maxy - bounds.miny) / cell).ceil() as isize).min(rows as isize - 1);
    let value = match pol {
        Pol::Dark => 1u8,
        Pol::Clear => 0u8,
    };
    for r in r0..=r1 {
        for c in c0..=c1 {
            let px = bounds.minx + (c as f64 + 0.5) * cell;
            let py = bounds.miny + (r as f64 + 0.5) * cell;
            if shape.signed_distance(Vec2::new(px, py)) <= 0.0 {
                copper[r as usize * cols + c as usize] = value;
            }
        }
    }
}

fn label_components(copper: &[u8], labels: &mut [u32], cols: usize, rows: usize) -> u32 {
    let mut next = 1u32;
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            let idx = r * cols + c;
            if copper[idx] == 0 || labels[idx] != 0 {
                continue;
            }
            let tag = next;
            next += 1;
            labels[idx] = tag;
            stack.push((c, r));
            while let Some((x, y)) = pop(&mut stack) {
                for (nx, ny) in neighbors(x, y, cols, rows) {
                    let ni = ny * cols + nx;
                    if copper[ni] != 0 && labels[ni] == 0 {
                        labels[ni] = tag;
                        stack.push((nx, ny));
                    }
                }
            }
        }
    }
    next - 1
}

fn pop<T>(v: &mut Vec<T>) -> Option<T> {
    v.pop()
}

fn neighbors(x: usize, y: usize, cols: usize, rows: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (dx, dy) in [
        (-1isize, 0isize),
        (1, 0),
        (0, -1),
        (0, 1),
    ] {
        let nx = x as isize + dx;
        let ny = y as isize + dy;
        if nx >= 0 && ny >= 0 && nx < cols as isize && ny < rows as isize {
            out.push((nx as usize, ny as usize));
        }
    }
    out
}
