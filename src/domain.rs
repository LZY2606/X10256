//! Core domain model.
//!
//! Every imported value keeps its *raw representation* (coordinate format,
//! digits, unit, polarity, aperture, step/repeat, layer role and the manual
//! layer map). Normalisation to a common coordinate basis happens only on
//! derived layers; values that cannot be converted stay in place and carry a
//! diagnostic instead of being replaced by a default.

use serde::{Deserialize, Serialize};

/// File/coordinate unit. All physical quantities on derived layers carry one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
    pub fn parse(s: &str) -> Option<Unit> {
        match s.trim().to_ascii_lowercase().as_str() {
            "mm" => Some(Unit::Mm),
            "in" | "inch" | "inches" => Some(Unit::In),
            _ => None,
        }
    }
}

/// Gerber zero suppression as declared by the FS command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZeroSuppression {
    None,
    Leading,
    Trailing,
}

/// Coordinate format declaration (`%FSLAX...Y...%` / Excellon units+format).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoordFormat {
    pub unit: Unit,
    pub zero: ZeroSuppression,
    pub x_int: u8,
    pub x_frac: u8,
    pub y_int: u8,
    pub y_frac: u8,
}

impl CoordFormat {
    /// Integer digits expected after the decimal for a given axis.
    pub fn frac_digits(&self, axis: Axis) -> u8 {
        match axis {
            Axis::X => self.x_frac,
            Axis::Y => self.y_frac,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Axis {
    X,
    Y,
}

/// Polarity of a Gerber image step. Order matters: dark then light clears.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Polarity {
    Dark,
    Clear,
}

/// Positional diagnostic. Never silently replaces the original value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: Severity,
    pub line: usize,
    pub col: usize,
    pub message: String,
    /// Offending token / value kept verbatim for forensic review.
    pub raw: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Warning,
    Error,
}

/// A coordinate exactly as written in the file plus its conversion (if any).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawCoord {
    pub axis: Axis,
    pub raw: String,
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Gerber,
    Excellon,
    Stackup,
    HandMap,
}

/// Immutable source material. The bytes and every declared attribute are
/// retained; derived data never overwrites this record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceFile {
    pub id: String,
    pub kind: SourceKind,
    pub filename: String,
    pub media_type: String,
    pub content: String,
    pub sha256: String,
    pub received_at: String,
}

/// Gerber aperture macro-free primitives we reconstruct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "lowercase")]
pub enum ApertureShape {
    Circle { diameter_nm: i64, hole_nm: Option<i64> },
    Rect { x_nm: i64, y_nm: i64, hole_nm: Option<i64> },
    Obround { x_nm: i64, y_nm: i64, hole_nm: Option<i64> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Aperture {
    pub code: String,
    pub shape: ApertureShape,
    /// Raw modifier string (`1.50/0.30`) retained from the AD command.
    pub modifiers_raw: String,
    pub line: usize,
}

/// Step-and-repeat block declaration (`%SR...%`), expanded only on derive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepRepeat {
    pub id: String,
    pub x_count: i64,
    pub y_count: i64,
    pub dx_raw: String,
    pub dy_raw: String,
    pub dx_nm: i64,
    pub dy_nm: i64,
    pub line: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DrawOp {
    Flash,
    Draw,
    Move,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlashDraw {
    pub op: DrawOp,
    pub aperture: String,
    pub x: Option<RawCoord>,
    pub y: Option<RawCoord>,
    pub x_nm: Option<i64>,
    pub y_nm: Option<i64>,
    /// Segment start for `Draw` operations (resolved common basis).
    pub from_x_nm: Option<i64>,
    pub from_y_nm: Option<i64>,
    pub line: usize,
    /// 0 = original, 1.. = occurrence index inside an SR expansion.
    pub sr_instance: i64,
    pub sr_id: Option<String>,
}

/// One image-polarity/step-repeat block in source order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GerberBlock {
    pub index: usize,
    pub polarity: Polarity,
    pub polarity_line: usize,
    pub step_repeat: Option<StepRepeat>,
    pub flashes: Vec<FlashDraw>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GerberDoc {
    pub source_id: String,
    pub format: Option<CoordFormat>,
    pub format_line: usize,
    pub unit_line: usize,
    pub apertures: Vec<Aperture>,
    pub blocks: Vec<GerberBlock>,
    pub file_attributes: Vec<(String, String)>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Plating {
    Pth,
    Npth,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrillTool {
    pub code: String,
    pub diameter_raw: String,
    pub diameter_nm: Option<i64>,
    pub plating: Plating,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrillHit {
    pub tool: String,
    pub x: RawCoord,
    pub y: RawCoord,
    pub x_nm: Option<i64>,
    pub y_nm: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExcellonDoc {
    pub source_id: String,
    pub unit: Option<Unit>,
    pub format: Option<CoordFormat>,
    pub tools: Vec<DrillTool>,
    pub hits: Vec<DrillHit>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Declared role of a conductive/physical layer in the stackup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerRole {
    Signal,
    Plane,
    Mask,
    Paste,
    Silk,
    Substrate,
    DrillSpan,
}

impl LayerRole {
    pub fn parse(s: &str) -> Option<LayerRole> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "signal" | "copper" => LayerRole::Signal,
            "plane" => LayerRole::Plane,
            "mask" | "soldermask" | "solder_mask" => LayerRole::Mask,
            "paste" => LayerRole::Paste,
            "silk" | "silkscreen" => LayerRole::Silk,
            "substrate" | "core" | "prepreg" | "dielectric" => LayerRole::Substrate,
            "drill_span" | "drillspan" | "drill" => LayerRole::DrillSpan,
            _ => return None,
        })
    }
    pub fn is_copper(self) -> bool {
        matches!(self, LayerRole::Signal | LayerRole::Plane)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackLayer {
    pub order: i64,
    pub name: String,
    pub role: LayerRole,
    pub role_raw: String,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrillSpanDecl {
    pub name: String,
    pub first_layer: String,
    pub last_layer: String,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackupDoc {
    pub source_id: String,
    pub unit: Option<Unit>,
    pub layers: Vec<StackLayer>,
    pub drill_spans: Vec<DrillSpanDecl>,
    pub diagnostics: Vec<Diagnostic>,
}

/// The immutable manual layer mapping (file -> declared layer name).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandMappingEntry {
    pub filename: String,
    pub layer: String,
    pub role_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandMapDoc {
    pub source_id: String,
    pub entries: Vec<HandMappingEntry>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Bundle of parsed source documents.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialBundle {
    pub sources: Vec<SourceFile>,
    pub gerbers: Vec<GerberDoc>,
    pub drills: Vec<ExcellonDoc>,
    pub stackups: Vec<StackupDoc>,
    pub handmaps: Vec<HandMapDoc>,
}
