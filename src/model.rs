//! Immutable domain model: raw source materials, decisions, computed state.

use serde::{Deserialize, Serialize};

/// Every domain quantity is tagged with its unit or coordinate basis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
    /// Millimetres per one unit.
    pub fn per_unit_mm(self) -> f64 {
        match self {
            LengthUnit::Inch => 25.4,
            LengthUnit::Mm => 1.0,
        }
    }
}

/// Polarity as declared by the source. Only `dark` deposits copper; `clear` removes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Polarity {
    Dark,
    Clear,
}

/// Layer role as declared by the source. `copper_mask` is a solder-mask layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerRole {
    Copper,
    PlatedMask,
    DrillGuide,
    Profile,
    Document,
    Unknown,
}

impl LayerRole {
    pub fn as_str(self) -> &'static str {
        match self {
            LayerRole::Copper => "copper",
            LayerRole::PlatedMask => "solder_mask",
            LayerRole::DrillGuide => "drill_guide",
            LayerRole::Profile => "profile",
            LayerRole::Document => "document",
            LayerRole::Unknown => "unknown",
        }
    }
}

/// The kind of raw file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Gerber,
    Excellon,
    Stackup,
}

/// A quantity that remembers exactly how it was written in the file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dimension {
    pub raw: String,
    pub unit: LengthUnit,
    /// Normalised millimetres; absent when the raw value cannot be converted.
    pub mm: Option<f64>,
}

/// Diagnostic that never lets an unconvertible value disappear.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    /// File position: 1-based line and optional byte/column offset.
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub raw: Option<String>,
    /// How severe: the parser refuses to fabricate geometry for `error`.
    pub severity: String,
}

impl Diagnostic {
    pub fn error(code: impl Into<String>, line: Option<u32>, raw: Option<String>, message: impl Into<String>) -> Self {
        Diagnostic { code: code.into(), message: message.into(), line, column: None, raw, severity: "error".into() }
    }
    pub fn warn(code: impl Into<String>, line: Option<u32>, raw: Option<String>, message: impl Into<String>) -> Self {
        Diagnostic { code: code.into(), message: message.into(), line, column: None, raw, severity: "warning".into() }
    }
}

/// Raw aperture definition, preserved verbatim plus parsed geometry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Aperture {
    pub code: String, // e.g. "D10"
    pub raw: String,  // the entire `%AD...*%` body
    pub shape: String, // C / R / O / P / macro-name
    /// Modifier values in declared units (raw strings retained).
    pub modifiers: Vec<Dimension>,
    pub unit: LengthUnit,
    pub line: u32,
}

/// Step-repeat instruction, preserved as written.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepRepeat {
    pub raw: String,
    pub name: String,
    pub x: i64,
    pub y: i64,
    pub dx: Dimension,
    pub dy: Dimension,
    pub line: u32,
}

/// A single graphics verb, coordinates kept both raw and normalised.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoordPoint {
    pub x_raw: Option<String>,
    pub y_raw: Option<String>,
    pub x_mm: Option<f64>,
    pub y_mm: Option<f64>,
    pub line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GerberShape {
    Flash,
    Segment,
    RegionPoint,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GerberOp {
    pub shape: GerberShape,
    pub aperture: Option<String>,
    pub polarity: Polarity,
    pub step_repeat: Option<String>,
    pub from: CoordPoint,
    pub to: CoordPoint,
    /// Region rings (outer/holes) when `shape == region_point` closes a region.
    pub rings: Option<Vec<Vec<(f64, f64)>>>,
    pub line: u32,
}

/// Parsed Gerber document with every imported attribute retained.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GerberDoc {
    pub format: String,        // FSLA style
    pub coordinate_format: String, // e.g. "3.5"
    pub zero_suppression: String, // L/T/none
    pub unit: Option<LengthUnit>,
    pub unit_command: Option<String>,
    pub apertures: Vec<Aperture>,
    pub ops: Vec<GerberOp>,
    pub step_repeats: Vec<StepRepeat>,
    pub file_attributes: Vec<(String, String)>,
    pub object_attributes: Vec<(String, String, String)>,
    pub diagnostics: Vec<Diagnostic>,
    pub byte_len: u64,
}

/// Excellon tool: diameter plus plating and layer-span hints if declared.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrillTool {
    pub code: String, // T01
    pub diameter: Dimension,
    /// `plated` / `non_plated` / `unknown` — NPTH files are non-plated.
    pub plating: String,
    /// `; LAYER span` marker.
    pub span: Option<(String, String)>,
    pub raw: String,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrillHit {
    pub tool: String,
    pub at: CoordPoint,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExcellonDoc {
    pub format: String, // "excellon-2"
    pub unit: Option<LengthUnit>,
    pub unit_command: Option<String>,
    pub tools: Vec<DrillTool>,
    pub hits: Vec<DrillHit>,
    pub plated_file: bool,
    pub diagnostics: Vec<Diagnostic>,
    pub byte_len: u64,
}

/// Stackup layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StackLayer {
    pub index: u32,
    pub name: String,
    pub role: LayerRole,
    pub thickness: Option<Dimension>,
    pub raw: String,
    pub line: u32,
}

/// Manual mapping between a filename and a stackup layer (or unmapped).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayerMapping {
    pub file: String,
    pub layer: Option<String>,
    pub raw: Option<String>,
    pub line: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StackupDoc {
    pub layers: Vec<StackLayer>,
    pub mappings: Vec<LayerMapping>,
    pub diagnostics: Vec<Diagnostic>,
    pub byte_len: u64,
}

/// Immutable imported material.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawSource {
    pub id: String,
    pub filename: String,
    pub kind: SourceKind,
    pub raw_text: String,
    pub sha256: String,
    pub gerber: Option<GerberDoc>,
    pub excellon: Option<ExcellonDoc>,
    pub stackup: Option<StackupDoc>,
    pub parse_ok: bool,
}

/// Versioned rule/parameter set. Versions are append-only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleParams {
    pub version: u32,
    /// Geometric contact tolerance in millimetres (squared comparisons internally).
    pub contact_tolerance_mm: f64,
    pub max_objects: u64,
    pub max_candidates: u64,
}

impl Default for RuleParams {
    fn default() -> Self {
        RuleParams { version: 1, contact_tolerance_mm: 0.005, max_objects: 20_000, max_candidates: 64 }
    }
}

/// Three distinct failure outcomes required by the specification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ComputeFailure {
    /// Resource budget exhausted: deterministic, retriable with a bigger budget.
    BudgetExhausted { code: String, used: u64, limit: u64, message: String },
    /// Corrupt/undecodable input, with positions.
    FormatCorrupt { diagnostics: Vec<Diagnostic> },
    /// Input parses but has no legal semantic interpretation.
    SemanticUnsolvable { code: String, message: String, diagnostics: Vec<Diagnostic> },
}
