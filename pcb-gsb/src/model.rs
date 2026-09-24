//! Shared value types: stable ids, time basis, diagnostics and units.

use serde::{Deserialize, Serialize};

/// Deterministic id generator namespace used by seeded offline cases.
pub const CASE_NAMESPACE: uuid::Uuid =
    uuid::Uuid::from_u128(0x70_63_62_5f_67_73_62_5f_63_61_73_65_00_00_00_01);

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Derive a stable id from a fixed parent + key (used by the bundled case pack).
pub fn derived_id(parent: &str, key: &str) -> String {
    uuid::Uuid::new_v5(
        &uuid::Uuid::new_v5(&CASE_NAMESPACE, parent.as_bytes()),
        key.as_bytes(),
    )
    .to_string()
}

/// Monotonic logical clock; every append gets one. Also stores an ISO wall time.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimeBasis {
    /// Anchor epoch millis used when the case was seeded.
    pub anchor_millis: i64,
    pub seq: u64,
    /// ISO-8601 wall time rendered with the configured timezone offset.
    pub iso: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// Byte/line anchored position inside an immutable source file.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SrcPos {
    pub file_id: String,
    pub line: usize,
    pub col: usize,
    pub byte_start: usize,
    pub byte_end: usize,
    pub snippet: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: String,
    pub severity: Severity,
    pub message: String,
    pub position: Option<SrcPos>,
}

impl Diagnostic {
    pub fn err(code: &str, message: impl Into<String>) -> Self {
        Diagnostic {
            code: code.to_string(),
            severity: Severity::Error,
            message: message.into(),
            position: None,
        }
    }
    pub fn warn(code: &str, message: impl Into<String>) -> Self {
        Diagnostic {
            code: code.to_string(),
            severity: Severity::Warning,
            message: message.into(),
            position: None,
        }
    }
    pub fn info(code: &str, message: impl Into<String>) -> Self {
        Diagnostic {
            code: code.to_string(),
            severity: Severity::Info,
            message: message.into(),
            position: None,
        }
    }
    pub fn at(mut self, p: SrcPos) -> Self {
        self.position = Some(p);
        self
    }
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

/// The three distinct failure classes required by the specification.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeLabel {
    /// Input was structurally damaged / unparseable.
    FormatCorrupt,
    /// Input parsed, but its meaning cannot be resolved (unit conflict, role clash).
    SemanticUnresolvable,
    /// Deterministic resource limits were exceeded.
    BudgetExhausted,
}

/// A display value that always carries its unit / coordinate basis and stable identity.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Tagged<T: Serialize> {
    pub id: String,
    pub value: T,
    pub unit: String,
    pub basis: String,
    pub valid: bool,
    /// Set when the value could not be converted; the raw representation is retained.
    pub invalid_reason: Option<String>,
    pub source: String,
}

impl<T: Serialize> Tagged<T> {
    pub fn new(id: impl Into<String>, value: T, unit: impl Into<String>, basis: impl Into<String>, source: impl Into<String>) -> Self {
        Tagged {
            id: id.into(),
            value,
            unit: unit.into(),
            basis: basis.into(),
            valid: true,
            invalid_reason: None,
            source: source.into(),
        }
    }
    pub fn invalid(id: impl Into<String>, value: T, raw_repr: impl Into<String>, reason: impl Into<String>, source: impl Into<String>) -> Self {
        Tagged {
            id: id.into(),
            value,
            unit: "unknown".to_string(),
            basis: raw_repr.into(),
            valid: false,
            invalid_reason: Some(reason.into()),
            source: source.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum LengthUnit {
    Mm,
    In,
}

impl LengthUnit {
    pub fn label(self) -> &'static str {
        match self {
            LengthUnit::Mm => "mm",
            LengthUnit::In => "in",
        }
    }
    /// Multiplier to a canonical mm representation.
    pub fn to_mm(self) -> f64 {
        match self {
            LengthUnit::Mm => 1.0,
            LengthUnit::In => 25.4,
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "mm" => Some(LengthUnit::Mm),
            "in" | "inch" | "inches" => Some(LengthUnit::In),
            _ => None,
        }
    }
}

/// Gerber FS specification: digits before / after the decimal point.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoordFormat {
    pub int_digits: u8,
    pub dec_digits: u8,
}

impl CoordFormat {
    /// Decode an unsigned Gerber/Excellon integer coordinate given the format.
    pub fn decode(self, raw_integer: i64, unit: LengthUnit) -> f64 {
        let scale = 10f64.powi(self.dec_digits as i32);
        (raw_integer as f64 / scale) * unit.to_mm()
    }
}

/// Project display units do not change stored geometry; only presentation.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DisplayUnit {
    Mm,
    In,
    Source,
}
