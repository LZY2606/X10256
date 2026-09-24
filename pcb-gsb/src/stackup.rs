//! Stackup & layer mapping description parser.
//!
//! The stackup is the authoritative *physical* layer order. File-to-layer binding
//! (`MAP`) is a manual, revisable decision and is always retained with the raw line.

use crate::model::{LayerRole, Unit};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StackLayer {
    pub index: u32,
    pub name: String,
    pub role: LayerRole,
    pub thickness: Option<f64>,
    pub thickness_raw: Option<String>,
    pub mapped_file: Option<String>,
    pub raw_line: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StackupDoc {
    pub source_id: String,
    pub layers: Vec<StackLayer>,
    pub raw_text: String,
    pub unit: Unit,
    pub warnings: Vec<String>,
}

impl StackupDoc {
    pub fn parse(source_id: &str, text: &str) -> StackupDoc {
        let mut layers = Vec::new();
        let mut warnings = Vec::new();
        let mut unit = Unit::Mm;
        for (idx, raw_line) in text.lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let upper = line.to_ascii_uppercase();
            if upper.starts_with("UNIT") {
                let val = line.splitn(2, '=').nth(1).unwrap_or("").trim();
                unit = match val.to_ascii_uppercase().as_str() {
                    "MM" | "MILLIMETRE" | "MILLIMETER" => Unit::Mm,
                    "IN" | "INCH" => Unit::In,
                    _ => {
                        warnings.push(format!("line {}: unknown unit `{val}`, defaulting to mm", idx + 1));
                        Unit::Mm
                    }
                };
                continue;
            }
            if upper.starts_with("LAYER") {
                // LAYER 1 top TOP_COPPER 0.035
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() < 4 {
                    warnings.push(format!("line {}: malformed LAYER row retained as-is", idx + 1));
                    continue;
                }
                let index = parts[1].parse::<u32>().unwrap_or_else(|_| {
                    warnings.push(format!("line {}: bad layer index", idx + 1));
                    (layers.len() as u32) + 1
                });
                let name = parts[2].to_string();
                let role = LayerRole::parse(parts[3]);
                if role == LayerRole::Unmapped {
                    warnings.push(format!("line {}: role `{}` unrecognised", idx + 1, parts[3]));
                }
                let thickness_raw = parts.get(4).map(|s| s.to_string());
                let thickness = thickness_raw.as_ref().and_then(|s| s.parse::<f64>().ok());
                layers.push(StackLayer {
                    index,
                    name,
                    role,
                    thickness,
                    thickness_raw,
                    mapped_file: None,
                    raw_line: line.to_string(),
                });
            } else if upper.starts_with("MAP") {
                // MAP top.gbr -> top
                if let Some((file, layer_name)) = line.split_once("->") {
                    let file = file.trim_start_matches("MAP").trim().trim_matches('"').to_string();
                    let layer_name = layer.trim().trim_matches('"').to_string();
                    if let Some(l) = layers.iter_mut().find(|l| l.name == layer_name) {
                        l.mapped_file = Some(file);
                        l.raw_line.push_str("  ||  ");
                        l.raw_line.push_str(line);
                    } else {
                        warnings.push(format!("line {}: MAP target layer `{layer_name}` not found", idx + 1));
                    }
                } else {
                    warnings.push(format!("line {}: malformed MAP line", idx + 1));
                }
            }
        }
        StackupDoc {
            source_id: source_id.into(),
            layers,
            raw_text: text.into(),
            unit,
            warnings,
        }
    }

    pub fn copper_layers(&self) -> Vec<&StackLayer> {
        self.layers.iter().filter(|l| l.role.is_copper()).collect()
    }
    pub fn layer_count(&self) -> u32 {
        self.layers.len() as u32
    }
}
