//! Immutable domain model.
//!
//! Everything that originates from an imported file is kept here in its
//! original representation: coordinate format, units, polarity words,
//! aperture definitions, step-repeat blocks, declared layer roles and any
//! manual layer mapping. Normalisation happens only on derived layers
//! (see `engine`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Mm,
    Inch,
}

impl Unit {
    pub fn code(self) -> &'static str {
        match self {
            Unit::Mm => "mm",
            Unit::Inch => "in",
        }
    }
    pub fn from_code(s: &str) -> Option<Unit> {
        match s.to_ascii_lowercase().as_str() {
            "mm" => Some(Unit::Mm),
            "in" | "inch" | "inches" => Some(Unit::Inch),
            _ => None,
        }
    }
    /// Millimetres per one of this unit.
    pub fn to_mm(self) -> f64 {
        match self {
            Unit::Mm => 1.0,
            Unit::Inch => 25.4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterialKind {
    GerberCopper,
    GerberSolderMask,
    Excellon,
    Stackup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Warning,
    Error,
}

/// A diagnostic that always carries a position back into the source material.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diag {
    pub severity: Severity,
    pub code: String,
    /// Stable material id, e.g. `mat_...`.
    pub material_id: String,
    /// 1-based line and (when known) 1-based column in the original file.
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub message: String,
    /// The original token that triggered the diagnostic. Never normalised away.
    pub raw: Option<String>,
}

impl Diag {
    pub fn err(code: impl Into<String>, material_id: &str, line: Option<u32>, message: impl Into<String>, raw: Option<String>) -> Diag {
        Diag { severity: Severity::Error, code: code.into(), material_id: material_id.to_string(), line, column: None, message: message.into(), raw }
    }
    pub fn warn(code: impl Into<String>, material_id: &str, line: Option<u32>, message: impl Into<String>, raw: Option<String>) -> Diag {
        Diag { severity: Severity::Warning, code: code.into(), material_id: material_id.to_string(), line, column: None, message: message.into(), raw }
    }
}

/// A numeric value that keeps the exact source token next to its parse.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawNumber {
    pub raw: String,
    /// Parsed in the *source* unit. `None` means the token could not be
    /// converted; it stays in place with a diagnostic instead of being dropped.
    pub value: Option<f64>,
    pub unit: Unit,
    pub line: u32,
}

impl RawNumber {
    /// Value in millimetres, when convertible.
    pub fn mm(&self) -> Option<f64> {
        self.value.map(|v| v * self.unit.to_mm())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CoordPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Polarity {
    Dark,
    Clear,
}

impl Polarity {
    pub fn code(self) -> &'static str {
        match self {
            Polarity::Dark => "D",
            Polarity::Clear => "C",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApertureShape {
    Circle,
    Rect,
    Obround,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApertureDef {
    /// D-code as used in the file, e.g. `10`.
    pub dcode: u32,
    pub shape: ApertureShape,
    /// Parameters exactly as printed in the `%ADD...*%` command.
    pub raw_params: Vec<String>,
    /// Interpreted dimensions in source units; `None` dimensions are
    /// unconvertible and carry a diagnostic.
    pub dims: Vec<Option<f64>>,
    pub line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZeroSuppression {
    None,
    Leading,
    Trailing,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoordFormat {
    pub int: u32,
    pub dec: u32,
    pub zeros: ZeroSuppression,
    /// `true` when the format came from `%FS...*%`; `false` for a declared
    /// assumption (which then yields a diagnostic).
    pub declared: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpKind {
    Flash,
    Line,
    Arc,
    Region,
}

/// One graphical operation in strict file order. Polarity is carried
/// explicitly so the positive/negative stacking order is replayable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GerberOp {
    pub seq: u32,
    pub kind: OpKind,
    pub dcode: Option<u32>,
    pub polarity: Polarity,
    /// Coordinates kept in the source coordinate format/unit.
    pub points: Vec<CoordPoint>,
    /// Whether every coordinate converted successfully.
    pub convertible: bool,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepRepeat {
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub nx: u32,
    pub ny: u32,
    pub dx: f64,
    pub dy: f64,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GerberFile {
    pub material_id: String,
    pub filename: String,
    pub unit: Unit,
    pub format: Option<CoordFormat>,
    pub apertures: Vec<ApertureDef>,
    pub step_repeats: Vec<StepRepeat>,
    pub active_sr: Option<String>,
    pub ops: Vec<GerberOp>,
    pub role_declared: Option<String>,
    pub layer_name: Option<String>,
    pub raw_text: String,
    pub diags: Vec<Diag>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Plating {
    Plated,
    NonPlated,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrillHit {
    pub tool: String,
    pub x: RawNumber,
    pub y: RawNumber,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrillTool {
    pub tool: String,
    /// Diameter in source unit; `None` when the original token was unparseable.
    pub diameter: Option<f64>,
    pub raw_diameter: String,
    pub plating: Plating,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExcellonFile {
    pub material_id: String,
    pub filename: String,
    pub unit: Unit,
    /// Explicit layer span statements taken from the file comments, e.g.
    /// `;LAYER_SPAN:2..4`. Blind/buried range originates here and is preserved.
    pub span_from: Option<u32>,
    pub span_to: Option<u32>,
    pub tools: Vec<DrillTool>,
    pub hits: Vec<DrillHit>,
    pub raw_text: String,
    pub diags: Vec<Diag>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackLayer {
    /// Order as declared in the stackup file (1 = top).
    pub order: u32,
    /// Original declared role token, e.g. `copper`, `prepreg`.
    pub role_declared: String,
    pub name: String,
    /// Filename to which this layer was mapped manually in the stackup file.
    pub mapped_file: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StackupFile {
    pub material_id: String,
    pub filename: String,
    pub layers: Vec<StackLayer>,
    /// Coordinate basis statement as authored (`mm` / `inch`), retained raw.
    pub coord_basis_raw: String,
    pub raw_text: String,
    pub diags: Vec<Diag>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ParsedMaterial {
    Gerber(GerberFile),
    Excellon(ExcellonFile),
    Stackup(StackupFile),
}

impl ParsedMaterial {
    pub fn material_id(&self) -> &str {
        match self {
            ParsedMaterial::Gerber(g) => &g.material_id,
            ParsedMaterial::Excellon(e) => &e.material_id,
            ParsedMaterial::Stackup(s) => &s.material_id,
        }
    }
    pub fn filename(&self) -> &str {
        match self {
            ParsedMaterial::Gerber(g) => &g.filename,
            ParsedMaterial::Excellon(e) => &e.filename,
            ParsedMaterial::Stackup(s) => &s.filename,
        }
    }
    pub fn raw_text(&self) -> &str {
        match self {
            ParsedMaterial::Gerber(g) => &g.raw_text,
            ParsedMaterial::Excellon(e) => &e.raw_text,
            ParsedMaterial::Stackup(s) => &s.raw_text,
        }
    }
    pub fn diags(&self) -> &[Diag] {
        match self {
            ParsedMaterial::Gerber(g) => &g.diags,
            ParsedMaterial::Excellon(e) => &e.diags,
            ParsedMaterial::Stackup(s) => &s.diags,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IngestOutcome {
    Parsed,
    /// File structure was damaged; the raw text is still retained.
    Corrupt,
}

/// A material is an immutable source document. Its id is content-derived, so
/// identical inputs produce identical identities and stable ordering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Material {
    pub id: String,
    pub filename: String,
    pub kind: MaterialKind,
    pub raw_text: String,
    pub imported_at: String,
    pub outcome: IngestOutcome,
    pub parsed: Option<ParsedMaterial>,
    pub diags: Vec<Diag>,
    /// Content hash of the raw bytes.
    pub sha256: String,
}
