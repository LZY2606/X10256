use crate::gerber::{Budget, ParseOutcome};
use crate::model::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DrillTool {
    pub code: u32,
    pub diameter: Nm,
    /// 原始定义行（原单位保留）。
    pub raw: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DrillHole {
    pub tool: u32,
    pub x: Nm,
    pub y: Nm,
    pub line: usize,
    /// 原始坐标串。
    pub raw: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DrillDoc {
    pub unit: Unit,
    pub tools: Vec<DrillTool>,
    pub holes: Vec<DrillHole>,
    pub diagnostics: Vec<Diagnostic>,
}

/// 解析 Excellon 子集：M48/METRIC|INCH/T 定义/坐标/M30。
pub fn parse_excellon(content: &str, material_id: &str, budget: &mut Budget) -> Result<ParseOutcome2, FailureKind> {
    if !content.contains("M48") {
        return Ok(ParseOutcome2::Corrupt("缺少 M48 头：不是可识别的 Excellon".to_string()));
    }
    if !content.contains("M30") {
        return Ok(ParseOutcome2::Corrupt("缺少 M30 结束符：文件截断或半写".to_string()));
    }
    let mid = Some(material_id);
    let mut unit: Option<Unit> = None;
    let mut tools: Vec<DrillTool> = Vec::new();
    let mut holes: Vec<DrillHole> = Vec::new();
    let mut diags: Vec<Diagnostic> = Vec::new();
    let mut cur_tool: Option<u32> = None;
    let mut in_header = true;

    for (ln, raw) in content.lines().enumerate() {
        let line = raw.trim();
        budget.spend(1)?;
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if line == "M48" {
            continue;
        }
        if line == "M30" {
            break;
        }
        if line == "%" {
            in_header = false;
            continue;
        }
        if line.starts_with("METRIC") {
            unit = Some(Unit::Mm);
            continue;
        }
        if line.starts_with("INCH") {
            unit = Some(Unit::Inch);
            continue;
        }
        if let Some(rest) = line.strip_prefix('T') {
            // 工具定义：T1C0.300；或（在 body 中）工具选择：T1
            if let Some(cpos) = rest.find('C') {
                let code: u32 = rest[..cpos].parse().unwrap_or(0);
                let dia_str = &rest[cpos + 1..];
                let dia: f64 = match dia_str.parse() {
                    Ok(v) => v,
                    Err(_) => {
                        diags.push(Diagnostic::error("bad-tool", "刀具直径无法解析", mid, Some(ln + 1), Some(line)));
                        continue;
                    }
                };
                let u = match unit {
                    Some(u) => u,
                    None => {
                        diags.push(Diagnostic::error("missing-unit", "刀具定义前缺少 METRIC/INCH", mid, Some(ln + 1), Some(line)));
                        continue;
                    }
                };
                tools.push(DrillTool { code, diameter: u.to_nm(dia), raw: line.to_string() });
            } else if !in_header {
                cur_tool = rest.trim().parse().ok();
                if cur_tool.is_none() {
                    diags.push(Diagnostic::error("bad-tool-select", "刀具选择无法解析", mid, Some(ln + 1), Some(line)));
                }
            }
            continue;
        }
        if line.starts_with('X') || line.starts_with('Y') {
            let u = match unit {
                Some(u) => u,
                None => {
                    diags.push(Diagnostic::error("missing-unit", "坐标出现前缺少 METRIC/INCH", mid, Some(ln + 1), Some(line)));
                    continue;
                }
            };
            let tool = match cur_tool {
                Some(t) => t,
                None => {
                    diags.push(Diagnostic::error("no-tool", "钻孔坐标前未选择刀具", mid, Some(ln + 1), Some(line)));
                    continue;
                }
            };
            if !tools.iter().any(|t| t.code == tool) {
                diags.push(Diagnostic::error("undefined-tool", "使用了未定义的刀具", mid, Some(ln + 1), Some(line)));
                continue;
            }
            let mut x: Option<Nm> = None;
            let mut y: Option<Nm> = None;
            for part in line.split_whitespace().flat_map(|p| {
                // 同行可能 X..Y.. 相连
                let mut v: Vec<String> = Vec::new();
                let mut cur = String::new();
                for c in p.chars() {
                    if (c == 'X' || c == 'Y') && !cur.is_empty() {
                        v.push(std::mem::take(&mut cur));
                    }
                    cur.push(c);
                }
                if !cur.is_empty() {
                    v.push(cur);
                }
                v
            }) {
                if let Some(rest) = part.strip_prefix('X') {
                    x = rest.parse::<f64>().ok().map(|v| u.to_nm(v));
                } else if let Some(rest) = part.strip_prefix('Y') {
                    y = rest.parse::<f64>().ok().map(|v| u.to_nm(v));
                }
            }
            match (x, y) {
                (Some(x), Some(y)) => holes.push(DrillHole { tool, x, y, line: ln + 1, raw: line.to_string() }),
                _ => diags.push(Diagnostic::error("bad-coord", "钻孔坐标不完整，保留原位未规范化", mid, Some(ln + 1), Some(line))),
            }
        }
    }
    let u = match unit {
        Some(u) => u,
        None => {
            diags.push(Diagnostic::error("incomplete-header", "缺少 METRIC/INCH 单位声明", mid, None, None));
            Unit::Mm
        }
    };
    Ok(ParseOutcome2::Ok(DrillDoc { unit: u, tools, holes, diagnostics: diags }))
}

pub enum ParseOutcome2 {
    Ok(DrillDoc),
    Corrupt(String),
}

// 与 gerber::ParseOutcome 对齐的便捷转换
pub enum DrillParse {
    Ok(DrillDoc),
    Corrupt(String),
}

pub fn parse_excellon2(content: &str, material_id: &str, budget: &mut Budget) -> Result<DrillParse, FailureKind> {
    match parse_excellon(content, material_id, budget)? {
        ParseOutcome2::Ok(d) => Ok(DrillParse::Ok(d)),
        ParseOutcome2::Corrupt(m) => Ok(DrillParse::Corrupt(m)),
    }
}
