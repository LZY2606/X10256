
//! Domain model: immutable materials, positions, diagnostics and the three
//! distinct failure outcomes.

pub mod geom;

use serde::{Deserialize, Serialize};

/// Physical length units. Only `mm` and `inch` are legal Gerber/Excellon
/// declarations; normalization to nanometres is exact for both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LengthUnit {
    Inch,
    Mm,
}

impl LengthUnit {
    pub fn as_str(self) -> &'static str {
        match self {
            LengthUnit::Inch => "inch",
            LengthUnit::Mm => "mm",
        }
    }

 /// One declared unit in nanometres: 1 inch = 25.4 mm exactly.
    pub fn nm_per_unit(self) -> i64 {
        match self {
            LengthUnit::Inch => 25_400_000,
            LengthUnit::Mm => 1_000_000,
        }
    }
}

/// Coordinate format `FS` declaration, e.g. 2:5 inch, 3:3 mm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoordFormat {
    pub integer: u8,
    pub decimal: u8,
    /// Raw declaration, verbatim, e.g. `%FSLAX25Y25*%`.
    pub raw: String,
}

/// Byte/line position inside an original file. 1-based line/column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub line: usize,
    pub column: usize,
    pub offset: usize,
    pub length: usize,
}

impl Span {
    pub fn new(line: usize, column: usize, offset: usize, length: usize) -> Self {
        Span {
            line,
            column,
            offset,
            length,
        }
    }
    pub fn point(line: usize, column: usize, offset: usize) -> Self {
        Span::new(line, column, offset, 0)
    }
}

/// Machine readable diagnostic codes. The numeric suffix never changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagCode {
    /// G001: token violates the file grammar.
    FormatCorrupt,
    /// G002: the file parsed but references something that cannot be resolved
    /// (unknown aperture, unknown layer, value that cannot be converted...).
    SemanticUnresolved,
    /// G003: a feature subset we do not evaluate was encountered.
    UnsupportedPrimitive,
    /// G004: a step-repeat expansion exceeded the configured resource budget.
    BudgetExhausted,
    /// G005: manual mapping / role information disagrees with file metadata.
    MappingConflict,
}

impl DiagCode {
    pub fn as_str(self) -> &'static str {
        match self {
            DiagCode::FormatCorrupt => "G001_FORMAT_CORRUPT",
            DiagCode::SemanticUnresolved => "G002_SEMANTIC_UNRESOLVED",
            DiagCode::UnsupportedPrimitive => "G003_UNSUPPORTED_PRIMITIVE",
            DiagCode::BudgetExhausted => "G004_BUDGET_EXHAUSTED",
            DiagCode::MappingConflict => "G005_MAPPING_CONFLICT",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

/// A located diagnostic. Values that cannot be converted stay at their
/// original position (`raw_value`) and never become silent defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diag {
    pub code: String,
    pub severity: Severity,
    pub message: String,
    pub material_id: Option<String>,
    pub filename: Option<String>,
    pub span: Option<Span>,
    pub raw_value: Option<String>,
}

impl Diag {
    pub fn new(code: DiagCode, severity: Severity, message: impl Into<String>) -> Self {
        Diag {
            code: code.as_str().to_string(),
            severity,
            message: message.into(),
            material_id: None,
            filename: None,
            span: None,
            raw_value: None,
        }
    }

    pub fn at(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }
    pub fn raw(mut self, value: impl Into<String>) -> Self {
        self.raw_value = Some(value.into());
        self
    }
    pub fn in_material(mut self, id: &str, filename: &str) -> Self {
        self.material_id = Some(id.to_string());
        self.filename = Some(filename.to_string());
        self
    }
}

/// The three mutually distinct terminal outcomes required by the workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    /// File grammar is broken; no partial success is reported.
    FormatCorrupt { diags: Vec<Diag> },
    /// Parsed fine, but the geometry/semantics have no legal interpretation.
    SemanticImpossible { diags: Vec<Diag> },
    /// Expansion would consume more than the configured resource budget.
    ResourceExhausted {
        code: String,
        limit: i64,
        used: i64,
        detail: String,
    },
}

impl Outcome {
    pub fn status_name(&self) -> &'static str {
        match self {
            Outcome::FormatCorrupt { .. } => "format_corrupt",
            Outcome::SemanticImpossible { .. } => "semantic_impossible",
            Outcome::ResourceExhausted { .. } => "resource_exhausted",
        }
    }
}

/// Internal error carrying the structured terminal outcome or a storage fault.
#[derive(Debug)]
pub enum PipelineError {
    Outcome(Outcome),
    Store(String),
}

impl std::fmt::Display for PipelineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PipelineError::Outcome(o) => write!(f, "pipeline outcome: {}", o.status_name()),
            PipelineError::Store(s) => write!(f, "storage error: {s}"),
        }
    }
}

impl std::error::Error for PipelineError {}

impl From<rusqlite::Error> for PipelineError {
    fn from(e: rusqlite::Error) -> Self {
        PipelineError::Store(e.to_string())
    }
}

pub type PipelineResult<T> = Result<T, PipelineError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MaterialKind {
    Gerber,
    Excellon,
    Stackup,
    LayerMap,
}

impl MaterialKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MaterialKind::Gerber => "gerber",
            MaterialKind::Excellon => "excellon",
            MaterialKind::Stackup => "stackup",
            MaterialKind::LayerMap => "layer_map",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "gerber" => MaterialKind::Gerber,
            "excellon" => MaterialKind::Excellon,
            "stackup" => MaterialKind::Stackup,
            "layer_map" => MaterialKind::LayerMap,
            _ => return None,
        })
    }
}

/// Immutable source material. The verbatim text is the authority; every
/// derived object references a material id and a record span.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Material {
    pub id: String,
    pub filename: String,
    pub kind: MaterialKind,
    pub content: String,
    pub sha256: String,
    pub byte_len: usize,
    /// Event sequence at import time (ordering evidence, never the identity).
    pub imported_seq: i64,
    /// Layer/role hint declared by the file itself, verbatim
    /// (e.g. Gerber `%TF.FileFunction,Copper,L2*%`).
    pub self_declared_role: Option<String>,
}

/// One verbatim source record (line / block) with its stable record id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRecord {
    pub id: String,
    pub seq: usize,
    pub span: Span,
    /// The exact source text of the record.
    pub raw: String,
    /// Structured classification (kept loose so nothing is dropped).
    pub tag: String,
}

/// Painter polarity. Dark deposits copper, clear removes whatever was there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
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

/// Layer roles recognized by the stackup. Other strings are retained
/// verbatim as `Other` instead of being silently reassigned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerRole {
    Copper,
    Dielectric,
    Soldermask,
    Silkscreen,
    BoardProfile,
    Other(String),
}

impl LayerRole {
    pub fn parse(s: &str) -> LayerRole {
        match s.trim().to_ascii_lowercase().as_str() {
            "copper" => LayerRole::Copper,
            "dielectric" | "core" | "prepreg" => LayerRole::Dielectric,
            "soldermask" | "solder_mask" | "mask" => LayerRole::Soldermask,
            "silkscreen" | "legend" => LayerRole::Silkscreen,
            "profile" | "outline" | "board_profile" => LayerRole::BoardProfile,
            other => LayerRole::Other(other.to_string()),
        }
    }
    pub fn as_str(&self) -> &str {
        match self {
            LayerRole::Copper => "copper",
            LayerRole::Dielectric => "dielectric",
            LayerRole::Soldermask => "soldermask",
            LayerRole::Silkscreen => "silkscreen",
            LayerRole::BoardProfile => "board_profile",
            LayerRole::Other(s) => s,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackLayer {
    /// 0-based physical order from top to bottom.
    pub order: usize,
    pub name: String,
    pub role: LayerRole,
    /// Verbatim thickness text from the stackup file (`"0.035mm"`), kept
    /// alongside the normalized value when convertible.
    pub thickness_raw: String,
    pub thickness_nm: Option<i64>,
}

/// A drill's layer extent. Through boards are `full_board`; blind/buried
/// holes name start/end copper layers. Unknown names stay unresolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerSpan {
    pub full_board: bool,
    pub start_layer: Option<String>,
    pub end_layer: Option<String>,
    /// Verbatim directive, e.g. `;SPAN=L1-L3`.
    pub raw: Option<String>,
}

/// One manual filename -> stack layer assignment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerMapEntry {
    pub filename: String,
    pub layer: String,
    pub role_override: Option<String>,
}

/// Injectable clock so replay in tests is deterministic.
pub trait Clock: Send + Sync {
    fn now_micros(&self) -> i64;
}

pub struct SystemClock;
impl Clock for SystemClock {
    fn now_micros(&self) -> i64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_micros() as i64)
            .unwrap_or(0)
    }
}

/// Format nanometres as an exact decimal millimetre string.
pub fn nm_to_mm(nm: i64) -> String {
    let neg = nm < 0;
    let v = nm.unsigned_abs();
    let int = v / 1_000_000;
    let frac = v % 1_000_000;
    format!(
        "{}{}.{:06}",
        if neg { "-" } else { "" },
        int,
        frac
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inch_is_exact_in_nm() {
        assert_eq!(LengthUnit::Inch.nm_per_unit(), 25_400_000);
        assert_eq!(nm_to_mm(25_400_000), "25.400000");
        assert_eq!(nm_to_mm(-1_500), "-0.001500");
    }
}
