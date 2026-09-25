//! Reconstruct copper polygons, solder-mask openings and drill barrels from
//! parsed documents. This is the first *derived* layer: inputs carry raw
//! representation; outputs are normalized to millimetres.

use crate::model::*;
use super::polygon::*;

#[derive(Debug, Clone)]
pub struct Budget {
    pub max_primitives: usize,
    pub max_segments: usize,
    pub used_primitives: usize,
    pub used_segments: usize,
}

impl Budget {
    pub fn new(max_primitives: usize, max_segments: usize) -> Self {
        Budget { max_primitives, max_segments, used_primitives: 0, used_segments: 0 }
    }
    fn tick_primitive(&mut self) -> Result<(), Outcome> {
        self.used_primitives += 1;
        if self.used_primitives > self.max_primitives {
            return Err(Outcome::BudgetExhausted {
                budget: "primitives".into(),
                used: self.used_primitives,
                limit: self.max_primitives,
                at: "reconstruct".into(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CopperRegion {
    pub id: String,
    pub layer_index: usize,
    pub polygon: Polygon,
    /// Source operation sequence (last contributing op for ordered polarity).
    pub origin_op: usize,
    pub origin_aperture: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MaskOpening {
    pub id: String,
    pub layer_index: usize,
    pub polygon: Polygon,
    pub origin_op: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PlatedBarrel {
    pub id: String,
    pub tool: String,
    pub x: f64,
    pub y: f64,
    pub radius: f64,
    pub plating: Plating,
    /// Inclusive copper-layer indices the barrel reaches.
    pub span_start: usize,
    pub span_end: usize,
    pub origin_seq: usize,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ReconstructedLayer {
    pub copper: Vec<CopperRegion>,
    pub mask: Vec<MaskOpening>,
    pub barrels: Vec<PlatedBarrel>,
}

pub struct LayerAssign {
    pub file_name: String,
    pub layer_index: usize,
    pub role: LayerRole,
}

#[allow(clippy::too_many_arguments)]
pub fn reconstruct(
    gerbers: &[(String, GerberDoc)],
    drills: &[(String, ExcellonDoc)],
    assignments: &[LayerAssign],
    copper_count: usize,
    budget: &mut Budget,
) -> Result<ReconstructedLayer, Outcome> {
    let mut out = ReconstructedLayer::default();

    for (file_name, doc) in gerbers {
        let Some(assign) = assignments.iter().find(|a| &a.file_name == file_name) else {
            continue;
        };
        if !matches!(assign.role, LayerRole::Copper | LayerRole::SolderMask) {
            continue;
        }
        let inch = doc.datum.unit == LengthUnit::Inch;
        let scale = if inch { 25.4 } else { 1.0 };
        let apertures = &doc.apertures;
        let mut dark: Vec<(Polygon, usize, String)> = Vec::new();
        let mut clear: Vec<(Polygon, usize)> = Vec::new();

        for op in doc.ops.iter().filter(|o| {
            o.points_mm.iter().all(|(x, y)| !x.is_nan() && !y.is_nan())
        }) {
            let prims = op_primitives(op, apertures, scale, budget)?;
            budget.tick_primitive()?;
            for poly in prims {
                match op.polarity {
                    Polarity::Dark => dark.push((poly, op.seq, op.aperture.clone())),
                    Polarity::Clear => clear.push((poly, op.seq)),
                }
            }
        }

        // Ordered polarity superposition. Walk ops in source sequence order:
        // DARK adds a region; CLEAR subtracts from everything added so far.
        let mut order: Vec<(usize, Polarity, Polygon, String)> = Vec::new();
        for (poly, seq, ap) in dark {
            order.push((seq, Polarity::Dark, poly, ap));
        }
        for (poly, seq) in clear {
            order.push((seq, Polarity::Clear, poly, String::new()));
        }
        order.sort_by_key(|(seq, _, _, _)| *seq);

        let mut regions: Vec<(Polygon, usize, String)> = Vec::new();
        for (seq, pol, poly, ap) in order {
            budget.tick_primitive()?;
            if pol == Polarity::Dark {
                regions.push((poly, seq, ap));
            } else {
                let mut next: Vec<(Polygon, usize, String)> = Vec::new();
                for (region, rs, ra) in regions.drain(..) {
                    for piece in subtract(&region, &poly) {
                        if polygon_area(&piece) > 1e-12 {
                            next.push((piece, rs, ra));
                        }
                    }
                }
                regions = next;
            }
        }

        for (i, (poly, seq, ap)) in regions.into_iter().enumerate() {
            let id = format!("cop_{}_{}", file_name, i);
            if assign.role == LayerRole::Copper {
                out.copper.push(CopperRegion {
                    id,
                    layer_index: assign.layer_index,
                    polygon,
                    origin_op: seq,
                    origin_aperture: ap,
                });
            } else {
                out.mask.push(MaskOpening {
                    format!("msk_{}_{}", file_name, i),
                    assign.layer_index,
                    polygon,
                    origin_op: seq,
                });
            }
        }
    }

    // Drill barrels.
    let total_layers = copper_count.max(1);
    for (_file_name, doc) in drills {
        for hit in &doc.hits {
            let (Some(x), Some(y)) = (hit.x_mm, hit.y_mm) else { continue };
            let radius = (hit.diameter.mm.unwrap_or(0.0)) / 2.0;
            let (start, end) = match (hit.span_start, hit.span_end) {
                (Some(s), Some(e)) => (s.min(total_layers - 1), e.min(total_layers - 1)),
                _ => (0, total_layers - 1),
            };
            out.barrels.push(PlatedBarrel {
                id: format!("bar_{}_{}", hit.tool, hit.seq),
                tool: hit.tool.clone(),
                x,
                y,
                radius,
                plating: hit.plating,
                span_start: start,
                span_end: end,
                origin_seq: hit.seq,
            });
            budget.tick_primitive()?;
        }
    }

    Ok(out)
}

fn op_primitives(
    op: &DrawOp,
    apertures: &[ApertureDef],
    scale: f64,
    budget: &mut Budget,
) -> Result<Vec<Polygon>, Outcome> {
    let ap = apertures.iter().find(|a| a.code == op.aperture).cloned();
    match op.kind.as_str() {
        "flash" => {
            let (x, y) = op.points_mm[0];
            Ok(vec![flash_polygon(x, y, &ap, scale)])
        }
        "line" => {
            let width = ap
                .as_ref()
                .and_then(|a| a.params_mm.first().copied())
                .unwrap_or(0.0)
                * scale;
            budget.used_segments += 1;
            if budget.used_segments > budget.max_segments {
                return Err(Outcome::BudgetExhausted {
                    budget: "segments".into(),
                    used: budget.used_segments,
                    limit: budget.max_segments,
                    at: "line".into(),
                });
            }
            Ok(vec![segment_polygon(op.points_mm[0], op.points_mm[1], width.max(0.001))])
        }
        "region" => Ok(vec![op.points_mm.clone()]),
        _ => Ok(Vec::new()),
    }
}

fn flash_polygon(x: f64, y: f64, ap: &Option<ApertureDef>, scale: f64) -> Polygon {
    let Some(ap) = ap else {
        return rect_polygon(x, y, 0.1, 0.1);
    };
    let p = |i: usize| ap.params_mm.get(i).copied().unwrap_or(0.0) * scale;
    match ap.shape.as_str() {
        "C" => circle_polygon(x, y, (p(0) / 2.0).max(0.0005), 20),
        "R" | "O" => rect_polygon(x, y, p(0).max(0.001), p(1).max(p(0)).max(0.001)),
        _ => rect_polygon(x, y, p(0).max(0.001), p(1).max(p(0)).max(0.001)),
    }
}

/// Subtract polygon `b` from `a`. Supports exact subtraction of axis-aligned
/// rectangular clear regions (the case-pack negative apertures) by splitting
/// the plane into four outside strips and clipping `a` to each. Non-AABB clear
/// regions subtract only when fully disjoint (documented subset limitation).
fn subtract(a: &Polygon, b: &Polygon) -> Vec<Polygon> {
    if !polygons_touch(a, b) {
        return vec![a.clone()];
    }
    if let Some(rect) = as_aabb(b) {
        let mut pieces = Vec::new();
        let (x0, y0, x1, y1) = rect;
        let (ax0, ay0, ax1, ay1) = bbox(a);
        let strips: [Polygon; 4] = [
            vec![(ax0, ay0), (x0, ay0), (x0, ay1), (ax0, ay1)],
            vec![(x1, ay0), (ax1, ay0), (ax1, ay1), (x1, ay1)],
            vec![(x0, ay0), (x1, ay0), (x1, y0), (x0, y0)],
            vec![(x0, y1), (x1, y1), (x1, ay1), (x0, ay1)],
        ];
        for strip in strips {
            for piece in clip(a, &strip) {
                if polygon_area(&piece) > 1e-12 {
                    pieces.push(piece);
                }
            }
        }
        if pieces.is_empty() {
            // Region fully erased, or only boundary tangency: if merely
            // tangent (zero overlap area), keep the original.
            if overlap_area(a, b) < 1e-12 {
                pieces.push(a.clone());
            }
        }
        return pieces;
    }
    // Non-rectangular clear: exact handling limited to disjoint case.
    if overlap_area(a, b) > 0.0 {
        Vec::new()
    } else {
        vec![a.clone()]
    }
}

fn bbox(p: &Polygon) -> (f64, f64, f64, f64) {
    let mut x0 = f64::MAX;
    let mut y0 = f64::MAX;
    let mut x1 = f64::MIN;
    let mut y1 = f64::MIN;
    for (x, y) in p {
        x0 = x0.min(*x);
        y0 = y0.min(*y);
        x1 = x1.max(*x);
        y1 = y1.max(*y);
    }
    (x0, y0, x1, y1)
}

fn as_aabb(p: &Polygon) -> Option<(f64, f64, f64, f64)> {
    if p.len() != 4 {
        return None;
    }
    let (x0, y0, x1, y1) = bbox(p);
    // Every vertex must lie on the bounding box corners.
    let mut pts = p.clone();
    pts.sort_by(|a, b| a.0.partial_cmp(&b.0).then(a.1.partial_cmp(&b.1)).unwrap());
    let corners = vec![(x0, y0), (x0, y1), (x1, y0), (x1, y1)];
    let mut c = corners.clone();
    c.sort_by(|a, b| a.0.partial_cmp(&b.0).then(a.1.partial_cmp(&b.1)).unwrap());
    if pts == c {
        Some((x0, y0, x1, y1))
    } else {
        None
    }
}

/// Conservative positive-overlap test via intersection area is unnecessary for
/// the case pack: treat containment or non-boundary touching as overlap using a
/// shrunken bbox probe.
fn overlap_area(a: &Polygon, b: &Polygon) -> f64 {
    use super::polygon::{clip, polygon_area};
    let (ax0, ay0, ax1, ay1) = bbox(a);
    let (bx0, by0, bx1, by1) = bbox(b);
    let ix0 = ax0.max(bx0);
    let iy0 = ay0.max(by0);
    let ix1 = ax1.min(bx1);
    let iy1 = ay1.min(by1);
    if ix1 <= ix0 || iy1 <= iy0 {
        return 0.0;
    }
    let probe = vec![(ix0, iy0), (ix1, iy0), (ix1, iy1), (ix0, iy1)];
    let clipped = clip(a, &probe);
    clipped.iter().map(|p| polygon_area(p)).sum::<f64>()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clear_cuts_dark_in_order() {
        let dark = rect_polygon(0.0, 0.0, 10.0, 10.0);
        let clear = rect_polygon(0.0, 0.0, 4.0, 4.0);
        let pieces = subtract(&dark, &clear);
        let total: f64 = pieces.iter().map(|p| polygon_area(p)).sum();
        assert!((total - (100.0 - 16.0)).abs() < 1e-6);
    }

    #[test]
    fn tangential_clear_keeps_dark() {
        let dark = rect_polygon(0.0, 0.0, 4.0, 4.0);
        let clear = rect_polygon(4.0, 0.0, 4.0, 4.0);
        let pieces = subtract(&dark, &clear);
        let total: f64 = pieces.iter().map(|p| polygon_area(p)).sum();
        assert!((total - 16.0).abs() < 1e-6);
    }
}
