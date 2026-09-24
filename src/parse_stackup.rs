//! Stackup description parser (small domain-specific text format).
//!
//! ```text
//! UNIT MM
//! LAYER 1 top        signal
//! LAYER 2 gnd        plane
//! DRILL_SPAN via1    top -> gnd
//! DRILL_SPAN thru    top -> bottom
//! ```
//! Layer roles and blind/buried spans are retained verbatim (role_raw) plus
//! normalised to [`LayerRole`]; unknown roles stay in place with diagnostics.

use crate::domain::*;

pub fn parse(source_id: &str, content: &str) -> StackupDoc {
    let mut unit = None;
    let mut layers = Vec::new();
    let mut drill_spans = Vec::new();
    let mut diagnostics = Vec::new();

    for (idx, raw_line) in content.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let upper = line.to_ascii_uppercase();
        if let Some(rest) = upper.strip_prefix("UNIT") {
            match Unit::parse(rest.trim()) {
                Some(u) => unit = Some(u),
                None => diagnostics.push(Diagnostic {
                    code: "STACKUP_UNIT_BAD".into(),
                    severity: Severity::Error,
                    line: line_no,
                    col: 1,
                    message: "unit must be MM or IN; raw retained".into(),
                    raw: line.into(),
                }),
            }
            continue;
        }
        if upper.starts_with("LAYER") {
            let rest = line[5..].trim_start();
            let mut it = rest.split_whitespace();
            let order = it.next().and_then(|v| v.parse::<i64>().ok());
            let name = it.next().map(|s| s.to_string());
            let role_raw = it.next().map(|s| s.to_string());
            match (order, name, role_raw) {
                (Some(order), Some(name), Some(role_raw)) => {
                    match LayerRole::parse(&role_raw) {
                        Some(role) => layers.push(StackLayer {
                            order, name, role, role_raw, line: line_no,
                        }),
                        None => {
                            diagnostics.push(Diagnostic {
                                code: "STACKUP_ROLE_UNKNOWN".into(),
                                severity: Severity::Error,
                                line: line_no,
                                col: 1,
                                message: "unknown layer role; kept raw, not defaulted".into(),
                                raw: role_raw.clone(),
                            });
                            layers.push(StackLayer {
                                order,
                                name,
                                role: LayerRole::Substrate,
                                role_raw,
                                line: line_no,
                            });
                        }
                    }
                }
                _ => diagnostics.push(Diagnostic {
                    code: "STACKUP_LAYER_MALFORMED".into(),
                    severity: Severity::Error,
                    line: line_no,
                    col: 1,
                    message: "LAYER needs <order> <name> <role>".into(),
                    raw: line.into(),
                }),
            }
            continue;
        }
        if upper.starts_with("DRILL_SPAN") {
            let rest = &line["DRILL_SPAN".len()..];
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() == 4 && parts[2] == "->" {
                drill_spans.push(DrillSpanDecl {
                    name: parts[0].to_string(),
                    first_layer: parts[1].to_string(),
                    last_layer: parts[3].to_string(),
                    line: line_no,
                });
            } else {
                diagnostics.push(Diagnostic {
                    code: "STACKUP_SPAN_MALFORMED".into(),
                    severity: Severity::Error,
                    line: line_no,
                    col: 1,
                    message: "DRILL_SPAN needs <name> <first> -> <last>".into(),
                    raw: line.into(),
                });
            }
            continue;
        }
        diagnostics.push(Diagnostic {
            code: "STACKUP_LINE_UNKNOWN".into(),
            severity: Severity::Error,
            line: line_no,
            col: 1,
            message: "unrecognised stackup directive".into(),
            raw: line.into(),
        });
    }

    StackupDoc {
        source_id: source_id.to_string(),
        unit,
        layers,
        drill_spans,
        diagnostics,
    }
}
