//! 层叠说明与手工层映射（JSON 子集，保留原始文本）。
use crate::types::{Diagnostic, Failure, Severity};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerRole {
    Copper,
    Dielectric,
    Soldermask,
    Drill,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StackLayer {
    pub name: String,        // L1..L4
    pub role: LayerRole,
    pub thickness: Option<f64>,
    pub unit: Option<crate::types::Unit>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stackup {
    pub layers: Vec<StackLayer>,
    /// 手工层映射：材料文件名 -> 层名（人工决定，可修订）
    pub manual_map: Vec<(String, String)>,
    pub diagnostics: Vec<Diagnostic>,
    pub failure: Option<Failure>,
}

pub fn parse(src: &str, material_id: &str) -> Stackup {
    let mut s = Stackup { layers: vec![], manual_map: vec![], diagnostics: vec![], failure: None };
    let v: serde_json::Value = match serde_json::from_str(src) {
        Ok(v) => v,
        Err(e) => {
            s.failure = Some(Failure::CorruptFormat { detail: format!("层叠说明 JSON 损坏: {}", e) });
            return s;
        }
    };
    let layers = match v.get("layers").and_then(|l| l.as_array()) {
        Some(a) => a,
        None => {
            s.failure = Some(Failure::SemanticUnsolvable { detail: "层叠说明缺少 layers 数组".into() });
            return s;
        }
    };
    for (i, l) in layers.iter().enumerate() {
        let name = l.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
        if name.is_empty() {
            s.diagnostics.push(Diagnostic {
                severity: Severity::Error,
                code: "STACK".into(),
                message: format!("第 {} 层缺少 name", i + 1),
                material_id: Some(material_id.into()),
                line: None,
                column: None,
            });
            continue;
        }
        let role = match l.get("role").and_then(|r| r.as_str()) {
            Some("copper") => LayerRole::Copper,
            Some("dielectric") => LayerRole::Dielectric,
            Some("soldermask") => LayerRole::Soldermask,
            Some("drill") => LayerRole::Drill,
            _ => LayerRole::Unknown,
        };
        let thickness = l.get("thickness").and_then(|t| t.as_f64());
        let unit = l.get("unit").and_then(|u| u.as_str()).map(|u| {
            if u == "inch" { crate::types::Unit::Inch } else { crate::types::Unit::Mm }
        });
        s.layers.push(StackLayer { name, role, thickness, unit });
    }
    if let Some(map) = v.get("manual_map").and_then(|m| m.as_object()) {
        for (k, val) in map {
            if let Some(layer) = val.as_str() {
                s.manual_map.push((k.clone(), layer.to_string()));
            }
        }
    }
    s
}
