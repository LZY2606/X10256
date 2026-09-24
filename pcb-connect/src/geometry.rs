//! 派生几何层：所有坐标均为规范化皮米，并携带来源操作身份。

use serde::{Deserialize, Serialize};

/// 轴对齐整数矩形（皮米），闭区间约定 `[x0,x1) x [y0,y1)`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Rect {
    pub x0: i64,
    pub y0: i64,
    pub x1: i64,
    pub y1: i64,
}

impl Rect {
    pub fn new(x0: i64, y0: i64, x1: i64, y1: i64) -> Rect {
        Rect { x0, y0, x1, y1 }
    }
    pub fn translated(&self, dx: i64, dy: i64) -> Rect {
        Rect { x0: self.x0 + dx, y0: self.y0 + dy, x1: self.x1 + dx, y1: self.y1 + dy }
    }
    pub fn intersects(&self, other: &Rect) -> bool {
        self.x0 < other.x1 && other.x0 < self.x1 && self.y0 < other.y1 && other.y0 < self.y1
    }
    /// 间隙距离（皮米）：重叠/相接为 0。
    pub fn gap(&self, other: &Rect) -> i64 {
        let dx = (other.x0 - self.x1).max(self.x0 - other.x1).max(0);
        let dy = (other.y0 - self.y1).max(self.y0 - other.y1).max(0);
        dx.max(dy)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContactKind {
    /// 正长度边相接或面积重叠：几何上必然导通。
    Firm,
    /// 仅单点（角点）接触：零宽度，按容差版本走候选，不直接导通。
    Point,
    /// 分离但间隙在容差窗口内：邻近候选，绝不自动当导通。
    Near,
    /// 超出容差：不接触。
    None,
}

/// 两个同层矩形的接触分类。`tolerance_pm` 是版本化几何接触容差。
pub fn classify_contact(a: &Rect, b: &Rect, tolerance_pm: i64) -> ContactKind {
    let ox = a.x1.min(b.x1) - a.x0.max(b.x0);
    let oy = a.y1.min(b.y1) - a.y0.max(b.y0);
    if ox > 0 && oy > 0 {
        return ContactKind::Firm;
    }
    let touch_x = ox == 0 && a.y0 < b.y1 && b.y0 < a.y1;
    let touch_y = oy == 0 && a.x0 < b.x1 && b.x0 < a.x1;
    if touch_x || touch_y {
        return ContactKind::Firm;
    }
    let point_x = ox == 0 && (a.y1 == b.y0 || b.y1 == a.y0);
    let point_y = oy == 0 && (a.x1 == b.x0 || b.x1 == a.x0);
    let gap = a.gap(b);
    if (point_x || point_y) && gap == 0 {
        // 角点接触：容差 >=0 时进入候选，但不是 Firm。
        return ContactKind::Point;
    }
    if gap > 0 && gap <= tolerance_pm {
        return ContactKind::Near;
    }
    ContactKind::None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_contact_is_firm_corner_is_point_gap_is_near_or_none() {
        let a = Rect::new(0, 0, 10, 10);
        assert_eq!(classify_contact(&a, &Rect::new(10, 0, 20, 10), 5), ContactKind::Firm);
        assert_eq!(classify_contact(&a, &Rect::new(10, 10, 20, 20), 5), ContactKind::Point);
        assert_eq!(classify_contact(&a, &Rect::new(13, 0, 23, 10), 5), ContactKind::Near);
        assert_eq!(classify_contact(&a, &Rect::new(13, 0, 23, 10), 2), ContactKind::None);
        assert_eq!(classify_contact(&a, &Rect::new(2, 2, 8, 8), 0), ContactKind::Firm);
    }
}
