//! 派生层：把不可变材料重放为结构化的连通核验状态。
//!
//! 所有计算结果都记录输入摘要、参数版本与父级依赖；无 netlist 时
//! 只输出连通候选，邻近（gap 大于版本化容差）永远不被当作导通。

use crate::geometry::*;
use crate::model::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 版本化几何参数。容差是版本参数，修改即产生新规则版本。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Params {
    /// 几何接触容差（mm）。gap == 0 为硬接触；0 < gap <= tol 为候选。
    pub contact_tolerance_mm: f64,
    pub rule_version: String,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            contact_tolerance_mm: 0.05,
            rule_version: "rules/v1".to_string(),
        }
    }
}

/// 一份材料在某个版本下的解析快照（不可变）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialVersion {
    pub filename: String,
    pub kind: String,
    pub version: i64,
    pub raw_text: String,
    pub parsed: ParsedFile,
    pub imported_event_id: String,
}

/// 事件（导入 / 参数修订 / 源数据修订 / 人工决定 / 层角色修订）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    /// 父事件（分支依赖）。
    pub parent_id: Option<String>,
    pub seq: i64,
    pub kind: String,
    pub label: String,
    pub timestamp: String,
    /// 结构化负载。
    pub payload: serde_json::Value,
}

/// 规范化后的孔（mm 坐标，span 为铜层索引）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormHole {
    pub id: String,
    pub x: f64,
 pub y: f64,
    pub diameter_mm: f64,
    pub plated: bool,
    pub span: Vec<usize>,
    pub raw_span: String,
    pub source_file: String,
    pub source_line: i64,
    pub unit: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EdgeKind {
    /// 几何接触（gap == 0），无条件导通。
    Hard,
    /// 0 < gap <= 容差：需要人工决定的候选。
    Tolerance,
    /// gap > 容差：仅邻近提示，永远不导通。
    Proximity,
}

impl EdgeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EdgeKind::Hard => "hard",
            EdgeKind::Tolerance => "tolerance",
            EdgeKind::Proximity => "proximity",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub node_type: String, // copper | hole
    pub layer: Option<usize>,
    pub label: String,
    /// 原始来源（文件、行、操作序号）。
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphEdge {
    pub id: String,
    pub a: String,
    pub b: String,
    pub gap_mm: f64,
    pub kind: String,
    pub layers: Vec<usize>,
}

/// 一份合法候选解：硬边 + 被采纳的容差边。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateSolution {
    pub id: String,
    pub accepted_tolerance_edges: Vec<String>,
    pub components: BTreeMap<String, Vec<String>>,
}

/// 矛盾位置：互相冲突的人工决定落在同一候选边。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Contradiction {
    pub edge_id: String,
    pub x: f64,
    pub y: f64,
    pub message: String,
}

/// 派生结果（一次重放的产物）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Derived {
    /// 输入摘要（每个文件的版本 + 原始 sha）。
    pub input_digest: BTreeMap<String, String>,
    pub rule_version: String,
    pub contact_tolerance_mm: f64,
    pub parent_event_id: String,
    pub layers: Vec<LayerView>,
    pub copper: Vec<CopperRect>,
    pub masks: Vec<MaskOpen>,
    pub holes: Vec<NormHole>,
    pub nodes: Vec<GraphNode>,
    pub hard_edges: Vec<GraphEdge>,
    pub tolerance_edges: Vec<GraphEdge>,
    pub proximity_edges: Vec<GraphEdge>,
    pub solutions: Vec<CandidateSolution>,
    pub components: BTreeMap<String, Vec<String>>,
    pub contradictions: Vec<Contradiction>,
    pub diagnostics: Vec<Diagnostic>,
    /// 有效区间（时间基准）：该派生结果由哪条事件产生。
    pub valid_from_event: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayerView {
    pub index: usize,
    pub role: String,
    pub file: String,
    pub role_source: String,
}

/// 重放工作区（事件发生时即时推进的可变状态）。
#[derive(Debug, Clone)]
pub struct Replay {
    pub materials: BTreeMap<String, MaterialVersion>,
    pub events: Vec<Event>,
    pub params: Params,
    /// 分支上被采纳/否定的容差边：edge_id -> (accepted, event_id)。
    pub decisions: BTreeMap<String, (bool, String)>,
    /// 手工层角色修订：file -> (role, event_id)。
    pub role_overrides: BTreeMap<String, (LayerRole, String)>,
    /// 每个事件对应的派生快照 id（由存储层填充）。
    pub derived_at: BTreeMap<String, String>,
}

impl Replay {
    pub fn new() -> Self {
        Replay {
            materials: BTreeMap::new(),
            events: Vec::new(),
            params: Params::default(),
            decisions: BTreeMap::new(),
            role_overrides: BTreeMap::new(),
            derived_at: BTreeMap::new(),
        }
    }

    pub fn head_event(&self) -> Option<&Event> {
        self.events.last()
    }

    /// 追加事件并应用其对工作区状态的影响。
    pub fn apply(&mut self, ev: Event) {
        match ev.kind.as_str() {
            "import_material" => {
                let filename = ev.payload["filename"].as_str().unwrap_or("").to_string();
                let raw_text = ev.payload["raw_text"].as_str().unwrap_or("").to_string();
                let parsed =
                    serde_json::from_value::<ParsedFile>(ev.payload["parsed"].clone()).unwrap();
                let version = self.materials.get(&filename).map_or(1, |m| m.version + 1);
                self.materials.insert(
                    filename.clone(),
                    MaterialVersion {
                        filename,
                        kind: parsed.kind.clone(),
                        version,
                        raw_text,
                        parsed,
                        imported_event_id: ev.id.clone(),
                    },
                );
            }
            "param_revision" => {
                if let Some(t) = ev.payload["contact_tolerance_mm"].as_f64() {
                    self.params.contact_tolerance_mm = t;
                }
                if let Some(v) = ev.payload["rule_version"].as_str() {
                    self.params.rule_version = v.to_string();
                }
            }
            "decision" => {
                if let Some(edge) = ev.payload["edge_id"].as_str() {
                    let accepted = ev.payload["accepted"].as_bool().unwrap_or(false);
                    self.decisions
                        .insert(edge.to_string(), (accepted, ev.id.clone()));
                }
            }
            "role_revision" => {
                if let Some(file) = ev.payload["file"].as_str() {
                    if let Some(role) = ev.payload["role"].as_str().and_then(LayerRole::parse) {
                        self.role_overrides
                            .insert(file.to_string(), (role, ev.id.clone()));
                    }
                }
            }
            _ => {}
        }
        self.events.push(ev);
    }

    /// 从不可变材料 + 事件重放到指定事件（不含之后的分支）。
    pub fn derive_at(&self, event_id: &str) -> Derived {
        let mut view = Replay::new();
        for ev in &self.events {
            view.apply(ev.clone());
            if ev.id == event_id {
                break;
            }
        }
        compute(&view, event_id)
    }
}

impl Default for Replay {
    fn default() -> Self {
        Self::new()
    }
}

/// 派生层主计算：材料 -> 规范化几何 -> 连通图 -> 候选解。
pub fn compute(view: &Replay, parent_event_id: &str) -> Derived {
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut input_digest = BTreeMap::new();

    // 1) 层角色：层叠手工映射 + 事件修订（修订优先）。
    let stackup_file = view
        .materials
        .values()
        .find(|m| m.kind == FileKind::Stackup.as_str());
    let mut role_map: BTreeMap<String, (LayerRole, String)> = BTreeMap::new();
    if let Some(mat) = stackup_file {
        if let Some(doc) = &mat.parsed.stackup {
            for e in &doc.entries {
                if let Some(role) = LayerRole::parse(&e.role) {
                    role_map.insert(
                        e.file.clone(),
                        (
                            role,
                            format!("{}#L{}", mat.filename, e.source_line),
                        ),
                    );
                }
            }
        }
        input_digest.insert(
            mat.filename.clone(),
            format!("v{}:{}", mat.version, digest_str(&mat.raw_text)),
        );
    }
    for (file, (role, ev)) in &view.role_overrides {
        role_map.insert(file.clone(), (*role, format!("event:{ev}")));
    }

    // 2) 铜层与阻焊按角色顺序规范化。
    let layer_order = [
        LayerRole::CopperTop,
        LayerRole::CopperInner,
        LayerRole::CopperBottom,
    ];
    let mut layers = Vec::new();
    let mut copper = Vec::new();
    let mut masks = Vec::new();
    let mut copper_layer_of_file: BTreeMap<String, usize> = BTreeMap::new();

    let mut gerbers: Vec<(&MaterialVersion, LayerRole, String)> = Vec::new();
    for (file, (role, src)) in &role_map {
        if let Some(mat) = view.materials.get(file) {
            if mat.kind == FileKind::Gerber.as_str() {
                input_digest.entry(file.clone()).or_insert_with(|| {
                    format!("v{}:{}", mat.version, digest_str(&mat.raw_text))
                });
                gerbers.push((mat, *role, src.clone()));
                diagnostics.extend(mat.parsed.diagnostics.iter().cloned());
            } else {
                diagnostics.push(Diagnostic {
                    kind: DiagnosticKind::SemanticUnresolved,
                    file: file.clone(),
                    line: 0,
                    code: "ROLE-KIND-MISMATCH".to_string(),
                    message: "层角色指派给了非 Gerber 文件".to_string(),
                    raw_snippet: String::new(),
                });
            }
        } else {
            diagnostics.push(Diagnostic {
                kind: DiagnosticKind::SemanticUnresolved,
                file: file.clone(),
                line: 0,
                code: "MATERIAL-MISSING".to_string(),
                message: "映射引用的文件尚未导入".to_string(),
                raw_snippet: String::new(),
            });
        }
    }

    gerbers.sort_by_key(|(mat, role, _)| {
        (
            layer_order
                .iter()
                .position(|r| r == role)
                .unwrap_or(99),
            mat.filename.clone(),
        )
    });

    let mut copper_index = 0usize;
    for (mat, role, role_source) in gerbers {
        if let Some(doc) = &mat.parsed.gerber {
            if role.is_copper() {
                let idx = copper_index;
                copper_layer_of_file.insert(mat.filename.clone(), idx);
                layers.push(LayerView {
                    index: idx,
                    role: role.as_str().to_string(),
                    file: mat.filename.clone(),
                    role_source: role_source.clone(),
                });
                copper.extend(compose_gerber(doc, role, idx));
                copper_index += 1;
            } else {
                let mask_idx = match role {
                    LayerRole::SolderMaskTop => 0,
                    LayerRole::SolderMaskBottom => copper_index,
                    _ => 0,
                };
                masks.extend(compose_mask(doc, role, mask_idx));
                layers.push(LayerView {
                    index: mask_idx,
                    role: role.as_str().to_string(),
                    file: mat.filename.clone(),
                    role_source,
                });
            }
        }
    }

    // 3) Excellon 孔规范化（单位换算；不可转换的单位已在解析层诊断）。
    let mut holes: Vec<NormHole> = Vec::new();
    for mat in view.materials.values() {
        if mat.kind != FileKind::Excellon.as_str() {
            continue;
        }
        input_digest.insert(
            mat.filename.clone(),
            format!("v{}:{}", mat.version, digest_str(&mat.raw_text)),
        );
        diagnostics.extend(mat.parsed.diagnostics.iter().cloned());
        if let Some(doc) = &mat.parsed.excellon {
            let factor = doc.unit.to_mm_factor();
            let Some(factor) = factor else {
                diagnostics.push(Diagnostic {
                    kind: DiagnosticKind::SemanticUnresolved,
                    file: mat.filename.clone(),
                    line: 0,
                    code: "UNIT-UNRESOLVED".to_string(),
                    message: "钻孔文件单位无法转换，孔坐标保留原位未规范化".to_string(),
                    raw_snippet: String::new(),
                });
                continue;
            };
            for h in &doc.holes {
                let span = if h.span.is_empty() {
                    (0..copper_index).collect()
                } else {
                    h.span
                        .iter()
                        .copied()
                        .filter(|i| *i < copper_index)
                        .collect()
                };
                if !h.plated && h.raw_span != "through" {
                    // NPTH 盲孔仍记录，但不导电；span 越界给出诊断。
                }
                if h.span.iter().any(|i| *i >= copper_index) {
                    diagnostics.push(Diagnostic {
                        kind: DiagnosticKind::SemanticUnresolved,
                        file: mat.filename.clone(),
                        line: h.source_line,
                        code: "SPAN-OUT-OF-RANGE".to_string(),
                        message: format!(
                            "孔 span {} 超出当前 {} 个铜层范围",
                            h.raw_span, copper_index
                        ),
                        raw_snippet: h.raw_span.clone(),
                    });
                }
                holes.push(NormHole {
                    id: format!("hole:{}#L{}", mat.filename, h.source_line),
                    x: h.x * factor,
                    y: h.y * factor,
                    diameter_mm: h.diameter * factor,
                    plated: h.plated,
                    span,
                    raw_span: h.raw_span.clone(),
                    source_file: mat.filename.clone(),
                    source_line: h.source_line,
                    unit: doc.unit.as_str().to_string(),
                });
            }
        }
    }

    let mut derived = Derived {
        input_digest,
        rule_version: view.params.rule_version.clone(),
        contact_tolerance_mm: view.params.contact_tolerance_mm,
        parent_event_id: parent_event_id.to_string(),
        layers,
        copper,
        masks,
        holes,
        nodes: Vec::new(),
        hard_edges: Vec::new(),
        tolerance_edges: Vec::new(),
        proximity_edges: Vec::new(),
        solutions: Vec::new(),
        components: BTreeMap::new(),
        contradictions: Vec::new(),
        diagnostics,
        valid_from_event: parent_event_id.to_string(),
    };

    build_graph(&mut derived, view);
    enumerate_solutions(&mut derived, view);
    derived
}

fn digest_str(s: &str) -> String {
    // 稳定的非加密摘要（FNV-1a 64bit），用于输入摘要与往返一致性。
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("fnv64:{hash:016x}")
}

fn layer_index_of(derived: &Derived, role: &str) -> Option<usize> {
    derived
        .layers
        .iter()
        .find(|l| l.role == role && role.starts_with("copper"))
        .map(|l| l.index)
}

fn build_graph(derived: &mut Derived, view: &Replay) {
    let tol = view.params.contact_tolerance_mm.max(0.0);

    for r in &derived.copper {
        let layer = layer_index_of(derived, &r.role);
        derived.nodes.push(GraphNode {
            id: r.id.clone(),
            node_type: "copper".to_string(),
            layer,
            label: format!("{} sq{}", r.role, r.last_seq),
            source: r.id.clone(),
        });
    }
    for h in &derived.holes {
        derived.nodes.push(GraphNode {
            id: h.id.clone(),
            node_type: "hole".to_string(),
            layer: None,
            label: format!("{}@({:.3},{:.3})", if h.plated { "PTH" } else { "NPTH" }, h.x, h.y),
            source: format!("{}#L{}", h.source_file, h.source_line),
        });
    }

    let mut edges: Vec<GraphEdge> = Vec::new();

    // 同层铜-铜：接触=硬；容差内=候选；更远=邻近。
    for i in 0..derived.copper.len() {
        for j in (i + 1)..derived.copper.len() {
            let a = &derived.copper[i];
            let b = &derived.copper[j];
            if a.role != b.role {
                continue;
            }
            let gap = rect_gap(a, b);
            if gap > tol * 5.0 + 1e-9 {
                continue; // 太远，连“邻近提示”都不保留，控制图规模
            }
            edges.push(GraphEdge {
                id: format!("edge:{}--{}", a.id, b.id),
                a: a.id.clone(),
                b: b.id.clone(),
                gap_mm: round6(gap),
                kind: classify(gap, tol).as_str().to_string(),
                layers: a
                    .role
                    .strip_prefix("copper_")
                    .and_then(role_layer_index)
                    .into_iter()
                    .collect(),
            });
        }
    }

    // 孔-铜：仅沉铜孔，且铜层在 span 内。
    for h in &derived.holes {
        if !h.plated {
            continue;
        }
        for r in &derived.copper {
            let Some(layer) = layer_index_of(derived, &r.role) else {
                continue;
            };
            if !h.span.contains(&layer) {
                continue;
            }
            let gap = point_rect_gap(h.x, h.y, r);
            if gap > tol * 5.0 + 1e-9 {
                continue;
            }
            edges.push(GraphEdge {
                id: format!("edge:{}--{}", h.id, r.id),
                a: h.id.clone(),
                b: r.id.clone(),
                gap_mm: round6(gap),
                kind: classify(gap, tol).as_str().to_string(),
                layers: vec![layer],
            });
        }
    }

    // 孔-孔：两个沉铜孔在某个共同铜层各自接触时已通过铜间接连通；
    // 这里直接补充同位置重叠孔的关系（例如复核重叠钻孔）。
    for i in 0..derived.holes.len() {
        for j in (i + 1)..derived.holes.len() {
            let a = &derived.holes[i];
            let b = &derived.holes[j];
            if !a.plated || !b.plated {
                continue;
            }
            let common: Vec<usize> = a.span.iter().filter(|l| b.span.contains(l)).copied().collect();
            if common.is_empty() {
                continue;
            }
            let d = ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt();
            let gap = (d - (a.diameter_mm + b.diameter_mm) / 2.0).max(0.0);
            if gap > tol + 1e-9 {
                continue;
            }
            edges.push(GraphEdge {
                id: format!("edge:{}--{}", a.id, b.id),
                a: a.id.clone(),
                b: b.id.clone(),
                gap_mm: round6(gap),
                kind: classify(gap, tol).as_str().to_string(),
                layers: common,
            });
        }
    }

    edges.sort_by(|a, b| {
        a.gap_mm
            .partial_cmp(&b.gap_mm)
            .unwrap()
            .then_with(|| a.id.cmp(&b.id))
    });

    for e in edges {
        match e.kind.as_str() {
            "hard" => derived.hard_edges.push(e),
            "tolerance" => derived.tolerance_edges.push(e),
            _ => derived.proximity_edges.push(e),
        }
    }
    derived.nodes.sort_by(|a, b| a.id.cmp(&b.id));
}

fn role_layer_index(role_suffix: &str) -> Option<usize> {
    match role_suffix {
        "top" => Some(0),
        "inner" => Some(1),
        "bottom" => Some(2),
        _ => None,
    }
}

fn classify(gap: f64, tol: f64) -> EdgeKind {
    if gap <= 1e-9 {
        EdgeKind::Hard
    } else if gap <= tol + 1e-9 {
        EdgeKind::Tolerance
    } else {
        EdgeKind::Proximity
    }
}

fn round6(v: f64) -> f64 {
    (v * 1e6).round() / 1e6
}

fn enumerate_solutions(derived: &mut Derived, view: &Replay) {
    let candidate_ids: Vec<String> =
        derived.tolerance_edges.iter().map(|e| e.id.clone()).collect();

    // 候选边最多枚举 12 条（4096 解）；更多时视为预算耗尽而非截断伪装成功。
    if candidate_ids.len() > 12 {
        derived.diagnostics.push(Diagnostic {
            kind: DiagnosticKind::BudgetExhausted,
            file: String::new(),
            line: 0,
            code: "SOLUTION-SPACE".to_string(),
            message: format!(
                "容差候选边 {} 条超过枚举预算 12，候选解未枚举",
                candidate_ids.len()
            ),
            raw_snippet: String::new(),
        });
    }

    let n = candidate_ids.len().min(12);
    let subsets: Vec<Vec<String>> = (0..(1u32 << n))
        .map(|mask| {
            (0..n)
                .filter(|i| mask & (1 << i) != 0)
                .map(|i| candidate_ids[i].clone())
                .collect()
        })
        .collect();

    for (idx, accepted) in subsets.into_iter().enumerate() {
        let comps = components_for(derived, &accepted);
        derived.solutions.push(CandidateSolution {
            id: format!("sol:{:04}", idx),
            accepted_tolerance_edges: accepted,
            components: comps,
        });
    }

    // 当前分支结论：硬边 + 已接受的人工决定。矛盾决定同时记录，不覆盖。
    let mut branch_accepted = Vec::new();
    for (edge_id, (accepted, ev_id)) in &view.decisions {
        if *accepted {
            branch_accepted.push(edge_id.clone());
        }
        // 同一候选边在不同分支上 accept 与 reject => 矛盾位置。
        // （同一分支重复决定取最新；跨分支冲突在 events 中检测。）
        let accepts = view
            .events
            .iter()
            .filter(|e| e.kind == "decision" && e.payload["edge_id"] == edge_id.as_str())
            .map(|e| e.payload["accepted"].as_bool().unwrap_or(false))
            .collect::<Vec<_>>();
        if accepts.contains(&true) && accepts.contains(&false) {
            if let Some(edge) = derived
                .tolerance_edges
                .iter()
                .chain(derived.hard_edges.iter())
                .find(|e| &e.id == edge_id)
            {
                let (x, y) = edge_midpoint(derived, edge);
                if !derived
                    .contradictions
                    .iter()
                    .any(|c| c.edge_id == *edge_id)
                {
                    derived.contradictions.push(Contradiction {
                        edge_id: edge_id.clone(),
                        x,
                        y,
                        message: format!(
                            "候选边在分支中同时被接受与否定（最近决定事件 {ev_id}）"
                        ),
                    });
                }
            }
        }
    }
    derived.components = components_for(derived, &branch_accepted);
}

fn edge_midpoint(derived: &Derived, edge: &GraphEdge) -> (f64, f64) {
    let pos = |id: &str| -> Option<(f64, f64)> {
        if let Some(h) = derived.holes.iter().find(|h| h.id == id) {
            return Some((h.x, h.y));
        }
        derived.copper.iter().find(|c| c.id == id).map(|c| {
            ((c.x0 + c.x1) / 2.0, (c.y0 + c.y1) / 2.0)
        })
    };
    let a = pos(&edge.a).unwrap_or((0.0, 0.0));
    let b = pos(&edge.b).unwrap_or(a);
    ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)
}

fn components_for(derived: &Derived, accepted_tol: &[String]) -> BTreeMap<String, Vec<String>> {
    let mut parent: BTreeMap<String, String> = BTreeMap::new();
    for n in &derived.nodes {
        parent.insert(n.id.clone(), n.id.clone());
    }
    let union = |parent: &mut BTreeMap<String, String>, a: &str, b: &str| {
        let ra = {
            let mut r = a.to_string();
            while parent[&r] != r {
                r = parent[&r].clone();
            }
            r
        };
        let rb = {
            let mut r = b.to_string();
            while parent[&r] != r {
                r = parent[&r].clone();
            }
            r
        };
        if ra != rb {
            // 以字典序最小为根，保证等价输入得到稳定顺序。
            let (small, big) = if ra < rb { (ra, rb) } else { (rb, ra) };
            parent.insert(big, small);
        }
    };

    for e in &derived.hard_edges {
        union(&mut parent, &e.a, &e.b);
    }
    for id in accepted_tol {
        if let Some(e) = derived.tolerance_edges.iter().find(|e| &e.id == id) {
            union(&mut parent, &e.a, &e.b);
        }
    }

    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for node in parent.keys().cloned().collect::<Vec<_>>() {
        let mut r = node.clone();
        while parent[&r] != r {
            r = parent[&r].clone();
        }
        groups.entry(r).or_default().push(node);
    }
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (idx, (_, mut members)) in groups.into_iter().enumerate() {
        members.sort();
        out.insert(format!("net:{idx:03}"), members);
    }
    out
}
