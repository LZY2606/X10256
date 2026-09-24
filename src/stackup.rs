//! 层叠说明解析（领域自定义文本格式）。
//! 声明铜层、阻焊面与 via（盲埋孔）的层范围。

use crate::model::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LayerDef {
    pub id: String,
    pub copper_order: Option<u32>,
    pub raw: String,
    pub line: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ViaDef {
    pub id: String,
    pub tool: String,
    pub from_layer: u32,
    pub to_layer: u32,
    pub raw: String,
    pub line: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParsedStackup {
    pub file: String,
    pub layers: Vec<LayerDef>,
    pub vias: Vec<ViaDef>,
    pub masks: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn parse_stackup(name: &str, src: &str) -> ParsedStackup {
    let mut s = ParsedStackup {
        file: name.into(),
        layers: Vec::new(),
        vias: Vec::new(),
        masks: Vec::new(),
        diagnostics: Vec::new(),
    };
    for (idx, raw_line) in src.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let pos = Some(SrcPos { file: name.into(), line: line_no, col: 1 });
        let parts: Vec<&str> = line.split_whitespace().collect();
        match parts.first().copied() {
            Some("layer") => {
                let id = parts.get(1).unwrap_or(&"").to_string();
                let kind = parts.get(2).copied().unwrap_or("");
                if !id.starts_with('L') || kind != "copper" {
                    s.diagnostics.push(Diagnostic::err(
                        "STACKUP-LAYER", "layer 声明应为 `layer L<n> copper`",
                        pos,
                    ));
                    continue;
                }
                let order = id.trim_start_matches('L').parse::<u32>().ok();
                if order.is_none() {
                    s.diagnostics.push(Diagnostic {
                        code: "STACKUP-LAYER-NUM".into(),
                        level: DiagLevel::Error,
                        message: "层号无法转换，原始行保留".into(),
                        pos,
                        raw_value: Some(line.to_string()),
                    });
                }
                s.layers.push(LayerDef { id, copper_order: order, raw: line.to_string(), line: line_no });
            }
            Some("mask") => {
                let side = parts.get(1).copied().unwrap_or("");
                if side != "top" && side != "bottom" {
                    s.diagnostics.push(Diagnostic::err(
                        "STACKUP-MASK", "mask 声明应为 `mask top|bottom`", pos,
                    ));
                    continue;
                }
                s.masks.push(side.to_string());
            }
            Some("via") => {
                // via V1 tool T1 from L1 to L2
                if parts.len() < 7 {
                    s.diagnostics.push(Diagnostic::err(
                        "STACKUP-VIA",
                        "via 声明应为 `via <id> tool T<n> from L<a> to L<b>`",
                        pos,
                    ));
                    continue;
                }
                let id = parts[1].to_string();
                let tool = parts[3].to_string();
                let from = parts[5].trim_start_matches('L').parse::<u32>().ok();
                let to = parts[6].trim_start_matches('L').parse::<u32>().ok();
                if !tool.starts_with('T') || from.is_none() || to.is_none() {
                    s.diagnostics.push(Diagnostic {
                        code: "STACKUP-VIA-SPAN".into(),
                        level: DiagLevel::Error,
                        message: "via 刀具或层范围无法转换，原始行保留".into(),
                        pos,
                        raw_value: Some(line.to_string()),
                    });
                    continue;
                }
                let (from, to) = (from.unwrap(), to.unwrap());
                if from > to {
                    s.diagnostics.push(Diagnostic::err(
                        "STACKUP-VIA-ORDER",
                        "via 起始层必须不大于终止层（盲埋孔范围非法）",
                        pos,
                    ));
                    continue;
                }
                s.vias.push(ViaDef { id, tool, from_layer: from, to_layer: to, raw: line.to_string(), line: line_no });
            }
            _ => s.diagnostics.push(Diagnostic {
                code: "STACKUP-LINE".into(),
                level: DiagLevel::Warning,
                message: "无法解析的行，原样保留".into(),
                pos,
                raw_value: Some(line.to_string()),
            }),
        }
    }
    s
}
