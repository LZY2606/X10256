//! 可参与电气连通判定的平面几何（整数纳米坐标，距离不含浮点）。

use crate::domain::P;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Shape {
    /// 圆（闪绘圆形 aperture）。
    Circle { c: P, r_nm: i64 },
    /// 轴对齐矩形（矩形 aperture 闪绘）。
    Rect { min: P, max: P },
    /// 轨迹段（draw，带宽度；端点为平面切口）。
    Segment { a: P, b: P, half_w: i64 },
}

impl Shape {
    pub fn center(&self) -> P {
        match self {
            Shape::Circle { c, .. } => *c,
            Shape::Rect { min, max } => P::new((min.x + max.x) / 2, (min.y + max.y) / 2),
            Shape::Segment { a, b, .. } => P::new((a.x + b.x) / 2, (a.y + b.y) / 2),
        }
    }

    pub fn bbox(&self) -> (P, P) {
        match self {
            Shape::Circle { c, r_nm } => (P::new(c.x - r_nm, c.y - r_nm), P::new(c.x + r_nm, c.y + r_nm)),
            Shape::Rect { min, max } => (*min, *max),
            Shape::Segment { a, b, half_w } => {
                let min = P::new(a.x.min(b.x) - half_w, a.y.min(b.y) - half_w);
                let max = P::new(a.x.max(b.x) + half_w, a.y.max(b.y) + half_w);
                (min, max)
            }
        }
    }
}

fn i128v(p: P) -> (i128, i128) {
    (p.x as i128, p.y as i128)
}

fn isqrt(n: i128) -> i128 {
    if n <= 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

/// 两形状间精确间隙（纳米）。0 表示接触或重叠。
pub fn gap_nm(a: &Shape, b: &Shape) -> i64 {
    match (a, b) {
        (Shape::Circle { c: c1, r_nm: r1 }, Shape::Circle { c: c2, r_nm: r2 }) => {
            let (x1, y1) = i128v(*c1);
            let (x2, y2) = i128v(*c2);
            let d2 = (x1 - x2).pow(2) + (y1 - y2).pow(2);
            let d = isqrt(d2);
            let need = *r1 as i128 + *r2 as i128;
            if d2 <= need * need {
                0
            } else {
                let mut g = (d - need).max(0) as i64;
                if g == 0 && d2 > need * need {
                    g = 1;
                }
                g
            }
        }
        _ => {
            if let Some(g) = axis_special(a, b) {
                return g;
            }
            if let Some(g) = axis_special(b, a) {
                return g;
            }
            generic_gap(a, b)
        }
    }
}

fn axis_special(a: &Shape, b: &Shape) -> Option<i64> {
    match (a, b) {
        (Shape::Circle { c, r_nm }, Shape::Rect { min, max }) => Some(circle_rect_gap(*c, *r_nm, *min, *max)),
        (Shape::Circle { c, r_nm }, Shape::Segment { a, b, half_w }) => {
            let rect = segment_bbox(*a, *b, *half_w);
            Some(circle_rect_gap(*c, *r_nm, rect.0, rect.1))
        }
        (Shape::Rect { min: m1, max: x1 }, Shape::Rect { min: m2, max: x2 }) => {
            Some(rect_gap(*m1, *x1, *m2, *x2))
        }
        (Shape::Rect { min, max }, Shape::Segment { a, b, half_w }) => {
            let rect = segment_bbox(*a, *b, *half_w);
            Some(rect_gap(*min, *max, rect.0, rect.1))
        }
        (Shape::Segment { a: a1, b: b1, half_w: h1 }, Shape::Segment { a: a2, b: b2, half_w: h2 }) => {
            let r1 = segment_bbox(*a1, *b1, *h1);
            let r2 = segment_bbox(*a2, *b2, *h2);
            Some(rect_gap(r1.0, r1.1, r2.0, r2.1))
        }
        _ => None,
    }
}

fn segment_bbox(a: P, b: P, half_w: i64) -> (P, P) {
    (
        P::new(a.x.min(b.x) - half_w, a.y.min(b.y) - half_w),
        P::new(a.x.max(b.x) + half_w, a.y.max(b.y) + half_w),
    )
}

fn rect_gap(min1: P, max1: P, min2: P, max2: P) -> i64 {
    let gx = (min2.x - max1.x).max(min1.x - max2.x).max(0);
    let gy = (min2.y - max1.y).max(min1.y - max2.y).max(0);
    if gx == 0 || gy == 0 {
        gx.max(gy)
    } else {
        isqrt((gx as i128).pow(2) + (gy as i128).pow(2)) as i64
    }
}

fn circle_rect_gap(c: P, r: i64, min: P, max: P) -> i64 {
    let cx = c.x.clamp(min.x, max.x);
    let cy = c.y.clamp(min.y, max.y);
    let dx = c.x - cx;
    let dy = c.y - cy;
    let d2 = (dx as i128).pow(2) + (dy as i128).pow(2);
    if d2 <= (r as i128).pow(2) {
        0
    } else {
        let g = isqrt(d2) - r as i128;
        g.max(1) as i64
    }
}

fn generic_gap(_a: &Shape, _b: &Shape) -> i64 {
    0
}

/// 形状是否包含点。
pub fn contains(shape: &Shape, p: P) -> bool {
    match shape {
        Shape::Circle { c, r_nm } => {
            let dx = p.x - c.x;
            let dy = p.y - c.y;
            (dx as i128).pow(2) + (dy as i128).pow(2) <= (*r_nm as i128).pow(2)
        }
        Shape::Rect { min, max } => p.x >= min.x && p.x <= max.x && p.y >= min.y && p.y <= max.y,
        Shape::Segment { a, b, half_w } => {
            let (min, max) = segment_bbox(*a, *b, *half_w);
            p.x >= min.x && p.x <= max.x && p.y >= min.y && p.y <= max.y
        }
    }
}
