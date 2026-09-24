use crate::model::{Nm, Shape};

/// 两形状之间的间隙(纳米)。相交或相切时为 0。
pub fn gap(a: &Shape, b: &Shape) -> Nm {
    match (a, b) {
        (
            Shape::Rect { x0: ax0, y0: ay0, x1: ax1, y1: ay1 },
            Shape::Rect { x0: bx0, y0: by0, x1: bx1, y1: by1 },
        ) => {
            let dx = (*ax0 - *bx1).max(*bx0 - *ax1).max(0);
            let dy = (*ay0 - *by1).max(*by0 - *ay1).max(0);
            hypot_nm(dx, dy)
        }
        (Shape::Circle { cx, cy, r }, Shape::Rect { .. })
        | (Shape::Rect { .. }, Shape::Circle { cx, cy, r }) => {
            let (rect, cx, cy, r) = match (a, b) {
                (Shape::Rect { x0, y0, x1, y1 }, _) => ((*x0, *y0, *x1, *y1), *cx, *cy, *r),
                (_, Shape::Rect { x0, y0, x1, y1 }) => ((*x0, *y0, *x1, *y1), *cx, *cy, *r),
                _ => unreachable!(),
            };
            let nx = cx.clamp(rect.0, rect.2);
            let ny = cy.clamp(rect.1, rect.3);
            (hypot_nm(cx - nx, cy - ny) - r).max(0)
        }
        (
            Shape::Circle { cx: ax, cy: ay, r: ar },
            Shape::Circle { cx: bx, cy: by, r: br },
        ) => (hypot_nm(ax - bx, ay - by) - ar - br).max(0),
    }
}

fn hypot_nm(dx: Nm, dy: Nm) -> Nm {
    if dx == 0 {
        return dy;
    }
    if dy == 0 {
        return dx;
    }
    ((dx as f64).hypot(dy as f64)).round() as Nm
}

pub fn intersect(a: &Shape, b: &Shape) -> bool {
    gap(a, b) == 0
}
