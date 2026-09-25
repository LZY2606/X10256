use serde::{Deserialize, Serialize};

pub type Nm = i64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum Shape {
    Rect { x0: Nm, y0: Nm, x1: Nm, y1: Nm },
    Circle { cx: Nm, cy: Nm, r: Nm },
}

impl Shape {
    pub fn rect(x0: Nm, y0: Nm, x1: Nm, y1: Nm) -> Shape {
        Shape::Rect { x0: x0.min(x1), y0: y0.min(y1), x1: x0.max(x1), y1: y0.max(y1) }
    }
    pub fn bbox(&self) -> (Nm, Nm, Nm, Nm) {
        match *self {
            Shape::Rect { x0, y0, x1, y1 } => (x0, y0, x1, y1),
            Shape::Circle { cx, cy, r } => (cx - r, cy - r, cx + r, cy + r),
        }
    }
    pub fn centroid(&self) -> (Nm, Nm) {
        match *self {
            Shape::Rect { x0, y0, x1, y1 } => ((x0 + x1) / 2, (y0 + y1) / 2),
            Shape::Circle { cx, cy, .. } => (cx, cy),
        }
    }
}

fn ceil_isqrt(n: u128) -> u128 {
    if n == 0 {
        return 0;
    }
    let mut x = (n as f64).sqrt() as u128;
    while x * x < n {
        x += 1;
    }
    while x > 0 && (x - 1) * (x - 1) >= n {
        x -= 1;
    }
    x
}

fn hypot_ceil(dx: Nm, dy: Nm) -> Nm {
    let (dx, dy) = (dx.unsigned_abs() as u128, dy.unsigned_abs() as u128);
    ceil_isqrt(dx * dx + dy * dy) as Nm
}

pub fn gap(a: &Shape, b: &Shape) -> Nm {
    match (*a, *b) {
        (Shape::Rect { x0: ax0, y0: ay0, x1: ax1, y1: ay1 },
         Shape::Rect { x0: bx0, y0: by0, x1: bx1, y1: by1 }) => {
            let dx = (ax0 - bx1).max(bx0 - ax1).max(0);
            let dy = (ay0 - by1).max(by0 - ay1).max(0);
            hypot_ceil(dx, dy)
        }
        (Shape::Circle { cx: ax, cy: ay, r: ar }, Shape::Circle { cx: bx, cy: by, r: br }) => {
            let d = hypot_ceil(ax - bx, ay - by);
            (d - ar - br).max(0)
        }
        (Shape::Rect { x0, y0, x1, y1 }, Shape::Circle { cx, cy, r })
        | (Shape::Circle { cx, cy, r }, Shape::Rect { x0, y0, x1, y1 }) => {
            let nx = cx.clamp(x0, x1);
            let ny = cy.clamp(y0, y1);
            let d = hypot_ceil(cx - nx, cy - ny);
            (d - r).max(0)
        }
    }
}

pub fn contains(outer: &Shape, inner: &Shape) -> bool {
    match (*outer, *inner) {
        (Shape::Rect { x0: a0, y0: b0, x1: a1, y1: b1 },
         Shape::Rect { x0: c0, y0: d0, x1: c1, y1: d1 }) => a0 <= c0 && b0 <= d0 && c1 <= a1 && d1 <= b1,
        (Shape::Circle { cx, cy, r }, Shape::Circle { cx: x2, cy: y2, r: r2 }) => {
            hypot_ceil(cx - x2, cy - y2) + r2 <= r
        }
        (Shape::Rect { .. }, Shape::Circle { .. }) => {
            let (ix0, iy0, ix1, iy1) = inner.bbox();
            contains(outer, &Shape::Rect { x0: ix0, y0: iy0, x1: ix1, y1: iy1 })
        }
        (Shape::Circle { cx, cy, r }, Shape::Rect { x0, y0, x1, y1 }) => {
            [(x0, y0), (x0, y1), (x1, y0), (x1, y1)]
                .iter()
                .all(|&(px, py)| hypot_ceil(cx - px, cy - py) <= r)
        }
    }
}

pub fn subtract(base: &Shape, cut: &Shape) -> (Vec<Shape>, bool) {
    if gap(base, cut) > 0 {
        return (vec![*base], true);
    }
    match (*base, *cut) {
        (Shape::Rect { x0: bx0, y0: by0, x1: bx1, y1: by1 },
         Shape::Rect { x0: cx0, y0: cy0, x1: cx1, y1: cy1 }) => {
            let ix0 = bx0.max(cx0);
            let iy0 = by0.max(cy0);
            let ix1 = bx1.min(cx1);
            let iy1 = by1.min(cy1);
            if ix0 >= ix1 || iy0 >= iy1 {
                return (vec![*base], true);
            }
            let mut out = Vec::new();
            if bx0 < ix0 {
                out.push(Shape::rect(bx0, by0, ix0, by1));
            }
            if ix1 < bx1 {
                out.push(Shape::rect(ix1, by0, bx1, by1));
            }
            if by0 < iy0 {
                out.push(Shape::rect(ix0, by0, ix1, iy0));
            }
            if iy1 < by1 {
                out.push(Shape::rect(ix0, iy1, ix1, by1));
            }
            (out, true)
        }
        _ => {
            if contains(cut, base) {
                (vec![], true)
            } else {
                (vec![*base], false)
            }
        }
    }
}

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
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent[ra.max(rb)] = ra.min(rb);
        }
    }
}
