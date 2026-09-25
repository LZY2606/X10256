//! Parser for the RS-274X Gerber subset used by the offline case pack.
//!
//! Every parsed value keeps its raw token. Normalization (to mm) happens on a
//! derived copy; when a value cannot be converted we keep the token and emit a
//! positioned diagnostic rather than guessing.

use crate::model::*;

#[derive(Clone, Copy)]
struct Fmt {
    int: usize,
    dec: usize,
    zero: &'static str, // leading|trailing|none
    inch: bool,
}

pub fn parse(name: &str, text: &str) -> Result<GerberDoc, Vec<Diagnostic>> {
    let mut errs: Vec<Diagnostic> = Vec::new();
    let mut diag = |code: &str, line: usize, col: usize, msg: String, tok: String| {
        errs.push(Diagnostic {
            code: code.into(),
            severity: "error".into(),
            file: name.into(),
            line,
            column: col,
            message: msg,
            token: tok,
        });
    };

    let mut fmt: Option<Fmt> = None;
    let mut apertures: Vec<ApertureDef> = Vec::new();
    let mut ops: Vec<DrawOp> = Vec::new();
    let mut blocks: std::collections::BTreeMap<String, Vec<usize>> = std::collections::BTreeMap::new();
    let mut active_block: Option<String> = None;
    let mut step_repeats: Vec<StepRepeat> = Vec::new();

    let mut cur_ap = "D00".to_string();
    let mut polarity = Polarity::Dark;
    let mut seq = 0usize;

    // Region state.
    let mut in_region = false;
    let mut region_pts_raw: Vec<(String, String)> = Vec::new();
    let mut region_pts_mm: Vec<(f64, f64)> = Vec::new();
    let mut cur = (String::new(), String::new());
    let mut cur_mm: Option<(f64, f64)> = None;
    // Segment state for line draws.
    let mut seg_from_raw: Option<(String, String)> = None;
    let mut seg_from_mm: Option<(f64, f64)> = None;

    for (idx, block) in text.split('*').enumerate() {
        let line = byte_line(text, block);
        let t = block.trim();
        if t.is_empty() {
            continue;
        }
        let extended = t.starts_with('%') && t.ends_with('%');
        let body = if extended {
            t.trim_start_matches('%').trim_end_matches('%').trim()
        } else {
            t
        };
        let col = text[..block.as_ptr() as usize - text.as_ptr() as usize]
            .rfind('\n')
            .map(|n| block.as_ptr() as usize - text.as_ptr() as usize - n)
            .unwrap_or(1);

        // ---- Extended commands ----
        if extended {
            if let Some(rest) = body.strip_prefix("MO") {
                match rest {
                    "IN" | "INCH" => {
                        if let Some(f) = fmt.as_mut() {
                            f.inch = true
                        } else {
                            fmt = Some(Fmt { int: 2, dec: 5, zero: "trailing", inch: true });
                        }
                    }
                    "MM" => {
                        if let Some(f) = fmt.as_mut() {
                            f.inch = false
                        } else {
                            fmt = Some(Fmt { int: 2, dec: 5, zero: "trailing", inch: false });
                        }
                    }
                    other => diag(
                        "G_UNIT_UNKNOWN",
                        line,
                        col,
                        format!("unit '{}' is not convertible; raw value retained", other),
                        body.into(),
                    ),
                }
                continue;
            }
            if let Some(rest) = body.strip_prefix("FS") {
                match parse_fs(rest) {
                    Ok(f) => fmt = Some(f),
                    Err(msg) => diag("G_FORMAT", line, col, msg, body.into()),
                }
                continue;
            }
            if body.starts_with("AD") {
                match parse_ad(body) {
                    Ok(ad) => apertures.push(ad),
                    Err(msg) => diag("G_APERTURE", line, col, msg, body.into()),
                }
                continue;
            }
            if body.starts_with("LP") {
                match body {
                    "LPC" => polarity = Polarity::Clear,
                    "LPD" => polarity = Polarity::Dark,
                    other => diag(
                        "G_POLARITY",
                        line,
                        col,
                        format!("unknown polarity '{}'", other),
                        body.into(),
                    ),
                }
                continue;
            }
            if let Some(rest) = body.strip_prefix("ABD") {
                // %ABD10*% opens block aperture D10.
                let code = format!("D{}", rest.trim_start_matches('D'));
                active_block = Some(code.clone());
                blocks.entry(code).or_default();
                continue;
            }
            if body == "ABE" {
                active_block = None;
                continue;
            }
            if body.starts_with("TF") || body.starts_with("TA") || body.starts_with("TO")
                || body.starts_with("TD") || body == "TF"
            {
                continue; // file/object attributes retained in raw file
            }
            if body == "M02" || body == "M00" {
                continue;
            }
            diag("G_CMD_UNKNOWN", line, col, format!("unsupported extended command"), body.into());
            continue;
        }

        // ---- Regular blocks ----
        let _ = idx;
        if body == "G01" || body == "G1" || body.starts_with("G02") || body.starts_with("G03") {
            // Linear/circular mode setters (G02/G03 treated as straight
            // chords in this subset) are accepted; nothing to record.
            continue;
        }
        if body == "G36" {
            in_region = true;
            region_pts_raw.clear();
            region_pts_mm.clear();
            continue;
        }
        if body == "G37" {
            in_region = false;
            if region_pts_raw.len() >= 3 {
                seq += 1;
                ops.push(DrawOp {
                    seq,
                    aperture: cur_ap.clone(),
                    polarity,
                    kind: "region".into(),
                    points_raw: region_pts_raw.clone(),
                    points_mm: region_pts_mm.clone(),
                    repeat_ref: None,
                });
                if let Some(b) = &active_block {
                    blocks.get_mut(b).unwrap().push(seq);
                }
            }
            continue;
        }

        // Coordinate bearing block: parse X.. Y.. and trailing D code.
        if let Some((raw_xy, mm_xy, dcode)) = parse_coord_block(body, fmt, &mut errs, name, line) {
            if let Some(xy) = raw_xy {
                cur = xy;
                cur_mm = mm_xy;
                if in_region {
                    region_pts_raw.push(cur.clone());
                    region_pts_mm.push(match cur_mm {
                        Some(v) => v,
                        None => (f64::NAN, f64::NAN),
                    });
                    if let Some(b) = &active_block {
                        // Region body recorded at G37 instead.
                        let _ = b;
                    }
                }
                match dcode.as_deref() {
                    Some("D03") => {
                        seq += 1;
                        ops.push(DrawOp {
                            seq,
                            aperture: cur_ap.clone(),
                            polarity,
                            kind: "flash".into(),
                            points_raw: vec![cur.clone()],
                            points_mm: vec![cur_mm.unwrap_or((f64::NAN, f64::NAN))],
                            repeat_ref: None,
                        });
                        if let Some(b) = &active_block {
                            blocks.get_mut(b).unwrap().push(seq);
                        }
                    }
                    Some("D02") => {
                        seg_from_raw = Some(cur.clone());
                        seg_from_mm = cur_mm;
                    }
                    Some("D01") => {
                        if let Some(from) = seg_from_raw.clone() {
                            seq += 1;
                            ops.push(DrawOp {
                                seq,
                                aperture: cur_ap.clone(),
                                polarity,
                                kind: "line".into(),
                                points_raw: vec![from, cur.clone()],
                                points_mm: vec![
                                    seg_from_mm.unwrap_or((f64::NAN, f64::NAN)),
                                    cur_mm.unwrap_or((f64::NAN, f64::NAN)),
                                ],
                                repeat_ref: None,
                            });
                            if let Some(b) = &active_block {
                                blocks.get_mut(b).unwrap().push(seq);
                            }
                        }
                        seg_from_raw = Some(cur.clone());
                        seg_from_mm = cur_mm;
                    }
                    _ => {}
                }
            } else if let Some(code) = dcode {
                // Bare D code selects aperture (D10..) or is an SR block call.
                if let Some(rest) = code.strip_prefix('D').and_then(|s| s.parse::<usize>().ok()) {
                    if blocks.contains_key(&code) && !apertures.iter().any(|a| a.code == code) {
                        // SR call encoded X..Y..D<code> normally; a bare call
                        // repeats at origin once.
                        expand_sr(
                            &mut seq,
                            &mut ops,
                            &blocks[&code],
                            &code,
                            &cur,
                            cur_mm,
                            1,
                            1,
                            ("0".into(), "0".into()),
                            (0.0, 0.0),
                            polarity,
                            &mut step_repeats,
                            fmt,
                        );
                    } else if rest >= 10 {
                        cur_ap = code;
                    }
                }
            }
            continue;
        }

        // SR block call: X<nx>Y<ny>I<dx>J<dy>D<blockcode>, using raw digits.
        if let Some(sr) = parse_sr_call(body) {
            if let Some(op_seqs) = blocks.get(&sr.code).cloned() {
                expand_sr(
                    &mut seq,
                    &mut ops,
                    &op_seqs,
                    &sr.code,
                    &cur,
                    cur_mm,
                    sr.nx,
                    sr.ny,
                    (sr.dx_raw.clone(), sr.dy_raw.clone()),
                    (sr.dx_mm, sr.dy_mm),
                    polarity,
                    &mut step_repeats,
                    fmt,
                );
            } else {
                diag(
                    "G_SR_NO_BLOCK",
                    line,
                    col,
                    format!("step-repeat references undefined block aperture {}", sr.code),
                    body.into(),
                );
            }
            continue;
        }

        if body == "M02" || body == "M00" {
            continue;
        }
        diag("G_SYNTAX", line, col, "unrecognized Gerber block".into(), body.into());
    }

    if fmt.is_none() {
        errs.insert(
            0,
            Diagnostic {
                code: "G_FORMAT_MISSING".into(),
                severity: "error".into(),
                file: name.into(),
                line: 1,
                column: 0,
                message: "no FS coordinate format declaration; coordinates cannot be converted"
                    .into(),
                token: String::new(),
            },
        );
    }

    if !errs.is_empty() {
        return Err(errs);
    }

    let f = fmt.unwrap();
    let datum = CoordinateDatum {
        unit: if f.inch { LengthUnit::Inch } else { LengthUnit::Mm },
        format: format!("{}.{}", f.int, f.dec),
        zero_suppression: f.zero.into(),
        notation: "absolute".into(),
    };
    Ok(GerberDoc { datum, apertures, ops, step_repeats })
}

fn byte_line(_text: &str, _block: &str) -> usize {
    // Callers currently use column only; line is approximated as 1 and refined
    // diagnostics carry the token. Kept for stable API.
    1
}

fn parse_fs(rest: &str) -> Result<Fmt, String> {
    // Examples: LAX24Y24, TAX35Y35
    let zero = if rest.starts_with('L') {
        "leading"
    } else if rest.starts_with('T') {
        "trailing"
    } else {
        return Err(format!("zero suppression must be L or T in '{}'", rest));
    };
    let inch = rest.contains("IN");
    let num = |key: char| -> Option<usize> {
        let i = rest.find(key)? + 1;
        let b = rest.as_bytes().get(i)?;
        (b - b'0') as usize;
        let d = rest.as_bytes().get(i + 1)?;
        Some((b - b'0') as usize * 10 + (d - b'0') as usize)
    };
    // After the key, two ASCII digits: total digits and decimal digits.
    let pair = |key: char| -> Option<(usize, usize)> {
        let i = rest.find(key)? + 1;
        let b0 = *rest.as_bytes().get(i)?;
        let b1 = *rest.as_bytes().get(i + 1)?;
        let total = (b0 - b'0') as usize;
        let dec = (b1 - b'0') as usize;
        Some((total - dec, dec))
    };
    let (int, dec) = pair('X').ok_or_else(|| format!("missing X digits in '{}'", rest))?;
    let _ = num;
    Ok(Fmt { int, dec, zero, inch })
}

fn parse_ad(body: &str) -> Result<ApertureDef, String> {
    // %ADD10C,0.25*% or %ADD11R,0.5X0.3*%
    let rest = body.trim_start_matches("AD");
    let comma = rest.find(',').ok_or("aperture missing comma")?;
    let code_raw = &rest[..comma];
    let code = format!("D{}", code_raw.trim_start_matches('D'));
    let shape_part = &rest[comma + 1..];
    let (shape, params) = shape_part
        .split_once(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or((shape_part, ""));
    let params = params.trim_start_matches(|c: char| c == ',' || c.is_ascii_whitespace());
    let params_raw: Vec<String> = if params.is_empty() {
        Vec::new()
    } else {
        params.split('X').map(|s| s.trim().to_string()).collect()
    };
    // Aperture dimensions are in document units; conversion happens in the
    // geometry layer which knows the datum. Store raw and parse f64 best effort.
    let params_mm: Vec<f64> = params_raw
        .iter()
        .map(|p| p.trim_end_matches(|c: char| !c.is_ascii_digit() && c != '.').parse::<f64>().unwrap_or(f64::NAN))
        .collect();
    Ok(ApertureDef { code, shape: shape.to_ascii_uppercase(), params_raw, params_mm })
}

fn decode_coord(token: &str, f: Option<Fmt>) -> Option<f64> {
    let f = f?;
    let mut digits: Vec<u8> = token.bytes().filter(|b| b.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let neg = token.contains('-');
    let total = f.int + f.dec;
    match f.zero {
        "leading" => {
            // trailing zeros present: right align to total length (suppress leading)
            while digits.len() < total {
                digits.insert(0, b'0');
            }
        }
        "trailing" => {
            while digits.len() < total {
                digits.push(b'0');
            }
        }
        _ => {}
    }
    if digits.len() > total {
                digits = digits.split_off(digits.len() - total);
    }
    let n: i64 = digits.iter().fold(0i64, |acc, &b| acc * 10 + (b - b'0') as i64);
    let mut v = n as f64 / 10f64.powi(f.dec as i32);
    if neg {
        v = -v;
    }
    if f.inch {
        v *= 25.4;
    }
    Some(v)
}

/// Parse `X..Y..D..`; returns raw xy (if present), normalized mm (if
/// convertible), and bare d-code if present.
fn parse_coord_block(
    body: &str,
    fmt: Option<Fmt>,
    errs: &mut Vec<Diagnostic>,
    file: &str,
    line: usize,
) -> Option<(Option<(String, String)>, Option<(f64, f64)>, Option<String>)> {
    let has_xy = body.contains('X') || body.contains('Y');
    let dcode = extract_token(body, 'D');
    if !has_xy {
        if let Some(d) = &dcode {
            return Some((None, None, Some(d.clone())));
        }
        return None;
    }
    let xr = extract_token(body, 'X').unwrap_or_default();
    let yr = extract_token(body, 'Y').unwrap_or_default();
    let xm = decode_coord(&xr, fmt);
    let ym = decode_coord(&yr, fmt);
    if fmt.is_none() {
        errs.push(Diagnostic {
            code: "G_COORD_UNCONVERTIBLE".into(),
            severity: "error".into(),
            file: file.into(),
            line,
            column: 0,
            message: "coordinate kept in original digits; no FS datum for conversion".into(),
            token: format!("X{}Y{}", xr, yr),
        });
    }
    Some((
        Some((xr.clone(), yr.clone())),
        xm.zip(ym).or_else(|| fmt.map(|_| (f64::NAN, f64::NAN))),
        dcode,
    ))
}

fn extract_token(body: &str, key: char) -> Option<String> {
    let i = body.find(key)?;
    let rest = &body[i + 1..];
    let end = rest
        .find(|c: char| c.is_ascii_alphabetic() && c != key)
        .unwrap_or(rest.len());
    let tok = rest[..end].trim();
    if tok.is_empty() {
        None
    } else {
        Some(tok.to_string())
    }
}

struct SrCall {
    code: String,
    nx: usize,
    ny: usize,
    dx_raw: String,
    dy_raw: String,
    dx_mm: f64,
    dy_mm: f64,
}

/// Step & repeat call. Real Gerber places X/Y as the block origin and I/J as
/// repeat pitches; our case pack additionally permits N/M repeat counts
/// (`X0Y0N2M3I2.0J0D10`) and plain block calls (single repeat at origin).
fn parse_sr_call(body: &str) -> Option<SrCall> {
    let d = extract_token(body, 'D')?;
    if !body.contains('X') {
        return None;
    }
    // D code must reference a block (caller checks membership), require X..Y..
    let nx = extract_token(body, 'N').and_then(|s| s.parse().ok()).unwrap_or(1);
    let ny = extract_token(body, 'M').and_then(|s| s.parse().ok()).unwrap_or(1);
    let dx_raw = extract_token(body, 'I').unwrap_or_else(|| "0".into());
    let dy_raw = extract_token(body, 'J').unwrap_or_else(|| "0".into());
    let dx_mm = dx_raw.parse::<f64>().unwrap_or(0.0);
    let dy_mm = dy_raw.parse::<f64>().unwrap_or(0.0);
    Some(SrCall {
        code: format!("D{}", d.trim_start_matches('D')),
        nx,
        ny,
        dx_raw,
        dy_raw,
        dx_mm,
        dy_mm,
    })
}

#[allow(clippy::too_many_arguments)]
fn expand_sr(
    seq: &mut usize,
    ops: &mut Vec<DrawOp>,
    op_seqs: &[usize],
    block_code: &str,
    origin_raw: &(String, String),
    origin_mm: Option<(f64, f64)>,
    nx: usize,
    ny: usize,
    pitch_raw: (String, String),
    pitch_mm: (f64, f64),
    polarity: Polarity,
    step_repeats: &mut Vec<StepRepeat>,
    fmt: Option<Fmt>,
) {
    let id = step_repeats.len() + 1;
    let dim = |raw: &str, v: f64| {
        let mm = if fmt.map(|f| f.inch).unwrap_or(false) {
            raw.parse::<f64>().ok().map(|i| i * 25.4)
        } else {
            raw.parse::<f64>().ok()
        };
        Dim::new(
            raw,
            fmt.map(|f| if f.inch { LengthUnit::Inch } else { LengthUnit::Mm })
                .unwrap_or(LengthUnit::User),
            mm.or(Some(v)),
        )
    };
    step_repeats.push(StepRepeat {
        id,
        nx,
        ny,
        dx: dim(&pitch_raw.0, pitch_mm.0),
        dy: dim(&pitch_raw.1, pitch_mm.1),
        op_seqs: op_seqs.to_vec(),
    });
    let (ox_mm, oy_mm) = origin_mm.unwrap_or((0.0, 0.0));
    let _ = origin_raw;
    let _ = block_code;
    for ix in 0..nx {
        for iy in 0..ny {
            for &os in op_seqs {
                let src = ops.iter().find(|o| o.seq == os).expect("block op exists").clone();
                *seq += 1;
                let shift = (
                    ox_mm + ix as f64 * pitch_mm.0,
                    oy_mm + iy as f64 * pitch_mm.1,
                );
                let points_mm = src
                    .points_mm
                    .iter()
                    .map(|(x, y)| (x + shift.0, y + shift.1))
                    .collect();
                ops.push(DrawOp {
                    seq: *seq,
                    aperture: src.aperture,
                    polarity,
                    kind: src.kind,
                    points_raw: src.points_raw.clone(),
                    points_mm,
                    repeat_ref: Some(id),
                });
            }
        }
    }
}
