//! Core domain model. Everything that describes source material carries its
//! original textual/coordinate representation and a coordinate/unit datum.

use serde::{Deserialize, Serialize};

/// Supported length units. We never invent a unit: source files must declare
/// one, otherwise the value stays in place with a diagnostic (see `Diagnostic`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum LengthUnit {
    Inch,
    Mm,
    /// Dimensionless Gerber coordinates expressed in user digits (never
    /// silently assumed to be inch or mm).
    User,
}

impl LengthUnit {
    pub fn as_str(self) -> &'static str {
        match self {
            LengthUnit::Inch => "inch",
            LengthUnit::Mm => "mm",
            LengthUnit::User => "user",
        }
    }
}

/// Coordinate datum shared by a whole document. Kept verbatim from the source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoordinateDatum {
    /// `inch` | `mm` | `user` — exactly as declared by the file.
    pub unit: LengthUnit,
    /// Integer digits / decimal digits for Gerber, format string for drill.
    pub format: String,
    /// Zero suppression: `leading`, `trailing`, `none`.
    pub zero_suppression: String,
    /// Absolute or incremental coordinates as declared.
    pub notation: String,
}

impl Default for CoordinateDatum {
    fn default() -> Self {
        CoordinateDatum {
            unit: LengthUnit::User,
            format: String::new(),
            zero_suppression: "none".into(),
            notation: "absolute".into(),
        }
    }
}

/// A numeric value kept both as the exact source token and (when convertible)
/// a normalized millimetre value. Unconvertible values are never dropped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dim {
    pub raw: String,
    pub unit: LengthUnit,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mm: Option<f64>,
}

impl Dim {
    pub fn new(raw: impl Into<String>, unit: LengthUnit, mm: Option<f64>) -> Self {
        Dim { raw: raw.into(), unit, mm }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
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

/// Layer role declared by the material stackup or inferred from a file name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum LayerRole {
    Copper,
    SolderMask,
    Legend,
    Drill,
    Plate,
    Dielectric,
    Unknown,
}

impl LayerRole {
    pub fn as_str(self) -> &'static str {
        match self {
            LayerRole::Copper => "copper",
            LayerRole::SolderMask => "solder_mask",
            LayerRole::Legend => "legend",
            LayerRole::Drill => "drill",
            LayerRole::Plate => "plate",
            LayerRole::Dielectric => "dielectric",
            LayerRole::Unknown => "unknown",
        }
    }
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "copper" | "cu" => LayerRole::Copper,
            "soldermask" | "solder_mask" | "sm" => LayerRole::SolderMask,
            "legend" | "silkscreen" | "ss" => LayerRole::Legend,
            "drill" | "drillmap" => LayerRole::Drill,
            "plate" | "plating" => LayerRole::Plate,
            "dielectric" | "core" | "prepreg" => LayerRole::Dielectric,
            _ => LayerRole::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApertureDef {
    pub code: String,
    pub shape: String,
    pub params_raw: Vec<String>,
    pub params_mm: Vec<f64>,
}

/// A single drawing operation, stored in source order with its active state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrawOp {
    pub seq: usize,
    pub aperture: String,
    pub polarity: Polarity,
    pub kind: String, // line | flash | region
    pub points_raw: Vec<(String, String)>,
    pub points_mm: Vec<(f64, f64)>,
    /// Result of SR (step-repeat) expansion marker.
    pub repeat_ref: Option<usize>,
}

/// Parsed Gerber document. The raw text is retained on the enclosing
/// `SourceFile`; here every field records its original representation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GerberDoc {
    pub datum: CoordinateDatum,
    pub apertures: Vec<ApertureDef>,
    pub ops: Vec<DrawOp>,
    /// Step-repeat blocks as declared: `(id, nx, ny, dx_raw, dy_raw, dx_mm, dy_mm, op_seqs)`.
    pub step_repeats: Vec<StepRepeat>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepRepeat {
    pub id: usize,
    pub nx: usize,
    pub ny: usize,
    pub dx: Dim,
    pub dy: Dim,
    pub op_seqs: Vec<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Plating {
    Plated,
    NonPlated,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrillHit {
    pub seq: usize,
    pub tool: String,
    pub diameter: Dim,
    pub x_raw: String,
    pub y_raw: String,
    pub x_mm: Option<f64>,
    pub y_mm: Option<f64>,
    pub plating: Plating,
    /// First/last copper layer indices (inclusive). `None` = through-hole.
    pub span_start: Option<usize>,
    pub span_end: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExcellonDoc {
    pub datum: CoordinateDatum,
    pub tools: Vec<(String, Dim, Plating, Option<(usize, usize)>)>,
    pub hits: Vec<DrillHit>,
}

/// A layer line from the stackup description, kept verbatim plus parsed fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StackLayer {
    pub line_no: usize,
    pub raw: String,
    pub index: usize,
    pub name: String,
    pub role: LayerRole,
    pub thickness: Option<Dim>,
    /// File this layer is manually mapped to (raw token).
    pub mapped_file: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StackupDoc {
    pub layers: Vec<StackLayer>,
}

/// Severity + exact location for anything that could not be fully converted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: String, // error | warning
    pub file: String,
    /// 1-based line, 0-based column/offset where meaningful.
    pub line: usize,
    pub column: usize,
    pub message: String,
    pub token: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocKind {
    Gerber,
    Excellon,
    Stackup,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceFile {
    pub name: String,
    pub kind: DocKind,
    /// Original bytes/text exactly as imported.
    pub raw_text: String,
}

/// A parsed bundle of one import: source text is immutable; parsed docs carry
/// original representations; diagnostics never block retention of raw input.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ParsedBundle {
    pub files: Vec<SourceFile>,
    pub gerbers: Vec<(String, GerberDoc)>,
    pub drills: Vec<(String, ExcellonDoc)>,
    pub stackups: Vec<(String, StackupDoc)>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Distinct terminal outcomes required by the specification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    /// Parsed & computed; may still carry warnings.
    Ok,
    /// Resource budget (segments/flashes/candidates) exhausted.
    BudgetExhausted { budget: String, used: usize, limit: usize, at: String },
    /// Lexical/structural corruption: file cannot be parsed.
    FormatCorrupt { diagnostics: Vec<Diagnostic> },
    /// Well-formed but no legal interpretation (e.g. blind span outside stack).
    SemanticUnresolved { diagnostics: Vec<Diagnostic> },
}

impl Outcome {
    pub fn is_ok(&self) -> bool {
        matches!(self, Outcome::Ok)
    }
}
