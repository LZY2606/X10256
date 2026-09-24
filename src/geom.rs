use crate::model::Nm;
use serde::{Deserialize, Serialize};

/// 规范化后的几何（派生层）：原始表示仍在 Material 中，这里统一为 nm 基准。
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub enum Shape {
    Rect { min: (Nm, Nm), max: (Nm, Nm) },
    Circle { c: (Nm, Nm), r: Nm },
}

impl Shape {
    pub fn bbox(&self) -> (Nm, Nm, Nm, Nm) {
        match *self {
            Shape::Rect { min, max } => (min.0, min.1, max.0, max.1),
            Shape::Circle { c, r } => (c.0 - r, c.1 - r, c.0 + r, c.1 + r),
        }
    }

    /// 点（含边界）是否落在形内。
    pub fn contains(&self, p: (Nm, Nm)) -> bool {
        match *self {
            Shape::Rect { min, max } => p.0 >= min.0 && p.0 <= max.0 && p.1 >= min.1 && p.1 <= max.1,
            Shape::Circle { c, r } => {
                let dx = (p.0 - c.0) as i128;
                let dy = (p.1 - c.1) as i128;
                dx * dx + dy * dy <= (r as i128) * (r as i128)
            }
        }
    }

    /// 两形之间的最近距离；相交或相切时为 0。
    pub fn distance(&self, other: &Shape) -> Nm {
        let (w, _) = closest_pair(self, other);
        w.0
    }

    /// 接触见证点：两形最近点对的中点（相交时落在交区内）。
    pub fn witness(&self, other: &Shape) -> (Nm, Nm) {
        closest_pair(self, other).1
    }
}

fn clamp(v: Nm, lo: Nm, hi: Nm) -> Nm {
    v.max(lo).min(hi)
}

/// 返回 (距离, 见证点)。
fn closest_pair(a: &Shape, b: &Shape) -> ((Nm,), (Nm, Nm)) {
    let (pa, pb) = match (*a, *b) {
        (Shape::Rect { min: amin, max: amax }, Shape::Rect { min: bmin, max: bmax }) => {
            let axis = |alo: Nm, ahi: Nm, blo: Nm, bhi: Nm| -> (Nm, Nm) {
                if ahi < blo {
                    (ahi, blo)
                } else if bhi < alo {
                    (alo, bhi)
                } else {
                    let m = (alo.max(blo) + ahi.min(bhi)) / 2;
                    (m, m)
                }
            };
            let (pax, pbx) = axis(amin.0, amax.0, bmin.0, bmax.0);
            let (pay, pby) = axis(amin.1, amax.1, bmin.1, bmax.1);
            ((pax, pay), (pbx, pby))
        }
        (Shape::Circle { c: ca, r: ra }, Shape::Circle { c: cb, r: rb }) => {
            let dx = cb.0 - ca.0;
            let dy = cb.1 - ca.1;
            let d = ((dx as i128 * dx as i128 + dy as i128 * dy as i128) as f64).sqrt();
            if d == 0.0 {
                ((ca.0, ca.1), (ca.0, ca.1))
            } else {
                let pa = (ca.0 + (dx as f64 * ra as f64 / d) as Nm, ca.1 + (dy as f64 * ra as f64 / d) as Nm);
                let pb = (cb.0 - (dx as f64 * rb as f64 / d) as Nm, cb.1 - (dy as f64 * rb as f64 / d) as Nm);
                (pa, pb)
            }
        }
        (Shape::Rect { min, max }, Shape::Circle { c, r }) => {
            let pb_on_rect = (clamp(c.0, min.0, max.0), clamp(c.1, min.1, max.1));
            let dx = c.0 - pb_on_rect.0;
            let dy = c.1 - pb_on_rect.1;
            let d = ((dx as i128 * dx as i128 + dy as i128 * dy as i128) as f64).sqrt();
            if d == 0.0 {
                (pb_on_rect, pb_on_rect)
            } else {
                let pc = (c.0 - (dx as f64 * r as f64 / d) as Nm, c.1 - (dy as f64 * r as f64 / d) as Nm);
                (pb_on_rect, pc)
            }
        }
        (Shape::Circle { .. }, Shape::Rect { .. }) => {
            let (pb, pa) = closest_pair(b, a);
            (pa, pb)
        }
    };
    let dx = (pa.0 - pb.0) as i128;
    let dy = (pa.1 - pb.1) as i128;
    let dist = ((dx * dx + dy * dy) as f64).sqrt().round() as Nm;
    let mid = ((pa.0 + pb.0) / 2, (pa.1 + pb.1) / 2);
    ((dist,), mid)
}

/// 并查集。
pub struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    pub fn new(n: usize) -> Self {
        UnionFind { parent: (0..n).collect() }
    }
    pub fn find(&mut self, x: usize) -> usize {
        let mut r = x;
        while self.parent[r] != r {
            r = self.parent[r];
        }
        let mut c = x;
        while self.parent[c] != r {
            let n = self.parent[c];
            self.parent[c] = r;
            c = n;
        }
        r
    }
    pub fn union(&mut self, a: usize, b: usize) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            // 确定性：小编号恒为根，保证等价输入得到稳定顺序。
            if ra < rb {
                self.parent[rb] = ra;
            } else {
                self.parent[ra] = rb;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tangent_rects_distance_zero() {
        let a = Shape::Rect { min: (0, 0), max: (10, 10) };
        let b = Shape::Rect { min: (10, 0), max: (20, 10) };
        assert_eq!(a.distance(&b), 0);
    }

    #[test]
    fn gap_distance() {
        let a = Shape::Rect { min: (0, 0), max: (10, 10) };
        let b = Shape::Rect { min: (15, 0), max: (20, 10) };
        assert_eq!(a.distance(&b), 5);
    }

    #[test]
    fn circle_rect() {
        let c = Shape::Circle { c: (0, 0), r: 5 };
        let r = Shape::Rect { min: (8, -1), max: (12, 1) };
        assert_eq!(c.distance(&r), 3);
    }
}
