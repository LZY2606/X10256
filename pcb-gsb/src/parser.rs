//! Source parsers: Gerber RS-274X subset, Excellon subset, stackup text.
//!
//! The parsed documents are the *material* layer: coordinates, units, polarity,
//! apertures, step/repeat and layer roles are preserved verbatim. Numeric
//! interpretation is best-effort; values that cannot be converted stay in
//! place together with a diagnostic.

use crate::model::*;

const MM_NM: i64 = 1_000_000;
const INCH_NM: i64 = 25_400_000;

fn err(code: &str, message: impl Into<String>, lf: u32, lt: u32) -> Diagnostic {
    Diagnostic { code: code.into(), message: message.into(), line_from: lf, line_to: lt, severity: Severity::Error }
}
fn warn(code: &str, message: impl Into<String>, lf: u32, lt: u32) -> Diagnostic {
    Diagnostic { code: code.into(), message: message.into(), line_from: lf, line_to: lt, severity: Severity::Warning }
}

/// Coordinate number interpretation given the active datum.
/// `raw` is the exact coordinate token body (digits, optional sign/decimal).
fn interpret_coord(raw: &str, datum: &CoordDatum) -> Option<i64> {
    if raw.is_empty() {
        return None;
    }
    // Decimal point always wins (explicit format).
    if raw.contains('.') {
        return match raw.parse::<f64>() {
            Ok(v) => Some((v * datum.unit_nm as f64).round() as i64),
            Err(_) => None,
        };
    }
    let (int_d, frac_d) = parse_format(&datum.format)?;
    let neg = raw.starts_with('-') || raw.starts_with('+');
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let total = (int_d + frac_d) as usize;
    let padded = match datum.zero_suppression.as_str() {
        // Leading zeros omitted: right aligned, pad zeros on the left.
        "L" => format!("{:0>width$}", digits, width = total),
        // Trailing zeros omitted: left aligned, pad zeros on the right.
        "T" => format!("{:<0width$}", digits, width = total),
        // No suppression: must be exact width.
        _ => {
            if digits.len() != total {
                return None;
            }
            digits
        }
    };
    if padded.len() != total {
        return None;
    }
    let int_part: i64 = padded[..int_d as usize].parse().ok()?;
    let frac_part: i64 = padded[int_d as usize..].parse().ok()?;
    let scale = 10i64.pow(frac_d as u32);
    let unit_value = (int_part * scale + frac_part) as f64 / scale as f64;
    let mut nm = (unit_value * datum.unit_nm as f64).round() as i64;
    if neg && raw.starts_with('-') {
        nm = -nm;
    }
    Some(nm)
}

fn parse_format(fmt: &str) -> Option<(u32, u32)> {
    let (a, b) = fmt.split_once('.')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

fn raw_num(raw: String, datum: &CoordDatum) -> RawNumber {
    let v = interpret_coord(&raw, datum);
    let note = if v.is_none() && !raw.is_empty() {
        Some("uninterpretable: left at original position".into())
    } else {
        None
    };
    RawNumber { raw, value_nm: v, note }
}

fn parse_measure(token: &str, unit_nm: i64) -> Option<i64> {
    let v: f64 = token.parse().ok()?;
    Some((v * unit_nm as f64).round() as i64)
}

// ---------------------------------------------------------------------------
// Gerber RS-274X subset
// ---------------------------------------------------------------------------

pub fn parse_gerber(text: &str) -> GerberDoc {
    let mut datum = CoordDatum::default();
    let mut role = LayerRole::Unknown;
    let mut apertures: Vec<ApertureDef> = Vec::new();
    let mut step_repeats: Vec<StepRepeat> = Vec::new();
    let mut ops: Vec<DrawOp> = Vec::new();
    let mut warnings: Vec<Diagnostic> = Vec::new();

    let mut polarity = Polarity::Dark;
    let mut current_ap = String::from("D10");
    let mut current_block: Option<String> = None;
    let mut active_repeat: Option<&StepRepeat> = None;
    let mut last: Option<(Option<i64>, Option<i64>)> = None;
    let mut step_mode = false;
    let mut step_name = String::new();
    let mut seq: u32 = 0;
    let mut in_body = false;

    for (idx, raw_line) in text.lines().enumerate() {
        let ln = (idx + 1) as u32;
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        // --- extended commands ---
        if let Some(rest) = line.strip_prefix("%") {
            let body = rest.trim_end_matches('%');
            if let Some(v) = body.strip_prefix("MO") {
                match v {
                    "IN" | "IN*" => { datum.unit = "inch".into(); datum.unit_nm = INCH_NM; }
                    "MM" | "MM*" => { datum.unit = "mm".into(); datum.unit_nm = MM_NM; }
                    other => warnings.push(warn("gerber-unit", format!("unknown unit '{other}'"), ln, ln)),
                }
                continue;
            }
            if let Some(v) = body.strip_prefix("FS") {
                // e.g. FSLAX36Y36
                datum.notation = if v.contains('A') { "A".into() } else if v.contains('I') { "I".into() } else { datum.notation };
                datum.zero_suppression = if v.contains('L') { "L".into() } else if v.contains('T') { "T".into() } else { "N".into() };
                if let Some(fmt) = extract_fs_format(v) {
                    datum.format = fmt;
                }
                continue;
            }
            if let Some(v) = body.strip_prefix("AD") {
                // ADD10C,0.25mm* / ADR12R1.5X0.8
                if let Some(ap) = parse_aperture(v, &datum, ln, &mut warnings) {
                    apertures.push(ap);
                }
                continue;
            }
            if let Some(v) = body.strip_prefix("LP") {
                polarity = if v.starts_with('C') { Polarity::Clear } else { Polarity::Dark };
                continue;
            }
            if let Some(v) = body.strip_prefix("TF") {
                // File function / polarity hints for role.
                if let Some(r) = v.strip_prefix(".FileFunction,") {
                    let first = r.split(',').next().unwrap_or("");
                    role = gerber_file_function(first);
                }
                continue;
            }
            if body.starts_with("SR") {
                // %SR3X2I0J0*% start a step-repeat open block
                if let Some(sr) = parse_sr_decl(body, &datum, ln, &mut warnings) {
                    step_repeats.push(sr.clone());
                    active_repeat = step_repeats.last();
                    step_mode = true;
                    step_name = format!("sr{}", step_repeats.len());
                }
                continue;
            }
            if body == "SR" {
                // close step/repeat
                step_mode = false;
                current_block = None;
                active_repeat = None;
                continue;
            }
            if body.starts_with("TD") || body == "TD" {
                // aperture / attribute template: preserve-ignore
                continue;
            }
            if body.starts_with("TA") || body.starts_with("TO") {
                continue;
            }
            warnings.push(warn("gerber-cmd", format!("unhandled extended command: {body}"), ln, ln));
            continue;
        }

        if line == "G04*" || line.starts_with("G04") {
            continue;
        }

        // --- G codes ---
        if line.starts_with("G36") {
            current_block = Some("region".into());
            continue;
        }
        if line.starts_with("G37") {
            current_block = None;
            continue;
        }
        if line.starts_with("G01") || line.starts_with("G1") || line.starts_with("G02") || line.starts_with("G2")
            || line.starts_with("G03") || line.starts_with("G3") {
            // interpolation; coordinates handled below
            let rest = line.trim_start_matches(|c: char| c.is_ascii_digit() || c == 'G');
            if rest.is_empty() || rest == "*" {
                continue;
            }
        }
        if line.starts_with("D") && line.ends_with('*') && !line.contains('X') && !line.contains('Y') {
            // aperture select e.g. D12*
            current_ap = line.trim_end_matches('*').to_string();
            continue;
        }

        if line == "M02*" {
            break;
        }

        // --- coordinate data ---
        if line.contains('X') || line.contains('Y') {
            in_body = true;
            seq += 1;
            let (xs, ys, dcode) = split_coord(line);
            let xraw = xs.unwrap_or_default();
            let yraw = ys.unwrap_or_default();
            let x = raw_num(xraw.clone(), &datum);
            let y = raw_num(yraw.clone(), &datum);
            if (x.value_nm.is_none() && !xraw.is_empty()) || (y.value_nm.is_none() && !yraw.is_empty()) {
                warnings.push(err("gerber-coord",
                    format!("coordinate '{line}' cannot be converted with datum {}/{}; kept raw", datum.unit, datum.format),
                    ln, ln));
            }
            let (fx, fy) = last.unwrap_or((None, None));
            let from = Position {
                x: RawNumber { raw: format!("{}", fx.unwrap_or(0)), value_nm: fx, note: None },
                y: RawNumber { raw: format!("{}", fy.unwrap_or(0)), value_nm: fy, note: None },
            };
            let to = Position { x, y };
            let kind = match dcode.as_deref() {
                Some("D03") | Some("D3") => "flash",
                Some("D02") | Some("D2") => "move",
                _ => "draw",
            };
            // step & repeat expansion: record each grid instance provenance
            let instances: Vec<Option<(u32, u32)>> = match (step_mode, active_repeat) {
                (true, Some(sr)) => {
                    let mut v = Vec::new();
                    for i in 0..sr.nx.max(1) {
                        for j in 0..sr.ny.max(1) {
                            v.push(Some((i, j)));
                        }
                    }
                    v
                }
                _ => vec![None],
            };
            for rep in instances {
                ops.push(DrawOp {
                    seq,
                    aperture: current_ap.clone(),
                    polarity,
                    kind: kind.to_string(),
                    from: from.clone(),
                    to: to.clone(),
                    repeat: rep,
                    block: current_block.clone().or_else(|| if step_mode { Some(step_name.clone()) } else { None }),
                    source_lines: (ln, ln),
                });
            }
            last = Some((to.x.value_nm.or(fx), to.y.value_nm.or(fy)));
            continue;
        }

        if in_body || !line.starts_with('%') {
            warnings.push(warn("gerber-line", format!("unrecognized line: {line}"), ln, ln));
        }
    }

    if datum.unit_nm == 0 {
        warnings.push(err("gerber-unit", "no MO unit declaration; coordinates cannot be normalized safely", 1, 1));
    }
    if datum.format == "unknown" {
        warnings.push(warn("gerber-format", "no FS coordinate format declared", 1, 1));
    }

    GerberDoc { datum, role, apertures, step_repeats, ops, warnings }
}

fn extract_fs_format(v: &str) -> Option<String> {
    // FSLAX36Y36 -> "3.6"
    let xpos = v.find('X')?;
    let rest = &v[xpos + 1..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let ypos = rest.find('Y')?;
    let ydigits: String = rest[ypos + 1..].chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.len() == 2 && ydigits.len() == 2 {
        Some(format!("{}.{}", &digits[..1], &digits[1..]))
    } else {
        None
    }
}

fn gerber_file_function(first: &str) -> LayerRole {
    let f = first.to_ascii_lowercase();
    if f.contains("copper") { LayerRole::Copper }
    else if f.contains("soldermask") || f.contains("solder_mask") { LayerRole::SolderMask }
    else if f.contains("legend") || f.contains("silkscreen") { LayerRole::Legend }
    else if f.contains("plated") || f.contains("nonplated") || f.contains("drill") { LayerRole::Drill }
    else if f.contains("paste") { LayerRole::Paste }
    else if f.contains("profile") || f.contains("outline") { LayerRole::Outline }
    else { LayerRole::Unknown }
}

fn parse_aperture(v: &str, datum: &CoordDatum, ln: u32, warnings: &mut Vec<Diagnostic>) -> Option<ApertureDef> {
    // D10C,0.25mm or D11R,1.5X0.8mm or D12O,0.05X0.1IN
    let body = v.trim_end_matches('*');
    let (code_part, rest) = body.split_at(body.find(|c: char| !c.is_ascii_digit()).unwrap_or(body.len()));
    let code = format!("D{code_part}");
    let mut chars = rest.chars();
    let prim_ch = chars.next()?;
    let primitive = match prim_ch {
        'C' => "circle",
        'R' => "rect",
        'O' => "obround",
        'P' => "polygon",
        other => {
            warnings.push(warn("aperture", format!("aperture {code}: primitive '{other}' not normalized", ln, ln)));
            "unknown"
        }
    }.to_string();
    let param_str = chars.as_str().trim_start_matches(',').trim_end_matches("in").trim_end_matches("mm").trim();
    let params_raw: Vec<String> = param_str.split('X').map(|s| s.to_string()).collect();
    let mut params_nm = Vec::new();
    for p in &params_raw {
        let nm = parse_measure(p, datum.unit_nm.max(MM_NM));
        if nm.is_none() && !p.is_empty() {
            warnings.push(warn("aperture", format!("aperture {code}: parameter '{p}' unreadable", ln, ln)));
        }
        params_nm.push(nm);
    }
    Some(ApertureDef { code, primitive, params_raw, params_nm })
}

fn parse_sr_decl(body: &str, datum: &CoordDatum, ln: u32, warnings: &mut Vec<Diagnostic>) -> Option<StepRepeat> {
    // SR3X2I2.5J1.5*
    let s = body.trim_start_matches("SR").trim_end_matches('*');
    let mut nx = 1u32;
    let mut ny = 1u32;
    let mut dx_raw = String::from("0");
    let mut dy_raw = String::from("0");
    for part in s.split(['X', 'I', 'J']) {
        let _ = part;
    }
    // tokenize explicitly
    let mut num = String::new();
    let mut fields: Vec<(char, String)> = Vec::new();
    for c in s.chars() {
        if c == 'X' || c == 'Y' || c == 'I' || c == 'J' {
            if !num.is_empty() {
                fields.push((c, std::mem::take(&mut num)));
            } else {
                fields.push((c, String::new()));
            }
        } else {
            num.push(c);
        }
    }
    if !num.is_empty() {
        warnings.push(warn("steprepeat", format!("trailing SR data '{num}'"), ln, ln));
    }
    for (k, val) in fields {
        let val = val.trim_end_matches("mm").trim_end_matches("in").to_string();
        match k {
            'X' => nx = val.parse().unwrap_or(1),
            'Y' => ny = val.parse().unwrap_or(1),
            'I' => dx_raw = val,
            'J' => dy_raw = val,
            _ => {}
        }
    }
    let dx_nm = parse_measure(&dx_raw, datum.unit_nm.max(MM_NM));
    let dy_nm = parse_measure(&dy_raw, datum.unit_nm.max(MM_NM));
    Some(StepRepeat { name: String::new(), nx, ny, dx_nm, dy_nm, dx_raw, dy_raw })
}

fn split_coord(line: &str) -> (Option<String>, Option<String>, Option<String>) {
    let s = line.trim_end_matches('*');
    let mut x = None;
    let mut y = None;
    let mut d = None;
    let mut cur = String::new();
    let mut tag: Option<char> = None;
    for c in s.chars() {
        if c == 'X' || c == 'Y' || c == 'I' || c == 'J' {
            if let Some(t) = tag {
                match t { 'X' => x = Some(std::mem::take(&mut cur)), 'Y' => y = Some(std::mem::take(&mut cur)), _ => {} }
            }
            tag = Some(c);
            cur.clear();
        } else if c == 'D' {
            if let Some(t) = tag {
                match t { 'X' => x = Some(std::mem::take(&mut cur)), 'Y' => y = Some(std::mem::take(&mut cur)), _ => {} }
            }
            tag = Some('D');
            cur.clear();
        } else {
            cur.push(c);
        }
    }
    if let Some(t) = tag {
        match t {
            'X' => x = Some(cur),
            'Y' => y = Some(cur),
            'D' => d = Some(cur),
            _ => {}
        }
    }
    (x, y, d)
}

// ---------------------------------------------------------------------------
// Excellon subset
// ---------------------------------------------------------------------------

pub fn parse_excellon(text: &str) -> ExcellonDoc {
    let mut datum = CoordDatum {
        unit: "inch".into(),
        format: "2.4".into(),
        zero_suppression: "T".into(),
        notation: "A".into(),
        unit_nm: INCH_NM,
    };
    let mut tools: Vec<DrillTool> = Vec::new();
    let mut hits: Vec<DrillHit> = Vec::new();
    let mut warnings: Vec<Diagnostic> = Vec::new();
    let mut span = LayerSpan::default();
    let mut current_tool = String::new();
    let mut seq: u32 = 0;

    for (idx, raw_line) in text.lines().enumerate() {
        let ln = (idx + 1) as u32;
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let upper = line.to_ascii_uppercase();

        if line.starts_with(';') || upper.starts_with("COMMENT") {
            // Comment directives may declare the layer span for blind/buried.
            if let Some(s) = parse_span_comment(line) {
                span = s;
            }
            continue;
        }
        if upper == "M48" || upper == "%" || upper == "M95" || upper == "M30" {
            continue;
        }
        if upper.starts_with("METRIC") {
            datum.unit = "mm".into();
            datum.unit_nm = MM_NM;
            if !upper.contains("TZ") && !upper.contains("LZ") {
                // retain format assumption but mark explicitly
            }
            if upper.contains("TZ") { datum.zero_suppression = "T".into(); }
            if upper.contains("LZ") { datum.zero_suppression = "L".into(); }
            continue;
        }
        if upper.starts_with("INCH") {
            datum.unit = "inch".into();
            datum.unit_nm = INCH_NM;
            if upper.contains("TZ") { datum.zero_suppression = "T".into(); }
            if upper.contains("LZ") { datum.zero_suppression = "L".into(); }
            continue;
        }
        if upper.starts_with("FORMAT,") {
            let f = upper.trim_start_matches("FORMAT,");
            let f = f.trim_end_matches([';', '*']);
            if let Some((a, b)) = f.split_once('.') {
                if a.len() == 1 && b.len() == 1 {
                    datum.format = format!("{a}.{b}");
                } else {
                    warnings.push(warn("excellon-format", format!("malformed FORMAT '{f}'"), ln, ln));
                }
            }
            continue;
        }
        if upper.starts_with("T") && upper.contains('C') {
            // T1C0.012  or  T2C0.3PLATED
            if let Some(tool) = parse_tool(line, &datum, ln, &mut warnings) {
                tools.push(tool);
            }
            continue;
        }
        if upper.starts_with('T') {
            // tool select T1
            current_tool = line.split(['*', ';']).next().unwrap_or("").to_string();
            if !tools.iter().any(|t| t.code == current_tool) {
                warnings.push(warn("excellon-tool", format!("tool {current_tool} selected but never defined"), ln, ln));
            }
            continue;
        }
        if upper.contains('X') && upper.contains('Y') {
            seq += 1;
            let (xs, ys, _) = split_coord(line);
            let xraw = xs.unwrap_or_default();
            let yraw = ys.unwrap_or_default();
            let x = raw_num(xraw.clone(), &datum);
            let y = raw_num(yraw.clone(), &datum);
            if x.value_nm.is_none() || y.value_nm.is_none() {
                warnings.push(err("excellon-coord",
                    format!("drill coordinate '{line}' not convertible under {}/{} zero={}; kept raw", datum.unit, datum.format, datum.zero_suppression),
                    ln, ln));
            }
            hits.push(DrillHit { seq, tool: current_tool.clone(), pos: Position { x, y }, source_lines: (ln, ln) });
            continue;
        }
        if upper.starts_with('%') {
            continue;
        }
        warnings.push(warn("excellon-line", format!("unrecognized: {line}"), ln, ln));
    }

    if span.raw.is_empty() {
        span.note = Some("through-all: no Lx-Ly directive found".into());
    }
    ExcellonDoc { datum, tools, hits, span, warnings }
}

fn parse_tool(line: &str, datum: &CoordDatum, ln: u32, warnings: &mut Vec<Diagnostic>) -> Option<DrillTool> {
    let up = line.to_ascii_uppercase();
    let cpos = up.find('C')?;
    let code = line[..cpos].to_string();
    let after = &line[cpos + 1..];
    let diameter_raw: String = after.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    let diameter_nm = parse_measure(&diameter_raw, datum.unit_nm);
    if diameter_nm.is_none() {
        warnings.push(warn("excellon-tool", format!("{code}: diameter '{diameter_raw}' unreadable"), ln, ln));
    }
    let plated = if up.contains("NONPLATED") || up.contains("NPTH") {
        Some(false)
    } else if up.contains("PLATED") || up.contains("PTH") {
        Some(true)
    } else {
        None
    };
    Some(DrillTool { code, diameter_raw, diameter_nm, plated })
}

fn parse_span_comment(line: &str) -> Option<LayerSpan> {
    // ; LAYER_SPAN L2-L4   or  ; BLIND L1-L3
    let up = line.to_ascii_uppercase();
    let idx = up.find('L')?;
    let rest = &line[idx..];
    let (a, b) = rest.split_once('-')?;
    let parse_l = |s: &str| -> Option<u32> {
        let d: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
        d.parse().ok().map(|n: u32| n.saturating_sub(1))
    };
    let start = parse_l(a);
    let end = parse_l(b);
    let raw = format!("{}-{}", a.trim(), b.trim());
    let note = if start.is_none() || end.is_none() {
        Some("span declared but layer indices unreadable".into())
    } else {
        None
    };
    Some(LayerSpan { start, end, raw, note })
}

// ---------------------------------------------------------------------------
// Stackup + manual layer mapping (simple, explicit offline format)
// ---------------------------------------------------------------------------

pub fn parse_stackup(text: &str) -> StackupDoc {
    let mut layers = Vec::new();
    let mut warnings = Vec::new();
    for (idx, raw_line) in text.lines().enumerate() {
        let ln = (idx + 1) as u32;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // LAYER 0 TopCu copper 0.035mm
        if let Some(rest) = line.to_ascii_uppercase().strip_prefix("LAYER ") {
            let mut it = line.split_whitespace();
            it.next(); // LAYER
            let index_tok = it.next().unwrap_or("");
            let index: u32 = match index_tok.parse() {
                Ok(n) => n,
                Err(_) => {
                    warnings.push(err("stackup", format!("layer index '{index_tok}' unreadable; layer kept but unresolved"), ln, ln));
                    continue;
                }
            };
            let name = it.next().unwrap_or("?").to_string();
            let kind = it.next().unwrap_or("unknown").to_string();
            let thickness_raw = it.next().unwrap_or("").to_string();
            let up = thickness_raw.to_ascii_uppercase();
            let unit = if up.ends_with("MM") { MM_NM } else if up.ends_with("IN") || up.ends_with("MIL") { INCH_NM } else { 0 };
            let thickness_nm = if unit != 0 {
                let num = thickness_raw.trim_end_matches(|c: char| c.is_ascii_alphabetic());
                parse_measure(num, unit)
            } else {
                None
            };
            if thickness_raw.is_empty() {
                warnings.push(warn("stackup", format!("layer {index} missing thickness"), ln, ln));
            }
            layers.push(StackLayer { index, name, kind, thickness_raw, thickness_nm });
            let _ = rest;
        } else {
            warnings.push(warn("stackup", format!("unrecognized line: {line}"), ln, ln));
        }
    }
    StackupDoc { layers, warnings }
}

/// Manual mapping file lines:  MAP <filename> L<index>
/// Lines without an index are preserved as unmapped declarations.
pub fn parse_layer_mapping(text: &str) -> Vec<LayerMapping> {
    let mut out = Vec::new();
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let up = line.to_ascii_uppercase();
        if up.starts_with("MAP ") {
            let mut physical_index = None;
            let toks: Vec<&str> = line.split_whitespace().collect();
            let mut filename = String::new();
            if toks.len() >= 2 {
                filename = toks[1].to_string();
                for t in &toks[2..] {
                    let tu = t.to_ascii_uppercase();
                    if let Some(n) = tu.strip_prefix('L').and_then(|s| s.parse::<u32>().ok()) {
                        physical_index = Some(n.saturating_sub(1));
                    }
                }
            }
            out.push(LayerMapping { material_id: String::new(), filename, physical_index, raw_declaration: line.to_string() });
        }
    }
    out
}
