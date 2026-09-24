//! Append-only operation events.
//!
//! Every state change is an event. Events are never rewritten; time travel to
//! an old event replays the prefix and recomputes derived geometry with the
//! parameter version and materials that were in force then.

use crate::model::DisplayUnit;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventPayload {
    ProjectCreated {
        name: String,
    },
    /// One immutable material revision entered the project.
    MaterialImported {
        material_id: String,
        file_name: String,
        revision: String,
        supersedes: Option<String>,
        sha256: String,
        kind: String,
        has_parse_errors: bool,
    },
    /// Reviewer resolved a (possibly duplicated/conflicting) manual mapping.
    ManualMappingResolved {
        file_name: String,
        layer_index: usize,
        role: String,
        /// Rejected duplicate mapping positions, retained for provenance.
        rejected_entries: Vec<String>,
        reason: String,
    },
    /// A new versioned parameter set (contact tolerance etc.).
    ParamsRevised {
        params_version: u32,
        touch_tolerance_mm: f64,
        annular_ring_mm: f64,
        note: String,
        parent_params_version: u32,
    },
    /// A new source revision that should only affect dependent derivations.
    SourceMaterialRevised {
        material_id: String,
        supersedes: String,
        file_name: String,
        revision: String,
        sha256: String,
    },
    /// Accept or reject one legal connectivity candidate (no netlist).
    CandidateDecided {
        /// Candidate id (deterministic per computed state).
        candidate_id: String,
        accepted: bool,
        reason: String,
        /// Params version this decision was made under.
        params_version: u32,
        layer: String,
        /// Member node ids carried for human readable provenance.
        member_nodes: Vec<String>,
    },
    /// Human revision of a stackup layer role.
    LayerRoleRevised {
        layer_index: usize,
        old_role: String,
        new_role: String,
        reason: String,
    },
    DisplayUnitChanged {
        unit: DisplayUnit,
    },
    ExportRecorded {
        export_id: String,
        sha256: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Event {
    pub id: String,
    pub seq: u64,
    pub branch: String,
    pub payload: EventPayload,
    /// ISO-8601 + logical time basis.
    pub at: crate::model::TimeBasis,
    pub actor: String,
    /// Material/parameter inputs this event was recorded against (input summary).
    pub input_summary: EventInputSummary,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct EventInputSummary {
    pub material_hashes: Vec<String>,
    pub params_version: u32,
    pub parent_event_id: Option<String>,
}
