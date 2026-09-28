//! Offline domain case pack, embedded into the binary.
//!
//! Builds the narrative on a main branch plus two replayable side branches:
//! * `tolerance` – one parameter revision (contact tolerance is versioned);
//! * `role-fix`   – a local source/material change plus a layer-role revision
//!                  and two mutually conflicting human decisions.

use crate::model::*;
use crate::parser::{parse_excellon, parse_gerber, parse_layer_mapping, parse_stackup};
use crate::project::*;
use sha2::{Digest, Sha256};

pub const TOP_COPPER: &str = include_str!("../cases/top_copper.gbr");
pub const INNER_COPPER: &str = include_str!("../cases/inner_copper.gbr");
pub const BOT_COPPER: &str = include_str!("../cases/bot_copper.gbr");
pub const TOP_MASK: &str = include_str!("../cases/top_mask.gbr");
pub const DRILL_THRU: &str = include_str!("../cases/drill_thru.drl");
pub const DRILL_BLIND: &str = include_str!("../cases/drill_blind.drl");
pub const DRILL_BADSPAN: &str = include_str!("../cases/drill_badspan.drl");
pub const BROKEN_COPPER: &str = include_str!("../cases/broken_copper.gbr");
pub const STACKUP: &str = include_str!("../cases/stackup.txt");
pub const LAYER_MAP: &str = include_str!("../cases/layer_map.txt");

pub fn hash_str(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn material(id: &str, filename: &str, doc: MaterialDoc, bytes: &str) -> Material {
    Material { id: id.into(), filename: filename.into(), doc, sha256: hash_str(bytes) }
}

struct Builder {
    events: Vec<Event>,
    seq_by_branch: std::collections::BTreeMap<String, u32>,
    last_by_branch: std::collections::BTreeMap<String, String>,
    clock: i64,
}

impl Builder {
    fn new() -> Self {
        Builder {
            events: Vec::new(),
            seq_by_branch: Default::default(),
            last_by_branch: Default::default(),
            clock: 1_759_000_000_000,
        }
    }
    fn push(&mut self, branch: &str, data: EventData) -> Event {
        self.clock += 1000;
        let seq = self.seq_by_branch.entry(branch.into()).or_insert(0);
        *seq += 1;
        let parent = self.last_by_branch.get(branch).cloned();
        let id = new_id("E");
        let ev = Event {
            id: id.clone(), seq: *seq, branch: branch.into(), parent,
            ts_millis: self.clock, actor: "seeder".into(), data,
            valid_from: self.clock, superseded_by: None,
        };
        self.last_by_branch.insert(branch.into(), id.clone());
        self.events.push(ev.clone());
        ev
    }
}

fn base_materials() -> Vec<(Material, &'static str)> {
    let st = parse_stackup(STACKUP);
    let mut mappings = parse_layer_mapping(LAYER_MAP);
    let mut mats: Vec<(Material, &'static str)> = vec![
        material("M_stack", "stackup.txt", MaterialDoc::Stackup(st), STACKUP),
        material("M_top", "top_copper.gbr", MaterialDoc::Gerber(parse_gerber(TOP_COPPER)), TOP_COPPER),
        material("M_inner", "inner_copper.gbr", MaterialDoc::Gerber(parse_gerber(INNER_COPPER)), INNER_COPPER),
        material("M_bot", "bot_copper.gbr", MaterialDoc::Gerber(parse_gerber(BOT_COPPER)), BOT_COPPER),
        material("M_mask", "top_mask.gbr", MaterialDoc::Gerber(parse_gerber(TOP_MASK)), TOP_MASK),
        material("M_thru", "drill_thru.drl", MaterialDoc::Excellon(parse_excellon(DRILL_THRU)), DRILL_THRU),
        material("M_blind", "drill_blind.drl", MaterialDoc::Excellon(parse_excellon(DRILL_BLIND)), DRILL_BLIND),
    ];
    // wire mappings to material ids by filename
    for m in &mats {
        for mp in &mut mappings {
            if mp.filename == m.0.filename && mp.material_id.is_empty() {
                mp.material_id = m.0.id.clone();
            }
        }
    }
    // through-all + blind drills need explicit manual layer mapping in case file
    for _ in 0..0 {}
    let _ = mappings;
    mats
}

fn analysis_event(state: &ProjectState, b: &mut Builder, branch: &str) {
    match state.compute() {
        Ok(result) => {
            b.push(branch, EventData::AnalysisRecorded { result, engine_error: None });
        }
        Err(crate::geometry::RunError::Semantic { contradictions }) => {
            b.push(branch, EventData::AnalysisRecorded {
                result: AnalysisResult {
                    features: vec![], holes: vec![], components: vec![],
                    contradictions, unresolved: vec![], params: state.params.clone(),
                    input_summary: "semantic-failure".into(),
                    parent_materials: vec![], parent_decisions: vec![],
                },
                engine_error: Some(EngineErrorView {
                    kind: "semantic".into(),
                    message: "no legal interpretation exists".into(),
                    detail: serde_json::json!({"outcome":"semantic_impossible"}),
                }),
            });
        }
        Err(crate::geometry::RunError::Budget { limit, used }) => {
            b.push(branch, EventData::AnalysisRecorded {
                result: AnalysisResult {
                    features: vec![], holes: vec![], components: vec![], contradictions: vec![],
                    unresolved: vec![], params: state.params.clone(),
                    input_summary: "budget-failure".into(),
                    parent_materials: vec![], parent_decisions: vec![],
                },
                engine_error: Some(EngineErrorView {
                    kind: "budget".into(),
                    message: format!("pair budget exhausted: {used} > {limit}"),
                    detail: serde_json::json!({"limit":limit,"used":used,"outcome":"budget_exhausted"}),
                }),
            });
        }
        Err(crate::geometry::RunError::Corrupt { material_id, diagnostics }) => {
            b.push(branch, EventData::AnalysisRecorded {
                result: AnalysisResult {
                    features: vec![], holes: vec![], components: vec![], contradictions: vec![],
                    unresolved: diagnostics, params: state.params.clone(),
                    input_summary: "corrupt-failure".into(),
                    parent_materials: vec![material_id], parent_decisions: vec![],
                },
                engine_error: Some(EngineErrorView {
                    kind: "corrupt".into(),
                    message: "source format corrupt".into(),
                    detail: serde_json::json!({"outcome":"format_corrupt"}),
                }),
            });
        }
    }
}

/// Build the seeded event graph. `project_id` is fixed so export/import keeps
/// every identity stable.
pub fn build_seed_events(project_id: &str) -> Vec<Event> {
    let mut b = Builder::new();
    let main = MAIN_BRANCH;

    b.push(main, EventData::ProjectCreated {
        name: "铜层连通谱：Gerber 图层、孔表与网络候选核验台".into(),
    });

    let mats = base_materials();
    // 1) immutable materials, imported in a stable order
    for (m, raw) in &mats {
        let kind = match m.doc {
            MaterialDoc::Gerber(_) => "gerber",
            MaterialDoc::Excellon(_) => "excellon",
            MaterialDoc::Stackup(_) => "stackup",
        };
        b.push(main, EventData::MaterialImported { material: m.clone(), source_kind: kind.into() });
        let _ = raw;
    }

    // 2) manual layer mapping (preserved verbatim from layer_map.txt)
    let mappings = parse_layer_mapping(LAYER_MAP);
    let mats_by_name: std::collections::BTreeMap<String, Material> =
        mats.iter().map(|(m, _)| (m.filename.clone(), m.clone())).collect();
    for mut mp in mappings {
        if let Some(m) = mats_by_name.get(&mp.filename) {
            mp.material_id = m.id.clone();
        }
        b.push(main, EventData::LayerMapped { mapping: mp });
    }

    // 3) baseline parameter version v1: exact contacts only
    b.push(main, EventData::ParamsRevised {
        from_version: 0,
        params: ParamVersion { version: 1, contact_tol_nm: 0, min_annular_nm: 0, pair_budget: 0,
                               note: "baseline: tangency only".into() },
    });

    // 4) baseline analysis
    let state = replay_branch(&b.events, project_id, main, None);
    analysis_event(&state, &mut b, main);

    // ---------------------------------------------------------------
    // Counter-example A: unit switch of one direct source (inch copy).
    // ---------------------------------------------------------------
    let inch_text = "G04 unit-switched copy of bottom copper*
%FSLAX25Y25*%
%MOIN*%
%TF.FileFunction,Copper,L3,Bot*%
%ADD10C,0.03937*%
D10*
X039370Y078740D03*
X078937Y078740D03*
X157480Y078740D03*
M02*
";
    let inch_mat = material("M_bot_in", "bot_copper_inch.gbr",
        MaterialDoc::Gerber(parse_gerber(inch_text)), inch_text);
    b.push(main, EventData::MaterialRevised {
        material: inch_mat, replaces: "M_bot".into(),
        scope: "single-source unit switch mm->inch; dependent contacts must recompute only on L4".into(),
    });
    // revert back to mm so the baseline story continues on the canonical source
    let bot_orig = mats.iter().find(|(m, _)| m.id == "M_bot").unwrap().0.clone();
    b.push(main, EventData::MaterialRevised {
        material: bot_orig, replaces: "M_bot_in".into(),
        scope: "revert unit-switch; restore mm source".into(),
    });
    let state = replay_branch(&b.events, project_id, main, None);
    analysis_event(&state, &mut b, main);

    // ---------------------------------------------------------------
    // Counter-example B: blind span off-board -> semantic impossibility.
    // Imported on a short-lived branch so the main review stays reviewable.
    // ---------------------------------------------------------------
    let bad = material("M_badspan", "drill_badspan.drl",
        MaterialDoc::Excellon(parse_excellon(DRILL_BADSPAN)), DRILL_BADSPAN);
    let bs = "span-error";
    // branch from main baseline analysis (current main head)
    let fork_from = b.last_by_branch.get(main).cloned();
    b.seq_by_branch.insert(bs.into(), 0);
    b.last_by_branch.insert(bs.into(), fork_from.clone().unwrap_or_default());
    b.push(bs, EventData::MaterialImported { material: bad, source_kind: "excellon".into() });
    let state = replay_branch(&b.events, project_id, bs, None);
    analysis_event(&state, &mut b, bs);

    // ---------------------------------------------------------------
    // Counter-example C: corrupt source -> distinct format-corrupt outcome.
    // ---------------------------------------------------------------
    let broken = material("M_broken", "broken_copper.gbr",
        MaterialDoc::Gerber(parse_gerber(BROKEN_COPPER)), BROKEN_COPPER);
    let br = "corrupt";
    b.seq_by_branch.insert(br.into(), 0);
    b.last_by_branch.insert(br.into(), fork_from.clone().unwrap_or_default());
    b.push(br, EventData::MaterialImported { material: broken, source_kind: "gerber".into() });
    let state = replay_branch(&b.events, project_id, br, None);
    analysis_event(&state, &mut b, br);

    // ---------------------------------------------------------------
    // Counter-example D: resource budget exhaustion (distinct outcome).
    // A parameter revision deliberately sets a tiny pairwise budget.
    // ---------------------------------------------------------------
    let bud = "budget";
    b.seq_by_branch.insert(bud.into(), 0);
    b.last_by_branch.insert(bud.into(), fork_from.clone().unwrap_or_default());
    b.push(bud, EventData::ParamsRevised {
        from_version: 1,
        params: ParamVersion { version: 9, contact_tol_nm: 0, min_annular_nm: 0,
                               pair_budget: 1,
                               note: "deliberately exhausted resource budget (1 pair)" .into() },
    });
    let state = replay_branch(&b.events, project_id, bud, None);
    analysis_event(&state, &mut b, bud);

    // ---------------------------------------------------------------
    // Branch 1: parameter revision raises tolerance -> near-miss pads become
    // legal candidates (still never forced to conduct).
    // ---------------------------------------------------------------
    let tol = "tolerance";
    b.seq_by_branch.insert(tol.into(), 0);
    b.last_by_branch.insert(tol.into(), fork_from.clone().unwrap_or_default());
    b.push(tol, EventData::ParamsRevised {
        from_version: 1,
        params: ParamVersion { version: 2, contact_tol_nm: 100_000, min_annular_nm: 0, pair_budget: 0,
                               note: "raise contact tolerance to 0.1mm for gap review".into() },
    });
    let state = replay_branch(&b.events, project_id, tol, None);
    analysis_event(&state, &mut b, tol);

    // human accepts the candidate bridge, then a conflicting decision rejects it
    let state = replay_branch(&b.events, project_id, tol, None);
    let ambiguous_component = state.latest_result.as_ref()
        .and_then(|r| r.components.iter().find(|c| c.ambiguity.is_some()))
        .cloned();
    if let Some(comp) = &ambiguous_component {
        b.push(tol, EventData::NetDecision {
            component_id: comp.id.clone(),
            chosen_alternative: vec![comp.feature_ids.join(",")],
            accepted: true,
            note: "reviewer accepts the within-tolerance bridge as intended".into(),
        });
        // conflicting human decision on the same component
        b.push(tol, EventData::NetDecision {
            component_id: comp.id.clone(),
            chosen_alternative: comp.feature_ids.iter().map(|f| vec![f.clone()]).flatten().collect::<Vec<_>>().chunks(1)
                .map(|c| c.join(",")).collect(),
            accepted: false,
            note: "CONFLICTING decision: proximity must not be read as conduction".into(),
        });
    }
    let state = replay_branch(&b.events, project_id, tol, None);
    analysis_event(&state, &mut b, tol);

    // ---------------------------------------------------------------
    // Branch 2: local source change affecting only local dependencies +
    // layer-role revision (mask mislabeled as copper -> solder_mask).
    // ---------------------------------------------------------------
    let rf = "role-fix";
    b.seq_by_branch.insert(rf.into(), 0);
    b.last_by_branch.insert(rf.into(), fork_from.clone().unwrap_or_default());

    // mislabeled copy first: mask file role deliberately read as copper
    let mut mask_doc = parse_gerber(TOP_MASK);
    mask_doc.role = LayerRole::Copper;
    let mask_mis = material("M_mask_mis", "top_mask_mislabeled.gbr",
        MaterialDoc::Gerber(mask_doc.clone()), TOP_MASK);
    b.push(rf, EventData::MaterialImported {
        material: mask_mis.clone(), source_kind: "gerber".into(),
    });
    b.push(rf, EventData::LayerMapped { mapping: LayerMapping {
        material_id: "M_mask_mis".into(), filename: "top_mask_mislabeled.gbr".into(),
        physical_index: Some(0), raw_declaration: "MAP top_mask_mislabeled.gbr L1".into(),
    }});
    let state = replay_branch(&b.events, project_id, rf, None);
    analysis_event(&state, &mut b, rf);

    // the role revision decision: copper -> solder_mask (only this material changes)
    b.push(rf, EventData::LayerRoleRevised {
        material_id: "M_mask_mis".into(),
        from_role: LayerRole::Copper,
        to_role: LayerRole::SolderMask,
        reason: "file function is a mask opening; wrong role inflated copper area".into(),
    });
    let state = replay_branch(&b.events, project_id, rf, None);
    analysis_event(&state, &mut b, rf);

    // local source change: move only one pad 0.05mm; dependencies local to L1
    let local_edit = "G04 one-pad local edit*
%FSLAX36Y36*%
%MOMM*%
%TF.FileFunction,Copper,L1,Top*%
%ADD10C,1.000mm*%
%ADD11R,1.000mmX1.000mm*%
%ADD12C,0.200mm*%
D10*
X1000000Y2050000D03*
X2000000Y2000000D03*
D12*
X01000000Y0205000D02*
X02000000Y02000000D01*
M02*
";
    let edited = material("M_top_v2", "top_copper.gbr",
        MaterialDoc::Gerber(parse_gerber(local_edit)), local_edit);
    b.push(rf, EventData::MaterialRevised {
        material: edited, replaces: "M_top".into(),
        scope: "one pad moved +50um Y; only L1 contacts/dependents recompute".into(),
    });
    let state = replay_branch(&b.events, project_id, rf, None);
    analysis_event(&state, &mut b, rf);

    b.events
}
