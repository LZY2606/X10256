//! Immutable source material models.
//!
//! Everything a fabrication engineer hands over is captured here *verbatim*:
//! original coordinate strings, units, polarity, aperture definitions,
//! step-repeat blocks, declared layer roles and hand-written layer mapping.
//! Derived, normalised geometry lives in [`crate::geometry`]; raw values are
//! never rewritten in place. Unconvertible values stay at their origin with a
//! diagnostic instead of being replaced by a default.

use serde::{Deserialize, Serialize};

/// Length unit declared by a source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LengthUnit {
    Mm,
    In,
}

impl LengthUnit {
    pub fn code(self) -> &'static str {
        match self {
            LengthUnit::Mm => "mm",
            LengthUnit::In => "in",
        }
    }
    /// Multiplier that turns a value expressed in this unit into millimetres.
    pub fn to_mm_factor(self) -> f64 {
        match self {
            LengthUnit::Mm => 1.0,
            LengthUnit::In => 25.4,
        }
    }
}

/// Gerber zero-suppression mode (FS specification).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ZeroMode {
    /// `L`: leading zeros omitted, e.g. `3400` for 0.3400 in 2.4 format.
    LeadingSuppressed,
    /// `T`: trailing zeros omitted.
    TrailingSuppressed,
    /// Explicit decimal point, no digit padding.
    Decimal,
}

/// Coordinate format declared by a Gerber `%FS...%` command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoordFmt {
    pub x_int: u8,
    pub x_dec: u8,
    pub y_int: u8,
    pub y_dec: u8,
    pub zero: ZeroMode,
}

impl CoordFmt {
    pub fn render(&self) -> String {
        let z = match self.zero {
            ZeroMode::LeadingSuppressed => "L",
            ZeroMode::TrailingSuppressed => "T",
            ZeroMode::Decimal => "D",
        };
        format!(
            "FS A {}{} X{}.{} Y{}.{}",
            z, z, self.x_int, self.x_dec, self.y_int, self.y_dec
        )
    }
}

/// File-position pointer back to the immutable material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Origin {
    pub material_id: String,
    pub filename: String,
    /// 1-based source line.
    pub line: usize,
    /// 1-based column of the token, when known.
    pub col: Option<usize>,
    /// Exact source token/line text.
    pub raw: String,
}

/// Diagnostic that never silently replaces a value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub severity: DiagSeverity,
    pub code: String,
    pub message: String,
    pub origin: Option<Origin>,
    /// The value that could not be converted, kept at its original position.
    pub unconverted: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagSeverity {
    Error,
    Warning,
    Info,
}

/// Gerber polarity in operation order.
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

/// Raw aperture definition, modifiers kept as source tokens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApertureDef {
    /// D-code as written, e.g. `D10`.
    pub code: String,
    /// Shape letter / macro name, e.g. `C`, `R`, `O`.
    pub kind: String,
    pub modifiers_raw: Vec<String>,
    pub origin: Origin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Interp {
    Line,
    ArcCw,
    ArcCcw,
}

/// A coordinate exactly as written, plus the parsed value in *file units*
/// when conversion was possible.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawCoord {
    pub raw: String,
    pub value: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GerberCmd {
    UnitDecl {
        unit: LengthUnit,
        origin: Origin,
    },
    FmtDecl {
        fmt: Option<CoordFmt>,
        origin: Origin,
    },
    ApertureDecl(ApertureDef),
    Polarity {
        polarity: Polarity,
        origin: Origin,
    },
    ApertureSelect {
        code: String,
        origin: Origin,
    },
    Move {
        x: Option<RawCoord>,
        y: Option<RawCoord>,
        origin: Origin,
    },
    Draw {
        x: Option<RawCoord>,
        y: Option<RawCoord>,
        i: Option<RawCoord>,
        j: Option<RawCoord>,
        interp: Interp,
        origin: Origin,
    },
    Flash {
        x: Option<RawCoord>,
        y: Option<RawCoord>,
        origin: Origin,
    },
    RegionBegin {
        origin: Origin,
    },
    RegionEnd {
        origin: Origin,
    },
    StepRepeat {
        nx: Option<RawCoord>,
        ny: Option<RawCoord>,
        dx: Option<RawCoord>,
        dy: Option<RawCoord>,
        /// `true` for `%SR*%` block exit.
        exit: bool,
        origin: Origin,
    },
    /// Retained, unsupported content — never dropped silently.
    Unsupported {
        text: String,
        origin: Origin,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawGerber {
    pub filename: String,
    pub material_id: String,
    pub declared_unit: Option<LengthUnit>,
    pub declared_fmt: Option<CoordFmt>,
    pub commands: Vec<GerberCmd>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawTool {
    pub id: String,
    pub diameter_raw: String,
    /// Diameter in file units, if convertible.
    pub diameter: Option<f64>,
    pub origin: Origin,
}

/// Layer span declaration (our documented Excellon comment extension).
///
/// `;SPAN=start-end` names stackup layers, `;METAL=plated|nonplated` sets
/// plating for following hits. Through hits have no explicit span.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawSpan {
    pub from_raw: String,
    pub to_raw: String,
    pub plated: bool,
    pub plated_raw: String,
    pub origin: Origin,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawHit {
    pub tool_id: String,
    pub x: RawCoord,
    pub y: RawCoord,
    /// Index into [`RawExcellon::spans`], `None` means whole stack (through).
    pub span_idx: Option<usize>,
    pub origin: Origin,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawExcellon {
    pub filename: String,
    pub material_id: String,
    pub declared_unit: Option<LengthUnit>,
    /// `(integer, decimal)` digit counts when declared.
    pub declared_fmt: Option<(u8, u8)>,
    pub tools: Vec<RawTool>,
    pub spans: Vec<RawSpan>,
    pub hits: Vec<RawHit>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Role a layer plays in fabrication. Copper layers participate in
/// connectivity; mask layers expose solder openings; the rest are retained
/// for review but do not conduct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LayerRole {
    Copper,
    MaskTop,
    MaskBottom,
    Silk,
    Paste,
    Outline,
    Dielectric,
    Documentation,
}

impl LayerRole {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "copper" | "sig" | "signal" => LayerRole::Copper,
            "mask_top" | "soldermask_top" | "masktop" => LayerRole::MaskTop,
            "mask_bottom" | "soldermask_bottom" | "maskbottom" => LayerRole::MaskBottom,
            "silk" | "silkscreen" => LayerRole::Silk,
            "paste" => LayerRole::Paste,
            "outline" | "profile" => LayerRole::Outline,
            "dielectric" | "core" | "prepreg" => LayerRole::Dielectric,
            "doc" | "documentation" | "mechanical" => LayerRole::Documentation,
            _ => return None,
        })
    }
    pub fn code(self) -> &'static str {
        match self {
            LayerRole::Copper => "copper",
            LayerRole::MaskTop => "mask_top",
            LayerRole::MaskBottom => "mask_bottom",
            LayerRole::Silk => "silk",
            LayerRole::Paste => "paste",
            LayerRole::Outline => "outline",
            LayerRole::Dielectric => "dielectric",
            LayerRole::Documentation => "documentation",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackLayer {
    pub name: String,
    pub role_raw: String,
    pub role: Option<LayerRole>,
    /// Path token as written in the stackup file.
    pub path: String,
    pub origin: Origin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackDrill {
    pub plated: bool,
    pub plated_raw: String,
    pub path: String,
    pub origin: Origin,
}

/// A hand-written mapping row `MAP logical=material` from the stackup file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapRow {
    pub logical: String,
    pub material_ref: String,
    pub origin: Origin,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawStackup {
    pub filename: String,
    pub material_id: String,
    pub layers: Vec<StackLayer>,
    pub drills: Vec<StackDrill>,
    pub netlist_path: Option<String>,
    pub mapping: Vec<MapRow>,
    pub diagnostics: Vec<Diagnostic>,
}

/// One netlist membership: `NET net layer:figure-ref`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetEntry {
    pub net: String,
    pub layer: String,
    pub figure_ref: String,
    pub origin: Origin,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawNetlist {
    pub filename: String,
    pub material_id: String,
    pub entries: Vec<NetEntry>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Kind of imported source material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MaterialKind {
    Gerber,
    Excellon,
    Stackup,
    Netlist,
}

impl MaterialKind {
    pub fn from_filename(name: &str) -> Option<Self> {
        let n = name.to_ascii_lowercase();
        Some(if n.ends_with(".gbr") || n.ends_with(".ger") || n.ends_with(".gtl") {
            MaterialKind::Gerber
        } else if n.ends_with(".drl") || n.ends_with(".xln") || n.ends_with(".txt") {
            MaterialKind::Excellon
        } else if n.ends_with(".stack") || n.ends_with(".stk") {
            MaterialKind::Stackup
        } else if n.ends_with(".net") {
            MaterialKind::Netlist
        } else {
            return None;
        })
    }
    pub fn code(self) -> &'static str {
        match self {
            MaterialKind::Gerber => "gerber",
            MaterialKind::Excellon => "excellon",
            MaterialKind::Stackup => "stackup",
            MaterialKind::Netlist => "netlist",
        }
    }
}
