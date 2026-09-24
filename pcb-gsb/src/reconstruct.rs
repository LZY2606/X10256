//! Reconstruction pipeline: copper/mask compositing, holes and connectivity candidates.
//!
//! Rules:
//! * dark/clear polarity is composited strictly in source operation order;
//! * geometric contact tolerance comes from the *versioned* `RuleParams`;
//! * without a netlist only connectivity *candidates* are produced. Proximity never
//!   becomes conduction; corner/point contact stays a human-decided candidate;
//! * every derived object has a deterministic stable id built from its provenance.

use crate::excellon::{ExcellonDoc, Plating};
use crate::geometry::{GridDef, Mask, Pt};
use crate::gerber::GerberDoc;
use crate::model::{Diagnostic, LayerRole, Polarity, RuleParams};
use crate::stackup::StackupDoc;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceRef {
    pub source_id: String,
    pub filename: String,
    pub kind: String,
    pub sha256: String,
    pub import_order: usize,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HoleView {
    pub id: String,
    pub source_id: String,
    pub tool: String,
    pub x_mm: Option<f64>,
    pub y_mm: Option<f64>,
    pub x_raw: String,
    pub y_raw: String,
    pub d_mm: Option<f64>,
    pub plating: Plating,
    pub span_from: u32,
    pub span_to: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayerView {
    pub stack_index: u32,
    pub name: String,
    pub role: LayerRole,
    pub source_id: Option<String>,
    pub grid: GridDef,
    /// run-length encoded mask: (run of covered count, run of empty count) alternating.
    pub rle: Vec<u64>,
    pub covered_cells: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PtView {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentView {
    pub id: String,
    pub copper_layer: u32,
    pub label: u32,
    pub merged_atoms: Vec<String>,
    pub cells: u64,
    pub centroid: PtView,
    pub bbox: (PtView, PtView),
    pub net_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateKind {
    PointTangency,
    Proximity,
    PointViaAlignment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateStatus {
    Open,
    Accepted,
    Rejected,
    Contradicted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateView {
    pub id: String,
    pub kind: CandidateKind,
    pub from_component: String,
    pub to_component: String,
    pub gap_mm: f64,
    pub at: PtView,
    pub evidence: Vec<String>,
    pub status: CandidateStatus,
    pub decided_by_event: Option<String>,
    pub rule_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AmbiguityView {
    pub id: String,
    pub summary: String,
    pub candidate_ids: Vec<String>,
    pub options: Vec<AmbiguityOption>,
    pub budget_exhausted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AmbiguityOption {
    pub name: String,
    pub accepted: Vec<String>,
    pub rejected: Vec<String>,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContradictionKind {
    ConflictingCandidateDecision,
    LayerRoleMismatch,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContradictionView {
    pub id: String,
    pub kind: ContradictionKind,
    pub message: String,
    pub at: PtView,
    pub event_ids: Vec<String>,
    pub object_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputSummaryItem {
    pub source_id: String,
    pub filename: String,
    pub kind: String,
    pub sha256: String,
    pub bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reconstruction {
    pub rule_version: String,
    pub input_summary: Vec<InputSummaryItem>,
    pub parent_event_id: String,
    pub parent_dependencies: Vec<String>,
    pub decision_events: Vec<String>,
    pub layers: Vec<LayerView>,
    pub holes: Vec<HoleView>,
    pub components: Vec<ComponentView>,
    pub candidates: Vec<CandidateView>,
    pub ambiguities: Vec<AmbiguityView>,
    pub contradictions: Vec<ContradictionView>,
    pub diagnostics: Vec<Diagnostic>,
    pub extent: (PtView, PtView),
    pub grid_res_mm: f64,
    pub budget_exhausted: Vec<String>,
    pub status: OutcomeStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeStatus {
    Ok,
    FormatDamaged,
    SemanticUnresolved,
    ResourceBudget,
}

/// Structured failure: the three required outcomes are distinct and carry evidence.
#[derive(Debug, Clone, PartialEq)]
pub enum ComputeOutcome {
    Complete(Box<Reconstruction>),
    BudgetExhausted { partial: Box<Reconstruction>, reasons: Vec<String> },
    FormatDamaged { diagnostics: Vec<Diagnostic>, partial: Option<Box<Reconstruction>> },
    SemanticUnresolved(Box<Reconstruction>),
}

fn warn_placeholder() -> Diagnostic {
    unreachable!()
}
#[allow(dead_code)]
fn _unused(_: Diagnostic) {
    let _ = warn_placeholder;
}

pub struct ParseBundle {
    pub gerbers: Vec<GerberDoc>,
    pub excellons: Vec<ExcellonDoc>,
    pub stackup: StackupDoc,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ManualOverrides {
    /// stack layer name -> manually assigned role (layer-role revision)
    pub role_by_layer: std::collections::BTreeMap<String, LayerRole>,
    /// stack layer name -> source id (manual mapping revision)
    pub map_by_layer: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub event_id: String,
    pub candidate_id: String,
    pub accept: bool,
    pub note: String,
}

fn sha_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

pub fn source_hash(content: &str) -> String {
    sha_hex(content.as_bytes())
}

fn rle_encode(cells: &[u8]) -> Vec<u64> {
    let mut out = Vec::new();
    let mut covered = 0u64;
    let mut empty = 0u64;
    for c in cells {
        if *c == 1 {
            if empty > 0 {
                out.push(covered);
                out.push(empty);
                covered = 0;
                empty = 0;
            }
            covered += 1;
        } else {
            empty += 1;
        }
    }
    out.push(covered);
    out.push(empty);
    out
}

struct CompositeLayer {
    stack_index: u32,
    name: String,
    role: LayerRole,
    source_id: Option<String>,
    mask: Mask,
}

fn composite_layer(
    stack_index: u32,
    name: &str,
    role: LayerRole,
    grid: GridDef,
    gerber: Option<&GerberDoc>,
    diagnostics: &mut Vec<Diagnostic>,
) -> CompositeLayer {
    let mut mask = Mask::empty(grid);
    let mut source_id = None;
    if let Some(g) = gerber {
        source_id = Some(g.source_id.clone());
        diagnostics.extend(g.diagnostics.iter().cloned());
        let draw = g.to_shapes(diagnostics);
        // Polarity composited strictly in op order; clear subtracts everything drawn so far.
        for ds in &draw.shapes {
            match ds.polarity {
                Polarity::Dark => mask.paint_shape(&ds.shape),
                Polarity::Clear => {
                    let mut cut = Mask::empty(grid);
                    cut.paint_shape(&ds.shape);
                    mask.subtract(&cut);
                }
            }
        }
    }
    CompositeLayer {
        stack_index,
        name: name.to_string(),
        role,
        source_id,
        mask,
    }
}

struct Atom {
    id: String,
    layer: u32,
    label: u32,
    cells: u64,
    centroid: Pt,
    bmin: Pt,
    bmax: Pt,
    comp_id: Option<String>,
}

impl ComputeOutcome {
    pub fn as_reconstruction(&self) -> Option<&Reconstruction> {
        match self {
            ComputeOutcome::Complete(r) => Some(r),
            ComputeOutcome::SemanticUnresolved(r) => Some(r),
            ComputeOutcome::FormatDamaged { partial, .. } => partial.as_deref(),
            ComputeOutcome::BudgetExhausted { partial, .. } => Some(partial),
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn reconstruct(
    bundle: &ParseBundle,
    sources: &[SourceRef],
    params: &RuleParams,
    overrides: &ManualOverrides,
    decisions: &[DecisionRecord],
    parent_event_id: &str,
    parent_dependencies: &[String],
    decision_event_ids: &[String],
    format_blockers: Vec<Diagnostic>,
) -> ComputeOutcome {
    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    if !format_blockers.is_empty() {
        return ComputeOutcome::FormatDamaged {
            diagnostics: format_blockers,
            partial: None,
        };
    }

    let stackup = &bundle.stackup;
    let n_copper = stackup.copper_layers().len().max(1) as u32;

    // ----- 1. resolve effective role/mapping (manual revisions applied, never overwrite raw) -----
    struct ResolvedLayer<'a> {
        stack: &'a crate::stackup::StackLayer,
        role: LayerRole,
        gerber: Option<&'a GerberDoc>,
    }
    let mut resolved: Vec<ResolvedLayer> = Vec::new();
    let mut role_conflicts: Vec<ContradictionView> = Vec::new();
    for layer in &stackup.layers {
        let mut role = layer.role;
        if let Some(new_role) = overrides.role_by_layer.get(&layer.name) {
            if *new_role != role {
                role_conflicts.push(ContradictionView {
                    id: format!("contr:role:{}", layer.name),
                    kind: ContradictionKind::LayerRoleMismatch,
                    message: format!(
                        "manual role revision: `{}` {} -> {} (raw stackup line retained)",
                        layer.name,
                        layer.role.as_str(),
                        new_role.as_str()
                    ),
                    at: PtView { x: 0.0, y: 0.0 },
                    event_ids: decision_event_ids.to_vec(),
                    object_ids: vec![format!("layer:{}", layer.name)],
                });
            }
            role = *new_role;
        }
        let bind_name = overrides
            .map_by_layer
            .get(&layer.name)
            .cloned()
            .or_else(|| layer.mapped_file.clone());
        let gerber = bind_name.and_then(|fname| {
            bundle
                .gerbers
                .iter()
                .find(|g| sources.iter().any(|s| s.source_id == g.source_id && s.filename == fname))
        });
        resolved.push(ResolvedLayer {
            stack: layer,
            role,
            gerber,
        });
    }

    // ----- 2. global extent from all drawable shapes & holes (mm) -----
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    let mut include_pt = |x: f64, y: f64| {
        if x.is_finite() {
            min_x = min_x.min(x);
            max_x = max_x.max(x);
        }
        if y.is_finite() {
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
    };
    let mut tmp_diag = Vec::new();
    for r in &resolved {
        if let Some(g) = r.gerber {
            let draw = g.to_shapes(&mut tmp_diag);
            for ds in &draw.shapes {
                match &ds.shape {
                    crate::geometry::Shape::Circle(c) => {
                        include_pt(c.c.x - c.d / 2.0, c.c.y - c.d / 2.0);
                        include_pt(c.c.x + c.d / 2.0, c.c.y + c.d / 2.0);
                    }
                    crate::geometry::Shape::Rect(rc) => {
                        include_pt(rc.min.x, rc.min.y);
                        include_pt(rc.max.x, rc.max.y);
                    }
                    crate::geometry::Shape::Polygon { pts } => {
                        for p in pts {
                            include_pt(p.x, p.y);
                        }
                    }
                    crate::geometry::Shape::Segment { a, b, width } => {
                        include_pt(a.x - width, a.y - width);
                        include_pt(b.x + width, b.y + width);
                    }
                }
            }
        }
    }
    for ex in &bundle.excellons {
        for h in &ex.holes {
            if let (Some(x), Some(y)) = (h.x_mm, h.y_mm) {
                include_pt(x, y);
            }
        }
    }
    if !min_x.is_finite() {
        min_x = 0.0;
        min_y = 0.0;
        max_x = 10.0;
        max_y = 10.0;
    }
    let pad = 0.5;
    min_x -= pad;
    min_y -= pad;
    max_x += pad;
    max_y += pad;
    let width_mm = max_x - min_x;
    let height_mm = max_y - min_y;

    // ----- 3. resource budget check (grid & ambiguity) -----
    let cols = (width_mm / params.grid_res_mm).ceil() as u64;
    let rows = (height_mm / params.grid_res_mm).ceil() as u64;
    let total = cols.checked_mul(rows).unwrap_or(u64::MAX);
    let mut budget_exhausted: Vec<String> = Vec::new();
    let grid_oversize = total > params.max_grid_cells;
    if grid_oversize {
        budget_exhausted.push(format!(
            "grid-allocation:{cols}x{rows}={total}>{}",
            params.max_grid_cells
        ));
        diagnostics.push(Diagnostic::err(
            "B001",
            "-",
            None,
            format!(
                "grid {cols}x{rows} = {total} cells exceeds budget {} under {}",
                params.max_grid_cells, params.version
            ),
        ));
    }
    let grid = GridDef {
        res_mm: params.grid_res_mm,
        origin_x: min_x,
        origin_y: min_y,
        width: cols as usize,
        height: rows as usize,
    };

    // ----- 4. composite each layer (polarity strictly in op order) -----
    let mut composites: Vec<CompositeLayer> = Vec::new();
    let mut layer_views: Vec<LayerView> = Vec::new();
    for r in &resolved {
        if grid_oversize {
            layer_views.push(LayerView {
                stack_index: r.stack.index,
                name: r.stack.name.clone(),
                role: r.role,
                source_id: r.gerber.map(|g| g.source_id.clone()),
                grid,
                rle: vec![0],
                covered_cells: 0,
            });
            composites.push(CompositeLayer {
                stack_index: r.stack.index,
                name: r.stack.name.clone(),
                role: r.role,
                source_id: r.gerber.map(|g| g.source_id.clone()),
                mask: Mask::empty(GridDef { width: 0, height: 0, ..grid }),
            });
        } else {
            let comp = composite_layer(r.stack.index, &r.stack.name, r.role, grid, r.gerber, &mut diagnostics);
            let rle = rle_encode(&comp.mask.cells);
            let covered = comp.mask.count();
            layer_views.push(LayerView {
                stack_index: r.stack.index,
                name: r.stack.name.clone(),
                role: r.role,
                source_id: comp.source_id.clone(),
                grid,
                rle,
                covered_cells: covered,
            });
            composites.push(comp);
        }
    }

    // ----- 5. holes -----
    let mut holes: Vec<HoleView> = Vec::new();
    for ex in &bundle.excellons {
        diagnostics.extend(ex.diagnostics.iter().cloned());
        for h in &ex.holes {
            let tool = ex.tools.iter().find(|t| t.code == h.tool);
            let (span_from, mut span_to) = ex.span_for(&h.tool, n_copper);
            if span_to < span_from {
                span_to = span_from;
                diagnostics.push(Diagnostic::err(
                    "B102",
                    &ex.source_id,
                    Some(h.line),
                    format!("hole {} span {}-{} inverted; clamped", h.tool, span_from, span_to),
                ));
            }
            holes.push(HoleView {
                id: format!("hole:{}:{}", ex.source_id, h.order),
                source_id: ex.source_id.clone(),
                tool: h.tool.clone(),
                x_mm: h.x_mm,
                y_mm: h.y_mm,
                x_raw: h.x_raw.clone(),
                y_raw: h.y_raw.clone(),
                d_mm: tool.and_then(|t| t.diameter.value_mm),
                plating: tool.map(|t| t.plating).unwrap_or(Plating::Unknown),
                span_from,
                span_to,
            });
        }
    }

    // ----- 6. label 4-connected copper atoms per layer -----
    let mut atoms: Vec<Atom> = Vec::new();
    for comp in &composites {
        if !comp.role.is_copper() {
            continue;
        }
        let (labels, sizes) = comp.mask.label_components_4();
        let w = grid.width;
        let h = grid.height;
        let n_lab = sizes.len() as u32;
        for label in 1..=n_lab {
            let (mut sx, mut sy, mut cnt) = (0.0f64, 0.0f64, 0u64);
            let (mut rmin, mut rmax, mut cmin, mut cmax) = (h, 0usize, w, 0usize);
            for r in 0..h {
                for c in 0..w {
                    if labels[r * w + c] == label {
                        cnt += 1;
                        sx += grid.x_of(c);
                        sy += grid.y_of(r);
                        rmin = rmin.min(r);
                        rmax = rmax.max(r);
                        cmin = cmin.min(c);
                        cmax = cmax.max(c);
                    }
                }
            }
            if cnt == 0 {
                continue;
            }
            let centroid = Pt::new(sx / cnt as f64, sy / cnt as f64);
            atoms.push(Atom {
                id: format!(
                    "atom:L{}:{}",
                    comp.stack_index,
                    // deterministic sub-id: top-left cell of the label
                    topleft_token(&labels, w, label)
                ),
                layer: comp.stack_index,
                label,
                cells: cnt,
                centroid,
                bmin: Pt::new(grid.x_of(cmin), grid.y_of(rmin)),
                bmax: Pt::new(grid.x_of(cmax), grid.y_of(rmax)),
                comp_id: None,
            });
        }
    }
    // stable atom ordering: layer, then centroid x/y
    atoms.sort_by(|a, b| {
        a.layer
            .cmp(&b.layer)
            .then(coord_ord(a.centroid.x, b.centroid.x))
            .then(coord_ord(a.centroid.y, b.centroid.y))
    });

    // index atoms by layer for lookup
    let mask_of_layer: std::collections::BTreeMap<u32, &Mask> =
        composites.iter().map(|c| (c.stack_index, &c.mask)).collect();

    // ----- 7. definite merging through plated holes whose span covers both layers -----
    // DSU over atom indices
    let na = atoms.len();
    let mut dsu: Vec<usize> = (0..na).collect();
    fn find(dsu: &mut [usize], x: usize) -> usize {
        let mut r = x;
        while dsu[r] != r {
            r = dsu[r];
        }
        let mut c = x;
        while dsu[c] != c {
            let n = dsu[c];
            dsu[c] = r;
            c = n;
        }
        r
    }
    fn unite(dsu: &mut [usize], a: usize, b: usize) {
        let ra = find(dsu, a);
        let rb = find(dsu, b);
        if ra != rb {
            // deterministic: smaller index becomes root
            if ra < rb {
                dsu[rb] = ra;
            } else {
                dsu[ra] = rb;
            }
        }
    }

    // atom label lookup for direct cell hits (label image cached per layer)
    let mut label_img: std::collections::BTreeMap<u32, (Vec<u32>, usize)> = Default::default();
    for comp in &composites {
        if comp.role.is_copper() {
            let (lab, _sizes) = comp.mask.label_components_4();
            label_img.insert(comp.stack_index, (lab, grid.width));
        }
    }
    let atom_by_layer_label: std::collections::BTreeMap<(u32, u32), usize> = atoms
        .iter()
        .enumerate()
        .map(|(i, a)| ((a.layer, a.label), i))
        .collect();

    fn labels_under_circle(
        grid: &GridDef,
        lab: &[u32],
        x: f64,
        y: f64,
        d: f64,
    ) -> Vec<u32> {
        let radius = (d * 0.5).max(grid.res_mm * 0.5);
        let cc = ((x - grid.origin_x) / grid.res_mm) as i64;
        let rr = ((y - grid.origin_y) / grid.res_mm) as i64;
        let k = (radius / grid.res_mm).ceil() as i64 + 1;
        let mut found = std::collections::BTreeSet::new();
        for r in (rr - k)..=(rr + k) {
            for c in (cc - k)..=(cc + k) {
                if c < 0 || r < 0 || c >= grid.width as i64 || r >= grid.height as i64 {
                    continue;
                }
                let v = lab[(r as usize) * grid.width + c as usize];
                if v != 0 {
                    found.insert(v);
                }
            }
        }
        found.into_iter().collect()
    }

    let mut hole_hits: Vec<Vec<usize>> = Vec::new();
    for hole in &holes {
        let mut hits = Vec::new();
        let (x, y) = match (hole.x_mm, hole.y_mm) {
            (Some(x), Some(y)) => (x, y),
            _ => {
                hole_hits.push(Vec::new());
                continue;
            }
        };
        for layer in hole.span_from..=hole.span_to {
            if let Some((lab, _w)) = label_img.get(&layer) {
                for label in labels_under_circle(&grid, lab, x, y, hole.d_mm.unwrap_or(grid.res_mm)) {
                    if let Some(&ai) = atom_by_layer_label.get(&(layer, label)) {
                        hits.push(ai);
                    }
                }
            }
        }
        hits.sort();
        hits.dedup();
        hole_hits.push(hits);
    }

    for (hi, hole) in holes.iter().enumerate() {
        if !matches!(hole.plating, Plating::Pth) {
            continue;
        }
        let hits = &hole_hits[hi];
        // span-limited: blind/buried holes only merge copper within the declared range.
        if hits.len() >= 2 {
            for w in hits.windows(2) {
                unite(&mut dsu, w[0], w[1]);
            }
        }
    }
            // connect every hit within the span (one plated barrel)
            if hits.len() >= 2 {
                for w in hits.windows(2) {
                    unite(&mut dsu, w[0], w[1]);
                }
            }
        }
        let _ = conductive;
    }

    // ----- 8. build component views from DSU roots -----
    let mut root_to_comp: std::collections::BTreeMap<usize, String> = Default::default();
    let mut comp_members: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
    for ai in 0..na {
        let root = find(&mut dsu, ai);
        comp_members.entry(root).or_default().push(ai);
    }
    let mut components: Vec<ComponentView> = Vec::new();
    for members in comp_members.values() {
        let root = members[0];
        let first = &atoms[members[0]];
        // deterministic component id: smallest atom id within group
        let lead = members
            .iter()
            .map(|i| atoms[*i].id.clone())
            .min()
            .unwrap_or_else(|| first.id.clone());
        let cid = format!("comp:{}", lead.trim_start_matches("atom:"));
        root_to_comp.insert(root, cid.clone());
        let mut sx = 0.0;
        let mut sy = 0.0;
        let mut cells = 0u64;
        let (mut bminx, mut bminy, mut bmaxx, mut bmaxy) =
            (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        let mut merged = Vec::new();
        for &i in members {
            let a = &atoms[i];
            a.comp_id = Some(cid.clone());
            merged.push(a.id.clone());
            sx += a.centroid.x * a.cells as f64;
            sy += a.centroid.y * a.cells as f64;
            cells += a.cells;
            bminx = bminx.min(a.bmin.x);
            bminy = bminy.min(a.bmin.y);
            bmaxx = bmaxx.max(a.bmax.x);
            bmaxy = bmaxy.max(a.bmax.y);
        }
        components.push(ComponentView {
            id: cid,
            copper_layer: first.layer,
            label: first.label,
            merged_atoms: merged,
            cells,
            centroid: PtView {
                x: sx / cells as f64,
                y: sy / cells as f64,
            },
            bbox: (
                PtView { x: bminx, y: bminy },
                PtView { x: bmaxx, y: bmaxy },
            ),
            net_hint: None,
        });
    }
    components.sort_by(|a, b| a.id.cmp(&b.id));

    // ----- 9. connectivity candidates (never auto-conductive without a netlist) -----
    let comp_of_atom = |ai: usize| -> String {
        let root = find(&mut dsu, ai);
        root_to_comp[&root].clone()
    };
    let mut candidates: Vec<CandidateView> = Vec::new();

    // 9a. point tangency on the same layer: foreground cells touching only diagonally
    // through a corner (4-connected background between them).
    for comp in &composites {
        if !comp.role.is_copper() {
            continue;
        }
        let (lab, _sizes) = comp.mask.label_components_4();
        let w = grid.width;
        let h = grid.height;
        let mut seen: std::collections::BTreeSet<(u32, u32, i64, i64)> = Default::default();
        for r in 0..(h - 1) {
            for c in 0..(w - 1) {
                let p00 = lab[r * w + c];
                let p11 = lab[(r + 1) * w + (c + 1)];
                let p01 = lab[r * w + (c + 1)];
                let p10 = lab[(r + 1) * w + c];
                if p00 != 0 && p11 != 0 && p00 != p11 && p01 == 0 && p10 == 0 {
                    record_tangency(
                        &lab, &atoms, &mut dsu, comp.stack_index, r, c, r + 1, c + 1,
                        &mut seen, &mut candidates, grid, params,
                    );
                }
                if p01 != 0 && p10 != 0 && p01 != p10 && p00 == 0 && p11 == 0 {
                    record_tangency(
                        &lab, &atoms, &mut dsu, comp.stack_index, r, c + 1, r + 1, c,
                        &mut seen, &mut candidates, grid, params,
                    );
                }
            }
        }
    }

    // 9b. proximity across *different* components on the same layer: nearest grid gap
    // within the versioned tolerance, excluding already-definite (same component).
    if params.proximity_tol_mm > 0.0 {
        let k_gap = (params.proximity_tol_mm / params.grid_res_mm).ceil() as i64;
        for comp in &composites {
            if !comp.role.is_copper() {
                continue;
            }
            let (lab, _sizes) = comp.mask.label_components_4();
            let w = grid.width;
            let h = grid.height;
            let mut seen: std::collections::BTreeSet<(u32, u32)> = Default::default();
            for r in 0..h {
                for c in 0..w {
                    let l0 = lab[r * w + c];
                    if l0 == 0 {
                        continue;
                    }
                    for dr in -k_gap..=k_gap {
                        for dc in -k_gap..=k_gap {
                            let rr = r as i64 + dr;
                            let cc = c as i64 + dc;
                            if rr < 0 || cc < 0 || rr >= h as i64 || cc >= w as i64 {
                                continue;
                            }
                            let l1 = lab[(rr as usize) * w + cc];
                            if l1 == 0 || l1 == l0 {
                                continue;
                            }
                            let key = if l0 < l1 { (l0, l1) } else { (l1, l0) };
                            if seen.contains(&key) {
                                continue;
                            }
                            // must not be touching (those are tangency/definite already)
                            let touching = (dr.abs() <= 1 && dc.abs() <= 1);
                            let gap_cells = (dr.abs().max(dc.abs()) - 1).max(0) as f64;
                            let gap_mm = gap_cells * params.grid_res_mm;
                            if !touching && gap_mm <= params.proximity_tol_mm + f64::EPSILON {
                                if let Some(pair) = atom_pair(&atoms, comp.stack_index, l0, l1) {
                                    let (ai, bi) = pair;
                                    let ca = comp_of_atom(ai);
                                    let cb = comp_of_atom(bi);
                                    if ca != cb {
                                        seen.insert(key);
                                        let ax = grid.x_of(c);
                                        let ay = grid.y_of(r);
                                        candidates.push(CandidateView {
                                            id: format!("cand:prox:L{}:{}:{}", comp.stack_index, l0, l1),
                                            kind: CandidateKind::Proximity,
                                            from_component: ca,
                                            to_component: cb,
                                            gap_mm,
                                            at: PtView { x: ax, y: ay },
                                            evidence: vec![
                                                format!("layer {} labels {} and {}", comp.stack_index, l0, l1),
                                                format!("rule {} proximity_tol_mm={}", params.version, params.proximity_tol_mm),
                                            ],
                                            status: CandidateStatus::Open,
                                            decided_by_event: None,
                                            rule_version: params.version.clone(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // 9c. point-via alignment: a hole center lands exactly on copper but the drill
    // diameter / plating does not give area overlap (NPTH or unresolved plating).
    for (hi, hole) in holes.iter().enumerate() {
        let definite = matches!(hole.plating, Plating::Pth) && hole_hits[hi].len() >= 2;
        if definite {
            continue;
        }
        let (x, y) = match (hole.x_mm, hole.y_mm) {
            (Some(x), Some(y)) => (x, y),
            _ => continue,
        };
        // exact center cell copper but no neighbour area overlap within drill radius
        let mut layer_atoms: Vec<(u32, u32)> = Vec::new();
        for layer in hole.span_from..=hole.span_to {
            if let Some((lab, _w)) = label_img.get(&layer) {
                let cc = grid.col_of(x);
                let rr = grid.row_of(y);
                if cc >= 0 && rr >= 0 && (cc as usize) < grid.width && (rr as usize) < grid.height {
                    let v = lab[(rr as usize) * grid.width + cc as usize];
                    if v != 0 {
                        layer_atoms.push((layer, v));
                    }
                }
            }
        }
        if layer_atoms.len() >= 2 || !matches!(hole.plating, Plating::Pth) {
            let comps: Vec<String> = layer_atoms
                .iter()
                .filter_map(|(l, la)| {
                    atom_pair_by_label(&atoms, *l, *la).map(|ai| comp_of_atom(ai))
                })
                .collect();
            if comps.len() >= 2 && comps[0] != comps[1] {
                candidates.push(CandidateView {
                    id: format!("cand:pvia:{}", hole.id.trim_start_matches("hole:")),
                    kind: CandidateKind::PointViaAlignment,
                    from_component: comps[0].clone(),
                    to_component: comps[1].clone(),
                    gap_mm: 0.0,
                    at: PtView { x, y },
                    evidence: vec![
                        format!("hole {} plating={:?} span {}-{}", hole.id, hole.plating, hole.span_from, hole.span_to),
                        "center coincides with copper on a point only".into(),
                    ],
                    status: CandidateStatus::Open,
                    decided_by_event: None,
                    rule_version: params.version.clone(),
                });
            }
        }
    }

    candidates.sort_by(|a, b| a.id.cmp(&b.id));
    candidates.dedup_by(|a, b| a.id == b.id);

    // ----- 10. replay human decisions on candidates; conflicting decisions kept -----
    let mut decision_by_cand: std::collections::BTreeMap<String, &DecisionRecord> = Default::default();
    let mut decision_contradictions: Vec<ContradictionView> = Vec::new();
    for d in decisions {
        if let Some(prev) = decision_by_cand.insert(d.candidate_id.clone(), d) {
            if prev.accept != d.accept {
                decision_contradictions.push(ContradictionView {
                    id: format!("contr:dec:{}", d.candidate_id),
                    kind: ContradictionKind::ConflictingCandidateDecision,
                    message: format!(
                        "candidate {} both accepted ({}) and rejected ({}) — both decisions retained, no forced net",
                        d.candidate_id, prev.note, d.note
                    ),
                    at: candidates
                        .iter()
                        .find(|c| c.id == d.candidate_id)
                        .map(|c| c.at.clone())
                        .unwrap_or(PtView { x: 0.0, y: 0.0 }),
                    event_ids: vec![prev.event_id.clone(), d.event_id.clone()],
                    object_ids: vec![d.candidate_id.clone()],
                });
                if let Some(c) = candidates.iter_mut().find(|c| c.id == d.candidate_id) {
                    c.status = CandidateStatus::Contradicted;
                }
            }
        }
    }
    for c in candidates.iter_mut() {
        if c.status == CandidateStatus::Contradicted {
            continue;
        }
        if let Some(d) = decision_by_cand.get(c.id.as_str()) {
            c.status = if d.accept {
                CandidateStatus::Accepted
            } else {
                CandidateStatus::Rejected
            };
            c.decided_by_event = Some(d.event_id.clone());
        }
    }

    // ----- 11. no-netlist ambiguity: enumerate legal candidate combinations -----
    // Each open candidate may be accept/reject; full cross product, capped by the
    // *versioned* ambiguity budget. Exhaustion is a distinct status, not a silent trim.
    let open: Vec<String> = candidates
        .iter()
        .filter(|c| c.status == CandidateStatus::Open)
        .map(|c| c.id.clone())
        .collect();
    let mut ambiguities = Vec::new();
    let mut budget_exhausted = budget_exhausted;
    let combos = 1u64 << open.len().min(63);
    let exhausted = open.len() as u64 >= 2 && combos > params.max_ambiguity_options;
    if open.len() >= 2 {
        let limit = if exhausted {
            params.max_ambiguity_options
        } else {
            combos
        };
        let mut options = Vec::new();
        for mask in 0..limit {
            let mut accepted = Vec::new();
            let mut rejected = Vec::new();
            for (i, id) in open.iter().enumerate() {
                if (mask >> i) & 1 == 1 {
                    accepted.push(id.clone());
                } else {
                    rejected.push(id.clone());
                }
            }
            options.push(AmbiguityOption {
                name: format!("option-{:02}", mask + 1),
                accepted,
                rejected,
                note: "one legal resolution under no-netlist; proximity never implies conduction".into(),
            });
        }
        ambiguities.push(AmbiguityView {
            id: "amb:open-candidates".into(),
            summary: format!(
                "no netlist: {} open candidates yield {} legal resolutions{}",
                open.len(),
                if exhausted { format!("{} (of {combos})", limit) } else { combos.to_string() },
                if exhausted { "; budget reached" } else { "" }
            ),
            candidate_ids: open.clone(),
            options,
            budget_exhausted: exhausted,
        });
        if exhausted {
            budget_exhausted.push(format!("ambiguity-enumeration:{combos}>{}", params.max_ambiguity_options));
        }
    }

    // ----- 12. outcome status & diagnostics -----
    let input_summary = sources
        .iter()
        .map(|s| InputSummaryItem {
            source_id: s.source_id.clone(),
            filename: s.filename.clone(),
            kind: s.kind.clone(),
            sha256: s.sha256.clone(),
            bytes: s.content.len(),
        })
        .collect();

    let mut all_contradictions = role_conflicts;
    all_contradictions.extend(decision_contradictions);

    if !all_contradictions.is_empty() {
        status = OutcomeStatus::SemanticUnresolved;
    }
    if !budget_exhausted.is_empty() {
        status = OutcomeStatus::ResourceBudget;
    }

    let recon = Reconstruction {
        rule_version: params.version.clone(),
        input_summary,
        parent_event_id: parent_event_id.to_string(),
        parent_dependencies: parent_dependencies.to_vec(),
        decision_events: decision_event_ids.to_vec(),
        layers: layer_views,
        holes,
        components,
        candidates,
        ambiguities,
        contradictions: all_contradictions,
        diagnostics,
        extent: (
            PtView { x: min_x, y: min_y },
            PtView { x: max_x, y: max_y },
        ),
        grid_res_mm: params.grid_res_mm,
        budget_exhausted,
        status,
    };
    match recon.status {
        OutcomeStatus::ResourceBudget => ComputeOutcome::BudgetExhausted {
            partial: Box::new(recon),
            reasons: std::mem::take(&mut Vec::new()),
        },
        OutcomeStatus::SemanticUnresolved => ComputeOutcome::SemanticUnresolved(Box::new(recon)),
        _ => ComputeOutcome::Complete(Box::new(recon)),
    }
}
