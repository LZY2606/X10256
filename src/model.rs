//! 核心领域模型：不可变材料、可修改决定、事件、派生结果。
//! 所有领域量携带单位或坐标基准；原始表示一律保留，派生层才做规范化。

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub type Id = String;

/// 稳定身份：对内容取 sha256，等价输入得到等价身份。
pub fn stable_id(prefix: &str, parts: &[&str]) -> Id {
    let mut h = Sha256::new();
    h.update(prefix.as_bytes());
    for p in parts {
        h.update(b"\x1f");
        h.update(p.as_bytes());
    }
    format!("{}_{}", prefix, hex(&h.finalize())[..16].to_string())
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

// ---------- 单位与坐标基准 ----------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Unit {
    Inch,
    Mm,
}

impl Unit {
    pub fn to_mm(self, v: f64) -> f64 {
        match self {
            Unit::Inch => v * 25.4,
            Unit::Mm => v,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Unit::Inch => "in",
            Unit::Mm => "mm",
        }
    }
}

/// 坐标基准（datum）：解析阶段的坐标都属于某个文件自身的原点。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Datum {
    /// Gerber 文件自身坐标系
    GerberFile,
    /// Excellon 文件自身坐标系
    ExcellonFile,
    /// 规范化后的板级坐标系（毫米，板左下角原点）
    BoardMm,
}

/// 源位置：诊断与原始值都锚定到具体文件位置。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SrcPos {
    pub file: String,
    pub line: u32,
    pub col: u32,
}

/// 带单位的量：raw 永远保留；value_native 为按声明单位解析出的值；
/// 无法转换时 value_native 为 None 且必须伴随诊断，不得用默认值伪装。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Qty {
    pub raw: String,
    pub unit: Option<Unit>,
    pub datum: Datum,
    pub value_native: Option<f64>,
    pub pos: Option<SrcPos>,
}

impl Qty {
    pub fn mm(&self) -> Option<f64> {
        match (self.value_native, self.unit) {
            (Some(v), Some(u)) => Some(u.to_mm(v)),
            _ => None,
        }
    }
}

// ---------- 诊断 ----------

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagLevel {
    Error,
    Warning,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub level: DiagLevel,
    pub message: String,
    pub pos: Option<SrcPos>,
    /// 无法转换的原始值留在原位置
    pub raw_value: Option<String>,
}

impl Diagnostic {
    pub fn err(code: &str, message: impl Into<String>, pos: Option<SrcPos>) -> Self {
        Diagnostic {
            code: code.into(),
            level: DiagLevel::Error,
            message: message.into(),
            pos,
            raw_value: None,
        }
    }
}

// ---------- 极性 / 光圈 / 步进重复 / 图层角色 ----------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Polarity {
    /// LPD：叠加铜
    Add,
    /// LPC：扣除铜
    Subtract,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Aperture {
    Circle { diameter: Qty },
    Rect { w: Qty, h: Qty },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StepRepeat {
    pub nx: u32,
    pub ny: u32,
    pub dx: Qty,
    pub dy: Qty,
}

/// 图层角色：由人工映射决定，可修订。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LayerRole {
    Copper(u32),
    SolderMaskTop,
    SolderMaskBottom,
    Drill,
    Stackup,
    Unknown,
}

impl LayerRole {
    pub fn label(&self) -> String {
        match self {
            LayerRole::Copper(n) => format!("copper-L{}", n),
            LayerRole::SolderMaskTop => "mask-top".into(),
            LayerRole::SolderMaskBottom => "mask-bottom".into(),
            LayerRole::Drill => "drill".into(),
            LayerRole::Stackup => "stackup".into(),
            LayerRole::Unknown => "unknown".into(),
        }
    }
}

// ---------- 几何形状（规范化后，板级坐标系，毫米） ----------

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Shape {
    Circle { c: Vec2, r: f64 },
    Rect { c: Vec2, hw: f64, hh: f64 },
    /// 圆头线段（draw 描边）
    Capsule { a: Vec2, b: Vec2, r: f64 },
}

/// 一个规范化图元：携带来源链（材料、行号、原始坐标串、步进重复下标）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Prim {
    pub id: Id,
    pub shape: Shape,
    pub polarity: Polarity,
    pub layer: LayerRole,
    pub material_id: Id,
    pub src_line: u32,
    pub raw_coord: String,
    pub repeat: Option<(u32, u32)>,
    /// 解析时的单位与基准（原始表示）
    pub src_unit: Unit,
    pub src_datum: Datum,
}

// ---------- 连通结果 ----------

/// 接触判定：几何接触容差 τ 是版本参数。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContactKind {
    /// gap <= 0：重叠或相切，确定导通候选
    Definite,
    /// 0 < gap <= τ：邻近，只产生候选，不把邻近当导通
    Marginal,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Contact {
    pub a: Id,
    pub b: Id,
    pub gap_mm: f64,
    pub point: Vec2,
    pub kind: ContactKind,
    /// 若被负极性图元切断，记录切断者与位置
    pub cut_by: Option<Id>,
}

/// 连通分量 = 无 netlist 时的网络候选。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NetCandidate {
    pub id: Id,
    pub members: Vec<Id>,
    /// 候选性质：无 netlist，不能宣布为已导通网络
    pub conducted: bool,
    pub solution: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Conflict {
    pub id: Id,
    pub target: Id,
    pub position: Vec2,
    pub sides: Vec<String>,
    pub reason: String,
}

// ---------- 计算参数（版本化） ----------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Params {
    pub version: u32,
    /// 几何接触容差 τ（毫米）——版本参数
    pub contact_tol_mm: f64,
    /// 候选解枚举预算
    pub max_solutions: u32,
    /// 接触对枚举预算（资源预算）
    pub max_contact_pairs: u64,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            version: 1,
            contact_tol_mm: 0.0,
            max_solutions: 64,
            max_contact_pairs: 1_000_000,
        }
    }
}

// ---------- 计算结果：三种互斥的失败 + 成功 ----------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ComputeError {
    /// 资源预算耗尽
    BudgetExhausted { budget: String, used: u64, limit: u64 },
    /// 格式损坏（带位置诊断）
    FormatCorrupt { diagnostics: Vec<Diagnostic> },
    /// 语义无解（如缺单位声明、via 无层范围）
    Unsolvable { reason: String, diagnostics: Vec<Diagnostic> },
}

impl ComputeError {
    pub fn kind_label(&self) -> &'static str {
        match self {
            ComputeError::BudgetExhausted { .. } => "budget-exhausted",
            ComputeError::FormatCorrupt { .. } => "format-corrupt",
            ComputeError::Unsolvable { .. } => "unsolvable",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DerivedState {
    pub id: Id,
    /// 输入摘要：所有输入材料 digest 与参数版本的哈希
    pub input_digest: String,
    pub param_version: u32,
    /// 父级依赖事件
    pub parents: Vec<Id>,
    pub prims: Vec<Prim>,
    pub contacts: Vec<Contact>,
    pub nets: Vec<NetCandidate>,
    pub conflicts: Vec<Conflict>,
    pub diagnostics: Vec<Diagnostic>,
    /// 有效区间
    pub valid_from_seq: i64,
    pub invalidated_by: Option<Id>,
    pub invalidate_reason: Option<String>,
}
