//! Strict-but-small Gerber X2 subset parser.
//!
//! Preserves coordinate format, unit, polarity, apertures and step/repeat
//! blocks in source order. Unsupported constructs (arcs, aperture macros,
//! rotation, incremental coordinates) become positional diagnostics instead
//! of being dropped.

use crate::domain::*;

const NM_PER_IN: i64 = 25_400_000;
const NM_PER_MM: i64 = 1_000_000;

pub fn convert_length(raw: &str, unit: Unit, frac: u8) -> Option<i64> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let (sign, digits) = match raw.strip_prefix('-') {
        Some(rest) => (-1i64, rest),
        None => (1i64, raw.strip_prefix('+').unwrap_or(raw)),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let total = digits.parse::<i64>().ok()?;
    let scale = if unit == Unit::In { NM_PER_IN } else { NM_PER_MM };
    let denom = 10i64.pow(frac as u32);
    Some(sign * total * scale / denom)
}

fn mk(code: &str, sev: Severity, line: usize, col: usize, msg: &str, raw: &str) -> Diagnostic {
    Diagnostic {
        code: code.to_string(),
        severity: sev,
        line,
        col,
        message: msg.to_string(),
        raw: raw.to_string(),
    }
}
fn err(code: &str, line: usize, col: usize, msg: &str, raw: &str) -> Diagnostic {
    mk(code, Severity::Error, line, col, msg, raw)
}

#[derive(Default)]
struct PState {
    format: Option<CoordFormat>,
    unit: Option<Unit>,
    format_line: usize,
    unit_line: usize,
    apertures: Vec<Aperture>,
    blocks: Vec<GerberBlock>,
    attrs: Vec<(String, String)>,
    diags: Vec<Diagnostic>,
    cur_pol: Polarity,
    cur_sr: Option<StepRepeat>,
    cur_x: Option<i64>,
    cur_y: Option<i64>,
    started: bool,
    sr_seq: usize,
    cur_ap: String,
}

pub fn parse(source_id: &str, content: &str) -> GerberDoc {
    let mut st = PState::default();
    st.cur_pol = Polarity::Dark;

    for (idx, raw_line) in content.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("G04") {
            continue;
        }
        if line.starts_with('%') {
            if let Some(body) = line.strip_prefix('%').and_then(|l| l.strip_suffix('%')) {
                for piece in body.split('*').filter(|p| !p.is_empty()) {
                    extended(&mut st, piece.trim(), line_no, raw_line);
                }
            } else {
                st.diags.push(err("GERBER_CMD_UNCLOSED", line_no, 1,
                    "extended command missing closing %", raw_line));
            }
        } else {
            plot_word(&mut st, line, line_no, raw_line);
        }
    }

    GerberDoc {
        source_id: source_id.to_string(),
        format: st.format,
        format_line: st.format_line,
        unit_line: st.unit_line,
        apertures: st.apertures,
        blocks: st.blocks,
        file_attributes: st.attrs,
        diagnostics: st.diags,
    }
}

fn unit_factor(unit: Unit) -> i64 {
    if unit == Unit::In { NM_PER_IN } else { NM_PER_MM }
}

fn aperture_size(raw: &str, unit: Option<Unit>, line_no: usize, diags: &mut Vec<Diagnostic>) -> i64 {
    match (raw.parse::<f64>(), unit) {
        (Ok(v), Some(u)) => (v * unit_factor(u) as f64) as i64,
        (Ok(_), None) => {
            diags.push(err("GERBER_APERTURE_NO_UNIT", line_no, 1,
                "aperture dimension before MO; kept raw, not normalised", raw));
            0
        }
        (Err(_), _) => {
            diags.push(err("GERBER_APERTURE_BAD_DIM", line_no, 1,
                "aperture dimension not numeric; kept raw", raw));
            0
        }
    }
}

fn extended(st: &mut PState, body: &str, line_no: usize, raw_line: &str) {
    if let Some(rest) = body.strip_prefix("FS") {
        // L A X n n Y n n
        let bytes = rest.as_bytes();
        if bytes.len() < 8 || bytes[0] != b'L' || bytes[1] != b'A' {
            st.diags.push(err("GERBER_FS_UNSUPPORTED", line_no, 3,
                "only leading/trailing zero omission and absolute coords supported", body));
            return;
        }
        let zero = match bytes[0] {
            b'L' => ZeroSuppression::Leading,
            b'T' => ZeroSuppression::Trailing,
            b'D' => ZeroSuppression::None,
            _ => ZeroSuppression::Leading,
        };
        let xi = (bytes[3] - b'0') as u8;
        let xf = (bytes[4] - b'0') as u8;
        let yi = (bytes[6] - b'0') as u8;
        let yf = (bytes[7] - b'0') as u8;
        let unit = st.unit.unwrap_or(Unit::Mm);
        st.format = Some(CoordFormat { unit, zero, x_int: xi, x_frac: xf, y_int: yi, y_frac: yf });
        st.format_line = line_no;
    } else if let Some(rest) = body.strip_prefix("MO") {
        match Unit::parse(rest) {
            Some(u) => {
                st.unit = Some(u);
                st.unit_line = line_no;
                if let Some(f) = st.format.as_mut() {
                    f.unit = u;
                }
            }
            None => st.diags.push(err("GERBER_MO_UNKNOWN", line_no, 3,
                "MO unit must be IN or MM", body)),
        }
    } else if let Some(rest) = body.strip_prefix("AD") {
        add_aperture(st, rest, line_no);
    } else if let Some(rest) = body.strip_prefix("LP") {
        let pol = match rest.chars().next() {
            Some('C') => Polarity::Clear,
            Some('D') => Polarity::Dark,
            _ => {
                st.diags.push(err("GERBER_LP_BAD", line_no, 3, "LP must be C or D", body));
                return;
            }
        };
        st.cur_pol = pol;
        st.cur_sr = None;
    } else if let Some(rest) = body.strip_prefix("SR") {
        // X..Y..I..J..  (counts X/Y, step distances I/J in Gerber 2023)
        let mut xc = 1i64;
        let mut yc = 1i64;
        let mut dx_raw = String::from("0");
        let mut dy_raw = String::from("0");
        let mut dx_nm = 0i64;
        let mut dy_nm = 0i64;
        field_parse(rest, &mut xc, &mut yc, &mut dx_raw, &mut dy_raw,
                    st.unit, &mut dx_nm, &mut dy_nm, line_no, &mut st.diags);
        st.sr_seq += 1;
        st.cur_sr = Some(StepRepeat {
            id: format!("sr{}", st.sr_seq),
            x_count: xc,
            y_count: yc,
            dx_raw,
            dy_raw,
            dx_nm,
            dy_nm,
            line: line_no,
        });
    } else if body.starts_with("TF") || body.starts_with("TA") {
        let rest = body.trim_start_matches(['T', 'F', 'A']);
        let (name, value) = match rest.split_once(',') {
            Some((n, v)) => (n.to_string(), v.to_string()),
            None => (rest.to_string(), String::new()),
        };
        st.attrs.push((name.trim().to_string(), value.trim().to_string()));
    } else if body.starts_with("TD") {
        // object attribute, value retained
        let rest = &body[2..];
        st.attrs.push(("TD".to_string(), rest.to_string()));
    } else if body == "LPD" || body == "LPC" {
        // handled above via LP prefix
    } else {
        st.diags.push(err("GERBER_CMD_UNKNOWN", line_no, 1,
            "extended command not in supported subset", body));
    }
    let _ = raw_line;
}

fn field_parse(
    rest: &str,
    xc: &mut i64,
    yc: &mut i64,
    dx_raw: &mut String,
    dy_raw: &mut String,
    unit: Option<Unit>,
    dx_nm: &mut i64,
    dy_nm: &mut i64,
    line_no: usize,
    diags: &mut Vec<Diagnostic>,
) {
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let key = bytes[i] as char;
        i += 1;
        let start = i;
        while i < bytes.len() && !matches!(bytes[i] as char, 'X' | 'Y' | 'I' | 'J') {
            i += 1;
        }
        let val = rest[start..i].trim();
        match key {
            'X' => *xc = val.parse().unwrap_or(1),
            'Y' => *yc = val.parse().unwrap_or(1),
            'I' => {
                *dx_raw = val.to_string();
                match (val.parse::<f64>(), unit) {
                    (Ok(v), Some(u)) => *dx_nm = (v * unit_factor(u) as f64) as i64,
                    (Ok(_), None) => diags.push(err("GERBER_SR_NO_UNIT", line_no, 1,
                        "SR distance before MO; raw retained", val)),
                    (Err(_), _) => diags.push(err("GERBER_SR_BAD_DIST", line_no, 1,
                        "SR I distance not numeric; raw retained", val)),
                }
            }
            'J' => {
                *dy_raw = val.to_string();
                match (val.parse::<f64>(), unit) {
                    (Ok(v), Some(u)) => *dy_nm = (v * unit_factor(u) as f64) as i64,
                    (Ok(_), None) => diags.push(err("GERBER_SR_NO_UNIT", line_no, 1,
                        "SR distance before MO; raw retained", val)),
                    (Err(_), _) => diags.push(err("GERBER_SR_BAD_DIST", line_no, 1,
                        "SR J distance not numeric; raw retained", val)),
                }
            }
            _ => {}
        }
    }
}

fn add_aperture(st: &mut PState, rest: &str, line_no: usize) {
    // D10C,1.50[ /hole] | D11R,1.2X0.8 | D12O,2.0X0.5
    let (code, tail) = match rest.find(|c: char| c.is_ascii_uppercase()) {
        Some(pos) => (rest[..pos].to_string(), &rest[pos..]),
        None => {
            st.diags.push(err("GERBER_AD_BAD", line_no, 3, "cannot parse aperture def", rest));
            return;
        }
    };
    let (name, mods) = match tail.split_once(',') {
        Some((n, m)) => (n, m),
        None => (tail, ""),
    };
    let modifiers_raw = format!("{},{}", name, mods);
    let (geom_part, hole_part) = match mods.split_once('/') {
        Some((g, h)) => (g, Some(h.trim())),
        None => (mods, None),
    };
    let dims: Vec<&str> = geom_part.split('X').map(|s| s.trim()).collect();
    let hole_nm = hole_part.and_then(|h| {
        h.parse::<f64>().ok().map(|v| match st.unit {
            Some(u) => (v * unit_factor(u) as f64) as i64,
            None => 0,
        })
    });
    let mk_size = |raw: &str| aperture_size(raw, st.unit, line_no, &mut st.diags);
    let shape = match (name.to_ascii_uppercase().as_str(), dims.as_slice()) {
        ("C", [d]) => ApertureShape::Circle { diameter_nm: mk_size(d), hole_nm },
        ("R", [x, y]) => ApertureShape::Rect {
            x_nm: mk_size(x),
            y_nm: mk_size(y),
            hole_nm,
        },
        ("O", [x, y]) => ApertureShape::Obround {
            x_nm: mk_size(x),
            y_nm: mk_size(y),
            hole_nm,
        },
        _ => {
            st.diags.push(err("GERBER_APERTURE_UNSUPPORTED", line_no, 3,
                "only C/R/O apertures (no macros) supported", rest));
            ApertureShape::Circle { diameter_nm: 0, hole_nm: None }
        }
    };
    st.apertures.push(Aperture { code, shape, modifiers_raw, line: line_no });
}

fn cur_block(st: &mut PState) -> &mut GerberBlock {
    if st.blocks.is_empty() || st.blocks.last().unwrap().polarity != st.cur_pol
        || st.blocks.last().unwrap().step_repeat != st.cur_sr
    {
        let index = st.blocks.len();
        st.blocks.push(GerberBlock {
            index,
            polarity: st.cur_pol,
            polarity_line: 0,
            step_repeat: st.cur_sr.clone(),
            flashes: Vec::new(),
        });
    }
    st.blocks.last_mut().unwrap()
}

fn parse_axis(
    st: &mut PState,
    word: &str,
    axis: Axis,
    line_no: usize,
) -> Option<(RawCoord, i64)> {
    let tag = if axis == Axis::X { 'X' } else { 'Y' };
    let pos = word.find(tag)?;
    let bytes = word.as_bytes();
    let mut end = pos + 1;
    while end < bytes.len() && (bytes[end] == b'-' || bytes[end].is_ascii_digit()) {
        end += 1;
    }
    let raw = &word[pos + 1..end];
    let rc = RawCoord { axis, raw: raw.to_string(), line: line_no, col: pos + 1 };
    let fmt = match &st.format {
        Some(f) => f,
        None => {
            st.diags.push(err("GERBER_NO_FS", line_no, pos + 1,
                "coordinate before FS format declaration; raw retained", raw));
            return Some((rc, 0));
        }
    };
    let frac = fmt.frac_digits(axis);
    let expanded = match fmt.zero {
        ZeroSuppression::None => raw.to_string(),
        ZeroSuppression::Leading | ZeroSuppression::Trailing => {
            let digits = raw.trim_start_matches('-').trim_start_matches('+');
            let total = fmt.x_int + fmt.x_frac;
            let _ = total;
            let width = (match axis {
                Axis::X => fmt.x_int + fmt.x_frac,
                Axis::Y => fmt.y_int + fmt.y_frac,
            }) as usize;
            if digits.len() >= width {
                digits.to_string()
            } else if matches!(fmt.zero, ZeroSuppression::Leading) {
                format!("{:0>width$}", digits, width = width)
            } else {
                format!("{:0<width$}", digits, width = width)
            }
        }
    };
    match convert_length(&expanded, fmt.unit, frac) {
        Some(v) => Some((rc, v)),
        None => {
            st.diags.push(err("GERBER_COORD_BAD", line_no, pos + 1,
                "coordinate not convertible; raw retained at source", raw));
            Some((rc, 0))
        }
    }
}

fn plot_word(st: &mut PState, line: &str, line_no: usize, raw_line: &str) {
    let word = line.trim_end_matches('*');
    // Mode / g-code prefixes.
    let body = if let Some(rest) = word.strip_prefix("G04") {
        rest
    } else {
        word
    };
    if body.starts_with("G02") || body.starts_with("G03") {
        st.diags.push(err("GERBER_ARC_UNSUPPORTED", line_no, 1,
            "arcs not reconstructed; plot word kept raw", raw_line));
        return;
    }
    if body.starts_with("G36") || body.starts_with("G37") {
        st.diags.push(err("GERBER_REGION_UNSUPPORTED", line_no, 1,
            "region fills not reconstructed; plot word kept raw", raw_line));
        return;
    }
    if body.contains("G91") {
        st.diags.push(err("GERBER_INCREMENTAL_UNSUPPORTED", line_no, 1,
            "incremental coordinates not supported; word kept raw", raw_line));
        return;
    }
    if body.contains("G70") && st.unit == Some(Unit::Mm) {
        st.diags.push(err("GERBER_UNIT_CONFLICT", line_no, 1,
            "G70 inches conflicts with MO-MM", raw_line));
    }
    if body.contains("G71") && st.unit == Some(Unit::In) {
        st.diags.push(err("GERBER_UNIT_CONFLICT", line_no, 1,
            "G71 mm conflicts with MO-IN", raw_line));
    }

    // Standalone aperture selection: D10 / D11 ...
    if let Some(code) = body.strip_prefix('D') {
        if !body.contains('X') && !body.contains('Y')
            && code.len() >= 2
            && code.chars().all(|c| c.is_ascii_digit())
            && code != "01" && code != "02" && code != "03"
        {
            st.started = true;
            st.cur_ap = format!("D{}", code);
            return;
        }
    }

    let d_code = if body.ends_with("D03") {
        Some((DrawOp::Flash, 3usize))
    } else if body.ends_with("D02") {
        Some((DrawOp::Move, 2))
    } else if body.ends_with("D01") {
        Some((DrawOp::Draw, 1))
    } else if body.ends_with("D00") {
        Some((DrawOp::Move, 0))
    } else {
        None
    };

    let (op, _dc) = d_code.unwrap_or((DrawOp::Flash, 3));
    let xparsed = parse_axis(st, body, Axis::X, line_no);
    let yparsed = parse_axis(st, body, Axis::Y, line_no);
    let xraw = xparsed.as_ref().map(|(r, _)| r.clone());
    let yraw = yparsed.as_ref().map(|(r, _)| r.clone());
    let xnew = xparsed.as_ref().map(|(_, v)| *v);
    let ynew = yparsed.as_ref().map(|(_, v)| *v);
    if xnew.is_none() && ynew.is_none() && !body.starts_with('D') {
        st.diags.push(err("GERBER_WORD_NO_COORDS", line_no, 1,
            "plot word without coordinates; kept raw", raw_line));
    }
    let from_x = st.cur_x;
    let from_y = st.cur_y;
    if let Some(v) = xnew { st.cur_x = Some(v); }
    if let Some(v) = ynew { st.cur_y = Some(v); }

    // Aperture selection embedded before coordinates (G54D12... or D12...).
    let mut aperture = String::new();
    let head_end = body.find(['X', 'Y']).unwrap_or(body.len());
    let head = &body[..head_end];
    if let Some(pos) = head.find('D') {
        let cand = &head[pos + 1..];
        if cand.len() >= 2 && cand.chars().all(|c| c.is_ascii_digit()) {
            aperture = format!("D{}", cand);
        }
    }
    if aperture.is_empty() {
        aperture = st.cur_ap.clone();
    }
    if aperture != st.cur_ap && aperture.len() > 1 {
        st.cur_ap = aperture.clone();
    }

    if op == DrawOp::Move {
        return;
    }
    let block = cur_block(st);
    let sr = block.step_repeat.clone();
    let (count, dx, dy) = match &sr {
        Some(s) => (s.x_count * s.y_count, s.dx_nm, s.dy_nm),
        None => (1i64, 0, 0),
    };
    let sr_id = sr.as_ref().map(|s| s.id.clone());
    for instance in 0..count.max(1) {
        let ix = if count > 1 { instance % sr.as_ref().unwrap().x_count } else { 0 };
        let iy = if count > 1 { instance / sr.as_ref().unwrap().x_count } else { 0 };
        let off_x = if count > 1 { ix * dx } else { 0 };
        let off_y = if count > 1 { iy * dy } else { 0 };
        block.flashes.push(FlashDraw {
            op,
            aperture: aperture.clone(),
            x: xraw.clone(),
            y: yraw.clone(),
            x_nm: xnew.map(|v| v + off_x),
            y_nm: ynew.map(|v| v + off_y),
            from_x_nm: from_x.map(|v| v + off_x),
            from_y_nm: from_y.map(|v| v + off_y),
            line: line_no,
            sr_instance: instance,
            sr_id: sr_id.clone(),
        });
    }
    let _ = st.started;
}
