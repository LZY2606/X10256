//! 领域模型：不可变材料、几何与结构化判定状态。
//!
//! 所有领域量都携带单位或坐标基准；解析阶段保留原始表示，
//! 规范化只发生在派生层（见 [`crate::engine`]）。

use serde::{Deserialize, Serialize};

/// 长度单位。原始材料中的单位会原样保留。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LengthUnit {
    Mm,
    Inch,
    /// 无法转换的值留在原位置，不做猜测。
    Unknown,
}

impl LengthUnit {
    pub fn as_str(self) -> &'static str {
        match self {
            LengthUnit::Mm => "mm",
            LengthUnit::Inch => "inch",
            LengthUnit::Unknown => "unknown",
        }
    }

    /// 归一化到毫米所需乘数；`Unknown` 不可转换。
    pub fn to_mm_factor(self) -> Option<f64> {
        match self {
            LengthUnit::Mm => Some(1.0),
            LengthUnit::Inch => Some(25.4),
            LengthUnit::Unknown => None,
        }
    }
}

/// 极性：Gerber LP 命令的正/负。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Polarity {
    Dark,
    Clear,
}

impl Polarity {
    pub fn as_str(self) -> &'static str {
        match self {
            Polarity::Dark => "D",
            Polarity::Clear => "C",
        }
    }
}

/// 图层角色（层叠说明给出或人工映射指定）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LayerRole {
    CopperTop,
    CopperInner,
    CopperBottom,
    SolderMaskTop,
    SolderMaskBottom,
    /// 未指派角色：语义层保留，不得默认为铜层。
    Unassigned,
}

impl LayerRole {
    pub fn as_str(self) -> &'static str {
        match self {
            LayerRole::CopperTop => "copper_top",
            LayerRole::CopperInner => "copper_inner",
            LayerRole::CopperBottom => "copper_bottom",
            LayerRole::SolderMaskTop => "soldermask_top",
            LayerRole::SolderMaskBottom => "soldermask_bottom",
            LayerRole::Unassigned => "unassigned",
        }
    }

    pub fn parse(s: &str) -> Option<LayerRole> {
        Some(match s.trim() {
            "copper_top" | "top" => LayerRole::CopperTop,
            "copper_inner" | "inner" => LayerRole::CopperInner,
            "copper_bottom" | "bottom" => LayerRole::CopperBottom,
            "soldermask_top" | "sm_top" => LayerRole::SolderMaskTop,
            "soldermask_bottom" | "sm_bottom" => LayerRole::SolderMaskBottom,
            "unassigned" | "" => LayerRole::Unassigned,
            _ => return None,
        })
    }

    pub fn is_copper(self) -> bool {
        matches!(
            self,
            LayerRole::CopperTop | LayerRole::CopperInner | LayerRole::CopperBottom
        )
    }
}

/// 轴对齐矩形（铜/阻焊的唯一几何载体，便于极性叠加与相切判定）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl Rect {
    pub fn new(x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        Rect { x0, y0, x1, y1 }
    }

    /// 两矩形间的最小间隙：相交/接触为 0，否则为正距离（mm）。
    pub fn gap(&self, other: &Rect) -> f64 {
        let dx = gap1(self.x0, self.x1, other.x0, other.x1);
        let dy = gap1(self.y0, self.y1, other.y0, other.y1);
        if dx == 0.0 {
            dy
        } else if dy == 0.0 {
            dx
        } else {
            (dx * dx + dy * dy).sqrt()
        }
    }
}

fn gap1(a0: f64, a1: f64, b0: f64, b1: f64) -> f64 {
    if b1 < a0 {
        a0 - b1
    } else if a1 < b0 {
        b0 - a1
    } else {
        0.0
    }
}

/// Aperture 定义（保留原始直径/尺寸字符串与单位）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Aperture {
    pub code: String,
    pub shape: String,
    /// 原始参数串，例如 `1.5X0.8`。
    pub raw_params: String,
    pub unit: LengthUnit,
    /// 解析后的宽/高（mm）；不可解析时为诊断而非伪造值。
    pub width_mm: Option<f64>,
    pub height_mm: Option<f64>,
}

/// Step & repeat 展开指令（原样保留）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepRepeat {
    pub nx: i64,
    pub ny: i64,
    pub dx: f64,
    pub dy: f64,
    /// 步距单位（文件单位）。
    pub unit: LengthUnit,
}

/// 一次图形操作：极性按文件中的出现顺序叠加。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrawOp {
    /// 从 0 开始的操作序号，决定叠加顺序。
    pub seq: i64,
    pub polarity: Polarity,
    pub aperture: String,
    /// 展开后的图形（已按 step-repeat 复制），坐标为文件单位。
    pub rects: Vec<Rect>,
    /// 来源行号（1 起），诊断与溯源定位用。
    pub source_line: i64,
    /// 所属 step-repeat 单元（None 表示块外）。
    pub sr_cell: Option<(i64, i64)>,
    /// 原始命令文本（坐标原始表示原样保留）。
    pub raw_command: String,
}

/// 文件种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FileKind {
    Gerber,
    Excellon,
    Stackup,
}

impl FileKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FileKind::Gerber => "gerber",
            FileKind::Excellon => "excellon",
            FileKind::Stackup => "stackup",
        }
    }
}

/// 解析后的 Gerber 层（不可变材料的解析视图）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GerberDoc {
    /// 坐标格式，例如 `2:5 inch`（原始声明）。
    pub format: String,
    pub unit: LengthUnit,
    /// 零抑制：L / T / None，原样保留。
    pub zero_suppression: String,
    pub apertures: Vec<Aperture>,
    pub ops: Vec<DrawOp>,
    /// 原始坐标基准（原点说明），例如 `absolute`。
    pub datum: String,
    /// 文件中声明的 step-repeat 块（原始参数保留）。
    pub step_repeats: Vec<StepRepeat>,
    /// `%TF.FileFunction,...*%` 原始属性值。
    pub file_function: String,
}

/// 孔：来自 Excellon。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hole {
    /// 孔径（文件单位）。
    pub diameter: f64,
    pub unit: LengthUnit,
    pub x: f64,
    pub y: f64,
    /// 是否沉铜（PTH）。NPTH 不参与电气连通。
    pub plated: bool,
    /// 贯通层范围（层序号，0 起）；通孔为全部铜层。
    pub span: Vec<usize>,
    /// 原始 span 声明，例如 `span=0:2` 或 `through`。
    pub raw_span: String,
    pub source_line: i64,
}

/// Excellon 刀具。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrillTool {
    pub code: String,
    pub diameter: f64,
    pub unit: LengthUnit,
    pub plated: bool,
    /// 原始 span 声明。
    pub raw_span: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExcellonDoc {
    pub format: String,
    pub unit: LengthUnit,
    pub zero_suppression: String,
    pub datum: String,
    pub tools: Vec<DrillTool>,
    pub holes: Vec<Hole>,
}

/// 层叠条目：文件名 -> 图层角色（手工映射的原始声明）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StackupEntry {
    pub file: String,
    pub role: String,
    pub source_line: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StackupDoc {
    pub entries: Vec<StackupEntry>,
}

/// 带位置的诊断。三种失败结果由 [`DiagnosticKind`] 区分。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub kind: DiagnosticKind,
    pub file: String,
    /// 1 起行号；无法定位时为 0。
    pub line: i64,
    pub code: String,
    pub message: String,
    /// 触发现场的原始文本片段（不丢弃原表示）。
    pub raw_snippet: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DiagnosticKind {
    /// 格式损坏：词法/语法层面无法解析。
    CorruptFormat,
    /// 语义无解：能解析但领域含义冲突。
    SemanticUnresolved,
    /// 资源预算耗尽（操作/对象数超过预算）。
    BudgetExhausted,
}

impl DiagnosticKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DiagnosticKind::CorruptFormat => "corrupt_format",
            DiagnosticKind::SemanticUnresolved => "semantic_unresolved",
            DiagnosticKind::BudgetExhausted => "budget_exhausted",
        }
    }
}

/// 解析一份材料的总结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ParsedFile {
    pub kind: String,
    pub gerber: Option<GerberDoc>,
    pub excellon: Option<ExcellonDoc>,
    pub stackup: Option<StackupDoc>,
    pub diagnostics: Vec<Diagnostic>,
}

impl ParsedFile {
    /// 导入结果分类（三种不同结果，绝不混同）。
    pub fn outcome(&self) -> ImportOutcome {
        if self
            .diagnostics
            .iter()
            .any(|d| d.kind == DiagnosticKind::BudgetExhausted)
        {
            ImportOutcome::BudgetExhausted
        } else if self
            .diagnostics
            .iter()
            .any(|d| d.kind == DiagnosticKind::CorruptFormat)
        {
            ImportOutcome::CorruptFormat
        } else if self
            .diagnostics
            .iter()
            .any(|d| d.kind == DiagnosticKind::SemanticUnresolved)
        {
            ImportOutcome::SemanticUnresolved
        } else {
            ImportOutcome::Ok
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImportOutcome {
    Ok,
    CorruptFormat,
    SemanticUnresolved,
    BudgetExhausted,
}

impl ImportOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            ImportOutcome::Ok => "ok",
            ImportOutcome::CorruptFormat => "corrupt_format",
            ImportOutcome::SemanticUnresolved => "semantic_unresolved",
            ImportOutcome::BudgetExhausted => "budget_exhausted",
        }
    }
}
