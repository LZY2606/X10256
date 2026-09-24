//! Domain types.
//!
//! Layering rule implemented here:
//! * `RawMaterial` is immutable and preserves the original textual
//!   representation, coordinate format, units, polarity, apertures,
//!   step/repeat directives, declared layer role and manual layer mapping.
//! * Derived/normalized values live in engine outputs only.
//! * Values that cannot be converted stay where they were in the raw material
//!   and carry a positioned diagnostic instead of being silently dropped.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::identity::Id;

/* ----------------------------- units & frames ---------------------------- */

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LengthUnit {
    #[serde(rename = "mm")]
    Mm,
    #[serde(rename = "in")]
    Inch,
}

impl LengthUnit {
    pub fn as_str(&self) -> &'static str {
        match self {
            LengthUnit::Mm => "mm",
            LengthUnit::Inch => "in",
        }
    }
    /// Conversion factor to millimetres.
    pub fn to_mm(&self) -> f64 {
        match self {
            LengthUnit::Mm => 1.0,
            LengthUnit::Inch => 25.4,
        }
    }
    /// Convert a value expressed in this unit into millimetres.
    pub fn mm(&self, value: f64) -> f64 {
        value * self.to_mm()
    }
}

/// How the raw numeric coordinate text maps to a length (e.g. 2.5 format,
/// explicit decimals). Kept verbatim for provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoordFormat {
    /// Gerber FS spec as written, e.g. `FSLAX36` / `FSLAX25`.
    pub fs_spec: String,
    pub integer_digits: u32,
    pub decimal_digits: u32,
    /// Excellon units may be declared; `None` when the file never says.
    pub declared_unit: Option<LengthUnit>,
    /// Leading/trailing zero handling as declared.
    pub zero_mode: String,
}

/// Origin / coordinate basis recorded for a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoordinateBasis {
    /// `L` (left/leading) or `R`, copied from the FS spec.
    pub x_origin_side: String,
    pub y_origin_side: String,
    /// Explicit coordinate offset applied via Gerber OF or equivalent, mm.
    pub offset_mm: (String, String),
    /// Axis selection (usually XY).
    pub axes: String,
}

/// Every domain length shown in the UI carries its unit + coordinate basis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DomainValue {
    pub id: Id,
    pub raw: String,
    pub value_mm: Option<f64>,
    pub source_unit: Option<LengthUnit>,
    pub basis: String,
}

/* ------------------------------ raw materials ----------------------------- */

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MaterialKind {
    Gerber,
    Excellon,
    Stackup,
}

impl MaterialKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            MaterialKind::Gerber => "gerber",
            MaterialKind::Excellon => "excellon",
            MaterialKind::Stackup => "stackup",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LayerRole {
    /// Copper signal/plane layer.
    Copper,
    /// Solder mask opening layer.
    SolderMask,
    /// Silkscreen (non-conductive, rendered but excluded from nets).
    Silkscreen,
    /// Drill/rout mechanical layer.
    Mechanical,
    /// Paste layer.
    Paste,
    /// Document/outline layer.
    Outline,
    /// Role declared but not mapped to anything conductive.
    Other,
}

impl LayerRole {
    pub fn key(&self) -> &'static str {
        match self {
            LayerRole::Copper => "copper",
            LayerRole::SolderMask => "solder_mask",
            LayerRole::Silkscreen => "silkscreen",
            LayerRole::Mechanical => "mechanical",
            LayerRole::Paste => "paste",
            LayerRole::Outline => "outline",
            LayerRole::Other => "other",
        }
    }
    pub fn conductive(&self) -> bool {
        matches!(self, LayerRole::Copper)
    }
}

/// A diagnostic that keeps an unconvertible value at its original position.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub line_no: usize,
    pub col: usize,
    /// Reference to the exact raw fragment, kept verbatim.
    pub raw_fragment: String,
    pub message: String,
}

/// Immutable imported source file. Parsed structure is kept beside the verbatim
/// text; nothing here is ever overwritten by a later decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawMaterial {
    pub id: Id,
    pub filename: String,
    pub kind: MaterialKind,
    pub content: String,
    pub imported_seq: u64,
    pub parse: ParsedMaterial,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ParsedMaterial {
    Gerber(ParsedGerber),
    Excellon(ParsedExcellon),
    Stackup(ParsedStackup),
}

impl ParsedMaterial {
    pub fn kind(&self) -> MaterialKind {
        match self {
            ParsedMaterial::Gerber(_) => MaterialKind::Gerber,
            ParsedMaterial::Excellon(_) => MaterialKind::Excellon,
            ParsedMaterial::Stackup(_) => MaterialKind::Stackup,
        }
    }
}

/* --------------------------------- Gerber --------------------------------- */

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Polarity {
    Dark,
    Clear,
}

impl Polarity {
    pub fn key(&self) -> &'static str {
        match self {
            Polarity::Dark => "dark",
            Polarity::Clear => "clear",
        }
    }
}

/// Aperture definition preserved in original order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Aperture {
    pub dcode: String,
    /// e.g. `C`, `R`, `O`, `P`.
    pub shape: String,
    /// Modifiers exactly as written.
    pub modifiers_raw: Vec<String>,
    pub modifiers_mm: Vec<Option<f64>>,
    pub source_unit: LengthUnit,
    pub definition_line: usize,
}

/// Step & Repeat directive preserved verbatim (we support a deterministic
/// subset and record unsupported rows without dropping them).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepRepeat {
    pub raw: String,
    pub line_no: usize,
    pub name: String,
    pub x_count: u32,
    pub y_count: u32,
    pub x_step_raw: String,
    pub y_step_raw: String,
    pub x_step_mm: Option<f64>,
    pub y_step_mm: Option<f64>,
}

/// One draw operation in the exact order it occurs. Polarity and step/repeat
/// expansion are recorded against it so polarity-over-op-order is replayable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GerberDraw {
    pub seq: usize,
    pub line_no: usize,
    pub raw: String,
    pub dcode: String,
    pub polarity: Polarity,
    /// Active step/repeat name at the time of the draw (`None` = base).
    pub step_repeat: Option<String>,
    pub op: DrawOp,
    /// True when this draw could not be converted and must stay out of the
    /// geometric model but remain visible in provenance.
    pub valid: bool,
    pub diagnostic: Option<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DrawOp {
    Flash {
        x_raw: String,
        y_raw: String,
        x_mm: Option<f64>,
        y_mm: Option<f64>,
    },
    Line {
        x1_raw: String,
        y1_raw: String,
        x2_raw: String,
        y2_raw: String,
        x1_mm: Option<f64>,
        y1_mm: Option<f64>,
        x2_mm: Option<f64>,
        y2_mm: Option<f64>,
    },
    /// Filled region contour (simplified closed polygon subset).
    Region {
        pts_raw: Vec<(String, String)>,
        pts_mm: Vec<Option<(f64, f64)>>,
    },
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedGerber {
    pub coord_format: CoordFormat,
    pub basis: CoordinateBasis,
    pub declared_role: Option<LayerRole>,
    /// File-level function attribute as written (e.g. `Copper,L1,Top`).
    pub file_function_raw: Option<String>,
    pub apertures: Vec<Aperture>,
    pub step_repeats: Vec<StepRepeat>,
    pub draws: Vec<GerberDraw>,
    /// Layer polarity as declared (MOIN/MOIP preserved separately here).
    pub image_polarity: String,
    pub diagnostics: Vec<Diagnostic>,
}

/* -------------------------------- Excellon -------------------------------- */

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Plating {
    Plated,
    NonPlated,
    /// Default when the file does not declare plating; never silently assumed
    /// conductive.
    Unknown,
}

impl Plating {
    pub fn key(&self) -> &'static str {
        match self {
            Plating::Plated => "plated",
            Plating::NonPlated => "non_plated",
            Plating::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDef {
    pub tcode: String,
    pub diameter_raw: String,
    pub diameter_mm: Option<f64>,
    pub source_unit: Option<LengthUnit>,
    pub plating: Plating,
    pub line_no: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrillHit {
    pub seq: usize,
    pub line_no: usize,
    pub raw: String,
    pub tcode: String,
    pub x_raw: String,
    pub y_raw: String,
    pub x_mm: Option<f64>,
    pub y_mm: Option<f64>,
    /// Blind/buried span as declared by a routed `LAYER`/span comment.
    pub span_start: Option<i32>,
    pub span_end: Option<i32>,
    pub valid: bool,
    pub diagnostic: Option<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedExcellon {
    pub coord_format: CoordFormat,
    pub basis: CoordinateBasis,
    pub tools: Vec<ToolDef>,
    pub hits: Vec<DrillHit>,
    pub diagnostics: Vec<Diagnostic>,
}

/* --------------------------------- Stackup -------------------------------- */

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StackLayer {
    pub order: i32,
    pub name: String,
    pub role: LayerRole,
    pub thickness_raw: String,
    pub thickness_mm: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManualMapping {
    pub filename: String,
    pub to_layer: String,
    pub line_no: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedStackup {
    pub layers: Vec<StackLayer>,
    pub manual_mappings: Vec<ManualMapping>,
    pub diagnostics: Vec<Diagnostic>,
}

/* ------------------------------- parameters ------------------------------- */

/// Versioned engine parameters. Bumping any value creates a new parameter
/// revision id and invalidates (does not overwrite) derived snapshots.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Params {
    pub version: u32,
    /// Geometric contact tolerance in mm. Two copper pieces only become a
    /// *tolerant* contact within this distance; exact touching stays exact.
    pub contact_tolerance_mm: f64,
    /// Hard safety ceiling on raster cells. Exceeding it is budget exhaustion.
    pub max_raster_cells: u64,
    /// Hard safety ceiling on step/repeat-expanded draw instances.
    pub max_draw_instances: u64,
    /// When a drill declares no span, assume it spans the whole stack only if
    /// this is true; otherwise it is reported as unresolved (never assumed).
    pub assume_through_hole_when_no_span: bool,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            version: 1,
            contact_tolerance_mm: 0.0,
            max_raster_cells: 4_000_000,
            max_draw_instances: 200_000,
            assume_through_hole_when_no_span: false,
        }
    }
}

/* -------------------------------- decisions ------------------------------- */

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DecisionKind {
    /// Operator accepts one connectivity candidate for a component group.
    AcceptCandidate,
    /// Operator rejects the whole run conclusion.
    Reject,
    /// Force two components to be treated as one net.
    ForceMerge,
    /// Force one component to be split into two nets at a witness.
    ForceSplit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub kind: DecisionKind,
    pub component_ids: Vec<Id>,
    pub candidate_id: Option<Id>,
    pub note: String,
}

/* ---------------------------------- misc ---------------------------------- */

/// Effective interval shown in the UI: a derived object is valid for events in
/// `[valid_from_seq, valid_until_seq)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectiveInterval {
    pub valid_from_seq: u64,
    pub valid_until_seq: Option<u64>,
    pub invalid_reason: Option<String>,
}

pub type Json = serde_json::Value;
pub type JsonMap = BTreeMap<String, Json>;
