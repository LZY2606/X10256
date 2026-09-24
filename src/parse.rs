//! Parsers for Gerber (RS-274X), Excellon drill, stackup and netlist.
//!
//! Parsing never fills defaults for missing declarations: a missing unit or
//! coordinate format keeps the coordinate unconverted and produces a
//! positioned diagnostic.

use crate::domain::*;
use crate::params::short_hash;

fn origin(filename: &str, material_id: &str, line: usize, col: Option<usize>, raw: &str) -> Origin {
    Origin {
        material_id: material_id.to_string(),
        filename: filename.to_string(),
        line,
        col,
        raw: raw.to_string(),
    }
}

fn diag(severity: DiagSeverity, code: &str, message: impl Into<String>, origin: Option<Origin>, unconverted: Option<String>) -> Diagnostic {
    Diagnostic {
        severity,
        code: code.to_string(),
        message: message.into(),
        origin,
        unconverted,
    }
}

/// Convert a coordinate token under the active format. Returns `None` when
/// the format/unit information is missing; the token is preserved then.
fn convert_coord(token: &str, fmt: Option<CoordFmt>) -> Option<f64> {
    let (axis, body) = if let Some(rest) = token.strip_prefix(['X', 'x']) {
        ('X', rest)
    } else if let Some(rest) = token.strip_prefix(['Y', 'y']) {
        ('Y', rest)
    } else if let Some(rest) = token.strip_prefix(['I', 'i']) {
        ('I', rest)
    } else {
        ('J', token.trim_start_matches(['J', 'j']))
    };
    let neg = body.starts_with('-');
    let digits: String = body.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    if body.contains('.') {
        let v: f64 = body.parse().ok()?;
        return Some(v);
    }
    let f = fmt?;
    let (int, dec) = if axis == 'Y' {
        (f.y_int, f.y_dec)
    } else {
        (f.x_int, f.x_dec)
    };
    let total = (int + dec) as usize;
    let padded;
    let digits = match f.zero {
        ZeroMode::LeadingSuppressed => {
            if digits.len() >= total {
                &digits[..]
            } else {
                padded = format!("{:0>width$}", digits, width = total);
                &padded
            }
        }
        ZeroMode::TrailingSuppressed => {
            if digits.len() >= total {
                &digits[..]
            } else {
                padded = format!("{:0<width$}", digits, width = total);
                &padded
            }
        }
        ZeroMode::Decimal => return None,
    };
    let split = total.saturating_sub(dec as usize);
    let (a, b) = if digits.len() >= total {
        (&digits[digits.len() - total..digits.len() - total + split], &digits[digits.len() - total + split..])
    } else {
        (&digits[..split.min(digits.len())], "")
    };
    let v: f64 = format!("{}.{}", a, b).parse().ok()?;
    Some(if neg { -v } else { v })
}

fn raw_coord(token: Option<&str>, fmt: Option<CoordFmt>) -> Option<RawCoord> {
    token.map(|t| RawCoord {
        raw: t.to_string(),
        value: convert_coord(t, fmt),
    })
}

fn split_coord_tokens(seg: &str) -> (Option<&str>, Option<&str>, Option<&str>, Option<&str>, &str) {
    let mut x = None;
    let mut y = None;
    let mut i = None;
    let mut j = None;
    let bytes = seg.as_bytes();
    let mut k = 0;
    while k < bytes.len() {
        let axis = bytes[k] as char;
        if matches!(axis, 'X' | 'Y' | 'I' | 'J') {
            let start = k;
            k += 1;
            while k < bytes.len() {
                let c = bytes[k] as char;
                if matches!(c, 'X' | 'Y' | 'I' | 'J' | 'D' | 'G') {
                    break;
                }
                k += 1;
            }
            let tok = &seg[start..k];
            match axis {
                'X' => x = Some(tok),
                'Y' => y = Some(tok),
                'I' => i = Some(tok),
                _ => j = Some(tok),
            }
        } else {
            k += 1;
        }
    }
    let d_pos = seg.find('D');
    let tail = match d_pos {
        Some(p) => &seg[p..],
        None => seg,
    };
    (x, y, i, j, tail)
}

/// Parse a Gerber RS-274X file body.
pub fn parse_gerber(filename: &str, material_id: &str, body: &str) -> RawGerber {
    let mut g = RawGerber {
        filename: filename.to_string(),
        material_id: material_id.to_string(),
        declared_unit: None,
        declared_fmt: None,
        commands: Vec::new(),
        diagnostics: Vec::new(),
    };
    let mut current_polarity = Polarity::Dark;
    let mut in_region = false;
    let mut interp = Interp::Line;
    let mut cmd_count: u64 = 0;

    for (idx, raw_line) in body.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let og = origin(filename, material_id, line_no, Some(1), line);

        if let Some(inner) = line.strip_prefix('%') {
            let inner = inner.strip_suffix('%').unwrap_or(inner);
            let mut chars = inner.chars();
            let head: String = chars.by_ref().take(2).collect();
            let rest: String = chars.collect();
            match head.as_str() {
                "MO" => {
                    let unit = match rest.trim_end_matches('*').trim() {
                        "MM" => Some(LengthUnit::Mm),
                        "IN" => Some(LengthUnit::In),
                        other => {
                            g.diagnostics.push(diag(
                                DiagSeverity::Error,
                                "GERBER_UNIT_UNKNOWN",
                                format!("unknown MO unit '{other}'"),
                                Some(origin(filename, material_id, line_no, Some(4), line)),
                                Some(other.to_string()),
                            ));
                            None
                        }
                    };
                    if let Some(u) = unit {
                        g.declared_unit = Some(u);
                        g.commands.push(GerberCmd::UnitDecl { unit: u, origin: og });
                    }
                }
                "FS" => {
                    let fmt = parse_fs(&rest);
                    if fmt.is_none() {
                        g.diagnostics.push(diag(
                            DiagSeverity::Error,
                            "GERBER_FMT_BAD",
                            "FS coordinate format could not be parsed; coordinates kept raw",
                            Some(og.clone()),
                            Some(rest.clone()),
                        ));
                    } else {
                        g.declared_fmt = fmt;
                    }
                    g.commands.push(GerberCmd::FmtDecl { fmt, origin: og });
                }
                "AD" => match parse_aperture(&rest, filename, material_id, line_no, line) {
                    Ok(def) => g.commands.push(GerberCmd::ApertureDecl(def)),
                    Err(d) => g.diagnostics.push(d),
                },
                "LP" => {
                    let pol = match rest.trim_end_matches('*').trim() {
                        "D" => Polarity::Dark,
                        "C" => Polarity::Clear,
                        other => {
                            g.diagnostics.push(diag(
                                DiagSeverity::Error,
                                "GERBER_POLARITY_BAD",
                                format!("unknown LP polarity '{other}'"),
                                Some(origin(filename, material_id, line_no, Some(4), line)),
                                Some(other.to_string()),
                            ));
                            continue;
                        }
                    };
                    current_polarity = pol;
                    g.commands.push(GerberCmd::Polarity { polarity: pol, origin: og });
                }
                "SR" => {
                    let body = rest.trim_end_matches('*');
                    if body.trim().is_empty() {
                        g.commands.push(GerberCmd::StepRepeat {
                            nx: None,
                            ny: None,
                            dx: None,
                            dy: None,
                            exit: true,
                            origin: og,
                        });
                    } else {
                        let (nx, ny, dx, dy) = parse_sr(body);
                        for (name, tok) in [("NX", nx.as_ref()), ("NY", ny.as_ref()), ("DX", dx.as_ref()), ("DY", dy.as_ref())] {
                            if let Some(t) = tok {
                                if t.value.is_none() {
                                    g.diagnostics.push(diag(
                                        DiagSeverity::Warning,
                                        "GERBER_SR_UNCONVERTED",
                                        format!("SR {name} value kept raw"),
                                        Some(og.clone()),
                                        Some(t.raw.clone()),
                                    ));
                                }
                            }
                        }
                        g.commands.push(GerberCmd::StepRepeat { nx, ny, dx, dy, exit: false, origin: og });
                    }
                }
                "TF" | "TA" | "TO" | "TD" => {
                    // File/object attributes are retained but do not change geometry.
                    g.commands.push(GerberCmd::Unsupported {
                        text: line.to_string(),
                        origin: og,
                    });
                }
                other => {
                    g.diagnostics.push(diag(
                        DiagSeverity::Warning,
                        "GERBER_COMMAND_UNKNOWN",
                        format!("extended command %{other}% retained, not interpreted"),
                        Some(og.clone()),
                        Some(line.to_string()),
                    ));
                    g.commands.push(GerberCmd::Unsupported {
                        text: line.to_string(),
                        origin: og,
                    });
                }
            }
            continue;
        }

        if line.starts_with('G') || line.contains('D') || line.contains('X') || line.contains('Y') {
            let l = line.trim_end_matches('*').trim();
            if let Some(rest) = l.strip_prefix("G04") {
                let _ = rest;
                continue;
            }
            if let Some(rest) = l.strip_prefix("G36") {
                in_region = true;
                g.commands.push(GerberCmd::RegionBegin { origin: og.clone() });
                if !rest.is_empty() {
                    push_draw_or_flash(&mut g, rest, filename, material_id, line_no, line, &mut interp, in_region);
                }
                continue;
            }
            if let Some(rest) = l.strip_prefix("G37") {
                in_region = false;
                g.commands.push(GerberCmd::RegionEnd { origin: og.clone() });
                if !rest.is_empty() {
                    push_draw_or_flash(&mut g, rest, filename, material_id, line_no, line, &mut interp, in_region);
                }
                continue;
            }
            if let Some(rest) = l.strip_prefix("G01") {
                interp = Interp::Line;
                if rest.is_empty() {
                    continue;
                }
                push_draw_or_flash(&mut g, rest, filename, material_id, line_no, line, &mut interp, in_region);
                continue;
            }
            if let Some(rest) = l.strip_prefix("G02") {
                interp = Interp::ArcCw;
                push_draw_or_flash(&mut g, rest, filename, material_id, line_no, line, &mut interp, in_region);
                continue;
            }
            if let Some(rest) = l.strip_prefix("G03") {
                interp = Interp::ArcCcw;
                push_draw_or_flash(&mut g, rest, filename, material_id, line_no, line, &mut interp, in_region);
                continue;
            }
            push_draw_or_flash(&mut g, l, filename, material_id, line_no, line, &mut interp, in_region);
            cmd_count += 1;
        } else {
            g.diagnostics.push(diag(
                DiagSeverity::Warning,
                "GERBER_LINE_UNKNOWN",
                "line retained but not interpreted",
                Some(og),
                Some(line.to_string()),
            ));
        }
    }

    if g.declared_unit.is_none() {
        g.diagnostics.push(diag(
            DiagSeverity::Error,
            "GERBER_UNIT_MISSING",
            "no MO unit declaration; derived normalisation cannot be trusted",
            None,
            None,
        ));
    }
    if g.declared_fmt.is_none() {
        g.diagnostics.push(diag(
            DiagSeverity::Warning,
            "GERBER_FMT_MISSING",
            "no FS coordinate format; coordinates without decimal point kept raw",
            None,
            None,
        ));
    }
    g
}

fn parse_fs(rest: &str) -> Option<CoordFmt> {
    // e.g. `FSLAX24Y24*`
    let r = rest.trim_end_matches('*');
    let mut chars = r.chars();
    let f = chars.next()?;
    let _s = chars.next()?;
    let zero = match chars.next()? {
        'L' => ZeroMode::LeadingSuppressed,
        'T' => ZeroMode::TrailingSuppressed,
        'D' => ZeroMode::Decimal,
        _ => return None,
    };
    let _a = chars.next()?;
    let mut nums = String::new();
    let mut letters: Vec<char> = Vec::new();
    for c in chars {
        if c.is_ascii_digit() {
            nums.push(c);
        } else if c == 'X' || c == 'Y' {
            letters.push(c);
        }
    }
    if nums.len() != 4 || letters != vec!['X', 'Y'] {
        return None;
    }
    let n: Vec<u8> = nums.chars().map(|c| c as u8 - b'0').collect();
    Some(CoordFmt {
        x_int: n[0],
        x_dec: n[1],
        y_int: n[2],
        y_dec: n[3],
        zero,
    })
}

fn parse_sr(body: &str) -> (Option<RawCoord>, Option<RawCoord>, Option<RawCoord>, Option<RawCoord>) {
    let mut nx = None;
    let mut ny = None;
    let mut dx = None;
    let mut dy = None;
    let mut k = 0;
    let bytes = body.as_bytes();
    while k < bytes.len() {
        let key: String = body[k..].chars().take(2).collect();
        if matches!(key.as_str(), "NX" | "NY" | "DX" | "DY") {
            let start = k;
            k += 2;
            while k < bytes.len() {
                let c = bytes[k] as char;
                if c.is_ascii_alphabetic() {
                    break;
                }
                k += 1;
            }
            let tok = body[start..k].trim();
            let rc = RawCoord { raw: tok.to_string(), value: tok.parse::<f64>().ok() };
            match key.as_str() {
                "NX" => nx = Some(rc),
                "NY" => ny = Some(rc),
                "DX" => dx = Some(rc),
                _ => dy = Some(rc),
            }
        } else {
            k += 1;
        }
    }
    (nx, ny, dx, dy)
}

fn parse_aperture(rest: &str, filename: &str, material_id: &str, line_no: usize, line: &str) -> Result<ApertureDef, Diagnostic> {
    // `D10C,0.5*`
    let body = rest.trim_end_matches('*');
    let d_end = body.find(['C', 'R', 'O', 'P', 'M']).ok_or_else(|| {
        diag(
            DiagSeverity::Error,
            "GERBER_APERTURE_BAD",
            "aperture definition has no shape",
            Some(origin(filename, material_id, line_no, Some(4), line)),
            Some(body.to_string()),
        )
    })?;
    let code = body[..d_end].trim().to_string();
    let shape_part = &body[d_end..];
    let (kind, mods) = if let Some(comma) = shape_part.find(',') {
        (shape_part[..comma].trim().to_string(), shape_part[comma + 1..].trim())
    } else {
        (shape_part.trim().to_string(), "")
    };
    let modifiers_raw = if mods.is_empty() {
        Vec::new()
    } else {
        mods.split('X').map(|s| s.trim().to_string()).collect()
    };
    Ok(ApertureDef {
        code,
        kind,
        modifiers_raw,
        origin: origin(filename, material_id, line_no, Some(4), line),
    })
}

fn push_draw_or_flash(
    g: &mut RawGerber,
    seg: &str,
    filename: &str,
    material_id: &str,
    line_no: usize,
    line: &str,
    interp: &mut Interp,
    _in_region: bool,
) {
    let seg = seg.trim();
    let og = origin(filename, material_id, line_no, None, line);
    let (xt, yt, it, jt, tail) = split_coord_tokens(seg);
    let x = raw_coord(xt, g.declared_fmt);
    let y = raw_coord(yt, g.declared_fmt);
    let i = raw_coord(it, g.declared_fmt);
    let j = raw_coord(jt, g.declared_fmt);
    for (name, c) in [("X", x.as_ref()), ("Y", y.as_ref()), ("I", i.as_ref()), ("J", j.as_ref())] {
        if let Some(c) = c {
            if c.value.is_none() {
                g.diagnostics.push(diag(
                    DiagSeverity::Error,
                    "GERBER_COORD_UNCONVERTED",
                    format!("{name} coordinate kept raw; no usable format"),
                    Some(og.clone()),
                    Some(c.raw.clone()),
                ));
            }
        }
    }
    // D02 move, D03 flash, D01 draw (D03 may be implicit in some code sets).
    let tail = tail.trim_end_matches('*').trim();
    if let Some(rest) = tail.strip_prefix('D') {
        match rest {
            "02" => {
                g.commands.push(GerberCmd::Move { x, y, origin: og });
                return;
            }
            "03" => {
                g.commands.push(GerberCmd::Flash { x, y, origin: og });
                return;
            }
            "01" | "1" => {
                g.commands.push(GerberCmd::Draw { x, y, i, j, interp: *interp, origin: og });
                return;
            }
            _ => {}
        }
    }
    if x.is_some() || y.is_some() {
        g.commands.push(GerberCmd::Draw { x, y, i, j, interp: *interp, origin: og });
    }
}

/// Parse an Excellon drill file with the documented blind/buried extensions
/// `;SPAN=from-to` and `;METAL=plated|nonplated`.
pub fn parse_excellon(filename: &str, material_id: &str, body: &str) -> RawExcellon {
    let mut e = RawExcellon {
        filename: filename.to_string(),
        material_id: material_id.to_string(),
        declared_unit: None,
        declared_fmt: None,
        tools: Vec::new(),
        spans: Vec::new(),
        hits: Vec::new(),
        diagnostics: Vec::new(),
    };
    let mut fmt: Option<(u8, u8)> = None;
    let mut active_span: Option<usize> = None;
    let mut plated = true;
    let mut current_tool: Option<String> = None;

    for (idx, raw_line) in body.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let og = origin(filename, material_id, line_no, Some(1), line);

        if let Some(comment) = line.strip_prefix(';') {
            if let Some(rest) = trimmed_after(comment, "FORMAT=") {
                match parse_excellon_fmt(&rest) {
                    Some(f) => {
                        fmt = Some(f);
                        e.declared_fmt = Some(f);
                    }
                    None => e.diagnostics.push(diag(
                        DiagSeverity::Error,
                        "DRILL_FMT_BAD",
                        ";FORMAT= could not be parsed (expected int.dec, e.g. 2.4)",
                        Some(og),
                        Some(rest),
                    )),
                }
            } else if let Some(rest) = trimmed_after(comment, "SPAN=") {
                let (a, b) = match rest.split_once('-') {
                    Some((a, b)) => (a.trim().to_string(), b.trim().to_string()),
                    None => {
                        e.diagnostics.push(diag(
                            DiagSeverity::Error,
                            "DRILL_SPAN_BAD",
                            ";SPAN= must look like L1-L2",
                            Some(og.clone()),
                            Some(rest.clone()),
                        ));
                        continue;
                    }
                };
                e.spans.push(RawSpan {
                    from_raw: a,
                    to_raw: b,
                    plated,
                    plated_raw: if plated { "plated" } else { "nonplated" }.to_string(),
                    origin: og,
                });
                active_span = Some(e.spans.len() - 1);
            } else if let Some(rest) = trimmed_after(comment, "METAL=") {
                match rest.trim().to_ascii_lowercase().as_str() {
                    "plated" | "pth" => plated = true,
                    "nonplated" | "npth" => plated = false,
                    other => {
                        e.diagnostics.push(diag(
                            DiagSeverity::Error,
                            "DRILL_METAL_BAD",
                            ";METAL= must be plated or nonplated",
                            Some(og.clone()),
                            Some(other.to_string()),
                        ));
                    }
                }
            }
            continue;
        }

        match line {
            "M48" | "%" | "M30" => continue,
            "INCH" => {
                e.declared_unit = Some(LengthUnit::In);
                continue;
            }
            "METRIC" => {
                e.declared_unit = Some(LengthUnit::Mm);
                continue;
            }
            "M71" => {
                e.declared_unit = Some(LengthUnit::Mm);
                continue;
            }
            "M72" => {
                e.declared_unit = Some(LengthUnit::In);
                continue;
            }
            _ => {}
        }

        if let Some(rest) = line.strip_prefix('T') {
            // Tool definition `T01C0.3` or selection `T01`.
            let end = rest.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(rest.len());
            let id_raw = rest[..end].trim_start_matches('0');
            let id = if id_raw.is_empty() { "0" } else { id_raw }.to_string();
            if end < rest.len() {
                let spec = &rest[end..];
                let diam_raw;
                let diameter;
                if let Some(v) = spec.strip_prefix('C') {
                    diam_raw = v.to_string();
                    diameter = v.trim().parse::<f64>().ok();
                } else {
                    diam_raw = spec.to_string();
                    diameter = None;
                    e.diagnostics.push(diag(
                        DiagSeverity::Error,
                        "DRILL_TOOL_BAD",
                        "tool definition has unsupported shape",
                        Some(og.clone()),
                        Some(spec.to_string()),
                    ));
                }
                if diameter.is_none() {
                    e.diagnostics.push(diag(
                        DiagSeverity::Error,
                        "DRILL_DIAMETER_UNCONVERTED",
                        "tool diameter kept raw",
                        Some(og.clone()),
                        Some(diam_raw.clone()),
                    ));
                }
                e.tools.push(RawTool {
                    id: id.clone(),
                    diameter_raw,
                    diameter,
                    origin: og.clone(),
                });
            }
            current_tool = Some(id);
            continue;
        }

        if line.starts_with('X') || line.starts_with('Y') {
            let (xt, yt, _, _, _) = split_coord_tokens(line);
            let x = match raw_coord(xt, fmt.map(|(i, d)| CoordFmt { x_int: i, x_dec: d, y_int: i, y_dec: d, zero: ZeroMode::LeadingSuppressed })) {
                Some(c) => c,
                None => RawCoord { raw: String::new(), value: None },
            };
            let y = match raw_coord(yt, fmt.map(|(i, d)| CoordFmt { x_int: i, x_dec: d, y_int: i, y_dec: d, zero: ZeroMode::LeadingSuppressed })) {
                Some(c) => c,
                None => RawCoord { raw: String::new(), value: None },
            };
            if x.value.is_none() {
                e.diagnostics.push(diag(
                    DiagSeverity::Error,
                    "DRILL_X_UNCONVERTED",
                    "hit X kept raw; missing FORMAT or invalid digits",
                    Some(og.clone()),
                    if x.raw.is_empty() { None } else { Some(x.raw.clone()) },
                ));
            }
            if y.value.is_none() {
                e.diagnostics.push(diag(
                    DiagSeverity::Error,
                    "DRILL_Y_UNCONVERTED",
                    "hit Y kept raw; missing FORMAT or invalid digits",
                    Some(og.clone()),
                    if y.raw.is_empty() { None } else { Some(y.raw.clone()) },
                ));
            }
            let tool_id = match &current_tool {
                Some(t) => t.clone(),
                None => {
                    e.diagnostics.push(diag(
                        DiagSeverity::Error,
                        "DRILL_NO_TOOL",
                        "hit before any T tool selection",
                        Some(og.clone()),
                        None,
                    ));
                    String::from("?")
                }
            };
            e.hits.push(RawHit { tool_id, x, y, span_idx: active_span, origin: og });
            continue;
        }

        e.diagnostics.push(diag(
            DiagSeverity::Warning,
            "DRILL_LINE_UNKNOWN",
            "line retained but not interpreted",
            Some(og),
            Some(line.to_string()),
        ));
    }

    if e.declared_unit.is_none() {
        e.diagnostics.push(diag(
            DiagSeverity::Error,
            "DRILL_UNIT_MISSING",
            "no METRIC/INCH (M71/M72) declaration",
            None,
            None,
        ));
    }
    if e.declared_fmt.is_none() {
        e.diagnostics.push(diag(
            DiagSeverity::Warning,
            "DRILL_FMT_MISSING",
            "no ;FORMAT= int.dec declaration",
            None,
            None,
        ));
    }
    e
}

fn parse_excellon_fmt(s: &str) -> Option<(u8, u8)> {
    let (a, b) = s.trim().split_once('.')?;
    let i: u8 = a.parse().ok()?;
    let d: u8 = b.parse().ok()?;
    Some((i, d))
}

fn trimmed_after<'a>(line: &'a str, key: &str) -> Option<String> {
    line.find(key).map(|p| line[p + key.len()..].trim().to_string())
}

/// Parse the documented stackup description:
/// `LAYER name role path`, `DRILL plated|nonplated path`, `NETLIST path`,
/// `MAP logical material`.
pub fn parse_stackup(filename: &str, material_id: &str, body: &str) -> RawStackup {
    let mut s = RawStackup {
        filename: filename.to_string(),
        material_id: material_id.to_string(),
        layers: Vec::new(),
        drills: Vec::new(),
        netlist_path: None,
        mapping: Vec::new(),
        diagnostics: Vec::new(),
    };
    for (idx, raw_line) in body.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let og = origin(filename, material_id, line_no, Some(1), line);
        let mut parts = line.split_whitespace();
        let keyword = parts.next().unwrap_or("");
        match keyword {
            "LAYER" => {
                let name = parts.next().unwrap_or("");
                let role_raw = parts.next().unwrap_or("");
                let path = parts.next().unwrap_or("");
                let role = LayerRole::parse(role_raw);
                if role.is_none() {
                    s.diagnostics.push(diag(
                        DiagSeverity::Error,
                        "STACK_ROLE_UNKNOWN",
                        format!("layer role '{role_raw}' not recognised; raw value retained"),
                        Some(og.clone()),
                        Some(role_raw.to_string()),
                    ));
                }
                if name.is_empty() || path.is_empty() {
                    s.diagnostics.push(diag(
                        DiagSeverity::Error,
                        "STACK_LAYER_INCOMPLETE",
                        "LAYER needs name role path; no default is invented",
                        Some(og),
                        Some(line.to_string()),
                    ));
                    continue;
                }
                s.layers.push(StackLayer {
                    name: name.to_string(),
                    role_raw: role_raw.to_string(),
                    role,
                    path: path.to_string(),
                    origin: og,
                });
            }
            "DRILL" => {
                let plated_raw = parts.next().unwrap_or("");
                let path = parts.next().unwrap_or("");
                let plated = match plated_raw {
                    "plated" | "PTH" => true,
                    "nonplated" | "NPTH" => false,
                    other => {
                        s.diagnostics.push(diag(
                            DiagSeverity::Error,
                            "STACK_DRILL_METAL_BAD",
                            "DRILL must say plated or nonplated",
                            Some(og.clone()),
                            Some(other.to_string()),
                        ));
                        continue;
                    }
                };
                if path.is_empty() {
                    s.diagnostics.push(diag(
                        DiagSeverity::Error,
                        "STACK_DRILL_NO_PATH",
                        "DRILL has no file path",
                        Some(og),
                        None,
                    ));
                    continue;
                }
                s.drills.push(StackDrill {
                    plated,
                    plated_raw: plated_raw.to_string(),
                    path: path.to_string(),
                    origin: og,
                });
            }
            "NETLIST" => {
                s.netlist_path = parts.next().map(|p| p.to_string());
            }
            "MAP" => {
                let assignment = parts.next().unwrap_or("");
                match assignment.split_once('=') {
                    Some((logical, mat)) => s.mapping.push(MapRow {
                        logical: logical.to_string(),
                        material_ref: mat.to_string(),
                        origin: og,
                    }),
                    None => s.diagnostics.push(diag(
                        DiagSeverity::Error,
                        "STACK_MAP_BAD",
                        "MAP must be logical=material",
                        Some(og),
                        Some(assignment.to_string()),
                    )),
                }
            }
            other => {
                s.diagnostics.push(diag(
                    DiagSeverity::Warning,
                    "STACK_LINE_UNKNOWN",
                    format!("unknown stackup keyword '{other}'"),
                    Some(og),
                    Some(line.to_string()),
                ));
            }
        }
    }
    if s.layers.is_empty() {
        s.diagnostics.push(diag(
            DiagSeverity::Error,
            "STACK_NO_LAYERS",
            "stackup declares no layers",
            None,
            None,
        ));
    }
    s
}

pub fn parse_netlist(filename: &str, material_id: &str, body: &str) -> RawNetlist {
    let mut n = RawNetlist {
        filename: filename.to_string(),
        material_id: material_id.to_string(),
        entries: Vec::new(),
        diagnostics: Vec::new(),
    };
    for (idx, raw_line) in body.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let og = origin(filename, material_id, line_no, Some(1), line);
        if let Some(rest) = line.strip_prefix("NET ") {
            let mut parts = rest.split_whitespace();
            let net = parts.next().unwrap_or("");
            let target = parts.next().unwrap_or("");
            match target.split_once(':') {
                Some((layer, fig)) if !net.is_empty() => n.entries.push(NetEntry {
                    net: net.to_string(),
                    layer: layer.to_string(),
                    figure_ref: fig.to_string(),
                    origin: og,
                }),
                _ => n.diagnostics.push(diag(
                    DiagSeverity::Error,
                    "NET_ENTRY_BAD",
                    "NET needs net layer:figure-ref",
                    Some(og),
                    Some(line.to_string()),
                )),
            }
        } else {
            n.diagnostics.push(diag(
                DiagSeverity::Warning,
                "NET_LINE_UNKNOWN",
                "line retained but not interpreted",
                Some(og),
                Some(line.to_string()),
            ));
        }
    }
    n
}

/// Stable material identity derived solely from kind, filename and bytes.
pub fn material_id(kind: MaterialKind, filename: &str, body: &str) -> String {
    format!("mat/{}#{}", kind.code(), short_hash(format!("{}\n{}", filename, body).as_bytes()))
}
