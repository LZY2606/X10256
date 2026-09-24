//! Excellon drill parser (focused deterministic subset).
//!
//! Retains the raw unit/format header, every tool definition with plating status,
//! and blind/buried via layer range (`T...` comments and explicit LAYER_RANGE event
//! attributes). Unconvertible coordinates stay in place with diagnostics.

use crate::model::{CoordBasis, Diagnostic, Quantity, Unit};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Plating {
    Pth,
    Npth,
    Unknown,
}

impl Plating {
    pub fn parse(s: &str) -> Plating {
        match s.trim().to_ascii_uppercase().as_str() {
            "PTH" | "PLATED" => Plating::Pth,
            "NPTH" | "NONPLATED" | "NON-PLATED" => Plating::Npth,
            _ => Plating::Unknown,
        }
    }
    pub fn conductive_across_stack(&self) -> bool {
        matches!(self, Plating::Pth)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDef {
    pub code: String,
    pub diameter: Quantity,
    pub plating: Plating,
    /// inclusive 1-based physical copper layer span; `(1, n)` for a through hole.
    pub span_from: u32,
    pub span_to: u32,
    pub raw: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hole {
    pub tool: String,
    pub x_mm: Option<f64>,
    pub y_mm: Option<f64>,
    pub x_raw: String,
    pub y_raw: String,
    pub order: usize,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExcellonDoc {
    pub source_id: String,
    pub basis: CoordBasis,
    pub tools: Vec<ToolDef>,
    pub holes: Vec<Hole>,
    pub raw_text: String,
    pub diagnostics: Vec<Diagnostic>,
}

fn err(doc: &str, line: usize, code: &str, msg: impl Into<String>) -> Diagnostic {
    Diagnostic::err(code, doc, Some(line), msg)
}

fn decode_fixed(token: &str, int: u8, dec: u8) -> Option<f64> {
    let neg = token.starts_with('-');
    let core = token.trim_start_matches('-');
    if core.is_empty() || !core.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let padded: String = if (core.len() as u8) < int + dec {
        "0".repeat((int + dec - core.len() as u8) as usize) + core
    } else {
        core.to_string()
    };
    let v = if dec == 0 {
        padded.parse::<f64>().ok()?
    } else {
        let split = padded.len().saturating_sub(dec as usize);
        let (a, b) = padded.split_at(split);
        format!("{a}.{b}").parse::<f64>().ok()?
    };
    Some(if neg { -v } else { v })
}

impl ExcellonDoc {
    pub fn parse(source_id: &str, text: &str) -> ExcellonDoc {
        let mut unit = Unit::Unknown;
        let mut unit_raw = None;
        let mut format_raw = None;
        let mut int = 2u8;
        let mut dec = 4u8;
        let mut tools: Vec<ToolDef> = Vec::new();
        let mut holes: Vec<Hole> = Vec::new();
        let mut diagnostics = Vec::new();
        let mut active: Option<String> = None;
        let mut current_span = (1u32, u32::MAX);
        let mut order = 0usize;

        for (idx, raw_line) in text.lines().enumerate() {
            let line_no = idx + 1;
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with(';') {
                let comment = line.trim_start_matches(';').trim();
                if let Some(rest) = comment.strip_prefix("FORMAT=") {
                    format_raw = Some(line.to_string());
                    if let Some((a, b)) = rest.split_once('.') {
                        int = a.trim().parse().unwrap_or(int);
                        dec = b.trim().parse().unwrap_or(dec);
                    }
                } else if let Some(rest) = comment
                    .strip_prefix("LAYER_RANGE=")
                    .or_else(|| comment.strip_prefix("LAYER RANGE "))
                {
                    if let Some((a, b)) = rest.split_once('-').or_else(|| rest.split_once(':')) {
                        let f = a.trim().parse::<u32>().unwrap_or(1);
                        let t = b.trim().parse::<u32>().unwrap_or(u32::MAX);
                        current_span = (f, t);
                    }
                }
                continue;
            }
            if line.starts_with("METRIC") {
                unit = Unit::Mm;
                unit_raw = Some(line.to_string());
                continue;
            }
            if line.starts_with("INCHZ") || line.starts_with("INCH") {
                unit = Unit::In;
                unit_raw = Some(line.to_string());
                continue;
            }
            if line.starts_with("M72") {
                unit = Unit::In;
                unit_raw = Some(line.to_string());
                continue;
            }
            if line.starts_with("M71") {
                unit = Unit::Mm;
                unit_raw = Some(line.to_string());
                continue;
            }
            if line.starts_with('T') {
                // T1C0.30PTH  / T2C0.20NPTH ; LAYER_RANGE=2-3
                let rest = &line[1..];
                let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                let after = &rest[num.len()..];
                if after.starts_with('C') {
                    let tail = &after[1..];
                    let end = tail
                        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
                        .unwrap_or(tail.len());
                    let dia_raw = tail[..end].trim().to_string();
                    let plating = Plating::parse(tail[end..].trim());
                    let q = Quantity::new(dia_raw.clone(), unit);
                    if !q.convertible {
                        diagnostics.push(
                            err(source_id, line_no, "E101", format!("tool diameter `{dia_raw}` not normalisable"))
                                .with_snippet(line, dia_raw),
                        );
                    }
                    tools.push(ToolDef {
                        code: format!("T{num}"),
                        diameter: q,
                        plating,
                        span_from: current_span.0,
                        span_to: current_span.1,
                        raw: line.to_string(),
                    });
                } else if !num.is_empty() {
                    active = Some(format!("T{num}"));
                }
                continue;
            }
            if line.starts_with('%') {
                continue;
            }
            if line == "M30" || line == "M00" || line == "M02" {
                continue;
            }
            // hole coordinate X..Y..
            if line.contains('X') || line.contains('Y') {
                let mut x_raw = String::new();
                let mut y_raw = String::new();
                let mut cur = ' ';
                for ch in line.chars() {
                    match ch {
                        'X' => cur = 'x',
                        'Y' => cur = 'y',
                        '0'..='9' | '-' => {
                            if cur == 'x' {
                                x_raw.push(ch);
                            } else if cur == 'y' {
                                y_raw.push(ch);
                            }
                        }
                        _ => {}
                    }
                }
                let xv = if x_raw.is_empty() {
                    None
                } else {
                    match decode_fixed(&x_raw, int, dec) {
                        Some(v) => unit.to_mm_factor().map(|f| v * f),
                        None => {
                            diagnostics.push(
                                err(source_id, line_no, "E102", "bad X coordinate digits")
                                    .with_snippet(line, x_raw.clone()),
                            );
                            None
                        }
                    }
                };
                let yv = if y_raw.is_empty() {
                    None
                } else {
                    match decode_fixed(&y_raw, int, dec) {
                        Some(v) => unit.to_mm_factor().map(|f| v * f),
                        None => {
                            diagnostics.push(
                                err(source_id, line_no, "E103", "bad Y coordinate digits")
                                    .with_snippet(line, y_raw.clone()),
                            );
                            None
                        }
                    }
                };
                let tool = active.clone().unwrap_or_else(|| {
                    diagnostics.push(err(
                        source_id,
                        line_no,
                        "E104",
                        "hole emitted without an active tool; assigned T? and retained",
                    ));
                    "T?".into()
                });
                holes.push(Hole {
                    tool,
                    x_mm: xv,
                    y_mm: yv,
                    x_raw,
                    y_raw,
                    order,
                    line: line_no,
                });
                order += 1;
            }
        }
        if unit == Unit::Unknown {
            diagnostics.push(err(
                source_id,
                0,
                "E100",
                "no unit header (METRIC/INCHZ); hole positions retained in raw form",
            ));
        }
        ExcellonDoc {
            source_id: source_id.into(),
            basis: CoordBasis {
                unit,
                unit_raw,
                format_raw,
                notation: "excellon-fixed".into(),
                x_int: int,
                x_dec: dec,
                y_int: int,
                y_dec: dec,
            },
            tools,
            holes,
            raw_text: text.into(),
            diagnostics,
        }
    }

    /// Resolved span in stackup layers; through holes become (1, layer_count).
    pub fn span_for(&self, tool_code: &str, layer_count: u32) -> (u32, u32) {
        match self.tools.iter().find(|t| t.code == tool_code) {
            Some(t) => {
                let to = if t.span_to == u32::MAX { layer_count } else { t.span_to };
                (t.span_from, to.min(layer_count))
            }
            None => (1, layer_count),
        }
    }
}
