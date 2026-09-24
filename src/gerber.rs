//! 最小但完整可追溯的 Gerber 解析器（子集）。
//! 保留：坐标格式、单位、极性、aperture、step-repeat、原始坐标串与行位置。

use crate::model::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CoordFormat {
    pub x_int: u8,
    pub x_dec: u8,
    pub y_int: u8,
    pub y_dec: u8,
    pub zero: String,
    pub raw: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApertureDef {
    pub code: u32,
    pub aperture: Aperture,
    pub line: u32,
    pub raw: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Tok {
    SrStart { sr: StepRepeat, line: u32, raw: String },
    SrEnd { line: u32, raw: String },
    Flash { line: u32, raw: String, x: Qty, y: Qty, aperture: u32, polarity: Polarity },
    Draw { line: u32, raw: String, x: Qty, y: Qty, aperture: u32, polarity: Polarity },
    Move { line: u32, raw: String, x: Qty, y: Qty },
    RegionIgnored { line: u32, raw: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParsedGerber {
    pub file: String,
    pub format: Option<CoordFormat>,
    pub unit: Option<Unit>,
    pub unit_raw: Option<String>,
    pub apertures: Vec<ApertureDef>,
    pub tokens: Vec<Tok>,
    pub diagnostics: Vec<Diagnostic>,
}

/// L = 前导零省略（左侧补零），T = 尾随零省略（右侧补零）
pub fn parse_coord(digits: &str, int: u8, dec: u8, zero: &str) -> Option<f64> {
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit() || b == b'-') {
        return None;
    }
    let (neg, digits) = match digits.strip_prefix('-') {
        Some(d) => (true, d),
        None => (false, digits),
    };
    let total = (int + dec) as usize;
    let padded: String = if digits.len() < total {
        if zero == "T" {
            format!("{:0<width$}", digits, width = total)
        } else {
            format!("{:0>width$}", digits, width = total)
        }
    } else {
        digits.to_string()
    };
    let (whole, frac) = padded.split_at(total - dec as usize);
    let v: f64 = format!("{}.{}", whole, frac).parse().ok()?;
    Some(if neg { -v } else { v })
}

fn missing_qty(file: &str, line: u32) -> Qty {
    Qty {
        raw: "<missing>".into(),
        unit: None,
        datum: Datum::GerberFile,
        value_native: None,
        pos: Some(SrcPos { file: file.into(), line, col: 0 }),
    }
}

fn make_coord_qty(file: &str, line: u32, col: u32, is_x: bool, digits: &str, doc: &ParsedGerber, raw: &str) -> Qty {
    let (vi, vd, zero) = match &doc.format {
        Some(f) if is_x => (f.x_int, f.x_dec, f.zero.clone()),
        Some(f) => (f.y_int, f.y_dec, f.zero.clone()),
        None => (4, 6, "L".to_string()),
    };
    let value = if doc.format.is_some() {
        parse_coord(digits, vi, vd, &zero)
    } else {
        digits.parse().ok()
    };
    Qty {
        raw: raw.to_string(),
        unit: doc.unit,
        datum: Datum::GerberFile,
        value_native: value,
        pos: Some(SrcPos { file: file.to_string(), line, col }),
    }
}

fn field<'a>(body: &'a str, name: char) -> Option<&'a str> {
    let start = body.find(name)? + 1;
    let rest = &body[start..];
    let end = rest
        .find(|c: char| c == 'X' || c == 'Y' || c == 'D' || c == '*')
        .unwrap_or(rest.len());
    Some(&rest[..end])
}

fn parse_aperture(file: &str, line_no: u32, body: &str, doc: &mut ParsedGerber) {
    // ADD10C,0.010*  /  ADD11R,0.05X0.02*
    let rest = body.trim_start_matches("AD");
    let num_end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    let code: u32 = match rest[..num_end].parse() {
        Ok(v) => v,
        Err(_) => {
            doc.diagnostics.push(Diagnostic::err(
                "GERBER-APERTURE-CODE",
                "aperture 编号无法解析",
                Some(SrcPos { file: file.into(), line: line_no, col: 3 }),
            ));
            return;
        }
    };
    let spec = &rest[num_end..];
    let pos = Some(SrcPos { file: file.into(), line: line_no, col: 3 + num_end as u32 });
    let unit = doc.unit;
    if let Some(rest) = spec.strip_prefix('C') {
        let raw = rest.trim_start_matches(',').to_string();
        let value = raw.parse().ok();
        if value.is_none() {
            doc.diagnostics.push(Diagnostic {
                code: "GERBER-APERTURE-SIZE".into(),
                level: DiagLevel::Error,
                message: "圆形 aperture 直径无法转换，原始值保留在原位置".into(),
                pos,
                raw_value: Some(raw.clone()),
            });
        }
        doc.apertures.push(ApertureDef {
            code,
            aperture: Aperture::Circle {
                diameter: Qty { raw, unit, datum: Datum::GerberFile, value_native: value, pos },
            },
            line: line_no,
            raw: body.to_string(),
        });
    } else if let Some(rest) = spec.strip_prefix('R') {
        let inner = rest.trim_start_matches(',');
        let (wraw, hraw) = match inner.split_once('X') {
            Some(v) => v,
            None => {
                doc.diagnostics.push(Diagnostic::err(
                    "GERBER-APERTURE-RECT",
                    "矩形 aperture 缺少 X 分隔的宽高",
                    pos,
                ));
                return;
            }
        };
        let (wv, hv) = (wraw.parse().ok(), hraw.parse().ok());
        if wv.is_none() || hv.is_none() {
            doc.diagnostics.push(Diagnostic {
                code: "GERBER-APERTURE-SIZE".into(),
                level: DiagLevel::Error,
                message: "矩形 aperture 尺寸无法转换，原始值保留".into(),
                pos,
                raw_value: Some(inner.to_string()),
            });
        }
        doc.apertures.push(ApertureDef {
            code,
            aperture: Aperture::Rect {
                w: Qty { raw: wraw.to_string(), unit, datum: Datum::GerberFile, value_native: wv, pos },
                h: Qty { raw: hraw.to_string(), unit, datum: Datum::GerberFile, value_native: hv, pos },
            },
            line: line_no,
            raw: body.to_string(),
        });
    } else {
        doc.diagnostics.push(Diagnostic {
            code: "GERBER-APERTURE-UNSUPPORTED".into(),
            level: DiagLevel::Warning,
            message: "仅支持 C(圆) 与 R(矩形) aperture，其余类型保留原始声明但不展开".into(),
            pos,
            raw_value: Some(spec.to_string()),
        });
    }
}

pub fn parse_gerber(name: &str, src: &str) -> ParsedGerber {
    let mut doc = ParsedGerber {
        file: name.to_string(),
        format: None,
        unit: None,
        unit_raw: None,
        apertures: Vec::new(),
        tokens: Vec::new(),
        diagnostics: Vec::new(),
    };
    let mut polarity = Polarity::Add;
    let mut aperture: Option<u32> = None;
    let mut in_region = false;

    for (idx, raw_line) in src.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        // G04 注释（含 G04 This is a comment*）
        if line.starts_with("G04") {
            continue;
        }
        // 扩展命令
        if let Some(body) = line.strip_prefix('%') {
            let body = body.trim_start_matches('%');
            let body = body.strip_suffix('%').unwrap_or(body);
            let body = body.strip_suffix('*').unwrap_or(body);
            if body.starts_with("FS") {
                let l = body.contains('L');
                let t = body.contains('T');
                let zero = if l { "L" } else if t { "T" } else { "L" }.to_string();
                let nums: Vec<u8> = body.chars().filter_map(|c| c.to_digit(10).map(|d| d as u8)).collect();
                if nums.len() >= 4 {
                    doc.format = Some(CoordFormat {
                        x_int: nums[0], x_dec: nums[1], y_int: nums[2], y_dec: nums[3],
                        zero, raw: body.to_string(),
                    });
                } else {
                    doc.diagnostics.push(Diagnostic::err(
                        "GERBER-FS", "坐标格式 FS 声明不完整",
                        Some(SrcPos { file: name.into(), line: line_no, col: 1 }),
                    ));
                }
            } else if body == "MOIN" {
                doc.unit = Some(Unit::Inch);
                doc.unit_raw = Some(body.to_string());
            } else if body == "MOMM" {
                doc.unit = Some(Unit::Mm);
                doc.unit_raw = Some(body.to_string());
            } else if body.starts_with("AD") {
                parse_aperture(name, line_no, body, &mut doc);
            } else if body == "LPD" {
                polarity = Polarity::Add;
            } else if body == "LPC" {
                polarity = Polarity::Subtract;
            } else if body.starts_with("SR") && body.len() > 2 {
                // %SRX2Y3I0.1J0.1*%
                let nx = field(body, 'X').and_then(|s| s.parse().ok()).unwrap_or(1);
                let ny = field(body, 'Y').and_then(|s| s.parse().ok()).unwrap_or(1);
                let dx_raw = field(body, 'I').unwrap_or("0").to_string();
                let dy_raw = field(body, 'J').unwrap_or("0").to_string();
                let (dxv, dyv) = (dx_raw.parse().ok(), dy_raw.parse().ok());
                let mk = |raw: String, v: Option<f64>| Qty {
                    raw, unit: doc.unit, datum: Datum::GerberFile, value_native: v,
                    pos: Some(SrcPos { file: name.into(), line: line_no, col: 3 }),
                };
                if dxv.is_none() || dyv.is_none() {
                    doc.diagnostics.push(Diagnostic {
                        code: "GERBER-SR-STEP".into(),
                        level: DiagLevel::Error,
                        message: "step-repeat 步距无法转换，原始值保留".into(),
                        pos: Some(SrcPos { file: name.into(), line: line_no, col: 3 }),
                        raw_value: Some(body.to_string()),
                    });
                }
                doc.tokens.push(Tok::SrStart {
                    sr: StepRepeat { nx, ny, dx: mk(dx_raw, dxv), dy: mk(dy_raw, dyv) },
                    line: line_no, raw: line.to_string(),
                });
            } else if body == "SR" {
                doc.tokens.push(Tok::SrEnd { line: line_no, raw: line.to_string() });
            } else {
                doc.diagnostics.push(Diagnostic {
                    code: "GERBER-CMD-UNKNOWN".into(),
                    level: DiagLevel::Warning,
                    message: "未识别的扩展命令，原样保留".into(),
                    pos: Some(SrcPos { file: name.into(), line: line_no, col: 1 }),
                    raw_value: Some(body.to_string()),
                });
            }
            continue;
        }

        if line.starts_with("G36") {
            in_region = true;
            doc.tokens.push(Tok::RegionIgnored { line: line_no, raw: line.to_string() });
            continue;
        }
        if line.starts_with("G37") {
            in_region = false;
            doc.tokens.push(Tok::RegionIgnored { line: line_no, raw: line.to_string() });
            continue;
        }
        if in_region {
            doc.tokens.push(Tok::RegionIgnored { line: line_no, raw: line.to_string() });
            continue;
        }
        if line.starts_with("M02") {
            break;
        }

        // 坐标行：提取 X/Y/D
        let body = line.strip_suffix('*').unwrap_or(line);
        let d_idx = body.find('D');
        let d_part = d_idx.map(|i| &body[i..]);
        let coord_body = d_idx.map(|i| &body[..i]).unwrap_or(body);
        if let Some(dp) = d_part {
            let code: String = dp.trim_start_matches('D').chars().take_while(|c| c.is_ascii_digit()).collect();
            match code.as_str() {
                "01" | "02" | "03" => {
                    let xraw = field(coord_body, 'X');
                    let yraw = field(coord_body, 'Y');
                    let x = match xraw {
                        Some(s) => make_coord_qty(name, line_no, 1, true, s, &doc, &format!("X{}", s)),
                        None => missing_qty(name, line_no),
                    };
                    let y = match yraw {
                        Some(s) => make_coord_qty(name, line_no, 1, false, s, &doc, &format!("Y{}", s)),
                        None => missing_qty(name, line_no),
                    };
                    if x.value_native.is_none() || y.value_native.is_none() {
                        doc.diagnostics.push(Diagnostic {
                            code: "GERBER-COORD".into(),
                            level: DiagLevel::Error,
                            message: "坐标无法按声明的 FS 格式转换，原始串保留".into(),
                            pos: Some(SrcPos { file: name.into(), line: line_no, col: 1 }),
                            raw_value: Some(line.to_string()),
                        });
                    }
                    match code.as_str() {
                        "03" => {
                            if aperture.is_none() {
                                doc.diagnostics.push(Diagnostic::err(
                                    "GERBER-NO-APERTURE", "flash 前未选择 aperture",
                                    Some(SrcPos { file: name.into(), line: line_no, col: 1 }),
                                ));
                            }
                            doc.tokens.push(Tok::Flash {
                                line: line_no, raw: line.to_string(), x, y,
                                aperture: aperture.unwrap_or(0), polarity,
                            });
                        }
                        "02" => doc.tokens.push(Tok::Move { line: line_no, raw: line.to_string(), x, y }),
                        _ => doc.tokens.push(Tok::Draw {
                            line: line_no, raw: line.to_string(), x, y,
                            aperture: aperture.unwrap_or(0), polarity,
                        }),
                    }
                }
                _ => {
                    // D10 等 aperture 选择
                    if let Ok(n) = code.parse::<u32>() {
                        if n >= 10 {
                            aperture = Some(n);
                        }
                    }
                }
            }
        } else {
            doc.diagnostics.push(Diagnostic {
                code: "GERBER-LINE".into(),
                level: DiagLevel::Warning,
                message: "无法解析的行，原样保留".into(),
                pos: Some(SrcPos { file: name.into(), line: line_no, col: 1 }),
                raw_value: Some(line.to_string()),
            });
        }
    }
    doc
}
