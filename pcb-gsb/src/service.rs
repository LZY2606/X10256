//! Axum service layer. All business rules run server-side through the event
//! store and engine; the browser is a renderer/controller.

use crate::project::*;
use crate::seed::build_seed_events;
use crate::storage::{ExportBundle, RecoveryReport, Store};
use crate::{geometry, model::*};
use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::Mutex;

pub const PROJECT_ID: &str = "P_demo_pcb_connectivity";

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Mutex<Store>>,
    pub recovery: Arc<Mutex<Option<RecoveryReport>>>,
}

#[derive(Debug, Serialize)]
struct ApiError {
    error: String,
    code: String,
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (StatusCode::BAD_REQUEST, Json(self)).into_response()
    }
}
fn ape(msg: impl Into<String>) -> ApiError {
    ApiError { error: msg.into(), code: "bad_request".into() }
}

#[derive(Deserialize)]
struct BranchQuery {
    branch: Option<String>,
    at: Option<String>,
}

#[derive(Serialize)]
struct ViewResponse {
    project_id: String,
    branch: String,
    at_event: Option<String>,
    name: String,
    branches: Vec<String>,
    events: Vec<EventView>,
    result: Option<AnalysisResult>,
    engine_error: Option<EngineErrorView>,
    params: ParamVersion,
    stackup: StackupView,
    materials: Vec<MaterialView>,
    recovery: Option<RecoveryReport>,
}

#[derive(Serialize)]
struct EventView {
    id: String,
    seq: u32,
    branch: String,
    parent: Option<String>,
    ts_millis: i64,
    valid_from: i64,
    superseded_by: Option<String>,
    #[serde(rename = "type")]
    data_type: String,
    data: EventData,
}

#[derive(Serialize)]
struct StackupView {
    layers: Vec<StackLayer>,
}

#[derive(Serialize)]
struct MaterialView {
    id: String,
    filename: String,
    sha256: String,
    source_kind: String,
    datum: Option<CoordDatum>,
    role: Option<LayerRole>,
    mapped_layer: Option<u32>,
    error_count: usize,
    warnings: Vec<Diagnostic>,
}

fn event_view(e: &Event) -> EventView {
    let t = match &e.data {
        EventData::ProjectCreated { .. } => "project_created",
        EventData::MaterialImported { .. } => "material_imported",
        EventData::LayerMapped { .. } => "layer_mapped",
        EventData::LayerRoleRevised { .. } => "layer_role_revised",
        EventData::MaterialRevised { .. } => "material_revised",
        EventData::ParamsRevised { .. } => "params_revised",
        EventData::NetDecision { .. } => "net_decision",
        EventData::AnalysisRecorded { .. } => "analysis_recorded",
    };
    EventView {
        id: e.id.clone(), seq: e.seq, branch: e.branch.clone(), parent: e.parent.clone(),
        ts_millis: e.ts_millis, valid_from: e.valid_from, superseded_by: e.superseded_by.clone(),
        data_type: t.into(), data: e.data.clone(),
    }
}

async fn view(State(st): State<AppState>, Query(q): Query<BranchQuery>) -> Result<Json<ViewResponse>, ApiError> {
    let branch = q.branch.unwrap_or_else(|| MAIN_BRANCH.into());
    let store = st.store.lock().await;
    let all = store.load_all_events().map_err(|e| ape(e.to_string()))?;
    let branches = store.list_branches().map_err(|e| ape(e.to_string()))?;

    // Replay to a point in time: main spine + branch events up to `at`.
    let mut state = ProjectState::new(PROJECT_ID.into(), "untitled".into());
    let keep = branch_event_filter(&branch);
    let mut ordered: Vec<&Event> = all.iter().filter(|e| keep(e)).collect();
    ordered.sort_by_key(|e| (if e.branch == MAIN_BRANCH { 0 } else { 1 }, e.seq));
    for ev in ordered {
        state.apply(ev);
        if let Some(at) = &q.at {
            if &ev.id == at {
                break;
            }
        }
    }

    // Compute the result *as of that point* using the recorded result when the
    // last applied event is an analysis; otherwise compute on demand. History
    // is never overwritten by a newer result.
    let recorded = all.iter().filter(|e| keep(e))
        .take_while(|e| q.at.as_ref().map_or(true, |at| &e.id != at))
        .filter_map(|e| if let EventData::AnalysisRecorded { result, engine_error } = &e.data {
            Some((result.clone(), engine_error.clone()))
        } else { None })
        .last();
    let (result, engine_error) = if let Some(r) = recorded {
        (Some(r.0), r.1)
    } else {
        match state.compute() {
            Ok(r) => (Some(r), None),
            Err(geometry::RunError::Semantic { contradictions }) => (None, Some(EngineErrorView {
                kind: "semantic".into(), message: "no legal interpretation".into(),
                detail: serde_json::json!({"contradictions": contradictions}),
            })),
            Err(geometry::RunError::Corrupt { material_id, diagnostics }) => (None, Some(EngineErrorView {
                kind: "corrupt".into(), message: format!("corrupt source {material_id}"),
                detail: serde_json::json!({"diagnostics": diagnostics}),
            })),
            Err(geometry::RunError::Budget { limit, used }) => (None, Some(EngineErrorView {
                kind: "budget".into(), message: format!("budget exhausted {used}>{limit}"),
                detail: serde_json::json!({"limit":limit,"used":used}),
            })),
        }
    };

    let mut stackup = StackupView { layers: vec![] };
    let mut materials = Vec::new();
    for m in &state.materials {
        let (kind, datum, role, errs, warns) = match &m.doc {
            MaterialDoc::Gerber(g) => ("gerber", Some(g.datum.clone()), Some(g.role),
                g.warnings.iter().filter(|d| d.severity == Severity::Error).count(), g.warnings.clone()),
            MaterialDoc::Excellon(e) => ("excellon", Some(e.datum.clone()), Some(LayerRole::Drill),
                e.warnings.iter().filter(|d| d.severity == Severity::Error).count(), e.warnings.clone()),
            MaterialDoc::Stackup(s) => {
                stackup = StackupView { layers: s.layers.clone() };
                ("stackup", None, None,
                 s.warnings.iter().filter(|d| d.severity == Severity::Error).count(), s.warnings.clone())
            }
        };
        let mapped_layer = state.mapping.iter().find(|mp| mp.material_id == m.id).and_then(|mp| mp.physical_index);
        materials.push(MaterialView {
            id: m.id.clone(), filename: m.filename.clone(), sha256: m.sha256.clone(),
            source_kind: kind.into(), datum, role, mapped_layer, error_count: errs, warnings: warns,
        });
    }

    let branch_events: Vec<EventView> = all.iter().filter(|e| keep(e))
        .filter(|e| q.at.as_ref().map_or(true, |at| e.ts_millis <= {
            let target = all.iter().find(|x| &x.id == at).map(|x| x.ts_millis).unwrap_or(i64::MAX);
            let _ = target; true
        }))
        .map(event_view).collect();

    let recovery = st.recovery.lock().await.clone();
    Ok(Json(ViewResponse {
        project_id: PROJECT_ID.into(), branch, at_event: q.at.clone(), name: state.name,
        branches, events: branch_events, result, engine_error,
        params: state.params.clone(), stackup, materials, recovery,
    }))
}

#[derive(Deserialize)]
struct ImportBody {
    filename: String,
    content: String,
    layer_index: Option<u32>,
}

fn now_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn classify_and_parse(filename: &str, content: &str) -> (MaterialDoc, String) {
    let lf = filename.to_ascii_lowercase();
    if lf.ends_with(".drl") || lf.contains("drill") {
        (MaterialDoc::Excellon(crate::parser::parse_excellon(content)), "excellon".into())
    } else if lf.contains("stackup") || lf.ends_with(".txt") && lf.contains("stack") {
        (MaterialDoc::Stackup(crate::parser::parse_stackup(content)), "stackup".into())
    } else {
        (MaterialDoc::Gerber(crate::parser::parse_gerber(content)), "gerber".into())
    }
}

async fn append_with_analysis(st: &AppState, branch: String, new_events: Vec<EventData>)
    -> Result<Vec<Event>, ApiError> {
    let mut store = st.store.lock().await;
    let all = store.load_all_events().map_err(|e| ape(e.to_string()))?;
    let seq = all.iter().filter(|e| e.branch == branch).count() as u32;
    let parent = all.iter().filter(|e| e.branch == branch).map(|e| e.id.clone()).last();
    let mut ts = now_millis();
    let mut made = Vec::new();
    for (i, data) in new_events.into_iter().enumerate() {
        let ev = Event {
            id: new_id("E"), seq: seq + i as u32 + 1, branch: branch.clone(),
            parent: if i == 0 { parent.clone() } else { made.last().map(|e: &Event| e.id.clone()) },
            ts_millis: ts, actor: "reviewer".into(), data, valid_from: ts, superseded_by: None,
        };
        ts += 1;
        made.push(ev);
    }
    // snapshot state before analysis for recomputation
    store.append_events(&made, false).map_err(|e| ape(e.to_string()))?;

    // recompute at branch head and append an analysis event
    let all2 = store.load_all_events().map_err(|e| ape(e.to_string()))?;
    let state = replay_branch(&all2, PROJECT_ID, &branch, None);
    let aev = match state.compute() {
        Ok(result) => EventData::AnalysisRecorded { result, engine_error: None },
        Err(geometry::RunError::Semantic { contradictions }) => EventData::AnalysisRecorded {
            result: AnalysisResult { features: vec![], holes: vec![], components: vec![],
                contradictions, unresolved: vec![], params: state.params.clone(),
                input_summary: "semantic".into(), parent_materials: vec![], parent_decisions: vec![] },
            engine_error: Some(EngineErrorView { kind: "semantic".into(),
                message: "no legal interpretation".into(),
                detail: serde_json::json!({"outcome":"semantic_impossible"}) }),
        },
        Err(geometry::RunError::Corrupt { material_id, diagnostics }) => EventData::AnalysisRecorded {
            result: AnalysisResult { features: vec![], holes: vec![], components: vec![],
                contradictions: vec![], unresolved: diagnostics, params: state.params.clone(),
                input_summary: "corrupt".into(), parent_materials: vec![material_id], parent_decisions: vec![] },
            engine_error: Some(EngineErrorView { kind: "corrupt".into(),
                message: "format corrupt".into(),
                detail: serde_json::json!({"outcome":"format_corrupt"}) }),
        },
        Err(geometry::RunError::Budget { limit, used }) => EventData::AnalysisRecorded {
            result: AnalysisResult { features: vec![], holes: vec![], components: vec![],
                contradictions: vec![], unresolved: vec![], params: state.params.clone(),
                input_summary: "budget".into(), parent_materials: vec![], parent_decisions: vec![] },
            engine_error: Some(EngineErrorView { kind: "budget".into(),
                message: format!("{used}>{limit}"),
                detail: serde_json::json!({"outcome":"budget_exhausted","limit":limit,"used":used}) }),
        },
    };
    let seq2 = all2.iter().filter(|e| e.branch == branch).count() as u32;
    let ev = Event {
        id: new_id("E"), seq: seq2 + 1, branch: branch.clone(),
        parent: made.last().map(|e| e.id.clone()).or(parent.clone()),
        ts_millis: ts + 1, actor: "reviewer".into(), data: aev,
        valid_from: ts + 1, superseded_by: None,
    };
    store.append_events(&[ev], false).map_err(|e| ape(e.to_string()))?;
    made.push(ev);
    Ok(made)
}

async fn import_material(State(st): State<AppState>,
                         Query(q): Query<BranchQuery>,
                         Json(body): Json<ImportBody>) -> Result<Json<serde_json::Value>, ApiError> {
    let branch = q.branch.unwrap_or_else(|| MAIN_BRANCH.into());
    let (doc, kind) = classify_and_parse(&body.filename, &body.content);
    let id = new_id("M");
    let material = Material { id: id.clone(), filename: body.filename.clone(), doc,
                               sha256: sha256_hex(body.content.as_bytes()) };
    let mut evs = vec![EventData::MaterialImported { material, source_kind: kind }];
    if let Some(idx) = body.layer_index {
        evs.push(EventData::LayerMapped { mapping: LayerMapping {
            material_id: id.clone(), filename: body.filename.clone(),
            physical_index: Some(idx),
            raw_declaration: format!("MAP {} L{}", body.filename, idx + 1),
        }});
    }
    let made = append_with_analysis(&st, branch, evs).await?;
    Ok(Json(serde_json::json!({"created": made.iter().map(|e| e.id.clone()).collect::<Vec<_>>()})))
}

#[derive(Deserialize)]
struct ParamsBody {
    contact_tol_nm: Option<i64>,
    pair_budget: Option<u64>,
    note: Option<String>,
}

async fn revise_params(State(st): State<AppState>,
                       Query(q): Query<BranchQuery>,
                       Json(body): Json<ParamsBody>) -> Result<Json<serde_json::Value>, ApiError> {
    let branch = q.branch.unwrap_or_else(|| MAIN_BRANCH.into());
    let store = st.store.lock().await;
    let all = store.load_all_events().map_err(|e| ape(e.to_string()))?;
    let state = replay_branch(&all, PROJECT_ID, &branch, None);
    drop(store);
    let next_version = state.params.version + 1;
    let params = ParamVersion {
        version: next_version,
        contact_tol_nm: body.contact_tol_nm.unwrap_or(state.params.contact_tol_nm),
        min_annular_nm: state.params.min_annular_nm,
        pair_budget: body.pair_budget.unwrap_or(state.params.pair_budget),
        note: body.note.unwrap_or_else(|| format!("revision v{next_version}")),
    };
    let made = append_with_analysis(&st, branch,
        vec![EventData::ParamsRevised { from_version: state.params.version, params }]).await?;
    Ok(Json(serde_json::json!({"created": made.iter().map(|e| e.id.clone()).collect::<Vec<_>>()})))
}

#[derive(Deserialize)]
struct NetDecisionBody {
    component_id: String,
    accepted: bool,
    chosen_alternative: Vec<String>,
    note: String,
}

async fn net_decision(State(st): State<AppState>,
                      Query(q): Query<BranchQuery>,
                      Json(body): Json<NetDecisionBody>) -> Result<Json<serde_json::Value>, ApiError> {
    let branch = q.branch.unwrap_or_else(|| MAIN_BRANCH.into());
    let made = append_with_analysis(&st, branch, vec![EventData::NetDecision {
        component_id: body.component_id,
        chosen_alternative: body.chosen_alternative,
        accepted: body.accepted,
        note: body.note,
    }]).await?;
    Ok(Json(serde_json::json!({"created": made.iter().map(|e| e.id.clone()).collect::<Vec<_>>()})))
}

#[derive(Deserialize)]
struct RoleBody {
    material_id: String,
    to_role: String,
    reason: String,
}

async fn revise_role(State(st): State<AppState>,
                     Query(q): Query<BranchQuery>,
                     Json(body): Json<RoleBody>) -> Result<Json<serde_json::Value>, ApiError> {
    let branch = q.branch.unwrap_or_else(|| MAIN_BRANCH.into());
    let store = st.store.lock().await;
    let all = store.load_all_events().map_err(|e| ape(e.to_string()))?;
    let state = replay_branch(&all, PROJECT_ID, &branch, None);
    drop(store);
    let mat = state.materials.iter().find(|m| m.id == body.material_id)
        .ok_or_else(|| ape("material not found"))?;
    let from_role = if let MaterialDoc::Gerber(g) = &mat.doc {
        state.roles.by_material.get(&mat.id).copied().unwrap_or(g.role)
    } else { LayerRole::Unknown };
    let to_role = LayerRole::parse(&body.to_role);
    let made = append_with_analysis(&st, branch, vec![EventData::LayerRoleRevised {
        material_id: body.material_id, from_role, to_role, reason: body.reason,
    }]).await?;
    Ok(Json(serde_json::json!({"created": made.iter().map(|e| e.id.clone()).collect::<Vec<_>>()})))
}

async fn export_bundle(State(st): State<AppState>) -> Result<Response, ApiError> {
    let store = st.store.lock().await;
    let bundle = store.export(PROJECT_ID.into(), now_millis()).map_err(|e| ape(e.to_string()))?;
    let json = serde_json::to_vec_pretty(&bundle).map_err(|e| ape(e.to_string()))?;
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json"),
         (header::CONTENT_DISPOSITION, "attachment; filename=\"pcb-gsb-export.json\"")],
        json,
    ).into_response())
}

async fn import_bundle(State(st): State<AppState>,
                       Json(bundle): Json<ExportBundle>) -> Result<Json<serde_json::Value>, ApiError> {
    let mut store = st.store.lock().await;
    store.import_bundle(&bundle).map_err(|e| ape(e.to_string()))?;
    Ok(Json(serde_json::json!({"imported_events": bundle.events.len(), "project_id": bundle.project_id})))
}

async fn recovery_status(State(st): State<AppState>) -> Json<serde_json::Value> {
    let r = st.recovery.lock().await.clone();
    Json(serde_json::to_value(r).unwrap_or(serde_json::json!(null)))
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"ok": true, "service": "pcb-gsb"}))
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/view", get(view))
        .route("/api/import", post(import_material))
        .route("/api/params", post(revise_params))
        .route("/api/net-decision", post(net_decision))
        .route("/api/role", post(revise_role))
        .route("/api/export", get(export_bundle))
        .route("/api/import-bundle", post(import_bundle))
        .route("/api/recovery", get(recovery_status))
        .fallback_service(tower_http::services::ServeDir::new("static"))
        .with_state(state)
}
