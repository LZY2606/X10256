use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// 内部统一坐标基准：纳米 (nm)，有符号 64 位。
pub type Nm = i64;

pub const NM_PER_MM: f64 = 1_000_000.0;
pub const NM_PER_IN: f64 = 25_400_000.0;

pub fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex(&h.finalize())
}

pub fn short_id(prefix: &str, data: &[u8]) -> String {
    let full = sha256_hex(data);
    format!("{}-{}", prefix, &full[..16])
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Unit {
    Mm,
    Inch,
}

impl Unit {
    pub fn to_nm(self, v: f64) -> Nm {
        (v * match self {
            Unit::Mm => NM_PER_MM,
            Unit::Inch => NM_PER_IN,
        })
        .round() as Nm
    }
    pub fn label(self) -> &'static str {
        match self {
            Unit::Mm => "mm",
            Unit::Inch => "in",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Polarity {
    Positive,
    Negative,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum MaterialKind {
    Gerber,
    Excellon,
    Stackup,
}

impl MaterialKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MaterialKind::Gerber => "gerber",
            MaterialKind::Excellon => "excellon",
            MaterialKind::Stackup => "stackup",
        }
    }
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "gerber" => Some(MaterialKind::Gerber),
            "excellon" => Some(MaterialKind::Excellon),
            "stackup" => Some(MaterialKind::Stackup),
            _ => None,
        }
    }
}

/// 不可变材料：原始表示完整保留，任何派生数据都不回写。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Material {
    pub id: String,
    pub name: String,
    pub kind: MaterialKind,
    /// 原始文件内容，逐字节保留（原坐标格式/单位/极性指令全在其中）。
    pub content: String,
    pub sha256: String,
    /// 若本材料修订了某个旧材料，记录旧材料 id（来源关系）。
    pub supersedes: Option<String>,
    /// 手工层映射等人工标注，随材料保留。
    pub note: Option<String>,
}

impl Material {
    pub fn new(name: &str, kind: MaterialKind, content: &str, supersedes: Option<String>, note: Option<String>) -> Self {
        let digest = sha256_hex(content.as_bytes());
        let id = short_id("mat", format!("{}|{}|{}", name, kind.as_str(), digest).as_bytes());
        Material {
            id,
            name: name.to_string(),
            kind,
            content: content.to_string(),
            sha256: digest,
            supersedes,
            note,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// 带位置的诊断：输入不完整时返回它，而不是用空值/默认值伪装成功。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnostic {
    pub severity: Severity,
    pub kind: String,
    pub message: String,
    pub material_id: Option<String>,
    pub line: Option<usize>,
    pub col: Option<usize>,
    pub snippet: Option<String>,
}

impl Diagnostic {
    pub fn error(kind: &str, message: &str, material_id: Option<&str>, line: Option<usize>, snippet: Option<&str>) -> Self {
        Diagnostic {
            severity: Severity::Error,
            kind: kind.to_string(),
            message: message.to_string(),
            material_id: material_id.map(|s| s.to_string()),
            line,
            col: None,
            snippet: snippet.map(|s| s.to_string()),
        }
    }
    pub fn warning(kind: &str, message: &str, material_id: Option<&str>, line: Option<usize>) -> Self {
        Diagnostic {
            severity: Severity::Warning,
            kind: kind.to_string(),
            message: message.to_string(),
            material_id: material_id.map(|s| s.to_string()),
            line,
            col: None,
            snippet: None,
        }
    }
}

/// 三种必须可区分的失败结果。
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum FailureKind {
    /// 资源预算耗尽
    BudgetExhausted,
    /// 格式损坏
    FormatCorrupt,
    /// 语义无解
    SemanticUnsolvable,
}

/// 版本化参数：几何接触容差等均为版本参数。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ParamSet {
    pub version: u32,
    /// 几何接触容差（nm）。距离 <= 容差 判为接触候选。
    pub contact_tolerance_nm: Nm,
    /// 孔与铜的附着环宽（nm）：孔心到铜形距离 <= 孔半径 + 环宽 视为附着。
    pub annular_ring_nm: Nm,
    /// 资源预算：解析与求交允许的最大操作数。
    pub op_budget: u64,
}

impl Default for ParamSet {
    fn default() -> Self {
        ParamSet {
            version: 1,
            contact_tolerance_nm: 5_000,   // 0.005 mm
            annular_ring_nm: 150_000,      // 0.15 mm
            op_budget: 1_000_000,
        }
    }
}

/// 机器可读取的规则判断事件载荷。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuleJudgment {
    pub rule: String,
    pub statement: String,
    pub basis: String,
}

pub const RULE_POLARITY_ORDERED: &str = "polarity_ordered_composition";
pub const RULE_TOLERANCE_VERSIONED: &str = "contact_tolerance_is_versioned";
pub const RULE_NO_NETLIST_CANDIDATES: &str = "no_netlist_candidates_only";
