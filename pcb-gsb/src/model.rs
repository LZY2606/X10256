//! Core domain model.
//!
//! All physical quantities carry an explicit unit / coordinate datum.
//! Materials are immutable; decisions and derived results are separate layers.

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

/// Coordinate datum for a source document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoordDatum {
    /// Display unit declared in the source (`inch`, `mm`).
    pub unit: String,
    /// Coordinate format as written in the source, e.g. "2.5" / "3.3".
    pub format: String,
    /// Zero suppression used by the source: `L` leading, `T` trailing, `N`.
    pub zero_suppression: String,
    /// Absolute (`A`) or incremental (`I`) notation.
    pub notation: String,
    /// Internal normalized scale: 1 source unit = `unit_nm` nanometres.
    pub unit_nm: i64,
}

impl Default for CoordDatum {
    fn default() -> Self {
        CoordDatum {
            unit: "unknown".into(),
            format: "unknown".into(),
            zero_suppression: "N".into(),
            notation: "unknown".into(),
            unit_nm: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawNumber {
    /// Exactly as written in the source file.
    pub raw: String,
    /// Interpreted value expressed in the source display unit (inch/mm),
    /// `None` when interpretation is impossible (left in place + diagnosed).
    pub value_nm: Option<i64>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub x: RawNumber,
    pub y: RawNumber,
}

/// Polarity as declared per image / block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Polarity {
    Dark,
    Clear,
}

impl Polarity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Polarity::Dark => "dark",
            Polarity::Clear => "clear",
        }
    }
}

/// Layer role. `Unknown` is a first-class value, never silently promoted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum LayerRole {
    Copper,
    SolderMask,
    Legend,
    Drill,
    Paste,
    Outline,
    Mechanical,
    Unknown,
}

impl LayerRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            LayerRole::Copper => "copper",
            LayerRole::SolderMask => "solder_mask",
            LayerRole::Legend => "legend",
            LayerRole::Drill => "drill",
            LayerRole::Paste => "paste",
            LayerRole::Outline => "outline",
            LayerRole::Mechanical => "mechanical",
            LayerRole::Unknown => "unknown",
        }
    }
    pub fn parse(s: &str) -> LayerRole {
        match s.trim().to_ascii_lowercase().as_str() {
            "copper" | "cu" => LayerRole::Copper,
            "solder_mask" | "soldermask" | "sm" | "mask" => LayerRole::SolderMask,
            "legend" | "silkscreen" => LayerRole::Legend,
            "drill" | "drillmap" => LayerRole::Drill,
            "paste" | "stencil" => LayerRole::Paste,
            "outline" | "profile" => LayerRole::Outline,
            "mechanical" | "mech" => LayerRole::Mechanical,
            _ => LayerRole::Unknown,
        }
    }
}

/// Aperture definition, preserved verbatim plus a normalized primitive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApertureDef {
    pub code: String,
    pub primitive: String,
    /// Parameters exactly as written.
    pub params_raw: Vec<String>,
    /// Parameters normalized to nm where interpretable; `None` entries stay raw.
    pub params_nm: Vec<Option<i64>>,
}

/// Step & repeat block declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepRepeat {
    pub name: String,
    pub nx: u32,
    pub ny: u32,
    pub dx_nm: Option<i64>,
    pub dy_nm: Option<i64>,
    pub dx_raw: String,
    pub dy_raw: String,
}

/// A single graphics operation as written between polarity/dark-clear changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrawOp {
    pub seq: u32,
    pub aperture: String,
    pub polarity: Polarity,
    /// D01 flash / D02 move / D03 flash alternate / line.
    pub kind: String,
    pub from: Position,
    pub to: Position,
    /// `Some` when produced by step & repeat expansion.
    pub repeat: Option<(u32, u32)>,
    pub block: Option<String>,
    pub source_lines: (u32, u32),
}

/// Flat aperture primitive usable by geometry normalization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum Shape {
    Circle { d_nm: Option<i64>, d_raw: String },
    Rect { w_nm: Option<i64>, h_nm: Option<i64>, w_raw: String, h_raw: String },
    Obround { w_nm: Option<i64>, h_nm: Option<i64>, w_raw: String, h_raw: String },
    Unknown { raw: String },
}

/// Parsed Gerber document (immutable material layer).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GerberDoc {
    pub datum: CoordDatum,
    pub role: LayerRole,
    pub apertures: Vec<ApertureDef>,
    pub step_repeats: Vec<StepRepeat>,
    pub ops: Vec<DrawOp>,
    pub warnings: Vec<Diagnostic>,
}

/// Drill tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrillTool {
    pub code: String,
    pub diameter_raw: String,
    pub diameter_nm: Option<i64>,
    pub plated: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrillHit {
    pub seq: u32,
    pub tool: String,
    pub pos: Position,
    pub source_lines: (u32, u32),
}

/// Parsed Excellon document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExcellonDoc {
    pub datum: CoordDatum,
    pub tools: Vec<DrillTool>,
    pub hits: Vec<DrillHit>,
    /// Layer span declared in the file header comments.
    pub span: LayerSpan,
    pub warnings: Vec<Diagnostic>,
}

/// Inclusive physical layer range. `start`/`end` index into the stackup (0 based).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerSpan {
    pub start: Option<u32>,
    pub end: Option<u32>,
    /// Raw declaration kept verbatim, e.g. "L2-L4".
    pub raw: String,
    pub note: Option<String>,
}

impl Default for LayerSpan {
    fn default() -> Self {
        LayerSpan { start: None, end: None, raw: "".into(), note: Some("through-all (no declaration)".into()) }
    }
}

/// One layer in the stackup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackLayer {
    pub index: u32,
    pub name: String,
    pub kind: String,
    pub thickness_raw: String,
    pub thickness_nm: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackupDoc {
    pub layers: Vec<StackLayer>,
    pub warnings: Vec<Diagnostic>,
}

/// Manual mapping of a source file to a physical layer index (a decision layer,
/// but the *declared* value is preserved in the material; only mapping edits
/// generate decision events).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerMapping {
    pub material_id: String,
    pub filename: String,
    pub physical_index: Option<u32>,
    pub raw_declaration: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MaterialDoc {
    Gerber(GerberDoc),
    Excellon(ExcellonDoc),
    Stackup(StackupDoc),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Material {
    pub id: String,
    pub filename: String,
    pub doc: MaterialDoc,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    /// 1-based line range in the source file.
    pub line_from: u32,
    pub line_to: u32,
    pub severity: Severity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

// ---------------------------------------------------------------------------
// Derived geometry
// ---------------------------------------------------------------------------

/// Normalized feature in nanometre board coordinates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Feature {
    pub id: String,
    pub material_id: String,
    pub layer_index: Option<u32>,
    pub polarity: Polarity,
    pub shape: NormShape,
    /// Ordered op sequence provenance.
    pub op_seq: u32,
    pub repeat: Option<(u32, u32)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum NormShape {
    Circle { cx: i64, cy: i64, r: Option<i64>, r_raw: String },
    Rect { x0: i64, y0: i64, x1: i64, y1: i64, r_raw: String },
    Line { x0: i64, y0: i64, x1: i64, y1: i64, width: Option<i64>, width_raw: String },
    /// Could not be normalized; carries the original shape text and stays put.
    Unresolved { raw: String, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hole {
    pub id: String,
    pub material_id: String,
    pub cx: i64,
    pub cy: i64,
    pub r_nm: Option<i64>,
    pub r_raw: String,
    pub plated: bool,
    pub span_start: Option<u32>,
    pub span_end: Option<u32>,
    pub span_raw: String,
    pub tool: String,
}

// ---------------------------------------------------------------------------
// Connectivity
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContactKind {
    /// Closed-shape boundary overlap with zero clearance (touch at edge).
    BoundaryTangent,
    /// Distance > 0 but <= contact tolerance: ambiguous without netlist.
    WithinTolerance,
    /// Plated hole ties copper on its span layers.
    PlatedHole,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contact {
    pub a: String,
    pub b: String,
    pub kind: ContactKind,
    /// Signed gap in nm: 0 tangent, positive for near-miss.
    pub gap_nm: i64,
    /// Layer index where the contact occurs (hole contacts recorded per layer).
    pub layer_index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetCandidateSet {
    /// Legal groupings of feature ids when contact evidence is ambiguous.
    pub alternatives: Vec<Vec<String>>,
    /// Contacts that the alternatives hinge on.
    pub contacts: Vec<Contact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectedComponent {
    pub id: String,
    pub feature_ids: Vec<String>,
    pub hole_ids: Vec<String>,
    /// `None` for hard-closed components; `Some` lists legal alternatives.
    pub ambiguity: Option<NetCandidateSet>,
    pub layers: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contradiction {
    pub id: String,
    pub code: String,
    pub message: String,
    pub feature_ids: Vec<String>,
    pub x_nm: Option<i64>,
    pub y_nm: Option<i64>,
}

/// Versioned geometry/contact parameters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParamVersion {
    pub version: u32,
    pub contact_tol_nm: i64,
    pub min_annular_nm: i64,
    /// 0 means use the engine default; otherwise caps pairwise evaluation.
    #[serde(default)]
    pub pair_budget: u64,
    pub note: String,
}

impl Default for ParamVersion {
    fn default() -> Self {
        ParamVersion { version: 0, contact_tol_nm: 0, min_annular_nm: 0, pair_budget: 0, note: "initial".into() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysisResult {
    pub features: Vec<Feature>,
    pub holes: Vec<Hole>,
    pub components: Vec<ConnectedComponent>,
    pub contradictions: Vec<Contradiction>,
    pub unresolved: Vec<Diagnostic>,
    pub params: ParamVersion,
    pub input_summary: String,
    pub parent_materials: Vec<String>,
    pub parent_decisions: Vec<String>,
}
