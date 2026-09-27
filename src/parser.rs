use crate::domain::*;
use std::collections::BTreeMap;

pub fn parse_material(filename: &str, text: &str) -> (MaterialKind, String, Option<ParsedDocument>, Vec<Diagnostic>) {
    if filename.ends_with(".drl") {
        let (doc, diags) = parse_excellon(filename, text);
        (MaterialKind::Excellon, "drill".into(), Some(ParsedDocument::Excellon(doc)), diags)
    } else if filename.ends_with(".stack.json") {
        match parse_stackup(text) {
            Ok(doc) => (MaterialKind::Stackup, "stackup".into(), Some(ParsedDocument::Stackup(doc)), vec![]),
            Err(diag) => (MaterialKind::Stackup, "stackup".into(), None, vec![diag]),
        }
    } else {
        let (doc, diags) = parse_gerber(filename, text);
        (MaterialKind::Gerber, doc.layer_role_hint.clone().unwrap_or_else(|| "unmapped".into()), Some(ParsedDocument::Gerber(doc)), diags)
    }
}

fn pos(line: usize, col: usize, text: &str) -> Position {
    Position { line, column: col, source_text: text.chars().take(160).collect() }
}

fn diag(code: &str, message: String, material: &str, line: usize, col: usize, text: &str) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        severity: DiagnosticSeverity::Error,
        message,
        position: Some(pos(line, col, text)),
        material_id: Some(material.into()),
    }
}

fn parse_measure(raw: &str, unit: UnitKind) -> Measure {
    Measure { raw: raw.to_string(), unit }
}

fn raw_point(x: Option<String>, y: Option<String>, current: &Option<RawPoint>, format: &str, unit: UnitKind) -> RawPoint {
    RawPoint {
        x: x.or_else(|| current.as_ref().map(|p| p.x.clone())).unwrap_or_default(),
        y: y.or_else(|| current.as_ref().map(|p| p.y.clone())).unwrap_or_default(),
        format: format.into(),
        unit,
    }
}

fn find_xy(line: &str) -> (Option<String>, Option<String>, usize, usize) {
    let mut x = None;
    let mut y = None;
    let mut xcol = 0;
    let mut ycol = 0;
    for (idx, ch) in line.char_indices() {
        if ch == 'X' {
            let end = line[idx + 1..].find(|c: char| c == 'Y' || c == 'D' || c == '*').map(|p| idx + 1 + p).unwrap_or(line.len());
            x = Some(line[idx + 1..end].to_string());
            xcol = idx + 1;
        }
        if ch == 'Y' {
            let end = line[idx + 1..].find(|c: char| c == 'X' || c == 'D' || c == '*').map(|p| idx + 1 + p).unwrap_or(line.len());
            y = Some(line[idx + 1..end].to_string());
            ycol = idx + 1;
        }
    }
    (x, y, xcol + 1, ycol + 1)
}

fn parse_gerber(material: &str, text: &str) -> (GerberDocument, Vec<Diagnostic>) {
    let mut diags = vec![];
    let mut unit = UnitKind::None;
    let mut coordinate_format = "unspecified".to_string();
    let mut polarity = Polarity::Positive;
    let mut apertures = BTreeMap::new();
    let mut operations = vec![];
    let mut layer_role_hint = None;
    let mut current_aperture = None;
    let mut current_point: Option<RawPoint> = None;
    let mut step_repeat = None;

    for (line_no, raw_line) in text.lines().enumerate().map(|(i, l)| (i + 1, l.trim())) {
        if raw_line.is_empty() { continue; }
        if raw_line.contains("%FSLAX") {
            coordinate_format = raw_line.trim_matches('*').to_string();
        } else if raw_line.contains("%MOMM*") {
            unit = UnitKind::Mm;
        } else if raw_line.contains("%MOIN*") {
            unit = UnitKind::Inch;
        } else if raw_line.starts_with("%LP") {
            polarity = if raw_line.contains('C') { Polarity::Negative } else { Polarity::Positive };
        } else if raw_line.starts_with("%LR") {
            layer_role_hint = raw_line.trim_matches('%').trim_start_matches("LR").trim_end_matches('*').split_once(':').map(|(_, v)| v.to_string());
        } else if raw_line.starts_with("%AD") {
            let body = raw_line.trim_matches('%').trim_end_matches('*');
            let Some((id, spec)) = body[2..].split_once(',') else {
                diags.push(diag("GBR-APERTURE", "aperture definition requires D<id>,<shape>".into(), material, line_no, 4, raw_line));
                continue;
            };
            let shape: String = spec.chars().take_while(|c| c.is_ascii_uppercase()).collect();
            let dims: Vec<&str> = spec[shape.len()..].trim_start_matches('X').split('X').collect();
            let raw0 = dims.first().copied().unwrap_or("");
            if raw0.is_empty() || raw0.chars().any(|c| !c.is_ascii_digit() && c != '.') {
                diags.push(diag("GBR-UNCONVERTIBLE-MEASURE", "aperture dimension remains in its original position".into(), material, line_no, 5 + shape.len(), raw_line));
            }
            apertures.insert(id.to_string(), ApertureRecord {
                id: id.into(),
                shape,
                width: parse_measure(raw0, unit),
                height: parse_measure(dims.get(1).copied().unwrap_or(raw0), unit),
                raw: raw_line.into(),
            });
        } else if raw_line.starts_with("%SR") {
            let body = raw_line.trim_matches('%').trim_start_matches("SR").trim_end_matches('*');
            let mut x_count = 1;
            let mut y_count = 1;
            let mut x_distance = "0".to_string();
            let mut y_distance = "0".to_string();
            for part in body.split_whitespace() {
                if let Some(v) = part.strip_prefix("X") { x_count = v.parse().unwrap_or(1); }
                if let Some(v) = part.strip_prefix("Y") { y_count = v.parse().unwrap_or(1); }
                if let Some(v) = part.strip_prefix("I") { x_distance = v.into(); }
                if let Some(v) = part.strip_prefix("J") { y_distance = v.into(); }
            }
            step_repeat = Some(StepRepeat { x_count, y_count, x_distance: parse_measure(&x_distance, unit), y_distance: parse_measure(&y_distance, unit), raw: raw_line.into() });
        } else if raw_line == "SR0*" {
            step_repeat = None;
        } else if raw_line.starts_with("D") && raw_line.ends_with('*') && raw_line.len() > 2 {
            let d = raw_line.trim_end_matches('*');
            if apertures.contains_key(d) {
                current_aperture = Some(d.to_string());
            }
        } else if raw_line.contains("D0") {
            let (rx, ry, xcol, ycol) = find_xy(raw_line);
            if rx.as_ref().is_some_and(|v| v.is_empty()) {
                diags.push(diag("GBR-UNCONVERTIBLE-COORD", "X coordinate cannot be converted".into(), material, line_no, xcol, raw_line));
            }
            if ry.as_ref().is_some_and(|v| v.is_empty()) {
                diags.push(diag("GBR-UNCONVERTIBLE-COORD", "Y coordinate cannot be converted".into(), material, line_no, ycol, raw_line));
            }
            let point = raw_point(rx, ry, &current_point, &coordinate_format, unit);
            let dpos = raw_line.find("D0").map(|p| p + 3).unwrap_or(raw_line.len());
            let op = raw_line[dpos.saturating_sub(2)..].trim_end_matches('*').to_string();
            let from = if op == "D01" { current_point.clone() } else { None };
            let to = if op == "D02" { None } else { Some(point.clone()) };
            current_point = Some(point);
            operations.push(GerberOperation {
                ordinal: operations.len() + 1,
                op,
                polarity,
                aperture: current_aperture.clone(),
                from,
                to,
                step_repeat: step_repeat.clone(),
                raw: raw_line.into(),
            });
        } else if raw_line != "G04*" && !raw_line.starts_with("G04") && raw_line != "M02*" && raw_line != "G01*" {
            diags.push(diag("GBR-SYNTAX", "unrecognized Gerber block".into(), material, line_no, 1, raw_line));
        }
    }
    (GerberDocument { coordinate_format, unit, polarity, apertures, operations, layer_role_hint }, diags)
}

fn parse_excellon(material: &str, text: &str) -> (ExcellonDocument, Vec<Diagnostic>) {
    let mut diags = vec![];
    let mut unit = UnitKind::None;
    let mut coordinate_format = "unspecified".to_string();
    let mut tools = BTreeMap::new();
    let mut hits = vec![];
    let mut current_tool = None;

    for (line_no, raw_line) in text.lines().enumerate().map(|(i, l)| (i + 1, l.trim())) {
        if raw_line.is_empty() { continue; }
        if raw_line == "M48" || raw_line == "%" || raw_line == "M30" { continue; }
        if raw_line.starts_with("METRIC") || raw_line.starts_with("INCH") || raw_line.starts_with("FORMAT") {
            if !raw_line.starts_with("FORMAT") { unit = if raw_line.starts_with("METRIC") { UnitKind::Mm } else { UnitKind::Inch }; }
            coordinate_format = raw_line.into();
        } else if let Some(rest) = raw_line.strip_prefix('T') {
            let head: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if raw_line.contains('C') {
                let plated = !raw_line.contains(";NONPLATED");
                let mut first = "L1".to_string();
                let mut last = "L10".to_string();
                if let Some(scope) = raw_line.strip_prefix(&format!("T{head}C")).and_then(|s| s.split(';').next()) {
                    let mut parts = scope.split('/');
                    if let (Some(a), Some(b)) = (parts.next(), parts.next()) {
                        first = a.into();
                        last = b.into();
                    }
                }
                let cidx = raw_line.find('C').map(|p| p + 2).unwrap_or(2);
                let diameter: String = raw_line[cidx.saturating_sub(1)..].split(';').next().unwrap_or("").chars().skip(1).collect();
                if diameter.is_empty() || diameter.chars().any(|c| !c.is_ascii_digit() && c != '.') {
                    diags.push(diag("DRL-UNCONVERTIBLE-DIAMETER", "tool diameter remains in original position".into(), material, line_no, cidx, raw_line));
                }
                tools.insert(head.clone(), DrillTool {
                    id: head.clone(),
                    diameter: parse_measure(&diameter, unit),
                    plated,
                    first_layer: first,
                    last_layer: last,
                    raw: raw_line.into(),
                });
            } else if !tools.contains_key(&head) {
                diags.push(diag("DRL-TOOL-UNDEFINED", "hit uses a tool that has no definition".into(), material, line_no, 1, raw_line));
            }
            current_tool = Some(head);
        } else if raw_line.starts_with('X') || raw_line.starts_with('Y') {
            let (x, y, xcol, ycol) = find_xy(raw_line);
            if x.as_ref().is_none_or(String::is_empty) {
                diags.push(diag("DRL-UNCONVERTIBLE-COORD", "X coordinate cannot be converted".into(), material, line_no, xcol, raw_line));
            }
            if y.as_ref().is_none_or(String::is_empty) {
                diags.push(diag("DRL-UNCONVERTIBLE-COORD", "Y coordinate cannot be converted".into(), material, line_no, ycol, raw_line));
            }
            let tool_id = current_tool.clone().unwrap_or_else(|| {
                diags.push(diag("DRL-TOOL-MISSING", "drill hit has no active tool".into(), material, line_no, 1, raw_line));
                "UNKNOWN".into()
            });
            let tool_plated = tools.get(&tool_id).map(|t| t.plated).unwrap_or(false);
            hits.push(DrillHit {
                ordinal: hits.len() + 1,
                point: RawPoint { x: x.unwrap_or_default(), y: y.unwrap_or_default(), format: coordinate_format.clone(), unit },
                tool_id,
                plated: tool_plated,
                raw: raw_line.into(),
            });
        } else {
            diags.push(diag("DRL-SYNTAX", "unrecognized Excellon block".into(), material, line_no, 1, raw_line));
        }
    }
    (ExcellonDocument { coordinate_format, unit, tools, hits }, diags)
}

fn parse_stackup(text: &str) -> Result<StackupDocument, Diagnostic> {
    let value: serde_json::Value = serde_json::from_str(text).map_err(|e| Diagnostic {
        code: "STACK-JSON".into(),
        severity: DiagnosticSeverity::Error,
        message: format!("stackup JSON is corrupt: {e}"),
        position: Some(Position { line: e.line(), column: e.column(), source_text: text.into() }),
        material_id: None,
    })?;
    let mut layers = vec![];
    for layer in value.get("layers").and_then(|v| v.as_array()).ok_or_else(|| stack_diag("stackup requires a layers array"))? {
        layers.push(StackLayer {
            id: require_string(layer, "id")?,
            role: parse_role(&require_string(layer, "role")?)?,
            role_label: layer.get("role_label").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            order: layer.get("order").and_then(|v| v.as_i64()).unwrap_or(0),
        });
    }
    let mut manual_mapping = BTreeMap::new();
    if let Some(map) = value.get("manual_mapping").and_then(|v| v.as_object()) {
        for (key, value) in map {
            manual_mapping.insert(key.clone(), value.as_str().unwrap_or("").to_string());
        }
    }
    Ok(StackupDocument {
        time_basis: value.get("time_basis").and_then(|v| v.as_str()).unwrap_or("unix milliseconds; project clock").into(),
        coordinate_basis: value.get("coordinate_basis").and_then(|v| v.as_str()).unwrap_or("lower-left board origin; derived values in mm").into(),
        layers,
        manual_mapping,
    })
}

fn stack_diag(message: &str) -> Diagnostic {
    Diagnostic { code: "STACK-JSON".into(), severity: DiagnosticSeverity::Error, message: message.into(), position: None, material_id: None }
}

fn require_string(value: &serde_json::Value, key: &str) -> Result<String, Diagnostic> {
    value.get(key).and_then(|v| v.as_str()).map(str::to_string).map_err(|_| stack_diag(&format!("missing string field {key}")))
}

fn parse_role(value: &str) -> Result<LayerRole, Diagnostic> {
    match value {
        "copper" => Ok(LayerRole::Copper),
        "solder_mask" => Ok(LayerRole::SolderMask),
        "drill_guide" => Ok(LayerRole::DrillGuide),
        "mechanical" => Ok(LayerRole::Mechanical),
        "plane" => Ok(LayerRole::Plane),
        other => Err(stack_diag(&format!("semantically unknown layer role: {other}"))),
    }
}
