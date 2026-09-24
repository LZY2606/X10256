//! 派生结果模型：每个结果记录输入摘要、参数版本与父级依赖，身份内容寻址。

use crate::diag::Diagnostic;
use crate::geometry::Rect;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildStatus {
    Ok,
    Corrupt,
    Semantic,
    Budget,
}

/// 一次具体图形（某次 polarity 操作或某次 SR 副本）的来源指针。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub source_id: String,
    pub layer_name: String,
    pub op_seq: usize,
    pub line: usize,
    pub polarity: String,
    pub sr_replica: Option<(u32, u32)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CopperCell {
    pub id: String,
    pub layer_name: String,
    pub rect: Rect,
    pub origins: Vec<Provenance>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoleView {
    pub id: String,
    pub source_id: String,
    pub tool: String,
    pub rect: Rect,
    pub plated: bool,
    pub span_from: Option<String>,
    pub span_to: Option<String>,
    pub connects_layers: Vec<String>,
    pub diagnostic_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaskOpening {
    pub id: String,
    pub layer_name: String,
    pub rect: Rect,
    pub origins: Vec<Provenance>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerView {
    pub name: String,
    pub role: String,
    pub order: i32,
    pub source_id: Option<String>,
    pub unit: String,
    /// 坐标基准（规范化后的皮米偏移）与原始基准并存。
    pub datum_x_pm: i64,
    pub datum_y_pm: i64,
    pub bbox: Option<Rect>,
}

/// 可选边：邻近/点接触不自动导通，只产生候选。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptionalEdge {
    pub id: String,
    pub kind: String,
    pub gap_pm: i64,
    pub a_component: String,
    pub b_component: String,
    pub layer_name: String,
    pub via_hole: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentView {
    pub id: String,
    /// 分层成员：层名 -> 单元 id 列表。
    pub members: std::collections::BTreeMap<String, Vec<String>>,
    pub holes: Vec<String>,
    pub bbox: Rect,
    /// 被人工决定赋予/否定的网络（不影响几何，只记录决定）。
    pub net_label: Option<String>,
    pub rejected: bool,
}

/// 无 netlist 时的一个合法候选解：可选边的一种取舍组合。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateSolution {
    pub id: String,
    pub accepted_edges: Vec<String>,
    pub component_groups: Vec<Vec<String>>,
    pub adopted: bool,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contradiction {
    pub id: String,
    pub kind: String,
    pub message: String,
    pub event_seq: i64,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildResult {
    pub id: String,
    pub status: BuildStatus,
    /// 输入摘要（每个来源的内容指纹 + 决定事件链）。
    pub input_digest: String,
    pub param_version: i64,
    pub tolerance_pm: i64,
    pub parent_event_seq: i64,
    /// 有效区间：自 `valid_from_seq` 起，直到后续相关修订之前。
    pub valid_from_seq: i64,
    pub valid_to_seq: Option<i64>,
    pub layers: Vec<LayerView>,
    pub cells: Vec<CopperCell>,
    pub holes: Vec<HoleView>,
    pub mask_openings: Vec<MaskOpening>,
    pub components: Vec<ComponentView>,
    pub optional_edges: Vec<OptionalEdge>,
    pub candidate_solutions: Vec<CandidateSolution>,
    pub contradictions: Vec<Contradiction>,
    pub diagnostics: Vec<Diagnostic>,
}
