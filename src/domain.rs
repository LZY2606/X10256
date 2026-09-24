//! Core domain types: units, raw representations, diagnostics and layer roles.
//!
//! Every imported quantity keeps its original textual representation. Only
//! derived layers normalize coordinates to the shared nanometer basis; values
//! that cannot be converted stay in place with a diagnostic instead of being
//! silently replaced by a default.

use serde::{Deserialize, Serialize};

pub const PARSER_VERSION: &str = "gerber-excellon-parser-1.0";
pub const ENGINE_VERSION: &str = "connectivity-engine-1.0";
pub const SCHEMA_VERSION: u32 = 1;
pub const COORDINATE_BASIS: &str = "nanometer";

/// Coordinate units as declared by the source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    In,
    Mm,
    /// Source never declared a unit; the value is retained, never guessed.
    Unspecified,
}

impl Unit {
    pub fn as_str(self) -> &'static str {
        match self {
            Unit::In => "in",
            Unit::Mm => "mm",
            Unit::Unspecified => "unspecified",
        }
    }
}

/// Normalized coordinate on the shared basis (nanometers, board origin).
pub type Coord = i64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Point {
    pub x: Coord,
    pub y: Coord,
}

/// A number exactly as authored, plus its normalized value when convertible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawNum {
    pub raw: String,
    pub unit: Unit,
    /// Normalized value in nanometers (length) when conversion succeeds.
    pub nano: Option<i64>,
}

impl RawNum {
    pub fn new(raw: impl Into<String>, unit: Unit, nano: Option<i64>) -> Self {
        Self { raw: raw.into(), unit, nano }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawPoint {
    pub x: RawNum,
    pub y: RawNum,
}

/// Byte/line/column location inside an immutable source material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourcePos {
    pub offset: u64,
    pub line: u32,
    pub col: u32,
}

impl SourcePos {
    pub fn new(offset: usize, line: u32, col: u32) -> Self {
        Self { offset: offset as u64, line, col }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// A diagnostic anchored at a position inside the original material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diag {
    pub code: String,
    pub message: String,
    pub severity: Severity,
    pub pos: Option<SourcePos>,
    /// The original representation that could not be consumed, retained here.
    pub raw: Option<String>,
}

impl Diag {
    pub fn error(code: &str, message: impl Into<String>, pos: Option<SourcePos>) -> Self {
        Self { code: code.into(), message: message.into(), severity: Severity::Error, pos, raw: None }
    }
    pub fn warning(code: &str, message: impl Into<String>, pos: Option<SourcePos>) -> Self {
        Self { code: code.into(), message: message.into(), severity: Severity::Warning, pos, raw: None }
    }
    pub fn with_raw(mut self, raw: impl Into<String>) -> Self {
        self.raw = Some(raw.into());
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterialKind {
    Gerber,
    Excellon,
    Stackup,
}

/// Role of a graphical layer inside the board stack.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum LayerRole {
    CopperTop,
    CopperInner { index: u32 },
    CopperBottom,
    SolderMaskTop,
    SolderMaskBottom,
    Drill,
    Other { name: String },
}

impl LayerRole {
    pub fn is_copper(&self) -> bool {
        matches!(self, LayerRole::CopperTop | LayerRole::CopperInner { .. } | LayerRole::CopperBottom)
    }
    pub fn is_mask(&self) -> bool {
        matches!(self, LayerRole::SolderMaskTop | LayerRole::SolderMaskBottom)
    }
    pub fn label(&self) -> String {
        match self {
            LayerRole::CopperTop => "copper_top".into(),
            LayerRole::CopperInner { index } => format!("copper_inner_{index}"),
            LayerRole::CopperBottom => "copper_bottom".into(),
            LayerRole::SolderMaskTop => "soldermask_top".into(),
            LayerRole::SolderMaskBottom => "soldermask_bottom".into(),
            LayerRole::Drill => "drill".into(),
            LayerRole::Other { name } => format!("other:{name}"),
        }
    }
}

/// The three distinct non-success outcomes required by the workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ComputeStatus {
    Ok,
    /// Syntax-level damage: input could not be parsed structurally.
    CorruptFormat,
    /// Structurally readable but the board model is contradictory/incomplete.
    SemanticNoSolution,
    /// Resource budget ran out; distinct from damage and semantic failure.
    BudgetExhausted,
}

/// Limits that, when exceeded, produce a `budget_exhausted` result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budget {
    pub max_grid_cells: u64,
    pub max_paint_ops: u64,
    pub max_candidate_groupings: u64,
}

impl Default for Budget {
    fn default() -> Self {
        Self { max_grid_cells: 4_000_000, max_paint_ops: 500_000, max_candidate_groupings: 64 }
    }
}

/// Convert a fixed-point Gerber/Excellon number to nanometers with rounding.
/// `n` is the signed integer formed by stripping sign and decimal point;
/// `frac` is the number of fractional digits implied by the coordinate format.
pub fn fixed_to_nm(n: i128, frac: u32, unit: Unit) -> Option<i64> {
    let per = match unit {
        Unit::Mm => 1_000_000i128,
        Unit::In => 25_400_000i128,
        Unit::Unspecified => return None,
    };
    let neg = n < 0;
    let n = n.abs();
    let scale = 10i128.checked_pow(frac)?;
    let num = n.checked_mul(per)?;
    let q = (num + scale / 2) / scale;
    let q = if neg { -q } else { q };
    i64::try_from(q).ok()
}

/// Convert an ASCII decimal number (e.g. manifest "1.6") to nanometers.
pub fn decimal_to_nm(s: &str, unit: Unit) -> Option<i64> {
    let s = s.trim();
    let (neg, rest) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let (int, frac_part) = match rest.split_once('.') {
        Some((a, b)) => (a, b),
        None => (rest, ""),
    };
    if int.is_empty() && frac_part.is_empty() {
        return None;
    }
    if !int.bytes().all(|b| b.is_ascii_digit()) || !frac_part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut digits = String::with_capacity(int.len() + frac_part.len());
    digits.push_str(int);
    digits.push_str(frac_part);
    let n: i128 = digits.parse().ok()?;
    let nm = fixed_to_nm(n, frac_part.len() as u32, unit)?;
    Some(if neg { -nm } else { nm })
}
