//! Minimal 2D primitives on the nanometer basis.
//!
//! Shapes are axis-aligned where the source is (flash rectangles/circles);
//! segments become capsules. Distance functions distinguish true contact
//! (zero gap) from mere proximity (positive gap) so proximity is never
//! reported as conduction without a netlist decision.

use crate::domain::Point;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "shape")]
pub enum Shape {
    Circle { cx: i64, cy: i64, r: i64 },
    Rect { x0: i64, y0: i64, x1: i64, y1: i64 },
    Capsule { ax: i64, ay: i64, bx: i64, by: i64, r: i64 },
}

impl Shape {
    pub fn contains_cell_center(&self, x: i64, y: i64) -> bool {
        match self {
            Shape::Circle { cx, cy, r } => {
                let dx = (x - cx) as i128;
                let dy = (y - cy) as i128;
                dx * dx + dy * dy <= (*r as i128) * (*r as i128)
            }
            Shape::Rect { x0, y0, x1, y1 } => x >= *x0 && x <= *x1 && y >= *y0 && y <= *y1,
            Shape::Capsule { ax, ay, bx, by, r } => {
                dist_point_segment(x, y, *ax, *ay, *bx, *by) <= (*r as i128) * (*r as i128)
            }
        }
    }

    pub fn bbox(&self) -> (i64, i64, i64, i64) {
        match self {
            Shape::Circle { cx, cy, r } => (cx - r, cy - r, cx + r, cy + r),
            Shape::Rect { x0, y0, x1, y1 } => (*x0, *y0, *x1, *y1),
            Shape::Capsule { ax, ay, bx, by, r } => {
                (ax.min(*bx) - r, ay.min(*by) - r, ax.max(*bx) + r, ay.max(*by) + r)
            }
        }
    }

    /// Minimum gap between two shapes in nanometers; 0 means geometric contact.
    pub fn gap_to(&self, other: &Shape) -> i64 {
        use Shape::*;
        match (self, other) {
            (Circle { cx: cax, cy: cay, r: ra }, Circle { cx: cbx, cy: cby, r: rb }) => {
                let d2 = dist2(*cax, *cay, *cbx, *cby);
                let rr = (ra + rb) as f64;
                let d = (d2 as f64).sqrt();
                (d - rr).max(0.0).round() as i64
            }
            (Rect { x0, y0, x1, y1 }, Rect { x0: o0, y0: q0, x1: o1, y1: q1 }) => {
                let dx = interval_gap(*x0, *x1, *o0, *o1);
                let dy = interval_gap(*y0, *y1, *q0, *q1);
                if dx == 0 || dy == 0 {
                    dx.max(dy)
                } else {
                    ((dx as f64).hypot(dy as f64)).round() as i64
                }
            }
            (Circle { cx, cy, r }, Rect { x0, y0, x1, y1 })
            | (Rect { x0, y0, x1, y1 }, Circle { cx, cy, r }) => {
                let nx = cx.clamp(x0, x1);
                let ny = cy.clamp(y0, y1);
                let d2 = dist2(*cx, *cy, nx, ny);
                let d = (d2 as f64).sqrt();
                (d - *r as f64).max(0.0).round() as i64
            }
            (Capsule { ax, ay, bx, by, r }, other) | (other, Capsule { ax, ay, bx, by, r }) => {
                let d = match other {
                    Shape::Circle { cx, cy, r: ro } => {
                        let seg = segment_dist_sqrt(*cx, *cy, *ax, *ay, *bx, *by);
                        seg - (*r as f64 + *ro as f64)
                    }
                    Shape::Rect { x0, y0, x1, y1 } => {
                        let mut best = f64::INFINITY;
                        for (px, py) in [
                            (*ax, *ay),
                            (*bx, *by),
                            (ax.clamp(x0, x1), *ay.clamp(y0, y1)),
                            (bx.clamp(x0, x1), *by.clamp(y0, y1)),
                        ] {
                            let d = dist_point_segment(px, py, *x0, *y0, *x0, *y1)
                                .min(dist_point_segment(px, py, *x0, *y1, *x1, *y1))
                                .min(dist_point_segment(px, py, *x1, *y1, *x1, *y0))
                                .min(dist_point_segment(px, py, *x1, *y0, *x0, *y0));
                            best = best.min(d);
                        }
                        (best as f64).sqrt() - *r as f64
                    }
                    Shape::Capsule { ax: oax, ay: oay, bx: obx, by: oby, r: ro } => {
                        let mut best = f64::INFINITY;
                        for (px, py) in [(*oax, *oay), (*obx, *oby)] {
                            best = best.min(segment_dist_sqrt(px, py, *ax, *ay, *bx, *by));
                        }
                        for (px, py) in [(*ax, *ay), (*bx, *by)] {
                            best = best.min(segment_dist_sqrt(px, py, *oax, *oay, *obx, *oby));
                        }
                        best - (*r as f64 + *ro as f64)
                    }
                };
                d.max(0.0).round() as i64
            }
        }
    }

    pub fn approx_points(&self) -> Vec<Point> {
        match self {
            Shape::Circle { cx, cy, r: _ } => vec![Point { x: *cx, y: *cy }],
            Shape::Rect { x0, y0, x1, y1 } => vec![
                Point { x: *x0, y: *y0 },
                Point { x: *x1, y: *y0 },
                Point { x: *x1, y: *y1 },
                Point { x: *x0, y: *y1 },
                Point { x: (x0 + x1) / 2, y: (y0 + y1) / 2 },
            ],
            Shape::Capsule { ax, ay, bx, by, r: _ } => {
                vec![Point { x: *ax, y: *ay }, Point { x: *bx, y: *by }]
            }
        }
    }
}

fn dist2(ax: i64, ay: i64, bx: i64, by: i64) -> i128 {
    let dx = (ax - bx) as i128;
    let dy = (ay - by) as i128;
    dx * dx + dy * dy
}

fn interval_gap(a0: i64, a1: i64, b0: i64, b1: i64) -> i64 {
    if a1 < b0 {
        b0 - a1
    } else if b1 < a0 {
        a0 - b1
    } else {
        0
    }
}

fn dist_point_segment(px: i64, py: i64, ax: i64, ay: i64, bx: i64, by: i64) -> i128 {
    let vx = (bx - ax) as i128;
    let vy = (by - ay) as i128;
    let wx = (px - ax) as i128;
    let wy = (py - ay) as i128;
    let l2 = vx * vx + vy * vy;
    let t = if l2 == 0 { 0 } else { ((wx * vx + wy * vy) * 2 / l2).clamp(0, 2) };
    let proj_x = ax as i128 + vx * t as i128 / 2;
    let proj_y = ay as i128 + vy * t as i128 / 2;
    let dx = px as i128 - proj_x;
    let dy = py as i128 - proj_y;
    dx * dx + dy * dy
}

fn segment_dist_sqrt(px: i64, py: i64, ax: i64, ay: i64, bx: i64, by: i64) -> f64 {
    (dist_point_segment(px, py, ax, ay, bx, by) as f64).sqrt()
}
