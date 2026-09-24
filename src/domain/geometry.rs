//! Planar geometry on integer nanometres.
//!
//! `gap` is the distance between solid regions: <= 0 means overlap/touch.
//! `strictly_inside` uses a 1 nm margin so that boundary tangency is NOT
//! treated as being erased by a clear shape.

use crate::domain::model::Shape;

fn point_seg(px: i64, py: i64, ax: i64, ay: i64, bx: i64, by: i64) -> (i64, i64) {
    let vx = bx - ax;
    let vy = by - ay;
    let wx = px - ax;
    let wy = py - ay;
    let len2 = (vx as i128 * vx as i128 + vy as i128 * vy as i128) as i128;
    let t = if len2 == 0 {
        0i128
    } else {
        ((wx as i128 * vx as i128 + wy as i128 * vy as i128) * 1024 / len2)
            .clamp(0, 1024)
    };
    let qx = ax as i128 + (vx as i128 * t / 1024);
    let qy = ay as i128 + (vy as i128 * t / 1024);
    ((px as i128 - qx) as i64, (py as i128 - qy) as i64)
}

fn dist_dxdy(dx: i64, dy: i64) -> i64 {
    let d2 = dx as f64 * dx as f64 + dy as f64 * dy as f64;
    d2.sqrt().round() as i64
}

/// Minimum distance between two solid shapes (negative when overlapping).
pub fn gap(a: &Shape, b: &Shape) -> i64 {
    use Shape::*;
    match (a, b) {
        (Circle { cx: ax, cy: ay, r: ra }, Circle { cx: bx, cy: by, r: rb }) => {
            dist_dxdy(ax - bx, ay - by) - ra - rb
        }
        (Circle { cx, cy, r }, Rect { x0, y0, x1, y1 })
        | (Rect { x0, y0, x1, y1 }, Circle { cx, cy, r }) => {
            let dx = [x0 - cx, 0, cx - x1].into_iter().max().unwrap();
            let dy = [y0 - cy, 0, cy - y1].into_iter().max().unwrap();
            let cr = if dx == 0 && dy == 0 {
                // circle centre inside rect: deepest penetration
                let to_left = cx - x0;
                let to_right = x1 - cx;
                let to_bot = cy - y0;
                let to_top = y1 - cy;
                -(to_left.min(to_right).min(to_bot).min(to_top))
            } else {
                dist_dxdy(*dx, *dy)
            };
            cr - r
        }
        (Rect { x0: ax0, y0: ay0, x1: ax1, y1: ay1 },
         Rect { x0: bx0, y0: by0, x1: bx1, y1: by1 }) => {
            let dx = [bx0 - ax1, 0, ax0 - bx1].into_iter().max().unwrap();
            let dy = [by0 - ay1, 0, ay0 - by1].into_iter().max().unwrap();
            if dx != 0 || dy != 0 {
                dist_dxdy(*dx, *dy)
            } else {
                let ix = ax1.min(bx1) - ax0.max(bx0);
                let iy = ay1.min(by1) - ay0.max(by0);
                -(ix.min(iy))
            }
        }
        (Trace { x1: a1x, y1: a1y, x2: a2x, y2: a2y, hw: ahw },
         Trace { x1: b1x, y1: b1y, x2: b2x, y2: b2y, hw: bhw }) => {
            let pts = [
                (*a1x, *a1y), (*a2x, *a2y), (*b1x, *b1y), (*b2x, *b2y),
            ];
            let mut best = i64::MAX;
            for (px, py) in [pts[0], pts[1]] {
                let (dx, dy) = point_seg(px, py, *b1x, *b1y, *b2x, *b2y);
                best = best.min(dist_dxdy(dx, dy) - bhw - ahw);
            }
            for (px, py) in [pts[2], pts[3]] {
                let (dx, dy) = point_seg(px, py, *a1x, *a1y, *a2x, *a2y);
                best = best.min(dist_dxdy(dx, dy) - ahw - bhw);
            }
            best
        }
        (Trace { x1, y1, x2, y2, hw }, Circle { cx, cy, r })
        | (Circle { cx, cy, r }, Trace { x1, y1, x2, y2, hw }) => {
            let (dx, dy) = point_seg(*cx, *cy, *x1, *y1, *x2, *y2);
            dist_dxdy(dx, dy) - r - hw
        }
        (Trace { x1, y1, x2, y2, hw }, Rect { x0, y0, x1: rx1, y1: ry1 })
        | (Rect { x0, y0, x1: rx1, y1: ry1 }, Trace { x1, y1, x2, y2, hw }) => {
            let corners = [(*x0, *y0), (*rx1, *y0), (*rx1, *ry1), (*x0, *ry1)];
            let mut best = i64::MAX;
            for (px, py) in corners {
                let (dx, dy) = point_seg(px, py, *x1, *y1, *x2, *y2);
                best = best.min(dist_dxdy(dx, dy) - hw);
            }
            // trace endpoint inside rect gives negative gap
            for (px, py) in [(*x1, *y1), (*x2, *y2)] {
                if px > *x0 && px < *rx1 && py > *y0 && py < *ry1 {
                    best = best.min(-hw);
                }
            }
            best
        }
    }
}

fn inside_circle(px: i64, py: i64, cx: i64, cy: i64, r: i64, margin: i64) -> bool {
    dist_dxdy(px - cx, py - cy) + margin <= r
}

fn inside_rect(px: i64, py: i64, x0: i64, y0: i64, x1: i64, y1: i64, margin: i64) -> bool {
    px >= x0 + margin && px <= x1 - margin && py >= y0 + margin && py <= y1 - margin
}

/// Strict interior test: a point on a shape boundary (e.g. a tangency witness)
/// is not considered covered by the clear shape.
pub fn point_strictly_inside_shape(px: i64, py: i64, s: &Shape, margin: i64) -> bool {
    match *s {
        Shape::Circle { cx, cy, r } => inside_circle(px, py, cx, cy, r, margin),
        Shape::Rect { x0, y0, x1, y1 } => inside_rect(px, py, x0, y0, x1, y1, margin),
        Shape::Trace { x1, y1, x2, y2, hw } => {
            let (dx, dy) = point_seg(px, py, x1, y1, x2, y2);
            dist_dxdy(dx, dy) + margin <= hw && {
                // exclude the capsule end caps beyond the segment endpoints?
                // For margin>0 witness points this stays simple/conservative.
                true
            }
        }
    }
}

/// Candidate witness points (boundary/centre samples) used to check whether a
/// later clear erases a dark/dark contact region.
pub fn witness_points(s: &Shape) -> Vec<(i64, i64)> {
    match *s {
        Shape::Circle { cx, cy, r } => {
            let mut v = vec![(cx, cy)];
            if r > 0 {
                for k in 0..12i32 {
                    let a = (k as f64) * std::f64::consts::FRAC_PI_6;
                    let rr = (r as f64 * 0.5).round() as i64;
                    v.push((
                        cx + (a.cos() * rr as f64).round() as i64,
                        cy + (a.sin() * rr as f64).round() as i64,
                    ));
                }
            }
            v
        }
        Shape::Rect { x0, y0, x1, y1 } => {
            let (mx, my) = ((x0 + x1) / 2, (y0 + y1) / 2);
            vec![
                (mx, my), (x0, my), (x1, my), (mx, y0), (mx, y1),
                (mx, (y0 + my) / 2), (mx, (my + y1) / 2),
                ((x0 + mx) / 2, my), ((mx + x1) / 2, my),
            ]
        }
        Shape::Trace { x1, y1, x2, y2, hw } => {
            let mx = (x1 + x2) / 2;
            let my = (y1 + y2) / 2;
            let mut v = vec![(x1, y1), (x2, y2), (mx, my)];
            if hw > 0 {
                let (dx, dy) = point_seg(mx, my, x1, y1, x2, y2);
                let _ = (dx, dy);
                // sample perpendicular offsets
                let (vx, vy) = (x2 - x1, y2 - y1);
                let len = dist_dxdy(vx, vy).max(1);
                let (nx, ny) = ((-vy as i128 / len as i128) as i64, (vx as i128 / len as i128) as i64);
                for (sx, sy) in [(x1, y1), (mx, my), (x2, y2)] {
                    v.push((sx + nx * hw, sy + ny * hw));
                    v.push((sx - nx * hw, sy - ny * hw));
                }
            }
            v
        }
    }
}
