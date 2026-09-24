//! Domain model. Every imported value keeps its original representation;
//! only derived layers are normalized to nanometres (`nm`) and the project
//! coordinate basis.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
    pub fn from_code(s: &str) -> Option<Self> {
        match s.trim() {
            "D" => Some(Polarity::Dark),
            "C" => Some(Polarity::Clear),
            _ => None,
        }
    }
}

/// Coordinate with original token retained. `nm` is None when the value cannot
/// be converted to the project basis; the diagnostic then stays attached to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Point {
    pub x: i64,
    pub y: i64,
    pub x_raw: String,
    pub y_raw: String,
}

impl Point {
    pub fn raw_pair(&self) -> String {
        format!("{},{}", self.x_raw, self.y_raw)
    }
}

/// A coordinate that may be unconvertible (kept in place, never dropped).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaybeCoord {
    pub raw: String,
    pub nm: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShapeForm {
    Circle,
    Rect,
    Obround,
    Polygon,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Aperture {
    pub id: String,
    pub form: ShapeForm,
    /// Normalised dimensions in nm (diameter / w / h).
    pub dims: Vec<i64>,
    /// Polygon vertices / rotation modifier, kept for provenance.
    pub vertices: Option<u32>,
    pub rotation_ddeg: i64,
    pub raw: String,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepRepeat {
    pub nx: i64,
    pub ny: i64,
    pub dx: i64,
    pub dy: i64,
    pub line: usize,
}

impl StepRepeat {
    pub fn count(&self) -> usize {
        (self.nx.max(1) * self.ny.max(1)) as usize
    }
    pub fn offset(&self, ix: i64, iy: i64) -> (i64, i64) {
        (ix * self.dx, iy * self.dy)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpKind {
    Flash {
        at: Vec<Point>,
        aperture: String,
    },
    Draw {
        a: Point,
        b: Point,
        aperture: String,
    },
    Arc {
        a: Point,
        b: Point,
        center: Point,
        clockwise: bool,
        aperture: String,
    },
    Region {
        contour: Vec<Point>,
    },
}

/// One ordered paint operation. Polarity is resolved in operation order so
/// dark/clear stacking is replayable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaintOp {
    pub index: usize,
    pub line: usize,
    pub polarity: Polarity,
    pub sr: Option<StepRepeat>,
    pub kind: OpKind,
}

impl PaintOp {
    /// Stable identity of a single painted occurrence, used by the UI to trace
    /// an object back to the source operation.
    pub fn occurrence_id(&self, instance: usize) -> String {
        format!("op{}#i{}", self.index, instance)
    }

    /// Anchor points (flattened arcs/regions), in operation order.
    pub fn points(&self) -> Vec<&Point> {
        match &self.kind {
            OpKind::Flash { at, .. } => at.iter().collect(),
            OpKind::Draw { a, b, .. } => vec![a, b],
            OpKind::Arc { a, b, .. } => vec![a, b],
            OpKind::Region { contour } => contour.iter().collect(),
        }
    }

    pub fn aperture(&self) -> Option<&str> {
        match &self.kind {
            OpKind::Flash { aperture, .. } | OpKind::Draw { aperture, .. } | OpKind::Arc { aperture, .. } => Some(aperture),
            OpKind::Region { .. } => None,
        }
    }
}

/// Layer role as declared by file attributes / stackup. Manual mapping revises
/// it; revisions are themselves versioned events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerRole {
    Copper,
    Soldermask,
    Silkscreen,
    Solderpaste,
    BoardOutline,
    Drill,
    Document,
}

impl LayerRole {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().replace(['-', ' '], "_").as_str() {
            "copper" | "cu" => LayerRole::Copper,
            "soldermask" | "solder_mask" | "sm" => LayerRole::Soldermask,
            "silkscreen" | "legend" | "ss" => LayerRole::Silkscreen,
            "solderpaste" | "paste" => LayerRole::Solderpaste,
            "board_outline" | "outline" | "profile" => LayerRole::BoardOutline,
            "drill" | "holes" | "plated" | "non_plated" => LayerRole::Drill,
            "document" | "doc" => LayerRole::Document,
            _ => return None,
        })
    }
    pub fn code(self) -> &'static str {
        match self {
            LayerRole::Copper => "copper",
            LayerRole::Soldermask => "soldermask",
            LayerRole::Silkscreen => "silkscreen",
            LayerRole::Solderpaste => "solderpaste",
            LayerRole::BoardOutline => "board_outline",
            LayerRole::Drill => "drill",
            LayerRole::Document => "document",
        }
    }
}

/// File-level facts about a Gerber document. Raw token strings are retained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GerberDoc {
    pub unit_raw: String,
    /// nm per local unit (25_400_000 for in, 1_000_000 for mm).
    pub unit_nm: i64,
    pub unit_line: usize,
    pub fs_raw: String,
    pub int_digits: u32,
    pub dec_digits: u32,
    pub leading_zero_suppression: bool,
    pub absolute: bool,
    pub apertures: Vec<Aperture>,
    pub ops: Vec<PaintOp>,
    /// File attribute hints as declared, e.g. `.FileFunction,Copper,L1`.
    pub file_attributes: Vec<String>,
    pub inferred_role: Option<LayerRole>,
    pub inferred_copper_order: Option<u32>,
    #[serde(default)]
    pub diagnostics: Vec<crate::error::Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrillTool {
    pub id: String,
    pub diameter_nm: i64,
    pub diameter_raw: String,
    pub plated: bool,
    /// Blind/buried via span: inclusive physical copper order, e.g. (1,2).
    /// Through-hole tools have None.
    pub layer_range: Option<(u32, u32)>,
    pub raw: String,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrillHit {
    pub tool: String,
    pub at: Point,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExcellonDoc {
    pub unit_raw: String,
    pub unit_nm: i64,
    pub unit_line: usize,
    pub tools: Vec<DrillTool>,
    pub hits: Vec<DrillHit>,
    #[serde(default)]
    pub diagnostics: Vec<crate::error::Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackLayer {
    pub physical_index: u32,
    pub role: LayerRole,
    pub copper_order: Option<u32>,
    pub material_id: Option<String>,
    pub name: String,
    pub raw: String,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackupDoc {
    pub layers: Vec<StackLayer>,
    #[serde(default)]
    pub diagnostics: Vec<crate::error::Diagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterialKind {
    Gerber,
    Excellon,
    Stackup,
}

/// Immutable source material. Raw text, coordinate format, units, polarity
/// apertures, step & repeat, role hints and mapping are all retained here and
/// never mutated after import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Material {
    pub id: String,
    pub filename: String,
    pub kind: MaterialKind,
    pub raw_text: String,
    pub content_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gerber: Option<GerberDoc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excellon: Option<ExcellonDoc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stackup: Option<StackupDoc>,
}
