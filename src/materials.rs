//! 不可变材料：Gerber RS-274X、Excellon 钻/铣、层叠说明、手工层映射。
//! 原文件坐标格式、单位、极性、aperture、step-repeat、图层角色与手工映射全部保留。

use crate::domain::{
    content_id, digits_to_nm, CoordFormat, Diagnostic, DiagKind, LengthUnit, LoadOutcome, P, Pos,
    RawCoord, Budgets,
};
use serde::{Deserialize, Serialize};

// ---------- 文本小工具 ----------

/// 小数字符串（mm）精确转纳米。
pub fn decimal_mm_to_nm(s: &str) -> Option<i64> {
    decimal_to_nm(s, LengthUnit::Mm)
}

pub fn decimal_in_to_nm(s: &str) -> Option<i64> {
    decimal_to_nm(s, LengthUnit::In)
}

fn decimal_to_nm(s: &str, unit: LengthUnit) -> Option<i64> {
    let s = s.trim();
    let (neg, body) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let (int, frac) = match body.split_once('.') {
        Some((a, b)) => (a, b),
        None => (body, ""),
    };
    if int.is_empty() && frac.is_empty() {
        return None;
    }
    if !int.bytes().chain(frac.bytes()).all(|b| b.is_ascii_digit()) {
        return None;
    }
    let int_v: i128 = if int.is_empty() { 0 } else { int.parse().ok()? };
    let frac_v: i128 = if frac.is_empty() {
        0
    } else {
        let digits = frac.len();
        let f: i128 = frac.parse().ok()?;
        f * 10i128.pow(9u32.saturating_sub(digits as u32))
    };
    let nm = match unit {
        LengthUnit::Mm => int_v * 1_000_000 + frac_v,
        LengthUnit::In => int_v * 25_400_000 + frac_v * 254 / 100,
    };
    let nm = if neg { -nm } else { nm };
    i64::try_from(nm).ok()
}

pub fn nm_to_mm_string(nm: i64) -> String {
    let neg = nm < 0;
    let v = nm.unsigned_abs();
    let int = v / 1_000_000;
    let frac = v % 1_000_000;
    format!("{}{}.{:06}", if neg { "-" } else { "" }, int, frac)
}

// ---------- Gerber ----------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApertureDef {
    pub code: String,
    pub shape: String,
    /// 原始参数（如 "0.5X0.3"），保留原单位文本。
    pub params_raw: Vec<String>,
    /// 以纳米规范化的圆形直径/矩形宽高；无法转换为 None。
    pub size_nm: Vec<Option<i64>>,
    pub pos: Pos,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Polarity {
    Dark,
    Clear,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GerberOp {
    pub seq: usize,
    pub polarity: Polarity,
    pub aperture: String,
    /// FLASH / MOVE / DRAW / UNSUPPORTED。
    pub action: String,
    pub raw: RawCoord,
    pub raw_line: String,
    pub pos: Pos,
    /// DRAW 的起点（D02 move 记忆），FLASH 为 None。
    pub from: Option<P>,
    pub to: Option<P>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SrBlock {
    pub x_count: i64,
    pub y_count: i64,
    pub x_step_raw: Option<String>,
    pub y_step_raw: Option<String>,
    pub header_pos: Pos,
    pub ops: Vec<GerberOp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GerberDoc {
    pub format: Option<CoordFormat>,
    pub unit_declared: Option<LengthUnit>,
    pub apertures: Vec<ApertureDef>,
    pub step_repeats: Vec<SrBlock>,
    pub ops: Vec<GerberOp>,
    pub raw_lines: Vec<String>,
    pub layer_hint: Option<String>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn parse_gerber(text: &str, budgets: Budgets) -> LoadOutcome<GerberDoc> {
    let mut doc = GerberDoc {
        format: None,
        unit_declared: None,
        apertures: Vec::new(),
        step_repeats: Vec::new(),
        ops: Vec::new(),
        raw_lines: text.lines().map(|l| l.to_string()).collect(),
        layer_hint: None,
        diagnostics: Vec::new(),
    };
    let mut corrupt: Vec<Diagnostic> = Vec::new();
    let mut polarity = Polarity::Dark;
    let mut aperture = String::new();
    let mut current: Option<P> = None;
    let mut sr_stack: Vec<(SrBlock, usize)> = Vec::new();
    let mut sr_cells_used = 0usize;
    let mut absolute = true;
    let mut seq = 0usize;
    let mut fs_parts: Option<(u8, u8, char, bool)> = None;

    if text.len() > budgets.max_file_bytes {
        return LoadOutcome::ResourceExhausted(
            doc,
            crate::domain::BudgetReport { limit: budgets.max_file_bytes, observed: text.len(), resource: "file_bytes" },
            vec![Diagnostic::new(
                DiagKind::ResourceExhausted,
                "FILE_TOO_LARGE",
                format!("文件 {} 字节超过预算 {}", text.len(), budgets.max_file_bytes),
                Pos::new(1, 1, 0, ""),
            )],
        );
    }

    let mut offset = 0usize;
    for (idx, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        let line_no = idx + 1;
        let pos = Pos::new(line_no, 1, offset, line);
        offset += raw_line.len() + 1;
        if line.is_empty() {
            continue;
        }

        if line.starts_with("%FS") {
            match parse_fs(line, pos.clone()) {
                Ok((xi, xd, _yi, _yd, zero, abs)) => {
                    absolute = abs;
                    fs_parts = Some((xi, xd, zero, abs));
                }
                Err(d) => corrupt.push(d),
            }
            continue;
        }
        if line.starts_with("%MO") {
            let body = line.trim_start_matches("%MO");
            let body = body.trim_end_matches('*').trim_end_matches('%');
            match LengthUnit::parse(body) {
                Some(u) => doc.unit_declared = Some(u),
                None => corrupt.push(Diagnostic::new(
                    DiagKind::FormatCorrupt,
                    "BAD_UNIT",
                    format!("无法识别的单位声明 {body}"),
                    pos,
                )),
            }
            continue;
        }
        if line.starts_with("%LP") {
            let body = line.trim_start_matches("%LP");
            match body.chars().next() {
                Some('D') => polarity = Polarity::Dark,
                Some('C') => polarity = Polarity::Clear,
                _ => corrupt.push(Diagnostic::new(
                    DiagKind::FormatCorrupt,
                    "BAD_POLARITY",
                    "层极性必须是 LPD 或 LPC",
                    pos,
                )),
            }
            continue;
        }
        if line.starts_with("%AD") {
            match parse_ad(line, &doc, pos.clone()) {
                Ok(ap) => doc.apertures.push(ap),
                Err(d) => corrupt.push(d),
            }
            continue;
        }
        if line.starts_with("%SR") {
            match parse_sr_header(line, pos.clone()) {
                Ok(block) => sr_stack.push((block, sr_cells_used)),
                Err(d) => corrupt.push(d),
            }
            continue;
        }
        if line.trim_matches('%').is_empty() && !sr_stack.is_empty() {
            let (block, _) = sr_stack.pop().expect("sr stack");
            doc.step_repeats.push(block);
            continue;
        }
        if line.starts_with("G04") {
            if let Some(hint) = line.strip_prefix("G04 @Layer:").or_else(|| line.strip_prefix("G04 @Layer")) {
                let hint = hint.trim().trim_end_matches('*').trim();
                if !hint.is_empty() {
                    doc.layer_hint = Some(hint.to_string());
                }
            }
            continue;
        }
        if line.starts_with("%TF") || line.starts_with("%TA") || line.starts_with("%TD") {
            continue;
        }
        if line == "M02*" {
            break;
        }
        if line.starts_with("G02") || line.starts_with("G03") {
            doc.diagnostics.push(Diagnostic::new(
                DiagKind::SemanticUnresolved,
                "ARC_UNSUPPORTED",
                "圆弧插补不支持：原行保留，需人工裁定",
                pos,
            ));
            continue;
        }
        if line.contains('D') && (line.contains('X') || line.contains('Y') || line.ends_with("D02*")) {
            match parse_coord_line(line) {
                Ok((rc, dcode)) => {
                    let rc = normalize_coord(rc, &doc, &mut corrupt, pos.clone());
                    match dcode.as_str() {
                        "D02" => {
                            if let (Some(x), Some(y)) = (rc.x_nm, rc.y_nm) {
                                let p = if absolute || current.is_none() {
                                    P::new(x, y)
                                } else {
                                    let cur = current.unwrap();
                                    P::new(cur.x + x, cur.y + y)
                                };
                                current = Some(p);
                            }
                        }
                        "D03" | "D01" => {
                            if doc.format.is_none() {
                                doc.diagnostics.push(Diagnostic::new(
                                    DiagKind::SemanticUnresolved,
                                    "NO_FS",
                                    "缺少 %FSLAX...% 坐标格式声明，坐标保留原值",
                                    pos.clone(),
                                ));
                            }
                            let to = match (rc.x_nm, rc.y_nm) {
                                (Some(x), Some(y)) => {
                                    if absolute || current.is_none() {
                                        Some(P::new(x, y))
                                    } else {
                                        let cur = current.unwrap();
                                        Some(P::new(cur.x + x, cur.y + y))
                                    }
                                }
                                _ => None,
                            };
                            let action = if dcode == "D03" { "FLASH" } else { "DRAW" };
                            let op = GerberOp {
                                seq,
                                polarity,
                                aperture: aperture.clone(),
                                action: action.to_string(),
                                raw: rc,
                                raw_line: raw_line.to_string(),
                                pos: pos.clone(),
                                from: if dcode == "D01" { current } else { None },
                                to,
                            };
                            seq += 1;
                            push_op(&mut doc, &mut sr_stack, op);
                            if let Some(t) = to {
                                current = Some(t);
                            }
                            sr_cells_used = update_sr_cells(&mut sr_stack, &mut sr_cells_used, budgets, &mut corrupt, pos.clone());
                            if sr_cells_used > budgets.max_step_repeat_cells {
                                return exhausted(&doc, budgets, sr_cells_used, corrupt);
                            }
                            if doc.ops.len() + sr_total_ops(&sr_stack) > budgets.max_copper_elements {
                                return exhausted_elements(&doc, budgets, corrupt);
                            }
                        }
                        _ => corrupt.push(Diagnostic::new(
                            DiagKind::FormatCorrupt,
                            "BAD_DCODE",
                            format!("未知绘图码 {dcode}"),
                            pos,
                        )),
                    }
                }
                Err(d) => corrupt.push(d),
            }
            continue;
        }
        if line.starts_with('D') && line.ends_with("D02*") == false && line.len() <= 5 {
            // 选 aperture，如 D10*
            let code = line.trim_end_matches('*').to_string();
            if code.chars().skip(1).all(|c| c.is_ascii_digit()) {
                aperture = code;
                continue;
            }
        }
        doc.diagnostics.push(Diagnostic::new(
            DiagKind::FormatCorrupt,
            "UNPARSED_LINE",
            "无法识别的 Gerber 记录",
            pos,
        ));
    }

    if let Some((xi, xd, zero, abs)) = fs_parts {
        doc.format = Some(CoordFormat {
            unit: doc.unit_declared.unwrap_or(LengthUnit::Mm),
            int_digits: xi,
            dec_digits: xd,
            zero_mode: zero,
            absolute: abs,
            raw: doc.raw_lines.iter().find(|l| l.trim().starts_with("%FS")).cloned().unwrap_or_default(),
        });
    } else if doc.unit_declared.is_some() {
        doc.diagnostics.push(Diagnostic::new(
            DiagKind::SemanticUnresolved,
            "NO_FS",
            "声明了单位但缺少 %FS 坐标格式：坐标无法规范化",
            Pos::new(0, 0, 0, ""),
        ));
    }
    if !sr_stack.is_empty() {
        corrupt.push(Diagnostic::new(
            DiagKind::FormatCorrupt,
            "SR_UNCLOSED",
            "step-repeat 块未以单独 % 行结束",
            Pos::new(text.lines().count(), 1, offset, ""),
        ));
    }
    finalize(doc, corrupt)
}

fn sr_total_ops(stack: &[(SrBlock, usize)]) -> usize {
    stack.iter().map(|(b, _)| b.ops.len()).sum()
}

fn push_op(doc: &mut GerberDoc, stack: &mut [(SrBlock, usize)], op: GerberOp) {
    if let Some((block, _)) = stack.last_mut() {
        block.ops.push(op);
    } else {
        doc.ops.push(op);
    }
}

fn update_sr_cells(
    stack: &mut [(SrBlock, usize)],
    used: &mut usize,
    budgets: Budgets,
    corrupt: &mut Vec<Diagnostic>,
    pos: Pos,
) -> usize {
    // 只有在 SR 块内第一条绘图操作时计数一次单元数。
    if let Some((block, baseline)) = stack.last_mut() {
        if block.ops.len() == 1 {
            let cells = block.x_count.max(1) as usize * block.y_count.max(1) as usize;
            if *used + cells > budgets.max_step_repeat_cells {
                corrupt.push(Diagnostic::new(
                    DiagKind::ResourceExhausted,
                    "SR_BUDGET",
                    format!("step-repeat 展开 {cells} 单元超过预算 {}", budgets.max_step_repeat_cells),
                    pos,
                ));
            }
            *used += cells;
        }
        let _ = baseline;
    }
    *used
}

fn exhausted(
    doc: &GerberDoc,
    budgets: Budgets,
    observed: usize,
    mut corrupt: Vec<Diagnostic>,
) -> LoadOutcome<GerberDoc> {
    if !corrupt.iter().any(|d| d.kind == DiagKind::ResourceExhausted) {
        corrupt.push(Diagnostic::new(
            DiagKind::ResourceExhausted,
            "SR_BUDGET",
            "step-repeat 展开超过预算",
            Pos::new(0, 0, 0, ""),
        ));
    }
    LoadOutcome::ResourceExhausted(
        doc.clone(),
        crate::domain::BudgetReport {
            limit: budgets.max_step_repeat_cells,
            observed,
            resource: "step_repeat_cells",
        },
        corrupt.into_iter().filter(|d| d.kind == DiagKind::ResourceExhausted).collect(),
    )
}

fn exhausted_elements(doc: &GerberDoc, budgets: Budgets, _corrupt: Vec<Diagnostic>) -> LoadOutcome<GerberDoc> {
    LoadOutcome::ResourceExhausted(
        doc.clone(),
        crate::domain::BudgetReport {
            limit: budgets.max_copper_elements,
            observed: doc.ops.len(),
            resource: "copper_elements",
        },
        vec![Diagnostic::new(
            DiagKind::ResourceExhausted,
            "COPPER_BUDGET",
            format!("铜区数量超过预算 {}", budgets.max_copper_elements),
            Pos::new(0, 0, 0, ""),
        )],
    )
}

fn finalize(mut doc: GerberDoc, corrupt: Vec<Diagnostic>) -> LoadOutcome<GerberDoc> {
    if !corrupt.is_empty() {
        return LoadOutcome::FormatCorrupt(corrupt);
    }
    // 语义校验：引用未定义 aperture。
    for op in doc.ops.iter().chain(doc.step_repeats.iter().flat_map(|b| b.ops.iter())) {
        if !op.aperture.is_empty() && !doc.apertures.iter().any(|a| a.code == op.aperture) {
            doc.diagnostics.push(Diagnostic::new(
                DiagKind::SemanticUnresolved,
                "APERTURE_UNDEFINED",
                format!("操作引用了未定义的 aperture {}", op.aperture),
                op.pos.clone(),
            ));
        }
        if op.to.is_none() {
            doc.diagnostics.push(Diagnostic::new(
                DiagKind::UnconvertibleValue,
                "COORD_UNCONVERTED",
                "坐标无法按声明格式规范化，原值保留",
                op.pos.clone(),
            ));
        }
    }
    if doc.diagnostics.iter().any(|d| matches!(d.kind, DiagKind::SemanticUnresolved)) {
        let diags = doc.diagnostics.clone();
        LoadOutcome::SemanticUnresolved(doc, diags)
    } else {
        LoadOutcome::Ok(doc)
    }
}

fn parse_fs(line: &str, pos: Pos) -> Result<(u8, u8, u8, u8, char, bool), Diagnostic> {
    // 例：%FSLAX34Y34*%
    let body = line.trim_start_matches("%FS").trim_start_matches('%');
    let body = body.trim_end_matches('%').trim_end_matches('*');
    let b = body.as_bytes();
    if b.len() < 8 {
        return Err(bad_at("FS 记录过短", &pos));
    }
    let zero = b[0] as char;
    if !matches!(zero, 'L' | 'T' | 'D') {
        return Err(bad_at("FS 零抑制必须是 L/T/D", &pos));
    }
    let absolute = match b[1] as char {
        'A' => true,
        'I' => false,
        _ => return Err(bad_at("FS 必须声明 A(绝对) 或 I(增量)", &pos)),
    };
    if b[2] != b'X' {
        return Err(bad_at("FS 缺少 X 格式", &pos));
    }
    let two = |i: usize| -> Option<(u8, u8)> {
        if b.len() <= i + 1 || !b[i].is_ascii_digit() || !b[i + 1].is_ascii_digit() {
            return None;
        }
        Some((b[i] - b'0', b[i + 1] - b'0'))
    };
    let (xi, xd) = two(3).ok_or_else(|| bad_at("FS X 数字位数非法", &pos))?;
    let ypos = body.find('Y').ok_or_else(|| bad_at("FS 缺少 Y 格式", &pos))?;
    let (yi, yd) = two(ypos + 1).ok_or_else(|| bad_at("FS Y 数字位数非法", &pos))?;
    if xi != yi || xd != yd {
        return Err(bad_at("FS X/Y 位数不一致", &pos));
    }
    Ok((xi, xd, yi, yd, zero, absolute))
}

fn bad_at(msg: &str, pos: &Pos) -> Diagnostic {
    Diagnostic::new(DiagKind::FormatCorrupt, "FS_BAD", msg, pos.clone())
}

fn current_format(
    fs_parts: Option<(u8, u8, char, bool)>,
    unit: Option<LengthUnit>,
    raw: &str,
) -> Option<CoordFormat> {
    let (xi, xd, zero, abs) = fs_parts?;
    Some(CoordFormat {
        unit: unit?,
        int_digits: xi,
        dec_digits: xd,
        zero_mode: zero,
        absolute: abs,
        raw: raw.to_string(),
    })
}

fn normalize_coord(
    rc: RawCoord,
    doc: &GerberDoc,
    corrupt: &mut Vec<Diagnostic>,
    pos: Pos,
) -> RawCoord {
    // parse 阶段 doc.format 尚未装配：直接从文本临时重建。
    let fmt_line = doc.raw_lines.iter().find(|l| l.trim().starts_with("%FS"));
    let unit_line = doc.raw_lines.iter().find(|l| l.trim().starts_with("%MO"));
    let unit = unit_line.and_then(|l| {
        let body = l.trim().trim_start_matches("%MO").trim_end_matches('*').trim_end_matches('%');
        LengthUnit::parse(body)
    });
    let fmt = match (fmt_line, unit) {
        (Some(fl), Some(u)) => match parse_fs(fl, Pos::new(0, 0, 0, fl)) {
            Ok((xi, xd, _, _, zero, abs)) => Some(CoordFormat {
                unit: u,
                int_digits: xi,
                dec_digits: xd,
                zero_mode: zero,
                absolute: abs,
                raw: fl.clone(),
            }),
            Err(d) => {
                corrupt.push(d);
                None
            }
        },
        _ => None,
    };
    let mut rc = rc;
    if let Some(fmt) = fmt {
        let dx = digits_of(&rc.text, 'X');
        let dy = digits_of(&rc.text, 'Y');
        rc.x_nm = dx.as_ref().and_then(|d| digits_to_nm(d, &fmt));
        rc.y_nm = dy.as_ref().and_then(|d| digits_to_nm(d, &fmt));
        if dx.is_some() && rc.x_nm.is_none() {
            corrupt.push(Diagnostic::new(
                DiagKind::FormatCorrupt,
                "COORD_BAD",
                "X 坐标不满足声明格式",
                pos.clone(),
            ));
        }
        if dy.is_some() && rc.y_nm.is_none() {
            corrupt.push(Diagnostic::new(
                DiagKind::FormatCorrupt,
                "COORD_BAD",
                "Y 坐标不满足声明格式",
                pos,
            ));
        }
    }
    rc
}

fn digits_of(line: &str, axis: char) -> Option<String> {
    let i = line.find(axis)?;
    let rest = &line[i + 1..];
    let d: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if d.is_empty() { None } else { Some(d) }
}

fn parse_coord_line(line: &str) -> Result<(RawCoord, String), Diagnostic> {
    // 例：X00020000Y00010000D03*
    let body = line.trim_end_matches('*');
    let dpos = body.find('D').filter(|&i| i > 0).ok_or_else(|| {
        Diagnostic::new(DiagKind::FormatCorrupt, "NO_DCODE", "绘图记录缺少 D 码", Pos::new(0, 0, 0, body))
    })?;
    let coord_part = &body[..dpos];
    let dcode = body[dpos..].split(['*']).next().unwrap().to_string();
    let has_x = coord_part.contains('X');
    let has_y = coord_part.contains('Y');
    if !has_x && !has_y {
        return Err(Diagnostic::new(DiagKind::FormatCorrupt, "NO_COORD", "绘图记录缺少 X/Y", Pos::new(0, 0, 0, body)));
    }
    Ok((
        RawCoord {
            text: coord_part.to_string(),
            axis: if has_x { 'X' } else { 'Y' },
            x_nm: None,
            y_nm: None,
        },
        dcode,
    ))
}

fn parse_ad(line: &str, doc: &GerberDoc, pos: Pos) -> Result<ApertureDef, Diagnostic> {
    // %ADD10C,0.5*%
    let body = line.trim_start_matches("%AD").trim_end_matches('%').trim_end_matches('*');
    let (code_part, rest) = body.split_at(body.find(',').unwrap_or(body.len()));
    let code = code_part.trim().to_string();
    if code.len() < 2 {
        return Err(Diagnostic::new(DiagKind::FormatCorrupt, "AD_BAD", "aperture 编码非法", pos));
    }
    let rest = rest.trim_start_matches(',');
    let (shape, params) = match rest.split_once(|c: char| c.is_whitespace() || c == 'X') {
        Some((s, p)) if !s.is_empty() => (s.to_string(), rest[s.len()..].to_string()),
        _ => {
            let end = rest.find('X').unwrap_or(rest.len());
            (rest[..end].to_string(), rest[end..].to_string())
        }
    };
    let shape = shape.trim().to_string();
    if shape.is_empty() {
        return Err(Diagnostic::new(DiagKind::FormatCorrupt, "AD_BAD", "aperture 缺少形状", pos));
    }
    let params_raw: Vec<String> = params.split('X').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect();
    let unit = doc.unit_declared.unwrap_or(LengthUnit::Mm);
    let size_nm = params_raw
        .iter()
        .map(|p| match unit {
            LengthUnit::Mm => decimal_mm_to_nm(p),
            LengthUnit::In => decimal_in_to_nm(p),
        })
        .collect();
    Ok(ApertureDef { code: format!("D{}", &code[1..]), shape, params_raw, size_nm, pos })
}

fn parse_sr_header(line: &str, pos: Pos) -> Result<SrBlock, Diagnostic> {
    // %SRX3Y2I2.0J1.0*%
    let body = line.trim_start_matches("%SR").trim_end_matches('%').trim_end_matches('*');
    let num_after = |key: char| -> Option<(i64, String)> {
        let i = body.find(key)?;
        let rest = &body[i + 1..];
        let d: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '-' || *c == '.')
            .collect();
        d.parse::<i64>().ok().map(|n| (n, d))
    };
    let (x_count, _) = num_after('X').ok_or_else(|| {
        Diagnostic::new(DiagKind::FormatCorrupt, "SR_BAD", "step-repeat 缺少 X 次数", pos.clone())
    })?;
    let (y_count, _) = num_after('Y').ok_or_else(|| {
        Diagnostic::new(DiagKind::FormatCorrupt, "SR_BAD", "step-repeat 缺少 Y 次数", pos.clone())
    })?;
    // I/J 步距在 I 与 J 或结尾之间，含小数点。
    let step_after = |start_key: char, stop: Option<char>| -> Option<String> {
        let i = body.find(start_key)? + 1;
        let tail = &body[i..];
        let end = stop.and_then(|c| tail.find(c)).unwrap_or(tail.len());
        let d: String = tail[..end].chars().take_while(|c| c.is_ascii_digit() || *c == '-' || *c == '.').collect();
        if d.is_empty() { None } else { Some(d) }
    };
    let i_raw = step_after('I', Some('J'));
    let j_raw = step_after('J', None);
    let unit = LengthUnit::Mm; // SR 块装配时调用方单位已知；这里仅记录原始，规范化延后。
    let _ = unit;
    Ok(SrBlock { x_count, y_count, x_step_raw: i_raw, y_step_raw: j_raw, header_pos: pos, ops: Vec::new() })
}

/// 把 step-repeat 与极性/操作序列展开为带顺序的绘图指令（供几何重建使用）。
#[derive(Debug, Clone)]
pub struct MaterialOp {
    pub op: GerberOp,
    pub sr_index: Option<usize>,
    pub cell_x: i64,
    pub cell_y: i64,
}

pub fn materialize_gerber(doc: &GerberDoc) -> Vec<MaterialOp> {
    let mut out = Vec::new();
    for op in doc.ops.clone() {
        out.push(MaterialOp { op, sr_index: None, cell_x: 0, cell_y: 0 });
    }
    for (idx, block) in doc.step_repeats.iter().enumerate() {
        let unit = doc.format.as_ref().map(|f| f.unit).or(doc.unit_declared).unwrap_or(LengthUnit::Mm);
        let conv = |raw: &Option<String>| -> i64 {
            raw.as_ref()
                .and_then(|r| match unit {
                    LengthUnit::Mm => decimal_mm_to_nm(r),
                    LengthUnit::In => decimal_in_to_nm(r),
                })
                .unwrap_or(0)
        };
        let sx = conv(&block.x_step_raw);
        let sy = conv(&block.y_step_raw);
        for cy in 0..block.y_count.max(1) {
            for cx in 0..block.x_count.max(1) {
                for op in &block.ops {
                    let to = op.to.map(|p| P::new(p.x + sx * cx, p.y + sy * cy));
                    let from = op.from.map(|p| P::new(p.x + sx * cx, p.y + sy * cy));
                    out.push(MaterialOp {
                        op: GerberOp {
                            seq: op.seq,
                            polarity: op.polarity,
                            aperture: op.aperture.clone(),
                            action: op.action.clone(),
                            raw: op.raw.clone(),
                            raw_line: op.raw_line.clone(),
                            pos: op.pos.clone(),
                            from,
                            to,
                        },
                        sr_index: Some(idx),
                        cell_x: cx,
                        cell_y: cy,
                    });
                }
            }
        }
    }
    out
}

// ---------- Excellon ----------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrillTool {
    pub code: String,
    pub diameter_nm: Option<i64>,
    pub diameter_raw: String,
    pub plated: Option<bool>,
    /// 盲/埋孔穿越层范围 [from, to]（1-based，按层叠自顶向下）。
    pub span_from: Option<usize>,
    pub span_to: Option<usize>,
    pub span_raw: Option<String>,
    pub pos: Pos,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrillHit {
    pub tool: String,
    pub raw: RawCoord,
    pub x_nm: Option<i64>,
    pub y_nm: Option<i64>,
    pub raw_line: String,
    pub pos: Pos,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExcellonDoc {
    pub unit_declared: Option<LengthUnit>,
    /// METRIC/INCH 行附带的小数位声明（如 0.0001）。
    pub format_raw: Option<String>,
    pub decimals: Option<u8>,
    pub tools: Vec<DrillTool>,
    pub hits: Vec<DrillHit>,
    pub raw_lines: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn parse_excellon(text: &str, budgets: Budgets) -> LoadOutcome<ExcellonDoc> {
    let mut doc = ExcellonDoc {
        unit_declared: None,
        format_raw: None,
        decimals: None,
        tools: Vec::new(),
        hits: Vec::new(),
        raw_lines: text.lines().map(|l| l.to_string()).collect(),
        diagnostics: Vec::new(),
    };
    let mut corrupt = Vec::new();
    let mut current_tool = String::new();

    if text.len() > budgets.max_file_bytes {
        return LoadOutcome::ResourceExhausted(
            doc,
            crate::domain::BudgetReport { limit: budgets.max_file_bytes, observed: text.len(), resource: "file_bytes" },
            vec![Diagnostic::new(DiagKind::ResourceExhausted, "FILE_TOO_LARGE", "钻文件超过字节预算", Pos::new(1, 1, 0, ""))],
        );
    }

    let mut offset = 0usize;
    for (idx, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        let line_no = idx + 1;
        let pos = Pos::new(line_no, 1, offset, line);
        offset += raw_line.len() + 1;
        if line.is_empty() {
            continue;
        }
        if line.starts_with(';') {
            // 扩展注释：;@span:T1=2-3  ;@plated:T1=yes
            if let Some(rest) = line.strip_prefix(";@span:") {
                apply_span(&mut doc, rest, pos.clone(), &mut corrupt);
            } else if let Some(rest) = line.strip_prefix(";@plated:") {
                apply_plated(&mut doc, rest, pos.clone(), &mut corrupt);
            }
            continue;
        }
        if line == "%" || line == "M30" || line == "M00" || line == "M02" {
            if line == "M30" {
                break;
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("METRIC") {
            doc.unit_declared = Some(LengthUnit::Mm);
            doc.format_raw = Some(rest.trim().trim_end_matches(',').to_string());
            doc.decimals = trailing_decimals(rest);
            continue;
        }
        if let Some(rest) = line.strip_prefix("INCH") {
            doc.unit_declared = Some(LengthUnit::In);
            doc.format_raw = Some(rest.trim().trim_end_matches(',').to_string());
            doc.decimals = trailing_decimals(rest);
            continue;
        }
        if line.starts_with('T') && line.contains('C') {
            // T1C0.3
            if let Some(tpos) = line.find('C') {
                let code = line[..tpos].trim().to_string();
                let diam_raw = line[tpos + 1..].trim().trim_end_matches('*').to_string();
                let diameter_nm = match doc.unit_declared {
                    Some(LengthUnit::Mm) => decimal_mm_to_nm(&diam_raw),
                    Some(LengthUnit::In) => decimal_in_to_nm(&diam_raw),
                    None => None,
                };
                doc.tools.push(DrillTool {
                    code,
                    diameter_nm,
                    diameter_raw: diam_raw,
                    plated: None,
                    span_from: None,
                    span_to: None,
                    span_raw: None,
                    pos,
                });
            }
            continue;
        }
        if line.starts_with('T') && line[1..].chars().next().is_some_and(|c| c.is_ascii_digit()) {
            let code = line.trim_end_matches('*').trim().to_string();
            if !doc.tools.iter().any(|t| t.code == code) {
                corrupt.push(Diagnostic::new(
                    DiagKind::SemanticUnresolved,
                    "TOOL_UNDEFINED",
                    format!("选择了未定义直径的刀具 {code}"),
                    pos,
                ));
            }
            current_tool = code;
            continue;
        }
        if line.starts_with('X') || line.starts_with('Y') {
            if doc.unit_declared.is_none() || doc.decimals.is_none() {
                doc.diagnostics.push(Diagnostic::new(
                    DiagKind::SemanticUnresolved,
                    "NO_DRILL_FORMAT",
                    "缺少 METRIC/INCH 小数位声明：孔位保留原值，不得用默认格式伪装",
                    pos.clone(),
                ));
                doc.hits.push(DrillHit {
                    tool: current_tool.clone(),
                    raw: RawCoord { text: line.trim_end_matches('*').to_string(), axis: 'X', x_nm: None, y_nm: None },
                    x_nm: None,
                    y_nm: None,
                    raw_line: raw_line.to_string(),
                    pos,
                });
                continue;
            }
            let (x, y) = parse_excellon_xy(line, doc.unit_declared.unwrap(), doc.decimals.unwrap());
            let mut rc = RawCoord { text: line.trim_end_matches('*').to_string(), axis: 'X', x_nm: x, y_nm: y };
            rc.axis = if x.is_some() { 'X' } else { 'Y' };
            doc.hits.push(DrillHit {
                tool: current_tool.clone(),
                raw: rc.clone(),
                x_nm: x,
                y_nm: y,
                raw_line: raw_line.to_string(),
                pos,
            });
            continue;
        }
        doc.diagnostics.push(Diagnostic::new(
            DiagKind::FormatCorrupt,
            "EXCELLON_UNPARSED",
            "无法识别的 Excellon 记录",
            pos,
        ));
    }

    if doc.hits.len() > budgets.max_copper_elements {
        return LoadOutcome::ResourceExhausted(
            doc,
            crate::domain::BudgetReport { limit: budgets.max_copper_elements, observed: doc.hits.len(), resource: "drill_hits" },
            vec![Diagnostic::new(DiagKind::ResourceExhausted, "DRILL_BUDGET", "孔数超过预算", Pos::new(0, 0, 0, ""))],
        );
    }
    if !corrupt.is_empty() {
        return LoadOutcome::FormatCorrupt(corrupt);
    }
    for tool in &doc.tools {
        if tool.plated.is_none() {
            doc.diagnostics.push(Diagnostic::new(
                DiagKind::SemanticUnresolved,
                "PLATING_UNDECLARED",
                format!("刀具 {} 未声明电镀/非电镀，不得默认导通", tool.code),
                tool.pos.clone(),
            ));
        }
        if tool.diameter_nm.is_none() {
            doc.diagnostics.push(Diagnostic::new(
                DiagKind::UnconvertibleValue,
                "DIAMETER_UNCONVERTED",
                format!("刀具 {} 直径无法转换", tool.code),
                tool.pos.clone(),
            ));
        }
    }
    if doc.diagnostics.iter().any(|d| d.kind == DiagKind::SemanticUnresolved) {
        let diags = doc.diagnostics.clone();
        LoadOutcome::SemanticUnresolved(doc, diags)
    } else {
        LoadOutcome::Ok(doc)
    }
}

fn trailing_decimals(rest: &str) -> Option<u8> {
    let token = rest.split(',').find_map(|t| {
        let t = t.trim();
        t.strip_prefix("0.").map(|f| f.len() as u8)
    });
    token
}

fn parse_excellon_xy(line: &str, unit: LengthUnit, decimals: u8) -> (Option<i64>, Option<i64>) {
    let body = line.trim_end_matches('*');
    let one = |axis: char| -> Option<i64> {
        let i = body.find(axis)? + 1;
        let raw: String = body[i..].chars().take_while(|c| c.is_ascii_digit() || *c == '-' || *c == '.').collect();
        let padded = if raw.contains('.') {
            raw
        } else {
            let neg = raw.starts_with('-');
            let digits = raw.trim_start_matches('-');
            let total = decimals as usize;
            let d = if digits.len() < total {
                format!("{:0>width$}", digits, width = total)
            } else {
                digits.to_string()
            };
            let d = if d.len() < decimals as usize {
                format!("{:0<width$}", d, width = decimals as usize)
            } else {
                d
            };
            let (i2, f2) = d.split_at(d.len() - decimals as usize);
            format!("{}{}.{}", if neg { "-" } else { "" }, i2, f2)
        };
        match unit {
            LengthUnit::Mm => decimal_mm_to_nm(&padded),
            LengthUnit::In => decimal_in_to_nm(&padded),
        }
    };
    (one('X'), one('Y'))
}

fn apply_span(doc: &mut ExcellonDoc, rest: &str, pos: Pos, corrupt: &mut Vec<Diagnostic>) {
    // T1=2-3
    let (code, range) = match rest.split_once('=') {
        Some(pair) => (pair.0.trim().to_string(), pair.1.trim()),
        None => {
            corrupt.push(Diagnostic::new(DiagKind::FormatCorrupt, "SPAN_BAD", ";@span 需要 code=from-to", pos));
            return;
        }
    };
    let (from_s, to_s) = match range.split_once('-') {
        Some(pair) => (pair.0.trim(), pair.1.trim()),
        None => {
            corrupt.push(Diagnostic::new(DiagKind::FormatCorrupt, "SPAN_BAD", "层范围需要 from-to", pos));
            return;
        }
    };
    let from = from_s.parse::<usize>().ok();
    let to = to_s.parse::<usize>().ok();
    match (from, to) {
        (Some(f), Some(t)) if f >= 1 && t >= f => match doc.tools.iter_mut().find(|tool| tool.code == code) {
            Some(tool) => {
                tool.span_from = Some(f);
                tool.span_to = Some(t);
                tool.span_raw = Some(range.to_string());
            }
            None => corrupt.push(Diagnostic::new(
                DiagKind::SemanticUnresolved,
                "SPAN_TOOL_UNKNOWN",
                format!("层范围引用了尚未定义的刀具 {code}（span 必须出现在 T 定义之后）"),
                pos,
            )),
        },
        _ => corrupt.push(Diagnostic::new(DiagKind::FormatCorrupt, "SPAN_BAD", "层范围数字非法或倒置", pos)),
    }
}

fn apply_plated(doc: &mut ExcellonDoc, rest: &str, pos: Pos, corrupt: &mut Vec<Diagnostic>) {
    let (code, flag) = match rest.split_once('=') {
        Some(pair) => (pair.0.trim().to_string(), pair.1.trim().to_ascii_lowercase()),
        None => {
            corrupt.push(Diagnostic::new(DiagKind::FormatCorrupt, "PLATED_BAD", ";@plated 需要 code=yes/no", pos));
            return;
        }
    };
    let plated = match flag.as_str() {
        "yes" | "pth" | "plated" => Some(true),
        "no" | "npth" | "nonplated" => Some(false),
        _ => None,
    };
    let plated = match plated {
        Some(v) => v,
        None => {
            corrupt.push(Diagnostic::new(DiagKind::FormatCorrupt, "PLATED_BAD", "电镀标记必须是 yes 或 no", pos));
            return;
        }
    };
    match doc.tools.iter_mut().find(|tool| tool.code == code) {
        Some(tool) => tool.plated = Some(plated),
        None => corrupt.push(Diagnostic::new(
            DiagKind::SemanticUnresolved,
            "PLATED_TOOL_UNKNOWN",
            format!("电镀标记引用了尚未定义的刀具 {code}"),
            pos,
        )),
    }
}

// ---------- 层叠说明 ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerRole {
    Copper,
    SolderMask,
    Silkscreen,
    DrillGuide,
    Dielectric,
}

impl LayerRole {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "copper" => LayerRole::Copper,
            "solder_mask" | "soldermask" | "mask" => LayerRole::SolderMask,
            "silkscreen" | "legend" => LayerRole::Silkscreen,
            "drill_guide" | "drillguide" => LayerRole::DrillGuide,
            "dielectric" | "core" | "prepreg" => LayerRole::Dielectric,
            _ => return None,
        })
    }
    pub fn code(self) -> &'static str {
        match self {
            LayerRole::Copper => "copper",
            LayerRole::SolderMask => "solder_mask",
            LayerRole::Silkscreen => "silkscreen",
            LayerRole::DrillGuide => "drill_guide",
            LayerRole::Dielectric => "dielectric",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackLayer {
    pub index: usize,
    pub name: String,
    pub role: LayerRole,
    pub thickness_nm: Option<i64>,
    pub thickness_raw: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackupDoc {
    pub layers: Vec<StackLayer>,
    pub raw: String,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn parse_stackup(text: &str) -> LoadOutcome<StackupDoc> {
    let mut diagnostics = Vec::new();
    let value: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            return LoadOutcome::FormatCorrupt(vec![Diagnostic::new(
                DiagKind::FormatCorrupt,
                "STACKUP_JSON",
                format!("层叠 JSON 损坏：{e}"),
                Pos::new(e.line(), e.column(), 0, ""),
            )])
        }
    };
    let mut layers = Vec::new();
    let arr = value.get("layers").and_then(|v| v.as_array());
    let arr = match arr {
        Some(a) => a,
        None => {
            return LoadOutcome::FormatCorrupt(vec![Diagnostic::new(
                DiagKind::FormatCorrupt,
                "STACKUP_SHAPE",
                "层叠说明必须包含 layers 数组",
                Pos::new(1, 1, 0, text),
            )])
        }
    };
    for (i, item) in arr.iter().enumerate() {
        let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let role_s = item.get("role").and_then(|v| v.as_str()).unwrap_or("");
        let role = match LayerRole::parse(role_s) {
            Some(r) => r,
            None => {
                diagnostics.push(Diagnostic::new(
                    DiagKind::SemanticUnresolved,
                    "LAYER_ROLE_UNKNOWN",
                    format!("第 {} 层角色 {role_s:?} 无法识别", i + 1),
                    Pos::new(i + 1, 1, 0, name),
                ));
                continue;
            }
        };
        let thickness_raw = item.get("thickness").and_then(|v| v.as_str()).map(|s| s.to_string());
        let thickness_nm = thickness_raw.as_deref().and_then(decimal_mm_to_nm);
        if thickness_raw.is_some() && thickness_nm.is_none() {
            diagnostics.push(Diagnostic::new(
                DiagKind::UnconvertibleValue,
                "THICKNESS_UNCONVERTED",
                format!("第 {} 层厚度无法转换，原值保留", i + 1),
                Pos::new(i + 1, 1, 0, thickness_raw.clone().unwrap_or_default()),
            ));
        }
        layers.push(StackLayer { index: i + 1, name, role, thickness_nm, thickness_raw });
    }
    let doc = StackupDoc { layers, raw: text.to_string(), diagnostics: diagnostics.clone() };
    if diagnostics.iter().any(|d| d.kind == DiagKind::SemanticUnresolved) {
        LoadOutcome::SemanticUnresolved(doc, diagnostics)
    } else {
        LoadOutcome::Ok(doc)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerMappingEntry {
    pub filename: String,
    pub layer_index: usize,
    pub declared_role: Option<LayerRole>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerMappingDoc {
    pub entries: Vec<LayerMappingEntry>,
    pub raw: String,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn parse_layer_mapping(text: &str) -> LoadOutcome<LayerMappingDoc> {
    let mut diagnostics = Vec::new();
    let value: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            return LoadOutcome::FormatCorrupt(vec![Diagnostic::new(
                DiagKind::FormatCorrupt,
                "MAPPING_JSON",
                format!("手工映射 JSON 损坏：{e}"),
                Pos::new(e.line(), e.column(), 0, ""),
            )])
        }
    };
    let mut entries = Vec::new();
    let arr = match value.get("mappings").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => {
            return LoadOutcome::FormatCorrupt(vec![Diagnostic::new(
                DiagKind::FormatCorrupt,
                "MAPPING_SHAPE",
                "手工映射必须包含 mappings 数组",
                Pos::new(1, 1, 0, text),
            )])
        }
    };
    for (i, item) in arr.iter().enumerate() {
        let filename = item.get("file").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let layer_index = item.get("layer").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        if filename.is_empty() || layer_index == 0 {
            diagnostics.push(Diagnostic::new(
                DiagKind::SemanticUnresolved,
                "MAPPING_ENTRY_BAD",
                format!("第 {i} 条映射缺少 file 或 layer"),
                Pos::new(i + 1, 1, 0, item.to_string()),
            ));
            continue;
        }
        let declared_role = item.get("role").and_then(|v| v.as_str()).and_then(LayerRole::parse);
        entries.push(LayerMappingEntry { filename, layer_index, declared_role });
    }
    let doc = LayerMappingDoc { entries, raw: text.to_string(), diagnostics: diagnostics.clone() };
    if diagnostics.iter().any(|d| d.kind == DiagKind::SemanticUnresolved) {
        LoadOutcome::SemanticUnresolved(doc, diagnostics)
    } else {
        LoadOutcome::Ok(doc)
    }
}
