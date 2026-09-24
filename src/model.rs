//! Domain model shared by parsers, geometry engine, storage and the web layer.
//!
//! Immutable material keeps its original representation; derived geometry is
//! normalized to nanometres (`Nano`) and always carries provenance back to
//! the originating file/line/aperture/step-repeat path.

use serde::{Deserialize, Serialize};
use crate::units::Nano;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerSide {
    Top,
    Bottom,
}

/// Electrical / drawing role of a layer. Inner copper layers carry an index.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LayerRole {
    CopperTop,
    CopperInner { index: usize },
    CopperBottom,
    SolderMask { side: LayerSide },
    Legend { side: LayerSide },
    DrillPlated,
    DrillNonPlated,
    Outline,
    Unknown,
}

impl LayerRole {
    pub fn is_copper(&self) -> bool {
        matches!(
            self,
            LayerRole::CopperTop | LayerRole::CopperInner { .. } | LayerRole::CopperBottom
        )
    }
    pub fn copper_order(&self) -> Option<usize> {
        match self {
            LayerRole::CopperTop => Some(0),
            LayerRole::CopperInner { index } => Some(*index),
            LayerRole::CopperBottom => Some(usize::MAX),
            _ => None,
        }
    }
    pub fn tag(&self) -> String {
        match self {
            LayerRole::CopperTop => "copper_top".into(),
            LayerRole::CopperInner { index } => format!("copper_inner_{index}"),
            LayerRole::CopperBottom => "copper_bottom".into(),
            LayerRole::SolderMask { side } => format!("soldermask_{}", side.tag()),
            LayerRole::Legend { side } => format!("legend_{}", side.tag()),
            LayerRole::DrillPlated => "drill_plated".into(),
            LayerRole::DrillNonPlated => "drill_nonplated".into(),
            LayerRole::Outline => "outline".into(),
            LayerRole::Unknown => "unknown".into(),
        }
    }
}

impl LayerSide {
    pub fn tag(self) -> &'static str {
        match self {
            LayerSide::Top => "top",
            LayerSide::Bottom => "bottom",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Polarity {
    /// Gerber D (dark) / positive polarity.
    Dark,
    /// Gerber C (clear) / negative polarity.
    Clear,
}

impl Polarity {
    pub fn tag(self) -> &'static str {
        match self {
            Polarity::Dark => "dark",
            Polarity::Clear => "clear",
        }
    }
}

/// Axis-aligned normalized geometry. Everything is a filled primitive;
/// tracks are rectangular flashes/strokes, pads are circles/rectangles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Primitive {
    Rect { x: Nano, y: Nano, w: Nano, h: Nano },
    Circle { x: Nano, y: Nano, d: Nano },
}

impl Primitive {
    pub fn bbox(&self) -> (Nano, Nano, Nano, Nano) {
        match *self {
            Primitive::Rect { x, y, w, h } => (x, y, x + w, y + h),
            Primitive::Circle { x, y, d } => (x - d / 2, y - d / 2, x + d / 2, y + d / 2),
        }
    }
}

/// Pointer back to the exact original material that produced a derived op.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRef {
    pub file_id: String,
    pub file_role: String,
    pub line: usize,
    pub byte_offset: usize,
    pub raw_text: String,
    pub aperture_raw: Option<String>,
}

/// Path inside an SR (step and repeat) expansion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepRepeatPath {
    pub aperture_block: String,
    pub i: usize,
    pub j: usize,
    pub nx: usize,
    pub ny: usize,
    pub dx_nm: Option<Nano>,
    pub dy_nm: Option<Nano>,
    pub dx_raw: String,
    pub dy_raw: String,
}

/// Original, un-normalized geometry tokens (kept verbatim).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawGeometry {
    pub coords: Vec<String>,
    pub modifiers: Vec<String>,
    pub shape_hint: String,
}

/// One ordered planar operation (dark/clear draw) after step-repeat expansion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanarOp {
    /// Stable op identity derived from file/line/order/step-repeat path.
    pub op_id: String,
    pub seq: usize,
    pub layer_key: String,
    pub polarity: Polarity,
    pub source: SourceRef,
    pub step_repeat: Option<StepRepeatPath>,
    /// `None` when the value cannot be converted: raw tokens + diagnostic remain.
    pub primitive: Option<Primitive>,
    pub raw: RawGeometry,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerInfo {
    pub key: String,
    pub file_id: String,
    pub declared_role: LayerRole,
    pub mapped_role: LayerRole,
    pub role_source: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// Positioned diagnostic; no value is ever silently dropped to make a picture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: Severity,
    pub message: String,
    pub file_id: Option<String>,
    pub line: Option<usize>,
    pub byte_offset: Option<usize>,
    pub raw_token: Option<String>,
}

/// A drill hit from Excellon, normalized when possible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hole {
    pub hole_id: String,
    pub source: SourceRef,
    pub tool: String,
    pub plated: bool,
    /// Layer span expressed as copper-order values: [from, to] inclusive.
    pub span_from: usize,
    pub span_to: usize,
    pub span_label: String,
    pub span_source: String,
    pub x_nm: Option<Nano>,
    pub y_nm: Option<Nano>,
    pub d_nm: Option<Nano>,
    pub raw_x: String,
    pub raw_y: String,
    pub raw_d: String,
}

/// Versioned computation parameters. `contact_tolerance_nm` is the versioned
/// geometric contact tolerance; touching within it is still only a candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Params {
    pub rules_version: String,
    pub contact_tolerance_nm: Nano,
    pub max_cells: usize,
    pub max_candidates: usize,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            rules_version: "rules-1.0.0".into(),
            contact_tolerance_nm: 0,
            max_cells: 200_000,
            max_candidates: 4096,
        }
    }
}

/// Snapshot of everything an evaluation consumes: material + decisions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerMappingEntry {
    pub file_id: String,
    pub layer_key: String,
    pub role: LayerRole,
    pub origin: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManualMapping {
    pub raw: String,
    pub entries: Vec<LayerMappingEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialSet {
    pub files: Vec<MaterialFile>,
    pub ops: Vec<PlanarOp>,
    pub holes: Vec<Hole>,
    pub layers: Vec<LayerInfo>,
    pub mapping: ManualMapping,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialFile {
    pub file_id: String,
    pub path: String,
    pub kind: String,
    pub content: String,
    pub sha256: String,
}
