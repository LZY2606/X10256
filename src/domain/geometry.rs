//! Deterministic 2D geometry over convex polygons.
//!
//! All derived coordinates are in millimetres. Original representations stay
//! on the imported materials; conversion happens only on the derived layer
//! (see parser modules). Circles are rasterized into convex polygons here.

use serde::{Deserialize, Serialize};

pub const EPS: f64 = 1e-7;
pub const AREA_EPS: f64 = 1e-9;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Pt {
    pub x: f64,
    pub y: f64,
}

impl Pt {
    pub fn new(x: f64, y: f64) -> Self {
        Pt { x, y }
    }
    pub fn add(self, o: Pt) -> Pt {
        Pt::new(self.x + o.x, self.y + o.y)
    }
    pub fn sub(self, o: Pt) -> Pt {
        Pt::new(self.x - o.x, self.y - o.y)
    }
    pub fn mul(self, k: f64) -> Pt {
        Pt::new(self.x * k, self.y * k)
    }
}

pub fn cross(a: Pt, b: Pt) -> f64 {
    a.x * b.y - a.y * b.x
}

pub fn dist2(a: Pt, b: Pt) -> f64 {
    let d = a.sub(b);
    d.x * d.x + d.y * d.y
}

pub fn near(a: f64, b: f64, eps: f64) -> bool {
    (a - b).abs() <= eps
}

/// Normalized shapes stored on derived copper features.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "shape", rename_all = "lowercase")]
pub enum Shape {
    Rect { x: f64, y: f64, w: f64, h: f64 },
    Circle { cx: f64, cy: f64, r: f64 },
    Polygon { pts: Vec<Pt> },
}

impl Shape {
    pub fn as_polygon(&self, circle_segments: u32) -> Vec<Pt> {
        match self {
            Shape::Rect { x, y, w, h } => {
                vec![
                    Pt::new(*x, *y),
                    Pt::new(x + w, *y),
                    Pt::new(x + w, y + h),
                    Pt::new(*x, y + h),
                ]
            }
            Shape::Polygon { pts } => pts.clone(),
            Shape::Circle { cx, cy, r } => {
                let n = circle_segments.max(3);
                (0..n)
                    .map(|i| {
                        let a = std::f64::consts::TAU * (i as f64) / (n as f64);
                        Pt::new(cx + r * a.cos(), cy + r * a.sin())
                    })
                    .collect()
            }
        }
    }
    pub fn center(&self) -> Pt {
        match self {
            Shape::Rect { x, y, w, h } => Pt::new(x + w / 2.0, y + h / 2.0),
            Shape::Circle { cx, cy, .. } => Pt::new(*cx, *cy),
            Shape::Polygon { pts } => {
                let mut sx = 0.0;
                let mut sy = 0.0;
                for p in pts {
                    sx += p.x;
                    sy += p.y;
                }
                let n = pts.len().max(1) as f64;
                Pt::new(sx / n, sy / n)
            }
        }
    }
}

/// Strict convex orientation: positive CCW, negative CW, zero degenerate.
pub fn polygon_signed_area(pts: &[Pt]) -> f64 {
    let mut a = 0.0;
    for i in 0..pts.len() {
        let p = pts[i];
        let q = pts[(i + 1) % pts.len()];
        a += p.x * q.y - q.x * p.y;
    }
    a / 2.0
}

pub fn ensure_ccw(pts: Vec<Pt>) -> Vec<Pt> {
    let mut pts = pts;
    if polygon_signed_area(&pts) < 0.0 {
        pts.reverse();
    }
    pts
}

pub fn is_convex(pts: &[Pt]) -> bool {
    let n = pts.len();
    if n < 3 {
        return false;
    }
    let mut sign = 0.0;
    for i in 0..n {
        let c = cross(
            pts[(i + 1) % n].sub(pts[i]),
            pts[(i + 2) % n].sub(pts[(i + 1) % n]),
        );
        if c.abs() > EPS {
            if sign == 0.0 {
                sign = c;
            } else if sign * c < 0.0 {
                return false;
            }
        }
    }
    true
}

fn line_halfplane(inside: &[Pt], a: Pt, b: Pt, eps: f64) -> Vec<Pt> {
    let edge = b.sub(a);
    let mut out = Vec::new();
    if inside.is_empty() {
        return out;
    }
    for i in 0..inside.len() {
        let cur = inside[i];
        let nxt = inside[(i + 1) % inside.len()];
        let dc = cross(edge, cur.sub(a));
        let dn = cross(edge, nxt.sub(a));
        let cin = dc >= -eps;
        let nin = dn >= -eps;
        if cin {
            out.push(cur);
        }
        if cin != nin {
            let t = dc / (dc - dn);
            out.push(Pt::new(
                cur.x + (nxt.x - cur.x) * t,
                cur.y + (nxt.y - cur.y) * t,
            ));
        }
    }
    dedup_ring(out)
}

fn dedup_ring(mut pts: Vec<Pt>) -> Vec<Pt> {
    if pts.len() < 2 {
        return pts;
    }
    let mut out: Vec<Pt> = Vec::new();
    for p in pts.drain(..) {
        if let Some(last) = out.last() {
            if dist2(*last, p) <= EPS * EPS {
                continue;
            }
        }
        out.push(p);
    }
    if out.len() > 1 {
        let first = out[0];
        if let Some(last) = out.last() {
            if dist2(*last, first) <= EPS * EPS {
                out.pop();
            }
        }
    }
    out
}

/// Intersection of two convex polygons (Sutherland-Hodgman), CCW required.
pub fn convex_intersect(a: &[Pt], b: &[Pt]) -> Vec<Pt> {
    if a.len() < 3 || b.len() < 3 {
        return Vec::new();
    }
    let mut clip = a.to_vec();
    for i in 0..b.len() {
        if clip.is_empty() {
            break;
        }
        clip = line_halfplane(&clip, b[i], b[(i + 1) % b.len()], EPS);
    }
    if clip.len() >= 3 && polygon_signed_area(&clip).abs() > AREA_EPS {
        clip
    } else {
        Vec::new()
    }
}

/// Intersection allowing boundary-only tangency (area may be zero). Used for
/// the boundary-tangent case distinction.
pub fn convex_touch(a: &[Pt], b: &[Pt]) -> Vec<Pt> {
    if a.len() < 3 || b.len() < 3 {
        return Vec::new();
    }
    let mut clip = a.to_vec();
    for i in 0..b.len() {
        if clip.is_empty() {
            break;
        }
        clip = line_halfplane(&clip, b[i], b[(i + 1) % b.len()], -EPS);
    }
    dedup_ring(clip)
}

fn point_in_convex(p: Pt, poly: &[Pt], eps: f64) -> bool {
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        if cross(b.sub(a), p.sub(a)) < -eps {
            return false;
        }
    }
    true
}

fn point_seg_dist2(p: Pt, a: Pt, b: Pt) -> f64 {
    let ab = b.sub(a);
    let len2 = ab.x * ab.x + ab.y * ab.y;
    let t = if len2 <= EPS * EPS {
        0.0
    } else {
        ((p.x - a.x) * ab.x + (p.y - a.y) * ab.y) / len2
    };
    let t = t.clamp(0.0, 1.0);
    let q = Pt::new(a.x + ab.x * t, a.y + ab.y * t);
    dist2(p, q)
}

pub fn point_in_or_on(p: Pt, poly: &[Pt]) -> bool {
    point_in_convex(p, poly, -EPS)
}

fn poly_edges(poly: &[Pt]) -> impl Iterator<Item = (Pt, Pt)> + '_ {
    (0..poly.len()).map(move |i| (poly[i], poly[(i + 1) % poly.len()]))
}

fn segment_distance2(a: Pt, b: Pt, c: Pt, d: Pt) -> Option<f64> {
    fn ori(p: Pt, q: Pt, r: Pt) -> f64 {
        cross(q.sub(p), r.sub(p))
    }
    fn on(p: Pt, q: Pt, r: Pt) -> bool {
        q.x <= p.x.max(r.x) + EPS
            && q.x + EPS >= p.x.min(r.x)
            && q.y <= p.y.max(r.y) + EPS
            && q.y + EPS >= p.y.min(r.y)
    }
    let o1 = ori(a, b, c);
    let o2 = ori(a, b, d);
    let o3 = ori(c, d, a);
    let o4 = ori(c, d, b);
    if (o1 * o2 < -EPS && o3 * o4 < -EPS)
        || (o1.abs() <= EPS && on(a, c, b))
        || (o2.abs() <= EPS && on(a, d, b))
        || (o3.abs() <= EPS && on(c, a, d))
        || (o4.abs() <= EPS && on(c, b, d))
    {
        return Some(0.0);
    }
    Some(
        [
            point_seg_dist2(a, c, d),
            point_seg_dist2(b, c, d),
            point_seg_dist2(c, a, b),
            point_seg_dist2(d, a, b),
        ]
        .into_iter()
        .fold(f64::INFINITY, f64::min),
    )
}

/// Minimum squared distance between two convex polygon boundaries. Returns 0
/// when boundaries intersect.
pub fn boundary_distance2(a: &[Pt], b: &[Pt]) -> f64 {
    let mut best = f64::INFINITY;
    for (p, q) in poly_edges(a) {
        for (r, s) in poly_edges(b) {
            if let Some(d2) = segment_distance2(p, q, r, s) {
                best = best.min(d2);
            }
        }
    }
    best
}

/// Length of the shared (coincident, opposite/equal direction) boundary
/// between two convex polygons, returning the longest collinear overlap.
pub fn shared_boundary_length(a: &[Pt], b: &[Pt]) -> f64 {
    let mut best = 0.0;
    for (p, q) in poly_edges(a) {
        let e1 = q.sub(p);
        let n1 = (e1.y, -e1.x);
        for (r, s) in poly_edges(b) {
            let e2 = s.sub(r);
            // collinear: r lies on line pq, and direction parallel
            if cross(e1, e2).abs() > EPS {
                continue;
            }
            if (n1.0 * (r.x - p.x) + n1.1 * (r.y - p.y)).abs() > EPS {
                continue;
            }
            let len = e1.x.hypot(e1.y);
            if len <= EPS {
                continue;
            }
            let ux = e1.x / len;
            let uy = e1.y / len;
            let ta = 0.0;
            let tb = len;
            let tc = (r.x - p.x) * ux + (r.y - p.y) * uy;
            let td = (s.x - p.x) * ux + (s.y - p.y) * uy;
            let lo = ta.max(tc.min(td));
            let hi = tb.min(tc.max(td));
            if hi > lo {
                best = best.max(hi - lo);
            }
        }
    }
    best
}

/// All polygon vertices that lie on both polygon boundaries.
pub fn shared_boundary_points(a: &[Pt], b: &[Pt]) -> Vec<Pt> {
    let mut out = Vec::new();
    for p in a.iter() {
        if point_on_poly_boundary(*p, b) && point_on_poly_boundary(*p, a) {
            if !out.iter().any(|q: &Pt| dist2(*q, *p) <= EPS * EPS) {
                out.push(*p);
            }
        }
    }
    for p in b.iter() {
        if point_on_poly_boundary(*p, a) && !out.iter().any(|q: &Pt| dist2(*q, *p) <= EPS * EPS) {
            out.push(*p);
        }
    }
    out
}

fn point_on_poly_boundary(p: Pt, poly: &[Pt]) -> bool {
    for (a, b) in poly_edges(poly) {
        if cross(b.sub(a), p.sub(a)).abs() <= EPS && point_seg_dist2(p, a, b) <= EPS * EPS {
            return true;
        }
    }
    false
}

/// Bounding box used for view transforms.
pub fn bounds_of(polys: &[Vec<Pt>]) -> Option<(f64, f64, f64, f64)> {
    let mut minx = f64::INFINITY;
    let mut miny = f64::INFINITY;
    let mut maxx = f64::NEG_INFINITY;
    let mut maxy = f64::NEG_INFINITY;
    for poly in polys {
        for p in poly {
            minx = minx.min(p.x);
            miny = miny.min(p.y);
            maxx = maxx.max(p.x);
            maxy = maxy.max(p.y);
        }
    }
    if minx.is_infinite() {
        None
    } else {
        Some((minx, miny, maxx, maxy))
    }
}
