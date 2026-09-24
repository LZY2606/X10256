//! 核心领域类型：所有领域量携带单位或坐标基准。
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Unit {
    Mm,
    Inch,
}

impl Unit {
    pub fn as_str(&self) -> &'static str {
        match self {
            Unit::Mm => "mm",
            Unit::Inch => "inch",
        }
    }
    pub fn to_mm_factor(&self) -> f64 {
        match self {
            Unit::Mm => 1.0,
            Unit::Inch => 25.4,
        }
    }
}

/// 带单位的量。原始表示（raw）在导入时保留，派生层才做规范化。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quantity {
    pub value: f64,
    pub unit: Unit,
    /// 原始文本表示（如 "X013500Y002000" 中的坐标字段），导入时保留。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
}

impl Quantity {
    pub fn mm(&self) -> f64 {
        self.value * self.unit.to_mm_factor()
    }
}

/// 带坐标基准的点。基准随项目坐标系声明（gerber 文件坐标系）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Coord {
    pub x: Quantity,
    pub y: Quantity,
    /// 坐标基准标识，例如 "gerber:L1" / "excellon:drill"。
    pub datum: String,
}

/// 诊断：带位置，不得用空值或默认值伪装成功。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    /// 位置：material_id + 行/字符位置。
    pub material_id: Option<String>,
    pub line: Option<u32>,
    pub column: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// 三种必须区分的失败结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Failure {
    /// 资源预算耗尽（aperture 数、step-repeat 展开数、元素数超限）。
    ResourceExhausted { budget: String, limit: u64, needed: u64 },
    /// 格式损坏：词法/语法层面无法解析。
    CorruptFormat { detail: String },
    /// 语义无解：语法合法但领域规则无法确定唯一解释。
    SemanticUnsolvable { detail: String },
}

pub fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex(&h.finalize())
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

/// 稳定身份：对规范化内容取哈希。
pub fn stable_id(prefix: &str, canonical: &str) -> String {
    format!("{}:{}", prefix, &sha256_hex(canonical.as_bytes())[..24])
}
