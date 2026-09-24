//! 几何核心：正负极性按操作顺序叠加。
//!
//! 采用轴对齐矩形模型。对每条扫描带（相邻矩形边之间）维护一个
//! “带符号”一维区间集合：dark 加一段，clear 删一段；因为区间边界
//! 携带最后一次覆盖它的操作序号与极性，负性只作用于此前的正性，
//! 之后的正性会重新覆盖回来——即严格按顺序叠加而非集合异或。

use crate::model::*;
use serde::{Deserialize, Serialize};

/// 叠加结果中的一个铜矩形（规范化单位：mm）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CopperRect {
    pub id: String,
    pub role: String,
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    /// 覆盖此区域的最后一次操作序号（溯源）。
    pub last_seq: i64,
}

/// 阻焊开窗（暗极性叠加后）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaskOpen {
    pub id: String,
    pub role: String,
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

/// 把一份 Gerber 文档按操作顺序叠加为矩形集合（坐标换算到 mm）。
pub fn compose_gerber(doc: &GerberDoc, role: LayerRole, layer_index: usize) -> Vec<CopperRect> {
    let factor = doc.unit.to_mm_factor().unwrap_or(1.0);
    // 收集带顺序与极性的矩形。
    let mut stamped: Vec<(Rect, i64, Polarity)> = Vec::new();
    for op in &doc.ops {
        for r in &op.rects {
            stamped.push((scale_rect(r, factor), op.seq, op.polarity));
        }
    }
    let bands = bands_from(&stamped);
    let role_s = role.as_str();
    let mut out = Vec::new();
    for (yi, (y0, y1)) in bands.iter().enumerate() {
        // 对该带按顺序叠加一维区间。
        let mut intervals: Vec<(f64, f64, i64)> = Vec::new(); // (x0,x1,last_seq) 正性
        for (r, seq, pol) in &stamped {
            if r.y0 <= *y0 && *y1 <= r.y1 {
                match pol {
                    Polarity::Dark => add_interval(&mut intervals, r.x0, r.x1, *seq),
                    Polarity::Clear => subtract_interval(&mut intervals, r.x0, r.x1),
                }
            }
        }
        for (xi, (x0, x1, seq)) in intervals.into_iter().enumerate() {
            out.push(CopperRect {
                id: format!("copper:{role_s}:{layer_index}:{yi}:{xi}"),
                role: role_s.to_string(),
                x0,
                y0: *y0,
                x1,
                y1: *y1,
                last_seq: seq,
            });
        }
    }
    out
}

/// 阻焊开窗同样按顺序叠加（只保留暗极性区域）。
pub fn compose_mask(doc: &GerberDoc, role: LayerRole, layer_index: usize) -> Vec<MaskOpen> {
    let factor = doc.unit.to_mm_factor().unwrap_or(1.0);
    let stamped: Vec<(Rect, i64, Polarity)> = doc
        .ops
        .iter()
        .flat_map(|op| {
            op.rects
                .iter()
                .map(move |r| (scale_rect(r, factor), op.seq, op.polarity))
        })
        .collect();
    let bands = bands_from(&stamped);
    let role_s = role.as_str();
    let mut out = Vec::new();
    for (yi, (y0, y1)) in bands.iter().enumerate() {
        let mut intervals: Vec<(f64, f64, i64)> = Vec::new();
        for (r, seq, pol) in &stamped {
            if r.y0 <= *y0 && *y1 <= r.y1 {
                match pol {
                    Polarity::Dark => add_interval(&mut intervals, r.x0, r.x1, *seq),
                    Polarity::Clear => subtract_interval(&mut intervals, r.x0, r.x1),
                }
            }
        }
        for (xi, (x0, x1, _)) in intervals.into_iter().enumerate() {
            out.push(MaskOpen {
                id: format!("mask:{role_s}:{layer_index}:{yi}:{xi}"),
                role: role_s.to_string(),
                x0,
                y0: *y0,
                x1,
                y1: *y1,
            });
        }
    }
    out
}

fn scale_rect(r: &Rect, f: f64) -> Rect {
    Rect::new(r.x0 * f, r.y0 * f, r.x1 * f, r.y1 * f)
}

/// 取所有矩形 y 边形成的扫描带（端点相切会自然分成两段或合并）。
fn bands_from(stamped: &[(Rect, i64, Polarity)]) -> Vec<(f64, f64)> {
    let mut ys: Vec<f64> = stamped
        .iter()
        .flat_map(|(r, _, _)| [r.y0, r.y1])
        .collect();
    ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
    ys.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
    ys.windows(2).map(|w| (w[0], w[1])).collect()
}

/// 加一段正性区间：重叠处不重复计，并把覆盖序号更新为最新操作。
fn add_interval(intervals: &mut Vec<(f64, f64, i64)>, a: f64, b: f64, seq: i64) {
    if b <= a {
        return;
    }
    intervals.push((a, b, seq));
    // 按起点排序后，用事件线拆分：每个微区间取最大 seq 作为正性。
    let mut bounds: Vec<f64> = intervals.iter().flat_map(|(p, q, _)| [*p, *q]).collect();
    bounds.sort_by(|x, y| x.partial_cmp(y).unwrap());
    bounds.dedup_by(|x, y| (*x - *y).abs() < 1e-12);
    let mut rebuilt: Vec<(f64, f64, i64)> = Vec::new();
    for w in bounds.windows(2) {
        let mid = (w[0] + w[1]) / 2.0;
        let mut best: Option<i64> = None;
        for (p, q, s) in intervals.iter() {
            if *p <= mid && mid <= *q {
                best = Some(best.map_or(*s, |cur| cur.max(*s)));
            }
        }
        if let Some(s) = best {
            rebuilt.push((w[0], w[1], s));
        }
    }
    *intervals = merge_adjacent(rebuilt);
}

/// 删一段（clear）：仅在当前正性集合内挖洞，不影响之后到来的 dark。
fn subtract_interval(intervals: &mut Vec<(f64, f64, i64)>, a: f64, b: f64) {
    let old = std::mem::take(intervals);
    let mut out = Vec::new();
    for (p, q, s) in old {
        if b <= p || q <= a {
            out.push((p, q, s));
        } else {
            if p < a {
                out.push((p, p.max(a), s));
            }
            if b < q {
                out.push((b.max(p), q, s));
            }
        }
    }
    *intervals = merge_adjacent(out);
}

fn merge_adjacent(v: Vec<(f64, f64, i64)>) -> Vec<(f64, f64, i64)> {
    let mut v = v;
    v.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let mut out: Vec<(f64, f64, i64)> = Vec::new();
    for (p, q, s) in v {
        if let Some(last) = out.last_mut() {
            if (last.1 - p).abs() < 1e-12 && last.2 == s {
                last.1 = last.1.max(q);
                continue;
            }
        }
        out.push((p, q, s));
    }
    out
}

/// 两个 mm 矩形之间的间隙；接触（共享边/点）距离为 0。
pub fn rect_gap(a: &CopperRect, b: &CopperRect) -> f64 {
    Rect::new(a.x0, a.y0, a.x1, a.y1).gap(&Rect::new(b.x0, b.y0, b.x1, b.y1))
}

/// 点到矩形的最短距离（mm）。
pub fn point_rect_gap(x: f64, y: f64, r: &CopperRect) -> f64 {
    let dx = gap1(x, x, r.x0, r.x1);
    let dy = gap1(y, y, r.y0, r.y1);
    if dx == 0.0 {
        dy
    } else if dy == 0.0 {
        dx
    } else {
        (dx * dx + dy * dy).sqrt()
    }
}
