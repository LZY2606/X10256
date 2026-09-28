//! Event-sourced project layer.
//!
//! Immutable materials vs. modifiable decisions are separate event kinds.
//! Every derived analysis records an input summary, parameter version and
//! parent dependencies. Replaying to any event restores that point in time;
//! newer computations never overwrite history.

use crate::geometry::{run, EngineConfig, RoleOverrides, RunError};
use crate::model::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub type EventId = String;
pub type BranchId = String;

pub const MAIN_BRANCH: &str = "main";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventData {
    ProjectCreated { name: String },
    MaterialImported { material: Material, source_kind: String },
    LayerMapped { mapping: LayerMapping },
    LayerRoleRevised { material_id: String, from_role: LayerRole, to_role: LayerRole, reason: String },
    /// Revise a *source* material. The old material stays in history.
    MaterialRevised { material: Material, replaces: String, scope: String },
    ParamsRevised { from_version: u32, params: ParamVersion },
    /// Accept / reject an ambiguous net candidate.
    NetDecision { component_id: String, chosen_alternative: Vec<String>, accepted: bool, note: String },
    AnalysisRecorded { result: AnalysisResult, engine_error: Option<EngineErrorView> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineErrorView {
    pub kind: String,
    pub message: String,
    pub detail: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub id: EventId,
    pub seq: u32,
    pub branch: BranchId,
    pub parent: Option<EventId>,
    pub ts_millis: i64,
    pub actor: String,
    pub data: EventData,
    /// Effective interval: null superseded events keep [created, superseded_at).
    pub valid_from: i64,
    pub superseded_by: Option<EventId>,
}

#[derive(Debug, Clone)]
pub struct ProjectState {
    pub project_id: String,
    pub name: String,
    pub materials: Vec<Material>,
    pub mapping: Vec<LayerMapping>,
    pub roles: RoleOverrides,
    pub stackup_layers: u32,
    pub params: ParamVersion,
    pub decisions: Vec<Event>,
    pub latest_result: Option<AnalysisResult>,
    pub latest_error: Option<EngineErrorView>,
}

impl ProjectState {
    fn new(project_id: String, name: String) -> Self {
        ProjectState {
            project_id, name,
            materials: Vec::new(), mapping: Vec::new(), roles: RoleOverrides::default(),
            stackup_layers: 0, params: ParamVersion::default(),
            decisions: Vec::new(), latest_result: None, latest_error: None,
        }
    }

    /// Re-apply one event, returning the analysis effect (if any is requested
    /// by the caller). Returns Err for a *semantic* replay conflict.
    fn apply(&mut self, ev: &Event) {
        match &ev.data {
            EventData::ProjectCreated { name } => { self.name = name.clone(); }
            EventData::MaterialImported { material, .. } => {
                if !self.materials.iter().any(|m| m.id == material.id) {
                    self.materials.push(material.clone());
                    if let MaterialDoc::Stackup(s) = &material.doc {
                        self.stackup_layers = s.layers.len() as u32;
                    }
                }
            }
            EventData::MaterialRevised { material, replaces, scope: _ } => {
                if let Some(pos) = self.materials.iter().position(|m| m.id == *replaces) {
                    self.materials[pos] = material.clone();
                    if let MaterialDoc::Stackup(s) = &material.doc {
                        self.stackup_layers = s.layers.len() as u32;
                    }
                } else if !self.materials.iter().any(|m| m.id == material.id) {
                    self.materials.push(material.clone());
                }
            }
            EventData::LayerMapped { mapping } => {
                if let Some(pos) = self.mapping.iter().position(|m| m.material_id == mapping.material_id) {
                    self.mapping[pos] = mapping.clone();
                } else {
                    self.mapping.push(mapping.clone());
                }
            }
            EventData::LayerRoleRevised { material_id, from_role: _, to_role, reason: _ } => {
                self.roles.by_material.insert(material_id.clone(), *to_role);
            }
            EventData::ParamsRevised { from_version: _, params } => { self.params = params.clone(); }
            EventData::NetDecision { .. } => { self.decisions.push(ev.clone()); }
            EventData::AnalysisRecorded { result, engine_error } => {
                self.latest_result = result.clone();
                self.latest_error = engine_error.clone();
            }
        }
    }

    pub fn precheck(&self) -> Result<(), RunError> {
        // Format corruption: any material carrying a parse-level Error is a
        // distinct outcome from semantic impossibility or budget exhaustion.
        for m in &self.materials {
            let errs: Vec<Diagnostic> = match &m.doc {
                MaterialDoc::Gerber(g) => g.warnings.iter().filter(|d| d.severity == Severity::Error).cloned().collect(),
                MaterialDoc::Excellon(e) => e.warnings.iter().filter(|d| d.severity == Severity::Error).cloned().collect(),
                MaterialDoc::Stackup(s) => s.warnings.iter().filter(|d| d.severity == Severity::Error).cloned().collect(),
            };
            if !errs.is_empty() {
                return Err(RunError::Corrupt { material_id: m.id.clone(), diagnostics: errs });
            }
        }
        Ok(())
    }

    pub fn compute(&self) -> Result<AnalysisResult, RunError> {
        self.precheck()?;
        let budget = if self.params.pair_budget > 0 { self.params.pair_budget } else { 200_000 };
        let cfg = EngineConfig { params: self.params.clone(), pair_budget: budget };
        let parents: Vec<String> = self.decisions.iter().map(|d| d.id.clone()).collect();
        run(&cfg, crate::geometry::NormInputs {
            materials: &self.materials,
            mapping: &self.mapping,
            roles: &self.roles,
            stackup_layers: self.stackup_layers,
        }, parents)
    }
}

/// Events that belong to a branch replay: the full `main` spine plus the
/// branch's own events. Other side branches never leak in.
pub fn branch_event_filter(branch: &str) -> impl Fn(&Event) -> bool + '_ {
    move |ev: &Event| ev.branch == MAIN_BRANCH || ev.branch == branch
}

/// Replay a linear branch of events up to (and including) `until`.
pub fn replay(events: &[Event], project_id: &str, until: Option<&str>) -> ProjectState {
    replay_branch(events, project_id, MAIN_BRANCH, until)
}

pub fn replay_branch(events: &[Event], project_id: &str, branch: &str,
                     until: Option<&str>) -> ProjectState {
    let mut state = ProjectState::new(project_id.into(), "untitled".into());
    let keep = branch_event_filter(branch);
    let mut ordered: Vec<&Event> = events.iter().filter(|e| keep(e)).collect();
    // global chronological order, with the fork branch appended after its base
    ordered.sort_by_key(|e| (if e.branch == MAIN_BRANCH { 0 } else { 1 }, e.seq));
    for ev in ordered {
        state.apply(ev);
        if let Some(u) = until {
            if ev.id == u {
                break;
            }
        }
    }
    state
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

pub fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", uuid::Uuid::new_v4().simple())
}
