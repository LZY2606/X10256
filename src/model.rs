//! Normalized board model and versioned connectivity parameters.
//!
//! The model is the *derived* layer: coordinates are normalized to nanometers.
//! Raw representations remain on the parsed documents; anything unconvertible
//! surfaces as a diagnostic and never produces fabricated geometry.

use crate::domain::*;
use crate::excellon::DrillDoc;
use crate::gerber::GerberDoc;
use crate::geometry::Shape;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelLayer {
    pub layer: String,
    pub role: LayerRole,
    pub z_order: u32,
    pub material_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelHole {
    pub id: String,
    pub tool: u32,
    pub x: Coord,
    pub y: Coord,
    pub radius: Coord,
    pub plated: bool,
    pub span_from: u32,
    pub span_to: u32,
    pub through: bool,
    pub material_id: String,
    pub pos: Option<SourcePos>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelShape {
    pub id: String,
    pub layer: String,
    /// Gerber polarity: dark adds copper, clear subtracts it (in source order).
    pub dark: bool,
    pub shape: Shape,
    pub material_id: String,
    pub op_seq: u64,
    pub pos: Option<SourcePos>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaskOpening {
    pub layer: String,
    pub shape: Shape,
    pub material_id: String,
}

/// Normalized board model. `diags` carries semantic problems (bad spans,
/// missing files, unconvertible geometry) with source positions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BoardModel {
    pub layers: Vec<ModelLayer>,
    pub shapes: Vec<ModelShape>,
    pub holes: Vec<ModelHole>,
    pub mask_openings: Vec<MaskOpening>,
    pub diags: Vec<Diag>,
    pub bounds_nm: (i64, i64, i64, i64),
}

/// Versioned geometric parameters. Contact tolerance is explicitly a versioned
/// parameter; changing it creates a new parameter revision, never mutating the
/// old conclusions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectParams {
    pub version: u32,
    pub label: String,
    /// Shapes closer than this may be considered touching (tangent inclusion).
    pub contact_tolerance_nm: i64,
    /// Raster sampling pitch used for in-layer occupation.
    pub grid_pitch_nm: i64,
    /// Gaps larger than contact but within this become proximity candidates.
    pub proximity_nm: i64,
    pub budget: Budget,
}

impl Default for ConnectParams {
    fn default() -> Self {
        Self {
            version: 1,
            label: "default v1".into(),
            contact_tolerance_nm: 1_000,
            grid_pitch_nm: 500,
            proximity_nm: 100_000,
            budget: Budget::default(),
        }
    }
}

/// A parsed material in the model assembly.
#[derive(Debug, Clone)]
pub struct ParsedMaterial {
    pub id: String,
    pub filename: String,
    pub kind: MaterialKind,
    pub raw: String,
    pub gerber: Option<GerberDoc>,
    pub drill: Option<DrillDoc>,
    pub stackup_raw: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Span {
    pub from: u32,
    pub to: u32,
}

/// Manual layer mapping as decided by the reviewer (event-sourced upstream).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LayerMapping {
    pub file_to_layer: BTreeMap<String, String>,
    pub spans: BTreeMap<String, Span>,
    pub plated_override: BTreeMap<String, bool>,
    pub role_revision: BTreeMap<String, LayerRole>,
}
