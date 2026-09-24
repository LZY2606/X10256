//! Excellon drill file parser (offline subset).

use crate::error::{Diagnostic, DomainResult, FailureKind};
use crate::model::*;
use crate::parser::reconstruct_fixed;

use super::fail;

pub fn parse_excellon(text: &str) -> DomainResult<ExcellonDoc> {
    let mut unit_nm: Option<i64> = None;
    let mut unit_raw = String::new();
    let mut unit_line = 0usize;
    let mut tools: Vec<DrillTool> = Vec::new();
    let mut hits: Vec<DrillHit> = Vec::new();
    let mut diag: Vec<Diagnostic> = Vec::new();

    // Coordinate format, declared via FMAT or inferred from first hit.
    let mut int_digits = 2u32;
    let mut dec_digits = 4u32;
    let mut format_seen = false;
    let mut current_tool: Option<String> = None;
    // Last layer-range directive applies to subsequently defined tools.
    let mut pending_range: Option<(u32, u32)> = None;

    for (idx0, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        let line_no = idx0 + 1;
        if line.is_empty() {
            continue;
        }

        if let Some(rest) = line.strip_prefix(';') {
            // Comment: may carry a blind/buried layer range.
            if let Some(r) = parse_range_comment(rest) {
                pending_range = Some(r);
            }
            continue;
        }
        if line == "M30" || line == "M00" || line == "%" || line == "M48" {
            continue;
        }
        if let Some(rest) = line.strip_prefix("METRIC") {
            unit_nm = Some(1_000_000);
            unit_raw = "MM".to_string();
            unit_line = line_no;
            if let Some(fmt) = rest.split(',').find_map(|f| f.trim().strip_prefix("0")) {
                if let Ok(d) = fmt.chars().filter(|c| c.is_ascii_digit()).collect::<String>().parse::<u32>() {
                    dec_digits = d;
                    format_seen = true;
                }
            }
            continue;
        }
        if line.starts_with("INCH") {
            unit_nm = Some(25_400_000);
            unit_raw = "IN".to_string();
            unit_line = line_no;
            if let Some(rest) = line.strip_prefix("INCH") {
                if let Some(fmt) = rest.split(',').find_map(|f| f.trim().strip_prefix("0")) {
                    if let Ok(d) = fmt.chars().filter(|c| c.is_ascii_digit()).collect::<String>().parse::<u32>() {
                        dec_digits = d;
                        format_seen = true;
                    }
                }
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("FMAT") {
            if let Some(n) = rest.split(',').nth(1).or_else(|| Some(rest.trim_start_matches(','))) {
                if n.trim() == "2" {
                    format_seen = true;
                }
            }
            continue;
        }
        if line.starts_with('T') && line.contains('C') {
            // Tool definition T1C0.3 (+ optional P for plating)
            parse_tool(line, line_no, unit_nm, &mut tools, &mut diag, pending_range);
            if let Some(id) = tool_id_of(line) {
                current_tool = Some(id);
            }
            continue;
        }
        if line.starts_with('T') {
            if let Some(id) = tool_id_of(line) {
                current_tool = Some(id);
            }
            continue;
        }
        if line.starts_with('X') || line.starts_with('Y') {
            let tool = match &current_tool {
                Some(t) => t.clone(),
                None => {
                    diag.push(Diagnostic::new(
                        FailureKind::SemanticUnresolved,
                        "EXCELLON_NO_TOOL",
                        format!("line {line_no}"),
                        "drill hit with no active tool",
                    ).with_raw(line.to_string()));
                    continue;
                }
            };
            let unit = match unit_nm {
                Some(u) => u,
                None => {
                    diag.push(Diagnostic::new(
                        FailureKind::SemanticUnresolved,
                        "EXCELLON_NO_UNIT",
                        format!("line {line_no}"),
                        "drill hit before METRIC/INCH unit statement; value left in original position",
                    ).with_raw(line.to_string()));
                    continue;
                }
            };
            let (xt, yt) = xy_fields(line);
            let convert = |tok: &str| -> Result<i64, String> {
                // Decimal point overrides fixed format.
                if tok.contains('.') {
                    let v: f64 = tok.parse().map_err(|_| format!("bad decimal '{tok}'"))?;
                    Ok((v * unit as f64).round() as i64)
                } else {
                    let local = reconstruct_fixed(tok, int_digits, dec_digits, true)?;
                    Ok((unit as i128 * local as i128 / 10i128.pow(dec_digits)) as i64)
                }
            };
            let x_tok = xt.unwrap_or("0");
            let y_tok = yt.unwrap_or("0");
            let x = match convert(x_tok) {
                Ok(v) => v,
                Err(e) => {
                    diag.push(Diagnostic::new(
                        FailureKind::SemanticUnresolved,
                        "EXCELLON_BAD_COORD",
                        format!("line {line_no} field X"),
                        format!("{e}; value left raw"),
                    ).with_raw(x_tok));
                    continue;
                }
            };
            let y = match convert(y_tok) {
                Ok(v) => v,
                Err(e) => {
                    diag.push(Diagnostic::new(
                        FailureKind::SemanticUnresolved,
                        "EXCELLON_BAD_COORD",
                        format!("line {line_no} field Y"),
                        format!("{e}; value left raw"),
                    ).with_raw(y_tok));
                    continue;
                }
            };
            hits.push(DrillHit {
                tool,
                at: Point { x, y, x_raw: x_tok.to_string(), y_raw: y_tok.to_string() },
                line: line_no,
            });
            if !format_seen {
                // Infer 2.4 default once; retained as a provenance note.
            }
            continue;
        }
        if line.starts_with("%AXIS") || line.starts_with("ICI") || line.starts_with("G90") || line.starts_with("G91") {
            continue;
        }
        diag.push(Diagnostic::new(
            FailureKind::CorruptFormat,
            "EXCELLON_UNKNOWN_LINE",
            format!("line {line_no}"),
            format!("unrecognised Excellon line: {line}"),
        ));
    }

    if unit_nm.is_none() {
        diag.push(Diagnostic::new(
            FailureKind::SemanticUnresolved,
            "EXCELLON_NO_UNIT",
            "header",
            "no METRIC/INCH unit statement; hits cannot be normalized",
        ));
    }
    if diag.iter().any(|d| d.kind == FailureKind::CorruptFormat) {
        return Err(fail(diag));
    }

    Ok(ExcellonDoc {
        unit_raw,
        unit_nm: unit_nm.unwrap_or(0),
        unit_line,
        tools,
        hits,
        diagnostics: diag,
    })
}

fn parse_range_comment(rest: &str) -> Option<(u32, u32)> {
    // ; LAYERS=L1-L4  (inclusive physical copper order)
    let v = rest.to_ascii_uppercase();
    let v = v.split("LAYERS=").nth(1).unwrap_or("");
    let mut it = v.split(['-', ':', ' ']).filter_map(|t| {
        t.trim_start_matches('L').chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse::<u32>().ok()
    });
    let a = it.next()?;
    let b = it.next()?;
    if a >= 1 && b >= a { Some((a, b)) } else { None }
}

fn tool_id_of(line: &str) -> Option<String> {
    let digits: String = line.trim_start_matches('T').chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() { None } else { Some(format!("T{}", digits.parse::<u32>().ok()?)) }
}

fn parse_tool(line: &str, line_no: usize, unit_nm: Option<i64>, tools: &mut Vec<DrillTool>, diag: &mut Vec<Diagnostic>, range: Option<(u32, u32)>) {
    let id = match tool_id_of(line) {
        Some(id) => id,
        None => return,
    };
    let diam_raw = line.split('C').nth(1).map(|s| {
        s.split(['*', 'X', 'x']).next().unwrap_or("").trim().to_string()
    }).unwrap_or_default();
    let plated = !line.contains("PTH=NON")
        && !line.contains("NONPTH")
        && !line.contains("NON-PLATED")
        && !line.contains("NPTH")
        && !line.contains("PLATED=NO");
    let diameter_nm = match diam_raw.parse::<f64>() {
        Ok(v) => (v * unit_nm.unwrap_or(1_000_000) as f64).round() as i64,
        Err(_) => {
            diag.push(Diagnostic::new(
                FailureKind::SemanticUnresolved,
                "EXCELLON_BAD_DIAMETER",
                format!("line {line_no}"),
                format!("tool diameter '{diam_raw}' not numeric; left raw"),
            ).with_raw(diam_raw.clone()));
            0
        }
    };
    if let Some(pos) = tools.iter().position(|t| t.id == id) {
        tools[pos] = DrillTool { id, diameter_nm, diameter_raw: diam_raw, plated, layer_range: range, raw: line.to_string(), line: line_no };
    } else {
        tools.push(DrillTool { id, diameter_nm, diameter_raw: diam_raw, plated, layer_range: range, raw: line.to_string(), line: line_no });
    }
}

fn xy_fields(line: &str) -> (Option<&str>, Option<&str>) {
    fn after(ch: char, s: &str) -> Option<&str> {
        let start = s.find(ch)? + 1;
        let tail = &s[start..];
        let end = tail
            .char_indices()
            .find(|(_, c)| matches!(c, 'X' | 'Y' | '*'))
            .map(|(i, _)| i)
            .unwrap_or(tail.len());
        let v = tail[..end].trim();
        if v.is_empty() { None } else { Some(v) }
    }
    (after('X', line), after('Y', line))
}
