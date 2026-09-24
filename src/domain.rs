//! Core domain model.
//!
//! Two tiers are kept strictly separate:
//! * immutable source material: original coordinate notation, units, polarity,
//!   apertures, step/repeat blocks, declared layer roles and manual layer
//!   mapping are all retained verbatim;
//! * derived geometry: normalized into picometers (`Pm`) on a shared board
//!   origin. Derived data never overwrites source representation.

use serde::{Deserialize, Serialize};

/// Length in picometers (1 nm = 1_000 pm, 1 mm = 1_000_000 pm).
pub type Pm = i64;

pub const PM_PER_MM: i64 = 1_000_000;
pub const PM_PER_IN: i64 = 25_400_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Unit {
    In,
    Mm,
    Unknown,
}

impl Unit {
    pub fn code(self) -> &'static str {
        match self {
            Unit::In => "IN",
            Unit::Mm => "MM",
            Unit::Unknown => "UNKNOWN",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
pub enum MaterialKind {
    Gerber,
    Excellon,
    Stackup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LayerRole {
    Copper,
    SolderMask,
    PlatedDrill,
    NonPlatedDrill,
    Silkscreen,
    Outline,
    Mechanical,
    Unknown,
}

impl LayerRole {
    pub fn code(self) -> &'static str {
        match self {
            LayerRole::Copper => "COPPER",
            LayerRole::SolderMask => "SOLDER_MASK",
            LayerRole::PlatedDrill => "PLATED_DRILL",
            LayerRole::NonPlatedDrill => "NON_PLATED_DRILL",
            LayerRole::Silkscreen => "SILKSCREEN",
            LayerRole::Outline => "OUTLINE",
            LayerRole::Mechanical => "MECHANICAL",
            LayerRole::Unknown => "UNKNOWN",
        }
    }

    pub fn parse(s: &str) -> LayerRole {
        match s.trim().to_ascii_uppercase().replace(['-', ' '], "_").as_str() {
            "COPPER" | "SIGNAL" => LayerRole::Copper,
            "SOLDER_MASK" | "MASK" | "SOLDERMASK" => LayerRole::SolderMask,
            "PLATED_DRILL" | "PLATED" | "PTH" => LayerRole::PlatedDrill,
            "NON_PLATED_DRILL" | "NPTH" => LayerRole::NonPlatedDrill,
            "SILKSCREEN" | "SILK" => LayerRole::Silkscreen,
            "OUTLINE" | "PROFILE" => LayerRole::Outline,
            "MECHANICAL" | "MECH" => LayerRole::Mechanical,
            _ => LayerRole::Unknown,
        }
    }

    pub fn is_conductive(self) -> bool {
        matches!(self, LayerRole::Copper)
    }
}

/// Position inside a source file. All positions are 1-based and refer to the
/// original text, so diagnostics can be chased even when the value could not be
/// converted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    pub material_id: Option<String>,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub command: Option<String>,
}

/// Diagnostic codes double as machine-readable judgement reasons.
pub mod diag_code {
    pub const FORMAT_CORRUPTED: &str = "FORMAT_CORRUPTED";
    pub const SEMANTIC_NO_SOLUTION: &str = "SEMANTIC_NO_SOLUTION";
    pub const INPUT_INCOMPLETE: &str = "INPUT_INCOMPLETE";
    pub const UNCONVERTIBLE: &str = "UNCONVERTIBLE";
    pub const BUDGET_EXHAUSTED: &str = "BUDGET_EXHAUSTED";
    pub const POLARITY_ORDER: &str = "POLARITY_ORDER";
    pub const LAYER_RANGE_INVALID: &str = "LAYER_RANGE_INVALID";
    pub const LAYER_UNMAPPED: &str = "LAYER_UNMAPPED";
    pub const ROLE_REVISION: &str = "ROLE_REVISION";
    pub const NEAR_MISS: &str = "NEAR_MISS";
    pub const DECISION_CONFLICT: &str = "DECISION_CONFLICT";
    pub const TANGENT: &str = "TANGENT";
    pub const NO_NETLIST: &str = "NO_NETLIST";
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: String, // "error" | "warning" | "info"
    pub message: String,
    pub location: Location,
    /// Original token that triggered the diagnostic. It stays in place and is
    /// never dropped "to make the picture draw".
    pub raw: Option<String>,
}

impl Diagnostic {
    pub fn error(code: impl Into<String>, msg: impl Into<String>, loc: Location, raw: Option<String>) -> Self {
        Diagnostic { code: code.into(), severity: "error".into(), message: msg.into(), location: loc, raw }
    }
    pub fn warning(code: impl Into<String>, msg: impl Into<String>, loc: Location, raw: Option<String>) -> Self {
        Diagnostic { code: code.into(), severity: "warning".into(), message: msg.into(), location: loc, raw }
    }
    pub fn info(code: impl Into<String>, msg: impl Into<String>, loc: Location, raw: Option<String>) -> Self {
        Diagnostic { code: code.into(), severity: "info".into(), message: msg.into(), location: loc, raw }
    }
}

/// Coordinate format declaration (`%FSLAX...Y...%`), retained in full.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoordFmt {
    pub raw: String,
    pub zero_omission: String, // L | T
    pub absolute: bool,
    pub x_int: u32,
    pub x_dec: u32,
    pub y_int: u32,
    pub y_dec: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ApShape {
    Circle,
    Rect,
    Obround,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApertureDecl {
    pub dcode: String,
    pub shape: ApShape,
    /// Dimensions in aperture-declaration units, normalized to pm.
    pub width: Pm,
    pub height: Pm,
    pub line: u32,
    pub raw: String,
}

/// A raw coordinate pair: original token text plus resolved pm value.
/// `resolved` is None exactly when the value could not be converted; the raw
/// token is retained and an UNCONVERTIBLE diagnostic is attached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoordToken {
    pub raw: String,
    pub resolved: Option<Pm>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GerberOp {
    UnitDecl { line: u32, raw: String, unit: Unit },
    CoordFmt { line: u32, fmt: CoordFmt },
    ApertureDef { line: u32, decl: ApertureDecl },
    Polarity { line: u32, raw: String, polarity: Polarity },
    /// `SRX<nx>Y<ny>I<dx>J<dy>*`: counts plus step spacings kept as original
    /// coordinate tokens (resolved by the engine in file units).
    StepRepeatBegin {
        line: u32,
        raw: String,
        nx: i32,
        ny: i32,
        dx: CoordToken,
        dy: CoordToken,
    },
    StepRepeatEnd { line: u32, raw: String },
    /// Filled region (G36..G37), points in document order.
    Region {
        line: u32,
        raw: String,
        points: Vec<(CoordToken, CoordToken)>,
    },
    Flash {
        line: u32,
        raw: String,
        dcode: String,
        x: CoordToken,
        y: CoordToken,
    },
    Draw {
        line: u32,
        raw: String,
        dcode: String,
        /// Visited points in document order, including implicit repeats.
        points: Vec<(CoordToken, CoordToken)>,
    },
    Other { line: u32, raw: String },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GerberDoc {
    pub unit: Unit,
    pub unit_line: Option<u32>,
    pub coord_fmt: Option<CoordFmt>,
    pub ops: Vec<GerberOp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrillHit {
    pub line: u32,
    pub raw: String,
    pub tool: String,
    pub x: CoordToken,
    pub y: CoordToken,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrillTool {
    pub code: String,
    /// Diameter in declaration units, normalized to pm.
    pub diameter: Option<Pm>,
    pub raw_diameter: String,
    pub plated: bool,
    /// Optional blind/buried span declared with a custom comment,
    /// e.g. `;SPAN L2-L3` (1-based copper indices, inclusive).
    pub span_from: Option<i32>,
    pub span_to: Option<i32>,
    pub line: u32,
    pub raw: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExcellonDoc {
    pub unit: Unit,
    pub tools: Vec<DrillTool>,
    pub hits: Vec<DrillHit>,
    /// Overall file is plated (default) or explicitly non-plated (`M48`
    /// header in an NPTH file plus `;NON_PLATED` marker).
    pub file_plated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackLayer {
    pub index: usize,
    pub role: LayerRole,
    /// Role as literally written before normalization.
    pub raw_role: String,
    pub name: String,
    /// Source filename this layer is bound to, as written.
    pub file: Option<String>,
    pub line: u32,
    /// Manual mapping note retained verbatim.
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackupDoc {
    pub layers: Vec<StackLayer>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParsedDoc {
    Gerber(GerberDoc),
    Excellon(ExcellonDoc),
    Stackup(StackupDoc),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParseOutcome {
    Parsed,
    FormatCorrupted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Material {
    pub id: String,
    pub filename: String,
    pub kind: MaterialKind,
    pub raw_content: String,
    pub doc: ParsedDoc,
    pub outcome: ParseOutcome,
    pub diagnostics: Vec<Diagnostic>,
}

// ---------------------------------------------------------------------------
// Derived geometry (picometers, board origin).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ShapeKind {
    Circle { cx: Pm, cy: Pm, r: Pm },
    Rect { x0: Pm, y0: Pm, x1: Pm, y1: Pm },
    Capsule { x0: Pm, y0: Pm, x1: Pm, y1: Pm, r: Pm },
    Polygon { points_idx_start: usize, points_idx_end: usize },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shape {
    pub id: String,
    pub material_id: String,
    pub op_index: usize,
    pub step_copy: Option<(i32, i32)>,
    pub polarity: Polarity,
    pub kind: ShapeKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hole {
    pub id: String,
    pub material_id: String,
    pub tool: String,
    pub plated: bool,
    pub x: Pm,
    pub y: Pm,
    pub diameter: Option<Pm>,
    /// Inclusive 1-based copper-layer indices when a blind/buried span is
    /// declared; through-holes use `None`.
    pub span: Option<(i32, i32)>,
}
