//! 重建引擎：从不可变材料 + 当前决定状态派生铜区 / 孔 / 阻焊开窗 / 连通候选。
//!
//! 该模块是纯函数内核，不接触数据库与 HTTP，存储层和测试可直接调用。

use crate::diag::{Diagnostic, OutcomeKind, Severity};
use crate::geometry::{classify_contact, ContactKind, Rect};
use crate::ids::short_id;
use crate::model::*;
use crate::result::*;
use crate::units::{decimal_to_pm, fixed_point_to_pm};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy)]
pub struct BuildLimits {
    /// 布尔叠加阶段最大网格单元数。
    pub max_cells: usize,
    /// 无 netlist 候选解枚举上限。
    pub max_solutions: usize,
}

impl Default for BuildLimits {
    fn default() -> Self {
        BuildLimits { max_cells: 200_000, max_solutions: 64 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetDecisionState {
    pub candidate_key: String,
    pub assign_net: String,
    pub rejected: bool,
    pub event_seq: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpanOverride {
    pub from: Option<(String, String)>,
    pub to: Option<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct BuildInput<'a> {
    pub sources: &'a [SourceFile],
    /// 应用层角色 / 单位 / 盲埋孔范围修订后的当前层映射与源视图。
    pub layer_overrides: &'a BTreeMap<String, LayerRole>,
    pub source_units: &'a BTreeMap<String, crate::units::LengthUnit>,
    pub span_overrides: &'a BTreeMap<String, SpanOverride>,
    pub tolerance_pm: i64,
    pub param_version: i64,
    pub parent_event_seq: i64,
    pub decisions: &'a [NetDecisionState],
    pub limits: BuildLimits,
}

#[derive(Debug, Clone)]
struct Prim {
    rect: Rect,
    polarity: Polarity,
    op_seq: usize,
    line: usize,
    sr: Option<(u32, u32)>,
}

#[derive(Debug, Clone)]
struct LayerBuild {
    view: LayerView,
    prims: Vec<Prim>,
    origins_index: BTreeMap<(i64, i64), Vec<usize>>,
}

fn diag(code: &str, msg: impl Into<String>, line: usize, kind: OutcomeKind) -> Diagnostic {
    Diagnostic::new(Severity::Error, kind, code, msg, line, 1, "")
}

fn coord_pm(
    raw: &str,
    unit: crate::units::LengthUnit,
    format: Option<&CoordFormat>,
    datum_pm: i128,
) -> Option<i64> {
    let value = if raw.contains('.') {
        decimal_to_pm(raw, unit)
    } else if let Some(fmt) = format {
        fixed_point_to_pm(raw, unit, fmt.int_digits, fmt.dec_digits)
    } else {
        // Excellon 默认 2:4（英寸）/ 3:3（毫米）。
        let (i, d) = match unit {
            crate::units::LengthUnit::Inch => (2, 4),
            _ => (3, 3),
        };
        fixed_point_to_pm(raw, unit, i, d)
    }?;
    i64::try_from(value + datum_pm).ok()
}

fn q_pm(q: &crate::units::RawQuantity) -> Option<i64> {
    q.pm.and_then(|v| i64::try_from(v).ok())
}

fn aperture_rect(ap: &Aperture) -> Option<(i64, i64)> {
    match ap {
        Aperture::Rect { width, height, .. } => Some((q_pm(width)?, q_pm(height)?)),
        Aperture::Circle { diameter, .. } => {
            let d = q_pm(diameter)?;
            Some((d, d))
        }
    }
}

/// 收集某铜/阻焊层按操作顺序（含 SR 展开）的全部图形。
fn collect_layer_prims(doc: &GerberDoc, effective_unit: crate::units::LengthUnit) -> (Vec<Prim>, Vec<Diagnostic>, Option<Rect>) {
    let apertures: BTreeMap<&str, &Aperture> = doc.apertures.iter().map(|a| (a.code(), a)).collect();
    let mut prims = Vec::new();
    let mut diags = Vec::new();
    let mut cur_x: Option<i64> = None;
    let mut cur_y: Option<i64> = None;
    let mut polarity = Polarity::Dark;
    // SR 栈：（nx,ny,dx_pm,dy_pm）。
    let mut sr_stack: Vec<(u32, u32, i64, i64)> = Vec::new();
    let mut bbox: Option<Rect> = None;
    let datum_x = i64::try_from(doc.datum.x_pm).unwrap_or(0);
    let datum_y = i64::try_from(doc.datum.y_pm).unwrap_or(0);

    let xy = |x_raw: Option<&str>, y_raw: Option<&str>, cur_x: &mut Option<i64>, cur_y: &mut Option<i64>|
        -> Option<(i64, i64)> {
        if let Some(raw) = x_raw {
            cur_x.replace(coord_pm(raw, effective_unit, doc.format.as_ref(), doc.datum.x_pm)?);
        }
        if let Some(raw) = y_raw {
            cur_y.replace(coord_pm(raw, effective_unit, doc.format.as_ref(), doc.datum.y_pm)?);
        }
        Some((cur_x?, cur_y?))
    };

    let push = |prims: &mut Vec<Prim>, rect: Rect, pol: Polarity, seq: usize, line: usize| {
        if sr_stack.is_empty() {
            prims.push(Prim { rect, polarity: pol, op_seq: seq, line, sr: None });
        } else {
            // 多层 SR 时副本坐标为各级重复偏移之和。
            let mut combos: Vec<(i64, i64, u32, u32)> = vec![(0, 0, 0, 0)];
            for &(nx, ny, dx, dy) in &sr_stack {
                let mut next = Vec::new();
                for (ox, oy, ix, iy) in &combos {
                    for a in 0..nx {
                        for b in 0..ny {
                            next.push((
                                ox + a as i64 * dx,
                                oy + b as i64 * dy,
                                if sr_stack.len() == 1 { a } else { *ix },
                                if sr_stack.len() == 1 { b } else { *iy },
                            ));
                        }
                    }
                }
                combos = next;
            }
            for (ox, oy, a, b) in combos {
                prims.push(Prim {
                    rect: rect.translated(ox, oy),
                    polarity: pol,
                    op_seq: seq,
                    line,
                    sr: Some((a, b)),
                });
            }
        }
    };

    for op in &doc.ops {
        match op {
            DrawOp::Move { x_raw, y_raw, .. } => {
                let _ = xy(x_raw.as_deref(), y_raw.as_deref(), &mut cur_x, &mut cur_y);
            }
            DrawOp::Flash { seq, line, aperture, polarity: pol, x_raw, y_raw } => {
                polarity = *pol;
                let Some((cx, cy)) = xy(x_raw.as_deref(), y_raw.as_deref(), &mut cur_x, &mut cur_y) else {
                    continue;
                };
                let Some(ap) = apertures.get(ap.as_str()) else {
                    diags.push(diag("E_AP_UNDEF", format!("引用未定义 aperture {aperture}"), *line, OutcomeKind::Semantic));
                    continue;
                };
                let Some((w, h)) = aperture_rect(ap) else { continue; };
                let rect = Rect::new(cx - w / 2, cy - h / 2, cx - w / 2 + w, cy - h / 2 + h);
                push(&mut prims, rect, polarity, *seq, *line);
                bbox = Some(bbox.map(|b| union(&b, &rect)).unwrap_or(rect));
            }
            DrawOp::Segment { seq, line, aperture, polarity: pol, x_raw, y_raw } => {
                polarity = *pol;
                let (x0, y0) = match (cur_x, cur_y) {
                    (Some(x), Some(y)) => (x, y),
                    _ => continue,
                };
                let Some((x1, y1)) = xy(x_raw.as_deref(), y_raw.as_deref(), &mut cur_x, &mut cur_y) else {
                    continue;
                };
                let Some(ap) = apertures.get(ap.as_str()) else {
                    diags.push(diag("E_AP_UNDEF", format!("线段引用未定义 aperture {aperture}"), *line, OutcomeKind::Semantic));
                    continue;
                };
                let Some((w, h)) = aperture_rect(ap) else { continue; };
                let line_rect = if x0 == x1 {
                    let (t, b) = (y0.max(y1), y0.min(y1));
                    Rect::new(x0 - w / 2, b - h / 2, x0 - w / 2 + w, t + h - h / 2)
                } else if y0 == y1 {
                    let (r, l) = (x0.max(x1), x0.min(x1));
                    Rect::new(l - w / 2, y0 - h / 2, r + w - w / 2, y0 - h / 2 + h)
                } else {
                    // 斜线在领域案例中不是必需：用包围盒保守覆盖并告警。
                    diags.push(Diagnostic::new(
                        Severity::Warning, OutcomeKind::Ok, "W_DIAGONAL",
                        "斜线段以轴对齐包围盒保守覆盖", *line, 1, "",
                    ));
                    Rect::new(x0.min(x1) - w / 2, y0.min(y1) - h / 2,
                              x0.max(x1) + w - w / 2, y0.max(y1) + h - h / 2)
                };
                push(&mut prims, line_rect, polarity, *seq, *line);
                bbox = Some(bbox.map(|b| union(&b, &line_rect)).unwrap_or(line_rect));
            }
            DrawOp::StepRepeat { x_count, y_count, dx, dy, .. } => {
                let dx = q_pm(dx).unwrap_or(0);
                let dy = q_pm(dy).unwrap_or(0);
                sr_stack.push((*x_count, *y_count, dx, dy));
            }
            DrawOp::EndStepRepeat { .. } => {
                sr_stack.pop();
            }
            DrawOp::Untranslatable { line, .. } => {
                diags.push(diag("E_UNTRANSLATABLE", "坐标无法精确转换，图形保留在来源但不参与几何", *line, OutcomeKind::Semantic));
            }
        }
    }
    let _ = (datum_x, datum_y);
    (prims, diags, bbox)
}

fn union(a: &Rect, b: &Rect) -> Rect {
    Rect::new(a.x0.min(b.x0), a.y0.min(b.y0), a.x1.max(b.x1), a.y1.max(b.y1))
}
