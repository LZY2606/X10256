//! A deliberately small but faithful Gerber/X2 subset parser.
//! It preserves coordinate format, unit, polarity, apertures, step-repeat,
//! layer role (file attributes) and never drops unconvertible values.

use crate::model::*;
use crate::parse::decode_coord;

pub fn parse(text: &str) -> GerberDoc {
    let mut doc = GerberDoc {
        format: String::new(),
        coordinate_format: String::new(),
        zero_suppression: String::new(),
        unit: None,
        unit_command: None,
        apertures: Vec::new(),
        ops: Vec::new(),
        step_repeats: Vec::new(),
        file_attributes: Vec::new(),
        object_attributes: Vec::new(),
        diagnostics: Vec::new(),
        byte_len: text.len() as u64,
    };

    let mut int_digits: usize = 2;
    let mut dec_digits: usize = 5;
    let mut zero = 'L';
    let mut current_ap: Option<String> = None;
    let mut polarity = Polarity::Dark;
    let mut current_sr: Option<String> = None;
    let mut last_point: Option<CoordPoint> = None;
    let mut in_region = false;
    let mut region_rings: Vec<Vec<(f64, f64)>> = Vec::new();
    let mut region_active_ring: Vec<(f64, f64)> = Vec::new();
    let mut region_pending_ring: Vec<(f64, f64)> = Vec::new();
    let mut last_object_attr: Option<(String, String)> = None;
    let mut terminator_seen = false;

    for (idx, raw_line) in text.lines().enumerate() {
        let line = (idx + 1) as u32;
        let l = raw_line.trim();
        if l.is_empty() {
            continue;
        }

        if l.starts_with("G04") {
            // Full-line comment.
            continue;
        }

        // Extended command block: %...*%
        if l.starts_with('%') {
            let body = l.trim_start_matches('%').trim_end_matches('%').trim_end_matches('*');
            handle_extended(
                body,
                line,
                &mut doc,
                &mut int_digits,
                &mut dec_digits,
                &mut zero,
                &mut polarity,
                &mut current_sr,
                &mut last_object_attr,
            );
            continue;
        }

        // G codes and graphics on one or more lines.
        for chunk in l.split('*') {
            let chunk = chunk.trim();
            if chunk.is_empty() {
                continue;
            }
            if chunk == "M02" || chunk == "M00" {
                terminator_seen = true;
                continue;
            }
            if chunk.starts_with('G') {
                let code = chunk.get(0..3).unwrap_or(chunk);
                match code {
                    "G01" | "G02" | "G03" => {
                        if code != "G01" {
                            doc.diagnostics.push(Diagnostic::warn(
                                "GERBER_ARC_UNSUPPORTED",
                                Some(line),
                                Some(chunk.to_string()),
                                "arc interpolation is retained as a diagnostic; linear cases only",
                            ));
                        }
                    }
                    "G36" => {
                        in_region = true;
                        region_rings.clear();
                        region_active_ring.clear();
                        region_pending_ring.clear();
                    }
                    "G37" => {
                        if !region_pending_ring.is_empty() {
                            region_rings.push(std::mem::take(&mut region_pending_ring));
                        } else if !region_active_ring.is_empty() {
                            region_rings.push(std::mem::take(&mut region_active_ring));
                        }
                        if in_region {
                            doc.ops.push(GerberOp {
                                shape: GerberShape::RegionPoint,
                                aperture: None,
                                polarity,
                                step_repeat: current_sr.clone(),
                                from: last_point.clone().unwrap_or(empty_point(line)),
                                to: last_point.clone().unwrap_or(empty_point(line)),
                                rings: Some(std::mem::take(&mut region_rings)),
                                line,
                            });
                        }
                        in_region = false;
                    }
                    _ => {
                        // may be a graphics chunk after G01, e.g. G01X..
                    }
                }
                // G01 can be prefix on a graphics chunk.
                let rest = if chunk.len() > 3 { &chunk[3..] } else { "" };
                if !rest.is_empty() && rest.contains('X') || (!rest.is_empty() && rest.contains('Y')) {
                    let (pt, mut ds) = decode_coord(
                        rest, int_digits, dec_digits, zero, doc.unit.unwrap_or(LengthUnit::Inch), line,
                    );
                    doc.diagnostics.append(&mut ds);
                    graphics(
                        &pt,
                        chunk,
                        line,
                        &mut doc,
                        &mut current_ap,
                        polarity,
                        &current_sr,
                        &mut last_point,
                        in_region,
                        &mut region_rings,
                        &mut region_active_ring,
                        &mut region_pending_ring,
                    );
                }
                continue;
            }
            if chunk.starts_with('D') && !chunk.contains('X') && !chunk.contains('Y') {
                if let Some(num) = chunk.trim_start_matches('D').strip_suffix("").and_then(|s| s.parse::<u32>().ok()) {
                    if num >= 10 {
                        current_ap = Some(chunk.to_string());
                    }
                }
                continue;
            }
            if chunk.contains('X') || chunk.contains('Y') {
                let (pt, mut ds) = decode_coord(
                    chunk, int_digits, dec_digits, zero, doc.unit.unwrap_or(LengthUnit::Inch), line,
                );
                doc.diagnostics.append(&mut ds);
                graphics(
                    &pt,
                    chunk,
                    line,
                    &mut doc,
                    &mut current_ap,
                    polarity,
                    &mut current_sr,
                    &mut last_point,
                    in_region,
                    &mut region_rings,
                    &mut region_active_ring,
                    &mut region_pending_ring,
                );
            } else if chunk.starts_with('D') {
                // D02 etc handled via graphics terminator above
            } else {
                doc.diagnostics.push(Diagnostic::warn(
                    "GERBER_UNKNOWN_STATEMENT",
                    Some(line),
                    Some(chunk.to_string()),
                    "statement not understood; raw line retained",
                ));
            }
        }
    }

    if !terminator_seen {
        doc.diagnostics.push(Diagnostic::error(
            "GERBER_NO_M02",
            None,
            None,
            "Gerber stream has no M02 terminator; treated as corrupt",
        ));
    }
    if doc.unit.is_none() {
        doc.diagnostics.push(Diagnostic::error(
            "GERBER_NO_UNIT",
            None,
            None,
            "no %MOIN*% / %MOMM*% unit command; coordinates cannot be normalised",
        ));
    }
    if doc.format.is_empty() {
        doc.diagnostics.push(Diagnostic::error(
            "GERBER_NO_FS",
            None,
            None,
            "no %FSLAX...*% coordinate format; raw coordinates retained",
        ));
    }
    doc
}

fn empty_point(line: u32) -> CoordPoint {
    CoordPoint { x_raw: None, y_raw: None, x_mm: None, y_mm: None, line }
}

#[allow(clippy::too_many_arguments)]
fn graphics(
    pt: &CoordPoint,
    chunk: &str,
    line: u32,
    doc: &mut GerberDoc,
    current_ap: &mut Option<String>,
    polarity: Polarity,
    current_sr: &Option<String>,
    last_point: &mut Option<CoordPoint>,
    in_region: bool,
    region_rings: &mut Vec<Vec<(f64, f64)>>,
    region_active_ring: &mut Vec<(f64, f64)>,
    region_pending_ring: &mut Vec<(f64, f64)>>,
) {
    let dterm: Option<u32> = if let Some(pos) = chunk.rfind('D') {
        chunk[pos + 1..].chars().take(2).collect::<String>().parse().ok()
    } else {
        None
    };
    let draw = dterm != Some(2);
    let flash = dterm == Some(3);

    if in_region {
        if let (Some(x), Some(y)) = (pt.x_mm, pt.y_mm) {
            if !draw {
                // pen up / D02: start a fresh ring
                if !region_pending_ring.is_empty() {
                    region_rings.push(std::mem::take(region_pending_ring));
                } else if !region_active_ring.is_empty() {
                    region_rings.push(std::mem::take(region_active_ring));
                }
                region_pending_ring.push((x, y));
            } else if region_pending_ring.is_empty() && region_active_ring.is_empty() && region_rings.is_empty() {
                region_pending_ring.push((x, y));
            } else if region_pending_ring.is_empty() {
                region_active_ring.push((x, y));
            } else {
                region_pending_ring.push((x, y));
            }
        }
        *last_point = Some(pt.clone());
        return;
    }

    let from = last_point.clone().unwrap_or_else(|| empty_point(line));
    let shape = if flash { GerberShape::Flash } else { GerberShape::Segment };
    doc.ops.push(GerberOp {
        shape,
        aperture: current_ap.clone(),
        polarity,
        step_repeat: current_sr.clone(),
        from: from.clone(),
        to: pt.clone(),
        rings: None,
        line,
    });
    if !draw {
        // D02 move: update current point but no draw (already recorded nothing meaningful)
    }
    *last_point = Some(pt.clone());
}
