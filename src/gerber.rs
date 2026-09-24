//! Gerber (RS-274X) 解析器：仅规范化矩形图形，
//! 坐标格式、单位、极性、aperture、step-repeat 与原始命令全部保留。

use crate::model::*;

/// 单文件解析的对象预算（超出 => budget_exhausted）。
pub const MAX_RECTS: usize = 100_000;

struct Cursor {
    x: f64,
    y: f64,
}

/// 解析 Gerber 文本。`filename` 仅用于诊断定位。
pub fn parse_gerber(text: &str, filename: &str) -> ParsedFile {
    let mut doc = GerberDoc {
        format: String::new(),
        unit: LengthUnit::Unknown,
        zero_suppression: "unknown".to_string(),
        apertures: Vec::new(),
        ops: Vec::new(),
        datum: "absolute".to_string(),
        step_repeats: Vec::new(),
        file_function: String::new(),
    };
    let mut diags: Vec<Diagnostic> = Vec::new();
    let mut cursor = Cursor { x: 0.0, y: 0.0 };
    let mut polarity = Polarity::Dark;
    let mut current_ap = String::new();
    let mut seq: i64 = 0;
    let mut fmt: Option<(u32, u32)> = None;

    // SR 块状态：块内操作先缓存，遇到 SR 结束按网格展开。
    #[derive(Clone)]
    struct Pending {
        polarity: Polarity,
        aperture: String,
        rects: Vec<Rect>,
        line: i64,
        raw: String,
    }
    let mut sr: Option<(StepRepeat, Vec<Pending>)> = None;
    let mut total_rects = 0usize;

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
        if line.is_empty() || line.starts_with("G04") {
            continue;
        }

        // 扩展命令 %...*%
        if let Some(body) = line.strip_prefix('%') {
            let body = body.trim_end_matches("*%");
            if let Some(rest) = body.strip_prefix("FS") {
                // FSLAX25Y25 / FSLAX35Y35
                doc.format = rest.to_string();
                doc.zero_suppression = if rest.contains("L") {
                    "L".to_string()
                } else if rest.contains("T") {
                    "T".to_string()
                } else {
                    "none".to_string()
                };
                if let Some((a, b)) = parse_format(rest) {
                    fmt = Some((a, b));
                } else {
                    diag!(
                        DiagnosticKind::CorruptFormat,
                        line_no,
                        "FS-FORMAT",
                        "无法解析坐标格式声明",
                        body
                    );
                }
            } else if body == "MOMM" {
                doc.unit = LengthUnit::Mm;
            } else if body == "MOIN" {
                doc.unit = LengthUnit::Inch;
            } else if body == "LPD" {
                polarity = Polarity::Dark;
            } else if body == "LPC" {
                polarity = Polarity::Clear;
            } else if let Some(rest) = body.strip_prefix("AD") {
                match parse_aperture(rest, doc.unit, filename, line_no) {
                    Ok(ap) => doc.apertures.push(ap),
                    Err(d) => diags.push(d),
                }
            } else if let Some(rest) = body.strip_prefix("SR") {
                match parse_sr(rest, doc.unit) {
                    Ok(s) => {
                        doc.step_repeats.push(s.clone());
                        sr = Some((s, Vec::new()));
                    }
                    Err(msg) => diag!(
                        DiagnosticKind::CorruptFormat,
                        line_no,
                        "SR-FORMAT",
                        msg,
                        body
                    ),
                }
            } else if let Some(rest) = body.strip_prefix("TF.FileFunction") {
                doc.file_function = rest.trim_start_matches(',').to_string();
            } else if body.starts_with("TF.")
                || body.starts_with("TA.")
                || body.starts_with("TD")
                || body.starts_with("TO.")
            {
                // 其它属性：保留语义不需要，但不是错误。
            } else if body.starts_with("AM") {
                diag!(
                    DiagnosticKind::SemanticUnresolved,
                    line_no,
                    "APERTURE-MACRO",
                    "aperture 宏本工作台不支持，几何未重建",
                    body
                );
            } else {
                diag!(
                    DiagnosticKind::CorruptFormat,
                    line_no,
                    "UNKNOWN-EXT",
                    "无法识别的扩展命令",
                    body
                );
            }
            continue;
        }

        // SR 块结束
        if line == "SR" && sr.is_some() {
            let (s, pendings) = sr.take().unwrap();
            for (ix, iy) in grid_iter(s.nx, s.ny) {
                let ox = ix as f64 * s.dx;
                let oy = iy as f64 * s.dy;
                for p in &pendings {
                    let mut rects = p.rects.clone();
                    for r in &mut rects {
                        r.x0 += ox;
                        r.x1 += ox;
                        r.y0 += oy;
                        r.y1 += oy;
                    }
                    total_rects += rects.len();
                    if total_rects > MAX_RECTS {
                        diag!(
                            DiagnosticKind::BudgetExhausted,
                            p.line,
                            "BUDGET-RECTS",
                            "step-repeat 展开对象超过预算",
                            &p.raw
                        );
                        return finish(doc, diags);
                    }
                    seq += 1;
                    doc.ops.push(DrawOp {
                        seq,
                        polarity: p.polarity,
                        aperture: p.aperture.clone(),
                        rects,
                        source_line: p.line,
                        sr_cell: Some((ix, iy)),
                        raw_command: p.raw.clone(),
                    });
                }
            }
            continue;
        }

        // M02 结束
        if line == "M02" || line == "M00" {
            break;
        }

        // 光圈选择 Dnn（无坐标）
        if let Some(code) = line.strip_prefix('D') {
            if let Ok(num) = code.trim_end_matches('*').parse::<u32>() {
                if num >= 10 {
                    current_ap = format!("D{num}");
                    continue;
                }
            }
        }

        // 图形命令：X..Y..D0[123]
        if line.contains('D') || line.starts_with('X') || line.starts_with('Y') {
            let Some((a, b)) = fmt else {
                diag!(
                    DiagnosticKind::CorruptFormat,
                    line_no,
                    "NO-FS",
                    "图形命令前缺少 FS 坐标格式声明",
                    line
                );
                continue;
            };
            if current_ap.is_empty() {
                diag!(
                    DiagnosticKind::SemanticUnresolved,
                    line_no,
                    "NO-APERTURE",
                    "绘图前未选择 aperture",
                    line
                );
                continue;
            }
            let (cmd, nx, ny) = match parse_coord_command(line, a, b) {
                Ok(v) => v,
                Err(msg) => {
                    diag!(
                        DiagnosticKind::CorruptFormat,
                        line_no,
                        "COORD",
                        msg,
                        line
                    );
                    continue;
                }
            };
            if nx.is_some() {
                cursor.x = nx.unwrap();
            }
            if ny.is_some() {
                cursor.y = ny.unwrap();
            }
            let ap = doc
                .apertures
                .iter()
                .find(|x| x.code == current_ap)
                .cloned();
            match cmd {
                3 => { /* 落光笔移动：无图形 */ }
                2 => { /* D02 仅移动：无图形 */ }
                1 => {
                    let Some(ap) = ap else {
                        diag!(
                            DiagnosticKind::SemanticUnresolved,
                            line_no,
                            "APERTURE-MISSING",
                            "引用了未定义的 aperture",
                            line
                        );
                        continue;
                    };
                    let Some(w) = ap.width_mm else {
                        diag!(
                            DiagnosticKind::SemanticUnresolved,
                            line_no,
                            "APERTURE-SIZE",
                            "aperture 尺寸无法转换，图形未重建",
                            line
                        );
                        continue;
                    };
                    let h = ap.height_mm.unwrap_or(w);
                    let factor = doc.unit.to_mm_factor().unwrap_or(1.0);
                    // 光标坐标按文件单位解释；aperture 尺寸换算回文件单位落矩形。
                    let wu = w / factor;
                    let hu = h / factor;
                    let rect = Rect::new(
                        cursor.x - wu / 2.0,
                        cursor.y - hu / 2.0,
                        cursor.x + wu / 2.0,
                        cursor.y + hu / 2.0,
                    );
                    let pending = Pending {
                        polarity,
                        aperture: current_ap.clone(),
                        rects: vec![rect],
                        line: line_no,
                        raw: line.to_string(),
                    };
                    if let Some((_, list)) = sr.as_mut() {
                        list.push(pending);
                    } else {
                        total_rects += 1;
                        if total_rects > MAX_RECTS {
                            diag!(
                                DiagnosticKind::BudgetExhausted,
                                line_no,
                                "BUDGET-RECTS",
                                "图形对象超过预算",
                                line
                            );
                            return finish(doc, diags);
                        }
                        seq += 1;
                        doc.ops.push(DrawOp {
                            seq,
                            polarity: pending.polarity,
                            aperture: pending.aperture,
                            rects: pending.rects,
                            source_line: line_no,
                            sr_cell: None,
                            raw_command: pending.raw,
                        });
                    }
                }
                _ => {
                    diag!(
                        DiagnosticKind::CorruptFormat,
                        line_no,
                        "D-CODE",
                        "不支持的 D 代码",
                        line
                    );
                }
            }
            continue;
        }

        diag!(
            DiagnosticKind::CorruptFormat,
            line_no,
            "UNKNOWN-LINE",
            "无法识别的 Gerber 行",
            line
        );
    }

    if sr.is_some() {
        diags.push(Diagnostic {
            kind: DiagnosticKind::CorruptFormat,
            file: filename.to_string(),
            line: 0,
            code: "SR-UNCLOSED".to_string(),
            message: "step-repeat 块未以 SR 结束".to_string(),
            raw_snippet: String::new(),
        });
    }
    finish(doc, diags)
}

fn finish(doc: GerberDoc, diags: Vec<Diagnostic>) -> ParsedFile {
    ParsedFile {
        kind: FileKind::Gerber.as_str().to_string(),
        gerber: Some(doc),
        excellon: None,
        stackup: None,
        diagnostics: diags,
    }
}

fn grid_iter(nx: i64, ny: i64) -> impl Iterator<Item = (i64, i64)> {
    let nx = nx.max(1);
    let ny = ny.max(1);
    (0..ny).flat_map(move |y| (0..nx).map(move |x| (x, y)))
}

fn parse_format(rest: &str) -> Option<(u32, u32)> {
    // LAX25Y25
    let xpos = rest.find('X')?;
    let ypos = rest.find('Y')?;
    let xs = &rest[xpos + 1..ypos];
    let ys = &rest[ypos + 1..];
    let xa = xs.chars().next()?.to_digit(10)?;
    let xb = xs.chars().nth(1)?.to_digit(10)?;
    let _ya = ys.chars().next()?.to_digit(10)?;
    let yb = ys.chars().nth(1)?.to_digit(10)?;
    Some((xa + xb, yb))
}

fn parse_aperture(
    rest: &str,
    unit: LengthUnit,
    filename: &str,
    line: i64,
) -> Result<Aperture, Diagnostic> {
    // D10R,1.5X0.8 / D11C,0.3
    let (code, tail) = rest
        .split_once(|c: char| c == 'R' || c == 'C' || c == 'O' || c == 'P')
        .ok_or_else(|| Diagnostic {
            kind: DiagnosticKind::CorruptFormat,
            file: filename.to_string(),
            line,
            code: "AD-FORMAT".to_string(),
            message: "无法解析 aperture 定义".to_string(),
            raw_snippet: rest.chars().take(160).collect(),
        })?;
    let shape = rest[code.len()..code.len() + 1].to_string();
    let raw_params = tail.trim_start_matches(',').to_string();
    if shape != "R" {
        return Err(Diagnostic {
            kind: DiagnosticKind::SemanticUnresolved,
            file: filename.to_string(),
            line,
            code: "APERTURE-SHAPE".to_string(),
            message: format!("仅支持矩形 R aperture，收到 {shape}"),
            raw_snippet: raw_params.clone(),
        });
    }
    if unit == LengthUnit::Unknown {
        return Err(Diagnostic {
            kind: DiagnosticKind::SemanticUnresolved,
            file: filename.to_string(),
            line,
            code: "NO-UNIT".to_string(),
            message: "aperture 声明前缺少 MO 单位".to_string(),
            raw_snippet: raw_params,
        });
    }
    let factor = unit.to_mm_factor().unwrap();
    let mut dims = raw_params.split('X');
    let w = dims
        .next()
        .and_then(|s| s.parse::<f64>().ok())
        .map(|v| v * factor);
    let h = dims
        .next()
        .and_then(|s| s.parse::<f64>().ok())
        .map(|v| v * factor)
        .or(w);
    Ok(Aperture {
        code: code.to_string(),
        shape,
        raw_params,
        unit,
        width_mm: w,
        height_mm: h,
    })
}

fn parse_sr(rest: &str, unit: LengthUnit) -> Result<StepRepeat, String> {
    // X2Y2I22.0J0.0
    let mut nx = 1i64;
    let mut ny = 1i64;
    let mut dx = 0.0f64;
    let mut dy = 0.0f64;
    let mut key = None;
    let mut num = String::new();
    for ch in rest.chars() {
        match ch {
            'X' | 'Y' | 'I' | 'J' => {
                if let Some(k) = key {
                    let v: f64 = num.parse().map_err(|_| format!("SR 参数错误: {rest}"))?;
                    assign_sr(k, v, &mut nx, &mut ny, &mut dx, &mut dy);
                }
                key = Some(ch);
                num.clear();
            }
            _ => num.push(ch),
        }
    }
    if let Some(k) = key {
        let v: f64 = num.parse().map_err(|_| format!("SR 参数错误: {rest}"))?;
        assign_sr(k, v, &mut nx, &mut ny, &mut dx, &mut dy);
    }
    Ok(StepRepeat {
        nx,
        ny,
        dx,
        dy,
        unit,
    })
}

fn assign_sr(k: char, v: f64, nx: &mut i64, ny: &mut i64, dx: &mut f64, dy: &mut f64) {
    match k {
        'X' => *nx = v as i64,
        'Y' => *ny = v as i64,
        'I' => *dx = v,
        'J' => *dy = v,
        _ => {}
    }
}

/// 解析 `X01234Y01234D01`，返回 (d码, 新x, 新y)。
fn parse_coord_command(
    line: &str,
    total_x: u32,
    frac_y: u32,
) -> Result<(u32, Option<f64>, Option<f64>), String> {
    let body = line.trim_end_matches('*');
    let mut x: Option<f64> = None;
    let mut y: Option<f64> = None;
    let mut d: Option<u32> = None;
    let mut key: Option<char> = None;
    let mut buf = String::new();
    for ch in body.chars() {
        if ch == 'X' || ch == 'Y' || ch == 'D' {
            if let Some(k) = key {
                let value = decode_number(&buf, total_x, frac_y, k)?;
                match k {
                    'X' => x = Some(value),
                    'Y' => y = Some(value),
                    'D' => d = Some(buf.parse().map_err(|_| "D 码错误".to_string())?),
                    _ => {}
                }
            }
            key = Some(ch);
            buf.clear();
        } else if ch.is_ascii_digit() || ch == '-' || ch == '+' {
            buf.push(ch);
        } else if !ch.is_whitespace() {
            return Err(format!("坐标命令含非法字符 {ch}"));
        }
    }
    if let Some(k) = key {
        if k == 'D' {
            d = Some(buf.parse().map_err(|_| "D 码错误".to_string())?);
        } else {
            let value = decode_number(&buf, total_x, frac_y, k)?;
            match k {
                'X' => x = Some(value),
                'Y' => y = Some(value),
                _ => {}
            }
        }
    }
    d.map(|c| (c, x, y))
        .ok_or_else(|| "图形命令缺少 D 代码".to_string())
}

/// 前导零抑制（Gerber 常见 L 省略）。`total` 为整数+小数总位数。
fn decode_number(buf: &str, total: u32, frac: u32, axis: char) -> Result<f64, String> {
    if buf.is_empty() {
        return Err(format!("{axis} 坐标为空"));
    }
    let neg = buf.starts_with('-');
    let digits: String = buf.chars().filter(|c| c.is_ascii_digit()).collect();
    // 补零到总位数（前导零抑制）。
    let total = total as usize;
    let padded = if digits.len() < total {
        "0".repeat(total - digits.len()) + &digits
    } else {
        digits
    };
    let frac = frac as usize;
    let (int_part, frac_part) = if padded.len() > frac {
        padded.split_at(padded.len() - frac)
    } else {
        ("0", padded.as_str())
    };
    let int_v: f64 = int_part.parse().map_err(|_| "整数部分错误".to_string())?;
    let frac_v: f64 = if frac_part.is_empty() {
        0.0
    } else {
        frac_part.parse().map_err(|_| "小数部分错误".to_string())?
    };
    let scale = 10f64.powi(frac as i32);
    let mut v = int_v + frac_v / scale;
    if neg {
        v = -v;
    }
    Ok(v)
}
