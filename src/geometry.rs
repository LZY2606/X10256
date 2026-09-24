//! 几何接触引擎：距离、最近点、容差分级接触、负极性切断判定。

use crate::model::{Contact, ContactKind, Polarity, Prim, Shape, Vec2};

pub fn sub(a: Vec2, b: Vec2) -> Vec2 {
    Vec2 { x: a.x - b.x, y: a.y - b.y }
}
pub fn add(a: Vec2, b: Vec2) -> Vec2 {
    Vec2 { x: a.x + b.x, y: a.y + b.y }
}
pub fn scale(a: Vec2, s: f64) -> Vec2 {
    Vec2 { x: a.x * s, y: a.y * s }
}
pub fn len(a: Vec2) -> f64 {
    (a.x * a.x + a.y * a.y).sqrt()
}
pub fn dist(a: Vec2, b: Vec2) -> f64 {
    len(sub(a, b))
}

fn closest_point_segment(p: Vec2, a: Vec2, b: Vec2) -> Vec2 {
    let ab = sub(b, a);
    let denom = ab.x * ab.x + ab.y * ab.y;
    if denom < 1e-18 {
        return a;
    }
    let t = ((p.x - a.x) * ab.x + (p.y - a.y) * ab.y) / denom;
    let t = t.clamp(0.0, 1.0);
    add(a, scale(ab, t))
}

fn point_in_rect(p: Vec2, c: Vec2, hw: f64, hh: f64) -> bool {
    p.x >= c.x - hw && p.x <= c.x + hw && p.y >= c.y - hh && p.y <= c.y + hh
}

fn rect_corners(c: Vec2, hw: f64, hh: f64) -> [(Vec2, Vec2); 4] {
    let (l, r, t, b) = (c.x - hw, c.x + hw, c.y + hh, c.y - hh);
    [
        (Vec2 { x: l, y: b }, Vec2 { x: r, y: b }),
        (Vec2 { x: r, y: b }, Vec2 { x: r, y: t }),
        (Vec2 { x: r, y: t }, Vec2 { x: l, y: t }),
        (Vec2 { x: l, y: t }, Vec2 { x: l, y: b }),
    ]
}

/// 返回 (gap, 最接近点对)；gap<=0 表示相交（近似）。
pub fn closest(a: &Shape, b: &Shape) -> (f64, Vec2, Vec2) {
    match (a, b) {
        (Shape::Circle { c: c1, r: r1 }, Shape::Circle { c: c2, r: r2 }) => {
            let d = dist(*c1, *c2);
            if d < 1e-15 {
                let p = Vec2 { x: c1.x + *r1, y: c1.y };
                return (-*r1.min(*r2), p, p);
            }
            let p1 = add(*c1, scale(sub(*c2, *c1), r1 / d));
            let p2 = add(*c2, scale(sub(*c1, *c2), r2 / d));
            (d - r1 - r2, p1, p2)
        }
        (Shape::Circle { c, r }, Shape::Rect { c: rc, hw, hh })
        | (Shape::Rect { c: rc, hw, hh }, Shape::Circle { c, r }) => {
            if point_in_rect(*c, *rc, *hw, *hh) {
                return (-*r, *c, *c);
            }
            let mut best = f64::MAX;
            let mut bp = *c;
            for (s, e) in rect_corners(*rc, *hw, *hh) {
                let q = closest_point_segment(*c, s, e);
                let d = dist(*c, q);
                if d < best {
                    best = d;
                    bp = q;
                }
            }
            (best - *r, *c, bp)
        }
        (Shape::Circle { c, r }, Shape::Capsule { a, b, r: r2 })
        | (Shape::Capsule { a, b, r: r2 }, Shape::Circle { c, r }) => {
            let q = closest_point_segment(*c, *a, *b);
            let d = dist(*c, q);
            let p = if d < 1e-15 {
                Vec2 { x: c.x, y: c.y + *r }
            } else {
                add(*c, scale(sub(q, *c), *r / d))
            };
            (d - r - r2, p, q)
        }
        (Shape::Capsule { a: a1, b: b1, r: r1 }, Shape::Capsule { a: a2, b: b2, r: r2 }) => {
            // 采样两条线段找最接近参数对（线段-线段精确解的稳定替代）
            let n = 24;
            let mut best = f64::MAX;
            let mut ps = (*a1, *a2);
            for i in 0..=n {
                let t = i as f64 / n as f64;
                let p = add(*a1, scale(sub(*b1, *a1), t));
                let q = closest_point_segment(p, *a2, *b2);
                let d = dist(p, q);
                if d < best {
                    best = d;
                    ps = (p, q);
                }
            }
            for j in 0..=n {
                let t = j as f64 / n as f64;
                let q = add(*a2, scale(sub(*b2, *a2), t));
                let p = closest_point_segment(q, *a1, *b1);
                let d = dist(p, q);
                if d < best {
                    best = d;
                    ps = (p, q);
                }
            }
            (best - r1 - r2, ps.0, ps.1)
        }
        (Shape::Rect { c: c1, hw: w1, hh: h1 }, Shape::Rect { c: c2, hw: w2, hh: h2 }) => {
            let dx = (c1.x - c2.x).abs() - (w1 + w2);
            let dy = (c1.y - c2.y).abs() - (h1 + h2);
            if dx <= 0.0 && dy <= 0.0 {
                return (-dx.max(dy), *c1, *c2);
            }
            if dx > 0.0 && dy > 0.0 {
                let p1 = Vec2 { x: if c1.x < c2.x { c1.x + w1 } else { c1.x - w1 }, y: c1.y + h1 * dy.signum() };
                let p2 = Vec2 { x: if c1.x < c2.x { c2.x - w2 } else { c2.x + w2 }, y: c2.y };
                return ((dx * dx + dy * dy).sqrt(), p1, p2);
            }
            (dx.max(dy), *c1, *c2)
        }
        (Shape::Rect { c: rc, hw, hh }, Shape::Capsule { a, b, r })
        | (Shape::Capsule { a, b, r }, Shape::Rect { c: rc, hw, hh }) => {
            let mut best = f64::MAX;
            let mut ps = (*a, *a);
            for (s, e) in rect_corners(*rc, *hw, *hh) {
                for j in 0..=12 {
                    let t = j as f64 / 12.0;
                    let p = add(*a, scale(sub(*b, *a), t));
                    let q = closest_point_segment(p, s, e);
                    let d = dist(p, q);
                    if d < best {
                        best = d;
                        ps = (p, q);
                    }
                }
            }
            let mid = add(*a, scale(sub(*b, *a), 0.5));
            if point_in_rect(mid, *rc, *hw, *hh) {
                return (-*r, mid, mid);
            }
            (best - *r, ps.0, ps.1)
        }
    }
}

pub fn midpoint(p: Vec2, q: Vec2) -> Vec2 {
    scale(add(p, q), 0.5)
}

fn point_covered_by(p: Vec2, s: &Shape) -> bool {
    match s {
        Shape::Circle { c, r } => dist(p, *c) <= *r + 1e-12,
        Shape::Rect { c, hw, hh } => point_in_rect(p, *c, *hw, *hh),
        Shape::Capsule { a, b, r } => dist(closest_point_segment(p, *a, *b), p) <= *r + 1e-12,
    }
}

/// 在同一层内构造接触表。
/// 接触容差 τ 是版本参数：gap<=0 确定；0<gap<=τ 仅为邻近候选。
/// 负极性图元按操作顺序叠加：接触点若被后续（或任何）负图元覆盖则切断。
pub fn build_contacts(prims: &[Prim], tol: f64, budget: u64) -> Result<Vec<Contact>, (u64, u64)> {
    let mut out: Vec<Contact> = Vec::new();
    let mut used: u64 = 0;
    // 按层分组（同角色才可能接触；孔在 compute 层跨层处理）
    let layers: Vec<_> = {
        let mut v: Vec<&Prim> = prims.iter().collect();
        v.sort_by(|a, b| a.layer.cmp(&b.layer).then(a.src_line.cmp(&b.src_line)));
        v
    };
    let mut i = 0;
    while i < layers.len() {
        let layer = layers[i].layer;
        let mut j = i;
        while j < layers.len() && layers[j].layer == layer {
            j += 1;
        }
        let group = &layers[i..j];
        for x in 0..group.len() {
            for y in (x + 1)..group.len() {
                used += 1;
                if used > budget {
                    return Err((used, budget));
                }
                let pa = group[x];
                let pb = group[y];
                let (gap, qa, qb) = closest(&pa.shape, &pb.shape);
                let point = midpoint(qa, qb);
                let kind = if gap <= 0.0 {
                    ContactKind::Definite
                } else if gap <= tol + 1e-12 {
                    ContactKind::Marginal
                } else {
                    continue;
                };
                // 正负极性按操作顺序叠加：
                // 接触点被负极性图元覆盖即切断（负图元顺序在二者之后的优先记录）。
                let mut cut_by: Option<String> = None;
                for cutter in group.iter().filter(|p| p.polarity == Polarity::Subtract) {
                    if cutter.id == pa.id || cutter.id == pb.id {
                        continue;
                    }
                    if point_covered_by(point, &cutter.shape) {
                        cut_by = Some(cutter.id.clone());
                    }
                }
                out.push(Contact {
                    a: pa.id.clone(),
                    b: pb.id.clone(),
                    gap_mm: gap,
                    point,
                    kind,
                    cut_by,
                });
            }
        }
        i = j;
    }
    // 稳定顺序：等价输入稳定
    out.sort_by(|x, y| x.a.cmp(&y.a).then(x.b.cmp(&y.b)));
    Ok(out)
}

/// 并查集
pub struct Dsu {
    parent: Vec<usize>,
}

impl Dsu {
    pub fn new(n: usize) -> Self {
        Dsu { parent: (0..n).collect() }
    }
    pub fn find(&mut self, x: usize) -> usize {
        if self.parent[x] != x {
            self.parent[x] = self.find(self.parent[x]);
        }
        self.parent[x]
    }
    pub fn union(&mut self, a: usize, b: usize) -> bool {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return false;
        }
        // 稳定：小 index 作为根
        if ra < rb {
            self.parent[rb] = ra;
        } else {
            self.parent[ra] = rb;
        }
        true
    }
}
