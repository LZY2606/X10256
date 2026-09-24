use crate::model::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct StepRepeat {
    pub nx: u32,
    pub ny: u32,
    pub dx: Nm,
    pub dy: Nm,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum ApertureShape {
    Circle { d: Nm },
    Rect { w: Nm, h: Nm },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Aperture {
    pub code: u32,
    pub shape: ApertureShape,
    /// 原始定义行（原单位/原格式保留）。
    pub raw: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum OpKind {
    Flash { ap: u32, x: Nm, y: Nm },
    Draw { ap: u32, from: (Nm, Nm), to: (Nm, Nm) },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GerberOp {
    pub index: usize,
    pub line: usize,
    /// 原始坐标串（未规范化），供界面回显原文件表示。
    pub raw: String,
    pub polarity: Polarity,
    pub sr: Option<StepRepeat>,
    pub kind: OpKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GerberDoc {
    pub unit: Unit,
    pub format: (u8, u8),
    pub apertures: Vec<Aperture>,
    pub ops: Vec<GerberOp>,
    pub diagnostics: Vec<Diagnostic>,
}

pub enum ParseOutcome {
    Ok(GerberDoc),
    /// 格式损坏：无法作为 Gerber 解释（如截断/半写文件）。
    Corrupt(String),
}

pub struct Budget {
    pub remaining: u64,
}

impl Budget {
    pub fn spend(&mut self, n: u64) -> Result<(), FailureKind> {
        if self.remaining < n {
            return Err(FailureKind::BudgetExhausted);
        }
        self.remaining -= n;
        Ok(())
    }
}

fn parse_coord(s: &str, dec: u8, unit: Unit) -> Option<Nm> {
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let v: f64 = s.parse().ok()?;
    let scale = 10f64.powi(dec as i32);
    Some(unit.to_nm(v / scale))
}

/// 解析 Gerber 子集：FS/MO/AD/LP/SR/D 码/坐标。原始行全部保留在 op.raw。
pub fn parse_gerber(content: &str, material_id: &str, budget: &mut Budget) -> Result<ParseOutcome, FailureKind> {
    if !content.contains("M02") {
        return Ok(ParseOutcome::Corrupt(
            "缺少 M02 结束符：文件截断或不是 Gerber".to_string(),
        ));
    }
    let mid = Some(material_id);
    let mut unit: Option<Unit> = None;
    let mut format: Option<(u8, u8)> = None;
    let mut apertures: Vec<Aperture> = Vec::new();
    let mut ops: Vec<GerberOp> = Vec::new();
    let mut diags: Vec<Diagnostic> = Vec::new();
    let mut polarity = Polarity::Positive;
    let mut sr: Option<StepRepeat> = None;
    let mut cur_ap: Option<u32> = None;
    let mut cur_pt: (Nm, Nm) = (0, 0);
    let mut op_index = 0usize;

    for (ln, raw_line) in content.lines().enumerate() {
        let line = raw_line.trim();
        budget.spend(1)?;
        if line.is_empty() || line.starts_with("G04") {
            continue;
        }
        if line.starts_with('%') {
            let body = line.trim_matches('%');
            for stmt in body.split('*').filter(|s| !s.is_empty()) {
                if let Some(rest) = stmt.strip_prefix("FS") {
                    // 例：LAX46Y46
                    let digits: String = rest.chars().filter(|c| c.is_ascii_digit()).collect();
                    if digits.len() >= 4 {
                        let xi = digits.as_bytes()[0] - b'0';
                        let xd = digits.as_bytes()[1] - b'0';
                        format = Some((xi, xd));
                    } else {
                        diags.push(Diagnostic::error("bad-fs", "FS 指令无法解析", mid, Some(ln + 1), Some(line)));
                    }
                } else if let Some(rest) = stmt.strip_prefix("MO") {
                    unit = match rest {
                        "MM" => Some(Unit::Mm),
                        "IN" => Some(Unit::Inch),
                        _ => {
                            diags.push(Diagnostic::error("bad-unit", "MO 单位无法识别", mid, Some(ln + 1), Some(line)));
                            None
                        }
                    };
                } else if let Some(rest) = stmt.strip_prefix("AD") {
                    // 例：D10C,0.010 或 D11R,0.020X0.010
                    let rest = rest.trim_start_matches('D');
                    let split = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
                    let code: u32 = rest[..split].parse().unwrap_or(0);
                    let def = &rest[split..];
                    let shape_letter: String = def.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
                    let params = &def[shape_letter.len()..];
                    let u = match unit {
                        Some(u) => u,
                        None => {
                            diags.push(Diagnostic::error(
                                "missing-unit",
                                "AD 出现前缺少 MO 单位声明，无法规范化 aperture",
                                mid,
                                Some(ln + 1),
                                Some(line),
                            ));
                            continue;
                        }
                    };
                    let nums: Vec<f64> = params
                        .trim_start_matches(',')
                        .split('X')
                        .filter_map(|p| p.parse::<f64>().ok())
                        .collect();
                    let shape = match shape_letter.as_str() {
                        "C" if !nums.is_empty() => Some(ApertureShape::Circle { d: u.to_nm(nums[0]) }),
                        "R" if nums.len() >= 2 => Some(ApertureShape::Rect { w: u.to_nm(nums[0]), h: u.to_nm(nums[1]) }),
                        _ => None,
                    };
                    match shape {
                        Some(s) => apertures.push(Aperture { code, shape: s, raw: line.to_string() }),
                        None => diags.push(Diagnostic::error(
                            "unsupported-aperture",
                            "不支持的 aperture 定义，原样保留但不参与规范化",
                            mid,
                            Some(ln + 1),
                            Some(line),
                        )),
                    }
                } else if stmt == "LPD" {
                    polarity = Polarity::Positive;
                } else if stmt == "LPC" {
                    polarity = Polarity::Negative;
                } else if let Some(rest) = stmt.strip_prefix("SR") {
                    if rest.is_empty() {
                        sr = None;
                    } else {
                        let u = match unit {
                            Some(u) => u,
                            None => {
                                diags.push(Diagnostic::error("missing-unit", "SR 出现前缺少 MO 单位声明", mid, Some(ln + 1), Some(line)));
                                continue;
                            }
                        };
                        let get = |k: char| -> Option<String> {
                            let mut out = String::new();
                            let mut take = false;
                            for c in rest.chars() {
                                if c == k {
                                    take = true;
                                    continue;
                                }
                                if take {
                                    if c.is_ascii_digit() || c == '.' {
                                        out.push(c);
                                    } else {
                                        break;
                                    }
                                }
                            }
                            if out.is_empty() { None } else { Some(out) }
                        };
                        let nx: u32 = get('X').and_then(|s| s.parse().ok()).unwrap_or(1);
                        let ny: u32 = get('Y').and_then(|s| s.parse().ok()).unwrap_or(1);
                        let dx = get('I').and_then(|s| s.parse::<f64>().ok()).map(|v| u.to_nm(v)).unwrap_or(0);
                        let dy = get('J').and_then(|s| s.parse::<f64>().ok()).map(|v| u.to_nm(v)).unwrap_or(0);
                        sr = Some(StepRepeat { nx, ny, dx, dy });
                    }
                }
            }
            continue;
        }
        // 非扩展命令行：可能含 D 码选择 / 坐标。
        let stmt = line.trim_end_matches('*');
        if stmt == "M02" || stmt == "M00" || stmt == "M01" {
            continue;
        }
        if stmt.starts_with('G') && !stmt.contains('X') && !stmt.contains('Y') && !stmt.contains('D') {
            continue; // G 码（如 G01/G75）本子集忽略
        }
        if let Some(rest) = stmt.strip_prefix('D') {
            if rest.chars().all(|c| c.is_ascii_digit()) && !rest.is_empty() {
                let code: u32 = rest.parse().unwrap_or(0);
                if code >= 10 {
                    cur_ap = Some(code);
                    continue;
                }
            }
        }
        // 坐标命令
        let (fx, fy) = match format {
            Some(f) => (f.1, f.1),
            None => {
                diags.push(Diagnostic::error("missing-format", "坐标出现前缺少 FS 格式声明", mid, Some(ln + 1), Some(line)));
                continue;
            }
        };
        let u = match unit {
            Some(u) => u,
            None => {
                diags.push(Diagnostic::error("missing-unit", "坐标出现前缺少 MO 单位声明", mid, Some(ln + 1), Some(line)));
                continue;
            }
        };
        let mut x: Option<Nm> = None;
        let mut y: Option<Nm> = None;
        let mut dcode: Option<u32> = None;
        let bytes = stmt.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i] as char;
            let mut j = i + 1;
            while j < bytes.len() && (bytes[j] as char).is_ascii_digit() {
                j += 1;
            }
            let tok = &stmt[i + 1..j];
            match c {
                'X' => x = parse_coord(tok, fx, u),
                'Y' => y = parse_coord(tok, fy, u),
                'D' => dcode = tok.parse().ok(),
                'G' => {}
                _ => {}
            }
            i = j.max(i + 1);
        }
        let nx = x.unwrap_or(cur_pt.0);
        let ny = y.unwrap_or(cur_pt.1);
        let ap = match cur_ap {
            Some(a) => a,
            None => {
                diags.push(Diagnostic::error("no-aperture", "坐标操作前未选择 D 码", mid, Some(ln + 1), Some(line)));
                continue;
            }
        };
        if !apertures.iter().any(|a| a.code == ap) {
            diags.push(Diagnostic::error("undefined-aperture", "使用了未定义的 D 码", mid, Some(ln + 1), Some(line)));
            continue;
        }
        match dcode {
            Some(3) => {
                ops.push(GerberOp {
                    index: op_index,
                    line: ln + 1,
                    raw: line.to_string(),
                    polarity,
                    sr,
                    kind: OpKind::Flash { ap, x: nx, y: ny },
                });
                op_index += 1;
            }
            Some(1) => {
                ops.push(GerberOp {
                    index: op_index,
                    line: ln + 1,
                    raw: line.to_string(),
                    polarity,
                    sr,
                    kind: OpKind::Draw { ap, from: cur_pt, to: (nx, ny) },
                });
                op_index += 1;
            }
            Some(2) | None => {}
            _ => {}
        }
        cur_pt = (nx, ny);
    }

    let (unit, format) = match (unit, format) {
        (Some(u), Some(f)) => (u, f),
        _ => {
            // 输入不完整：带位置诊断，绝不默认单位伪装成功。
            return Ok(ParseOutcome::Ok(GerberDoc {
                unit: unit.unwrap_or(Unit::Mm),
                format: format.unwrap_or((0, 0)),
                apertures,
                ops,
                diagnostics: {
                    diags.push(Diagnostic::error(
                        "incomplete-header",
                        "缺少 FS 或 MO，文件头不完整；坐标值保留原位，未做规范化",
                        mid,
                        None,
                        None,
                    ));
                    diags
                },
            }));
        }
    };
    Ok(ParseOutcome::Ok(GerberDoc { unit, format, apertures, ops, diagnostics: diags }))
}
