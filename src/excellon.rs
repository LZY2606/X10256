//! Excellon 钻孔文件解析（子集）。
//! 支持 M48/M72 或公制模式、T 刀具表、坐标行、G90 绝对。
//! 盲埋孔层范围通过注释指令声明：;SPAN L1-L3（层名与层叠说明对应）。
use crate::types::{Diagnostic, Failure, Severity, Unit};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tool {
    pub code: String, // T1
    /// 直径，文件单位
    pub diameter: f64,
    pub raw: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hole {
    pub tool: String,
    pub x_raw: String,
    pub y_raw: String,
    pub x: f64,
    pub y: f64,
    /// 盲埋孔层范围，如 ["L1","L3"]；None = 通孔（贯穿全部层）
    pub span: Option<(String, String)>,
    pub plated_hint: Option<bool>, // ;PLATED / ;NPTH
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrillFile {
    pub unit: Option<Unit>,
    pub unit_raw: Option<String>,
    pub tools: Vec<Tool>,
    pub holes: Vec<Hole>,
    pub diagnostics: Vec<Diagnostic>,
    pub failure: Option<Failure>,
}

pub fn parse(src: &str, material_id: &str) -> DrillFile {
    let mut f = DrillFile {
        unit: None,
        unit_raw: None,
        tools: Vec::new(),
        holes: Vec::new(),
        diagnostics: Vec::new(),
        failure: None,
    };
    let mut cur_tool: Option<String> = None;
    let mut pending_span: Option<(String, String)> = None;
    let mut pending_plated: Option<bool> = None;
    let mut in_header = true;
    let mut saw_m30 = false;

    for (idx, raw) in src.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if line == "M48" {
            in_header = true;
            continue;
        }
        if line == "M30" {
            saw_m30 = true;
            break;
        }
        if line == "%" {
            in_header = false;
            continue;
        }
        if line == "M72" {
            f.unit = Some(Unit::Inch);
            f.unit_raw = Some(line.to_string());
            continue;
        }
        if line == "M71" || line == "METRIC" {
            f.unit = Some(Unit::Mm);
            f.unit_raw = Some(line.to_string());
            continue;
        }
        if let Some(rest) = line.strip_prefix(';') {
            // 注释指令
            let rest = rest.trim();
            if let Some(span) = rest.strip_prefix("SPAN ") {
                let parts: Vec<&str> = span.split('-').collect();
                if parts.len() == 2 {
                    pending_span = Some((parts[0].trim().to_string(), parts[1].trim().to_string()));
                } else {
                    f.diagnostics.push(diag("SPAN", format!("盲埋孔层范围格式错误: {}", rest), line_no, material_id));
                }
            } else if rest == "PLATED" {
                pending_plated = Some(true);
            } else if rest == "NPTH" {
                pending_plated = Some(false);
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix('T') {
            if let Some(cpos) = rest.find('C') {
                let code = format!("T{}", &rest[..cpos]);
                match rest[cpos + 1..].parse::<f64>() {
                    Ok(d) => f.tools.push(Tool { code, diameter: d, raw: line.to_string() }),
                    Err(_) => {
                        f.failure = Some(Failure::CorruptFormat {
                            detail: format!("line {}: 刀具直径损坏: {}", line_no, line),
                        });
                    }
                }
            } else if rest.chars().all(|c| c.is_ascii_digit()) {
                cur_tool = Some(format!("T{}", rest));
            }
            continue;
        }
        if line.starts_with('X') || line.starts_with('Y') {
            if in_header {
                f.diagnostics.push(diag("HDR", "头部出现坐标行", line_no, material_id));
            }
            let tool = match &cur_tool {
                Some(t) => t.clone(),
                None => {
                    f.failure = Some(Failure::SemanticUnsolvable {
                        detail: format!("line {}: 坐标行之前未选择刀具", line_no),
                    });
                    continue;
                }
            };
            let (xs, ys) = split_xy(line);
            let (x, y) = match (xs.as_deref().and_then(parse_num), ys.as_deref().and_then(parse_num)) {
                (Some(x), Some(y)) => (x, y),
                _ => {
                    f.diagnostics.push(diag("XY", format!("坐标无法转换，保留原位置: {}", line), line_no, material_id));
                    continue;
                }
            };
            f.holes.push(Hole {
                tool,
                x_raw: xs.unwrap_or_default(),
                y_raw: ys.unwrap_or_default(),
                x,
                y,
                span: pending_span.take(),
                plated_hint: pending_plated.take(),
                line: line_no,
            });
            continue;
        }
        if line == "G90" || line == "G05" || line == "G80" {
            continue;
        }
        f.diagnostics.push(diag("??", format!("未识别行: {}", line), line_no, material_id));
    }
    if !saw_m30 {
        f.diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            code: "EOF".into(),
            message: "缺少 M30 结束标记".into(),
            material_id: Some(material_id.into()),
            line: None,
            column: None,
        });
    }
    if f.unit.is_none() && f.failure.is_none() {
        f.failure = Some(Failure::SemanticUnsolvable {
            detail: "缺少 M71/M72 单位声明，孔坐标没有单位".into(),
        });
    }
    f
}

fn diag(code: &str, msg: String, line: u32, mid: &str) -> Diagnostic {
    Diagnostic {
        severity: Severity::Error,
        code: code.into(),
        message: msg,
        material_id: Some(mid.into()),
        line: Some(line),
        column: None,
    }
}

fn split_xy(line: &str) -> (Option<String>, Option<String>) {
    let mut x = None;
    let mut y = None;
    let bytes = line.as_bytes();
    let mut k = 0;
    while k < bytes.len() {
        match bytes[k] as char {
            'X' | 'Y' => {
                let key = bytes[k] as char;
                let start = k + 1;
                let mut e = start;
                while e < bytes.len() && !matches!(bytes[e] as char, 'X' | 'Y') {
                    e += 1;
                }
                let v = line[start..e].to_string();
                if key == 'X' { x = Some(v) } else { y = Some(v) }
                k = e;
            }
            _ => k += 1,
        }
    }
    (x, y)
}

fn parse_num(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok()
}
