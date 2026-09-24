//! Excellon 钻孔解析器：刀具、沉铜/不沉铜、盲埋孔 span 原样保留。

use crate::model::*;

pub const MAX_HOLES: usize = 200_000;

pub fn parse_excellon(text: &str, filename: &str) -> ParsedFile {
    let mut doc = ExcellonDoc {
        format: String::new(),
        unit: LengthUnit::Unknown,
        zero_suppression: "unknown".to_string(),
        datum: "absolute".to_string(),
        tools: Vec::new(),
        holes: Vec::new(),
    };
    let mut diags: Vec<Diagnostic> = Vec::new();
    let mut fmt: Option<(u32, u32)> = None;
    let mut current_tool: Option<DrillTool> = None;
    let mut x = 0.0f64;
    let mut y = 0.0f64;
    let mut count = 0usize;

    macro_rules! diag {
        ($kind:expr, $line:expr, $code:expr, $msg:expr, $raw:expr) => {
            diags.push(Diagnostic {
                kind: $kind,
                file: filename.to_string(),
                line: $line,
                code: $code.to_string(),
                message: $msg.to_string(),
                raw_snippet: $raw.chars().take(160).collect(),
            })
        };
    }

    for (idx, raw_line) in text.lines().enumerate() {
        let line_no = (idx + 1) as i64;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }

        if line == "M48" || line == "%" || line == "M95" || line == "M30" {
            continue;
        }
        if line == "METRIC" || line.starts_with("METRIC,") {
            doc.unit = LengthUnit::Mm;
            continue;
        }
        if line == "INCH" || line.starts_with("INCH,") {
            doc.unit = LengthUnit::Inch;
            continue;
        }
        if let Some(rest) = line.strip_prefix("FMAT") {
            doc.format = rest.trim_start_matches(',').to_string();
            continue;
        }
        if let Some(rest) = line.strip_prefix("ICI") {
            // 增量模式声明保留到 datum。
            doc.datum = if rest.contains("OFF") {
                "absolute".to_string()
            } else {
                "incremental".to_string()
            };
            continue;
        }
        if let Some(rest) = line.strip_prefix(";FORMAT") {
            // ;FORMAT 2:5
            let v = rest.trim_start_matches(',').trim();
            doc.format = v.to_string();
            if let Some((a, b)) = parse_pair(v) {
                fmt = Some((a, b));
            }
            continue;
        }

        // 刀具定义：T1C0.3 [PLATED|NPTH] [span=0:2]
        if let Some(rest) = line.strip_prefix('T') {
            if let Some(c_idx) = rest.find('C') {
                let code = format!("T{}", &rest[..c_idx]);
                let tail = &rest[c_idx + 1..];
                let mut parts = tail.split_whitespace();
                let diam_s = parts.next().unwrap_or("");
                let diam = match diam_s.parse::<f64>() {
                    Ok(v) => v,
                    Err(_) => {
                        diag!(
                            DiagnosticKind::CorruptFormat,
                            line_no,
                            "TOOL-DIAM",
                            "刀具直径无法解析",
                            line
                        );
                        continue;
                    }
                };
                if doc.unit == LengthUnit::Unknown {
                    diag!(
                        DiagnosticKind::SemanticUnresolved,
                        line_no,
                        "NO-UNIT",
                        "刀具声明前缺少 METRIC/INCH",
                        line
                    );
                    continue;
                }
                let rest2: Vec<&str> = parts.collect();
                let mut plated = true;
                let mut raw_span = "through".to_string();
                for token in &rest2 {
                    let upper = token.to_ascii_uppercase();
                    if upper == "NPTH" || upper == "NONPLATED" {
                        plated = false;
                    } else if upper == "PTH" || upper == "PLATED" {
                        plated = true;
                    } else if let Some(v) = token.strip_prefix("span=") {
                        raw_span = v.to_string();
                    }
                }
                if parse_span(&raw_span).is_none() {
                    diag!(
                        DiagnosticKind::SemanticUnresolved,
                        line_no,
                        "SPAN-FORMAT",
                        "盲埋孔 span 无法解析（应为 a:b 或 through）",
                        line
                    );
                    continue;
                }
                let tool = DrillTool {
                    code,
                    diameter: diam,
                    unit: doc.unit,
                    plated,
                    raw_span,
                };
                doc.tools.push(tool.clone());
                current_tool = Some(tool);
                continue;
            } else {
                // 单纯选刀 T1
                let code = format!("T{}", rest.trim_end_matches('*'));
                current_tool = doc.tools.iter().find(|t| t.code == code).cloned();
                if current_tool.is_none() {
                    diag!(
                        DiagnosticKind::SemanticUnresolved,
                        line_no,
                        "TOOL-MISSING",
                        "选择了未定义刀具",
                        line
                    );
                }
                continue;
            }
        }

        // 孔坐标 X..Y..
        if line.starts_with('X') || line.starts_with('Y') {
            let Some(tool) = current_tool.clone() else {
                diag!(
                    DiagnosticKind::SemanticUnresolved,
                    line_no,
                    "NO-TOOL",
                    "孔坐标前未选择刀具",
                    line
                );
                continue;
            };
            let Some((total, frac)) = fmt else {
                diag!(
                    DiagnosticKind::CorruptFormat,
                    line_no,
                    "NO-FORMAT",
                    "孔坐标前缺少 ;FORMAT a:b",
                    line
                );
                continue;
            };
            match parse_xy(line, total, frac) {
                Ok((nx, ny)) => {
                    if let Some(v) = nx {
                        x = v;
                    }
                    if let Some(v) = ny {
                        y = v;
                    }
                    count += 1;
                    if count > MAX_HOLES {
                        diag!(
                            DiagnosticKind::BudgetExhausted,
                            line_no,
                            "BUDGET-HOLES",
                            "孔数量超过预算",
                            line
                        );
                        break;
                    }
                    let span = parse_span(&tool.raw_span).unwrap_or_default();
                    doc.holes.push(Hole {
                        diameter: tool.diameter,
                        unit: tool.unit,
                        x,
                        y,
                        plated: tool.plated,
                        span,
                        raw_span: tool.raw_span.clone(),
                        source_line: line_no,
                    });
                }
                Err(msg) => diag!(
                    DiagnosticKind::CorruptFormat,
                    line_no,
                    "COORD",
                    msg,
                    line
                ),
            }
            continue;
        }

        diag!(
            DiagnosticKind::CorruptFormat,
            line_no,
            "UNKNOWN-LINE",
            "无法识别的 Excellon 行",
            line
        );
    }

    ParsedFile {
        kind: FileKind::Excellon.as_str().to_string(),
        gerber: None,
        excellon: Some(doc),
        stackup: None,
        diagnostics: diags,
    }
}

fn parse_pair(v: &str) -> Option<(u32, u32)> {
    let (a, b) = v.split_once(':')?;
    let ai: u32 = a.trim().parse().ok()?;
    let bi: u32 = b.trim().parse().ok()?;
    Some((ai + bi, bi))
}

/// 解析 span：`through` 或 `0:2`（含端点层）。
pub fn parse_span(raw: &str) -> Option<Vec<usize>> {
    if raw == "through" {
        return Some(Vec::new());
    }
    let (a, b) = raw.split_once(':')?;
    let ai: usize = a.trim().parse().ok()?;
    let bi: usize = b.trim().parse().ok()?;
    if ai > bi {
        return None;
    }
    Some((ai..=bi).collect())
}

fn parse_xy(line: &str, total: u32, frac: u32) -> Result<(Option<f64>, Option<f64>), String> {
    let body = line.trim_end_matches('*');
    let mut x: Option<f64> = None;
    let mut y: Option<f64> = None;
    let mut key: Option<char> = None;
    let mut buf = String::new();
    for ch in body.chars() {
        if ch == 'X' || ch == 'Y' {
            if let Some(k) = key {
                let v = decode(&buf, total, frac)?;
                if k == 'X' {
                    x = Some(v);
                } else {
                    y = Some(v);
                }
            }
            key = Some(ch);
            buf.clear();
        } else if ch.is_ascii_digit() || ch == '-' {
            buf.push(ch);
        } else if !ch.is_whitespace() {
            return Err(format!("非法字符 {ch}"));
        }
    }
    if let Some(k) = key {
        let v = decode(&buf, total, frac)?;
        if k == 'X' {
            x = Some(v);
        } else {
            y = Some(v);
        }
    }
    Ok((x, y))
}

fn decode(buf: &str, total: u32, frac: u32) -> Result<f64, String> {
    let neg = buf.starts_with('-');
    let digits: String = buf.chars().filter(|c| c.is_ascii_digit()).collect();
    let total = total as usize;
    let padded = if digits.len() < total {
        "0".repeat(total - digits.len()) + &digits
    } else {
        digits
    };
    let frac = frac as usize;
    let (int_part, frac_part) = padded.split_at(padded.len() - frac);
    let iv: f64 = int_part.parse().map_err(|_| "整数错误".to_string())?;
    let fv: f64 = if frac_part.is_empty() {
        0.0
    } else {
        frac_part.parse().map_err(|_| "小数错误".to_string())?
    };
    let scale = 10f64.powi(frac as i32);
    let mut v = iv + fv / scale;
    if neg {
        v = -v;
    }
    Ok(v)
}
