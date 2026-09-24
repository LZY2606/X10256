//! Core domain model: units, polarity, layer roles, parameters, diagnostics.
//! All domain quantities carry their unit / coordinate basis and raw representation.

use serde::{Deserialize, Serialize};

/// Physical unit declared by the source file. Never inferred silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    /// Gerber `%MOMM*%` / Excellon `METRIC`
    Mm,
    /// Gerber `%MOIN*%` / Excellon `INCHZ`
    In,
    /// Unit directive absent: raw values are kept in place and a diagnostic is emitted.
    Unknown,
}

impl Unit {
    pub fn label(&self) -> &'static str {
        match self {
            Unit::Mm => "mm",
            Unit::In => "in",
            Unit::Unknown => "unknown",
        }
    }
    pub fn to_mm_factor(&self) -> Option<f64> {
        match self {
            Unit::Mm => Some(1.0),
            Unit::In => Some(25.4),
            Unit::Unknown => None,
        }
    }
}

/// Image polarity, applied strictly in source operation order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Polarity {
    /// `%LPD*%` dark / additive
    Dark,
    /// `%LPC*%` clear / subtractive
    Clear,
}

/// Electrical / fabrication role of a layer in the stackup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum LayerRole {
    TopCopper,
    BottomCopper,
    InnerCopper,
    TopSolderMask,
    BottomSolderMask,
    Drill,
    Document,
    Unmapped,
}

impl LayerRole {
    pub fn is_copper(&self) -> bool {
        matches!(
            self,
            LayerRole::TopCopper | LayerRole::BottomCopper | LayerRole::InnerCopper
        )
    }
    pub fn is_mask(&self) -> bool {
        matches!(self, LayerRole::TopSolderMask | LayerRole::BottomSolderMask)
    }
    pub fn parse(s: &str) -> LayerRole {
        match s.trim().to_ascii_uppercase().as_str() {
            "TOP_COPPER" | "TOP" | "COPPER_TOP" => LayerRole::TopCopper,
            "BOTTOM_COPPER" | "BOTTOM" | "COPPER_BOTTOM" => LayerRole::BottomCopper,
            "INNER_COPPER" | "INNER" => LayerRole::InnerCopper,
            "TOP_SOLDER_MASK" | "TOP_MASK" | "TSM" => LayerRole::TopSolderMask,
            "BOTTOM_SOLDER_MASK" | "BOTTOM_MASK" | "BSM" => LayerRole::BottomSolderMask,
            "DRILL" | "NC" | "EXCELLON" => LayerRole::Drill,
            "DOCUMENT" | "DOC" => LayerRole::Document,
            _ => LayerRole::Unmapped,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            LayerRole::TopCopper => "top_copper",
            LayerRole::BottomCopper => "bottom_copper",
            LayerRole::InnerCopper => "inner_copper",
            LayerRole::TopSolderMask => "top_solder_mask",
            LayerRole::BottomSolderMask => "bottom_solder_mask",
            LayerRole::Drill => "drill",
            LayerRole::Document => "document",
            LayerRole::Unmapped => "unmapped",
        }
    }
}

/// Coordinate basis exactly as declared by the source (Gerber FS specification or
/// Excellon format comment). The raw directive is always preserved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoordBasis {
    pub unit: Unit,
    /// Raw unit directive, e.g. `%MOMM*%` or `METRIC,MM`.
    pub unit_raw: Option<String>,
    /// Raw coordinate format directive, e.g. `%FSLAX34Y34*%` or `; FORMAT 3.3`.
    pub format_raw: Option<String>,
    /// Gerber zero suppression / notation, informational.
    pub notation: String,
    pub x_int: u8,
    pub x_dec: u8,
    pub y_int: u8,
    pub y_dec: u8,
}

impl CoordBasis {
    pub fn unknown() -> Self {
        CoordBasis {
            unit: Unit::Unknown,
            unit_raw: None,
            format_raw: None,
            notation: "unspecified".into(),
            x_int: 0,
            x_dec: 0,
            y_int: 0,
            y_dec: 0,
        }
    }
}

/// A domain value that keeps its original textual representation even when conversion
/// to millimetres is impossible. Derived layers use `value_mm`; originals never lose data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quantity {
    pub raw: String,
    pub unit: Unit,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_mm: Option<f64>,
    pub convertible: bool,
}

impl Quantity {
    pub fn new(raw: impl Into<String>, unit: Unit) -> Self {
        let raw = raw.into();
        let parsed: Option<f64> = raw.trim().parse().ok();
        let value_mm = parsed.zip(unit.to_mm_factor()).map(|(v, f)| v * f);
        Quantity {
            raw,
            unit,
            value_mm,
            convertible: value_mm.is_some(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// Diagnostic anchored at a source position. Never silently replaces a value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: Severity,
    pub message: String,
    pub source_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    /// Raw token that could not be converted / understood, retained in place.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_token: Option<String>,
}

impl Diagnostic {
    pub fn err(code: &str, source_id: &str, line: Option<usize>, message: impl Into<String>) -> Self {
        Diagnostic {
            code: code.into(),
            severity: Severity::Error,
            message: message.into(),
            source_id: source_id.into(),
            line,
            column: None,
            snippet: None,
            raw_token: None,
        }
    }
    pub fn with_snippet(mut self, snip: impl Into<String>, token: impl Into<String>) -> Self {
        self.snippet = Some(snip.into());
        self.raw_token = Some(token.into());
        self
    }
}

/// How exact point contact is interpreted; versioned because tolerances are rule
/// parameters, not constants baked into the geometry code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TangencyRule {
    /// Corner / single-point contact stays a candidate, never a definite connection.
    PointContactIsCandidate,
}

/// Versioned geometry & connectivity rule set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleParams {
    pub version: String,
    pub published_at: String,
    pub grid_res_mm: f64,
    /// Two copper regions whose gap is within this distance are *proximity candidates*;
    /// proximity alone is never treated as conduction.
    pub proximity_tol_mm: f64,
    pub tangency: TangencyRule,
    pub max_grid_cells: u64,
    pub max_ambiguity_options: u64,
    pub note: String,
}

pub fn builtin_params() -> Vec<RuleParams> {
    vec![
        RuleParams {
            version: "rules-2024-strict".into(),
            published_at: "2024-01-15".into(),
            grid_res_mm: 0.02,
            proximity_tol_mm: 0.0,
            tangency: TangencyRule::PointContactIsCandidate,
            max_grid_cells: 4_000_000,
            max_ambiguity_options: 32,
            note: "Strict baseline: only edge/area overlap is definite; point touch is a candidate."
                .into(),
        },
        RuleParams {
            version: "rules-2025-tol01".into(),
            published_at: "2025-03-02".into(),
            grid_res_mm: 0.02,
            proximity_tol_mm: 0.10,
            tangency: TangencyRule::PointContactIsCandidate,
            max_grid_cells: 4_000_000,
            max_ambiguity_options: 32,
            note: "Tolerance revision: gaps <= 0.10 mm become visible proximity candidates, \
                   still never auto-conductive."
                .into(),
        },
        RuleParams {
            version: "rules-budget-probe".into(),
            published_at: "2025-06-20".into(),
            grid_res_mm: 0.02,
            proximity_tol_mm: 0.10,
            tangency: TangencyRule::PointContactIsCandidate,
            max_grid_cells: 4_000_000,
            max_ambiguity_options: 4,
            note: "Tiny ambiguity budget used to exercise resource exhaustion deterministically."
                .into(),
        },
    ]
}

pub fn params_by_version(version: &str) -> Option<RuleParams> {
    builtin_params().into_iter().find(|p| p.version == version)
}
