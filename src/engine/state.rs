//! Output structures of a recomputation. These are the *derived* layers; all
//! identities are stable content addresses so export/import preserves them.

use serde::{Deserialize, Serialize};

use crate::model::Polarity;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionKind {
    /// Overlap / d < tolerance: definite geometric contact.
    DefiniteContact,
    /// Distance is exactly zero (boundary tangency).
    Tangent,
    /// 0 < d <= tolerance: contact, kept as a candidate under candidate policy.
    ToleranceContact,
    /// tolerance < d <= near-miss band: proximity, never conduction.
    NearMiss,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderAperture {
    pub id: String,
    pub form: String,
    pub dims_nm: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderOp {
    /// Stable occurrence id, e.g. "mat#op3#i0".
    pub id: String,
    pub op_index: usize,
    pub source_line: usize,
    pub polarity: Polarity,
    pub shape: String,
    /// Expanded occurrences (step & repeat produces many).
    pub instances: Vec<RenderInstance>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderInstance {
    pub id: String,
    pub ox: i64,
    pub oy: i64,
    /// A simple drawable geometry: circles/rings/segments/regions in nm.
    pub geom: RenderGeom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RenderGeom {
    Circle { cx: i64, cy: i64, r: i64 },
    Segment { ax: i64, ay: i64, bx: i64, by: i64, width: i64 },
    Polyline { pts: Vec<(i64, i64)>, width: i64 },
    Polygon { pts: Vec<(i64, i64)> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderLayer {
    pub layer_key: String,
    pub role: String,
    pub copper_order: Option<u32>,
    pub material_id: String,
    pub unit_raw: String,
    pub basis: String,
    pub ops: Vec<RenderOp>,
    /// Raster statistics used by the UI.
    pub width_cells: u32,
    pub height_cells: u32,
    pub bbox: (i64, i64, i64, i64),
}

#[derive(Debug,Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderHit {
    pub id: String,
    pub material_id: String,
    pub tool: String,
    pub plated: bool,
    pub diameter_nm: i64,
    pub layer_range: Option<(u32, u32)>,
    pub x: i64,
    pub y: i64,
    pub source_line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentNode {
    pub id: String,
    pub layer_key: String,
    pub copper_order: Option<u32>,
    /// Representative centroid for the layer view.
    pub cx: i64,
    pub cy: i64,
    pub bbox: (i64, i64, i64, i64),
    /// Operation occurrences that contribute copper, in paint order.
    pub occurrences: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkEdge {
    pub id: String,
    pub kind: String,
    pub a: String,
    pub b: String,
    pub distance_nm: i64,
    /// Optional plated hole occurrence that mediates a cross-layer link.
    pub via_hit: Option<String>,
    pub source_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ambiguity {
    pub id: String,
    pub description: String,
    /// Legal edges that may be independently accepted or rejected.
    pub edges: Vec<String>,
    pub source_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateSolution {
    pub id: String,
    pub rank: usize,
    pub accepted_edges: Vec<String>,
    pub component_groups: Vec<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conflict {
    pub id: String,
    pub position: String,
    pub message: String,
    pub decision_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaintRender {
    pub layers: Vec<RenderLayer>,
    pub holes: Vec<RenderHit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecomputedState {
    pub status: String,
    pub input_digest: String,
    pub rule_version_id: String,
    pub parent_event_id: String,
    pub coordinate_basis: String,
    pub layers: Vec<RenderLayer>,
    pub holes: Vec<RenderHit>,
    pub components: Vec<ComponentNode>,
    /// Edges used in the conservative base solution.
    pub base_edges: Vec<LinkEdge>,
    /// Contact edges held back as candidates (never auto-conducting).
    pub candidate_edges: Vec<LinkEdge>,
    /// Proximity-only records; explicitly not conduction.
    pub near_misses: Vec<LinkEdge>,
    pub ambiguities: Vec<Ambiguity>,
    pub candidates: Vec<CandidateSolution>,
    pub conflicts: Vec<Conflict>,
    pub diagnostics: Vec<crate::error::Diagnostic>,
    /// Why this derived state is invalid for the current head (if so).
    pub invalid_reason: Option<String>,
    pub valid_for_head: bool,
}

impl RecomputedState {
    pub const OK: &'static str = "ok";
    pub const BUDGET: &'static str = "budget_exhausted";
    pub const CORRUPT: &'static str = "corrupt_format";
    pub const SEMANTIC: &'static str = "semantic_unresolved";
}
