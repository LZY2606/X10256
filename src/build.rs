//! Assemble parsed materials into the normalized [`BoardModel`].

use crate::domain::*;
use crate::geometry::Shape;
use crate::model::*;
use crate::stackup::{self, Manifest};
use std::collections::{BTreeMap, BTreeSet};

pub struct AssemblyInput<'a> {
    pub materials: &'a [ParsedMaterial],
    pub mapping: &'a LayerMapping,
}

pub fn assemble(input: AssemblyInput) -> BoardModel {
    let mut model = BoardModel::default();
    let mut by_file: BTreeMap<&str, &ParsedMaterial> = BTreeMap::new();
    for m in input.materials {
        by_file.insert(m.filename.as_str(), m);
    }

    // 1. Find the newest stackup manifest and retain its declarations.
    let manifest = input.materials.iter().rev().find_map(|m| {
        m.stackup_raw.as_ref().map(|raw| (m.id.clone(), raw.clone(), stackup::parse(raw)))
    });

    let mut layers: Vec<ModelLayer> = Vec::new();
    let mut declared_names: BTreeSet<String> = BTreeSet::new();
    if let Some((_, _, stk)) = &manifest {
        model.diags.extend(stk.diags.clone());
        for l in &stk.layers {
            declared_names.insert(l.name.clone());
            layers.push(ModelLayer {
                layer: l.name.clone(),
                role: l.role.clone(),
                z_order: l.z_order,
                material_id: String::new(),
            });
        }
    }

    // 2. Resolve file -> layer mapping (manual mapping overrides manifest).
    let mut file_layer: BTreeMap<String, (String, Option<LayerRole>)> = BTreeMap::new();
    if let Some((_, _, stk)) = &manifest {
        for mp in &stk.mappings {
            file_layer.insert(mp.file.clone(), (mp.layer.clone(), mp.role_override.clone()));
        }
    }
    for (file, layer) in &input.mapping.file_to_layer {
        let role = input.mapping.role_revision.get(file).cloned();
        file_layer.insert(file.clone(), (layer.clone(), role));
    }

    // 3. Register layers actually mapped, applying role revisions.
    for (file, (layer, role_override)) in &file_layer {
        let mat = by_file.get(file.as_str());
        let mat_id = mat.map(|m| m.id.clone()).unwrap_or_default();
        if mat.is_none() {
            model.diags.push(Diag::error(
                "BUILD_MISSING_FILE",
                format!("mapping references missing file {file}"),
                None,
            ));
        }
        let base_role = role_override
            .clone()
            .or_else(|| layers.iter().find(|l| &l.layer == layer).map(|l| l.role.clone()));
        let role = match base_role {
            Some(r) => r,
            None => infer_role(file, mat.cloned()),
        };
        if let Some(existing) = layers.iter_mut().find(|l| &l.layer == layer) {
            existing.material_id = mat_id;
            if role_override.is_some() {
                existing.role = role;
            }
        } else {
            let z = layers.len() as u32 + 1;
            layers.push(ModelLayer { layer: layer.clone(), role, z_order: z, material_id: mat_id });
        }
    }

    // 4. Gerber shapes (paint ops in strict source order; polarity preserved).
    for (file, (layer, _)) in &file_layer {
        let Some(mat) = by_file.get(file.as_str()) else { continue };
        let Some(doc) = &mat.gerber else { continue };
        let Some(ml) = layers.iter().find(|l| &l.layer == layer) else { continue };
        model.diags.extend(doc.diags.clone());
        if doc.unit == Unit::Unspecified {
            model
                .diags
                .push(Diag::error("BUILD_GERBER_NO_UNIT", format!("{file} has no declared unit"), None));
        }
        let apertures: BTreeMap<u32, _> = doc.apertures.iter().map(|a| (a.code, a)).collect();
        for op in &doc.ops {
            let Some(ap) = apertures.get(&op.aperture) else {
                model.diags.push(Diag::error(
                    "BUILD_UNDEFINED_APERTURE",
                    format!("aperture D{} used but never defined", op.aperture),
                    Some(op.pos),
                ));
                continue;
            };
            for shape in op_to_shapes(op, ap, doc, &mut model.diags) {
                if let Some(s) = shape {
                    model.shapes.push(ModelShape {
                        id: format!("{}#op{}#sr{}-{}", mat.id, op.seq, op.sr.ix, op.sr.iy),
                        layer: layer.clone(),
                        dark: matches!(op.polar, crate::gerber::Polar::Dark),
                        shape: s,
                        material_id: mat.id.clone(),
                        op_seq: op.seq,
                        pos: Some(op.pos),
                    });
                }
            }
            let _ = ml;
        }
        // mask openings are the dark copper-side openings on mask roles
        if ml.role.is_mask() {
            for ms in &model.shapes {
                if ms.material_id == mat.id {
                    model.mask_openings.push(MaskOpening {
                        layer: layer.clone(),
                        shape: ms.shape.clone(),
                        material_id: mat.id.clone(),
                    });
                }
            }
        }
    }

    // 5. Holes and layer spans (blind/buried validation).
    let max_z = layers.iter().map(|l| l.z_order).max().unwrap_or(0);
    for (file, (layer, _)) in &file_layer {
        let Some(mat) = by_file.get(file.as_str()) else { continue };
        let Some(doc) = &mat.drill else { continue };
        model.diags.extend(doc.diags.clone());
        let ml = layers.iter().find(|l| &l.layer == layer);
        let role_is_drill = ml.map(|l| matches!(l.role, LayerRole::Drill)).unwrap_or(false);
        if !role_is_drill {
            model.diags.push(Diag::warning(
                "BUILD_DRILL_ROLE",
                format!("{file} is a drill file mapped to non-drill layer {layer}"),
                None,
            ));
        }
        let (from, to) = match (input.mapping.spans.get(file), manifest_span(&manifest, file)) {
            (Some(s), _) => (s.from, s.to),
            (None, Some(s)) => s,
            _ => (1, max_z),
        };
        let through = from == 1 && to == max_z;
        if from < 1 || to > max_z || from > to {
            model
                .diags
                .push(Diag::error("BUILD_BAD_SPAN", format!("{file} span {from}-{to} outside stack 1-{max_z}"), None));
        }
        let plated = input
            .mapping
            .plated_override
            .get(file)
            .copied()
            .or(doc.plated_directive.map(|(p, _)| p))
            .unwrap_or(true);
        let tools: BTreeMap<u32, _> = doc.tools.iter().map(|t| (t.code, t)).collect();
        for hit in &doc.hits {
            let Some((x, y)) = hit.at.x.nano.zip(hit.at.y.nano) else { continue };
            let Some(tool) = tools.get(&hit.tool) else {
                model.diags.push(Diag::error(
                    "BUILD_UNDEFINED_TOOL",
                    format!("hit uses undefined tool T{}", hit.tool),
                    Some(hit.pos),
                ));
                continue;
            };
            let Some(radius) = tool.diameter.nano.map(|d| d / 2) else { continue };
            model.holes.push(ModelHole {
                id: format!("{}#hit{}", mat.id, hit.seq),
                tool: hit.tool,
                x,
                y,
                radius,
                plated,
                span_from: from,
                span_to: to,
                through,
                material_id: mat.id.clone(),
                pos: Some(hit.pos),
            });
        }
    }

    model.layers = layers;
    model.bounds_nm = compute_bounds(&model);
    model
}

fn infer_role(file: &str, mat: Option<&ParsedMaterial>) -> LayerRole {
    let f = file.to_ascii_lowercase();
    if mat.map(|m| m.kind) == Some(MaterialKind::Excellon) || f.contains("drill") || f.contains(".drl") {
        return LayerRole::Drill;
    }
    if f.contains("sm") || f.contains("mask") {
        return if f.contains("bot") || f.contains("bottom") {
            LayerRole::SolderMaskBottom
        } else {
            LayerRole::SolderMaskTop
        };
    }
    if f.contains("bot") || f.contains("bottom") {
        return LayerRole::CopperBottom;
    }
    if f.contains("in") || f.contains("inner") || f.contains("lyr") {
        return LayerRole::Other { name: "unmapped".into() };
    }
    LayerRole::CopperTop
}

fn manifest_span<'a>(manifest: &'a Option<(String, String, Manifest)>, file: &str) -> Option<(u32, u32)> {
    let (_, _, stk) = manifest.as_ref()?;
    let mp = stk.mappings.iter().find(|m| m.file == file)?;
    Some((mp.span_from?, mp.span_to?))
}

fn op_to_shapes(
    op: &crate::gerber::PaintOp,
    ap: &crate::gerber::Aperture,
    _doc: &crate::gerber::GerberDoc,
    diags: &mut Vec<Diag>,
) -> Vec<Option<Shape>> {
    use crate::gerber::PaintKind;
    let out = match &op.op_kind {
        PaintKind::Flash { at } => {
            let Some((x, y)) = at.x.nano.zip(at.y.nano) else { return vec![None] };
            vec![aperture_shape(&ap.shape, x, y, diags, &ap.raw)]
        }
        PaintKind::Segment { from, to } => {
            let Some((ax, ay)) = from.x.nano.zip(from.y.nano) else { return vec![None] };
            let Some((bx, by)) = to.x.nano.zip(to.y.nano) else { return vec![None] };
            match &ap.shape {
                crate::gerber::ApertureShape::Circle { diameter } => {
                    let Some(d) = diameter.nano else { return vec![None] };
                    vec![Some(Shape::Capsule { ax, ay, bx, by, r: d / 2 })]
                }
                crate::gerber::ApertureShape::Rect { width, height } => {
                    let (Some(w), Some(h)) = (width.nano, height.nano) else { return vec![None] };
                    // axis-aligned segment approximated as AABB union of end caps
                    vec![
                        Some(Shape::Rect { x0: ax.min(bx) - w / 2, y0: ay.min(by) - h / 2, x1: ax.max(bx) + w / 2, y1: ay.max(by) + h / 2 }),
                    ]
                }
                crate::gerber::ApertureShape::Obround { width, height } => {
                    let (Some(w), Some(h)) = (width.nano, height.nano) else { return vec![None] };
                    let r = w.min(h) / 2;
                    vec![Some(Shape::Capsule { ax, ay, bx, by, r }), Some(Shape::Rect { x0: ax.min(bx) - w / 2, y0: ay.min(by) - h / 2, x1: ax.max(bx) + w / 2, y1: ay.max(by) + h / 2 })]
                }
                crate::gerber::ApertureShape::Unsupported { raw } => {
                    diags.push(Diag::warning("BUILD_UNSUPPORTED_APERTURE", "aperture shape not rasterized", Some(op.pos)).with_raw(raw));
                    vec![None]
                }
            }
        }
        PaintKind::Outline { .. } => vec![None],
    };
    out
}

fn aperture_shape(shape: &crate::gerber::ApertureShape, x: i64, y: i64, diags: &mut Vec<Diag>, raw: &str) -> Option<Shape> {
    match shape {
        crate::gerber::ApertureShape::Circle { diameter } => {
            diameter.nano.map(|d| Shape::Circle { cx: x, cy: y, r: d / 2 })
        }
        crate::gerber::ApertureShape::Rect { width, height } | crate::gerber::ApertureShape::Obround { width, height } => {
            let (w, h) = (width.nano?, height.nano?);
            Some(Shape::Rect { x0: x - w / 2, y0: y - h / 2, x1: x + w / 2, y1: y + h / 2 })
        }
        crate::gerber::ApertureShape::Unsupported { raw: r } => {
            diags.push(Diag::warning("BUILD_UNSUPPORTED_APERTURE", "aperture shape not rasterized", None).with_raw(r));
            let _ = raw;
            None
        }
    }
}

fn compute_bounds(model: &BoardModel) -> (i64, i64, i64, i64) {
    let mut minx = i64::MAX;
    let mut miny = i64::MAX;
    let mut maxx = i64::MIN;
    let mut maxy = i64::MIN;
    let mut touch = |s: &Shape| {
        let (x0, y0, x1, y1) = s.bbox();
        minx = minx.min(x0);
        miny = miny.min(y0);
        maxx = maxx.max(x1);
        maxy = maxy.max(y1);
    };
    for s in &model.shapes {
        touch(&s.shape);
    }
    for h in &model.holes {
        touch(&Shape::Circle { cx: h.x, cy: h.y, r: h.radius });
    }
    if minx == i64::MAX {
        (0, 0, 0, 0)
    } else {
        (minx, miny, maxx, maxy)
    }
}
