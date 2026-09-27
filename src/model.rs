//! Core domain model.
//!
//! Every domain quantity travels with its original textual representation,
//! source unit and coordinate basis. Canonical geometry is expressed in
//! nanometres (`Nano`) and is only produced on derived layers; values that
//! cannot be converted stay at their original position together with a
//! positioned diagnostic instead of being replaced by a default.

use serde::{Deserialize, Serialize};

/// Canonical geometry unit: nanometres in the project board frame.
pub type Nano = i64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Mm,
    In,
    Unknown,
}

impl Unit {
    pub fn as_str(self) -> &'static str {
        match self {
            Unit::Mm => "mm",
            Unit::In => "in",
            Unit::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// A diagnostic anchored at an exact source position.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub id: String,
    pub severity: Severity,
    pub code: String,
    pub message: String,
    pub file_id: String,
    pub line_no: usize,
    pub column: usize,
    pub stage: String,
}

/// A scalar length preserving the original representation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Length {
    pub raw: String,
    pub unit: Unit,
    /// Coordinate basis label, e.g. source file frame.
    pub frame: String,
    pub nm: Option<i64>,
    /// Sub-nanometre remainder in picometres (0..1000).
    pub res_pm: i64,
    pub diagnostic_ids: Vec<String>,
}

/// A 2D point preserving per-axis original tokens.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Point {
    pub x_raw: Option<String>,
    pub y_raw: Option<String>,
    pub unit: Unit,
    pub frame: String,
    pub x_nm: Option<i64>,
    pub y_nm: Option<i64>,
    pub x_res_pm: i64,
    pub y_res_pm: i64,
    pub diagnostic_ids: Vec<String>,
}

impl Point {
    pub fn unresolved(&self) -> bool {
        self.x_nm.is_none() || self.y_nm.is_none()
    }
    pub fn xy(&self) -> Option<(f64, f64)> {
        match (self.x_nm, self.y_nm) {
            (Some(x), Some(y)) => Some((x as f64 + self.x_res_pm as f64 / 1000.0,
                                        y as f64 + self.y_res_pm as f64 / 1000.0)),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZeroMode {
    Leading,
    Trailing,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoordinateFormat {
    pub raw: String,
    pub digits_int: u8,
    pub digits_dec: u8,
    pub zero: ZeroMode,
    pub notation: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApertureKind {
    Circle,
    Rect,
    Obround,
    Polygon,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApertureDef {
    pub code: String,
    pub kind: ApertureKind,
    pub raw: String,
    pub line_no: usize,
    pub width: Option<Length>,
    pub height: Option<Length>,
    pub diagnostic_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepRepeatDef {
    pub raw: String,
    pub line_no: usize,
    pub nx: i64,
    pub ny: i64,
    pub dx_nm: Option<i64>,
    pub dy_nm: Option<i64>,
    pub diagnostic_ids: Vec<String>,
}

/// One polarity-ordered paint operation after step-repeat expansion.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PaintOp {
    SetPolarity { line_no: usize, polarity: Polarity },
    Flash {
        line_no: usize,
        aperture: String,
        at: Point,
        /// Step-repeat instance coordinates (ix, iy), 0-based.
        sr_instance: Option<(i64, i64)>,
    },
    Segment {
        line_no: usize,
        aperture: String,
        from: Point,
        to: Point,
        sr_instance: Option<(i64, i64)>,
    },
    Polygon {
        line_no: usize,
        points: Vec<Point>,
        sr_instance: Option<(i64, i64)>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GerberDoc {
    pub file_id: String,
    pub unit: Unit,
    pub format: Option<CoordinateFormat>,
    pub apertures: Vec<ApertureDef>,
    pub step_repeats: Vec<StepRepeatDef>,
    pub ops: Vec<PaintOp>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Plating {
    Plated,
    NonPlated,
    /// Source does not determine plating: multiple legal interpretations.
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrillToolDef {
    pub code: String,
    pub raw: String,
    pub line_no: usize,
    pub diameter: Option<Length>,
    pub plating: Plating,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrillHit {
    pub line_no: usize,
    pub tool: String,
    pub at: Point,
    pub diameter_nm: Option<i64>,
    pub plating: Plating,
    pub range_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerRange {
    pub name: String,
    pub raw: String,
    pub line_no: usize,
    pub from_layer: String,
    pub to_layer: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExcellonDoc {
    pub file_id: String,
    pub unit: Unit,
    pub format_raw: Option<String>,
    pub tools: Vec<DrillToolDef>,
    pub holes: Vec<DrillHit>,
    pub layer_ranges: Vec<LayerRange>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerKind {
    Copper,
    Mask,
    Legend,
    Core,
    Prepreg,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackLayer {
    pub id: String,
    pub order: i64,
    pub kind: LayerKind,
    pub role: String,
    pub file_name: Option<String>,
    pub raw: String,
    pub line_no: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackDrill {
    pub id: String,
    pub file_name: Option<String>,
    pub from_layer: Option<String>,
    pub to_layer: Option<String>,
    pub plating: Plating,
    pub raw: String,
    pub line_no: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackupDoc {
    pub file_id: String,
    pub layers: Vec<StackLayer>,
    pub drills: Vec<StackDrill>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MappingEntry {
    pub file_name: String,
    pub layer_id: String,
    pub raw: String,
    pub line_no: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleAssignment {
    pub layer_id: String,
    pub role: String,
    pub raw: String,
    pub line_no: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MappingDoc {
    pub file_id: String,
    pub maps: Vec<MappingEntry>,
    pub roles: Vec<RoleAssignment>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Gerber,
    Excellon,
    Stackup,
    Mapping,
}

/// Immutable source material revision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceFile {
    pub file_id: String,
    pub revision: i64,
    pub name: String,
    pub kind: SourceKind,
    pub content: String,
    pub sha256: String,
}
