//! Core domain model: raw representation, diagnostics, geometry, materials.
//!
//! Every imported value keeps its original textual representation. Normalised
//! values live only on derived structures and are always `Option`al together
//! with a conversion diagnostic.

use serde::{Deserialize, Serialize};

pub const NM_PER_MM: i64 = 1_000_000;
pub const NM_PER_IN: i64 = 25_400_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FileUnit {
    Mm,
    Inch,
}

impl FileUnit {
    pub fn as_str(self) -> &'static str {
        match self {
            FileUnit::Mm => "mm",
            FileUnit::Inch => "inch",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Polarity {
    Dark,
    Clear,
}

impl Polarity {
    pub fn as_str(self) -> &'static str {
        match self {
            Polarity::Dark => "D",
            Polarity::Clear => "C",
        }
    }
}

/// Position inside an original material file (1-based line/column, 0-based offset).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SrcPos {
    pub line: u32,
    pub col: u32,
    pub offset: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// Diagnostic that always references a position in some material file and keeps
/// the original textual token when a value could not be converted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: Severity,
    pub message: String,
    pub material_id: Option<String>,
    pub pos: Option<SrcPos>,
    /// Original token preserved verbatim; nothing is dropped to draw a picture.
    pub raw: Option<String>,
}

impl Diagnostic {
    pub fn error(code: impl Into<String>, msg: impl Into<String>) -> Self {
        Diagnostic {
            code: code.into(),
            severity: Severity::Error,
            message: msg.into(),
            material_id: None,
            pos: None,
            raw: None,
        }
    }
    pub fn warning(code: impl Into<String>, msg: impl Into<String>) -> Self {
        let mut d = Self::error(code, msg);
        d.severity = Severity::Warning;
        d
    }
    pub fn at(mut self, material_id: &str, pos: SrcPos) -> Self {
        self.material_id = Some(material_id.to_string());
        self.pos = Some(pos);
        self
    }
    pub fn raw(mut self, token: impl Into<String>) -> Self {
        self.raw = Some(token.into());
        self
    }
}

/// A value as written, plus normalised nanometres when conversion succeeded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LengthRaw {
    pub text: String,
    pub unit: FileUnit,
    pub nm: Option<i64>,
}

/// A coordinate as written: absent axes stay absent (never defaulted to 0).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoordRaw {
    pub x: Option<LengthRaw>,
    pub y: Option<LengthRaw>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ShapeKind {
    Circle,
    Rect,
    Trace,
}

/// Normalised planar solid in nanometres, Gerber image plane orientation
/// (x right, y up after file transform).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Shape {
    Circle { cx: i64, cy: i64, r: i64 },
    /// Axis-aligned rectangle.
    Rect { x0: i64, y0: i64, x1: i64, y1: i64 },
    /// Centreline trace (capsule): half width `hw`.
    Trace { x1: i64, y1: i64, x2: i64, y2: i64, hw: i64 },
}

impl Shape {
    pub fn kind(&self) -> ShapeKind {
        match self {
            Shape::Circle { .. } => ShapeKind::Circle,
            Shape::Rect { .. } => ShapeKind::Rect,
            Shape::Trace { .. } => ShapeKind::Trace,
        }
    }
    pub fn bbox(&self) -> (i64, i64, i64, i64) {
        match *self {
            Shape::Circle { cx, cy, r } => (cx - r, cy - r, cx + r, cy + r),
            Shape::Rect { x0, y0, x1, y1 } => (x0, y0, x1, y1),
            Shape::Trace { x1, y1, x2, y2, hw } => {
                let (xa, xb) = (x1.min(x2) - hw, x1.max(x2) + hw);
                let (ya, yb) = (y1.min(y2) - hw, y1.max(y2) + hw);
                (xa, ya, xb, yb)
            }
        }
    }
}

/// Where a feature came from: immutable provenance pointer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureOrigin {
    pub material_id: String,
    /// 1-based source line.
    pub line: u32,
    /// Original source text of the record.
    pub raw_record: String,
    /// Aperture id as written (D10 ...), when applicable.
    pub aperture: Option<String>,
    /// Step/repeat instance (x-index, y-index); None for the base image.
    pub sr_instance: Option<(i64, i64)>,
}

/// An emitted drawable after aperture/step-repeat expansion, in operation
/// order. Polarity is the polarity active at emission time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Feature {
    pub id: String,
    pub layer_id: String,
    pub shape: Shape,
    pub polarity: Polarity,
    pub op_index: u64,
    pub origin: FeatureOrigin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FileKind {
    Gerber,
    Excellon,
    Stackup,
    LayerMap,
    Netlist,
}

impl FileKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FileKind::Gerber => "gerber",
            FileKind::Excellon => "excellon",
            FileKind::Stackup => "stackup",
            FileKind::LayerMap => "layer_map",
            FileKind::Netlist => "netlist",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LayerCategory {
    Copper,
    Soldermask,
    Silkscreen,
    Profile,
    /// Drill/rout tooling data; span lives on each drill tool.
    Drill,
    Other,
}

impl LayerCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            LayerCategory::Copper => "copper",
            LayerCategory::Soldermask => "soldermask",
            LayerCategory::Silkscreen => "silkscreen",
            LayerCategory::Profile => "profile",
            LayerCategory::Drill => "drill",
            LayerCategory::Other => "other",
        }
    }
    pub fn parse(s: &str) -> Option<LayerCategory> {
        Some(match s.to_ascii_lowercase().as_str() {
            "copper" => LayerCategory::Copper,
            "soldermask" | "solder_mask" | "mask" => LayerCategory::Soldermask,
            "silkscreen" | "legend" | "silk" => LayerCategory::Silkscreen,
            "profile" | "outline" => LayerCategory::Profile,
            "drill" | "nc" | "excellon" => LayerCategory::Drill,
            "other" | "misc" => LayerCategory::Other,
            _ => return None,
        })
    }
}

/// Role assigned to a physical file (by the layer-map material or by a manual
/// decision). Layer id references the stackup; masks/silkscreens may carry the
/// side layer id too.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRole {
    pub category: LayerCategory,
    pub layer_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackLayer {
    pub id: String,
    pub name: String,
    pub category: LayerCategory,
    /// Z ordering; original text kept via `z_text`.
    pub z_nm: Option<i64>,
    pub z_text: String,
    pub z_unit: Option<FileUnit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stackup {
    pub layers: Vec<StackLayer>,
}

/// A plated/non-plated drill hole in nanometres.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hole {
    pub id: String,
    pub cx: i64,
    pub cy: i64,
    pub r: i64,
    pub plated: Plating,
    /// Inclusive copper-layer span (stackup layer ids order indices).
    pub span_lo: usize,
    pub span_hi: usize,
    pub tool: String,
    pub origin: FeatureOrigin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Plating {
    Pth,
    Npth,
    /// Material does not state plating: both graphs are legal candidates.
    Unknown,
}

/// One verbatim source record: even unconvertible/corrupt rows stay here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawRecord {
    pub line: u32,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoordinateFormat {
    pub unit: FileUnit,
    pub zero_mode: String, // L (leading omitted) | T (trailing omitted) | D (explicit decimals)
    pub notation: String,  // A | I
    pub x_int: u8,
    pub x_frac: u8,
    pub y_int: u8,
    pub y_frac: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApertureInfo {
    pub code: String,
    pub shape: String,
    pub params_text: String,
}

/// Parsed Gerber file, keeping everything about the original representation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GerberDoc {
    pub format: Option<CoordinateFormat>,
    pub apertures: Vec<ApertureInfo>,
    pub features: Vec<Feature>,
    pub raw_records: Vec<RawRecord>,
    pub role_hint: Option<FileRole>,
    pub diagnostics: Vec<Diagnostic>,
    pub step_repeat_blocks: Vec<StepRepeatBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepRepeatBlock {
    pub line: u32,
    pub nx: i64,
    pub ny: i64,
    pub dx_text: String,
    pub dy_text: String,
    pub dx_nm: Option<i64>,
    pub dy_nm: Option<i64>,
    pub instances: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrillFormat {
    pub unit: Option<FileUnit>,
    pub int: Option<u8>,
    pub frac: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrillTool {
    pub code: String,
    pub diameter_text: String,
    pub diameter_nm: Option<i64>,
    pub plated: Plating,
    /// None = through all copper; Some((lo,hi)) = blind/buried explicit span.
    pub span: Option<(usize, usize)>,
    pub span_text: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExcellonDoc {
    pub format: DrillFormat,
    pub tools: Vec<DrillTool>,
    pub holes: Vec<Hole>,
    pub raw_records: Vec<RawRecord>,
    pub diagnostics: Vec<Diagnostic>,
}

/// High level, typed pipeline failures — three deliberately distinct results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PipelineFailure {
    /// Resource budget exhausted while expanding the input (e.g. step-repeat).
    BudgetExhausted {
        limit: u64,
        used: u64,
        what: String,
    },
    /// Bytes/records that cannot be parsed at all, with positions.
    FormatCorrupt { diagnostics: Vec<Diagnostic> },
    /// Parsed but the geometry/stackup semantics have no unique interpretation.
    SemanticUnresolvable { diagnostics: Vec<Diagnostic> },
}
