//! Excellon drill/route subset parser.
//!
//! Preserved from source: raw integer representation, unit (`METRIC`/`INCH`),
//! tool table (`T..C..`) and layer-range annotations (`;LAYER_RANGE=a..b`).
//! When the coordinate format is not declared (`;FORMAT=2:4` / FMAT style),
//! integer coordinates cannot be converted: they are retained in place and
//! flagged invalid instead of being guessed.

use crate::model::{Diagnostic, LengthUnit, SrcPos};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LayerRange {
    pub from: usize,
    pub to: usize,
    pub raw: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ToolDef {
    pub code: String,
    /// Diameter in file units (mm if METRIC, inch if INCH).
    pub diameter_raw: f64,
    pub plated: bool,
    pub layer_range: Option<LayerRange>,
    pub raw: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DrillHit {
    pub tool: String,
    /// Raw integer tokens as written, e.g. `X01270Y01000`.
    pub x_int: Option<i64>,
    pub y_int: Option<i64>,
    /// Decoded position in mm; None when format is not known (invalid, retained).
    pub x_mm: Option<f64>,
    pub y_mm: Option<f64>,
    pub valid: bool,
    pub invalid_reason: Option<String>,
    pub raw: String,
    pub pos: Option<SrcPos>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ExcellonDoc {
    pub unit: Option<LengthUnit>,
    /// Digits after the decimal point implied by the integer representation.
    pub dec_digits: Option<u8>,
    pub tools: Vec<ToolDef>,
    pub hits: Vec<DrillHit>,
    /// File-level default layer range (1-based stackup span).
    pub default_layer_range: Option<LayerRange>,
    pub raw_lines: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn parse(input: &str, file_id: &str) -> Result<ExcellonDoc, (ExcellonDoc, Vec<Diagnostic>)> {
    let mut doc = ExcellonDoc::default();
    let mut current_tool: Option<String> = None;
    let mut current_range: Option<LayerRange> = None;
    for (idx, raw_line) in input.lines().enumerate() {
        let line = raw_line.trim();
        doc.raw_lines.push(raw_line.to_string());
        if line.is_empty() {
            continue;
        }
        let line_no = idx + 1;
        let pos = SrcPos {
            file_id: file_id.to_string(),
            line: line_no,
            col: 1,
            byte_start: 0,
            byte_end: raw_line.len(),
            snippet: line.to_string(),
        };
        if line.starts_with(';') {
            handle_comment(line, &mut doc, &mut current_range, &pos);
            continue;
        }
        if line == "%" {
            continue;
        }
        if line.starts_with("METRIC") {
            doc.unit = Some(LengthUnit::Mm);
            if let Some(d) = dec_from_header(line) {
                doc.dec_digits = Some(d);
            }
            continue;
        }
        if line.starts_with("INCH") {
            doc.unit = Some(LengthUnit::In);
            if let Some(d) = dec_from_header(line) {
                doc.dec_digits = Some(d);
            }
            continue;
        }
        if line.starts_with("T") && line.contains('C') {
            if let Some(td) = parse_tool(line, current_range.clone()) {
                doc.tools.push(td.clone());
                current_tool = Some(td.code);
            } else {
                doc.diagnostics
                    .push(Diagnostic::err("EXCELLON_BAD_TOOL", format!("unparseable tool line `{line}`")).at(pos.clone()));
            }
            continue;
        }
        if line.starts_with('T') {
            let code = line.split(|c: char| c.is_whitespace()).next().unwrap_or("").to_string();
            if code.len() >= 2 && code[1..].chars().all(|c| c.is_ascii_digit()) {
                current_tool = Some(code);
                continue;
            }
        }
        if line.contains('X') || line.contains('Y') {
            let (x_int, y_int) = parse_xy_ints(line);
            let (x_mm, y_mm, valid, reason) = decode(x_int, y_int, doc.unit, doc.dec_digits);
            doc.hits.push(DrillHit {
                tool: current_tool.clone().unwrap_or_else(|| "T?".to_string()),
                x_int,
                y_int,
                x_mm,
                y_mm,
                valid,
                invalid_reason: reason,
                raw: line.to_string(),
                pos: Some(pos),
            });
            continue;
        }
        if line == "M30" || line == "M00" || line.starts_with("FMAT") {
            continue;
        }
        doc.diagnostics.push(
            Diagnostic::warn("EXCELLON_UNKNOWN_LINE", format!("line `{line}` retained but not interpreted")).at(pos),
        );
    }
    if doc.unit.is_none() {
        doc.diagnostics.push(Diagnostic::warn(
            "EXCELLON_NO_UNIT",
            "no METRIC/INCH header; tool and coordinate units cannot be determined",
        ));
    }
    if doc.dec_digits.is_none() {
        doc.diagnostics.push(Diagnostic::warn(
            "EXCELLON_NO_FORMAT",
            "no integer format declaration (e.g. ;FORMAT=3:3); drill coordinates retained as raw integers",
        ));
    }
    if doc.diagnostics.iter().any(|d| d.is_error()) {
        Err((doc.clone(), doc.diagnostics.clone()))
    } else {
        Ok(doc)
    }
}

fn handle_comment(line: &str, doc: &mut ExcellonDoc, current_range: &mut Option<LayerRange>, pos: &SrcPos) {
    let text = line.trim_start_matches(';').trim();
    if let Some(rest) = text.strip_prefix("FORMAT=") {
        if let Some((_i, d)) = parse_fmt(rest) {
            doc.dec_digits = Some(d);
        } else {
            doc.diagnostics
                .push(Diagnostic::warn("EXCELLON_BAD_FORMAT", format!("unparseable `{text}`")).at(pos.clone()));
        }
    } else if let Some(rest) = text.strip_prefix("LAYER_RANGE=") {
        match parse_range(rest) {
            Some(r) => {
                *current_range = Some(r.clone());
                doc.default_layer_range = Some(r);
            }
            None => doc
                .diagnostics
                .push(Diagnostic::err("EXCELLON_BAD_RANGE", format!("unparseable layer range `{text}`")).at(pos.clone())),
        }
    }
    // Other comments retained verbatim via raw_lines.
}

fn dec_from_header(line: &str) -> Option<u8> {
    // METRIC,TZ or METRIC,000.000 ; we look for a FORMAT comment instead normally.
    let _ = line;
    None
}

fn parse_fmt(rest: &str) -> Option<(u8, u8)> {
    let (a, b) = rest.split_once(':')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

fn parse_range(rest: &str) -> Option<LayerRange> {
    let (a, b) = rest.split_once("..").or_else(|| rest.split_once('-'))?;
    Some(LayerRange {
        from: a.trim().parse().ok()?,
        to: b.trim().parse().ok()?,
        raw: rest.to_string(),
    })
}

fn parse_tool(line: &str, range: Option<LayerRange>) -> Option<ToolDef> {
    // T1C0.30 or T01F00S00C0.2 ; PLATED
    let (code_part, rest) = line.split_once('C')?;
    let code = code_part
        .split(|c: char| c.is_whitespace() || c == ',')
        .next()?
        .to_string();
    let after: String = rest
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let diameter = after.parse().ok()?;
    let plated = line.contains("PLATED") && !line.contains("NON-PLATED") && !line.contains("NONPLATED");
    Some(ToolDef {
        code,
        diameter_raw: diameter,
        plated,
        layer_range: range,
        raw: line.to_string(),
    })
}

fn parse_xy_ints(line: &str) -> (Option<i64>, Option<i64>) {
    fn grab(line: &str, key: char) -> Option<i64> {
        let i = line.find(key)? + 1;
        let bytes = line.as_bytes();
        let mut j = i;
        while j < bytes.len() {
            let c = bytes[j] as char;
            if c.is_ascii_alphabetic() {
                break;
            }
            j += 1;
        }
        line[i..j].trim().parse().ok()
    }
    (grab(line, 'X'), grab(line, 'Y'))
}

fn decode(
    x: Option<i64>,
    y: Option<i64>,
    unit: Option<LengthUnit>,
    dec: Option<u8>,
) -> (Option<f64>, Option<f64>, bool, Option<String>) {
    match (unit, dec) {
        (Some(u), Some(d)) => {
            let scale = 10f64.powi(d as i32);
            let conv = |n: i64| (n as f64 / scale) * u.to_mm();
            (x.map(conv), y.map(conv), true, None)
        }
        (None, _) => (None, None, false, Some("no unit header (METRIC/INCH)".to_string())),
        (_, None) => (None, None, false, Some("no coordinate format declaration".to_string())),
    }
}
