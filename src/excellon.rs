//! Excellon drill subset parser.
//!
//! Retains raw coordinate representations and declared units/formats. Layer
//! span for blind/buried holes is provided by stackup mapping; a `; PLATED`
//! / `; NONPLATED` directive may additionally declare plating.

use crate::domain::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrillFormat {
    pub int: u32,
    pub frac: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tool {
    pub code: u32,
    pub diameter: RawNum,
    pub plated: Option<bool>,
    pub raw: String,
    pub pos: SourcePos,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hit {
    pub seq: u64,
    pub tool: u32,
    pub at: RawPoint,
    pub raw: String,
    pub pos: SourcePos,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DrillDoc {
    pub unit: Unit,
    pub format: Option<DrillFormat>,
    pub tools: Vec<Tool>,
    pub hits: Vec<Hit>,
    /// `; LAYER_SPAN=2-3` style raw declarations retained verbatim.
    pub layer_span_directives: Vec<(String, SourcePos)>,
    pub plated_directive: Option<(bool, SourcePos)>,
    pub diags: Vec<Diag>,
}

pub fn parse(src: &str) -> DrillDoc {
    let mut d = DrillDoc::default();
    let mut fmt: Option<(u32, u32)> = None;
    let mut cur_tool: Option<u32> = None;
    let mut seq = 0u64;

    for (idx, raw_line) in src.split('\n').enumerate() {
        let line = raw_line.trim_end_matches('\r');
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let offset = src
            .match_indices('\n')
            .nth(idx.saturating_sub(1))
            .map(|(i, _)| i + 1)
            .unwrap_or(0);
        let col = (line.len() - line.trim_start_matches(' ').len()) as u32 + 1;
        let pos = SourcePos::new(offset, idx as u32 + 1, col);

        if let Some(rest) = trimmed.strip_prefix(';') {
            handle_comment(&mut d, rest.trim(), pos);
            continue;
        }
        if trimmed == "%" {
            continue;
        }
        if let Some(rest) = stripped_prefix_any(trimmed, &["M48", "G90", "G05", "M95", "M30", "M00"]) {
            // absolute/metric modes handled elsewhere; end-of-program ignored
            let _ = rest;
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("METRIC") {
            d.unit = Unit::Mm;
            fmt = parse_fml(rest).or(fmt);
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("INCH") {
            d.unit = Unit::In;
            fmt = parse_fml(rest).or(fmt);
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("FMAT") {
            let _ = rest;
            continue;
        }
        if trimmed.starts_with('T') && trimmed.contains('C') {
            handle_tool(&mut d, trimmed, pos, d.unit);
            continue;
        }
        if trimmed.starts_with('T') {
            if let Some(code) = tool_code(trimmed) {
                cur_tool = Some(code);
            } else {
                d.diags.push(Diag::error("DRL_BAD_TOOL", "invalid tool select", Some(pos)).with_raw(trimmed));
            }
            continue;
        }
        if trimmed.contains('X') || trimmed.contains('Y') {
            let Some(tool) = cur_tool else {
                d.diags.push(Diag::error("DRL_HIT_NO_TOOL", "coordinate hit before Tn tool select", Some(pos)).with_raw(trimmed));
                continue;
            };
            let (xw, yw) = split_xy(trimmed);
            let frac = fmt.map(|f| f.1).unwrap_or_else(|| if trimmed.contains('.') { 0 } else { 0 });
            let x = parse_drill_num(xw, frac, d.unit, pos, &mut d.diags);
            let y = parse_drill_num(yw, frac, d.unit, pos, &mut d.diags);
            seq += 1;
            d.hits.push(Hit {
                seq,
                tool,
                at: RawPoint { x, y },
                raw: trimmed.to_string(),
                pos,
            });
            continue;
        }
        d.diags.push(Diag::warning("DRL_UNKNOWN_RECORD", "unrecognized drill record retained", Some(pos)).with_raw(trimmed));
    }

    d.format = fmt.map(|(int, frac)| DrillFormat { int, frac });
    if d.unit == Unit::Unspecified && !d.hits.is_empty() {
        d.diags.push(Diag::error(
            "DRL_NO_UNIT",
            "drill file never declares METRIC/INCH; hits retained raw and excluded from geometry",
            d.hits.first().map(|h| h.pos),
        ));
    }
    d
}

fn handle_comment(d: &mut DrillDoc, rest: &str, pos: SourcePos) {
    let up = rest.to_ascii_uppercase();
    if up == "PLATED" {
        d.plated_directive = Some((true, pos));
    } else if up == "NONPLATED" || up == "NPTH" {
        d.plated_directive = Some((false, pos));
    } else if up.starts_with("LAYER_SPAN=") {
        d.layer_span_directives.push((rest.to_string(), pos));
    }
}

fn stripped_prefix_any<'a>(s: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    prefixes.iter().find_map(|p| s.strip_prefix(p))
}

fn parse_fml(rest: &str) -> Option<(u32, u32)> {
    let token = rest.trim().trim_start_matches(',').trim();
    let (a, b) = token.split_once('.')?;
    Some((a.parse().ok()?, b.len() as u32))
}

fn tool_code(s: &str) -> Option<u32> {
    let digits: String = s[1..].chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

fn handle_tool(d: &mut DrillDoc, s: &str, pos: SourcePos, unit: Unit) {
    let code = match tool_code(s) {
        Some(c) => c,
        None => {
            d.diags.push(Diag::error("DRL_BAD_TOOL", "invalid tool definition", Some(pos)).with_raw(s));
            return;
        }
    };
    let dia_s = match s.split(['C', 'c']).nth(1) {
        Some(v) => v.trim(),
        None => {
            d.diags.push(Diag::error("DRL_BAD_TOOL", "tool missing C diameter", Some(pos)).with_raw(s));
            return;
        }
    };
    let diameter = parse_drill_num(dia_s, 0, unit, pos, &mut d.diags);
    let plated = d.plated_directive.map(|(p, _)| p);
    d.tools.push(Tool { code, diameter, plated, raw: s.to_string(), pos });
}

fn split_xy(s: &str) -> (&str, &str) {
    let bytes = s.as_bytes();
    let mut x = "";
    let mut y = "";
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'X' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] != b'Y' {
                j += 1;
            }
            x = s[i + 1..j].trim();
            i = j;
        } else if bytes[i] == b'Y' {
            let mut j = i + 1;
            while j < bytes.len() && !(bytes[j] == b'X' || bytes[j] == b'T') {
                j += 1;
            }
            y = s[i + 1..j].trim();
            i = j;
        } else {
            i += 1;
        }
    }
    (x, y)
}

fn parse_drill_num(word: &str, frac: u32, unit: Unit, pos: SourcePos, diags: &mut Vec<Diag>) -> RawNum {
    if word.is_empty() {
        diags.push(Diag::error("DRL_MISSING_COORD", "hit missing X or Y coordinate", Some(pos)));
        return RawNum::new(word, unit, None);
    }
    let nano = if unit == Unit::Unspecified {
        None
    } else if word.contains('.') {
        decimal_to_nm(word, unit)
    } else {
        word.parse::<i128>().ok().and_then(|n| fixed_to_nm(n, frac, unit))
    };
    if nano.is_none() {
        diags.push(
            Diag::error("DRL_UNCONVERTIBLE_COORD", "drill coordinate cannot be normalized; retained raw", Some(pos)).with_raw(word),
        );
    }
    RawNum::new(word, unit, nano)
}
