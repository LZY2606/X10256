//! Reconstruction + connectivity engine.
//!
//! Pure, deterministic and directly callable from storage/tests.
//!
//! Rules encoded here (see `sample/CASEBOOK.md` for the narrative):
//! * dark/clear polarity is overlaid **in operation order**; a later clear
//!   removes an earlier dark, a later dark re-fills;
//! * geometry contact tolerance is a **versioned parameter**, never a global
//!   constant; tangent contact has an explicit per-version policy;
//! * without a netlist the engine emits only connectivity *candidates* and
//!   keeps **every legal** resolution of ambiguous contacts; closeness across
//!   a positive gap is never conductivity;
//! * resource exhaustion, format corruption and semantic impossibility are
//!   three distinct outcomes.

use crate::domain::*;
use crate::geometry::{contact as geom_contact, bridges, Contact, Shape};
use crate::ids::short_hash;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TangencyPolicy {
    /// Tangency (shared boundary point, zero area) counts as bonded copper.
    Bonded,
    /// Tangency is only a candidate: fabrication variance may open the joint.
    Candidate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Params {
    pub version: String,
    /// Non-negative gap budget in nanometres. `0` means strict contact only.
    pub contact_tolerance_nm: i64,
    pub tangency: TangencyPolicy,
    /// Maximum number of distinct candidate partitions retained.
    pub candidate_budget: u64,
}

impl Params {
    pub fn v1() -> Params {
        Params {
            version: "rules-v1".into(),
            contact_tolerance_nm: 0,
            tangency: TangencyPolicy::Bonded,
            candidate_budget: 256,
        }
    }
    pub fn fingerprint(&self) -> String {
        short_hash(&[
            &self.version,
            &self.contact_tolerance_nm.to_string(),
            match self.tangency {
                TangencyPolicy::Bonded => "bonded",
                TangencyPolicy::Candidate => "candidate",
            },
            &self.candidate_budget.to_string(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DecisionData {
    /// Two ambiguous pads/barrels are electrically the same net.
    Join { edge_id: String },
    /// They must be kept apart even when a candidate bridges them.
    Isolate { edge_id: String },
    /// Revision of a stackup layer's role (e.g. mask -> signal).
    RoleRevision { layer: String, role: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub data: DecisionData,
    pub accepted: bool,
    pub deciding_event: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvItem {
    pub source_id: String,
    pub filename: String,
    pub block: Option<usize>,
    pub line: usize,
    pub aperture: Option<String>,
    pub polarity: Option<String>,
    pub sr_id: Option<String>,
    pub sr_instance: Option<i64>,
    pub op_order: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum NodeKind {
    Pad,
    Barrel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GNode {
    pub id: String,
    pub kind: NodeKind,
    pub layer: String,
    pub layers: Vec<String>,
    pub shape: Shape,
    pub label: String,
    pub plating: Option<String>,
    pub removed: bool,
    pub removed_reason: Option<String>,
    pub provenance: Vec<ProvItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeStrength {
    /// Interior area overlap surviving polarity overlay.
    Overlap,
    /// Zero-area boundary touch, bonded per the active rule version.
    TangentBond,
    /// Positive gap closed only by the versioned tolerance: candidate only.
    Bridge,
    /// Plated drill barrel joining layers: candidate only w/o netlist.
    PlatedBarrel,
    /// Plating unknown (no PTH annotation): candidate only.
    PlatingUnknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GEdge {
    pub id: String,
    pub a: String,
    pub b: String,
    pub strength: EdgeStrength,
    /// Gap in nm (0 for contact). Present for reviewer-facing display.
    pub gap_nm: i64,
    pub across_span: Option<String>,
    pub alive: bool,
    pub removed_reason: Option<String>,
    pub witness: Witness,
    pub provenance: Vec<ProvItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Witness {
    pub x: i64,
    pub y: i64,
    /// Polarity of the last covering operation at the witness (op order).
    pub last_polarity: String,
    pub last_op_order: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaskOpening {
    pub id: String,
    pub layer: String,
    pub shape: Shape,
    pub provenance: Vec<ProvItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidatePartition {
    pub id: String,
    pub rank: usize,
    pub merged_weak_edges: Vec<String>,
    /// Components as sorted groups of node ids.
    pub components: Vec<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conflict {
    pub id: String,
    pub edge_id: String,
    pub join_event: String,
    pub isolate_event: String,
    pub at_line: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeKind {
    Ok,
    Incomplete,
    FormatCorrupt,
    SemanticUnsolvable,
    BudgetExhausted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysisResult {
    pub id: String,
    pub params_version: String,
    pub params_fingerprint: String,
    pub coordinate_basis: String,
    pub netlist_present: bool,
    pub outcome: OutcomeKind,
    pub outcome_reason: Option<String>,
    pub input_summary: InputSummary,
    pub nodes: Vec<GNode>,
    pub edges: Vec<GEdge>,
    pub mask_openings: Vec<MaskOpening>,
    pub partitions: Vec<CandidatePartition>,
    pub partition_count_total: Option<u64>,
    pub conflicts: Vec<Conflict>,
    pub diagnostics: Vec<Diagnostic>,
    pub candidate_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputSummary {
    pub source_hashes: Vec<(String, String)>,
    pub parent_event: String,
    pub decisions_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Primitive {
    id: String,
    layer: String,
    polarity: Polarity,
    shape: Shape,
    order: usize,
    prov: ProvItem,
}
