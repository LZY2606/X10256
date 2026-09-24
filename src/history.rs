//! Event-sourced history. Immutable material and mutable decisions are
//! separated: imports and rule versions are never edited, while mapping and
//! accept/reject decisions append new events.
//!
//! Every event has a stable id, an ordered sequence, a parent, an author and a
//! timestamp. Derived results reference the event they were computed from, so
//! checking out an old event restores its dependency versions and conclusions.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::engine::rules::RuleVersion;
use crate::engine::state::RecomputedState;
use crate::model::{LayerRole, Material};

pub fn hex_sha(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    let mut out = String::with_capacity(64);
    for b in h.finalize() {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    ProjectCreated,
    MaterialImported,
    MaterialRevised,
    RuleVersionAdded,
    RuleSelected,
    LayerMapped,
    Decision,
    Recomputed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub seq: i64,
    pub branch: String,
    pub parent_id: Option<String>,
    pub kind: EventType,
    pub author: String,
    pub created_at: String,
    pub summary: String,
    pub payload: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    /// Candidate edge id being judged.
    pub edge_id: String,
    pub accepted: bool,
    pub rationale: String,
}

/// A material revision replaces one direct source while keeping the id and
/// the prior blob as an immutable ancestor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialRevision {
    pub material_id: String,
    pub supersedes_content_hash: String,
    pub material: Material,
}

/// Full replay state at some point on a branch.
#[derive(Default)]
pub struct ReplayState {
    pub materials: Vec<Material>,
    pub material_order: Vec<String>,
    /// material id -> effective role (manual mapping wins)
    pub effective_roles: std::collections::BTreeMap<String, LayerRole>,
    pub rules: Vec<RuleVersion>,
    pub current_rule_id: String,
    pub decisions: Vec<(String, Decision)>,
    /// material id -> list of accepted/rejected edge decisions
    pub last_result: Option<RecomputedState>,
    pub result_by_event: std::collections::BTreeMap<String, RecomputedState>,
}

impl ReplayState {
    pub fn current_rule(&self) -> Option<&RuleVersion> {
        self.rules.iter().find(|r| r.id == self.current_rule_id)
    }

    pub fn material(&self, id: &str) -> Option<&Material> {
        self.materials.iter().find(|m| m.id == id)
    }
}

/// Apply one event to replay state. Returns the recomputed state when the event
/// is Recomputed; results also carry parent_event_id for provenance.
pub fn apply_event(state: &mut ReplayState, ev: &Event) -> Option<RecomputedState> {
    match ev.kind {
        EventType::ProjectCreated => {}
        EventType::MaterialImported => {
            if let Ok(m) = serde_json::from_value::<Material>(ev.payload.clone()) {
                if !state.materials.iter().any(|x| x.id == m.id) {
                    state.material_order.push(m.id.clone());
                    state.materials.push(m);
                }
            }
        }
        EventType::MaterialRevised => {
            if let Ok(rev) = serde_json::from_value::<MaterialRevision>(ev.payload.clone()) {
                if let Some(slot) = state.materials.iter_mut().find(|m| m.id == rev.material_id) {
                    *slot = rev.material;
                } else {
                    state.material_order.push(rev.material_id.clone());
                    state.materials.push(rev.material);
                }
            }
        }
        EventType::RuleVersionAdded => {
            if let Ok(rule) = serde_json::from_value::<RuleVersion>(ev.payload.clone()) {
                if !state.rules.iter().any(|r| r.id == rule.id) {
                    state.rules.push(rule);
                }
            }
        }
        EventType::RuleSelected => {
            state.current_rule_id = ev.payload.get("rule_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        }
        EventType::LayerMapped => {
            let material_id = ev.payload.get("material_id").and_then(|v| v.as_str()).unwrap_or("");
            if let Some(role) = ev
                .payload
                .get("role")
                .and_then(|v| v.as_str())
                .and_then(LayerRole::parse)
            {
                state.effective_roles.insert(material_id.to_string(), role);
            }
        }
        EventType::Decision => {
            if let Ok(d) = serde_json::from_value::<Decision>(ev.payload.clone()) {
                state.decisions.push((ev.id.clone(), d));
            }
        }
        EventType::Recomputed => {
            if let Ok(mut result) = serde_json::from_value::<RecomputedState>(ev.payload.clone()) {
                result.parent_event_id = ev.id.clone();
                state.result_by_event.insert(ev.id.clone(), result.clone());
                state.last_result = Some(result.clone());
                return Some(result);
            }
        }
    }
    None
}

/// Replay the given ordered events from scratch, returning state at the end.
pub fn replay(events: &[Event]) -> ReplayState {
    let mut state = ReplayState::default();
    for ev in events {
        apply_event(&mut state, ev);
    }
    state
}

/// Replay up to (and including) a specific event id on its branch ancestry.
pub fn replay_until(events: &[Event], target_id: &str) -> Option<ReplayState> {
    let chain = ancestry_chain(events, target_id)?;
    let mut state = ReplayState::default();
    for ev in &chain {
        apply_event(&mut state, ev);
    }
    Some(state)
}

/// Return events in ancestry order from the project root to `target_id`.
pub fn ancestry_chain(events: &[Event], target_id: &str) -> Option<Vec<Event>> {
    let by_id: std::collections::HashMap<&str, &Event> =
        events.iter().map(|e| (e.id.as_str(), e)).collect();
    let mut chain: Vec<Event> = Vec::new();
    let mut cursor = by_id.get(target_id).copied();
    while let Some(ev) = cursor {
        chain.push(ev.clone());
        match &ev.parent_id {
            Some(p) => cursor = by_id.get(p.as_str()).copied(),
            None => break,
        }
    }
    if chain.is_empty() {
        return None;
    }
    chain.reverse();
    Some(chain)
}

/// Produce a new event id derived from content so re-imports are stable.
pub fn new_event_id(kind: EventType, seq: i64, payload: &serde_json::Value) -> String {
    let body = format!("{:?}:{}:{}", kind, seq, payload);
    format!("ev-{}-{}", seq, &hex_sha(body.as_bytes())[..10])
}
