//! Layer-stack description and manual layer mapping files.
//!
//! Both files are small line-oriented domain formats used by fabs. Every raw
//! line is retained; role revisions and duplicate manual mappings are kept
//! (not auto-resolved) so contradictions stay visible in the review page.

use crate::model::{Diagnostic, SrcPos};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum LayerRole {
    Copper,
    Soldermask,
    Silkscreen,
    Drill,
    Paste,
    Other,
}

impl LayerRole {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_uppercase().as_str() {
            "COPPER" | "COPPER_LAYER" | "CU" => Some(LayerRole::Copper),
            "SOLDERMASK" | "MASK" | "SM" => Some(LayerRole::Soldermask),
            "SILKSCREEN" | "SILK" | "SS" => Some(LayerRole::Silkscreen),
            "DRILL" | "HOLES" => Some(LayerRole::Drill),
            "PASTE" => Some(LayerRole::Paste),
            "OTHER" => Some(LayerRole::Other),
            _ => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            LayerRole::Copper => "copper",
            LayerRole::Soldermask => "soldermask",
            LayerRole::Silkscreen => "silkscreen",
            LayerRole::Drill => "drill",
            LayerRole::Paste => "paste",
            LayerRole::Other => "other",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct StackLayer {
    /// 1-based physical position, top = 1.
    pub index: usize,
    pub name: String,
    pub role: LayerRole,
    pub thickness: Option<crate::model::Tagged<f64>>,
    pub raw_line: String,
    pub pos: Option<SrcPos>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct StackupDoc {
    pub layers: Vec<StackLayer>,
    pub raw_lines: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Parse a `.stack` file: `LAYER <index> <name> <ROLE> [thickness:mm]`.
/// `#` starts a comment.
pub fn parse_stackup(input: &str, file_id: &str) -> Result<StackupDoc, (StackupDoc, Vec<Diagnostic>)> {
    let mut doc = StackupDoc::default();
    for (idx, raw) in input.lines().enumerate() {
        let line = raw.trim();
        doc.raw_lines.push(raw.to_string());
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let pos = SrcPos {
            file_id: file_id.to_string(),
            line: idx + 1,
            col: 1,
            byte_start: 0,
            byte_end: raw.len(),
            snippet: line.to_string(),
        };
        let mut parts = line.split_whitespace();
        let head = parts.next().unwrap_or("");
        if head != "LAYER" {
            doc.diagnostics.push(
                Diagnostic::warn("STACKUP_UNKNOWN_LINE", format!("line `{line}` retained but not interpreted")).at(pos),
            );
            continue;
        }
        let index = match parts.next().and_then(|s| s.parse::<usize>().ok()) {
            Some(i) => i,
            None => {
                doc.diagnostics
                    .push(Diagnostic::err("STACKUP_BAD_INDEX", format!("bad layer index in `{line}`")).at(pos.clone()));
                continue;
            }
        };
        let name = parts.next().unwrap_or("").to_string();
        let role_tok = parts.next().unwrap_or("");
        let role = match LayerRole::parse(role_tok) {
            Some(r) => r,
            None => {
                doc.diagnostics.push(
                    Diagnostic::err("STACKUP_BAD_ROLE", format!("unknown layer role `{role_tok}`")).at(pos.clone()),
                );
                continue;
            }
        };
        let thickness = parts.next().and_then(|tok| {
            let (num, unit) = tok.split_once(':')?;
            let v: f64 = num.parse().ok()?;
            let u = crate::model::LengthUnit::parse(unit)?;
            Some(crate::model::Tagged::new(
                format!("{file_id}:l{index}:thickness"),
                v * u.to_mm(),
                "mm",
                format!("raw {num} {unit}"),
                format!("{file_id}#{0}", idx + 1),
            ))
        });
        doc.layers.push(StackLayer {
            index,
            name,
            role,
            thickness,
            raw_line: raw.to_string(),
            pos: Some(pos),
        });
    }
    // role-vs-name sanity is deferred to the engine (layer-role revision scenario).
    if doc.diagnostics.iter().any(|d| d.is_error()) {
        Err((doc.clone(), doc.diagnostics.clone()))
    } else {
        Ok(doc)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MappingEntry {
    /// Logical Gerber/drill file name as imported.
    pub file_name: String,
    /// 1-based stackup layer this file is manually assigned to.
    pub layer_index: usize,
    pub role: LayerRole,
    pub raw_line: String,
    pub pos: Option<SrcPos>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct MappingDoc {
    pub entries: Vec<MappingEntry>,
    pub raw_lines: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Parse a `.map` file: `MAP <file> LAYER <index> ROLE <role>`.
/// Duplicate `file` rows are kept and reported as a semantic contradiction.
pub fn parse_mapping(input: &str, file_id: &str) -> Result<MappingDoc, (MappingDoc, Vec<Diagnostic>)> {
    let mut doc = MappingDoc::default();
    for (idx, raw) in input.lines().enumerate() {
        let line = raw.trim();
        doc.raw_lines.push(raw.to_string());
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let pos = SrcPos {
            file_id: file_id.to_string(),
            line: idx + 1,
            col: 1,
            byte_start: 0,
            byte_end: raw.len(),
            snippet: line.to_string(),
        };
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if tokens.len() < 6 || tokens[0] != "MAP" || tokens[2] != "LAYER" || tokens[4] != "ROLE" {
            doc.diagnostics.push(
                Diagnostic::err("MAP_BAD_LINE", format!("expected `MAP <file> LAYER <n> ROLE <role>`, got `{line}`"))
                    .at(pos),
            );
            continue;
        }
        let layer_index = match tokens[3].parse::<usize>() {
            Ok(n) => n,
            Err(_) => {
                doc.diagnostics
                    .push(Diagnostic::err("MAP_BAD_LAYER", format!("bad layer index in `{line}`")).at(pos.clone()));
                continue;
            }
        };
        let role = match LayerRole::parse(tokens[5]) {
            Some(r) => r,
            None => {
                doc.diagnostics
                    .push(Diagnostic::err("MAP_BAD_ROLE", format!("unknown role `{}`", tokens[5])).at(pos.clone()));
                continue;
            }
        };
        doc.entries.push(MappingEntry {
            file_name: tokens[1].to_string(),
            layer_index,
            role,
            raw_line: raw.to_string(),
            pos: Some(pos),
        });
    }
    let mut seen: std::collections::BTreeMap<String, usize> = Default::default();
    for e in &doc.entries {
        if let Some(prev) = seen.get(&e.file_name) {
            doc.diagnostics.push(
                Diagnostic::err(
                    "MAP_DUPLICATE",
                    format!(
                        "file `{}` manually mapped twice (first at line {prev}, again at line {})",
                        e.file_name,
                        e.pos.as_ref().map(|p| p.line).unwrap_or(0)
                    ),
                )
                .at(e.pos.clone().unwrap()),
            );
        } else {
            seen.insert(e.file_name.clone(), e.pos.as_ref().map(|p| p.line).unwrap_or(0));
        }
    }
    if doc.diagnostics.iter().any(|d| d.is_error()) {
        Err((doc.clone(), doc.diagnostics.clone()))
    } else {
        Ok(doc)
    }
}
