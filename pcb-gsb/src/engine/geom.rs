//! Canonical mm geometry primitives.
//!
//! Derived layer only: sources stay in their original representation
//! ([`crate::gerber`], [`crate::excellon`]); everything here is already
//! normalised to millimetres in the board coordinate frame.

use crate::gerber::Polarity;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum Shape {
    Circle { cx: f64, cy: f64, r: f64 },
    Rect { x: f64, y: f64, w: f64, h: f64 },
    /// Capsule: rectangle `[x1,x2] x [y1,y2]` of a draw, with rounded ends.
    Capsule { x1: f64, y1: f64, x2: f64, y2: f64, w: f64, h: f64 },
    /// Simple axis-aligned rectilinear polygon (filled), points in order, mm.
    Polygon { points: Vec<(f64, f64)> },
}

impl Shape {
    pub fn bounds(&self) -> (f64, f64, f64, f64) {
        match self {
            Shape::Circle { cx, cy, r } => (cx - r, cy - r, cx + r, cy + r),
            Shape::Rect { x, y, w, h } => (*x, *y, x + w, y + h),
            Shape::Capsule { x1, y1, x2, y2, .. } => {
                (x1.min(*x2), y1.min(*y2), x1.max(*x2), y1.max(*y2))
            }
            Shape::Polygon { points } => {
                let x0 = points.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
                let x1 = points.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
                let y0 = points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
                let y1 = points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
                (x0, y0, x1, y1)
            }
        }
    }

    pub fn area(&self) -> f64 {
        match self {
            Shape::Circle { r, .. } => std::f64::consts::PI * r * r,
            Shape::Rect { w, h, .. } => w * h,
            Shape::Capsule { w, h, x1, x2, y1, y2 } => {
                let r = (w.min(*h)) / 2.0;
                let len = ((x2 - x1).powi(2) + (y2 - y1).powi(2)).sqrt();
                if (x1 - x2).abs() >= (y1 - y2).abs() {
                    w * h.max(0.0) + std::f64::consts::PI * r * r
                } else {
                    h.max(0.0) * len + std::f64::consts::PI * r * r
                }
            }
            Shape::Polygon { points } => polygon_area(points),
        }
    }

    /// Closed-set point containment (boundary counts as contact).
    pub fn contains(&self, px: f64, py: f64) -> bool {
        match self {
            Shape::Circle { cx, cy, r } => {
                let dx = px - cx;
                let dy = py - cy;
                dx * dx + dy * dy <= r * r + 1e-12
            }
            Shape::Rect { x, y, w, h } => {
                px >= x - 1e-12 && px <= x + w + 1e-12 && py >= y - 1e-12 && py <= y + h + 1e-12
            }
            Shape::Capsule { x1, y1, x2, y2, w, h } => {
                let r = (w.min(*h)) / 2.0;
                let minx = x1.min(*x2);
                let maxx = x1.max(*x2);
                let miny = y1.min(*y2);
                let maxy = y1.max(*y2);
                if (x1 - x2).abs() >= (y1 - y2).abs() {
                    if px >= minx && px <= maxx && py >= miny - 1e-12 && py <= maxy + 1e-12 {
                        return true;
                    }
                    dist2(px, py, *x1, *y1) <= r * r + 1e-12
                        || dist2(px, py, *x2, *y2) <= r * r + 1e-12
                } else if py >= miny && py <= maxy && px >= minx - 1e-12 && px <= maxx + 1e-12 {
                    true
                } else {
                    dist2(px, py, *x1, *y1) <= r * r + 1e-12
                        || dist2(px, py, *x2, *y2) <= r * r + 1e-12
                }
            }
            Shape::Polygon { points } => point_in_polygon(px, py, points),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Prim {
    /// Stable id derived from feature order + sr instance, reused across recompute.
    pub id: String,
    pub shape: Shape,
    pub polarity: Polarity,
    /// Gerber operation order (drives overlay).
    pub order: usize,
    /// Source feature identity for provenance.
    pub feature_ref: String,
    pub material_id: String,
    pub layer_key: String,
}

pub fn dist2(ax: f64, ay: f64, bx: f64, by: f64) -> f64 {
    let dx = ax - bx;
    let dy = ay - by;
    dx * dx + dy * dy
}

fn polygon_area(pts: &[(f64, f64)]) -> f64 {
    if pts.len() < 3 {
        return 0.0;
    }
    let mut s = 0.0;
    for i in 0..pts.len() {
        let (x1, y1) = pts[i];
        let (x2, y2) = pts[(i + 1) % pts.len()];
        s += x1 * y2 - x2 * y1;
    }
    s.abs() / 2.0
}

/// Ray casting; boundary counts as inside (closed set).
pub fn point_in_polygon(px: f64, py: f64, pts: &[(f64, f64)]) -> bool {
    let n = pts.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = pts[i];
        let (xj, yj) = pts[j];
        if ((yi > py) != (yj > py)) && (px < (xj - xi) * (py - yi) / (yj - yi) + xi) {
            inside = !inside;
        }
        j = i;
    }
    if inside {
        return true;
    }
    // boundary check (axis-aligned edges or segments)
    for i in 0..n {
        let (x1, y1) = pts[i];
        let (x2, y2) = pts[(i + 1) % n];
        if dist2(px, py, x1, y1) + dist2(px, py, x2, y2) - dist2(x1, y1, x2, y2) <= 1e-9 {
            let d = dist2(px, py, x1, y1) + dist2(px, py, x2, y2) - dist2(x1, y1, x2, y2);
            if d.abs() < 1e-9 {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_boundary_counts_as_contact() {
        let r = Shape::Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
        assert!(r.contains(0.0, 0.5));
        assert!(r.contains(1.0, 0.0));
        assert!(!r.contains(1.0 + 1e-3, 0.5));
    }

    #[test]
    fn circle_boundary_counts_as_contact() {
        let c = Shape::Circle { cx: 0.0, cy: 0.0, r: 1.0 };
        assert!(c.contains(1.0, 0.0));
        assert!(!c.contains(1.0 + 1e-3, 0.0));
    }
}
