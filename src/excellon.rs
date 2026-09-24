//! Excellon 钻孔文件解析（子集）。保留单位、刀具声明与原始坐标串。

use crate::model::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DrillTool {
    pub code: String,
    pub diameter: Qty,
    pub line: u32,
    pub raw: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DrillHit {
    pub line: u32,
    pub raw: String,
    pub x: Qty,
    pub y: Qty,
    pub tool: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParsedDrill {
    pub file: String,
    pub unit: Option<Unit>,
    pub unit_raw: Option<String>,
    pub tools: Vec<DrillTool>,
    pub hits: Vec<DrillHit>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn parse_drill(name: &str, src: &str) -> ParsedDrill {
    let mut d = ParsedDrill {
        file: name.into(),
        unit: None,
        unit_raw: None,
        tools: Vec::new(),
        hits: Vec::new(),
        diagnostics: Vec::new(),
    };
    let mut header = false;
    let mut tool = "".to_string();

    for (idx, raw_line) in src.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let pos = Some(SrcPos { file: name.into(), line: line_no, col: 1 });
        match line {
            "M48" => {
                header = true;
                continue;
            }
            "%" => {
                header = false;
                continue;
            }
            "M30" => break,
            _ => {}
        }
        if line.starts_with("METRIC") {
            d.unit = Some(Unit::Mm);
            d.unit_raw = Some(line.to_string());
            continue;
        }
        if line.starts_with("INCH") {
            d.unit = Some(Unit::Inch);
            d.unit_raw = Some(line.to_string());
            continue;
        }
        if line.starts_with('T') && header {
            // T1C0.8  或  T01C0.03
            let (code, rest) = line.split_at(line.find(|c: char| c == 'C' || c == 'F').unwrap_or(line.len()));
            let rest = rest.trim_start_matches('C');
            let raw = rest.to_string();
            let value = raw.parse().ok();
            if value.is_none() {
                d.diagnostics.push(Diagnostic {
                    code: "DRILL-TOOL-SIZE".into(),
                    level: DiagLevel::Error,
                    message: "刀具直径无法转换，原始值保留".into(),
                    pos,
                    raw_value: Some(line.to_string()),
                });
            }
            d.tools.push(DrillTool {
                code: code.to_string(),
                diameter: Qty {
                    raw,
                    unit: d.unit,
                    datum: Datum::ExcellonFile,
                    value_native: value,
                    pos,
                },
                line: line_no,
                raw: line.to_string(),
            });
            continue;
        }
        if line.starts_with('T') {
            // 选刀：T1
            let code: String = line[1..].chars().take_while(|c| c.is_ascii_digit()).collect();
            if !code.is_empty() {
                tool = format!("T{}", code.trim_start_matches('0'));
            }
            continue;
        }
        if line.starts_with('X') || line.starts_with('Y') {
            let xs = segment(line, 'X');
            let ys = segment(line, 'Y');
            let mk = |raw: &str, v: Option<f64>| Qty {
                raw: raw.to_string(),
                unit: d.unit,
                datum: Datum::ExcellonFile,
                value_native: v,
                pos,
            };
            let xraw = xs.unwrap_or("");
            let yraw = ys.unwrap_or("");
            let xv = xraw.parse().ok();
            let yv = yraw.parse().ok();
            if xv.is_none() || yv.is_none() {
                d.diagnostics.push(Diagnostic {
                    code: "DRILL-COORD".into(),
                    level: DiagLevel::Error,
                    message: "孔坐标无法转换，原始串保留".into(),
                    pos,
                    raw_value: Some(line.to_string()),
                });
            }
            d.hits.push(DrillHit {
                line: line_no,
                raw: line.to_string(),
                x: mk(xraw, xv),
                y: mk(yraw, yv),
                tool: tool.clone(),
            });
            continue;
        }
        d.diagnostics.push(Diagnostic {
            code: "DRILL-LINE".into(),
            level: DiagLevel::Warning,
            message: "无法解析的行，原样保留".into(),
            pos,
            raw_value: Some(line.to_string()),
        });
    }
    d
}

fn segment(line: &str, name: char) -> Option<&str> {
    let start = line.find(name)? + 1;
    let rest = &line[start..];
    let end = rest.find(|c: char| c == 'X' || c == 'Y').unwrap_or(rest.len());
    Some(&rest[..end])
}
