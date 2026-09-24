//! Excellon drill subset parser.
//!
//! Retains tool diameters with their raw decimal representation, per-tool
//! plating (PTH/NPTH via the `;@:...` annotation common to fab houses),
//! units and hit coordinates. Unconvertible hits stay raw with diagnostics.

use crate::domain::*;
use crate::parse_gerber::convert_length;

#[derive(Default)]
struct DState {
    unit: Option<Unit>,
    format: Option<CoordFormat>,
    tools: Vec<DrillTool>,
    hits: Vec<DrillHit>,
    diags: Vec<Diagnostic>,
    tool: String,
    x_int: u8,
    x_frac: u8,
}

fn error(line: usize, code: &str, msg: &str, raw: &str) -> Diagnostic {
    Diagnostic {
        code: code.to_string(),
        severity: Severity::Error,
        line,
        col: 1,
        message: msg.to_string(),
        raw: raw.to_string(),
    }
}

pub fn parse(source_id: &str, content: &str) -> ExcellonDoc {
    let mut st = DState::default();
    st.x_int = 2;
    st.x_frac = 4;

    for (idx, raw_line) in content.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with(';') {
            // Tool annotation e.g. ;@:T01:PTH
            if let Some(rest) = line.strip_prefix(";@:") {
                let mut parts = rest.split(':');
                let code = parts.next().unwrap_or("").trim().to_string();
                let kind = parts.next().unwrap_or("").trim().to_ascii_uppercase();
                let plating = match kind.as_str() {
                    "PTH" => Plating::Pth,
                    "NPTH" => Plating::Npth,
                    _ => Plating::Unknown,
                };
                if let Some(t) = st.tools.iter_mut().find(|t| t.code == code) {
                    t.plating = plating;
                }
            }
            continue;
        }
        if line == "%" {
            continue;
        }
        if let Some(rest) = line.strip_prefix("T") {
            // T01 or T01C0.30
            let mut split = rest.splitn(2, |c: char| c == 'C' || c == 'c');
            let code_num = split.next().unwrap_or("").trim();
            if code_num.is_empty() || !code_num.chars().all(|c| c.is_ascii_digit()) {
                st.diags.push(error(line_no, "EXCELLON_TOOL_BAD", "bad tool code", raw_line));
                continue;
            }
            let code = format!("T{}", code_num);
            if let Some(dia) = split.next() {
                let diameter_raw = dia.trim().trim_end_matches(['*', 'F']).to_string();
                let diameter_nm = diameter_raw.parse::<f64>().ok().map(|v| {
                    let f = st.unit.unwrap_or(Unit::In);
                    (v * if f == Unit::In { 25_400_000f64 } else { 1_000_000f64 }) as i64
                });
                if diameter_nm.is_none() {
                    st.diags.push(error(line_no, "EXCELLON_DIAMETER_BAD",
                        "tool diameter not numeric; raw retained", &diameter_raw));
                }
                st.tools.push(DrillTool {
                    code,
                    diameter_raw,
                    diameter_nm,
                    plating: Plating::Unknown,
                    line: line_no,
                });
            } else {
                st.tool = code;
            }
            continue;
        }
        match line {
            "M71" | "METRIC" | "METRIC,0000.0" => {
                st.unit = Some(Unit::Mm);
                st.x_frac = 3;
                continue;
            }
            "M72" | "INCH" => {
                st.unit = Some(Unit::In);
                st.x_frac = 4;
                continue;
            }
            "%" | "M30" | "M00" | "M02" => continue,
            _ => {}
        }
        if line.starts_with("G81") || line.starts_with("G05") || line.starts_with("G90")
            || line.starts_with("M15") || line.starts_with("M16") || line.starts_with("G00")
        {
            // mode words that may be glued to coordinates handled below
            if !line.contains('X') && !line.contains('Y') {
                continue;
            }
        }
        if line.starts_with('%') {
            continue;
        }
        // coordinate hit
        let (xraw, yraw) = split_xy(line, line_no, &mut st.diags);
        if xraw.is_none() && yraw.is_none() {
            st.diags.push(error(line_no, "EXCELLON_WORD_UNKNOWN",
                "unrecognised drill word; kept raw", raw_line));
            continue;
        }
        let conv = |rc: Option<&RawCoord>| -> Option<i64> {
            let rc = rc?;
            let frac = st.x_frac;
            let u = st.unit?;
            if rc.raw.contains('.') {
                // Explicit decimal always wins over the implied format.
                let v: f64 = rc.raw.parse().ok()?;
                let f = if u == Unit::In { 25_400_000f64 } else { 1_000_000f64 };
                return Some((v * f) as i64);
            }
            let digits = &rc.raw;
            let width = (st.x_int + frac) as usize;
            let expanded = if digits.len() >= width {
                digits.clone()
            } else {
                format!("{:0>width$}", digits, width = width)
            };
            convert_length(&expanded, u, frac)
        };
        let x_nm = conv(xraw.as_ref());
        let y_nm = conv(yraw.as_ref());
        if st.unit.is_none() {
            st.diags.push(error(line_no, "EXCELLON_NO_UNIT",
                "drill hit before unit declaration; raw retained", raw_line));
        }
        if let (Some(rx), None) = (&xraw, x_nm) {
            if st.unit.is_some() {
                st.diags.push(error(rx.line, "EXCELLON_COORD_BAD",
                    "x coordinate not convertible; raw retained", &rx.raw));
            }
        }
        if let (Some(ry), None) = (&yraw, y_nm) {
            if st.unit.is_some() {
                st.diags.push(error(ry.line, "EXCELLON_COORD_BAD",
                    "y coordinate not convertible; raw retained", &ry.raw));
            }
        }
        st.hits.push(DrillHit {
            tool: st.tool.clone(),
            x: xraw.unwrap_or(RawCoord { axis: Axis::X, raw: String::new(), line: line_no, col: 1 }),
            y: yraw.unwrap_or(RawCoord { axis: Axis::Y, raw: String::new(), line: line_no, col: 1 }),
            x_nm,
            y_nm,
        });
    }

    let format = st.unit.map(|u| CoordFormat {
        unit: u,
        zero: ZeroSuppression::Leading,
        x_int: st.x_int,
        x_frac: st.x_frac,
        y_int: st.x_int,
        y_frac: st.x_frac,
    });
    ExcellonDoc {
        source_id: source_id.to_string(),
        unit: st.unit,
        format,
        tools: st.tools,
        hits: st.hits,
        diagnostics: st.diags,
    }
}

fn split_xy(
    line: &str,
    line_no: usize,
    diags: &mut Vec<Diagnostic>,
) -> (Option<RawCoord>, Option<RawCoord>) {
    let bytes = line.as_bytes();
    let mut take = |tag: u8, axis: Axis| -> Option<RawCoord> {
        let pos = bytes.iter().position(|b| *b == tag)?;
        let mut end = pos + 1;
        while end < bytes.len()
            && (bytes[end] == b'-' || bytes[end] == b'.' || bytes[end].is_ascii_digit())
        {
            end += 1;
        }
        let raw = std::str::from_utf8(&bytes[pos + 1..end]).unwrap_or("").to_string();
        Some(RawCoord { axis, raw, line: line_no, col: pos + 1 })
    };
    let x = take(b'X', Axis::X);
    let y = take(b'Y', Axis::Y);
    let _ = diags;
    (x, y)
}
