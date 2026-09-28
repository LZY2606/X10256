//! Exact rational geometry. All board quantities are carried in the declared
//! source unit (millimetres or inches) as rational numbers; only presentation
//! to the SVG layer is converted to f64.
use num_rational::Ratio;
use num_traits::Zero;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

pub type R = Ratio<i128>;

/// Rational stored as exact decimal `numerator / 10^scale` so that JSON and
/// the database never lose precision.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Q {
    pub num: i128,
    /// Non-negative; value = num / 10^scale.
    pub scale: u32,
}

impl Q {
    pub const fn raw(num: i128, scale: u32) -> Self {
        Q { num, scale }
    }
    pub fn integer(v: i128) -> Self {
        Q { num: v, scale: 0 }
    }
    pub fn ratio(&self) -> R {
        Ratio::new(self.num, 10i128.pow(self.scale))
    }
    pub fn from_ratio(r: R) -> Self {
        let mut den = *r.denom();
        let mut scale = 0u32;
        while den != 1 {
            if den % 2 == 0 {
                den /= 2;
                scale += 1;
            } else if den % 5 == 0 {
                den /= 5;
                scale += 1;
            } else {
                // Non-terminating decimal: this must never happen for board
                // quantities; inch/mm conversion (127/5) is terminating.
                panic!("non-terminating rational {:?}", r);
            }
        }
        let mut num = *r.numer();
        let mut s = scale;
        while s > 0 && num % 10 == 0 {
            num /= 10;
            s -= 1;
        }
        Q { num, scale: s }
    }
    pub fn zero() -> Self {
        Q::integer(0)
    }
    pub fn is_zero(&self) -> bool {
        self.num == 0
    }
    pub fn add(&self, o: &Q) -> Q {
        Q::from_ratio(self.ratio() + o.ratio())
    }
    pub fn sub(&self, o: &Q) -> Q {
        Q::from_ratio(self.ratio() - o.ratio())
    }
    pub fn mul(&self, o: &Q) -> Q {
        Q::from_ratio(self.ratio() * o.ratio())
    }
    pub fn neg(&self) -> Q {
        Q { num: -self.num, scale: self.scale }
    }
    pub fn f64(&self) -> f64 {
        *self.ratio().numer() as f64 / *self.ratio().denom() as f64
    }
    /// Exact decimal representation, never scientific notation.
    pub fn decimal(&self) -> String {
        if self.scale == 0 {
            return self.num.to_string();
        }
        let neg = self.num < 0;
        let n = self.num.unsigned_abs();
        let mut s = format!("{:0width$}", n, width = (self.scale + 1) as usize);
        let dot = s.len() - self.scale as usize;
        s.insert(dot, '.');
        if neg {
            format!("-{}", s)
        } else {
            s
        }
    }
}

impl std::fmt::Debug for Q {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.decimal())
    }
}

impl PartialOrd for Q {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.ratio().cmp(&other.ratio()))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Debug)]
pub struct Pt {
    pub x: Q,
    pub y: Q,
}

impl Pt {
    pub fn new(x: Q, y: Q) -> Self {
        Pt { x, y }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Debug)]
pub struct Rect {
    pub x0: Q,
    pub y0: Q,
    pub x1: Q,
    pub y1: Q,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Debug)]
pub struct Circle {
    pub cx: Q,
    pub cy: Q,
    pub r: Q,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Debug)]
pub enum Shape {
    Rect(Rect),
    Circle(Circle),
}

impl Rect {
    pub fn corners(&self) -> [(Q, Q); 4] {
        [
            (self.x0, self.y0),
            (self.x1, self.y0),
            (self.x1, self.y1),
            (self.x0, self.y1),
        ]
    }
    pub fn width(&self) -> Q {
        self.x1.sub(&self.x0)
    }
    pub fn height(&self) -> Q {
        self.y1.sub(&self.y0)
    }
    pub fn area_positive(&self) -> bool {
        self.x1 > self.x0 && self.y1 > self.y0
    }
}

impl Shape {
    pub fn rect(r: Rect) -> Self {
        Shape::Rect(r)
    }
    pub fn circle(c: Circle) -> Self {
        Shape::Circle(c)
    }
    /// Intersection with non-empty interior (overlap), as opposed to mere
    /// boundary contact.
    pub fn overlaps(&self, other: &Shape) -> bool {
        match (self, other) {
            (Shape::Rect(a), Shape::Rect(b)) => {
                a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1
            }
            (Shape::Circle(a), Shape::Circle(b)) => {
                let d2 = dist2(a.cx, a.cy, b.cx, b.cy);
                let rsum = a.r.add(&b.r);
                d2 < rsum.mul(&rsum).ratio() && a.r.add(&b.r).f64() > 0.0
            }
            _ => {
                let (c, r) = swap(other, self);
                circle_rect_overlap(c, r)
            }
        }
    }

    /// Minimum Euclidean distance between boundaries of the two shapes.
    /// Negative when one shape contains the other without touching.
    pub fn boundary_gap(&self, other: &Shape) -> Q {
        Q::from_ratio(self.boundary_gap_r(other))
    }

    fn boundary_gap_r(&self, other: &Shape) -> R {
        match (self, other) {
            (Shape::Rect(a), Shape::Rect(b)) => {
                let dx = interval_gap(a.x0.ratio(), a.x1.ratio(), b.x0.ratio(), b.x1.ratio());
                let dy = interval_gap(a.y0.ratio(), a.y1.ratio(), b.y0.ratio(), b.y1.ratio());
                if dx <= R::zero() && dy <= R::zero() {
                    dx.max(dy)
                } else if dx > R::zero() && dy > R::zero() {
                    (dx * dx + dy * dy).sqrt()
                } else {
                    dx.max(dy)
                }
            }
            (Shape::Circle(a), Shape::Circle(b)) => {
                let d = dist2(a.cx, a.cy, b.cx, b.cy).sqrt();
                d - a.r.ratio() - b.r.ratio()
            }
            _ => {
                let (c, r) = swap(other, self);
                circle_rect_gap(c, r)
            }
        }
    }

    /// True when the shapes touch only at boundary points (zero interior
    /// overlap): point contact or collinear edge overlap of zero width.
    pub fn boundary_touches(&self, other: &Shape) -> bool {
        !self.overlaps(other) && self.boundary_gap_r(other) == R::zero()
    }

    pub fn approx_sqdist_leq(&self, other: &Shape, eps: R) -> bool {
        let g = self.boundary_gap_r(other);
        if g <= R::zero() {
            return true;
        }
        g <= eps
    }
}

fn swap<'a>(a: &'a Shape, b: &'a Shape) -> (&'a Circle, &'a Rect) {
    match (a, b) {
        (Shape::Circle(c), Shape::Rect(r)) => (c, r),
        _ => unreachable!(),
    }
}

fn dist2(x0: Q, y0: Q, x1: Q, y1: Q) -> R {
    let dx = x1.ratio() - x0.ratio();
    let dy = y1.ratio() - y0.ratio();
    dx * dx + dy * dy
}

fn interval_gap(a0: R, a1: R, b0: R, b1: R) -> R {
    if b0 >= a1 {
        b0 - a1
    } else if a0 >= b1 {
        a0 - b1
    } else {
        let over = (a1 - b0).min(b1 - a0);
        -over
    }
}

fn nearest_point_on_rect(r: &Rect, x: R, y: R) -> (R, R) {
    let cx = x.clamp(r.x0.ratio(), r.x1.ratio());
    let cy = y.clamp(r.y0.ratio(), r.y1.ratio());
    (cx, cy)
}

fn circle_rect_overlap(c: &Circle, r: &Rect) -> bool {
    // Interior overlap iff centre-to-rect distance < r with at least some
    // interior points; degenerate r is handled by callers' shapes.
    let (nx, ny) = nearest_point_on_rect(r, c.cx.ratio(), c.cy.ratio());
    let dx = c.cx.ratio() - nx;
    let dy = c.cy.ratio() - ny;
    dx * dx + dy * dy < c.r.ratio() * c.r.ratio()
}

fn circle_rect_gap(c: &Circle, r: &Rect) -> R {
    let inside_x = r.x0.ratio() < c.cx.ratio() && c.cx.ratio() < r.x1.ratio();
    let inside_y = r.y0.ratio() < c.cy.ratio() && c.cy.ratio() < r.y1.ratio();
    if inside_x && inside_y {
        // circle centre inside rect: boundaries may still be disjoint
        let to_left = c.cx.ratio() - r.x0.ratio();
        let to_right = r.x1.ratio() - c.cx.ratio();
        let to_bottom = c.cy.ratio() - r.y0.ratio();
        let to_top = r.y1.ratio() - c.cy.ratio();
        return to_left.min(to_right).min(to_bottom).min(to_top) - c.r.ratio();
    }
    let (nx, ny) = nearest_point_on_rect(r, c.cx.ratio(), c.cy.ratio());
    let d = ((c.cx.ratio() - nx).pow(2) + (c.cy.ratio() - ny).pow(2)).sqrt();
    d - c.r.ratio()
}

/// Axis-aligned rectangles only: subtract `hole` from `base`, returning the
/// remaining rectangles (0 to 4). Boundary-only contact leaves `base` intact.
pub fn rect_subtract(base: &Rect, hole: &Rect) -> Vec<Rect> {
    if !Shape::Rect(*base).overlaps(&Shape::Rect(*hole)) {
        return vec![*base];
    }
    let ix0 = base.x0.ratio().max(hole.x0.ratio());
    let iy0 = base.y0.ratio().max(hole.y0.ratio());
    let ix1 = base.x1.ratio().min(hole.x1.ratio());
    let iy1 = base.y1.ratio().min(hole.y1.ratio());
    let q = Q::from_ratio;
    let mut out = Vec::new();
    // bottom strip
    if iy0 > base.y0.ratio() {
        out.push(Rect { x0: base.x0, y0: base.y0, x1: base.x1, y1: q(iy0) });
    }
    // top strip
    if iy1 < base.y1.ratio() {
        out.push(Rect { x0: base.x0, y0: q(iy1), x1: base.x1, y1: base.y1 });
    }
    // left strip
    if ix0 > base.x0.ratio() {
        out.push(Rect { x0: base.x0, y0: q(iy0), x1: q(ix0), y1: q(iy1) });
    }
    // right strip
    if ix1 < base.x1.ratio() {
        out.push(Rect { x0: q(ix1), y0: q(iy0), x1: base.x1, y1: q(iy1) });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn qi(v: i128) -> Q {
        Q::integer(v)
    }

    #[test]
    fn rect_gap_touch_overlap() {
        let a = Rect { x0: qi(0), y0: qi(0), x1: qi(2), y1: qi(2) };
        let b = Rect { x0: qi(2), y0: qi(0), x1: qi(3), y1: qi(2) };
        assert!(a.boundary_touches(&Shape::Rect(b)));
        assert!(!Shape::Rect(a).overlaps(&Shape::Rect(b)));
        let c = Rect { x0: qi(1), y0: qi(1), x1: qi(3), y1: qi(3) };
        assert!(Shape::Rect(a).overlaps(&Shape::Rect(c)));
    }

    #[test]
    fn circle_point_tangency_is_exact() {
        let r = Rect { x0: qi(0), y0: qi(0), x1: qi(10), y1: qi(10) };
        let c = Circle { cx: qi(11), cy: qi(5), r: qi(1) };
        assert!(!Shape::Rect(r).overlaps(&Shape::Circle(c)));
        assert!(Shape::Rect(r).boundary_touches(&Shape::Circle(c)));
    }
}
