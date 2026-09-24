//! Connectivity engine.
//!
//! Rules (captured as structured state, not just log lines):
//! * Gerber dark/clear polarity is applied strictly in operation order.
//! * Geometric contact tolerance is a *versioned parameter*.
//! * Without a netlist the engine only produces connectivity candidates;
//!   proximity is never silently treated as conduction.
//!
//! Three distinct non-success results are reported:
//! [`ComputeStatus::CorruptFormat`], [`ComputeStatus::SemanticNoSolution`] and
//! [`ComputeStatus::BudgetExhausted`].

use crate::domain::*;
use crate::geometry::Shape;
use crate::model::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Cell {
    pub gx: i64,
    pub gy: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum EdgeKind {
    /// Zero-gap geometric contact on the same copper layer.
    Contact,
    /// Positive gap within contact tolerance (tangency parameter decides).
    ToleranceContact { gap_nm: i64 },
    /// Nearby but not conductive absent a decision.
    ProximityCandidate { gap_nm: i64 },
    /// Plated hole bridging two copper layers across its span.
    PlatedHole { hole_id: String, from: u32, to: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub id: String,
    pub a: String,
    pub b: String,
    pub layer_a: String,
    pub layer_b: String,
    pub kind: EdgeKind,
    pub gap_nm: i64,
    /// Evidence shapes / hole whose geometry produced this edge.
    pub shape_a: String,
    pub shape_b: String,
    pub pos_a: Option<SourcePos>,
    pub pos_b: Option<SourcePos>,
    pub material_a: String,
    pub material_b: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Component {
    pub id: String,
    pub layer: String,
    /// Occupied cells as run-length encoded spans (`gy`, then gx..gx+len).
    pub runs: Vec<(i64, i64, u32)>,
    pub shape_ids: Vec<String>,
    pub anchor: (i64, i64),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GalvanicGroup {
    pub id: String,
    pub components: Vec<String>,
    pub holes: Vec<String>,
}

/// One legal way to merge galvanic groups using candidate edges. Without a
/// netlist every legal candidate set is retained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateSolution {
    pub index: u32,
    pub applied_proximity_edges: Vec<String>,
    pub groups: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contradiction {
    pub id: String,
    pub message: String,
    pub pos: Option<SourcePos>,
    pub material_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputSummary {
    pub materials: Vec<MaterialDigest>,
    pub material_hash: String,
    pub mapping_hash: String,
    pub params_hash: String,
    pub parser_version: String,
    pub engine_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterialDigest {
    pub id: String,
    pub filename: String,
    pub kind: MaterialKind,
    pub bytes: u64,
    pub content_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub status: ComputeStatus,
    pub params: ConnectParams,
    pub input: InputSummary,
    pub coordinate_basis: String,
    pub origin: (i64, i64),
    pub grid_pitch_nm: i64,
    pub layers: Vec<ModelLayer>,
    pub components: Vec<Component>,
    pub edges: Vec<Edge>,
    pub holes: Vec<ModelHole>,
    pub mask_openings: Vec<MaskOpening>,
    pub galvanic_groups: Vec<GalvanicGroup>,
    pub candidate_solutions: Vec<CandidateSolution>,
    pub contradictions: Vec<Contradiction>,
    pub diags: Vec<Diag>,
    pub bounds_nm: (i64, i64, i64, i64),
    /// Stable hash of the entire computed result.
    pub report_hash: String,
}

fn corrupt_codes() -> [&'static str; 8] {
    [
        "GBR_BAD_FORMAT",
        "GBR_BAD_APERTURE",
        "GBR_BAD_DCODE",
        "GBR_RESERVED_DCODE",
        "STK_JSON",
        "DRL_BAD_TOOL",
        "DRL_MISSING_COORD",
        "GBR_FLASH_WITHOUT_XY",
    ]
}

pub fn compute(model: &BoardModel, params: &ConnectParams, materials: &[ParsedMaterial], mapping: &LayerMapping) -> Report {
    let mut diags = model.diags.clone();

    let has_corrupt = diags.iter().any(|d| d.severity == Severity::Error && corrupt_codes().contains(&d.code.as_str()));
    let has_semantic = diags.iter().any(|d| d.severity == Severity::Error && !corrupt_codes().contains(&d.code.as_str()));

    let input = summarize_input(materials, mapping, params);
    let mut report = Report {
        status: ComputeStatus::Ok,
        params: params.clone(),
        input,
        coordinate_basis: COORDINATE_BASIS.into(),
        origin: (model.bounds_nm.0, model.bounds_nm.1),
        grid_pitch_nm: params.grid_pitch_nm,
        layers: model.layers.clone(),
        components: Vec::new(),
        edges: Vec::new(),
        holes: model.holes.clone(),
        mask_openings: model.mask_openings.clone(),
        galvanic_groups: Vec::new(),
        candidate_solutions: Vec::new(),
        contradictions: Vec::new(),
        diags: Vec::new(),
        bounds_nm: model.bounds_nm,
        report_hash: String::new(),
    };

    if has_corrupt {
        report.status = ComputeStatus::CorruptFormat;
        report.diags = diags;
        finalize_hash(&mut report);
        return report;
    }

    let pitch = params.grid_pitch_nm.max(1);
    let (minx, miny, maxx, maxy) = model.bounds_nm;
    let wcells = (((maxx - minx) as f64) / pitch as f64).ceil() as u64 + 1;
    let hcells = (((maxy - miny) as f64) / pitch as f64).ceil() as u64 + 1;
    let total_cells = wcells.saturating_mul(hcells);
    if total_cells > params.budget.max_grid_cells {
        report.status = ComputeStatus::BudgetExhausted;
        report.contradictions.push(Contradiction {
            id: "budget-grid".into(),
            message: format!("grid requires {total_cells} cells, budget is {}", params.budget.max_grid_cells),
            pos: None,
            material_id: None,
        });
        report.diags = diags;
        finalize_hash(&mut report);
        return report;
    }
    let total_ops: u64 = model.shapes.len() as u64;
    if total_ops > params.budget.max_paint_ops {
        report.status = ComputeStatus::BudgetExhausted;
        report.contradictions.push(Contradiction {
            id: "budget-ops".into(),
            message: format!("{total_ops} paint ops exceed budget {}", params.budget.max_paint_ops),
            pos: None,
            material_id: None,
        });
        report.diags = diags;
        finalize_hash(&mut report);
        return report;
    }

    let copper_layers: Vec<&ModelLayer> = model.layers.iter().filter(|l| l.role.is_copper()).collect();

    // Per-layer raster: owner[cell] = shape index within that layer's paint list.
    let mut layer_shapes: BTreeMap<String, Vec<&ModelShape>> = BTreeMap::new();
    for s in &model.shapes {
        let is_copper = model.layers.iter().any(|l| l.layer == s.layer && l.role.is_copper());
        if is_copper {
            layer_shapes.entry(s.layer.clone()).or_default().push(s);
        }
    }
    for v in layer_shapes.values_mut() {
        v.sort_by_key(|s| (s.op_seq, s.id.clone()));
    }

    let mut all_components: Vec<Component> = Vec::new();
    // shape id -> component id
    let mut shape_comp: BTreeMap<String, String> = BTreeMap::new();

    for layer in &copper_layers {
        let shapes = layer_shapes.get(&layer.layer).cloned().unwrap_or_default();
        let mut owner: BTreeMap<Cell, usize> = BTreeMap::new();
        for (idx, ms) in shapes.iter().enumerate() {
            paint_shape(&ms.shape, pitch, minx, miny, &mut |cell| {
                if ms.dark {
                    owner.insert(cell, idx);
                } else {
                    owner.remove(&cell);
                }
            });
        }
        let comps = flood_components(layer, &owner, &shapes, pitch, minx, miny);
        for c in &comps {
            for sid in &c.shape_ids {
                shape_comp.insert(sid.clone(), c.id.clone());
            }
        }
        all_components.extend(comps);
    }

    report.components = all_components;

    report.edges = build_edges(model, params, &report.components);

    let (definite_edges, candidate_edges) = split_edges(&report.edges);
    report.galvanic_groups = union_groups(&report.components, &definite_edges, &report.holes);

    report.candidate_solutions = enumerate_solutions(&report.galvanic_groups, &candidate_edges, params.budget.max_candidate_groupings);
    if report.candidate_solutions.is_empty() && !candidate_edges.is_empty() {
        report.status = ComputeStatus::BudgetExhausted;
    }

    // Cross-layer plated holes that find no copper become contradictions.
    for h in &model.holes {
        if h.plated {
            let touches = report.edges.iter().any(|e| matches!(e.kind, EdgeKind::PlatedHole { hole_id, .. } if hole_id == &h.id));
            if !touches {
                report.contradictions.push(Contradiction {
                    id: format!("hole-float:{}", h.id),
                    message: format!("plated hole {} spans L{}-L{} but touches no copper", h.id, h.span_from, h.span_to),
                    pos: h.pos,
                    material_id: Some(h.material_id.clone()),
                });
            }
        }
    }

    if report.status == ComputeStatus::Ok && has_semantic {
        report.status = ComputeStatus::SemanticNoSolution;
        for d in diags.iter().filter(|d| d.severity == Severity::Error) {
            report.contradictions.push(Contradiction {
                id: format!("diag:{}:{}", d.code, d.pos.map(|p| p.offset).unwrap_or(0)),
                message: d.message.clone(),
                pos: d.pos,
                material_id: None,
            });
        }
    }
    report.diags = diags;
    finalize_hash(&mut report);
    report
}

fn paint_shape<F: FnMut(Cell)>(shape: &Shape, pitch: i64, minx: i64, miny: i64, emit: &mut F) {
    let (x0, y0, x1, y1) = shape.bbox();
    let gx0 = (x0 - minx).div_euclid(pitch);
    let gx1 = (x1 - minx).div_euclid(pitch);
    let gy0 = (y0 - miny).div_euclid(pitch);
    let gy1 = (y1 - miny).div_euclid(pitch);
    for gx in gx0..=gx1 {
        for gy in gy0..=gy1 {
            let cx = minx + gx * pitch + pitch / 2;
            let cy = miny + gy * pitch + pitch / 2;
            if shape.contains_cell_center(cx, cy) {
                emit(Cell { gx, gy });
            }
        }
    }
}

fn flood_components(
    layer: &ModelLayer,
    owner: &BTreeMap<Cell, usize>,
    shapes: &[&ModelShape],
    _pitch: i64,
    _minx: i64,
    _miny: i64,
) -> Vec<Component> {
    let mut comp_of: BTreeMap<Cell, usize> = BTreeMap::new();
    let mut raw: Vec<(Vec<Cell>, BTreeSet<usize>)> = Vec::new();
    let cells_sorted: Vec<Cell> = owner.keys().copied().collect();
    for start in cells_sorted {
        if comp_of.contains_key(&start) {
            continue;
        }
        let cid = raw.len();
        let mut stack = vec![start];
        let mut cells = Vec::new();
        let mut shape_set: BTreeSet<usize> = BTreeSet::new();
        comp_of.insert(start, cid);
        while let Some(c) = stack.pop() {
            if let Some(&idx) = owner.get(&c) {
                shape_set.insert(idx);
            }
            cells.push(c);
            for n in [
                Cell { gx: c.gx + 1, gy: c.gy },
                Cell { gx: c.gx - 1, gy: c.gy },
                Cell { gx: c.gx, gy: c.gy + 1 },
                Cell { gx: c.gx, gy: c.gy - 1 },
            ] {
                if owner.contains_key(&n) && !comp_of.contains_key(&n) {
                    comp_of.insert(n, cid);
                    stack.push(n);
                }
            }
        }
        raw.push((cells, shape_set));
    }

    raw.into_iter()
        .map(|(mut cells, shape_set)| {
            cells.sort_by_key(|c| (c.gy, c.gx));
            let mut runs: Vec<(i64, i64, u32)> = Vec::new();
            for c in &cells {
                if let Some(last) = runs.last_mut() {
                    if last.0 == c.gy && last.1 + last.2 as i64 == c.gx {
                        last.2 += 1;
                        continue;
                    }
                }
                runs.push((c.gy, c.gx, 1));
            }
            let shape_ids: Vec<String> = shape_set.iter().map(|i| shapes[*i].id.clone()).collect();
            let anchor_cell = cells.first().copied().unwrap_or(Cell { gx: 0, gy: 0 });
            let h = crate::hash::canonical_hash(&(layer.layer.clone(), shape_ids.clone()));
            Component {
                id: format!("comp-{}", &h[..10]),
                layer: layer.layer.clone(),
                runs,
                shape_ids,
                anchor: (anchor_cell.gx, anchor_cell.gy),
            }
        })
        .collect()
}


fn split_edges(edges: &[Edge]) -> (Vec<Edge>, Vec<Edge>) {
    let mut definite = Vec::new();
    let mut candidate = Vec::new();
    for e in edges {
        match e.kind {
            EdgeKind::ProximityCandidate { .. } => candidate.push(e.clone()),
            _ => definite.push(e.clone()),
        }
    }
    (definite, candidate)
}

fn build_edges(
    model: &BoardModel,
    params: &ConnectParams,
    components: &[Component],
) -> Vec<Edge> {
    let mut edges = Vec::new();
    let shape_by: BTreeMap<&str, &ModelShape> = model.shapes.iter().map(|s| (s.id.as_str(), s)).collect();

    let comp_shapes = |c: &Component| -> Vec<&ModelShape> {
        c.shape_ids.iter().filter_map(|id| shape_by.get(id.as_str()).copied()).collect()
    };

    // In-layer geometric edges.
    for layer in model.layers.iter().filter(|l| l.role.is_copper()) {
        let comps: Vec<&Component> = components.iter().filter(|c| c.layer == layer.layer).collect();
        for i in 0..comps.len() {
            for j in (i + 1)..comps.len() {
                let (a, b) = (comps[i], comps[j]);
                let mut best: Option<(i64, &ModelShape, &ModelShape)> = None;
                for sa in comp_shapes(a) {
                    for sb in comp_shapes(b) {
                        let gap = sa.shape.gap_to(&sb.shape);
                        if best.map(|(g, _, _)| gap < g).unwrap_or(true) {
                            best = Some((gap, sa, sb));
                        }
                    }
                }
                if let Some((gap, sa, sb)) = best {
                    if gap <= params.contact_tolerance_nm {
                        let kind = if gap == 0 {
                            EdgeKind::Contact
                        } else {
                            EdgeKind::ToleranceContact { gap_nm: gap }
                        };
                        edges.push(make_edge(a, b, sa, sb, kind, gap));
                    } else if gap <= params.proximity_nm {
                        edges.push(make_edge(a, b, sa, sb, EdgeKind::ProximityCandidate { gap_nm: gap }, gap));
                    }
                }
            }
        }
    }

    // Cross-layer edges through plated holes.
    let z_of: BTreeMap<&str, u32> = model.layers.iter().map(|l| (l.layer.as_str(), l.z_order)).collect();
    for h in &model.holes {
        if !h.plated {
            continue;
        }
        let hs = Shape::Circle { cx: h.x, cy: h.y, r: h.radius };
        let mut touched: Vec<(String, &ModelShape)> = Vec::new();
        for layer in model.layers.iter().filter(|l| l.role.is_copper()) {
            let z = layer.z_order;
            if z < h.span_from || z > h.span_to {
                continue;
            }
            for c in components.iter().filter(|c| c.layer == layer.layer) {
                let mut hit_shape: Option<&ModelShape> = None;
                for sa in comp_shapes(c) {
                    if sa.shape.gap_to(&hs) <= params.contact_tolerance_nm {
                        hit_shape = Some(sa);
                        break;
                    }
                }
                if let Some(sa) = hit_shape {
                    if !touched.iter().any(|(cid, _)| cid == &c.id) {
                        touched.push((c.id.clone(), sa));
                    }
                }
            }
        }
        for i in 0..touched.len() {
            for j in (i + 1)..touched.len() {
                let (ca, sa) = &touched[i];
                let (cb, sb) = &touched[j];
                let la = components.iter().find(|c| &c.id == ca).map(|c| c.layer.clone()).unwrap_or_default();
                let lb = components.iter().find(|c| &c.id == cb).map(|c| c.layer.clone()).unwrap_or_default();
                let id = format!("edge-{}", &crate::hash::canonical_hash(&(&ca, &cb, &h.id))[..10]);
                edges.push(Edge {
                    id,
                    a: ca.clone(),
                    b: cb.clone(),
                    layer_a: la,
                    layer_b: lb,
                    kind: EdgeKind::PlatedHole { hole_id: h.id.clone(), from: h.span_from, to: h.span_to },
                    gap_nm: 0,
                    shape_a: sa.id.clone(),
                    shape_b: sb.id.clone(),
                    pos_a: sa.pos,
                    pos_b: sb.pos,
                    material_a: sa.material_id.clone(),
                    material_b: sb.material_id.clone(),
                });
            }
        }
    }

    edges.sort_by(|a, b| a.id.cmp(&b.id));
    edges.dedup_by(|a, b| a.id == b.id);
    edges
}

fn make_edge(a: &Component, b: &Component, sa: &ModelShape, sb: &ModelShape, kind: EdgeKind, gap: i64) -> Edge {
    let id = format!("edge-{}", &crate::hash::canonical_hash(&(&a.id, &b.id, gap))[..10]);
    Edge {
        id,
        a: a.id.clone(),
        b: b.id.clone(),
        layer_a: a.layer.clone(),
        layer_b: b.layer.clone(),
        kind,
        gap_nm: gap,
        shape_a: sa.id.clone(),
        shape_b: sb.id.clone(),
        pos_a: sa.pos,
        pos_b: sb.pos,
        material_a: sa.material_id.clone(),
        material_b: sb.material_id.clone(),
    }
}

fn union_groups(components: &[Component], definite: &[Edge], holes: &[ModelHole]) -> Vec<GalvanicGroup> {
    let ids: Vec<String> = components.iter().map(|c| c.id.clone()).collect();
    let mut parent: BTreeMap<String, String> = ids.iter().map(|id| (id.clone(), id.clone())).collect();
    fn find(parent: &mut BTreeMap<String, String>, x: &str) -> String {
        let p = parent.get(x).cloned().unwrap_or_else(|| x.to_string());
        if &p == x {
            p
        } else {
            let root = find(parent, &p);
            parent.insert(x.to_string(), root.clone());
            root
        }
    }
    for e in definite {
        let ra = find(&mut parent, &e.a);
        let rb = find(&mut parent, &e.b);
        if ra != rb {
            parent.insert(ra.clone(), rb.clone());
        }
    }
    let mut groups: BTreeMap<String, GalvanicGroup> = BTreeMap::new();
    for id in &ids {
        let root = find(&mut parent, id);
        let g = groups.entry(root).or_insert_with(|| GalvanicGroup { id: format!("net-{}", &crate::hash::sha256_hex(root.as_bytes())[..10]), components: Vec::new(), holes: Vec::new() });
        g.components.push(id.clone());
    }
    for e in definite {
        if let EdgeKind::PlatedHole { hole_id, .. } = &e.kind {
            let root = find(&mut parent, &e.a);
            if let Some(g) = groups.get_mut(&root) {
                if !g.holes.contains(hole_id) {
                    g.holes.push(hole_id.clone());
                }
            }
        }
    }
    for g in groups.values_mut() {
        g.components.sort();
        g.holes.sort();
    }
    let mut out: Vec<GalvanicGroup> = groups.into_values().collect();
    let _ = holes;
    out.sort_by(|a, b| a.components.join(",").cmp(&b.components.join(",")));
    // assign stable sequential ids while keeping deterministic component set hash
    for (i, g) in out.iter_mut().enumerate() {
        g.id = format!("net-{:03}-{}", i + 1, &crate::hash::canonical_hash(&g.components)[..6]);
    }
    out
}

fn enumerate_solutions(groups: &[GalvanicGroup], candidates: &[Edge], limit: u64) -> Vec<CandidateSolution> {
    if candidates.is_empty() {
        return Vec::new();
    }
    // Each candidate edge may be accepted or rejected. We retain the distinct
    // resulting partitions, capped by the budget (budget exhaustion is distinct
    // from "no candidates" and from semantic failure).
    let mut seen: BTreeSet<Vec<Vec<String>>> = BTreeSet::new();
    let mut out = Vec::new();
    let n = candidates.len().min(20) as u32;
    let total = 1u64 << n;
    if total > limit.saturating_mul(8) {
        return Vec::new();
    }
    for mask in 0..total {
        let mut parent: BTreeMap<String, String> = BTreeMap::new();
        for g in groups {
            for c in &g.components {
                parent.insert(c.clone(), c.clone());
            }
        }
        let mut applied = Vec::new();
        for (i, e) in candidates.iter().enumerate().take(n as usize) {
            if mask & (1u64 << i) == 0 {
                continue;
            }
            union2(&mut parent, &e.a, &e.b);
            applied.push(e.id.clone());
        }
        let partition = partition_of(groups, &mut parent);
        if seen.insert(partition.clone()) {
            out.push(CandidateSolution {
                index: out.len() as u32,
                applied_proximity_edges: applied,
                groups: partition,
            });
        }
        if out.len() as u64 > limit {
            return Vec::new();
        }
    }
    out.sort_by(|a, b| a.groups.cmp(&b.groups));
    for (i, s) in out.iter_mut().enumerate() {
        s.index = i as u32;
    }
    out
}

fn union2(parent: &mut BTreeMap<String, String>, a: &str, b: &str) {
    fn find(parent: &mut BTreeMap<String, String>, x: &str) -> String {
        let p = parent.get(x).cloned().unwrap_or_else(|| x.to_string());
        if &p == x {
            p
        } else {
            let r = find(parent, &p);
            parent.insert(x.to_string(), r.clone());
            r
        }
    }
    let ra = find(parent, a);
    let rb = find(parent, b);
    if ra != rb {
        let lo = ra.min(rb.clone());
        let hi = rb.max(ra.clone());
        parent.insert(hi, lo);
    }
}

fn partition_of(groups: &[GalvanicGroup], parent: &mut BTreeMap<String, String>) -> Vec<Vec<String>> {
    let mut roots: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for g in groups {
        for c in &g.components {
            let r = union2_find(parent, c);
            roots.entry(r).or_default().push(c.clone());
        }
    }
    let mut part: Vec<Vec<String>> = roots.into_values().map(|mut v| { v.sort(); v }).collect();
    part.sort();
    part
}

fn union2_find(parent: &mut BTreeMap<String, String>, x: &str) -> String {
    let p = parent.get(x).cloned().unwrap_or_else(|| x.to_string());
    if &p == x {
        p
    } else {
        let r = union2_find(parent, &p);
        parent.insert(x.to_string(), r.clone());
        r
    }
}

fn summarize_input(materials: &[ParsedMaterial], mapping: &LayerMapping, params: &ConnectParams) -> InputSummary {
    let digests: Vec<MaterialDigest> = materials
        .iter()
        .map(|m| MaterialDigest {
            id: m.id.clone(),
            filename: m.filename.clone(),
            kind: m.kind,
            bytes: m.raw.len() as u64,
            content_hash: crate::hash::sha256_hex(m.raw.as_bytes()),
        })
        .collect();
    let material_hash = crate::hash::canonical_hash(&digests);
    let mapping_hash = crate::hash::canonical_hash(mapping);
    let params_hash = crate::hash::canonical_hash(params);
    InputSummary {
        materials: digests,
        material_hash,
        mapping_hash,
        params_hash,
        parser_version: PARSER_VERSION.into(),
        engine_version: ENGINE_VERSION.into(),
    }
}

fn finalize_hash(report: &mut Report) {
    let h = crate::hash::canonical_hash(report);
    report.report_hash = format!("report-{}", &h[..16]);
}
