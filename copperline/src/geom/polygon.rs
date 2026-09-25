//! Minimal 2D polygon helpers (rectangle/circle primitives, Sutherland-Hodgman
//! clipping, signed area, touching/distance predicates).

pub type Polygon = Vec<(f64, f64)>;

pub fn polygon_area(poly: &Polygon) -> f64 {
    let mut a = 0.0;
    let n = poly.len();
    for i in 0..n {
        let (x1, y1) = poly[i];
        let (x2, y2) = poly[(i + 1) % n];
        a += x1 * y2 - x2 * y1;
    }
    a.abs() * 0.5
}

fn orient(poly: &Polygon, ccw: bool) -> Polygon {
    let mut p = poly.clone();
    let mut area = 0.0;
    for i in 0..p.len() {
        let (x1, y1) = p[i];
        let (x2, y2) = p[(i + 1) % p.len()];
        area += x1 * y2 - x2 * y1;
    }
    let is_ccw = area > 0.0;
    if is_ccw != ccw {
        p.reverse();
    }
    p
}

fn line_intersection(p1: (f64, f64), p2: (f64, f64), a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    let (x1, y1) = p1;
    let (x2, y2) = p2;
    let (x3, y3) = a;
    let (x4, y4) = b;
    let den = (x1 - x2) * (y3 - y4) - (y1 - y2) * (x3 - x4);
    let t = ((x1 - x3) * (y3 - y4) - (y1 - y3) * (x3 - x4)) / den;
    (x1 + t * (x2 - x1), y1 + t * (y2 - y1))
}

fn inside(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> bool {
    (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0) >= 0.0
}

/// Clip subject polygon by a convex clip polygon (assumed CCW).
pub fn clip(subject: &Polygon, clip_poly: &Polygon) -> Vec<Polygon> {
    let clip_poly = orient(clip_poly, true);
    let mut out = orient(subject, true);
    for i in 0..clip_poly.len() {
        if out.is_empty() {
            break;
        }
        let a = clip_poly[i];
        let b = clip_poly[(i + 1) % clip_poly.len()];
        let input = std::mem::take(&mut out);
        if input.is_empty() {
            break;
        }
        let mut new = Vec::new();
        let n = input.len();
        for k in 0..n {
            let cur = input[k];
            let prev = input[(k + n - 1) % n];
            let cur_in = inside(cur, a, b);
            let prev_in = inside(prev, a, b);
            if cur_in {
                if !prev_in {
                    new.push(line_intersection(prev, cur, a, b));
                }
                new.push(cur);
            } else if prev_in {
                new.push(line_intersection(prev, cur, a, b));
            }
        }
        out = new;
    }
    if out.len() < 3 || polygon_area(&out) < 1e-15 {
        Vec::new()
    } else {
        vec![out]
    }
}

fn point_in_poly(p: (f64, f64), poly: &Polygon) -> bool {
    let mut inside_flag = false;
    let n = poly.len();
    let (px, py) = p;
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = poly[i];
        let (xj, yj) = poly[j];
        let intersect = ((yi > py) != (yj > py))
            && (px < (xj - xi) * (py - yi) / (yj - yi) + xi);
        if intersect {
            inside_flag = !inside_flag;
        }
        j = i;
    }
    inside_flag
}

fn seg_dist(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 {
        0.0
    } else {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    };
    let qx = a.0 + t * dx;
    let qy = a.1 + t * dy;
    ((p.0 - qx).powi(2) + (p.1 - qy).powi(2)).sqrt()
}

fn edges_intersect(p1: (f64, f64), p2: (f64, f64), p3: (f64, f64), p4: (f64, f64)) -> bool {
    let d = |a: (f64, f64), b: (f64, f64), c: (f64, f64)| {
        (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
    };
    let d1 = d(p3, p4, p1);
    let d2 = d(p3, p4, p2);
    let d3 = d(p1, p2, p3);
    let d4 = d(p1, p2, p4);
    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
}

fn on_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> bool {
    seg_dist(p, a, b) < 1e-9
}

/// Minimum Euclidean gap between polygon boundaries (0 if they intersect or
/// touch, including boundary-only tangency).
pub fn polygons_distance(a: &Polygon, b: &Polygon) -> f64 {
    if polygons_touch(a, b) {
        return 0.0;
    }
    let mut best = f64::MAX;
    let consider = |p: (f64, f64), poly: &Polygon| {
        for e in poly.windows(2) {
            best = best.min(seg_dist(p, e[0], e[1]));
        }
        best = best.min(seg_dist(p, poly[poly.len() - 1], poly[0]));
    };
    for v in a {
        consider(*v, b);
    }
    for v in b {
        consider(*v, a);
    }
    best
}

/// True for overlap OR boundary tangency. Proximity without touching is false.
pub fn polygons_touch(a: &Polygon, b: &Polygon) -> bool {
    // Any edge intersection (including endpoint contact) counts.
    for i in 0..a.len() {
        let a1 = a[i];
        let a2 = a[(i + 1) % a.len()];
        for j in 0..b.len() {
            let b1 = b[j];
            let b2 = b[(j + 1) % b.len()];
            if edges_intersect(a1, a2, b1, b2) {
                return true;
            }
            if on_segment(a1, b1, b2) || on_segment(b1, a1, a2) {
                return true;
            }
        }
    }
    point_in_poly(a[0], b) || point_in_poly(b[0], a)
}

pub fn circle_polygon(cx: f64, cy: f64, r: f64, seg: usize) -> Polygon {
    let n = seg.max(8);
    (0..n)
        .map(|i| {
            let ang = std::f64::consts::TAU * i as f64 / n as f64;
            (cx + r * ang.cos(), cy + r * ang.sin())
        })
        .collect()
}

pub fn rect_polygon(cx: f64, cy: f64, w: f64, h: f64) -> Polygon {
    vec![
        (cx - w / 2.0, cy - h / 2.0),
        (cx + w / 2.0, cy - h / 2.0),
        (cx + w / 2.0, cy + h / 2.0),
        (cx - w / 2.0, cy + h / 2.0),
    ]
}

/// Thick segment as a rectangle polygon.
pub fn segment_polygon(a: (f64, f64), b: (f64, f64), width: f64) -> Polygon {
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-12 {
        return rect_polygon(a.0, a.1, width, width);
    }
    let nx = -dy / len * width / 2.0;
    let ny = dx / len * width / 2.0;
    vec![
        (a.0 + nx, a.1 + ny),
        (b.0 + nx, b.1 + ny),
        (b.0 - nx, b.1 - ny),
        (a.0 - nx, a.1 - ny),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tangency_and_gap() {
        let a = rect_polygon(0.0, 0.0, 2.0, 2.0);
        let touch = rect_polygon(2.0, 0.0, 2.0, 2.0);
        assert!(polygons_touch(&a, &touch));
        let gap = rect_polygon(2.01, 0.0, 2.0, 2.0);
        assert!(!polygons_touch(&a, &gap));
        assert!((polygons_distance(&a, &gap) - 0.01).abs() < 1e-9);
    }
}
