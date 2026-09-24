//! Immutable material model and layer-role vocabulary.
//!
//! Everything in this file is imported once and never rewritten: raw source
//! text, coordinate format, units, polarity, apertures, step/repeat blocks,
//! declared layer roles and manual layer mappings. Derived (normalized) layers
//! live in [`crate::domain::pipeline`].

use crate::domain::{Diagnostic, Unit};
use serde::{Deserialize, Serialize};

/// Physical role of a layer in the stackup. `Copper` layers carry indices
/// ("L1".."Ln"); other roles affect mask/drill cross-checks.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum LayerRole {
    Copper,
    Soldermask,
    Legend,
    Drill,
    Paste,
    Outline,
    Mechanical,
}

impl LayerRole {
    pub fn parse(s: &str) -> Option<LayerRole> {
        Some(match s.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "copper" | "cu" | "sig" | "signal" => LayerRole::Copper,
            "soldermask" | "solder_mask" | "sm" | "mask" => LayerRole::Soldermask,
            "legend" | "silkscreen" | "silk" => LayerRole::Legend,
            "drill" | "drill_map" | "holes" => LayerRole::Drill,
            "paste" | "stencil" => LayerRole::Paste,
            "outline" | "profile" => LayerRole::Outline,
            "mechanical" | "mech" => LayerRole::Mechanical,
            _ => return None,
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            LayerRole::Copper => "copper",
            LayerRole::Soldermask => "soldermask",
            LayerRole::Legend => "legend",
            LayerRole::Drill => "drill",
            LayerRole::Paste => "paste",
            LayerRole::Outline => "outline",
            LayerRole::Mechanical => "mechanical",
        }
    }
}

/// Kind of an imported material file.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MaterialKind {
    Gerber,
    Excellon,
    Stackup,
}

impl MaterialKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MaterialKind::Gerber => "gerber",
            MaterialKind::Excellon => "excellon",
            MaterialKind::Stackup => "stackup",
        }
    }
}

/// Polarities present in the source (image polarity and per-block polarity).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
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

/// A step/repeat declaration exactly as the file stated it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StepRepeat {
    pub name: String,
    pub nx: i64,
    pub ny: i64,
    pub dx_raw: String,
    pub dy_raw: String,
    pub dx_unit: Unit,
    pub dy_unit: Unit,
    pub line: usize,
}

/// Manual layer mapping: maps a source file logical layer name to a physical
/// role / copper index as decided by a human. Never auto-reassigned.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ManualMapping {
    pub source_layer: String,
    pub role: String,
    pub copper_index: Option<i64>,
    pub line: usize,
}

/// Raw layer role declaration inside a stackup material.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LayerDecl {
    pub layer_name: String,
    pub role: String,
    pub copper_index: Option<i64>,
    pub side: Option<String>,
    pub line: usize,
}

/// Blind/buried hole layer-range declaration from the drill file comments,
/// retained verbatim plus the interpreted indices when parseable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ViaLayerRange {
    pub raw: String,
    pub from_layer: Option<i64>,
    pub to_layer: Option<i64>,
    pub line: usize,
}

/// A complete immutable import. `raw_text` is preserved byte-for-byte; all
/// parsed detail stays here and never gets overwritten by later decisions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Material {
    pub id: String,
    pub kind: MaterialKind,
    pub filename: String,
    pub raw_text: String,
    pub imported_at: String,
    /// Parsed payload keyed by material kind; normalization never mutates it.
    pub gerber: Option<crate::domain::gerber::GerberDoc>,
    pub excellon: Option<crate::domain::excellon::ExcellonDoc>,
    pub stackup: Option<crate::domain::stackup::StackupDoc>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Convenience: list every source-level quantity/format facet the review UI
/// must expose, so the "nothing dropped" check is explicit.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FacetReport {
    pub material_id: String,
    pub coord_format: Option<String>,
    pub unit: Option<String>,
    pub zero_suppression: Option<String>,
    pub polarities: Vec<String>,
    pub aperture_ids: Vec<String>,
    pub step_repeats: Vec<String>,
    pub layer_roles: Vec<String>,
    pub manual_mappings: Vec<String>,
}
