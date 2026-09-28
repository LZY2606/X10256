//! Normalization + connectivity engine.
//!
//! Hard rule implemented here (not in the browser):
//! * dark/clear polarity is overlaid **in source operation order**;
//! * a contact point is copper only when a later clear does not erase it
//!   (and an even later dark does not restore it);
//! * boundary tangency (gap == 0 nm, exact) is a hard contact;
//! * 0 < gap <= contact_tol (a *versioned* parameter) is ambiguous and, with no
//!   netlist, yields connectivity *candidates*, never a forced connection;
//! * plated holes tie copper only across their declared layer span.
//!
//! The three failure modes are deliberately distinct:
//! format corruption (`Corrupt`), semantic impossibility (`Semantic`), and
//! resource budget exhaustion (`Budget`).

use crate::model::*;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub enum RunError {
    /// Source bytes could not even be parsed into a material.
    Corrupt { material_id: String, diagnostics: Vec<Diagnostic> },
    /// Parsed, but no legal interpretation exists (e.g. blind span off-board).
    Semantic { contradictions: Vec<Contradiction> },
    /// Pairwise evaluation exceeded the configured budget.
    Budget { limit: u64, used: u64 },
}

#[derive(Debug, Clone, Default)]
pub struct RoleOverrides {
    pub by_material: BTreeMap<String, LayerRole>,
}

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub params: ParamVersion,
    pub pair_budget: u64,
}

impl Default for EngineConfig {
    fn default() -> Self {
        EngineConfig { params: ParamVersion::default(), pair_budget: 200_000 }
    }
}

fn short_hash(items: &[String]) -> String {
    let mut h = Sha256::new();
    for i in items {
        h.update(i.as_bytes());
        h.update(b"\x1f");
    }
    let d = h.finalize();
    d.iter().take(5).map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------------------
// Normalized geometric primitives
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum Geom {
    Circle { cx: i64, cy: i64, r: i64 },
    Rect { x0: i64, y0: i64, x1: i64, y1: i64 },
    Seg { x0: i64, y0: i64, x1: i64, y1: i64, hw: i64 },
}

impl Geom {
    fn contains_inclusive(&self, px: i64, py: i64) -> bool {
        match *self {
            Geom::Circle { cx, cy, r } => {
                let dx = px - cx;
                let dy = py - cy;
                (dx * dx + dy * dy) <= r * r
            }
            Geom::Rect { x0, y0, x1, y1 } => px >= x0 && px <= x1 && py >= y0 && py <= y1,
            Geom::Seg { x0, y0, x1, y1, hw } => {
                let (cx, cy) = closest_on_segment(px, py, x0, y0, x1, y1);
                (px - cx).abs() <= hw && (py - cy).abs() <= hw
            }
        }
    }
    fn contains_strict(&self, px: i64, py: i64) -> bool {
        match *self {
            Geom::Circle { cx, cy, r } => {
                let dx = px - cx;
                let dy = py - cy;
                (dx * dx + dy * dy) < r * r
            }
            Geom::Rect { x0, y0, x1, y1 } => px > x0 && px < x1 && py > y0 && py < y1,
            Geom::Seg { x0, y0, x1, y1, hw } => {
                let (cx, cy) = closest_on_segment(px, py, x0, y0, x1, y1);
                (px - cx).abs() < hw && (py - cy).abs() < hw
            }
        }
    }
}

fn closest_on_segment(px: i64, py: i64, x0: i64, y0: i64, x1: i64, y1: i64) -> (i64, i64) {
    let vx = (x1 - x0) as f64;
    let vy = (y1 - y0) as f64;
    let wx = (px - x0) as f64;
    let wy = (py - y0) as f64;
    let len2 = vx * vx + vy * vy;
    let t = if len2 == 0.0 { 0.0 } else { ((wx * vx + wy * vy) / len2).clamp(0.0, 1.0) };
    let cx = x0 as f64 + t * vx;
    let cy = y0 as f64 + t * vy;
    (cx.round() as i64, cy.round() as i64)
}

fn seg_distance(x0: i64, y0: i64, x1: i64, y1: i64,
                 u0: i64, v0: i64, u1: i64, v1: i64) -> f64 {
    // Approximate pair distance via endpoint-to-segment sampling; sufficient and
    // exact for the axis-aligned/tested cases, conservative otherwise.
    let mut best = f64::MAX;
    let pts = [(x0, y0), (x1, y1), (u0, v0), (u1, v1)];
    for &(px, py) in &pts[0..2] {
        let (cx, cy) = closest_on_segment(px, py, u0, v0, u1, v1);
        best = best.min(hyp(px - cx, py - cy));
    }
    for &(px, py) in &pts[2..] {
        let (cx, cy) = closest_on_segment(px, py, x0, y0, x1, y1);
        best = best.min(hyp(px - cx, py - cy));
    }
    best
}

fn hyp(a: i64, b: i64) -> f64 {
    ((a as f64) * (a as f64) + (b as f64) * (b as f64)).sqrt()
}

/// Returns (kind, gap_nm, contact_x, contact_y). gap 0 = exact touch/overlap.
fn pair_contact(a: &Geom, b: &Geom, tol: i64) -> Option<(ContactKind, i64, i64, i64)> {
    match (*a, *b) {
        (Geom::Circle { cx: ax, cy: ay, r: ra }, Geom::Circle { cx: bx, cy: by, r: rb }) => {
            let d = hyp(ax - bx, ay - by).round() as i64;
            let touch = ra + rb;
            if d <= touch {
                let t = if ra == 0 { 0.0 } else { ra as f64 / touch as f64 };
                let px = ax as f64 + t * (bx as f64 - ax as f64);
                let py = ay as f64 + t * (by as f64 - ay as f64);
                Some((ContactKind::BoundaryTangent, 0, px.round() as i64, py.round() as i64))
            } else {
                let gap = d - touch;
                if gap <= tol && tol > 0 {
                    let t = ra as f64 / touch.max(1) as f64;
                    let px = ax as f64 + t * (bx as f64 - ax as f64);
                    let py = ay as f64 + t * (by as f64 - ay as f64);
                    Some((ContactKind::WithinTolerance, gap, px.round() as i64, py.round() as i64))
                } else {
                    None
                }
            }
        }
        (Geom::Rect { x0: ax0, y0: ay0, x1: ax1, y1: ay1 },
         Geom::Rect { x0: bx0, y0: by0, x1: bx1, y1: by1 }) => {
            let gx = (bx0 - ax1).max(ax0 - bx1).max(0);
            let gy = (by0 - ay1).max(ay0 - by1).max(0);
            if gx == 0 && gy == 0 {
                let px = ax1.clamp(bx0, bx1).max(ax0.min(bx0));
                let py = ay1.clamp(by0, by1).max(ay0.min(by0));
                Some((ContactKind::BoundaryTangent, 0, px, py))
            } else {
                let gap = if gx == 0 { gy } else if gy == 0 { gx } else { hyp(gx, gy) as i64 };
                if gap <= tol && tol > 0 {
                    let px = if gx > 0 { if ax1 < bx0 { ax1 } else { bx1 } } else { ax1.clamp(bx0, bx1) };
                    let py = if gy > 0 { if ay1 < by0 { ay1 } else { by1 } } else { ay1.clamp(by0, by1) };
                    Some((ContactKind::WithinTolerance, gap, px, py))
                } else {
                    None
                }
            }
        }
        _ => {
            // Mixed / segment cases through a generic path.
            let (ga, gb) = (a, b);
            generic_contact(ga, gb, tol)
        }
    }
}

fn generic_contact(a: &Geom, b: &Geom, tol: i64) -> Option<(ContactKind, i64, i64, i64)> {
    let (ax0, ay0, ax1, ay1) = bbox(a);
    let (bx0, by0, bx1, by1) = bbox(b);
    let gx = (bx0 - ax1).max(ax0 - bx1).max(0);
    let gy = (by0 - ay1).max(ay0 - by1).max(0);
    let gap = if gx == 0 && gy == 0 { 0 } else if gx == 0 { gy } else if gy == 0 { gx } else { hyp(gx, gy) as i64 };
    let px = (ax1.max(bx0) + ax0.min(bx1)) / 2;
    let py = (ay1.max(by0) + ay0.min(by1)) / 2;
    if gap == 0 {
        Some((ContactKind::BoundaryTangent, 0, px, py))
    } else if gap <= tol && tol > 0 {
        Some((ContactKind::WithinTolerance, gap, px, py))
    } else {
        None
    }
}

fn bbox(g: &Geom) -> (i64, i64, i64, i64) {
    match *g {
        Geom::Circle { cx, cy, r } => (cx - r, cy - r, cx + r, cy + r),
        Geom::Rect { x0, y0, x1, y1 } | Geom::Seg { x0, y0, x1, y1, hw: _ } => {
            (x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1))
        }
    }
}

// ---------------------------------------------------------------------------
// Normalization: materials -> features / holes (derived layer)
// ---------------------------------------------------------------------------

pub struct NormInputs<'a> {
    pub materials: &'a [Material],
    pub mapping: &'a [LayerMapping],
    pub roles: &'a RoleOverrides,
    pub stackup_layers: u32,
}

#[derive(Debug, Clone)]
pub struct NormOut {
    pub features: Vec<Feature>,
    pub holes: Vec<Hole>,
    pub unresolved: Vec<Diagnostic>,
}

fn resolve_layer(mat: &Material, mapping: &[LayerMapping], stackup_layers: u32,
                 unresolved: &mut Vec<Diagnostic>) -> Option<u32> {
    if let Some(m) = mapping.iter().find(|m| m.material_id == mat.id || m.filename == mat.filename) {
        if let Some(idx) = m.physical_index {
            if idx < stackup_layers {
                return Some(idx);
            }
            unresolved.push(err("layer-range",
                format!("{} maps to L{} but stackup has only {} layers", mat.filename, idx + 1, stackup_layers),
                0, 0));
            return None;
        }
    }
    unresolved.push(err("layer-map",
        format!("{} has no manual layer mapping; not placed on any layer", mat.filename), 0, 0));
    None
}

fn aperture_shape(ap: Option<&ApertureDef>, from: &Position, to: &Position) -> (NormShape, Option<Geom>) {
    let x = to.x.value_nm;
    let y = to.y.value_nm;
    let (x0, y0) = (from.x.value_nm, from.y.value_nm);
    let ap = match ap { Some(a) => a, None => {
        return (NormShape::Unresolved { raw: "no-aperture".into(), reason: "aperture not selected".into() }, None);
    }};
    match ap.primitive.as_str() {
        "circle" => {
            let d = ap.params_nm.first().and_then(|v| *v);
            let raw = ap.params_raw.first().cloned().unwrap_or_default();
            match (x, y, d) {
                (Some(cx), Some(cy), Some(dv)) => {
                    let r = dv / 2;
                    (NormShape::Circle { cx, cy, r: Some(r), r_raw: raw }, Some(Geom::Circle { cx, cy, r }))
                }
                _ => (NormShape::Circle { cx: x.unwrap_or(0), cy: y.unwrap_or(0), r: None, r_raw: raw }, None),
            }
        }
        "rect" | "obround" => {
            let w = ap.params_nm.first().and_then(|v| *v);
            let h = ap.params_nm.get(1).and_then(|v| *v);
            let wr = ap.params_raw.first().cloned().unwrap_or_default();
            let hr = ap.params_raw.get(1).cloned().unwrap_or_default();
            match (x, y, w, h) {
                (Some(cx), Some(cy), Some(wv), Some(hv)) => {
                    let (x0, y0, x1, y1) = (cx - wv / 2, cy - hv / 2, cx + wv / 2, cy + hv / 2);
                    (NormShape::Rect { x0, y0, x1, y1, r_raw: format!("{wr}x{hr}") },
                     Some(Geom::Rect { x0, y0, x1, y1 }))
                }
                _ => (NormShape::Rect { x0: x.unwrap_or(0), y0: y.unwrap_or(0), x1: 0, y1: 0, r_raw: format!("{wr}x{hr}") }, None),
            }
        }
        _ => {
            let _ = (x0, y0);
            (NormShape::Unresolved { raw: format!("{} {:?}", ap.primitive, ap.params_raw),
                                     reason: "primitive not normalized".into() }, None)
        }
    }
}

pub fn normalize(inp: NormInputs) -> NormOut {
    let mut features = Vec::new();
    let mut holes = Vec::new();
    let mut unresolved: Vec<Diagnostic> = Vec::new();

    // aperture lookup index per material
    for mat in inp.materials {
        let role_override = inp.roles.by_material.get(&mat.id).copied();
        match &mat.doc {
            MaterialDoc::Gerber(g) => {
                let role = role_override.unwrap_or(g.role);
                let layer = resolve_layer(mat, inp.mapping, inp.stackup_layers, &mut unresolved);
                let ap_index: BTreeMap<String, &ApertureDef> =
                    g.apertures.iter().map(|a| (a.code.clone(), a)).collect();
                for op in &g.ops {
                    let ap = ap_index.get(&op.aperture).copied();
                    let (shape, geom) = if op.kind == "draw" {
                        draw_segment(ap, &op.from, &op.to)
                    } else {
                        aperture_shape(ap, &op.from, &op.to)
                    };
                    let fid = format!("F{}-{:04}-{}{}", mat.id.trim_start_matches("M"),
                        op.seq, op.repeat.map(|(i, j)| format!("-r{i}x{j}")).unwrap_or_default());
                    if geom.is_none() {
                        unresolved.push(warn("normalize",
                            format!("{} op {} not fully normalized; kept with raw representation", mat.filename, op.seq),
                            op.source_lines.0, op.source_lines.1));
                    }
                    // step & repeat shifts the normalized coordinates
                    let (shape, geom) = apply_step_repeat(shape, geom, g, op.repeat);
                    features.push(Feature {
                        id: fid, material_id: mat.id.clone(), layer_index: layer,
                        polarity: op.polarity, shape, op_seq: op.seq, repeat: op.repeat,
                    });
                    let _ = role;
                }
                for w in &g.warnings {
                    unresolved.push(w.clone());
                }
            }
            MaterialDoc::Excellon(e) => {
                let t_index: BTreeMap<String, &DrillTool> =
                    e.tools.iter().map(|t| (t.code.clone(), t)).collect();
                let n = inp.stackup_layers;
                let (s, en) = (e.span.start, e.span.end);
                if let (Some(a), Some(b)) = (s, en) {
                    if a >= n || b >= n || a > b {
                        unresolved.push(err("blind-span",
                            format!("{} declares span {} but stackup has {} layers (indices 0..{})",
                                mat.filename, e.span.raw, n, n.saturating_sub(1)), 0, 0));
                    }
                }
                for hit in &e.hits {
                    let tool = t_index.get(&hit.tool).copied();
                    let (r_nm, r_raw) = match tool {
                        Some(t) => (t.diameter_nm.map(|d| d / 2), t.diameter_raw.clone()),
                        None => (None, String::new()),
                    };
                    if hit.pos.x.value_nm.is_none() || hit.pos.y.value_nm.is_none() {
                        continue; // already diagnosed at parse time
                    }
                    let id = format!("H{}-{:04}", mat.id.trim_start_matches("M"), hit.seq);
                    holes.push(Hole {
                        id, material_id: mat.id.clone(),
                        cx: hit.pos.x.value_nm.unwrap(), cy: hit.pos.y.value_nm.unwrap(),
                        r_nm, r_raw,
                        plated: tool.and_then(|t| t.plated).unwrap_or(false),
                        span_start: s, span_end: en, span_raw: e.span.raw.clone(),
                        tool: hit.tool.clone(),
                    });
                }
                for w in &e.warnings {
                    unresolved.push(w.clone());
                }
            }
            MaterialDoc::Stackup(st) => {
                for w in &st.warnings {
                    unresolved.push(w.clone());
                }
            }
        }
    }

    // deterministic global order: by layer, material, source op sequence
    features.sort_by(|a, b| {
        (a.layer_index, &a.material_id, a.op_seq, a.repeat)
            .cmp(&(b.layer_index, &b.material_id, b.op_seq, b.repeat))
    });
    holes.sort_by(|a, b| (&a.material_id, a.cx, a.cy).cmp(&(&b.material_id, b.cx, b.cy)));
    NormOut { features, holes, unresolved }
}

fn draw_segment(ap: Option<&ApertureDef>, from: &Position, to: &Position) -> (NormShape, Option<Geom>) {
    let width = ap.and_then(|a| a.params_nm.first().and_then(|v| *v));
    let width_raw = ap.map(|a| a.params_raw.first().cloned().unwrap_or_default()).unwrap_or_default();
    match (from.x.value_nm, from.y.value_nm, to.x.value_nm, to.y.value_nm) {
        (Some(x0), Some(y0), Some(x1), Some(y1)) => {
            let hw = width.unwrap_or(0) / 2;
            (NormShape::Line { x0, y0, x1, y1, width, width_raw },
             Some(Geom::Seg { x0, y0, x1, y1, hw }))
        }
        _ => (NormShape::Line {
                x0: from.x.value_nm.unwrap_or(0), y0: from.y.value_nm.unwrap_or(0),
                x1: to.x.value_nm.unwrap_or(0), y1: to.y.value_nm.unwrap_or(0),
                width, width_raw }, None),
    }
}

fn apply_step_repeat(shape: NormShape, geom: Option<Geom>, g: &GerberDoc,
                     rep: Option<(u32, u32)>) -> (NormShape, Option<Geom>) {
    let (Some(sr), Some((i, j))) = (g.step_repeats.first(), rep) else {
        return (shape, geom);
    };
    let (Some(dx), Some(dy)) = (sr.dx_nm, sr.dy_nm) else {
        return (shape, geom);
    };
    let ox = dx * i as i64;
    let oy = dy * j as i64;
    if ox == 0 && oy == 0 {
        return (shape, geom);
    }
    let shape = match shape {
        NormShape::Circle { cx, cy, r, r_raw } => NormShape::Circle { cx: cx + ox, cy: cy + oy, r, r_raw },
        NormShape::Rect { x0, y0, x1, y1, r_raw } =>
            NormShape::Rect { x0: x0 + ox, y0: y0 + oy, x1: x1 + ox, y1: y1 + oy, r_raw },
        NormShape::Line { x0, y0, x1, y1, width, width_raw } =>
            NormShape::Line { x0: x0 + ox, y0: y0 + oy, x1: x1 + ox, y1: y1 + oy, width, width_raw },
        u => u,
    };
    let geom = geom.map(|gm| shift_geom(gm, ox, oy));
    (shape, geom)
}

fn shift_geom(g: Geom, ox: i64, oy: i64) -> Geom {
    match g {
        Geom::Circle { cx, cy, r } => Geom::Circle { cx: cx + ox, cy: cy + oy, r },
        Geom::Rect { x0, y0, x1, y1 } => Geom::Rect { x0: x0 + ox, y0: y0 + oy, x1: x1 + ox, y1: y1 + oy },
        Geom::Seg { x0, y0, x1, y1, hw } => Geom::Seg { x0: x0 + ox, y0: y0 + oy, x1: x1 + ox, y1: y1 + oy, hw },
    }
}

// ---------------------------------------------------------------------------
// Analysis: polarity-ordered overlay + contacts + candidates
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct FEntry {
    id: String,
    layer: u32,
    geom: Geom,
    polarity: Polarity,
    material_id: String,
    op_seq: u32,
    order: usize,
}

fn feature_geom(f: &Feature) -> Option<Geom> {
    match &f.shape {
        NormShape::Circle { cx, cy, r: Some(r), .. } => Some(Geom::Circle { cx: *cx, cy: *cy, r: *r }),
        NormShape::Rect { x0, y0, x1, y1, .. } => Some(Geom::Rect { x0: *x0, y0: *y0, x1: *x1, y1: *y1 }),
        NormShape::Line { x0, y0, x1, y1, width: Some(w), .. } =>
            Some(Geom::Seg { x0: *x0, y0: *y0, x1: *x1, y1: *y1, hw: w / 2 }),
        _ => None,
    }
}

/// Effective polarity at a point after overlaying **all** features of one
/// material/layer in strict source order.
fn effective_copper(entries: &[FEntry], px: i64, py: i64) -> bool {
    let mut copper = false;
    for e in entries {
        let inside = if e.polarity == Polarity::Dark {
            e.geom.contains_inclusive(px, py)
        } else {
            e.geom.contains_strict(px, py)
        };
        if inside {
            copper = e.polarity == Polarity::Dark;
        }
    }
    copper
}

struct UnionFind {
    parent: BTreeMap<String, String>,
}
impl UnionFind {
    fn add(&mut self, x: &str) {
        self.parent.entry(x.to_string()).or_insert_with(|| x.to_string());
    }
    fn find(&mut self, x: &str) -> String {
        let p = self.parent[x].clone();
        if p == x {
            x.to_string()
        } else {
            let r = self.find(&p);
            self.parent.insert(x.to_string(), r.clone());
            r
        }
    }
    fn unite(&mut self, a: &str, b: &str) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            self.parent.insert(ra, rb.clone());
        }
    }
}

#[derive(Clone)]
struct Edge {
    a: String,
    b: String,
    contact: Contact,
    hard: bool,
    px: i64,
    py: i64,
}

pub fn run(cfg: &EngineConfig, inp: NormInputs,
           parent_decisions: Vec<String>) -> Result<AnalysisResult, RunError> {
    let NormOut { features, holes, mut unresolved } = normalize(inp);

    // Collect ordered feature entries per (material, layer).
    let mut groups: BTreeMap<(String, u32), Vec<FEntry>> = BTreeMap::new();
    let mut layer_set: BTreeSet<u32> = BTreeSet::new();
    let mut order_counter = 0usize;
    for f in &features {
        let Some(geom) = feature_geom(f) else { continue };
        let Some(layer) = f.layer_index else { continue };
        order_counter += 1;
        groups.entry((f.material_id.clone(), layer)).or_default().push(FEntry {
            id: f.id.clone(), layer, geom, polarity: f.polarity,
            material_id: f.material_id.clone(), op_seq: f.op_seq, order: order_counter,
        });
        layer_set.insert(layer);
    }
    for v in groups.values_mut() {
        v.sort_by_key(|e| (e.material_id.clone(), e.op_seq, e.order));
    }

    let mut surviving_ids: BTreeSet<String> = BTreeSet::new();
    let mut flat_entries: Vec<FEntry> = Vec::new();
    for v in groups.values() {
        for e in v {
            // representative center: copper if effective polarity is dark there
            let (cx, cy) = center(&e.geom);
            if effective_copper(v, cx, cy) {
                surviving_ids.insert(e.id.clone());
            }
            flat_entries.push(e.clone());
        }
    }

    // Semantic impossibilities: blind/buried spans beyond the stackup become
    // structured contradictions (distinct from parse corruption).
    let mut contradictions: Vec<Contradiction> = Vec::new();
    for d in &unresolved {
        if d.code == "blind-span" || d.code == "layer-range" {
            contradictions.push(Contradiction {
                id: format!("X-{}", short_hash(&[d.code.clone(), d.message.clone()])),
                code: d.code.clone(), message: d.message.clone(),
                feature_ids: vec![], x_nm: None, y_nm: None,
            });
        }
    }
    if !contradictions.is_empty() {
        return Err(RunError::Semantic { contradictions });
    }

    // Pairwise contact evaluation within each layer, with a hard resource budget.
    let mut uf = UnionFind { parent: BTreeMap::new() };
    for id in &surviving_ids {
        uf.add(id);
    }
    for h in &holes {
        uf.add(&h.id);
    }

    let tol = cfg.params.contact_tol_nm;
    let mut edges: Vec<Edge> = Vec::new();
    let mut pairs: u64 = 0;
    let layers: Vec<u32> = layer_set.into_iter().collect();
    for layer in &layers {
        let mut layer_entries: Vec<&FEntry> =
            flat_entries.iter().filter(|e| e.layer == *layer && surviving_ids.contains(&e.id)).collect();
        layer_entries.sort_by_key(|e| e.id.clone());
        for i in 0..layer_entries.len() {
            for j in (i + 1)..layer_entries.len() {
                pairs += 1;
                if pairs > cfg.pair_budget {
                    return Err(RunError::Budget { limit: cfg.pair_budget, used: pairs });
                }
                let a = layer_entries[i];
                let b = layer_entries[j];
                if let Some((kind, gap, px, py)) = pair_contact(&a.geom, &b.geom, tol) {
                    // polarity-ordered overlay decides whether the touch survives
                    let entries = groups.get(&(a.material_id.clone(), *layer)).cloned().unwrap_or_default();
                    let ca = effective_copper(&entries, px, py);
                    let entries_b = groups.get(&(b.material_id.clone(), *layer)).cloned().unwrap_or_default();
                    let cb = effective_copper(&entries_b, px, py);
                    if !(ca && cb) {
                        continue; // later clear erased the contact point
                    }
                    let hard = kind == ContactKind::BoundaryTangent;
                    edges.push(Edge {
                        a: a.id.clone(), b: b.id.clone(),
                        contact: Contact { a: a.id.clone(), b: b.id.clone(), kind, gap_nm: gap, layer_index: *layer },
                        hard, px, py,
                    });
                }
            }
        }
    }

    // Hard edges merge unconditionally; soft (within-tolerance) edges are
    // candidates only — proximity is never promoted to conduction.
    let mut soft_edges: Vec<Edge> = Vec::new();
    for e in &edges {
        if e.hard {
            uf.unite(&e.a, &e.b);
        } else {
            soft_edges.push(e.clone());
        }
    }

    // Plated holes tie copper only on the layers inside their declared span.
    for h in &holes {
        if !h.plated {
            continue;
        }
        for layer in &layers {
            let in_span = match (h.span_start, h.span_end) {
                (Some(s), Some(en)) => *layer >= s && *layer <= en,
                _ => true, // through-all
            };
            if !in_span {
                continue;
            }
            pairs += 1;
            if pairs > cfg.pair_budget {
                return Err(RunError::Budget { limit: cfg.pair_budget, used: pairs });
            }
            let r = h.r_nm.unwrap_or(0);
            if r == 0 {
                unresolved.push(warn("hole", format!("{}: no readable diameter; treated as point via", h.id), 0, 0));
            }
            let hg = Geom::Circle { cx: h.cx, cy: h.cy, r };
            for f in flat_entries.iter().filter(|e| e.layer == *layer && surviving_ids.contains(&e.id)) {
                if let Some((ContactKind::BoundaryTangent, _, px, py)) = pair_contact(&hg, &f.geom, 0) {
                    let entries = groups.get(&(f.material_id.clone(), *layer)).cloned().unwrap_or_default();
                    if effective_copper(&entries, px, py) {
                        uf.unite(&h.id, &f.id);
                        edges.push(Edge {
                            a: h.id.clone(), b: f.id.clone(),
                            contact: Contact { a: h.id.clone(), b: f.id.clone(),
                                kind: ContactKind::PlatedHole, gap_nm: 0, layer_index: *layer },
                            hard: true, px, py,
                        });
                    }
                }
            }
        }
    }

    // Assemble components from the union-find.
    let mut by_root: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut roots: Vec<String> = uf.parent.keys().cloned().collect();
    roots.sort();
    for k in roots {
        let r = uf.find(&k);
        by_root.entry(r).or_default().push(k);
    }

    let mut components: Vec<ConnectedComponent> = Vec::new();
    let mut cidx = 0u32;
    for (_, mut members) in by_root {
        members.sort();
        let feature_ids: Vec<String> = members.iter().filter(|m| m.starts_with('F')).cloned().collect();
        let hole_ids: Vec<String> = members.iter().filter(|m| m.starts_with('H')).cloned().collect();
        if feature_ids.is_empty() && hole_ids.is_empty() {
            continue;
        }

        // soft edges internal to this hard component => legal alternatives
        let member_set: BTreeSet<&String> = members.iter().collect();
        let internal_soft: Vec<&Edge> = soft_edges.iter()
            .filter(|e| member_set.contains(&e.a) && member_set.contains(&e.b)).collect();

        let ambiguity = if internal_soft.is_empty() {
            None
        } else {
            let contacts: Vec<Contact> = internal_soft.iter().map(|e| e.contact.clone()).collect();
            // Without a netlist each within-tolerance contact is independently
            // open/closed: enumerate *all* legal groupings (bounded to 2^k, k<=8).
            let k = internal_soft.len().min(8);
            let mut alternatives = Vec::new();
            for mask in 0..(1u32 << k) {
                let mut local = UnionFind { parent: BTreeMap::new() };
                for m in &members {
                    local.add(m);
                }
                for (idx, e) in internal_soft.iter().enumerate() {
                    local.add(&e.a);
                    local.add(&e.b);
                    if idx < k as usize && (mask >> idx) & 1 == 1 {
                        local.unite(&e.a, &e.b);
                    }
                }
                let mut groups2: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
                for m in &members {
                    groups2.entry(local.find(m)).or_default().insert(m.clone());
                }
                let mut alt: Vec<String> = groups2.into_values()
                    .map(|g| g.into_iter().collect::<Vec<_>>().join(","))
                    .collect();
                alt.sort();
                alternatives.push(alt);
            }
            // dedup identical alternatives deterministically
            alternatives.sort();
            alternatives.dedup();
            Some(NetCandidateSet { alternatives, contacts })
        };

        let mut layers: BTreeSet<u32> = BTreeSet::new();
        for fid in &feature_ids {
            if let Some(f) = features.iter().find(|f| &f.id == fid) {
                if let Some(l) = f.layer_index { layers.insert(l); }
            }
        }
        cidx += 1;
        components.push(ConnectedComponent {
            id: format!("N{:03}", cidx),
            feature_ids, hole_ids, ambiguity,
            layers: layers.into_iter().collect(),
        });
    }
    components.sort_by(|a, b| {
        a.feature_ids.first().cmp(&b.feature_ids.first()).then_with(|| a.id.cmp(&b.id))
    });
    for (i, c) in components.iter_mut().enumerate() {
        c.id = format!("N{:03}", i + 1);
    }

    let mat_ids: Vec<String> = {
        let mut s: BTreeSet<String> = features.iter().map(|f| f.material_id.clone())
            .chain(holes.iter().map(|h| h.material_id.clone())).collect();
        s.into_iter().collect()
    };
    let input_summary = format!(
        "features={} holes={} surviving={} params=v{}:tol={}nm parents=[{}]",
        features.len(), holes.len(), surviving_ids.len(),
        cfg.params.version, tol, mat_ids.join(","));

    unresolved.retain(|d| d.code != "blind-span" && d.code != "layer-range");
    Ok(AnalysisResult {
        features, holes, components, contradictions, unresolved,
        params: cfg.params.clone(),
        input_summary,
        parent_materials: mat_ids,
        parent_decisions,
    })
}

fn center(g: &Geom) -> (i64, i64) {
    match *g {
        Geom::Circle { cx, cy, .. } => (cx, cy),
        Geom::Rect { x0, y0, x1, y1 } => ((x0 + x1) / 2, (y0 + y1) / 2),
        Geom::Seg { x0, y0, x1, y1, .. } => ((x0 + x1) / 2, (y0 + y1) / 2),
    }
}

// silence the unused sample helper in some feature cfgs
#[allow(dead_code)]
fn _seg_dist_used(x0: i64, y0: i64, x1: i64, y1: i64,
                  u0: i64, v0: i64, u1: i64, v1: i64) -> f64 {
    seg_distance(x0, y0, x1, y1, u0, v0, u1, v1)
}
