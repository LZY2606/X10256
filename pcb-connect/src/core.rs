//! Core domain types shared by parsers, geometry, connectivity and storage.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Unit {
    Mm,
    In,
}

impl Unit {
    pub fn as_str(self) -> &'static str {
        match self {
            Unit::Mm => "mm",
            Unit::In => "in",
        }
    }
    /// Convert a native value in this unit to millimetres.
    pub fn to_mm(self, v: f64) -> f64 {
        match self {
            Unit::Mm => v,
            Unit::In => v * 25.4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Polarity {
    Dark,
    Clear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerRole {
    Copper,
    SolderMask,
    Silk,
    PlatedDrill,
    NonPlatedDrill,
    Paste,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Top,
    Bottom,
    Inner,
}

/// Coordinate format/number basis declared by a source file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CoordFmt {
    /// Gerber `X...Y...I...J...` format like "3.5", or Excellon "3:3".
    pub format: String,
    pub integer: u8,
    pub decimal: u8,
    /// "leading" (LZ), "trailing" (TZ) or "none".
    pub zeros: String,
    pub unit: Unit,
    /// "absolute" or "incremental".
    pub notation: String,
}

/// Location inside an imported source file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Loc {
    pub source_file: String,
    pub line_no: usize,
    pub line_text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagKind {
    /// Byte/token level: the source cannot be parsed.
    Corrupt,
    /// Parsed, but the meaning cannot be resolved (missing basis, unknown code, ...).
    Semantic,
    /// Hard resource limit was hit (e.g. step-and-repeat expansion budget).
    Budget,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Diag {
    pub code: String,
    pub kind: DiagKind,
    pub message: String,
    pub loc: Option<Loc>,
    /// Raw, unconvertible token kept at its original position, never dropped.
    pub raw_value: Option<String>,
}

impl Diag {
    pub fn corrupt(code: impl Into<String>, msg: impl Into<String>, loc: Option<Loc>) -> Diag {
        Diag { code: code.into(), kind: DiagKind::Corrupt, message: msg.into(), loc, raw_value: None }
    }
    pub fn semantic(code: impl Into<String>, msg: impl Into<String>, loc: Option<Loc>) -> Diag {
        Diag { code: code.into(), kind: DiagKind::Semantic, message: msg.into(), loc, raw_value: None }
    }
    pub fn with_raw(mut self, raw: impl Into<String>) -> Diag {
        self.raw_value = Some(raw.into());
        self
    }
}

/// Structured tri-state failure plus success. Equivalent input must always
/// produce the same variant and the same ordering inside it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Outcome<T> {
    Ok(T),
    /// Resource budget exhausted during expansion/parsing.
    BudgetExceeded { budget: String, used: usize, limit: usize, diag: Diag },
    /// Format level damage.
    Corrupt { diags: Vec<Diag> },
    /// Grammatically readable but semantically unresolvable.
    SemanticImpossible { diags: Vec<Diag> },
}

impl<T> Outcome<T> {
    pub fn as_ref(&self) -> Outcome<&T> {
        match self {
            Outcome::Ok(t) => Outcome::Ok(t),
            Outcome::BudgetExceeded { budget, used, limit, diag } => Outcome::BudgetExceeded {
                budget: budget.clone(),
                used: *used,
                limit: *limit,
                diag: diag.clone(),
            },
            Outcome::Corrupt { diags } => Outcome::Corrupt { diags: diags.clone() },
            Outcome::SemanticImpossible { diags } => {
                Outcome::SemanticImpossible { diags: diags.clone() }
            }
        }
    }
    pub fn is_ok(&self) -> bool {
        matches!(self, Outcome::Ok(_))
    }
}

/// A numeric value carrying its original representation and coordinate basis.
/// When it cannot be converted, `value_mm` is `None` and a semantic diagnostic
/// is attached at the same position instead of silently dropping the value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RawNum {
    pub raw: String,
    pub unit: Option<Unit>,
    pub value_mm: Option<f64>,
}

/// A coordinate pair preserving the original Gerber/Excellon token form.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RawCoord {
    pub line_no: usize,
    pub x_raw: Option<String>,
    pub y_raw: Option<String>,
    pub x_mm: Option<f64>,
    pub y_mm: Option<f64>,
    /// True when at least one axis token could not be converted.
    pub unresolved: bool,
}
