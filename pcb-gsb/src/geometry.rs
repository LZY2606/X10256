//! Deterministic raster geometry engine.
//!
//! All coordinates are millimetres in the project coordinate frame. Rasterisation is
//! purely a derived representation; every result records its grid resolution (a rule
//! parameter) so re-renders under a different parameter version are reproducible.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Pt {
    pub x: f64,
    pub y: f64,
}

impl Pt {
    pub fn new(x: f64, y: f64) -> Self {
        Pt { x, y }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Circle {
    pub c: Pt,
    pub d: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub min: Pt,
    pub max: Pt,
}

/// A planar draw primitive in millimetres. Arcs are approximated by the parser.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Shape {
    Circle(Circle),
    Rect(Rect),
    Polygon { pts: Vec<Pt> },
    Segment { a: Pt, b: Pt, width: f64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrawnShape {
    pub shape: Shape,
    /// aperture macro/number responsible, retained for provenance.
    pub aperture: String,
    /// serial index of the source draw operation.
    pub op_index: usize,
    /// polarity in effect when the operation was emitted.
    pub polarity: super::model::Polarity,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayerDraw {
    pub source_id: String,
    pub shapes: Vec<DrawnShape>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GridDef {
    pub res_mm: f64,
    pub origin_x: f64,
    pub origin_y: f64,
    pub width: usize,
    pub height: usize,
}

pub type Grid = GridDef;

impl GridDef {
    pub fn x_of(&self, col: usize) -> f64 {
        self.origin_x + (col as f64 + 0.5) * self.res_mm
    }
    pub fn y_of(&self, row: usize) -> f64 {
        self.origin_y + (row as f64 + 0.5) * self.res_mm
    }
    pub fn col_of(&self, x: f64) -> i64 {
        ((x - self.origin_x) / self.res_mm).floor() as i64
    }
    pub fn row_of(&self, y: f64) -> i64 {
        ((y - self.origin_y) / self.res_mm).floor() as i64
    }
}

/// Coverage mask; 0 = empty, 1 = covered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mask {
    pub grid: GridDef,
    pub cells: Vec<u8>,
}

impl Mask {
    pub fn empty(g: GridDef) -> Self {
        Mask {
            grid: g,
            cells: vec![0u8; g.width * g.height],
        }
    }
    #[inline]
    fn idx(&self, r: usize, c: usize) -> usize {
        r * self.grid.width + c
    }
    pub fn set(&mut self, r: i64, c: i64) {
        if c >= 0 && r >= 0 && (c as usize) < self.grid.width && (r as usize) < self.grid.height {
            let i = self.idx(r as usize, c as usize);
            self.cells[i] = 1;
        }
    }
    pub fn get(&self, r: i64, c: i64) -> u8 {
        if c < 0 || r < 0 || (c as usize) >= self.grid.width || (r as usize) >= self.grid.height {
            0
        } else {
            self.cells[self.idx(r as usize, c as usize)]
        }
    }
    pub fn paint_circle(&mut self, cx: f64, cy: f64, d: f64) {
        let g = &self.grid;
        let radius = (d * 0.5).max(0.0);
        let c0 = g.origin_x;
        let c1 = g.origin_y;
        let res = g.res_mm;
        let r_cells = radius / res;
        let cc = (cx - c0) / res - 0.5;
        let cr = (cy - c1) / res - 0.5;
        let r0 = (cr - r_cells - 1.0).floor() as i64;
        let r1 = (cr + r_cells + 1.0).ceil() as i64;
        let ccol0 = (cc - r_cells - 1.0).floor() as i64;
        let ccol1 = (cc + r_cells + 1.0).ceil() as i64;
        // 2x2 supersample keeps small pads / thin drills stable at the coarse grid.
        for r in r0..=r1 {
            for col in ccol0..=ccol1 {
                let mut hit = 0u8;
                for sr in 0..2 {
                    for sc in 0..2 {
                        let sx = c0 + (col as f64 + (sc as f64 + 0.5) / 2.0) * res;
                        let sy = c1 + (r as f64 + (sr as f64 + 0.5) / 2.0) * res;
                        let dx = sx - cx;
                        let dy = sy - cy;
                        if dx * dx + dy * dy <= radius * radius {
                            hit = 1;
                        }
                    }
                }
                if hit == 1 {
                    self.set(r, col);
                }
            }
        }
    }
    pub fn paint_rect(&mut self, xmin: f64, ymin: f64, xmax: f64, ymax: f64) {
        let g = &self.grid;
        let c0 = (g.col_of(xmin)).max(0);
        let c1 = (g.col_of(xmax)).min(g.width as i64 - 1);
        let r0 = (g.row_of(ymin)).max(0);
        let r1 = (g.row_of(ymax)).min(g.height as i64 - 1);
        for r in r0..=r1 {
            for col in c0..=c1 {
                self.set(r, col);
            }
        }
    }
    fn paint_scanline_polygon(&mut self, pts: &[Pt]) {
        if pts.len() < 3 {
            return;
        }
        let g = &self.grid;
        let (mut min_y, mut max_y) = (f64::INFINITY, f64::NEG_INFINITY);
        for p in pts {
            min_y = min_y.min(p.y);
            max_y = max_y.max(p.y);
        }
        let row_start = (g.row_of(min_y)).max(0).max(0);
        let row_end = (g.row_of(max_y) + 1).min(g.height as i64 - 1);
        for r in row_start..=row_end {
            let y = g.y_of(r as usize);
            let mut xs: Vec<f64> = Vec::new();
            let n = pts.len();
            for i in 0..n {
                let a = pts[i];
                let b = pts[(i + 1) % n];
                if (a.y <= y && b.y > y) || (b.y <= y && a.y > y) {
                    let t = (y - a.y) / (b.y - a.y);
                    xs.push(a.x + t * (b.x - a.x));
                }
            }
            xs.sort_by(|p, q| p.partial_cmp(q).unwrap());
            let mut i = 0;
            while i + 1 < xs.len() {
                let xa = xs[i];
                let xb = xs[i + 1];
                let ca = g.col_of(xa).max(0);
                let cb = g.col_of(xb).min(g.width as i64 - 1);
                for col in ca..=cb {
                    self.set(r, col);
                }
                i += 2;
            }
        }
    }
    pub fn paint_segment(&mut self, ax: f64, ay: f64, bx: f64, by: f64, width: f64) {
        let radius = width * 0.5;
        let dx = bx - ax;
        let dy = by - ay;
        let len = (dx * dx + dy * dy).sqrt();
        if len <= f64::EPSILON {
            self.paint_circle(ax, ay, width);
            return;
        }
        let steps = (len / (self.grid.res_mm * 0.5)).ceil() as i64;
        for i in 0..=steps.max(1) {
            let t = i as f64 / steps.max(1) as f64;
            self.paint_circle(ax + dx * t, ay + dy * t, width * 0.92);
        }
        // explicit extents guarantee closed joins
        let _ = radius;
    }
    pub fn paint_shape(&mut self, shape: &Shape) {
        match shape {
            Shape::Circle(c) => self.paint_circle(c.c.x, c.c.y, c.d),
            Shape::Rect(r) => self.paint_rect(r.min.x, r.min.y, r.max.x, r.max.y),
            Shape::Polygon { pts } => self.paint_scanline_polygon(pts),
            Shape::Segment { a, b, width } => self.paint_segment(a.x, a.y, b.x, b.y, *width),
        }
    }

    /// Subtract `other` from `self` (clear polarity).
    pub fn subtract(&mut self, other: &Mask) {
        for (a, b) in self.cells.iter_mut().zip(other.cells.iter()) {
            if *b == 1 {
                *a = 0;
            }
        }
    }

    pub fn bbox_cells(&self) -> Option<(usize, usize, usize, usize)> {
        let w = self.grid.width;
        let h = self.grid.height;
        let (mut rmin, mut rmax, mut cmin, mut cmax) = (h, 0usize, w, 0usize);
        for r in 0..h {
            for c in 0..w {
                if self.cells[r * w + c] == 1 {
                    rmin = rmin.min(r);
                    rmax = rmax.max(r);
                    cmin = cmin.min(c);
                    cmax = cmax.max(c);
                }
            }
        }
        if cmin > cmax {
            None
        } else {
            Some((rmin, rmax, cmin, cmax))
        }
    }

    /// Label 4-connected foreground runs. Returns label image (0 = background) and
    /// per-label cell counts. Equivalent inputs always produce the same label ordering:
    /// labels are assigned by first cell encountered in row-major raster order.
    pub fn label_components_4(&self) -> (Vec<u32>, Vec<u64>) {
        let w = self.grid.width;
        let h = self.grid.height;
        let mut lab = vec![0u32; w * h];
        let mut sizes: Vec<u64> = Vec::new();
        let mut stack: Vec<usize> = Vec::new();
        let mut next: u32 = 1;
        for r in 0..h {
            for c in 0..w {
                let i = r * w + c;
                if self.cells[i] == 0 || lab[i] != 0 {
                    continue;
                }
                let cur = next;
                next += 1;
                sizes.push(0);
                lab[i] = cur;
                stack.push(i);
                while let Some(idx) = stack.pop() {
                    sizes[(cur - 1) as usize] += 1;
                    let rr = idx / w;
                    let cc = idx % w;
                    if cc > 0 {
                        let n = idx - 1;
                        if self.cells[n] == 1 && lab[n] == 0 {
                            lab[n] = cur;
                            stack.push(n);
                        }
                    }
                    if cc + 1 < w {
                        let n = idx + 1;
                        if self.cells[n] == 1 && lab[n] == 0 {
                            lab[n] = cur;
                            stack.push(n);
                        }
                    }
                    if rr > 0 {
                        let n = idx - w;
                        if self.cells[n] == 1 && lab[n] == 0 {
                            lab[n] = cur;
                            stack.push(n);
                        }
                    }
                    if rr + 1 < h {
                        let n = idx + w;
                        if self.cells[n] == 1 && lab[n] == 0 {
                            lab[n] = cur;
                            stack.push(n);
                        }
                    }
                }
            }
        }
        (lab, sizes)
    }

    pub fn count(&self) -> u64 {
        self.cells.iter().map(|c| *c as u64).sum()
    }
}
