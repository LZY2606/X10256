//! Excellon drill subset parser.
//!
//! Retains raw tokens; recognizes standard `T..C..` tools, `METRIC`/`INCH`,
//! `M71/M72`, and the case-pack annotations `; LAYERSPAN=start-end` and
//! `; PLATING=PTH|NPTH` attached to a tool definition.

use crate::model::*;

pub fn parse(name: &str, text: &str) -> Result<ExcellonDoc, Vec<Diagnostic>> {
    let mut errs = Vec::new();
    let mut unit: Option<LengthUnit> = None;
    let mut fmt = String::new();
    let mut zero = "trailing";
    let mut tools: Vec<(String, Dim, Plating, Option<(usize, usize)>)> = Vec::new();

    // Pending annotations for the next tool.
    let mut pend_plating = Plating::Plated;
    let mut pend_span: Option<(usize, usize)> = None;

    let mut cur_tool = String::new();
    let mut hits: Vec<DrillHit> = Vec::new();
    let mut seq = 0usize;
    let mut format_declared = false;

    for (raw_line_no, raw_line) in text.lines().enumerate() {
        let line = raw_line_no + 1;
        let t = raw_line.trim();
        if t.is_empty() {
            continue;
        }
        if let Some(c) = t.strip_prefix(';') {
            let c = c.trim();
            if let Some(v) = c.strip_prefix("LAYERSPAN=") {
                match parse_span(v) {
                    Ok(sp) => pend_span = Some(sp),
                    Err(m) => errs.push(diag("E_SPAN", name, line, m, v.into())),
                }
            } else if let Some(v) = c.strip_prefix("PLATING=") {
                match v.trim() {
                    "PTH" => pend_plating = Plating::Plated,
                    "NPTH" => pend_plating = Plating::NonPlated,
                    other => errs.push(diag(
                        "E_PLATING",
                        name,
                        line,
                        format!("unknown plating '{}'", other),
                        t.into(),
                    )),
                }
            }
            continue;
        }
        if t == "METRIC" || t.starts_with("METRIC") {
            unit = Some(LengthUnit::Mm);
            format_declared = true;
            continue;
        }
        if t == "INCH" || t.starts_with("INCH") {
            unit = Some(LengthUnit::Inch);
            format_declared = true;
            continue;
        }
        if t == "M71" {
            unit = Some(LengthUnit::Mm);
            format_declared = true;
            continue;
        }
        if t == "M72" {
            unit = Some(LengthUnit::Inch);
            format_declared = true;
            continue;
        }
        if t.contains("FMAT") {
            fmt = t.to_string();
            format_declared = true;
            continue;
        }
        if t == "M30" || t == "M00" || t == "M02" || t == "M95" || t == "G90" || t == "G05" {
            continue;
        }
        if t == "%" {
            continue;
        }
        // Tool definition: T1C0.3 or T01F...S...
        if let Some(rest) = t.strip_prefix('T') {
            let digits_end = rest
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(rest.len());
            let tnum = rest[..digits_end].to_string();
            let tail = &rest[digits_end..];
            if tail.starts_with('C') {
                let diam_raw = tail[1..]
                    .split(|c: char| c.is_ascii_alphabetic())
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let mm = diam_raw
                    .parse::<f64>()
                    .ok()
                    .map(|v| if unit == Some(LengthUnit::Inch) { v * 25.4 } else { v });
                tools.push((
                    format!("T{}", tnum),
                    Dim::new(diam_raw.clone(), unit.unwrap_or(LengthUnit::User), mm),
                    pend_plating,
                    pend_span,
                ));
                pend_plating = Plating::Plated;
                pend_span = None;
                continue;
            }
            if tail.is_empty() {
                cur_tool = format!("T{}", tnum);
                continue;
            }
            // Tool change with trailing coords like T01X..Y..
            cur_tool = format!("T{}", tnum);
            if tail.contains('X') || tail.contains('Y') {
                push_hit(
                    &mut seq,
                    tail,
                    &cur_tool,
                    unit,
                    &tools,
                    &mut hits,
                    &mut errs,
                    name,
                    line,
                    format_declared,
                );
                continue;
            }
            continue;
        }
        if t.contains('X') || t.contains('Y') {
            push_hit(
                &mut seq,
                t,
                &cur_tool,
                unit,
                &tools,
                &mut hits,
                &mut errs,
                name,
                line,
                format_declared,
            );
            continue;
        }
        errs.push(diag("E_SYNTAX", name, line, "unrecognized Excellon block".into(), t.into()));
    }

    if !format_declared {
        errs.insert(
            0,
            Diagnostic {
                code: "E_FORMAT_MISSING".into(),
                severity: "error".into(),
                file: name.into(),
                line: 1,
                column: 0,
                message: "no INCH/METRIC declaration; drill coordinates cannot be converted".into(),
                token: String::new(),
            },
        );
    }

    if !errs.is_empty() {
        return Err(errs);
    }
    Ok(ExcellonDoc {
        datum: CoordinateDatum {
            unit: unit.unwrap_or(LengthUnit::User),
            format: fmt,
            zero_suppression: zero.into(),
            notation: "absolute".into(),
        },
        tools,
        hits,
    })
}

fn diag(code: &str, file: &str, line: usize, message: String, token: String) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        severity: "error".into(),
        file: file.into(),
        line,
        column: 0,
        message,
        token,
    }
}

fn parse_span(v: &str) -> Result<(usize, usize), String> {
    let (a, b) = v
        .split_once('-')
        .ok_or_else(|| format!("LAYERSPAN must be start-end, got '{}'", v))?;
    let s = a.trim().parse::<usize>().map_err(|_| format!("bad span start '{}'", a))?;
    let e = b.trim().parse::<usize>().map_err(|_| format!("bad span end '{}'", b))?;
    if s > e {
        return Err(format!("span start {} > end {}", s, e));
    }
    Ok((s, e))
}

#[allow(clippy::too_many_arguments)]
fn push_hit(
    seq: &mut usize,
    body: &str,
    tool: &str,
    unit: Option<LengthUnit>,
    tools: &[(String, Dim, Plating, Option<(usize, usize)>)],
    hits: &mut Vec<DrillHit>,
    errs: &mut Vec<Diagnostic>,
    name: &str,
    line: usize,
    format_declared: bool,
) {
    let xr = drill_tok(body, 'X').unwrap_or_default();
    let yr = drill_tok(body, 'Y').unwrap_or_default();
    let conv = |raw: &str| -> Option<f64> {
        let v: f64 = raw.parse().ok()?;
        Some(if unit == Some(LengthUnit::Inch) { v * 25.4 } else { v })
    };
    let x_mm = conv(&xr);
    let y_mm = conv(&yr);
    if !format_declared {
        errs.push(diag(
            "E_COORD_UNCONVERTIBLE",
            name,
            line,
            "hit kept in original digits; no unit declaration".into(),
            format!("X{}Y{}", xr, yr),
        ));
    }
    let tool_entry = tools.iter().find(|(t, _, _, _)| t == tool);
    let (diameter, plating, span) = match tool_entry {
        Some((_, d, p, sp)) => (d.clone(), *p, *sp),
        None => {
            errs.push(diag(
                "E_TOOL_UNDEFINED",
                name,
                line,
                format!("hit references undefined tool {}", tool),
                body.into(),
            ));
            return;
        }
    };
    *seq += 1;
    hits.push(DrillHit {
        seq: *seq,
        tool: tool.to_string(),
        diameter,
        x_raw: xr,
        y_raw: yr,
        x_mm,
        y_mm,
        plating,
        span_start: span.map(|(s, _)| s),
        span_end: span.map(|(_, e)| e),
    });
}

fn drill_tok(body: &str, key: char) -> Option<String> {
    let i = body.find(key)?;
    let rest = &body[i + 1..];
    let end = rest
        .find(|c: char| c.is_ascii_alphabetic())
        .unwrap_or(rest.len());
    let t = rest[..end].trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}
