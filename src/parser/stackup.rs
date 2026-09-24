//! Stackup manifest parser (small, explicit offline DSL).
//!
//! ```text
//! # comments
//! LAYER 1 copper order L1 name Top
//! LAYER 2 dielectric
//! LAYER 3 copper order L2 name Bottom
//! DRILL plated.drl plated
//! DRILL blind-l1-l2.drl plated range L1-L2
//! DRILL npth.drl nonplated
//! ```

use crate::error::{Diagnostic, DomainResult, FailureKind};
use crate::model::*;

use super::fail;

pub fn parse_stackup(text: &str) -> DomainResult<StackupDoc> {
    let mut layers: Vec<StackLayer> = Vec::new();
    let mut diag: Vec<Diagnostic> = Vec::new();

    for (idx0, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        let line_no = idx0 + 1;
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with("DRILL") {
            // Drill role declarations are advisory and not modeled as stack layers.
            continue;
        }
        let rest = match line.strip_prefix("LAYER") {
            Some(r) => r.trim(),
            None => {
                diag.push(Diagnostic::new(
                    FailureKind::CorruptFormat,
                    "STACKUP_BAD_DIRECTIVE",
                    format!("line {line_no}"),
                    format!("expected LAYER/DRILL directive: {line}"),
                ));
                continue;
            }
        };
        let tokens: Vec<&str> = rest.split_whitespace().collect();
        if tokens.len() < 2 {
            diag.push(Diagnostic::new(
                FailureKind::SemanticUnresolved,
                "STACKUP_SHORT",
                format!("line {line_no}"),
                "LAYER needs an index and a role",
            ));
            continue;
        }
        let physical_index = match tokens[0].parse::<u32>() {
            Ok(v) if v >= 1 => v,
            _ => {
                diag.push(Diagnostic::new(
                    FailureKind::SemanticUnresolved,
                    "STACKUP_BAD_INDEX",
                    format!("line {line_no}"),
                    format!("layer index '{}' is not a positive integer", tokens[0]),
                ).with_raw(tokens[0]));
                continue;
            }
        };
        let role = match LayerRole::parse(tokens[1]) {
            Some(r) => r,
            None => {
                // Dielectric/core are stack mechanics, retained as document role.
                match tokens[1].to_ascii_lowercase().as_str() {
                    "dielectric" | "core" | "prepreg" => LayerRole::Document,
                    _ => {
                        diag.push(Diagnostic::new(
                            FailureKind::SemanticUnresolved,
                            "STACKUP_BAD_ROLE",
                            format!("line {line_no}"),
                            format!("unknown layer role '{}'", tokens[1]),
                        ).with_raw(tokens[1]));
                        continue;
                    }
                }
            }
        };
        let mut copper_order = None;
        let mut material_id = None;
        let mut name_parts: Vec<&str> = Vec::new();
        let mut i = 2;
        while i < tokens.len() {
            match tokens[i] {
                "order" => {
                    if let Some(v) = tokens.get(i + 1).and_then(|t| t.trim_start_matches('L').parse::<u32>().ok()) {
                        copper_order = Some(v);
                        i += 2;
                        continue;
                    }
                    i += 1;
                }
                "material" => {
                    if let Some(v) = tokens.get(i + 1) {
                        material_id = Some(v.to_string());
                        i += 2;
                        continue;
                    }
                    i += 1;
                }
                "name" => {
                    if let Some(v) = tokens.get(i + 1) {
                        name_parts.push(*v);
                        i += 2;
                        continue;
                    }
                    i += 1;
                }
                other => {
                    name_parts.push(other);
                    i += 1;
                }
            }
        }
        let name = if name_parts.is_empty() {
            format!("layer-{physical_index}")
        } else {
            name_parts.join(" ")
        };
        layers.push(StackLayer {
            physical_index,
            role,
            copper_order,
            material_id,
            name,
            raw: raw_line.to_string(),
            line: line_no,
        });
    }

    if diag.iter().any(|d| d.kind == FailureKind::CorruptFormat) {
        return Err(fail(diag));
    }
    Ok(StackupDoc { layers, diagnostics: diag })
}
