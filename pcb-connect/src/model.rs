//! 领域数据模型：不可变材料层（原始文件与解析结果）与可修改决定层（事件）。

use crate::diag::Diagnostic;
use crate::units::{Datum, LengthUnit, RawQuantity};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Gerber,
    Excellon,
    Stackup,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::Gerber => "gerber",
            SourceKind::Excellon => "excellon",
            SourceKind::Stackup => "stackup",
        }
    }
}

/// 不可变原始材料。原文、声明单位与解析出的文档都挂在这个对象上。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceFile {
    pub id: String,
    pub filename: String,
    pub kind: SourceKind,
    pub content: String,
    pub declared_unit: Option<LengthUnit>,
    pub parsed: ParsedDocument,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ParsedDocument {
    Gerber(GerberDoc),
    Excellon(ExcellonDoc),
    Stackup(StackupDoc),
    Unparsed { diagnostics: Vec<Diagnostic> },
}

// ============================ Gerber ============================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Polarity {
    Dark,
    Clear,
}

impl Polarity {
    pub fn as_str(self) -> &'static str {
        match self {
            Polarity::Dark => "dark",
            Polarity::Clear => "clear",
        }
    }
}

/// 坐标格式（`%FSLAX25Y25*%` 等）。原始规范整体保留在 `format_spec`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoordFormat {
    pub format_spec: String,
    pub absolute: bool,
    pub leading_zeros: bool,
    pub int_digits: u32,
    pub dec_digits: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum Aperture {
    Circle { code: String, diameter: RawQuantity },
    Rect { code: String, width: RawQuantity, height: RawQuantity },
}

impl Aperture {
    pub fn code(&self) -> &str {
        match self {
            Aperture::Circle { code, .. } | Aperture::Rect { code, .. } => code,
        }
    }
}

/// 单个图形操作的原始片段。坐标保留原始 token；规范化值在派生层生成。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum DrawOp {
    Flash {
        seq: usize,
        line: usize,
        aperture: String,
        polarity: Polarity,
        x_raw: Option<String>,
        y_raw: Option<String>,
    },
    Move {
        seq: usize,
        line: usize,
        x_raw: Option<String>,
        y_raw: Option<String>,
    },
    Segment {
        seq: usize,
        line: usize,
        aperture: String,
        polarity: Polarity,
        x_raw: Option<String>,
        y_raw: Option<String>,
    },
    /// step-repeat 指令本身保留；展开在派生层按顺序发生。
    StepRepeat {
        seq: usize,
        line: usize,
        x_count: u32,
        y_count: u32,
        dx: RawQuantity,
        dy: RawQuantity,
    },
    EndStepRepeat { seq: usize, line: usize },
    /// 坐标无法转换：保留在原位，附诊断，几何引擎跳过该图形。
    Untranslatable {
        seq: usize,
        line: usize,
        raw: String,
        diagnostic_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GerberDoc {
    pub unit: LengthUnit,
    pub unit_spec: String,
    pub format: Option<CoordFormat>,
    pub datum: Datum,
    pub apertures: Vec<Aperture>,
    pub ops: Vec<DrawOp>,
    pub diagnostics: Vec<Diagnostic>,
}

// ============================ Excellon ============================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HolePlating {
    Plated,
    NonPlated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrillTool {
    pub code: String,
    pub diameter: RawQuantity,
    pub plating: HolePlating,
    /// `SPAN` 声明的起止导电层名；None 表示通孔（贯通全部铜层）。
    pub span: Option<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrillHit {
    pub seq: usize,
    pub line: usize,
    pub tool: String,
    pub x_raw: Option<String>,
    pub y_raw: Option<String>,
    /// 坐标无法精确转换时保留并附诊断，绝不静默丢弃。
    pub diagnostic_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExcellonDoc {
    pub unit: LengthUnit,
    pub unit_spec: String,
    pub int_digits: u32,
    pub dec_digits: u32,
    pub datum: Datum,
    pub tools: Vec<DrillTool>,
    pub hits: Vec<DrillHit>,
    pub diagnostics: Vec<Diagnostic>,
}

// ============================ 层叠说明 ============================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum LayerRole {
    Copper,
    Mask,
    Silk,
    Mechanical,
    Drill,
    Core,
    Prepreg,
}

impl LayerRole {
    pub fn as_str(self) -> &'static str {
        match self {
            LayerRole::Copper => "copper",
            LayerRole::Mask => "mask",
            LayerRole::Silk => "silk",
            LayerRole::Mechanical => "mechanical",
            LayerRole::Drill => "drill",
            LayerRole::Core => "core",
            LayerRole::Prepreg => "prepreg",
        }
    }
    pub fn parse(s: &str) -> Option<LayerRole> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "copper" => LayerRole::Copper,
            "mask" | "soldermask" => LayerRole::Mask,
            "silk" | "silkscreen" => LayerRole::Silk,
            "mechanical" | "mech" | "outline" => LayerRole::Mechanical,
            "drill" => LayerRole::Drill,
            "core" => LayerRole::Core,
            "prepreg" => LayerRole::Prepreg,
            _ => return None,
        })
    }
}

/// 层叠中的一层，映射到 Gerber/Excellon 源文件；手工映射原样保留。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackLayer {
    pub name: String,
    pub order: i32,
    pub role: LayerRole,
    pub source_file: Option<String>,
    /// 手工映射与按文件名推断的自动映射并存，决定层可修订。
    pub manual: bool,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackupDoc {
    pub layers: Vec<StackLayer>,
    pub diagnostics: Vec<Diagnostic>,
}

// ============================ 决定层：事件 ============================

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventPayload {
    /// 导入一组原始文件（基线）。
    Import { source_ids: Vec<String>, stackup_id: Option<String> },
    /// 单位切换：只影响该源及其派生层；原值保留，规范化值重算。
    UnitSwitch {
        source_id: String,
        from_unit: LengthUnit,
        to_unit: LengthUnit,
    },
    /// 几何接触容差是版本参数（同时记录原始输入）。
    ToleranceRevision {
        from_pm: i128,
        to_pm: i128,
        raw: String,
        unit: LengthUnit,
        note: String,
    },
    /// 只改动一个直接来源（源数据修订），局部依赖重算。
    SourceRevision { source_id: String, new_source_id: String, reason: String },
    /// 层角色修订（如误判的机械层改为铜层）。
    LayerRoleRevision {
        layer_name: String,
        from_role: LayerRole,
        to_role: LayerRole,
        reason: String,
    },
    /// 盲埋孔层范围修订。
    SpanRevision {
        tool_code: String,
        from: Option<(String, String)>,
        to: Option<(String, String)>,
    },
    /// 对某连通候选的人工接受 / 否定；冲突的人工决定会形成矛盾。
    NetDecision {
        candidate_key: String,
        assign_net: String,
        rejected: bool,
        rationale: String,
    },
    /// 一次重算请求（结果记录输入摘要、参数版本、父级依赖）。
    Compute { result_id: Option<String>, requested_by: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventRecord {
    pub seq: i64,
    pub id: String,
    pub branch: String,
    pub parent_event_id: Option<String>,
    pub payload: EventPayload,
    /// 序号即逻辑时间基准；墙钟时间不参与等价性。
    pub created_revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Branch {
    pub name: String,
    pub head_seq: i64,
    pub label: String,
}
