//! 领域基础：长度单位、坐标基准、带位置诊断、内容寻址身份。

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const RULES_VERSION: &str = "rules-2026.09.1-pcb-spectrum-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LengthUnit {
    Mm,
    In,
}

impl LengthUnit {
    pub fn code(self) -> &'static str {
        match self {
            LengthUnit::Mm => "MM",
            LengthUnit::In => "IN",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            LengthUnit::Mm => "mm",
            LengthUnit::In => "in",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_uppercase().as_str() {
            "MM" | "METRIC" => Some(LengthUnit::Mm),
            "IN" | "INCH" => Some(LengthUnit::In),
            _ => None,
        }
    }
}

impl std::fmt::Display for LengthUnit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// 文件内坐标格式（保留原始表示所需）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoordFormat {
    pub unit: LengthUnit,
    pub int_digits: u8,
    pub dec_digits: u8,
    /// zero 抑制：L=前导零省略，T=后随零省略，D=不省略。
    pub zero_mode: char,
    pub absolute: bool,
    pub raw: String,
}

/// 原文件中的坐标文本，绝不丢弃；无法规范化时 coord_nm 为 None 并附诊断。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawCoord {
    pub text: String,
    pub axis: char,
    pub x_nm: Option<i64>,
    pub y_nm: Option<i64>,
}

/// 指向字节流/行的位置，所有诊断必须带位置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pos {
    pub file_id: Option<String>,
    pub filename: Option<String>,
    pub line: usize,
    pub col: usize,
    pub offset: usize,
    pub snippet: String,
}

impl Pos {
    pub fn new(line: usize, col: usize, offset: usize, snippet: impl Into<String>) -> Self {
        Pos {
            file_id: None,
            filename: None,
            line,
            col,
            offset,
            snippet: snippet.into(),
        }
    }
    pub fn with_file(mut self, file_id: &str, filename: &str) -> Self {
        self.file_id = Some(file_id.to_string());
        self.filename = Some(filename.to_string());
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagKind {
    /// 词法/语法损坏：字节序列不满足格式语法。
    FormatCorrupt,
    /// 语法可解析但语义无解（孔径未定义、层范围不存在等）。
    SemanticUnresolved,
    /// 值无法转换为统一基准，原值保留在原位置。
    UnconvertibleValue,
    /// 资源预算耗尽（step-repeat 展开、候选解数量等）。
    ResourceExhausted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub kind: DiagKind,
    pub code: String,
    pub message: String,
    pub pos: Pos,
}

impl Diagnostic {
    pub fn new(kind: DiagKind, code: &str, message: impl Into<String>, pos: Pos) -> Self {
        Diagnostic {
            kind,
            code: code.to_string(),
            message: message.into(),
            pos,
        }
    }
}

/// 三种必须彼此区分的失败结果 + 成功。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadOutcome<T> {
    Ok(T),
    FormatCorrupt(Vec<Diagnostic>),
    SemanticUnresolved(T, Vec<Diagnostic>),
    ResourceExhausted(T, BudgetReport, Vec<Diagnostic>),
}

impl<T> LoadOutcome<T> {
    pub fn diagnostics(&self) -> &[Diagnostic] {
        match self {
            LoadOutcome::Ok(_) => &[],
            LoadOutcome::FormatCorrupt(d) => d,
            LoadOutcome::SemanticUnresolved(_, d) => d,
            LoadOutcome::ResourceExhausted(_, _, d) => d,
        }
    }
    pub fn is_ok(&self) -> bool {
        matches!(self, LoadOutcome::Ok(_))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budgets {
    pub max_step_repeat_cells: usize,
    pub max_copper_elements: usize,
    pub max_candidate_solutions: usize,
    pub max_file_bytes: usize,
}

impl Default for Budgets {
    fn default() -> Self {
        Budgets {
            max_step_repeat_cells: 10_000,
            max_copper_elements: 200_000,
            max_candidate_solutions: 16,
            max_file_bytes: 8 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetReport {
    pub limit: usize,
    pub observed: usize,
    pub resource: &'static str,
}

/// 规范化坐标（整数纳米，避免浮点导致相切判定不稳定）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct P {
    pub x: i64,
    pub y: i64,
}

impl P {
    pub const fn new(x: i64, y: i64) -> Self {
        P { x, y }
    }
}

/// Gerber 坐标数字串 -> 纳米整数。digits 为去符号后的数字。
pub fn digits_to_nm(digits: &str, format: &CoordFormat) -> Option<i64> {
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let total = (format.int_digits + format.dec_digits) as usize;
    let padded = match format.zero_mode {
        'L' => format!("{:0>width$}", digits, width = total),
        'T' => format!("{:0<width$}", digits, width = total),
        _ => {
            if digits.len() != total {
                format!("{:0>width$}", digits, width = total)
            } else {
                digits.to_string()
            }
        }
    };
    let (int_part, dec_part) = padded.split_at(format.int_digits as usize);
    let int_v: i128 = int_part.parse().ok()?;
    let dec_v: i128 = if format.dec_digits == 0 {
        0
    } else {
        dec_part.parse().ok()?
    };
    let value = int_v * 10i128.pow(format.dec_digits as u32) + dec_v;
    let nm = match format.unit {
        LengthUnit::Mm => value * 10i128.pow(6 - format.dec_digits as u32),
        LengthUnit::In => value * 25_400_000i128 / 10i128.pow(format.dec_digits as u32),
    };
    if nm > i64::MAX as i128 || nm < i64::MIN as i128 {
        return None;
    }
    Some(nm as i64)
}

/// 任意可序列化值 -> 带前缀的稳定内容身份。对象键按字典序规范化。
pub fn content_id(prefix: &str, value: &impl Serialize) -> String {
    let json = serde_json::to_value(value).expect("serialize id input");
    let canonical = canonical_json(&json);
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    let digest = hasher.finalize();
    format!("{}_{}", prefix, hex::encode(&digest[..10]))
}

fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let parts: Vec<String> = keys
                .iter()
                .map(|k| format!("{}:{}", serde_json::to_string(k).unwrap(), canonical_json(&map[*k])))
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        serde_json::Value::Array(items) => {
            format!("[{}]", items.iter().map(canonical_json).collect::<Vec<_>>().join(","))
        }
        other => other.to_string(),
    }
}
