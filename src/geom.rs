//! Raster geometry engine.
//!
//! Copper is composed strictly in operation order: positive polarity adds,
//! negative polarity subtracts. Contact tolerance is a versioned parameter
//! applied only when deriving connectivity components, never baked into the
//! stored geometry.
use crate::model::{CaseError, Nm};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Raster quantum: 1 micrometre, in nanometres.
pub const Q: Nm = 1_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Shape {
    Circle { c: [Nm; 2], r: Nm },
    Rect { c: [Nm; 2], hw: Nm, hh: Nm },
    Capsule { a: [Nm; 2], b: [Nm; 2], r: Nm },
    SegRect { a: [Nm; 2], b: [Nm; 2], hw: Nm, hh: Nm },
}

impl Shape {
    pub fn translate(&self, dx: Nm, dy: Nm) -> Shape {
        match self.clone() {
            Shape::Circle { c, r } => Shape::Circle { c: [c[0] + dx, c[1] + dy], r },
            Shape::Rect { c, hw, hh } => Shape::Rect { c: [c[0] + dx, c[1] + dy], hw, hh },
            Shape::Capsule { a, b, r } => Shape::Capsule { a: [a[0] + dx, a[1] + dy], b: [b[0] + dx, b[1] + dy], r },
            Shape::SegRect { a, b, hw, hh } => Shape::SegRect { a: [a[0] + dx, a[1] + dy], b: [b[0] + dx, b[1] + dy], hw, hh },
        }
    }

    /// Approximate every shape as a polygon in integer nm.
    pub fn to_polygon(&self) -> Vec<(Nm, Nm)> {
        match *self {
            Shape::Circle { c, r } => ngon(c, r, r, 64),
            Shape::Rect { c, hw, hh } => vec![
                (c[0] - hw, c[1] - hh), (c[0] + hw, c[1] - hh),
                (c[0] + hw, c[1] + hh), (c[0] - hw, c[1] + hh),
            ],
            Shape::Capsule { a, b, r } => {
                // stadium polygon: two semicircles
                let n = 32;
                let (dx, dy) = ((b[0] - a[0]) as f64, (b[1] - a[1]) as f64);
                let ang = dy.atan2(dx);
                let mut pts = Vec::with_capacity(2 * n + 2);
                for i in 0..=n {
                    let t = ang + std::f64::consts::FRAC_PI_2 - (i as f64) * std::f64::consts::PI / n as f64;
                    pts.push((b[0] + (r as f64 * t.cos()).round() as Nm, b[1] + (r as f64 * t.sin()).round() as Nm));
                }
                for i in 0..=n {
                    let t = ang - std::f64::consts::FRAC_PI_2 - (i as f64) * std::f64::consts::PI / n as f64;
                    pts.push((a[0] + (r as f64 * t.cos()).round() as Nm, a[1] + (r as f64 * t.sin()).round() as Nm));
                }
                pts
            }
            Shape::SegRect { a, b, hw, hh } => {
                // convex hull of the two endpoint rectangles
                let mut pts = vec![
                    (a[0] - hw, a[1] - hh), (a[0] + hw, a[1] - hh), (a[0] + hw, a[1] + hh), (a[0] - hw, a[1] + hh),
                    (b[0] - hw, b[1] - hh), (b[0] + hw, b[1] - hh), (b[0] + hw, b[1] + hh), (b[0] - hw, b[1] + hh),
                ];
                convex_hull(&mut pts)
            }
        }
    }
}

fn ngon(c: [Nm; 2], rx: Nm, ry: Nm, n: usize) -> Vec<(Nm, Nm)> {
    (0..n)
        .map(|i| {
            let t = 2.0 * std::f64::consts::PI * i as f64 / n as f64;
            (c[0] + (rx as f64 * t.cos()).round() as Nm, c[1] + (ry as f64 * t.sin()).round() as Nm)
        })
        .collect()
}

fn convex_hull(pts: &mut Vec<(Nm, Nm)>) -> Vec<(Nm, Nm)> {
    pts.sort();
    pts.dedup();
    if pts.len() <= 2 { return pts.clone(); }
    let cross = |o: (Nm, Nm), a: (Nm, Nm), b: (Nm, Nm)| -> i128 {
        (a.0 - o.0) as i128 * (b.1 - o.1) as i128 - (a.1 - o.1) as i128 * (b.0 - o.0) as i128
    };
    let mut lower: Vec<(Nm, Nm)> = vec![];
    for &p in pts.iter() {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0 { lower.pop(); }
        lower.push(p);
    }
    let mut upper: Vec<(Nm, Nm)> = vec![];
    for &p in pts.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0 { upper.pop(); }
        upper.push(p);
    }
    lower.pop(); upper.pop();
    lower.extend(upper);
    lower
}

#[derive(Debug)]
pub struct Budget {
    pub max_cells: u64,
    pub used: u64,
}

impl Budget {
    pub fn new(max_cells: u64) -> Self { Budget { max_cells, used: 0 } }
    pub fn charge(&mut self, cells: u64) -> Result<(), CaseError> {
        self.used += cells;
        if self.used > self.max_cells {
            return Err(CaseError::ResourceBudgetExhausted {
                detail: "raster cell budget".into(), used: self.used, budget: self.max_cells,
            });
        }
        Ok(())
    }
}

/// Merge sorted disjoint intervals with a new one (in place union).
fn union_into(rows: &mut Vec<(Nm, Nm)>, iv: (Nm, Nm)) {
    rows.push(iv);
    rows.sort();
    let mut out: Vec<(Nm, Nm)> = Vec::with_capacity(rows.len());
    for &(a, b) in rows.iter() {
        if let Some(last) = out.last_mut() {
            if a <= last.1 { last.1 = last.1.max(b); continue; }
        }
        out.push((a, b));
    }
    *rows = out;
}

/// self -= other (per row interval subtraction).
fn subtract_rows(dst: &mut Vec<(Nm, Nm)>, sub: &[(Nm, Nm)]) {
    let mut out = Vec::with_capacity(dst.len());
    for &(a, b) in dst.iter() {
        let mut segs = vec![(a, b)];
        for &(sa, sb) in sub {
            let mut next = vec![];
            for (x0, x1) in segs {
                if sb <= x0 || sa >= x1 { next.push((x0, x1)); continue; }
                if sa > x0 { next.push((x0, sa.min(x1))); }
                if sb < x1 { next.push((sb.max(x0), x1)); }
            }
            segs = next;
        }
        out.extend(segs);
    }
    *dst = out;
}

/// Sparse row-interval raster. Rows are indexed in quantum units; each row
/// holds sorted disjoint x-intervals [a, b) in quantum units.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Raster {
    pub rows: BTreeMap<i64, Vec<(i64, i64)>>,
}

impl Raster {
    pub fn cell_count(&self) -> u64 {
        self.rows.values().flat_map(|v| v.iter()).map(|(a, b)| (b - a) as u64).sum()
    }

    fn apply_polygon(&mut self, poly: &[(Nm, Nm)], add: bool, budget: &mut Budget) -> Result<(), CaseError> {
        if poly.len() < 3 { return Ok(()); }
        let miny = poly.iter().map(|p| p.1).min().unwrap();
        let maxy = poly.iter().map(|p| p.1).max().unwrap();
        let r0 = miny.div_euclid(Q);
        let r1 = maxy.div_euclid(Q);
        let n = poly.len();
        for row in r0..=r1 {
            let y = row * Q + Q / 2; // sample line at strip centre
            let mut xs: Vec<Nm> = vec![];
            for i in 0..n {
                let (x1, y1) = poly[i];
                let (x2, y2) = poly[(i + 1) % n];
                if (y1 <= y && y2 > y) || (y2 <= y && y1 > y) {
                    // x at intersection (integer interpolation)
                    let dy = (y2 - y1) as i128;
                    let x = x1 as i128 + ((y - y1) as i128) * ((x2 - x1) as i128) / dy;
                    xs.push(x as Nm);
                }
            }
            xs.sort();
            let mut pairs = xs.chunks_exact(2);
            for seg in &mut pairs {
                let ca = seg.0.div_euclid(Q);
                let cb = (seg.1 + Q - 1).div_euclid(Q);
                if cb <= ca { continue; }
                budget.charge((cb - ca) as u64)?;
                if add {
                    union_into(self.rows.entry(row).or_default(), (ca, cb));
                } else {
                    let sub = vec![(ca, cb)];
                    if let Some(row_ivs) = self.rows.get_mut(&row) {
                        subtract_rows(row_ivs, &sub);
                    }
                }
            }
        }
        self.rows.retain(|_, v| !v.is_empty());
        Ok(())
    }

    pub fn add_shape(&mut self, s: &Shape, budget: &mut Budget) -> Result<(), CaseError> {
        self.apply_polygon(&s.to_polygon(), true, budget)
    }
    pub fn subtract_shape(&mut self, s: &Shape, budget: &mut Budget) -> Result<(), CaseError> {
        self.apply_polygon(&s.to_polygon(), false, budget)
    }

    /// All runs in nanometres: (row_y_nm, x0_nm, x1_nm).
    pub fn runs_nm(&self) -> Vec<(Nm, Nm, Nm)> {
        let mut out = vec![];
        for (&row, ivs) in &self.rows {
            for &(a, b) in ivs {
                out.push((row * Q, a * Q, b * Q));
            }
        }
        out
    }

    /// Point-in-copper test (nm), for tests and contact queries.
    pub fn covers(&self, x: Nm, y: Nm) -> bool {
        let row = y.div_euclid(Q);
        let cell = x.div_euclid(Q);
        if let Some(ivs) = self.rows.get(&row) {
            ivs.iter().any(|&(a, b)| cell >= a && cell < b)
        } else { false }
    }

    /// Connected components under the versioned contact tolerance.
    /// Two runs touch when their dilated-by-tolerance intervals intersect.
    pub fn components(&self, tol: Nm) -> Vec<Vec<(Nm, Nm, Nm)>> {
        let runs = self.runs_nm();
        let mut parent: Vec<usize> = (0..runs.len()).collect();
        fn find(p: &mut Vec<usize>, i: usize) -> usize {
            if p[i] != i { p[i] = find(p, p[i]); }
            p[i]
        }
        // group run indices by row
        let mut by_row: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
        for (i, r) in runs.iter().enumerate() { by_row.entry(r.0).or_default().push(i); }
        let rows: Vec<i64> = by_row.keys().cloned().collect();
        for (ri, &row) in rows.iter().enumerate() {
            for &row2 in rows.iter().skip(ri) {
                let dy = row2 - row;
                if dy > tol + Q { break; }
                for &i in &by_row[&row] {
                    for &j in &by_row[&row2] {
                        if i >= j { continue; }
                        let (_, ax0, ax1) = runs[i];
                        let (_, bx0, bx1) = runs[j];
                        if ax0 - tol < bx1 && bx0 - tol < ax1 {
                            let (fi, fj) = (find(&mut parent, i), find(&mut parent, j));
                            if fi != fj { parent[fi] = fj; }
                        }
                    }
                }
            }
        }
        let mut groups: BTreeMap<usize, Vec<(Nm, Nm, Nm)>> = BTreeMap::new();
        for (i, r) in runs.iter().enumerate() {
            let f = find(&mut parent, i);
            groups.entry(f).or_default().push(*r);
        }
        let mut comps: Vec<Vec<(Nm, Nm, Nm)>> = groups.into_values().collect();
        // stable order: by min (y, x0) of first run
        for c in &mut comps { c.sort(); }
        comps.sort_by_key(|c| c[0]);
        comps
    }
}

/// Stable component identity: hash of layer + sorted runs.
pub fn component_id(layer: &str, runs: &[(Nm, Nm, Nm)]) -> String {
    let mut s = String::from(layer);
    for r in runs { s.push_str(&format!(";{},{},{}", r.0, r.1, r.2)); }
    crate::model::make_id("cmp", &[&s])
}
