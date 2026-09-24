//! Recompute derived copper, holes, solder-mask openings and connectivity
//! candidates from immutable materials at one rule version.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use sha2::{Digest, Sha256};

use crate::engine::raster::Raster;
use crate::engine::rules::RuleParams;
use crate::engine::state::*;
use crate::error::{Diagnostic, Failure, FailureKind};
use crate::model::*;

/// Everything the engine needs about the project at one point in history.
pub struct ComputeInput<'a> {
    pub materials: &'a [Material],
    /// layer_key -> effective role after manual mapping decisions
    pub effective_roles: &'a BTreeMap<String, LayerRole>,
    pub params: &'a RuleParams,
    pub parent_event_id: String,
}

fn digest_of_parts(parts: &[String]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update(b"\x00");
    }
    hex_encode(&h.finalize())
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn stable_id(prefix: &str, key: &str) -> String {
    let mut h = Sha256::new();
    h.update(key.as_bytes());
    let d = h.finalize();
    format!("{}-{}", prefix, &hex_encode(&d)[..12])
}

pub fn compute_state(input: ComputeInput<'_>) -> Result<RecomputedState, Failure> {
    if let Some(err) = input.params.validate() {
        return Err(Failure::single(
            FailureKind::SemanticUnresolved,
            "RULE_PARAMS_INVALID",
            "rule-params",
            err,
        ));
    }

    // Collect diagnostics from parsed documents (warnings stay attached).
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    for m in input.materials {
        if let Some(g) = &m.gerber {
            diagnostics.extend(g.diagnostics.iter().map(|d| prefix_diag(d, &m.id)));
        }
        if let Some(e) = &m.excellon {
            diagnostics.extend(e.diagnostics.iter().map(|d| prefix_diag(d, &m.id)));
        }
        if let Some(s) = &m.stackup {
            diagnostics.extend(s.diagnostics.iter().map(|d| prefix_diag(d, &m.id)));
        }
    }
    if diagnostics.iter().any(|d| d.kind == FailureKind::SemanticUnresolved) {
        // Semantic problems on input mean no single derived answer; we surface
        // the diagnostics rather than faking success with defaults.
        return Err(Failure { kind: FailureKind::SemanticUnresolved, diagnostics });
    }

    // Determine layer assignment for Gerber materials.
    let mut stack_copper_order: BTreeMap<String, u32> = BTreeMap::new();
    for m in input.materials {
        if let Some(st) = &m.stackup {
            for l in &st.layers {
                if let Some(mat) = &l.material_id {
                    if l.role == LayerRole::Copper {
                        if let Some(o) = l.copper_order {
                            stack_copper_order.insert(mat.clone(), o);
                        }
                    }
                }
            }
        }
    }

    let mut layers: Vec<RenderLayer> = Vec::new();
    let mut layer_meta: Vec<LayerMeta> = Vec::new();
    for m in input.materials {
        let Some(g) = &m.gerber else { continue };
        let role = input
            .effective_roles
            .get(&m.id)
            .copied()
            .or(g.inferred_role)
            .unwrap_or(LayerRole::Document);
        if !matches!(
            role,
            LayerRole::Copper | LayerRole::Soldermask | LayerRole::BoardOutline
        ) {
            // Non-conductive art layers are retained as render layers but do
            // not participate in connectivity.
        }
        let copper_order = stack_copper_order
            .get(&m.id)
            .copied()
            .or(g.inferred_copper_order);
        let layer_key = m.id.clone();

        let (ops, mut all_geoms, _occ_origin) = expand_ops(m, g);
        let bbox = bbox_of(&all_geoms);
        all_geoms.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        let width_cells = (((bbox.2 - bbox.0) / input.params.cell_nm) + 2) as u32;
        let height_cells = (((bbox.3 - bbox.1) / input.params.cell_nm) + 2) as u32;

        layers.push(RenderLayer {
            layer_key: layer_key.clone(),
            role: role.code().to_string(),
            copper_order,
            material_id: m.id.clone(),
            unit_raw: g.unit_raw.clone(),
            basis: "project-nm+top-left-y-up".to_string(),
            ops,
            width_cells,
            height_cells,
            bbox,
        });
        layer_meta.push(LayerMeta {
            material_id: m.id.clone(),
            layer_key,
            role,
            copper_order,
            bbox,
        });
    }

    // Holes.
    let mut holes: Vec<RenderHit> = Vec::new();
    for m in input.materials {
        let Some(e) = &m.excellon else { continue };
        for hit in &e.hits {
            let tool = e.tools.iter().find(|t| t.id == hit.tool);
            holes.push(RenderHit {
                id: format!("{}#{}#L{}", m.id, hit.tool, hit.line),
                material_id: m.id.clone(),
                tool: hit.tool.clone(),
                plated: tool.map(|t| t.plated).unwrap_or(true),
                diameter_nm: tool.map(|t| t.diameter_nm).unwrap_or(0),
                layer_range: tool.and_then(|t| t.layer_range),
                x: hit.at.x,
                y: hit.at.y,
                source_line: hit.line,
            });
        }
    }

    let mut ctx = Compute {
        params: input.params,
        diagnostics: &mut diagnostics,
        layers: &layers,
        meta: &layer_meta,
        holes: &holes,
        rasters: Vec::new(),
    };
    ctx.build_rasters()?;
    let (components, base_edges, candidate_edges, near_misses, ambiguities, candidates, conflicts) =
        ctx.connectivity()?;

    let digest_parts: Vec<String> = input
        .materials
        .iter()
        .map(|m| format!("{}:{}", m.id, m.content_hash))
        .chain(input.effective_roles.iter().map(|(k, v)| format!("role:{}={}", k, v.code())))
        .chain(std::iter::once(format!("rules:{}", input.params.contact_tolerance_nm)))
        .collect();
    let input_digest = digest_of_parts(&digest_parts);

    Ok(RecomputedState {
        status: RecomputedState::OK,
        input_digest,
        rule_version_id: String::new(),
        parent_event_id: input.parent_event_id,
        coordinate_basis: "project-nm+y-up".to_string(),
        layers,
        holes,
        components,
        base_edges,
        candidate_edges,
        near_misses,
        ambiguities,
        candidates,
        conflicts,
        diagnostics,
        invalid_reason: None,
        valid_for_head: true,
    })
}

fn prefix_diag(d: &Diagnostic, mat: &str) -> Diagnostic {
    let mut x = d.clone();
    x.position = format!("{mat}:{}", d.position);
    x
}

struct LayerMeta {
    material_id: String,
    layer_key: String,
    role: LayerRole,
    copper_order: Option<u32>,
    bbox: (i64, i64, i64, i64),
}

fn aperture_lookup<'a>(g: &'a GerberDoc, id: &str) -> Option<&'a Aperture> {
    g.apertures.iter().find(|a| a.id == id)
}

/// Radius / half-extents approximation for an aperture, in nm.
fn aperture_stroke(ap: &Aperture) -> i64 {
    match ap.form {
        ShapeForm::Circle => ap.dims.first().copied().unwrap_or(0) / 2,
        ShapeForm::Rect | ShapeForm::Obround => {
            ap.dims.first().copied().unwrap_or(0).min(ap.dims.get(1).copied().unwrap_or(0)) / 2
        }
        ShapeForm::Polygon => ap.dims.first().copied().unwrap_or(0) / 2,
    }
}

type OccOrigin = (String, String); // (occurrence id, polarity code)

fn expand_ops(m: &Material, g: &GerberDoc) -> (Vec<RenderOp>, Vec<(i64, i64)>, Vec<OccOrigin>) {
    let mut render_ops: Vec<RenderOp> = Vec::new();
    let mut anchor_pts: Vec<(i64, i64)> = Vec::new();
    let mut origins: Vec<OccOrigin> = Vec::new();

    for op in &g.ops {
        let mut instances: Vec<RenderInstance> = Vec::new();
        let base_pts: Vec<(i64, i64)> = op.points().iter().map(|p| (p.x, p.y)).collect();

        let sr_count = op.sr.as_ref().map(|s| s.count()).unwrap_or(1);
        for instance in 0..sr_count {
            let (ix, iy) = match &op.sr {
                Some(sr) => ((instance as i64) % sr.nx.max(1), (instance as i64) / sr.nx.max(1)),
                None => (0, 0),
            };
            let (ox, oy) = op.sr.as_ref().map(|sr| sr.offset(ix, iy)).unwrap_or((0, 0));
            let id = format!("{}#{}#i{}", m.id, op.occurrence_id(0).split('#').next().unwrap_or(""), instance);
            let inst_id = format!("{}#{}", m.id, op.occurrence_id(instance));

            let geom = match &op.kind {
                OpKind::Flash { at, aperture } => {
                    let p = &at[0];
                    let geom = flash_geom(g, aperture, p.x + ox, p.y + oy);
                    if let Some(RenderGeom::Circle { cx, cy, .. }) = &geom {
                        anchor_pts.push((*cx, *cy));
                    } else {
                        anchor_pts.push((p.x + ox, p.y + oy));
                    }
                    geom
                }
                OpKind::Draw { a, b, aperture } => {
                    let ap = aperture_lookup(g, aperture);
                    let width = ap.map(|a| a.dims.first().copied().unwrap_or(0)).unwrap_or(0);
                    anchor_pts.push((b.x + ox, b.y + oy));
                    RenderGeom::Segment { ax: a.x + ox, ay: a.y + oy, bx: b.x + ox, by: b.y + oy, width }
                }
                OpKind::Arc { a, b, center, clockwise, aperture } => {
                    let ap = aperture_lookup(g, aperture);
                    let width = ap.map(|a| a.dims.first().copied().unwrap_or(0)).unwrap_or(0);
                    let pts = flatten_arc(a, b, center, *clockwise);
                    let pts: Vec<(i64, i64)> = pts.into_iter().map(|(x, y)| (x + ox, y + oy)).collect();
                    anchor_pts.push((b.x + ox, b.y + oy));
                    let _ = ap;
                    RenderGeom::Polyline { pts, width }
                }
                OpKind::Region { contour } => {
                    let pts: Vec<(i64, i64)> = contour.iter().map(|p| (p.x + ox, p.y + oy)).collect();
                    if let Some(p) = pts.first() {
                        anchor_pts.push(*p);
                    }
                    RenderGeom::Polygon { pts }
                }
            };
            instances.push(RenderInstance { id: inst_id.clone(), ox, oy, geom });
            origins.push((inst_id, op.polarity.code().to_string()));
        }

        let shape = match &op.kind {
            OpKind::Flash { aperture, .. } => format!("flash:{aperture}"),
            OpKind::Draw { aperture, .. } => format!("draw:{aperture}"),
            OpKind::Arc { aperture, .. } => format!("arc:{aperture}"),
            OpKind::Region { .. } => "region".to_string(),
        };
        render_ops.push(RenderOp {
            id: format!("{}#op{}", m.id, op.index),
            op_index: op.index,
            source_line: op.line,
            polarity: op.polarity,
            shape,
            instances,
        });
        let _ = (base_pts, id);
    }
    (render_ops, anchor_pts, origins)
}

fn flash_geom(g: &GerberDoc, aperture: &str, x: i64, y: i64) -> RenderGeom {
    let ap = aperture_lookup(g, aperture);
    match ap {
        Some(ap) if ap.form == ShapeForm::Circle => {
            let r = ap.dims.first().copied().unwrap_or(0) / 2;
            RenderGeom::Circle { cx: x, cy: y, r }
        }
        Some(ap) if ap.form == ShapeForm::Polygon => {
            let r = ap.dims.first().copied().unwrap_or(0) / 2;
            let n = ap.vertices.unwrap_or(8).max(3) as i64;
            let rot = (ap.rotation_ddeg as f64).to_radians();
            let mut pts = Vec::new();
            for i in 0..n {
                let ang = std::f64::consts::TAU * (i as f64) / (n as f64) + rot;
                pts.push(((x as f64 + r as f64 * ang.cos()) as i64, (y as f64 + r as f64 * ang.sin()) as i64));
            }
            RenderGeom::Polygon { pts }
        }
        Some(ap) => {
            let w = ap.dims.first().copied().unwrap_or(0);
            let h = ap.dims.get(1).copied().unwrap_or(w);
            RenderGeom::Polygon {
                pts: vec![
                    (x - w / 2, y - h / 2),
                    (x + w / 2, y - h / 2),
                    (x + w / 2, y + h / 2),
                    (x - w / 2, y + h / 2),
                ],
            }
        }
        None => RenderGeom::Polygon { pts: vec![(x, y)] },
    }
}

fn flatten_arc(a: &Point, b: &Point, center: &Point, clockwise: bool) -> Vec<(i64, i64)> {
    let sx = (a.x - center.x) as f64;
    let sy = (a.y - center.y) as f64;
    let ex = (b.x - center.x) as f64;
    let ey = (b.y - center.y) as f64;
    let r = sx.hypot(sy).max(1.0);
    let a0 = sy.atan2(sx);
    let mut a1 = ey.atan2(ex);
    let dir = if clockwise { -1.0 } else { 1.0 };
    // Choose the sweep in the requested direction.
    let mut sweep = a1 - a0;
    if dir > 0.0 && sweep < 0.0 {
        sweep += std::f64::consts::TAU;
    } else if dir < 0.0 && sweep > 0.0 {
        sweep -= std::f64::consts::TAU;
    }
    a1 = a0 + sweep;
    let steps = ((sweep.abs() / (4.0_f64).to_radians()).ceil() as i64).max(2);
    let mut pts = vec![(a.x, a.y)];
    for i in 1..steps {
        let t = i as f64 / steps as f64;
        let ang = a0 + sweep * t;
        pts.push((
            (center.x as f64 + r * ang.cos()).round() as i64,
            (center.y as f64 + r * ang.sin()).round() as i64,
        ));
    }
    pts.push((b.x, b.y));
    pts
}

fn bbox_of(pts: &[(i64, i64)]) -> (i64, i64, i64, i64) {
    let mut min_x = i64::MAX;
    let mut min_y = i64::MAX;
    let mut max_x = i64::MIN;
    let mut max_y = i64::MIN;
    for (x, y) in pts {
        min_x = min_x.min(*x);
        min_y = min_y.min(*y);
        max_x = max_x.max(*x);
        max_y = max_y.max(*y);
    }
    if min_x == i64::MAX {
        (0, 0, 0, 0)
    } else {
        (min_x, min_y, max_x, max_y)
    }
}

struct Compute<'a> {
    params: &'a RuleParams,
    diagnostics: &'a mut Vec<Diagnostic>,
    layers: &'a [RenderLayer],
    meta: &'a [LayerMeta],
    holes: &'a [RenderHit],
    rasters: Vec<RasterBundle>,
}

struct RasterBundle {
    layer_index: usize,
    raster: Raster,
    labels: Vec<u32>,
    comp_count: u32,
    /// component label -> centroid / bbox / occurrences
    nodes: Vec<ComponentNode>,
}

impl<'a> Compute<'a> {
    fn build_rasters(&mut self) -> Result<(), Failure> {
        for (li, layer) in self.layers.iter().enumerate() {
            if self.meta[li].role != LayerRole::Copper {
                continue;
            }
            let cell = self.params.cell_nm;
            // Bounding box from geometries.
            let mut bbox = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
            for op in &layer.ops {
                for inst in &op.instances {
                    include_geom_bbox(&inst.geom, &mut bbox);
                }
            }
            for h in self.holes {
                if hole_affects_layer(h, self.meta[li].copper_order) {
                    let r = h.diameter_nm / 2;
                    bbox.0 = bbox.0.min(h.x - r);
                    bbox.1 = bbox.1.min(h.y - r);
                    bbox.2 = bbox.2.max(h.x + r);
                    bbox.3 = bbox.3.max(h.y + r);
                }
            }
            if bbox.0 == i64::MAX {
                bbox = (0, 0, 0, 0);
            }
            let width = ((bbox.2 - bbox.0) / cell + 2) as u64;
            let height = ((bbox.3 - bbox.1) / cell + 2) as u64;
            let cells = width.saturating_mul(height);
            if cells > self.params.max_cells {
                return Err(Failure::single(
                    FailureKind::BudgetExhausted,
                    "RASTER_CELL_BUDGET",
                    format!("layer {}", layer.layer_key),
                    format!(
                        "raster requires {cells} cells (limit {}) at cell_nm={cell}",
                        self.params.max_cells
                    ),
                ));
            }
            let mut r = Raster::new(bbox, cell);
            // Paint strictly in operation order, including polarity.
            for op in &layer.ops {
                let value = op.polarity == Polarity::Dark;
                for inst in &op.instances {
                    paint_geom(&mut r, &inst.geom, value);
                }
            }
            // Physical plated/non-plated through-drills remove copper at the
            // barrel footprint on every affected copper layer.
            for h in self.holes {
                if hole_affects_layer(h, self.meta[li].copper_order) {
                    r.fill_circle(h.x, h.y, h.diameter_nm / 2, false);
                }
            }
            let (labels, comp_count) = r.label_components();
            self.rasters.push(RasterBundle {
                layer_index: li,
                raster: r,
                labels,
                comp_count,
                nodes: Vec::new(),
            });
        }
        // Derive nodes per component label.
        for bundle in self.rasters.iter_mut() {
            let li = bundle.layer_index;
            let layer = &self.layers[li];
            let mut sums: HashMap<u32, (i64, i64, u64)> = HashMap::new();
            let mut boxes: HashMap<u32, (i64, i64, i64, i64)> = HashMap::new();
            for (i, lab) in bundle.labels.iter().enumerate() {
                if *lab == 0 {
                    continue;
                }
                let cx = (i % bundle.raster.width as usize) as i64;
                let cy = (i / bundle.raster.width as usize) as i64;
                let x = bundle.raster.min_x + cx * bundle.raster.cell_nm;
                let y = bundle.raster.min_y + cy * bundle.raster.cell_nm;
                let e = sums.entry(*lab).or_insert((0, 0, 0));
                e.0 += x;
                e.1 += y;
                e.2 += 1;
                let b = boxes.entry(*lab).or_insert((x, y, x, y));
                b.0 = b.0.min(x);
                b.1 = b.1.min(y);
                b.2 = b.2.max(x);
                b.3 = b.3.max(y);
            }
            for lab in 1..=bundle.comp_count {
                let Some((sx, sy, n)) = sums.get(&lab) else { continue };
                let cx = sx / *n as i64;
                let cy = sy / *n as i64;
                let bbox = boxes[&lab];
                // Find contributing occurrences by sampling op geometry.
                let mut occs: Vec<String> = Vec::new();
                for op in &layer.ops {
                    for inst in &op.instances {
                        if op.polarity == Polarity::Dark
                            && geom_contains_label(&inst.geom, &bundle.raster, cx, cy, &bundle.labels, *lab)
                        {
                            occs.push(inst.id.clone());
                        }
                    }
                }
                occs.sort();
                occs.dedup();
                let node_id = stable_id(
                    "comp",
                    &format!("{}#L{}#c{}", layer.layer_key, li, lab),
                );
                bundle.nodes.push(ComponentNode {
                    id: node_id,
                    layer_key: layer.layer_key.clone(),
                    copper_order: self.meta[li].copper_order,
                    cx,
                    cy,
                    bbox,
                    occurrences: occs,
                });
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn connectivity(
        &mut self,
    ) -> Result<
        (
            Vec<ComponentNode>,
            Vec<LinkEdge>,
            Vec<LinkEdge>,
            Vec<LinkEdge>,
            Vec<Ambiguity>,
            Vec<CandidateSolution>,
            Vec<Conflict>,
        ),
        Failure,
    > {
        let mut components: Vec<ComponentNode> = Vec::new();
        for bundle in &self.rasters {
            components.extend(bundle.nodes.clone());
        }

        // All candidate edges collected from geometry + holes.
        let mut base_edges: Vec<LinkEdge> = Vec::new();
        let mut candidate_edges: Vec<LinkEdge> = Vec::new();
        let mut near_misses: Vec<LinkEdge> = Vec::new();
        let mut ambiguities: Vec<Ambiguity> = Vec::new();

        // Cross-layer links through plated holes.
        self.hole_links(&components, &mut base_edges, &mut candidate_edges, &mut near_misses, &mut ambiguities);

        // Same-layer tangent contacts between distinct components are detected
        // analytically from source geometry (raster already merged overlaps).
        self.tangent_links(&components, &mut base_edges, &mut candidate_edges, &mut near_misses);

        // Conservative base solution: union only definite edges.
        let parent: HashMap<&str, &str> = HashMap::new();
        let mut uf = UnionFind::new(components.iter().map(|c| c.id.clone()));
        for e in &base_edges {
            uf.union(&e.a, &e.b);
        }
        let _ = parent;

        // Enumerate legal candidate solutions over candidate edges.
        let candidates = self.enumerate_candidates(&components, &base_edges, &candidate_edges);

        let conflicts = Vec::new();
        Ok((components, base_edges, candidate_edges, near_misses, ambiguities, candidates, conflicts))
    }
}

fn include_geom_bbox(g: &RenderGeom, b: &mut (i64, i64, i64, i64)) {
    let mut pts: Vec<(i64, i64)> = Vec::new();
    match g {
        RenderGeom::Circle { cx, cy, r } => {
            pts.push((*cx - *r, *cy - *r));
            pts.push((*cx + *r, *cy + *r));
        }
        RenderGeom::Segment { ax, ay, bx, by, width } => {
            let w = width / 2;
            pts.push((*ax - w, *ay - w));
            pts.push((*bx + w, *by + w));
        }
        RenderGeom::Polyline { pts: p, width } => {
            let w = width / 2;
            for (x, y) in p {
                pts.push((*x - w, *y - w));
                pts.push((*x + w, *y + w));
            }
        }
        RenderGeom::Polygon { pts: p } => pts.extend(p.iter().copied()),
    }
    for (x, y) in pts {
        b.0 = b.0.min(x);
        b.1 = b.1.min(y);
        b.2 = b.2.max(x);
        b.3 = b.3.max(y);
    }
}

fn paint_geom(r: &mut Raster, g: &RenderGeom, value: bool) {
    match g {
        RenderGeom::Circle { cx, cy, r: rad } => {
            r.fill_circle(*cx, *cy, *rad, value);
        }
        RenderGeom::Segment { ax, ay, bx, by, width } => {
            r.fill_line(*ax, *ay, *bx, *by, *width, value);
        }
        RenderGeom::Polyline { pts, width } => {
            for w in pts.windows(2) {
                r.fill_line(w[0].0, w[0].1, w[1].0, w[1].1, *width, value);
            }
        }
        RenderGeom::Polygon { pts } => {
            r.fill_polygon(pts, value);
        }
    }
}

fn geom_contains_label(
    g: &RenderGeom,
    raster: &Raster,
    cx: i64,
    cy: i64,
    labels: &[u32],
    want: u32,
) -> bool {
    // Sample a small 3x3 neighbourhood of the component centroid and test
    // whether the geometry covers any cell carrying the component label.
    let (gcx, gcy) = raster.cell_of(cx, cy);
    let mut found = false;
    for dy in -2..=2 {
        for dx in -2..=2 {
            let x = gcx + dx;
            let y = gcy + dy;
            if x < 0 || y < 0 || x >= raster.width as i64 || y >= raster.height as i64 {
                continue;
            }
            let idx = y as usize * raster.width as usize + x as usize;
            if labels[idx] != want {
                continue;
            }
            let px = raster.min_x + x * raster.cell_nm;
            let py = raster.min_y + y * raster.cell_nm;
            if point_in_geom(px, py, g) {
                found = true;
            }
        }
    }
    found
}

fn point_in_geom(x: i64, y: i64, g: &RenderGeom) -> bool {
    match g {
        RenderGeom::Circle { cx, cy, r } => {
            let dx = x - cx;
            let dy = y - cy;
            dx * dx + dy * dy <= r * r
        }
        RenderGeom::Polygon { pts } => point_in_poly(x, y, pts),
        RenderGeom::Segment { ax, ay, bx, by, width } => {
            dist_point_segment(x, y, *ax, *ay, *bx, *by) <= width / 2
        }
        RenderGeom::Polyline { pts, width } => pts
            .windows(2)
            .any(|w| dist_point_segment(x, y, w[0].0, w[0].1, w[1].0, w[1].1) <= width / 2),
    }
}

fn point_in_poly(x: i64, y: i64, pts: &[(i64, i64)]) -> bool {
    let mut inside = false;
    let mut j = pts.len() - 1;
    for i in 0..pts.len() {
        let (xi, yi) = (pts[i].0 as f64, pts[i].1 as f64);
        let (xj, yj) = (pts[j].0 as f64, pts[j].1 as f64);
        let (xf, yf) = (x as f64, y as f64);
        if (yi > yf) != (yj > yf) && xf < (xj - xi) * (yf - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn dist_point_segment(px: i64, py: i64, ax: i64, ay: i64, bx: i64, by: i64) -> i64 {
    let dx = (bx - ax) as f64;
    let dy = (by - ay) as f64;
    let l2 = dx * dx + dy * dy;
    if l2 == 0.0 {
        return ((px - ax) as f64).hypot((py - ay) as f64) as i64;
    }
    let t = (((px - ax) as f64 * dx + (py - ay) as f64 * dy) / l2).clamp(0.0, 1.0);
    let cx = ax as f64 + t * dx;
    let cy = ay as f64 + t * dy;
    ((px as f64 - cx).hypot(py as f64 - cy)) as i64
}

/// A drill affects a copper layer when it is through-hole (no range) or the
/// layer's copper order is within the blind/buried inclusive range.
fn hole_affects_layer(h: &RenderHit, copper_order: Option<u32>) -> bool {
    if copper_order.is_none() {
        return false;
    }
    match h.layer_range {
        Some((lo, hi)) => copper_order.unwrap() >= lo && copper_order.unwrap() <= hi,
        None => true,
    }
}

impl<'a> Compute<'a> {
    fn hole_links(
        &self,
        components: &[ComponentNode],
        base_edges: &mut Vec<LinkEdge>,
        candidate_edges: &mut Vec<LinkEdge>,
        near_misses: &mut Vec<LinkEdge>,
        ambiguities: &mut Vec<Ambiguity>,
    ) {
        for h in self.holes {
            if !h.plated {
                // Non-plated holes never conduct. Record a near-miss if copper
                // lies inside the tolerance band, to make the distinction
                // explicit in the review UI.
                for n in components {
                    let d = point_box_distance(h.x, h.y, n.bbox);
                    if d <= self.params.contact_tolerance_nm * 3 {
                        near_misses.push(LinkEdge {
                            id: stable_id("nm", &format!("{}#{}", h.id, n.id)),
                            kind: ConnectionKind::NearMiss.kind(),
                            a: h.id.clone(),
                            b: n.id.clone(),
                            distance_nm: d,
                            via_hit: Some(h.id.clone()),
                            source_ref: format!("{}:L{}", h.material_id, h.source_line),
                        });
                    }
                }
                continue;
            }
            // Plated: find copper components on each affected layer.
            let touched: Vec<&ComponentNode> = components
                .iter()
                .filter(|n| {
                    n.copper_order
                        .map(|o| match h.layer_range {
                            Some((lo, hi)) => o >= lo && o <= hi,
                            None => true,
                        })
                        .unwrap_or(false)
                })
                .collect();
            // Distance from hole centre to component bbox.
            let mut groups: Vec<(u32, Vec<&ComponentNode>)> = Vec::new();
            for n in touched {
                let order = n.copper_order.unwrap();
                let d = point_box_distance(h.x, h.y, n.bbox);
                let entry = groups.iter_mut().find(|(o, _)| *o == order);
                if let Some(e) = entry {
                    e.1.push(n);
                } else {
                    groups.push((order, vec![n]));
                }
                let _ = d;
            }

            // Edges between components on distinct layers sharing the barrel.
            let mut layer_nodes: Vec<(u32, &ComponentNode, i64)> = Vec::new();
            for (o, nodes) in &groups {
                // Pick nearest component on the layer for definite linking.
                let mut best: Option<(&ComponentNode, i64)> = None;
                for n in nodes {
                    let d = point_box_distance(h.x, h.y, n.bbox);
                    if best.map(|(_, bd)| d < bd).unwrap_or(true) {
                        best = Some((n, d));
                    }
                }
                if let Some((n, d)) = best {
                    layer_nodes.push((*o, n, d));
                }
            }

            for i in 0..layer_nodes.len() {
                for j in (i + 1)..layer_nodes.len() {
                    let (_, na, da) = layer_nodes[i];
                    let (_, nb, db) = layer_nodes[j];
                    let d = da.max(db);
                    let source_ref = format!("{}:L{}", h.material_id, h.source_line);
                    let edge = LinkEdge {
                        id: stable_id("edge", &format!("{}#{}#{}", h.id, na.id, nb.id)),
                        kind: ConnectionKind::DefiniteContact.kind(),
                        a: na.id.clone(),
                        b: nb.id.clone(),
                        distance_nm: d,
                        via_hit: Some(h.id.clone()),
                        source_ref,
                    };
                    if d <= self.params.contact_tolerance_nm {
                        base_edges.push(edge);
                    } else if d <= self.params.contact_tolerance_nm * 3 {
                        near_misses.push(LinkEdge {
                            id: stable_id("nm", &format!("{}#{}#{}", h.id, h.id, nb.id)),
                            kind: ConnectionKind::NearMiss.kind(),
                            a: na.id.clone(),
                            b: nb.id.clone(),
                            distance_nm: d,
                            via_hit: Some(h.id.clone()),
                            source_ref: format!("{}:L{}", h.material_id, h.source_line),
                        });
                    }
                }
            }

            // Same layer, multiple distinct components touch the barrel =>
            // without a netlist the conduction is ambiguous: keep every legal
            // candidate instead of choosing one.
            for (o, nodes) in &groups {
                if nodes.len() >= 2 {
                    let mut edges = Vec::new();
                    for x in 0..nodes.len() {
                        for y in (x + 1)..nodes.len() {
                            let eid = stable_id(
                                "ambedge",
                                &format!("{}#{}#{}", h.id, nodes[x].id, nodes[y].id),
                            );
                            let d = point_box_distance(h.x, h.y, nodes[x].bbox)
                                .max(point_box_distance(h.x, h.y, nodes[y].bbox));
                            candidate_edges.push(LinkEdge {
                                id: eid.clone(),
                                kind: ConnectionKind::ToleranceContact.kind(),
                                a: nodes[x].id.clone(),
                                b: nodes[y].id.clone(),
                                distance_nm: d,
                                via_hit: Some(h.id.clone()),
                                source_ref: format!("{}:L{}", h.material_id, h.source_line),
                            });
                            edges.push(eid);
                        }
                    }
                    ambiguities.push(Ambiguity {
                        id: stable_id("amb", &format!("{}#L{}", h.id, o)),
                        description: format!(
                            "plated hole {} touches {} separate copper components on layer L{}; no netlist: all legal candidates retained",
                            h.id,
                            nodes.len(),
                            o
                        ),
                        edges,
                        source_ref: format!("{}:L{}", h.material_id, h.source_line),
                    });
                }
            }
        }
    }

    fn tangent_links(
        &self,
        components: &[ComponentNode],
        base_edges: &mut Vec<LinkEdge>,
        candidate_edges: &mut Vec<LinkEdge>,
        near_misses: &mut Vec<LinkEdge>,
    ) {
        // Compare pairs of components on the same layer whose bboxes are close.
        for i in 0..components.len() {
            for j in (i + 1)..components.len() {
                let a = &components[i];
                let b = &components[j];
                if a.layer_key != b.layer_key {
                    continue;
                }
                let d = box_box_distance(a.bbox, b.bbox);
                if d > self.params.contact_tolerance_nm * 3 {
                    continue;
                }
                // d == 0 here normally means raster already merged them; only
                // emit an edge for genuinely separate labels (ids differ).
                let source_ref = format!("layer:{}", a.layer_key);
                if d == 0 {
                    continue;
                }
                let id = stable_id("tan", &format!("{}#{}", a.id, b.id));
                let edge = LinkEdge {
                    id,
                    kind: if d <= self.params.contact_tolerance_nm {
                        ConnectionKind::ToleranceContact.kind()
                    } else {
                        ConnectionKind::NearMiss.kind()
                    },
                    a: a.id.clone(),
                    b: b.id.clone(),
                    distance_nm: d,
                    via_hit: None,
                    source_ref,
                };
                if d <= self.params.contact_tolerance_nm {
                    // Exact tangency policy is versioned.
                    match self.params.tangent_policy.as_str() {
                        super::rules::TANGENT_CONNECTED => base_edges.push(LinkEdge {
                            kind: ConnectionKind::Tangent.kind(),
                            ..edge
                        }),
                        super::rules::TANGENT_GAP => near_misses.push(edge),
                        _ => candidate_edges.push(edge),
                    }
                } else {
                    near_misses.push(edge);
                }
            }
        }
    }

    fn enumerate_candidates(
        &self,
        components: &[ComponentNode],
        base_edges: &[LinkEdge],
        candidate_edges: &[LinkEdge],
    ) -> Vec<CandidateSolution> {
        // Base solution first (rank 0, no candidate edges).
        let mut solutions: Vec<CandidateSolution> = Vec::new();
        let groups_of = |extra: &[&LinkEdge]| -> Vec<Vec<String>> {
            let mut uf = UnionFind::new(components.iter().map(|c| c.id.clone()));
            for e in base_edges {
                uf.union(&e.a, &e.b);
            }
            for e in extra {
                uf.union(&e.a, &e.b);
            }
            uf.groups_sorted()
        };
        let base_groups = groups_of(&[]);
        solutions.push(CandidateSolution {
            id: "cand-base".to_string(),
            rank: 0,
            accepted_edges: vec![],
            component_groups: base_groups,
        });

        // Each independent candidate edge yields a distinct legal solution;
        // bounded by max_candidates so ambiguity enumeration is budgeted.
        let mut rank = 1usize;
        for e in candidate_edges.iter().take(self.params.max_candidates.saturating_sub(1)) {
            let groups = groups_of(&[e]);
            solutions.push(CandidateSolution {
                id: stable_id("cand", &format!("{}#{}", e.id, rank)),
                rank,
                accepted_edges: vec![e.id.clone()],
                component_groups: groups,
            });
            rank += 1;
        }
        // Also enumerate the all-accepted solution when there are 2+ edges.
        if candidate_edges.len() >= 2 && rank < self.params.max_candidates {
            let all: Vec<&LinkEdge> = candidate_edges.iter().collect();
            let groups = groups_of(&all);
            solutions.push(CandidateSolution {
                id: stable_id("cand", "all-candidate-edges"),
                rank,
                accepted_edges: candidate_edges.iter().map(|e| e.id.clone()).collect(),
                component_groups: groups,
            });
        }
        // Stable order by rank, then by id.
        solutions.sort_by(|a, b| a.rank.cmp(&b.rank).then_with(|| a.id.cmp(&b.id)));
        solutions
    }
}

fn point_box_distance(x: i64, y: i64, b: (i64, i64, i64, i64)) -> i64 {
    let dx = (b.0 - x).max(0).max(x - b.2);
    let dy = (b.1 - y).max(0).max(y - b.3);
    if dx == 0 && dy == 0 {
        0
    } else if dx == 0 {
        dy
    } else if dy == 0 {
        dx
    } else {
        (dx as f64).hypot(dy as f64) as i64
    }
}

fn box_box_distance(a: (i64, i64, i64, i64), b: (i64, i64, i64, i64)) -> i64 {
    let dx = (a.0 - b.2).max(0).max(b.0 - a.2);
    let dy = (a.1 - b.3).max(0).max(b.1 - a.3);
    if dx == 0 && dy == 0 {
        0
    } else if dx == 0 {
        dy
    } else if dy == 0 {
        dx
    } else {
        (dx as f64).hypot(dy as f64) as i64
    }
}

impl ConnectionKind {
    fn kind(self) -> String {
        match self {
            ConnectionKind::DefiniteContact => "definite_contact".to_string(),
            ConnectionKind::Tangent => "tangent".to_string(),
            ConnectionKind::ToleranceContact => "tolerance_contact".to_string(),
            ConnectionKind::NearMiss => "near_miss".to_string(),
        }
    }
}

struct UnionFind {
    parent: BTreeMap<String, String>,
}

impl UnionFind {
    fn new(ids: impl Iterator<Item = String>) -> Self {
        let mut parent = BTreeMap::new();
        for id in ids {
            parent.insert(id.clone(), id);
        }
        Self { parent }
    }
    fn find(&mut self, x: &str) -> String {
        let p = self.parent.get(x).cloned().unwrap_or_else(|| x.to_string());
        if p == x {
            return x.to_string();
        }
        let root = self.find(&p);
        self.parent.insert(x.to_string(), root.clone());
        root
    }
    fn union(&mut self, a: &str, b: &str) {
        self.parent.entry(a.to_string()).or_insert_with(|| a.to_string());
        self.parent.entry(b.to_string()).or_insert_with(|| b.to_string());
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            // Deterministic root: lexical minimum keeps ordering stable.
            let (small, large) = if ra < rb { (ra, rb) } else { (rb, ra) };
            self.parent.insert(large, small);
        }
    }
    fn groups_sorted(&mut self) -> Vec<Vec<String>> {
        let keys: Vec<String> = self.parent.keys().cloned().collect();
        let mut groups: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for k in keys {
            let root = self.find(&k);
            groups.entry(root).or_default().insert(k);
        }
        let mut out: Vec<Vec<String>> = groups.into_values().map(|g| g.into_iter().collect()).collect();
        out.sort_by(|a, b| a.first().cmp(&b.first()));
        out
    }
}

#[allow(dead_code)]
fn ensure_used(_: BTreeMap<String, String>) {}
