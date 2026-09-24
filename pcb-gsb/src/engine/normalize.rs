//! Normalise source documents into canonical-mm primitives.
//!
//! This is the ONLY place source-unit numbers are converted. Sources stay
//! untouched; every produced [`Prim`] carries its material id and feature ref.

use super::geom::{Prim, Shape};
use crate::excellon::ExcellonDoc;
use crate::gerber::{Aperture, ApertureGeom, FeatureShape, GerberDoc};
use crate::material::Material;
use crate::model::{Diagnostic, LengthUnit};
use std::collections::HashMap;

pub struct NormalizedLayer {
    /// `material_id` + logical layer name, assigned by the assembler.
    pub layer_key: String,
    pub material_id: String,
    pub prims: Vec<Prim>,
}

pub fn normalize_gerber(doc: &GerberDoc, material_id: &str, layer_key: &str) -> (NormalizedLayer, Vec<Diagnostic>) {
    let mut diags = Vec::new();
    let unit = doc.unit.unwrap_or(LengthUnit::Mm);
    let k = unit.to_mm();
    let aps: HashMap<&str, &Aperture> = doc.apertures.iter().map(|a| (a.code.as_str(), a)).collect();
    let mut prims = Vec::new();
    for f in &doc.features {
        let ap_code = match &f.shape {
            FeatureShape::Flash { aperture, .. } | FeatureShape::Draw { aperture, .. } => Some(aperture.as_str()),
            FeatureShape::RegionRect { .. } => None,
        };
        let shape: Option<Shape> = match &f.shape {
            FeatureShape::Flash { aperture, x_raw, y_raw } => {
                let ap = aps.get(ap_code.as_str());
                match ap {
                    Some(ap) => Some(flash_shape(ap, x_raw * k, y_raw * k)),
                    None => {
                        diags.push(Diagnostic::err(
                            "ENGINE_MISSING_APERTURE",
                            format!("aperture `{aperture}` referenced but never defined"),
                        ));
                        None
                    }
                }
            }
            FeatureShape::Draw { aperture, x1_raw, y1_raw, x2_raw, y2_raw } => {
                let ap = aps.get(ap_code.as_deref().unwrap_or(aperture));
                match ap {
                    Some(ap) => Some(draw_shape(ap, x1_raw * k, y1_raw * k, x2_raw * k, y2_raw * k)),
                    None => {
                        diags.push(Diagnostic::err(
                            "ENGINE_MISSING_APERTURE",
                            format!("aperture `{aperture}` referenced but never defined"),
                        ));
                        None
                    }
                }
            }
            FeatureShape::RegionRect { x_raw, y_raw, w_raw, h_raw } => Some(Shape::Rect {
                x: x_raw * k,
                y: y_raw * k,
                w: w_raw * k,
                h: h_raw * k,
            }),
        };
        if let Some(shape) = shape {
            prims.push(Prim {
                id: format!("{material_id}:f{}:{}", f.order, f.sr_instance),
                shape,
                polarity: f.polarity,
                order: f.order,
                feature_ref: format!("f{}:{}", f.order, f.sr_instance),
                material_id: material_id.to_string(),
                layer_key: layer_key.to_string(),
            });
        }
    }
    (
        NormalizedLayer {
            layer_key: layer_key.to_string(),
            material_id: material_id.to_string(),
            prims,
        },
        diags,
    )
}

fn flash_shape(ap: &Aperture, cx: f64, cy: f64) -> Shape {
    match &ap.geom {
        ApertureGeom::Circle { diameter_raw } => Shape::Circle {
            cx,
            cy,
            r: diameter_raw * ap_unit_factor(ap) / 2.0,
        },
        ApertureGeom::Rect { x_size_raw, y_size_raw } => {
            let k = ap_unit_factor(ap);
            let w = x_size_raw * k;
            let h = y_size_raw * k;
            Shape::Rect {
                x: cx - w / 2.0,
                y: cy - h / 2.0,
                w,
                h,
            }
        }
        ApertureGeom::Obround { x_size_raw, y_size_raw } => {
            let k = ap_unit_factor(ap);
            Shape::Capsule {
                x1: cx - x_size_raw * k / 2.0,
                y1: cy,
                x2: cx + x_size_raw * k / 2.0,
                y2: cy,
                w: x_size_raw * k,
                h: y_size_raw * k,
            }
        }
    }
}

fn draw_shape(ap: &Aperture, x1: f64, y1: f64, x2: f64, y2: f64) -> Shape {
    match &ap.geom {
        ApertureGeom::Circle { diameter_raw } => {
            let r = diameter_raw * ap_unit_factor(ap) / 2.0;
            if (x1 - x2).abs() < 1e-12 && (y1 - y2).abs() < 1e-12 {
                return Shape::Circle { cx: x1, cy: y1, r };
            }
            if (y1 - y2).abs() < 1e-9 {
                let xmin = x1.min(x2);
                let xmax = x1.max(x2);
                return Shape::Capsule {
                    x1: xmin,
                    y1,
                    x2: xmax,
                    y2: y1,
                    w: (xmax - xmin) + 2.0 * r,
                    h: 2.0 * r,
                };
            }
            if (x1 - x2).abs() < 1e-9 {
                let ymin = y1.min(y2);
                let ymax = y1.max(y2);
                return Shape::Capsule {
                    x1,
                    y1: ymin,
                    x2: x1,
                    y2: ymax,
                    w: 2.0 * r,
                    h: (ymax - ymin) + 2.0 * r,
                };
            }
            // diagonal thin draw: approximate with end circles + polygon corridor
            let n = 8;
            let mut pts = Vec::with_capacity(n * 2);
            for i in 0..=n {
                let t = i as f64 / n as f64;
                let x = x1 + (x2 - x1) * t;
                let y = y1 + (y2 - y1) * t;
                pts.push((x + r, y));
            }
            for i in (0..=n).rev() {
                let t = i as f64 / n as f64;
                let x = x1 + (x2 - x1) * t;
                let y = y1 + (y2 - y1) * t;
                pts.push((x - r, y));
            }
            Shape::Polygon { points: pts }
        }
        ApertureGeom::Rect { x_size_raw, y_size_raw } | ApertureGeom::Obround { x_size_raw, y_size_raw } => {
            let k = ap_unit_factor(ap);
            let w = x_size_raw * k;
            let h = y_size_raw * k;
            let xmin = x1.min(x2);
            let xmax = x1.max(x2);
            let ymin = y1.min(y2);
            let ymax = y1.max(y2);
            let x = if (x1 - x2).abs() < 1e-9 { x1 - w / 2.0 } else { xmin };
            let y = if (y1 - y2).abs() < 1e-9 { y1 - h / 2.0 } else { ymin };
            Shape::Rect {
                x,
                y,
                w: if (x1 - x2).abs() < 1e-9 { w } else { (xmax - xmin) },
                h: if (y1 - y2).abs() < 1e-9 { h } else { (ymax - ymin) },
            }
        }
    }
}

/// Aperture dimensions share the Gerber file unit.
fn ap_unit_factor(_ap: &Aperture) -> f64 {
    1.0
}

pub struct NormalizedHole {
    pub id: String,
    pub tool: String,
    pub x_mm: f64,
    pub y_mm: f64,
    pub r_mm: f64,
    pub plated: bool,
    /// 1-based inclusive stackup range.
    pub from_layer: usize,
    pub to_layer: usize,
    pub invalid: bool,
    pub invalid_reason: Option<String>,
}

pub fn normalize_holes(doc: &ExcellonDoc, material_id: &str) -> (Vec<NormalizedHole>, Vec<Diagnostic>) {
    let mut diags = Vec::new();
    let k = doc.unit.map(|u| u.to_mm()).unwrap_or(1.0);
    let mut holes = Vec::new();
    for (i, hit) in doc.hits.iter().enumerate() {
        let tool = doc.tools.iter().find(|t| t.code == hit.tool);
        let (r_mm, plated, range) = match tool {
            Some(t) => (
                t.diameter_raw * k / 2.0,
                t.plated,
                t.layer_range
                    .clone()
                    .or_else(|| doc.default_layer_range.clone()),
            ),
            None => {
                diags.push(Diagnostic::warn(
                    "ENGINE_DRILL_NO_TOOL",
                    format!("hit #{} references undefined tool {}", i + 1, hit.tool),
                ));
                (0.0, false, doc.default_layer_range.clone())
            }
        };
        let (from, to) = match range {
            Some(r) => (r.from, r.to),
            None => (1usize, usize::MAX),
        };
        let (x, y, invalid, reason) = match (hit.x_mm, hit.y_mm) {
            (Some(x), Some(y)) => (x, y, false, None),
            _ => (
                hit.x_int.unwrap_or(0) as f64,
                hit.y_int.unwrap_or(0) as f64,
                true,
                hit.invalid_reason
                    .clone()
                    .or_else(|| Some("drill coordinate could not be converted".to_string())),
            ),
        };
        holes.push(NormalizedHole {
            id: format!("{material_id}:h{i}"),
            tool: hit.tool.clone(),
            x_mm: x,
            y_mm: y,
            r_mm,
            plated,
            from_layer: from,
            to_layer: to,
            invalid,
            invalid_reason: reason,
        });
    }
    (holes, diags)
}

/// Files imported but structurally unusable are surfaced, never silently dropped.
pub fn unusable_materials(materials: &[Material]) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for m in materials {
        if let crate::material::ParsedDoc::Unparseable { reason } = &m.parsed {
            out.push(
                Diagnostic::err("IMPORT_UNPARSEABLE", format!("{}: {reason}", m.file_name))
                    .at(crate::model::SrcPos {
                        file_id: m.id.clone(),
                        line: 0,
                        col: 0,
                        byte_start: 0,
                        byte_end: 0,
                        snippet: m.file_name.clone(),
                    }),
            );
        }
    }
    out
}
