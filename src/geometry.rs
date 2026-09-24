//! Integer geometry on a common derived basis (nanometres, i64).
//!
//! Only axis-aligned primitives are reconstructed, matching the offline case
//! material. Rotation/arcs are refused upstream with diagnostics instead of
//! being approximated. Distance predicates are exact integer arithmetic; no
//! tolerance is silently applied here.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Point {
    pub x: i64,
    pub y: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Shape {
    Circle { cx: i64, cy: i64, r: i64 },
    Rect { x0: i64, y0: i64, x1: i64, y1: i64 },
    /// Capsule: segment with radius (extruded round-end stroke).
    Capsule { ax: i64, ay: i64, bx: i64, by: i64, r: i64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub x0: i64,
    pub y0: i64,
    pub x1: i64,
    pub y1: i64,
}

impl Shape {
    pub fn bounds(self) -> Bounds {
        match self {
            Shape::Circle { cx, cy, r } => Bounds { x0: cx - r, y0: cy - r, x1: cx + r, y1: cy + r },
            Shape::Rect { x0, y0, x1, y1 } => Bounds { x0, y0, x1, y1 },
            Shape::Capsule { ax, ay, bx, by, r } => Bounds {
                x0: ax.min(bx) - r,
                y0: ay.min(by) - r,
                x1: ax.max(bx) + r,
                y1: ay.max(by) + r,
            },
        }
    }
}

/// Signed squared gap: negative means area overlap, zero means tangent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Contact {
    /// Interior overlap (shared area), `penetration = -signed_gap >= 0`.
    Overlap { penetration: i64 },
    /// Boundary tangency: a single point or line segment shared, zero area.
    Tangent,
    /// Strictly disjoint, squared Euclidean gap in nm^2.
    Gap { gap_sq: i64 },
}

fn iabs(v: i64) -> i64 {
    v.unsigned_abs() as i64
}

/// Distance^2 between a point and an axis-aligned segment.
fn point_hseg_dist2(px: i64, py: i64, ax: i64, ay: i64, bx: i64, by: i64) -> i64 {
    let (dx, dy) = (bx - ax, by - ay);
    let t = if dx == 0 && dy == 0 {
        0i64
    } else {
        let num = (px - ax) * dx + (py - ay) * dy;
        let den = dx * dx + dy * dy;
        (num.clamp(0, den)) / den.max(1)
    };
    let qx = ax + dx * t;
    let qy = ay + dy * t;
    iabs(px - qx).pow(2) + iabs(py - qy).pow(2)
}

pub fn contact(a: Shape, b: Shape) -> Contact {
    use Shape::*;
    match (a, b) {
        (Circle { cx: acx, cy: acy, r: ar }, Circle { cx: bcx, cy: bcy, r: br }) => {
            let d2 = iabs(acx - bcx).pow(2) + iabs(acy - bcy).pow(2);
            let rr = ar + br;
            if d2 < rr * rr {
                // penetration estimate by squared difference is not linear;
                // keep an integer lower bound sufficient for ordering.
                Contact::Overlap { penetration: rr * rr - d2 }
            } else if d2 == rr * rr {
                Contact::Tangent
            } else {
                Contact::Gap { gap_sq: d2 - rr * rr }
            }
        }
        (Rect { x0: ax0, y0: ay0, x1: ax1, y1: ay1 }, Rect { x0: bx0, y0: by0, x1: bx1, y1: by1 }) => {
            let ox = (ax1 - bx0).min(bx1 - ax0); // >=0 when projections overlap
            let oy = (ay1 - by0).min(by1 - ay0);
            if ox > 0 && oy > 0 {
                Contact::Overlap { penetration: ox.min(oy) }
            } else if ox >= 0 && oy >= 0 {
                Contact::Tangent
            } else {
                let gx = if ox < 0 { -ox } else { 0 };
                let gy = if oy < 0 { -oy } else { 0 };
                Contact::Gap { gap_sq: gx * gx + gy * gy }
            }
        }
        (Circle { cx, cy, r }, Rect { x0, y0, x1, y1 })
        | (Rect { x0, y0, x1, y1 }, Circle { cx, cy, r }) => {
            let qx = cx.clamp(x0, x1);
            let qy = cy.clamp(y0, y1);
            let d2 = iabs(cx - qx).pow(2) + iabs(cy - qy).pow(2);
            if d2 < r * r {
                Contact::Overlap { penetration: r * r - d2 }
            } else if d2 == r * r {
                Contact::Tangent
            } else {
                Contact::Gap { gap_sq: d2 - r * r }
            }
        }
        (Capsule { ax, ay, bx, by, r }, Circle { cx, cy, r: cr })
        | (Circle { cx, cy, r: cr }, Capsule { ax, ay, bx, by, r }) => {
            let d2 = point_hseg_dist2(cx, cy, ax, ay, bx, by);
            let rr = r + cr;
            if d2 < rr * rr {
                Contact::Overlap { penetration: rr * rr - d2 }
            } else if d2 == rr * rr {
                Contact::Tangent
            } else {
                Contact::Gap { gap_sq: d2 - rr * rr }
            }
        }
        (Capsule { ax: ax2, ay: ay2, bx: bx2, by: by2, r: r2 }, Rect { x0, y0, x1, y1 })
        | (Rect { x0, y0, x1, y1 }, Capsule { ax: ax2, ay: ay2, bx: bx2, by: by2, r: r2 }) => {
            // Distance segment -> rect boundary via the four edges; sample the
            // segment exactly against each axis-aligned edge using projection.
            let rect = Shape::Rect { x0, y0, x1, y1 };
            // Quick accept: either endpoint inside rect.
            for (px, py) in [(ax2, ay2), (bx2, by2)] {
                if px >= x0 && px <= x1 && py >= y0 && py <= y1 {
                    return Contact::Overlap { penetration: 1 };
                }
            }
            let edges = [
                (x0, y0, x1, y0),
                (x1, y0, x1, y1),
                (x1, y1, x0, y1),
                (x0, y1, x0, y0),
            ];
            let mut best: Option<i64> = None;
            for &(ex0, ey0, ex1, ey1) in &edges {
                let d2 = seg_dist2(ax2, ay2, bx2, by2, ex0, ey0, ex1, ey1);
                best = Some(best.map_or(d2, |v| v.min(d2)));
            }
            let d2 = best.unwrap_or(i64::MAX);
            if d2 < r2 * r2 {
                Contact::Overlap { penetration: r2 * r2 - d2 }
            } else if d2 == r2 * r2 {
                Contact::Tangent
            } else {
                let _ = rect;
                Contact::Gap { gap_sq: d2 - r2 * r2 }
            }
        }
        (Capsule { ax: aax, ay: aay, bx: abx, by: aby, r: ar },
         Capsule { ax: bax, ay: bay, bx: bbx, by: bby, r: br }) => {
            let d2 = seg_dist2(aax, aay, abx, aby, bax, bay, bbx, bby);
            let rr = ar + br;
            if d2 < rr * rr {
                Contact::Overlap { penetration: rr * rr - d2 }
            } else if d2 == rr * rr {
                Contact::Tangent
            } else {
                Contact::Gap { gap_sq: d2 - rr * rr }
            }
        }
    }
}

/// Squared distance between two axis-aligned segments (horizontal or vertical).
fn seg_dist2(ax: i64, ay: i64, bx: i64, by: i64, cx: i64, cy: i64, dx: i64, dy: i64) -> i64 {
    let horiz_a = ay == by;
    let horiz_c = cy == dy;
    match (horiz_a, horiz_c) {
        (true, true) => {
            if ay == cy {
                let gap = interval_gap(ax.min(bx), ax.max(bx), cx.min(dx), cx.max(dx));
                gap * gap
            } else {
                // Parallel horizontal, different rows.
                let gx = interval_gap(ax.min(bx), ax.max(bx), cx.min(dx), cx.max(dx));
                gx * gx + iabs(ay - cy).pow(2)
            }
        }
        (false, false) => {
            if ax == cx {
                let gap = interval_gap(ay.min(by), ay.max(by), cy.min(dy), cy.max(dy));
                gap * gap
            } else {
                let gy = interval_gap(ay.min(by), ay.max(by), cy.min(dy), cy.max(dy));
                gy * gy + iabs(ax - cx).pow(2)
            }
        }
        (true, false) => cross_dist2(ax, ay, bx, cx, cy, dy),
        (false, true) => cross_dist2(cx, cy, dx, ax, ay, by),
    }
}

/// Gap between two 1D intervals; negative means overlap length, 0 = touch.
fn interval_gap(a0: i64, a1: i64, b0: i64, b1: i64) -> i64 {
    if a1 < b0 {
        b0 - a1
    } else if b1 < a0 {
        a0 - b1
    } else {
        -(a1.min(b1) - a0.max(b0))
    }
}

/// Horizontal segment (h0,y)-(h1,y) vs vertical segment (x,v0)-(x,v1).
fn cross_dist2(h0: i64, y: i64, h1: i64, x: i64, v0: i64, v1: i64) -> i64 {
    let hlo = h0.min(h1);
    let hhi = h0.max(h1);
    let vlo = v0.min(v1);
    let vhi = v0.max(v1);
    let inside = x >= hlo && x <= hhi && y >= vlo && y <= vhi;
    if inside {
        0
    } else {
        let gx = if x < hlo { hlo - x } else if x > hhi { x - hhi } else { 0 };
        let gy = if y < vlo { vlo - y } else if y > vhi { y - vhi } else { 0 };
        gx * gx + gy * gy
    }
}

/// True Euclidean classification against a non-negative gap budget (nm).
pub fn bridges(gap_sq: i64, tolerance_nm: i64) -> bool {
    gap_sq >= 0 && gap_sq <= tolerance_nm * tolerance_nm
}
