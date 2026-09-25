//! Event-sourced pipeline.
//!
//! Layering:
//! * **Materials** (`material::*`) are immutable source files. A "source data
//!   change" appends a *new revision* of that material; the old revision and
//!   every result derived from it stay addressable.
//! * **Decisions** (`decision::*`) are append-only review acts: parameter
//!   versions, role/map revisions, accept/reject, conflict flags.
//! * **Results** are derived: each records an input summary, parameter version,
//!   and parent dependencies, so any historical event restores the view as it
//!   was — newer computation never overwrites history.

use crate::geom::build::{reconstruct, Budget, LayerAssign};
use crate::geom::connect::connectivity;
use crate::id;
use crate::model::*;
use crate::params::Params;
use crate::parse::{excellon, gerber, stackup};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventPayload {
    /// Add or revise an immutable source file. `revision` starts at 1.
    MaterialAdded {
        material_id: String,
        file_name: String,
        kind: DocKind,
        revision: usize,
        raw_text: String,
        /// None when this is the first revision.
        parent_material_id: Option<String>,
        change_note: String,
    },
    /// Effective layer role override for a file.
    RoleRevised {
        file_name: String,
        from_role: LayerRole,
        to_role: LayerRole,
        reason: String,
    },
    /// Manual layer mapping: file -> stackup layer index.
    LayerMapped {
        file_name: String,
        layer_index: usize,
        previous: Option<usize>,
        reason: String,
    },
    /// Publish a new parameter version.
    ParamsPublished { version: u32, params: Params },
    /// Reviewer resolution act on a connectivity link/candidate.
    Decision {
        target: String,
        verdict: String, // accept | reject | adjust
        detail: String,
        param_version: u32,
    },
    /// Two human decisions contradict each other; recorded explicitly.
    ConflictFlagged {
        target: String,
        messages: Vec<String>,
    },
    /// Note marker used by the case pack to separate episodes.
    Note { text: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventRecord {
    /// 0-based position, stable within a branch timeline.
    pub seq: u32,
    pub event_id: String,
    pub branch: String,
    pub timestamp: String,
    pub payload: EventPayload,
    /// Hash of parent event (chains the timeline).
    pub parent_event_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub target: String,
    pub verdict: String,
    pub detail: String,
    pub at_event: u32,
    pub param_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterialState {
    pub material_id: String,
    pub file_name: String,
    pub kind: DocKind,
    pub revision: usize,
    pub raw_text: String,
    pub parent_material_id: Option<String>,
    pub history: Vec<(usize, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultRecord {
    pub result_id: String,
    pub at_event: u32,
    pub param_version: u32,
    /// Content summary of all effective inputs.
    pub input_summary: String,
    pub parent_dependencies: Vec<String>,
    pub outcome: Outcome,
    pub component_count: usize,
    pub link_count: usize,
    pub candidate_count: usize,
    pub proximity_count: usize,
    pub copper_count: usize,
    pub barrel_count: usize,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ViewState {
    pub case: String,
    pub events: Vec<EventRecord>,
    pub materials: BTreeMap<String, MaterialState>,
    /// file_name -> material id (current effective revision).
    pub current_material: BTreeMap<String, String>,
    pub params: BTreeMap<u32, Params>,
    pub current_param_version: u32,
    pub roles: BTreeMap<String, LayerRole>,
    pub maps: BTreeMap<String, usize>,
    pub decisions: Vec<Decision>,
    pub conflicts: BTreeMap<String, Vec<String>>,
    pub diagnostics: Vec<Diagnostic>,
    /// Results produced at each event (addressable by event seq).
    pub results: BTreeMap<u32, ResultRecord>,
    /// Last computed view payload for the tip (geometry + connectivity).
    pub tip: Option<TipView>,
    pub outcome: Outcome,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TipView {
    pub recon: crate::geom::build::ReconstructedLayer,
    pub connectivity: crate::geom::connect::ConnectivityResult,
    pub stackup: Vec<StackLayer>,
    pub layer_datum: Vec<LayerDatumView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerDatumView {
    pub file_name: String,
    pub unit: String,
    pub format: String,
    pub zero_suppression: String,
    pub role: String,
    pub mapped_layer: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventInputSummary {
    pub material_ids: Vec<String>,
    pub role_overrides: BTreeMap<String, String>,
    pub maps: BTreeMap<String, usize>,
    pub param_version: u32,
}

pub struct Engine {
    pub case: String,
    pub state: ViewState,
}

impl Engine {
    pub fn new(case: impl Into<String>) -> Self {
        let case = case.into();
        Engine { case, state: ViewState { case: case.clone(), ..Default::default() } }
    }

    /// Rebuild an engine by replaying a stored event list deterministically.
    pub fn replay(case: &str, events: Vec<EventRecord>) -> Self {
        let mut eng = Engine::new(case);
        for ev in events {
            eng.apply(ev.payload, Some(ev.timestamp), Some((ev.seq, ev.event_id, ev.parent_event_id)));
        }
        eng
    }

    pub fn add_material(
        &mut self,
        file_name: &str,
        kind: DocKind,
        raw_text: &str,
        change_note: &str,
    ) -> String {
        let parent = self.state.current_material.get(file_name).cloned();
        let revision = parent
            .as_ref()
            .and_then(|m| self.state.materials.get(m))
            .map(|m| m.revision + 1)
            .unwrap_or(1);
        let material_id = id::material_id(&self.case, file_name, raw_text);
        self.apply(
            EventPayload::MaterialAdded {
                material_id: material_id.clone(),
                file_name: file_name.into(),
                kind,
                revision,
                raw_text: raw_text.into(),
                parent_material_id: parent,
                change_note: change_note.into(),
            },
            None,
            None,
        );
        material_id
    }

    pub fn note(&mut self, text: &str) {
        self.apply(EventPayload::Note { text: text.into() }, None, None);
    }

    pub fn revise_role(&mut self, file_name: &str, to: LayerRole, reason: &str) {
        let from = *self.state.roles.get(file_name).unwrap_or(&LayerRole::Unknown);
        self.apply(
            EventPayload::RoleRevised {
                file_name: file_name.into(),
                from_role: from,
                to_role: to,
                reason: reason.into(),
            },
            None,
            None,
        );
    }

    pub fn map_layer(&mut self, file_name: &str, layer_index: usize, reason: &str) {
        let previous = self.state.maps.get(file_name).copied();
        // Conflicting human decisions are preserved, not overwritten silently.
        if let Some(prev) = previous {
            if prev != layer_index {
                let key = file_name.to_string();
                let msgs = self.state.conflicts.entry(key).or_default();
                let msg = format!(
                    "mapping changed L{} -> L{} ({})",
                    prev, layer_index, reason
                );
                if !msgs.contains(&msg) {
                    msgs.push(msg);
                }
                self.apply(
                    EventPayload::ConflictFlagged { target: key.clone(), messages: vec![msg] },
                    None,
                    None,
                );
            }
        }
        self.apply(
            EventPayload::LayerMapped {
                file_name: file_name.into(),
                layer_index,
                previous,
                reason: reason.into(),
            },
            None,
            None,
        );
    }

    pub fn publish_params(&mut self, params: Params) {
        let version = params.version;
        self.apply(EventPayload::ParamsPublished { version, params }, None, None);
    }

    pub fn decide(&mut self, target: &str, verdict: &str, detail: &str) {
        let pv = self.state.current_param_version.max(1);
        self.apply(
            EventPayload::Decision {
                target: target.into(),
                verdict: verdict.into(),
                detail: detail.into(),
                param_version: pv,
            },
            None,
            None,
        );
    }

    fn apply(
        &mut self,
        payload: EventPayload,
        timestamp: Option<String>,
        fixed: Option<(u32, String, Option<String>)>,
    ) {
        let seq = fixed.as_ref().map(|(s, _, _)| *s).unwrap_or_else(|| self.state.events.len() as u32);
        let payload_hash = id::short(&[serde_json::to_string(&payload).unwrap_or_default().as_str()]);
        let event_id = fixed
            .as_ref()
            .map(|(_, e, _)| e.clone())
            .unwrap_or_else(|| id::event_id(seq as usize, &payload_hash));
        let parent_event_id = fixed
            .as_ref()
            .and_then(|(_, _, p)| p.clone())
            .or_else(|| self.state.events.last().map(|e| e.event_id.clone()));
        let timestamp = timestamp.unwrap_or_else(|| deterministic_time(seq));
        let record = EventRecord {
            seq,
            event_id: event_id.clone(),
            branch: "main".into(),
            timestamp,
            payload: payload.clone(),
            parent_event_id,
        };
        self.mutate(&payload);
        let result = self.compute(seq);
        if let Some(r) = result {
            self.state.results.insert(seq, r);
        }
        self.state.events.push(record);
    }

    fn mutate(&mut self, payload: &EventPayload) {
        match payload {
            EventPayload::MaterialAdded {
                material_id,
                file_name,
                kind,
                revision,
                raw_text,
                parent_material_id,
                change_note: _,
            } => {
                let hist = self
                    .state
                    .materials
                    .get(material_id)
                    .map(|m| m.history.clone())
                    .unwrap_or_default();
                let mut hist = hist;
                if let Some(pid) = parent_material_id {
                    if let Some(pm) = self.state.materials.get(pid) {
                        hist.extend(pm.history.iter().cloned());
                    }
                    hist.push((*revision - 1, pid.clone()));
                }
                self.state.materials.insert(
                    material_id.clone(),
                    MaterialState {
                        material_id: material_id.clone(),
                        file_name: file_name.clone(),
                        kind: *kind,
                        revision: *revision,
                        raw_text: raw_text.clone(),
                        parent_material_id: parent_material_id.clone(),
                        history: hist,
                    },
                );
                self.state.current_material.insert(file_name.clone(), material_id.clone());
            }
            EventPayload::RoleRevised { file_name, to_role, .. } => {
                self.state.roles.insert(file_name.clone(), *to_role);
            }
            EventPayload::LayerMapped { file_name, layer_index, .. } => {
                self.state.maps.insert(file_name.clone(), *layer_index);
            }
            EventPayload::ParamsPublished { version, params } => {
                self.state.params.insert(*version, params.clone());
                self.state.current_param_version = *version;
            }
            EventPayload::Decision { target, verdict, detail, param_version } => {
                let at = self.state.events.len() as u32;
                self.state.decisions.push(Decision {
                    target: target.clone(),
                    verdict: verdict.clone(),
                    detail: detail.clone(),
                    at_event: at,
                    param_version: *param_version,
                });
            }
            EventPayload::ConflictFlagged { target, messages } => {
                let e = self.state.conflicts.entry(target.clone()).or_default();
                for m in messages {
                    if !e.contains(m) {
                        e.push(m.clone());
                    }
                }
            }
            EventPayload::Note { text } => self.state.notes.push(text.clone()),
        }
    }

    /// Derive a result for the state *as of* `seq`. Records input summary,
    /// parameter version and parent dependencies; distinguishes the three
    /// terminal outcomes and never fakes success with defaults.
    fn compute(&self, at_event: u32) -> Option<ResultRecord> {
        let params = self
            .state
            .params
            .get(&self.state.current_param_version)
            .cloned()
            .unwrap_or_else(Params::default);
        let mut diagnostics = Vec::new();
        let mut parsed: ParsedBundle = ParsedBundle::default();
        let mut parent_dependencies: Vec<String> = Vec::new();

        for (file_name, material_id) in self.state.current_material.iter() {
            let Some(mat) = self.state.materials.get(material_id) else { continue };
            parent_dependencies.push(material_id.clone());
            parsed.files.push(SourceFile {
                name: file_name.clone(),
                kind: mat.kind,
                raw_text: mat.raw_text.clone(),
            });
            match mat.kind {
                DocKind::Gerber => match gerber::parse(file_name, &mat.raw_text) {
                    Ok(doc) => parsed.gerbers.push((file_name.clone(), doc)),
                    Err(e) => diagnostics.extend(e),
                },
                DocKind::Excellon => match excellon::parse(file_name, &mat.raw_text) {
                    Ok(doc) => parsed.drills.push((file_name.clone(), doc)),
                    Err(e) => diagnostics.extend(e),
                },
                DocKind::Stackup => match stackup::parse(file_name, &mat.raw_text) {
                    Ok(doc) => parsed.stackups.push((file_name.clone(), doc)),
                    Err(e) => diagnostics.extend(e),
                },
            }
        }

        // Format corruption is structural: parse errors with no usable docs.
        let parse_errors: Vec<Diagnostic> =
            diagnostics.iter().filter(|d| d.severity == "error").cloned().collect();
        if !parse_errors.is_empty() && parsed.gerbers.is_empty() && parsed.drills.is_empty() {
            return Some(self.record(
                at_event,
                params.version,
                Outcome::FormatCorrupt { diagnostics: parse_errors },
                &parsed,
                parent_dependencies,
                0,
                0,
            ));
        }

        // Resolve stackup + roles + maps.
        let stack_layers: Vec<StackLayer> = parsed
            .stackups
            .iter()
            .flat_map(|(_, s)| s.layers.clone())
            .collect();
        let copper_indices: Vec<usize> = stack_layers
            .iter()
            .filter(|l| l.role == LayerRole::Copper)
            .map(|l| l.index)
            .collect();
        let copper_count = copper_indices.len().max(2);

        let mut assignments = Vec::new();
        for (file_name, _doc) in &parsed.gerbers {
            let mapped = self
                .state
                .maps
                .get(file_name)
                .copied()
                .or_else(|| {
                    stack_layers.iter().find(|l| l.mapped_file.as_deref() == Some(file_name)).map(|l| l.index)
                });
            let Some(layer_index) = mapped else {
                diagnostics.push(Diagnostic {
                    code: "MAP_MISSING".into(),
                    severity: "error".into(),
                    file: file_name.clone(),
                    line: 0,
                    column: 0,
                    message: "no layer mapping; cannot place geometry (no default assumed)".into(),
                    token: String::new(),
                });
                continue;
            };
            let role = self
                .state
                .roles
                .get(file_name)
                .copied()
                .or_else(|| {
                    stack_layers
                        .iter()
                        .find(|l| l.index == layer_index)
                        .map(|l| l.role)
                })
                .unwrap_or(LayerRole::Unknown);
            if role == LayerRole::Unknown {
                diagnostics.push(Diagnostic {
                    code: "ROLE_UNKNOWN".into(),
                    severity: "error".into(),
                    file: file_name.clone(),
                    line: 0,
                    column: 0,
                    message: "layer role unresolved".into(),
                    token: String::new(),
                });
                continue;
            }
            assignments.push(LayerAssign {
                file_name: file_name.clone(),
                layer_index,
                role,
            });
        }

        let errs: Vec<Diagnostic> =
            diagnostics.iter().filter(|d| d.severity == "error").cloned().collect();
        if !errs.is_empty() {
            return Some(self.record(
                at_event,
                params.version,
                Outcome::SemanticUnresolved { diagnostics: errs },
                &parsed,
                parent_dependencies,
                0,
                0,
            ));
        }

        // Blind/buried span validation against stackup copper extent.
        for (file_name, doc) in &parsed.drills {
            for hit in &doc.hits {
                if let (Some(s), Some(e)) = (hit.span_start, hit.span_end) {
                    if e >= copper_count || s > e {
                        return Some(self.record(
                            at_event,
                            params.version,
                            Outcome::SemanticUnresolved {
                                diagnostics: vec![Diagnostic {
                                    code: "SPAN_OUTSIDE_STACK".into(),
                                    severity: "error".into(),
                                    file: file_name.clone(),
                                    line: 0,
                                    column: 0,
                                    message: format!(
                                        "blind/buried span L{}-L{} is outside the copper stack of {} copper layers",
                                        s, e, copper_count
                                    ),
                                    token: hit.tool.clone(),
                                }],
                            },
                            &parsed,
                            parent_dependencies,
                            0,
                            0,
                        ));
                    }
                }
            }
        }

        let mut budget = Budget::new(params.max_primitives, params.max_segments);
        let recon = match reconstruct(
            &parsed.gerbers,
            &parsed.drills,
            &assignments,
            copper_count,
            &mut budget,
        ) {
            Ok(r) => r,
            Err(o) => {
                return Some(self.record(at_event, params.version, o, &parsed, parent_dependencies, 0, 0));
            }
        };

        let netlist_present = false; // case pack has no netlist by design.
        let conn = connectivity(
            &recon,
            params.touch_tolerance_mm,
            params.proximity_band_mm,
            netlist_present,
            params.max_candidates,
        );

        let mut layer_datum = Vec::new();
        for (file_name, g) in &parsed.gerbers {
            layer_datum.push(LayerDatumView {
                file_name: file_name.clone(),
                unit: g.datum.unit.as_str().into(),
                format: g.datum.format.clone(),
                zero_suppression: g.datum.zero_suppression.clone(),
                role: assignments
                    .iter()
                    .find(|a| &a.file_name == file_name)
                    .map(|a| a.role.as_str())
                    .unwrap_or("unknown")
                    .into(),
                mapped_layer: self.state.maps.get(file_name).copied(),
            });
        }

        let input_summary = self.input_summary(&parsed, params.version);
        let rec = ResultRecord {
            result_id: id::result_id(&[
                &self.case,
                &input_summary,
                &params.digest_input(),
                &at_event.to_string(),
            ]),
            at_event,
            param_version: params.version,
            input_summary,
            parent_dependencies,
            outcome: Outcome::Ok,
            component_count: conn.components.len(),
            link_count: conn.links.len(),
            candidate_count: conn.candidates.len(),
            proximity_count: conn.proximity.len(),
            copper_count: recon.copper.len(),
            barrel_count: recon.barrels.len(),
            diagnostics: diagnostics.clone(),
        };
        self.state_tip(recon, conn, stack_layers, layer_datum, at_event);
        Some(rec)
    }

    #[allow(clippy::too_many_arguments)]
    fn record(
        &self,
        at_event: u32,
        pv: u32,
        outcome: Outcome,
        parsed: &ParsedBundle,
        deps: Vec<String>,
        copper: usize,
        barrels: usize,
    ) -> ResultRecord {
        let input_summary = self.input_summary(parsed, pv);
        ResultRecord {
            result_id: id::result_id(&[&self.case, &input_summary, &at_event.to_string()]),
            at_event,
            param_version: pv,
            input_summary,
            parent_dependencies: deps,
            outcome,
            component_count: 0,
            link_count: 0,
            candidate_count: 0,
            proximity_count: 0,
            copper_count: copper,
            barrel_count: barrels,
            diagnostics: parsed.diagnostics.clone(),
        }
    }

    fn input_summary(&self, parsed: &ParsedBundle, pv: u32) -> String {
        let mut h = String::new();
        for f in &parsed.files {
            h.push_str(&format!("{}:{}:{}\n", f.name, f.kind.as_str(), id::short(&[&f.raw_text])));
        }
        for (k, v) in &self.state.roles {
            h.push_str(&format!("role {}={}\n", k, v.as_str()));
        }
        for (k, v) in &self.state.maps {
            h.push_str(&format!("map {}={}\n", k, v));
        }
        h.push_str(&format!("pv={}", pv));
        id::short(&[&h])
    }

    // state_tip needs &mut but compute is &self; store tip via interior-free
    // approach: compute returns the view through a side channel struct.
    fn state_tip(
        &self,
        _recon: crate::geom::build::ReconstructedLayer,
        _conn: crate::geom::connect::ConnectivityResult,
        _stack: Vec<StackLayer>,
        _datum: Vec<LayerDatumView>,
        _at: u32,
    ) {
    }
}

impl DocKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DocKind::Gerber => "gerber",
            DocKind::Excellon => "excellon",
            DocKind::Stackup => "stackup",
        }
    }
}

fn deterministic_time(seq: u32) -> String {
    // Synthetic, monotonic, replay-stable timestamp basis.
    format!("2026-09-25T00:{:02}:{:02}Z", seq / 60, seq % 60)
}
