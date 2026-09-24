//! Layer stack description (board house format): JSON manifest declaring
//! layers, their roles and the manual file->layer mapping.
//!
//! The manifest raw text is retained. Unknown roles become `other` rather than
//! forcing a default electrical interpretation.

use crate::domain::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestLayer {
    pub name: String,
    pub role: LayerRole,
    pub z_order: u32,
    pub thickness: Option<RawNum>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMapping {
    pub file: String,
    pub layer: String,
    /// For drill files: first/last copper z-order ("from" <= "to").
    pub span_from: Option<u32>,
    pub span_to: Option<u32>,
    pub plated: Option<bool>,
    pub role_override: Option<LayerRole>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub unit: Unit,
    pub layers: Vec<ManifestLayer>,
    pub mappings: Vec<FileMapping>,
    pub diags: Vec<Diag>,
    /// byte offset of a JSON pointer-ish path (best effort), for positions.
    pub raw: String,
}

pub fn parse(src: &str) -> Manifest {
    let mut diags = Vec::new();
    let value: serde_json::Value = match serde_json::from_str(src) {
        Ok(v) => v,
        Err(e) => {
            let line = e.line().max(1) as u32;
            let col = e.column().max(1) as u32;
            let offset = line_offset(src, line);
            diags.push(Diag::error("STK_JSON", format!("invalid stackup JSON: {e}"), Some(SourcePos::new(offset, line, col))).with_raw(src));
            return Manifest { unit: Unit::Unspecified, layers: Vec::new(), mappings: Vec::new(), diags, raw: src.to_string() };
        }
    };

    let unit = match value.get("unit").and_then(|v| v.as_str()) {
        Some("in") => Unit::In,
        Some("mm") => Unit::Mm,
        Some(other) => {
            diags.push(Diag::error("STK_UNIT", format!("unknown stackup unit {other}"), find_key(src, "unit")));
            Unit::Unspecified
        }
        None => {
            diags.push(Diag::error("STK_UNIT", "stackup missing unit declaration", find_key(src, "layers")));
            Unit::Unspecified
        }
    };

    let mut layers = Vec::new();
    if let Some(arr) = value.get("layers").and_then(|v| v.as_array()) {
        for (i, lv) in arr.iter().enumerate() {
            let name = lv.get("name").and_then(|v| v.as_str()).unwrap_or_default().to_string();
            let role = match parse_role(lv.get("role")) {
                Ok(r) => r,
                Err(msg) => {
                    diags.push(Diag::error("STK_ROLE", msg, Some(SourcePos::new(0, 1, 1))).with_raw(lv.to_string()));
                    LayerRole::Other { name: name.clone() }
                }
            };
            let z_order = lv.get("z_order").and_then(|v| v.as_u64()).unwrap_or(i as u64) as u32;
            let thickness = lv.get("thickness").and_then(|v| v.as_str()).map(|t| {
                let n = RawNum::new(t, unit, decimal_to_nm(t, unit));
                if n.nano.is_none() {
                    diags.push(Diag::warning("STK_THICKNESS", "layer thickness not convertible; retained raw", Some(SourcePos::new(0, 1, 1))).with_raw(t));
                }
                n
            });
            layers.push(ManifestLayer { name, role, z_order, thickness });
        }
    } else {
        diags.push(Diag::error("STK_LAYERS", "stackup missing layers array", find_key(src, "mappings")));
    }

    let mut mappings = Vec::new();
    if let Some(arr) = value.get("mappings").and_then(|v| v.as_array()) {
        for mv in arr.iter() {
            let file = mv.get("file").and_then(|v| v.as_str()).unwrap_or_default().to_string();
            let layer = mv.get("layer").and_then(|v| v.as_str()).unwrap_or_default().to_string();
            if file.is_empty() || layer.is_empty() {
                diags.push(Diag::error("STK_MAPPING", "mapping requires file and layer", Some(SourcePos::new(0, 1, 1))).with_raw(mv.to_string()));
            }
            let span_from = mv.get("span_from").and_then(|v| v.as_u64()).map(|n| n as u32);
            let span_to = mv.get("span_to").and_then(|v| v.as_u64()).map(|n| n as u32);
            if let (Some(a), Some(b)) = (span_from, span_to) {
                if a > b {
                    diags.push(Diag::error("STK_SPAN", "drill span_from must be <= span_to", Some(SourcePos::new(0, 1, 1))).with_raw(mv.to_string()));
                }
            }
            let role_override = match mv.get("role_override") {
                Some(v) => match parse_role(Some(v)) {
                    Ok(r) => Some(r),
                    Err(msg) => {
                        diags.push(Diag::warning("STK_ROLE", msg, Some(SourcePos::new(0, 1, 1))).with_raw(v.to_string()));
                        None
                    }
                },
                None => None,
            };
            mappings.push(FileMapping {
                file,
                layer,
                span_from,
                span_to,
                plated: mv.get("plated").and_then(|v| v.as_bool()),
                role_override,
            });
        }
    }

    Manifest { unit, layers, mappings, diags, raw: src.to_string() }
}

fn parse_role(v: Option<&serde_json::Value>) -> Result<LayerRole, String> {
    let Some(v) = v else {
        return Err("missing role".into());
    };
    if let Some(s) = v.as_str() {
        return Ok(match s {
            "copper_top" => LayerRole::CopperTop,
            "copper_bottom" => LayerRole::CopperBottom,
            "drill" => LayerRole::Drill,
            "soldermask_top" => LayerRole::SolderMaskTop,
            "soldermask_bottom" => LayerRole::SolderMaskBottom,
            other if other.starts_with("copper_inner_") => {
                let idx = other["copper_inner_".len()..].parse().map_err(|_| format!("bad inner role {other}"))?;
                LayerRole::CopperInner { index: idx }
            }
            other if other.starts_with("other:") => LayerRole::Other { name: other["other:".len()..].into() },
            other => LayerRole::Other { name: other.to_string() },
        });
    }
    if let Some(obj) = v.as_object() {
        if let Some(idx) = obj.get("index").and_then(|i| i.as_u64()) {
            match obj.get("kind").and_then(|k| k.as_str()) {
                Some("copper_inner") => return Ok(LayerRole::CopperInner { index: idx as u32 }),
                _ => {}
            }
        }
    }
    Err(format!("unparseable role {v}"))
}

fn line_offset(src: &str, line: u32) -> usize {
    let mut off = 0;
    for _ in 0..line.saturating_sub(1) {
        match src[off..].find('\n') {
            Some(p) => off += p + 1,
            None => break,
        }
    }
    off
}

fn find_key(src: &str, key: &str) -> Option<SourcePos> {
    let pat = format!("\"{key}\"");
    let off = src.find(&pat)?;
    let prefix = &src[..off];
    let line = prefix.matches('\n').count() as u32 + 1;
    let col = (off - prefix.rfind('\n').map(|p| p + 1).unwrap_or(0)) as u32 + 1;
    Some(SourcePos::new(off, line, col))
}
