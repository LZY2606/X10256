//! Gerber (RS-274X 子集) 解析器。
//! 保留：原文件坐标格式(FS)、单位(MO)、极性(LP)、aperture(AD/AM)、
//! step-repeat(SR)、图层角色提示(TF)，以及原始坐标文本。
use crate::types::{Diagnostic, Failure, Severity, Unit};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoordFormat {
    /// G01 绝对 / G91 相对
    pub absolute: bool,
    pub int_x: u8,
    pub dec_x: u8,
    pub int_y: u8,
    pub dec_y: u8,
    pub omit_leading: bool,
    pub raw: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "lowercase")]
pub enum ApertureShape {
    Circle { diameter: f64 },
    Rect { width: f64, height: f64 },
    Obround { width: f64, height: f64 },
    Macro { name: String, body: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Aperture {
    pub code: String, // 如 D10
    pub shape: ApertureShape,
    pub raw: String,  // 原行
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Polarity {
    Dark,
    Clear,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepRepeat {
    pub x: f64,
    pub y: f64,
    pub i: u32,
    pub j: u32,
    pub raw: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoordRaw {
    pub x_raw: Option<String>,
    pub y_raw: Option<String>,
    /// 规范化后的文件单位坐标（mm/inch），无法转换时为 None 并附诊断。
    pub x: Option<f64>,
    pub y: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Op {
    /// D02 移动
    Move(CoordRaw),
    /// D01 画线（线性；圆弧在解析时被拒绝并诊断，避免静默错误）
    Draw(CoordRaw),
    /// D03 曝光
    Flash(CoordRaw),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Graphic {
    pub op: Op,
    /// 生效极性
    pub polarity: Polarity,
    /// 生效 aperture（Move 可为 None）
    pub aperture: Option<String>,
    /// step-repeat 序号（0 表示基本步）
    pub sr_index: u32,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayerFile {
    pub unit: Option<Unit>,
    pub unit_raw: Option<String>,
    pub format: Option<CoordFormat>,
    pub apertures: Vec<Aperture>,
    pub step_repeat: Option<StepRepeat>,
    /// TF 文件属性中的图层角色提示（原始文本，不自动当结论）
    pub role_hint: Option<String>,
    pub graphics: Vec<Graphic>,
    pub diagnostics: Vec<Diagnostic>,
    pub failure: Option<Failure>,
}

struct Parser<'a> {
    lines: std::str::Lines<'a>,
    out: LayerFile,
    line_no: u32,
    cur_pol: Polarity,
    cur_ap: Option<String>,
    sr_index: u32,
    pos: (f64, f64),
    started: bool,
}

pub fn parse(src: &str, material_id: &str) -> LayerFile {
    let mut p = Parser {
        lines: src.lines(),
        out: LayerFile {
            unit: None,
            unit_raw: None,
            format: None,
            apertures: Vec::new(),
            step_repeat: None,
            role_hint: None,
            graphics: Vec::new(),
            diagnostics: Vec::new(),
            failure: None,
        },
        line_no: 0,
        cur_pol: Polarity::Dark,
        cur_ap: None,
        sr_index: 0,
        pos: (0.0, 0.0),
        started: false,
    };
    p.run(material_id);
    p.out
}

impl<'a> Parser<'a> {
    fn err(&mut self, code: &str, msg: impl Into<String>, line: u32, col: u32, mid: &str) {
        self.out.diagnostics.push(Diagnostic {
            severity: Severity::Error,
            code: code.to_string(),
            message: msg.into(),
            material_id: Some(mid.to_string()),
            line: Some(line),
            column: Some(col),
        });
    }

    fn run(&mut self, mid: &str) {
        let mut saw_end = false;
        while let Some(raw_line) = self.lines.next() {
            self.line_no += 1;
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('%') && line.len() == 1 {
                continue;
            }
            if line == "M02*" {
                saw_end = true;
                break;
            }
            if let Some(body) = line.strip_prefix('%').and_then(|l| l.strip_suffix('*')) {
                self.command_block(body, raw_line, mid);
                continue;
            }
            if line.starts_with("G02") || line.starts_with("G03") {
                self.out.failure = Some(Failure::SemanticUnsolvable {
                    detail: format!("line {}: 圆弧插补不在支持子集，避免用折线静默替代", self.line_no),
                });
                self.err("G03", "圆弧插补不支持", self.line_no, 1, mid);
                continue;
            }
            if line.contains('X') || line.contains('Y') || line.starts_with('D') {
                self.coord_line(line, mid);
            } else if !line.is_empty() {
                self.err("G??", format!("无法识别的行: {}", trunc(line)), self.line_no, 1, mid);
                if self.out.failure.is_none() {
                    self.out.failure = Some(Failure::CorruptFormat {
                        detail: format!("line {}: {}", self.line_no, trunc(line)),
                    });
                }
            }
        }
        if !saw_end && self.out.failure.is_none() {
            self.out.diagnostics.push(Diagnostic {
                severity: Severity::Warning,
                code: "EOF".into(),
                message: "缺少 M02* 结束标记".into(),
                material_id: Some(mid.into()),
                line: Some(self.line_no),
                column: None,
            });
        }
        // 缺 FS/MO 是语义不完整，不能用默认格式伪装成功。
        if self.out.format.is_none() && self.out.failure.is_none() {
            self.out.failure = Some(Failure::SemanticUnsolvable {
                detail: "缺少 %FSLAX...* 坐标格式声明，坐标无法确定解释".into(),
            });
        }
        if self.out.unit.is_none() {
            self.out.diagnostics.push(Diagnostic {
                severity: Severity::Error,
                code: "MO".into(),
                message: "缺少 %MOIN*/*%MOMM% 单位声明，坐标量没有单位".into(),
                material_id: Some(mid.into()),
                line: None,
                column: None,
            });
            if self.out.failure.is_none() {
                self.out.failure = Some(Failure::SemanticUnsolvable {
                    detail: "缺少单位声明".into(),
                });
            }
        }
    }

    fn command_block(&mut self, body: &str, raw_line: &str, mid: &str) {
        let body = body.strip_suffix('%').unwrap_or(body);
        if let Some(rest) = body.strip_prefix("FSLAX") {
            self.parse_fs(rest, raw_line, mid);
        } else if body == "MOMM" {
            self.out.unit = Some(Unit::Mm);
            self.out.unit_raw = Some(raw_line.to_string());
        } else if body == "MOIN" {
            self.out.unit = Some(Unit::Inch);
            self.out.unit_raw = Some(raw_line.to_string());
        } else if body == "LPD" {
            self.cur_pol = Polarity::Dark;
        } else if body == "LPC" {
            self.cur_pol = Polarity::Clear;
        } else if let Some(rest) = body.strip_prefix("AD") {
            self.parse_ad(rest, raw_line, mid);
        } else if body == "SR" {
            // %SR%* 结束 step-repeat 块
            self.out.step_repeat = None;
        } else if let Some(rest) = body.strip_prefix("SR") {
            self.parse_sr(rest, raw_line, mid);
        } else if body.starts_with("TF.FileFunction") || body.starts_with("TF.FileFunction,") {
            self.out.role_hint = Some(raw_line.to_string());
        } else if body.starts_with("TF") || body.starts_with("TA") || body.starts_with("TD") {
            // 其它属性保留为提示，不报错
        } else {
            self.err("CMD", format!("不支持的命令块: {}", trunc(body)), self.line_no, 2, mid);
        }
    }

    fn parse_fs(&mut self, rest: &str, raw_line: &str, mid: &str) {
        // 形如 24Y24 或 34Y34
        let mut chars = rest.chars();
        let ix = chars.next().and_then(|c| c.to_digit(10));
        let dx = chars.next().and_then(|c| c.to_digit(10));
        let y = chars.next();
        let iy = chars.next().and_then(|c| c.to_digit(10));
        let dy = chars.next().and_then(|c| c.to_digit(10));
        match (ix, dx, y, iy, dy) {
            (Some(ix), Some(dx), Some('Y'), Some(iy), Some(dy)) => {
                self.out.format = Some(CoordFormat {
                    absolute: true,
                    int_x: ix as u8,
                    dec_x: dx as u8,
                    int_y: iy as u8,
                    dec_y: dy as u8,
                    omit_leading: true,
                    raw: raw_line.to_string(),
                });
            }
            _ => {
                self.err("FS", "坐标格式声明损坏", self.line_no, 6, mid);
                self.out.failure = Some(Failure::CorruptFormat {
                    detail: format!("FS 声明损坏: {}", trunc(raw_line)),
                });
            }
        }
    }

    fn parse_ad(&mut self, rest: &str, raw_line: &str, mid: &str) {
        // D10C,0.5* 或 D11R,0.6X0.3*
        let (code, tail) = match rest.find(|c: char| c == 'C' || c == 'R' || c == 'O') {
            Some(idx) => (rest[..idx].to_string(), &rest[idx..]),
            None => {
                self.err("AD", "aperture 定义损坏", self.line_no, 3, mid);
                return;
            }
        };
        let code = if code.starts_with('D') { code } else { format!("D{}", code) };
        let tail = tail.strip_suffix('*').unwrap_or(tail);
        let shape = if let Some(v) = tail.strip_prefix('C') {
            let v = v.trim_start_matches(',').trim_end_matches('*');
            match v.parse::<f64>() {
                Ok(d) => ApertureShape::Circle { diameter: d },
                Err(_) => {
                    self.err("AD", "圆 aperture 直径不可解析", self.line_no, 4, mid);
                    return;
                }
            }
        } else if let Some(v) = tail.strip_prefix('R') {
            let v = v.trim_start_matches(',');
            let mut it = v.split('X');
            match (it.next().and_then(|s| s.parse().ok()), it.next().and_then(|s| s.parse().ok())) {
                (Some(w), Some(h)) => ApertureShape::Rect { width: w, height: h },
                _ => {
                    self.err("AD", "矩形 aperture 尺寸不可解析", self.line_no, 4, mid);
                    return;
                }
            }
        } else if let Some(v) = tail.strip_prefix('O') {
            let v = v.trim_start_matches(',');
            let mut it = v.split('X');
            match (it.next().and_then(|s| s.parse().ok()), it.next().and_then(|s| s.parse().ok())) {
                (Some(w), Some(h)) => ApertureShape::Obround { width: w, height: h },
                _ => {
                    self.err("AD", "obround aperture 尺寸不可解析", self.line_no, 4, mid);
                    return;
                }
            }
        } else {
            ApertureShape::Macro {
                name: tail.to_string(),
                body: raw_line.to_string(),
            }
        };
        self.out.apertures.push(Aperture { code, shape, raw: raw_line.to_string() });
    }

    fn parse_sr(&mut self, rest: &str, raw_line: &str, mid: &str) {
        // SRX10.0Y8.0I2J3*
        let rest = rest.strip_suffix('*').unwrap_or(rest);
        let mut x = 0.0;
        let mut y = 0.0;
        let mut i = 1u32;
        let mut j = 1u32;
        let mut k = 0usize;
        let bytes = rest.as_bytes();
        while k < bytes.len() {
            let key = bytes[k] as char;
            k += 1;
            let start = k;
            while k < bytes.len() && !matches!(bytes[k] as char, 'X' | 'Y' | 'I' | 'J') {
                k += 1;
            }
            let val: f64 = rest[start..k].parse().unwrap_or(0.0);
            match key {
                'X' => x = val,
                'Y' => y = val,
                'I' => i = val as u32,
                'J' => j = val as u32,
                _ => {}
            }
        }
        if x < 0.0 || y < 0.0 || i == 0 || j == 0 {
            self.err("SR", "step-repeat 参数非法", self.line_no, 3, mid);
            return;
        }
        self.out.step_repeat = Some(StepRepeat { x, y, i, j, raw: raw_line.to_string() });
        // SR 块内的每个图形按 I*J 展开；展开时用 sr_index 0..I*J-1 标记
        self.sr_index = 0;
    }

    fn coord_line(&mut self, line: &str, mid: &str) {
        // 可能形如 D10*（仅选 aperture）、X..Y..D02*、G01X..Y..D01*
        let (line, g_motion) = if let Some(rest) = line.strip_prefix("G01") {
            (rest, Some("G01"))
        } else if let Some(rest) = line.strip_prefix("G02") {
            (rest, Some("G02"))
        } else if let Some(rest) = line.strip_prefix("G03") {
            (rest, Some("G03"))
        } else if let Some(rest) = line.strip_prefix("G36") {
            (rest, Some("G36"))
        } else if let Some(rest) = line.strip_prefix("G37") {
            (rest, Some("G37"))
        } else {
            (line, None)
        };
        if let Some(g) = g_motion {
            if g == "G02" || g == "G03" {
                self.err("G03", "圆弧插补不支持", self.line_no, 1, mid);
                self.out.failure = Some(Failure::SemanticUnsolvable {
                    detail: "圆弧插补不支持".into(),
                });
                return;
            }
            if g == "G36" || g == "G37" {
                self.err("REGION", "区域填充 G36/G37 不在支持子集", self.line_no, 1, mid);
                return;
            }
        }
        let line = line.strip_suffix('*').unwrap_or(line);
        // 单独 D 码选择
        if !line.contains('X') && !line.contains('Y') {
            if let Some(code) = line.strip_prefix('D') {
                if code.chars().all(|c| c.is_ascii_digit()) {
                    let n: u32 = code.parse().unwrap_or(0);
                    if n >= 10 {
                        self.cur_ap = Some(format!("D{}", n));
                        return;
                    }
                }
            }
            self.err("D", format!("无法识别: {}", trunc(line)), self.line_no, 1, mid);
            return;
        }
        let cr = self.parse_coord(line, mid);
        let op_code = extract_op(line);
        let op = match op_code.as_deref() {
            Some("D02") => Op::Move(cr),
            Some("D03") => Op::Flash(cr),
            Some("D01") | None => Op::Draw(cr),
            Some(other) => {
                self.err("D", format!("未知绘图码 {}", other), self.line_no, 1, mid);
                return;
            }
        };
        self.emit(op, mid);
    }

    fn emit(&mut self, op: Op, _mid: &str) {
        // step-repeat 展开：每个图形按 I*J 复制，偏移 (k*X, l*Y)。
        if let Some(sr) = &self.out.step_repeat {
            let mut idx = 0u32;
            for j in 0..sr.j {
                for i in 0..sr.i {
                    let mut g = op.clone();
                    apply_sr_offset(&mut g, i as f64 * sr.x, j as f64 * sr.y);
                    self.out.graphics.push(Graphic {
                        op: g,
                        polarity: self.cur_pol,
                        aperture: self.cur_ap.clone(),
                        sr_index: idx,
                        line: self.line_no,
                    });
                    idx += 1;
                }
            }
        } else {
            self.out.graphics.push(Graphic {
                op,
                polarity: self.cur_pol,
                aperture: self.cur_ap.clone(),
                sr_index: 0,
                line: self.line_no,
            });
        }
    }

    fn parse_coord(&mut self, line: &str, mid: &str) -> CoordRaw {
        let (x_raw, y_raw) = split_xy(line);
        let fmt = match &self.out.format {
            Some(f) => f.clone(),
            None => {
                self.err("FS", "坐标出现时仍无格式声明", self.line_no, 1, mid);
                return CoordRaw { x_raw, y_raw, x: None, y: None };
            }
        };
        let x = x_raw.as_deref().map(|r| coord_value(r, fmt.int_x, fmt.dec_x));
        let y = y_raw.as_deref().map(|r| coord_value(r, fmt.int_y, fmt.dec_y));
        CoordRaw { x_raw, y_raw, x, y }
    }
}

fn trunc(s: &str) -> String {
    if s.chars().count() > 40 {
        format!("{}…", s.chars().take(40).collect::<String>())
    } else {
        s.to_string()
    }
}

fn split_xy(line: &str) -> (Option<String>, Option<String>) {
    let mut x = None;
    let mut y = None;
    let bytes = line.as_bytes();
    let mut k = 0;
    while k < bytes.len() {
        match bytes[k] as char {
            'X' => {
                let start = k + 1;
                let mut e = start;
                while e < bytes.len() && !matches!(bytes[e] as char, 'X' | 'Y' | 'D' | 'G') {
                    e += 1;
                }
                x = Some(line[start..e].to_string());
                k = e;
            }
            'Y' => {
                let start = k + 1;
                let mut e = start;
                while e < bytes.len() && !matches!(bytes[e] as char, 'X' | 'Y' | 'D' | 'G') {
                    e += 1;
                }
                y = Some(line[start..e].to_string());
                k = e;
            }
            _ => k += 1,
        }
    }
    (x, y)
}

fn extract_op(line: &str) -> Option<String> {
    if let Some(idx) = line.find('D') {
        let tail = &line[idx + 1..];
        let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.len() >= 2 {
            return Some(format!("D{}", digits));
        }
    }
    None
}

/// Gerber 省略前导零格式：如 24 格式下 "13500" => 0013.500。
fn coord_value(raw: &str, int_digits: u8, dec_digits: u8) -> f64 {
    let neg = raw.starts_with('-');
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    let total = (int_digits + dec_digits) as usize;
    let padded = if digits.len() < total {
        format!("{:0>width$}", digits, width = total)
    } else {
        digits
    };
    let (ip, dp) = padded.split_at(int_digits as usize);
    let v: f64 = format!("{}.{}", ip, dp).parse().unwrap_or(0.0);
    if neg { -v } else { v }
}

fn apply_sr_offset(op: &mut Op, dx: f64, dy: f64) {
    let cr = match op {
        Op::Move(c) | Op::Draw(c) | Op::Flash(c) => c,
    };
    if let Some(x) = cr.x.as_mut() {
        *x += dx;
    }
    if let Some(y) = cr.y.as_mut() {
        *y += dy;
    }
}
