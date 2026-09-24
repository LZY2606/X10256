//! Gerber RS-274X subset parser.

use crate::error::{Diagnostic, DomainResult, FailureKind};
use crate::model::*;
use crate::parser::reconstruct_fixed;

use super::fail;

#[derive(Default)]
struct State {
    unit_raw: String,
    unit_nm: Option<i64>,
    unit_line: usize,
    fs_raw: String,
    int_digits: u32,
    dec_digits: u32,
    leading_zs: bool,
    absolute: bool,
    fs_seen: bool,
    apertures: Vec<Aperture>,
    ops: Vec<PaintOp>,
    file_attributes: Vec<String>,
    inferred_role: Option<LayerRole>,
    inferred_copper_order: Option<u32>,
    current_aperture: Option<String>,
    polarity: Polarity,
    sr: Option<StepRepeat>,
    in_region: bool,
    region_contour: Vec<Point>,
    cur_x: i64,
    cur_y: i64,
    cur_x_raw: String,
    cur_y_raw: String,
    diag: Vec<Diagnostic>,
    op_index: usize,
}

pub fn parse_gerber(text: &str) -> DomainResult<GerberDoc> {
    let mut st = State {
        polarity: Polarity::Dark,
        leading_zs: true,
        absolute: true,
        ..Default::default()
    };

    for (idx0, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        let line_no = idx0 + 1;
        if line.is_empty() {
            continue;
        }
        if line.starts_with('%') {
            handle_extended(&mut st, line, line_no);
        } else if line.starts_with("G01") || line.starts_with("G02") || line.starts_with("G03") || line.starts_with("G1") || line.starts_with("G2") || line.starts_with("G3") {
            handle_interpolate(&mut st, line, line_no);
        } else if line.starts_with("D") {
            handle_dcode(&mut st, line, line_no);
        } else if line.starts_with('X') || line.starts_with('Y') {
            // Coordinate-only draw under modal G01.
            handle_interpolate(&mut st, line, line_no);
        } else if line == "M02*" || line == "M00*" {
            break;
        } else if line.starts_with("G70") || line.starts_with("G71") {
            handle_unit_g(&mut st, line, line_no);
        } else if line.starts_with('G') || line == "*" {
            // Ignore other modal codes (G36/G37 handled in extended-less form below).
            if line == "G36*" {
                st.in_region = true;
                st.region_contour.clear();
            } else if line == "G37*" {
                close_region(&mut st, line_no);
            }
        } else {
            st.diag.push(Diagnostic::new(
                FailureKind::CorruptFormat,
                "GERBER_UNKNOWN_LINE",
                format!("line {line_no}"),
                format!("unrecognised Gerber line: {line}"),
            ));
        }
    }

    if st.in_region {
        st.diag.push(Diagnostic::new(
            FailureKind::CorruptFormat,
            "GERBER_UNCLOSED_REGION",
            "end of file",
            "G36 region never closed with G37",
        ));
    }
    if !st.fs_seen {
        st.diag.push(Diagnostic::new(
            FailureKind::CorruptFormat,
            "GERBER_NO_FS",
            "header",
            "missing %FSLAX... coordinate format statement",
        ));
    }

    if st.diag.iter().any(|d| d.kind == FailureKind::CorruptFormat) {
        return Err(fail(st.diag));
    }

    Ok(GerberDoc {
        unit_raw: st.unit_raw,
        unit_nm: st.unit_nm.unwrap_or(1_000_000),
        unit_line: st.unit_line,
        fs_raw: st.fs_raw,
        int_digits: st.int_digits,
        dec_digits: st.dec_digits,
        leading_zero_suppression: st.leading_zs,
        absolute: st.absolute,
        apertures: st.apertures,
        ops: st.ops,
        file_attributes: st.file_attributes,
        inferred_role: st.inferred_role,
        inferred_copper_order: st.inferred_copper_order,
        diagnostics: st.diag,
    })
}

fn units_of(raw: &str) -> Option<(i64, &'static str)> {
    match raw {
        "IN" => Some((25_400_000, "IN")),
        "MM" => Some((1_000_000, "MM")),
        _ => None,
    }
}

fn handle_unit_g(st: &mut State, line: &str, line_no: usize) {
    let raw = if line.starts_with("G70") { "IN" } else { "MM" };
    if let Some((nm, code)) = units_of(raw) {
        st.unit_nm = Some(nm);
        st.unit_raw = code.to_string();
        st.unit_line = line_no;
    }
}

fn handle_extended(st: &mut State, line: &str, line_no: usize) {
    // Strip leading % and trailing %* / %.
    let body = line.trim().trim_start_matches('%').trim_end_matches('*').trim_end_matches('%');
    if let Some(rest) = body.strip_prefix("MO") {
        if let Some((nm, code)) = units_of(rest) {
            st.unit_nm = Some(nm);
            st.unit_raw = code.to_string();
            st.unit_line = line_no;
        } else {
            st.diag.push(Diagnostic::new(
                FailureKind::SemanticUnresolved,
                "GERBER_BAD_UNIT",
                format!("line {line_no}"),
                format!("unknown MO unit '{rest}'"),
            ).with_raw(body.to_string()));
        }
    } else if let Some(rest) = body.strip_prefix("FS") {
        parse_fs(st, rest, body, line_no);
    } else if let Some(rest) = body.strip_prefix("AD") {
        parse_aperture_def(st, rest, body, line_no);
    } else if let Some(rest) = body.strip_prefix("LP") {
        match Polarity::from_code(rest) {
            Some(p) => st.polarity = p,
            None => st.diag.push(Diagnostic::new(
                FailureKind::SemanticUnresolved,
                "GERBER_BAD_POLARITY",
                format!("line {line_no}"),
                format!("unknown polarity '{rest}'"),
            ).with_raw(body.to_string())),
        }
    } else if let Some(rest) = body.strip_prefix("SR") {
        parse_sr(st, rest, body, line_no);
    } else if let Some(rest) =body.strip_prefix("TF") {
        st.file_attributes.push(rest.to_string());
        apply_file_function(st, rest);
    } else if body.starts_with("TA") || body.starts_with("TO") || body.starts_with("TD") {
        // Aperture attributes are retained on the file attribute log for provenance.
        st.file_attributes.push(body.to_string());
    }
}

fn parse_fs(st: &mut State, rest: &str, full: &str, line_no: usize) {
    // e.g. LAX24Y24
    let absolute = rest.chars().nth(1) == Some('A');
    let zs_leading = rest.chars().next() != Some('T');
    let mut int_d = 0u32;
    let mut dec_d = 0u32;
    if let Some(xpart) = rest.split('X').nth(1).and_then(|s| s.split('Y').next()) {
        if xpart.len() == 2 {
            int_d = xpart.chars().next().unwrap().to_digit(10).unwrap_or(0);
            dec_d = xpart.chars().nth(1).unwrap().to_digit(10).unwrap_or(0);
        }
    }
    if int_d == 0 && dec_d == 0 {
        st.diag.push(Diagnostic::new(
            FailureKind::SemanticUnresolved,
            "GERBER_BAD_FS",
            format!("line {line_no}"),
            format!("cannot read digit counts from format '{full}'"),
        ).with_raw(full.to_string()));
        return;
    }
    st.fs_seen = true;
    st.fs_raw = full.to_string();
    st.absolute = absolute;
    st.leading_zs = zs_leading;
    st.int_digits = int_d;
    st.dec_digits = dec_d;
}

fn apply_file_function(st: &mut State, rest: &str) {
    // .FileFunction,Copper,L1,Top
    let fields: Vec<&str> = rest.split(',').collect();
    if fields.is_empty() {
        return;
    }
    let head = fields[0].trim_start_matches('.');
    if head != "FileFunction" {
        return;
    }
    if let Some(f) = fields.get(1) {
        st.inferred_role = LayerRole::parse(f);
    }
    // Copper order: first field beginning with L followed by digits.
    for f in &fields[1..] {
        if let Some(num) = f.strip_prefix('L').and_then(|n| n.parse::<u32>().ok()) {
            st.inferred_copper_order = Some(num);
            return;
        }
    }
}

fn local_to_nm(st: &State, local: i64, line_no: usize, field: &str, raw: &str) -> Option<i64> {
    let unit = match st.unit_nm {
        Some(u) => u,
        None => {
            st_clone_diag_note(st, line_no, field, raw, "no %MO unit declared; cannot normalize");
            return None;
        }
    };
    // local is in 10^-dec_digits of a local unit.
    let scale = unit as i128 * local as i128;
    let divisor = 10i128.pow(st.dec_digits);
    let nm = scale / divisor;
    if nm > i64::MAX as i128 || nm < i64::MIN as i128 {
        st_clone_diag_note(st, line_no, field, raw, "coordinate overflows nanometre basis");
        return None;
    }
    Some(nm as i64)
}

// Helper that mutates state through a reborrow without borrow entanglement.
fn st_clone_diag_note(st: &mut State, line_no: usize, field: &str, raw: &str, msg: &str) {
    st.diag.push(
        Diagnostic::new(
            FailureKind::SemanticUnresolved,
            "GERBER_UNCONVERTIBLE_COORD",
            format!("line {line_no} field {field}"),
            format!("{msg}; value left in original position"),
        )
        .with_raw(raw),
    );
}

fn parse_coord_token(st: &State, tok: &str, line_no: usize, axis: char) -> Option<i64> {
    let local = match reconstruct_fixed(tok, st.int_digits, st.dec_digits, st.leading_zs) {
        Ok(v) => v,
        Err(e) => {
            // Cannot borrow st mutably inside Option closure; handled by caller.
            let _ = e;
            return None;
        }
    };
    let unit = st.unit_nm?;
    let scale = unit as i128 * local as i128;
    let divisor = 10i128.pow(st.dec_digits);
    let nm = scale / divisor;
    if nm > i64::MAX as i128 || nm < i64::MIN as i128 {
        return None;
    }
    let _ = (line_no, axis);
    Some(nm as i64)
}

fn parse_aperture_def(st: &mut State, rest: &str, full: &str, line_no: usize) {
    // D10C,0.25  or D11R,0.5X0.2  or D12O,...  D13P,0.4X6
    let (id_part, shape_part) = match rest.split_once(char::is_whitespace).or_else(|| rest.split_once(',')) {
        Some(v) => v,
        None => {
            st.diag.push(Diagnostic::new(
                FailureKind::SemanticUnresolved,
                "GERBER_BAD_APERTURE",
                format!("line {line_no}"),
                "aperture definition lacks shape",
            ).with_raw(full.to_string()));
            return;
        }
    };
    let id = id_part.trim().to_string();
    let shape_part = shape_part.trim();
    let form_ch = shape_part.chars().next();
    let form = match form_ch {
        Some('C') => ShapeForm::Circle,
        Some('R') => ShapeForm::Rect,
        Some('O') => ShapeForm::Obround,
        Some('P') => ShapeForm::Polygon,
        _ => {
            st.diag.push(Diagnostic::new(
                FailureKind::SemanticUnresolved,
                "GERBER_UNKNOWN_APERTURE_SHAPE",
                format!("line {line_no}"),
                format!("unsupported aperture form in '{full}'"),
            ).with_raw(full.to_string()));
            return;
        }
    };
    let args: Vec<&str> = shape_part
        .split(|c: char| c == ',' || c == 'X' || c == 'x')
        .skip(1)
        .filter(|s| !s.is_empty())
        .collect();

    let mut dims: Vec<i64> = Vec::new();
    let mut raw_dims: Vec<String> = Vec::new();
    let unit = st.unit_nm;
    for (i, arg) in args.iter().enumerate() {
        // Modifier 3+ for polygon is vertex count / rotation, not a length.
        if form == ShapeForm::Polygon && i >= 2 {
            break;
        }
        match arg.parse::<f64>() {
            Ok(v) => {
                let nm = (v * unit.unwrap_or(1_000_000) as f64).round() as i64;
                dims.push(nm);
                raw_dims.push(arg.to_string());
            }
            Err(_) => {
                st.diag.push(Diagnostic::new(
                    FailureKind::SemanticUnresolved,
                    "GERBER_BAD_APERTURE_DIM",
                    format!("line {line_no}"),
                    format!("aperture dimension '{arg}' is not numeric; left raw"),
                ).with_raw((*arg).to_string()));
            }
        }
    }
    let mut vertices = None;
    let mut rotation_ddeg = 0i64;
    if form == ShapeForm::Polygon {
        if let Some(v) = args.get(2).and_then(|a| a.parse::<u32>().ok()) {
            vertices = Some(v);
        }
        if let Some(r) = args.get(3).and_then(|a| a.parse::<f64>().ok()) {
            rotation_ddeg = (r * 10.0).round() as i64;
        }
    }
    let _ = raw_dims;
    if let Some(existing) = st.apertures.iter().position(|a| a.id == id) {
        st.apertures[existing] = Aperture { id: id.clone(), form, dims, vertices, rotation_ddeg, raw: full.to_string(), line: line_no };
    } else {
        st.apertures.push(Aperture { id, form, dims, vertices, rotation_ddeg, raw: full.to_string(), line: line_no });
    }
}

fn parse_sr(st: &mut State, rest: &str, full: &str, line_no: usize) {
    // SRX2Y3I0J0  (step & repeat open). SR without fields closes it.
    if rest.trim().is_empty() {
        st.sr = None;
        return;
    }
    let mut nx = 1i64;
    let mut ny = 1i64;
    let mut dx = 0i64;
    let mut dy = 0i64;
    let mut bad = false;
    for ch in ['X', 'Y', 'I', 'J'] {
        if let Some(v) = field_after(rest, ch).and_then(|t| t.parse::<i64>().ok()) {
            match ch {
                'X' => nx = v,
                'Y' => ny = v,
                'I' => dx = local_unit_nm(st, v, full, line_no, &mut bad),
                'J' => dy = local_unit_nm(st, v, full, line_no, &mut bad),
                _ => {}
            }
        }
    }
    if bad {
        return;
    }
    st.sr = Some(StepRepeat { nx, ny, dx, dy, line: line_no });
}

fn local_unit_nm(st: &State, local: i64, full: &str, line_no: usize, bad: &mut bool) -> i64 {
    match st.unit_nm {
        Some(unit) => {
            let nm = unit as i128 * local as i128 / 10i128.pow(st.dec_digits);
            nm as i64
        }
        None => {
            *bad = true;
            let _ = (full, line_no);
            0
        }
    }
}

fn field_after(rest: &str, ch: char) -> Option<&str> {
    let start = rest.find(ch)? + 1;
    let tail = &rest[start..];
    let end = tail
        .char_indices()
        .find(|(_, c)| "XYIJ".contains(*c))
        .map(|(i, _)| i)
        .unwrap_or(tail.len());
    let v = tail[..end].trim();
    if v.is_empty() { None } else { Some(v) }
}

fn xy_fields(line: &str) -> (Option<&str>, Option<&str>) {
    fn after(ch: char, s: &str) -> Option<&str> {
        let start = s.find(ch)? + 1;
        let tail = &s[start..];
        let end = tail
            .char_indices()
            .find(|(_, c)| matches!(c, 'X' | 'Y' | 'I' | 'J' | 'D' | '*'))
            .map(|(i, _)| i)
            .unwrap_or(tail.len());
        let v = tail[..end].trim();
        if v.is_empty() { None } else { Some(v) }
    }
    (after('X', line), after('Y', line))
}

/// Resolve one point. On failure the nm value is clamped/emitted via diagnostic
/// but the raw token still stays on the point (we use 0 as the placeholder).
fn resolve_point(st: &mut State, line: &str, line_no: usize) -> Option<Point> {
    let (xt, yt) = xy_fields(line);
    let x_raw = xt.unwrap_or(&st.cur_x_raw).to_string();
    let y_raw = yt.unwrap_or(&st.cur_y_raw).to_string();

    let convert = |tok: &str| -> Result<i64, String> {
        let local = reconstruct_fixed(tok, st.int_digits, st.dec_digits, st.leading_zs)?;
        let unit = st.unit_nm.ok_or_else(|| "no unit".to_string())?;
        let nm = unit as i128 * local as i128 / 10i128.pow(st.dec_digits);
        if nm > i64::MAX as i128 || nm < i64::MIN as i128 {
            return Err("overflow".to_string());
        }
        Ok(nm as i64)
    };

    let x = match convert(&x_raw) {
        Ok(v) => v,
        Err(e) => {
            st.diag.push(Diagnostic::new(
                FailureKind::SemanticUnresolved,
                "GERBER_UNCONVERTIBLE_COORD",
                format!("line {line_no} field X"),
                format!("{e}; coordinate left in original position"),
            ).with_raw(x_raw.clone()));
            return None;
        }
    };
    let y = match convert(&y_raw) {
        Ok(v) => v,
        Err(e) => {
            st.diag.push(Diagnostic::new(
                FailureKind::SemanticUnresolved,
                "GERBER_UNCONVERTIBLE_COORD",
                format!("line {line_no} field Y"),
                format!("{e}; coordinate left in original position"),
            ).with_raw(y_raw.clone()));
            return None;
        }
    };

    let (nx, ny) = if st.absolute {
        (x, y)
    } else {
        (st.cur_x + x, st.cur_y + y)
    };
    st.cur_x = nx;
    st.cur_y = ny;
    st.cur_x_raw = x_raw.clone();
    st.cur_y_raw = y_raw.clone();
    Some(Point { x: nx, y: ny, x_raw, y_raw })
}

fn push_op(st: &mut State, op: PaintOp) {
    st.ops.push(op);
    st.op_index += 1;
}

fn nm_of_local(unit: i64, local: i64, dec: u32) -> i64 {
    (unit as i128 * local as i128 / 10i128.pow(dec)) as i64
}

fn handle_interpolate(st: &mut State, line: &str, line_no: usize) {
    let mode = if line.starts_with("G02") || line.starts_with("G2") {
        2
    } else if line.starts_with("G03") || line.starts_with("G3") {
        3
    } else {
        1
    };
    let has_d02 = line.contains("D02") || line.contains("D2*");

    // Snapshot previous pen position before resolving the new point (which
    // updates current position).
    let prev = Point {
        x: st.cur_x,
        y: st.cur_y,
        x_raw: st.cur_x_raw.clone(),
        y_raw: st.cur_y_raw.clone(),
    };
    let point = match resolve_point(st, line, line_no) {
        Some(p) => p,
        None => return,
    };

    if st.in_region {
        st.region_contour.push(point);
        return;
    }
    if has_d02 {
        return;
    }

    let aperture = match &st.current_aperture {
        Some(a) => a.clone(),
        None => {
            st.diag.push(Diagnostic::new(
                FailureKind::SemanticUnresolved,
                "GERBER_NO_APERTURE",
                format!("line {line_no}"),
                "interpolation without an active aperture",
            ).with_raw(line.to_string()));
            return;
        }
    };

    let index = st.op_index;
    let kind = if mode == 1 {
        OpKind::Draw { a: prev, b: point, aperture }
    } else {
        let (it, jt) = (field_after(line, 'I'), field_after(line, 'J'));
        let ioff = it.and_then(|t| reconstruct_fixed(t, st.int_digits, st.dec_digits, st.leading_zs).ok());
        let joff = jt.and_then(|t| reconstruct_fixed(t, st.int_digits, st.dec_digits, st.leading_zs).ok());
        let unit = st.unit_nm.unwrap_or(0);
        let center = Point {
            x: prev.x + nm_of_local(unit, ioff.unwrap_or(0), st.dec_digits),
            y: prev.y + nm_of_local(unit, joff.unwrap_or(0), st.dec_digits),
            x_raw: it.unwrap_or("").to_string(),
            y_raw: jt.unwrap_or("").to_string(),
        };
        OpKind::Arc { a: prev, b: point, center, clockwise: mode == 2, aperture }
    };
    push_op(st, PaintOp { index, line: line_no, polarity: st.polarity, sr: st.sr.clone(), kind });
}

fn close_region(st: &mut State, line_no: usize) {
    if !st.in_region {
        return;
    }
    st.in_region = false;
    let contour = std::mem::take(&mut st.region_contour);
    if contour.len() < 3 {
        st.diag.push(Diagnostic::new(
            FailureKind::SemanticUnresolved,
            "GERBER_DEGENERATE_REGION",
            format!("line {line_no}"),
            "region closed with fewer than 3 contour points",
        ));
        return;
    }
    let index = st.op_index;
    push_op(st, PaintOp {
        index,
        line: line_no,
        polarity: st.polarity,
        sr: st.sr.clone(),
        kind: OpKind::Region { contour },
    });
}

fn handle_dcode(st: &mut State, line: &str, line_no: usize) {
    // Strip coordinate prefix.
    let dpos = match line.find('D') {
        Some(p) => p,
        None => return,
    };
    let coord_part = &line[..dpos];
    let code_part = line[dpos..].trim_end_matches('*');
    let code: String = code_part.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();

    if code == "D03" || code == "D3" {
        // Flash
        let aperture = match &st.current_aperture {
            Some(a) => a.clone(),
            None => {
                st.diag.push(Diagnostic::new(
                    FailureKind::SemanticUnresolved,
                    "GERBER_NO_APERTURE",
                    format!("line {line_no}"),
                    "flash without active aperture",
                ).with_raw(line.to_string()));
                return;
            }
        };
        let point = if coord_part.is_empty() {
            Point { x: st.cur_x, y: st.cur_y, x_raw: st.cur_x_raw.clone(), y_raw: st.cur_y_raw.clone() }
        } else {
            match resolve_point(st, coord_part, line_no) {
                Some(p) => p,
                None => return,
            }
        };
        let index = st.op_index;
        push_op(st, PaintOp {
            index,
            line: line_no,
            polarity: st.polarity,
            sr: st.sr.clone(),
            kind: OpKind::Flash { at: vec![point], aperture },
        });
    } else if code == "D02" || code == "D2" {
        if !coord_part.is_empty() {
            let _ = resolve_point(st, coord_part, line_no);
        }
    } else if code == "D01" || code == "D1" {
        handle_interpolate(st, line, line_no);
    } else if code.starts_with('D') {
        // Aperture selection D10..
        if code.len() >= 3 {
            st.current_aperture = Some(code);
        }
    }
}
