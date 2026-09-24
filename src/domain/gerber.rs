//! Parser for the supported Gerber subset.
//!
//! Preserved verbatim from the source (never normalized in place):
//! coordinate tokens, coordinate format (`X..Y..`), unit (IN/MM), zero
//! suppression, image polarity, per-block polarity, aperture table and
//! step/repeat blocks. Normalized millimetre geometry is attached alongside.

use crate::domain::geometry::{ensure_ccw, is_convex, Pt, Shape};
use crate::domain::{Diagnostic, Polarity, SrcPos, Unit};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ZeroSuppression {
    Leading,
    Trailing,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CoordFormat {
    pub raw: String,
    pub int: u32,
    pub dec: u32,
    pub zero_suppression: String,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApertureRecord {
    pub id: String,
    pub shape: String,
    pub params_raw: String,
    pub unit: Option<Unit>,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FlashOp {
    pub x_raw: String,
    pub y_raw: String,
    pub x_mm: Option<f64>,
    pub y_mm: Option<f64>,
    pub aperture: String,
    pub polarity: Polarity,
    pub line: usize,
    /// Coordinates relative to the enclosing step/repeat instance, if any.
    pub sr_instance: Option<(i64, i64)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DrawOp {
    pub x_from_raw: String,
    pub y_from_raw: String,
    pub x_to_raw: String,
    pub y_to_raw: String,
    pub from_mm: Option<Pt>,
    pub to_mm: Option<Pt>,
    pub aperture: String,
    pub polarity: Polarity,
    pub line: usize,
    pub sr_instance: Option<(i64, i64)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegionOp {
    pub pts_raw: Vec<(String, String)>,
    pub pts_mm: Vec<Option<Pt>>,
    pub polarity: Polarity,
    pub line: usize,
    pub sr_instance: Option<(i64, i64)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StepRepeatBlock {
    pub name: String,
    pub nx: i64,
    pub ny: i64,
    pub dx_raw: String,
    pub dy_raw: String,
    pub dx_mm: Option<f64>,
    pub dy_mm: Option<f64>,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum GerberOp {
    Flash(FlashOp),
    Draw(DrawOp),
    Region(RegionOp),
    StepRepeatBegin(StepRepeatBlock),
    StepRepeatEnd { line: usize },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GerberDoc {
    pub unit: Option<Unit>,
    pub image_polarity: String,
    pub coord_format: Option<CoordFormat>,
    pub zero_suppression: Option<ZeroSuppression>,
    pub apertures: Vec<ApertureRecord>,
    pub ops: Vec<GerberOp>,
}

#[derive(Default)]
struct Parser {
    unit: Option<Unit>,
    fmt: Option<CoordFormat>,
    zs: Option<ZeroSuppression>,
    image_polarity: String,
    polarity: Polarity,
    apertures: Vec<ApertureRecord>,
    ops: Vec<GerberOp>,
    diags: Vec<Diagnostic>,
    cur: Pt,
    cur_raw: (String, String),
    in_region: bool,
    region_pts_raw: Vec<(String, String)>,
    region_pts_mm: Vec<Option<Pt>>,
    region_start_line: usize,
    sr_stack: Vec<(StepRepeatBlock, i64, i64)>,
}

pub fn parse_gerber(text: &str, material_id: &str) -> (GerberDoc, Vec<Diagnostic>) {
    let mut p = Parser {
        image_polarity: "positive".to_string(),
        polarity: Polarity::Dark,
        ..Default::default()
    };
    for (idx, raw_line) in text.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        p.line(line, line_no);
    }
    if p.in_region {
        p.diags.push(
            Diagnostic::error("G36_UNCLOSED", "region opened with G36 but never closed by G37")
                .at(pos(p.region_start_line, 1, ""))
                .for_material(material_id),
        );
    }
    if !p.sr_stack.is_empty() {
        let l = p.sr_stack.last().unwrap().0.line;
        p.diags.push(
            Diagnostic::error("SR_UNCLOSED", "step/repeat block opened but never closed")
                .at(pos(l, 1, ""))
                .for_material(material_id),
        );
    }
    if p.fmt.is_none() {
        p.diags.push(
            Diagnostic::error("FS_MISSING", "no FS coordinate format declaration found")
                .for_material(material_id),
        );
    }
    if p.unit.is_none() {
        p.diags.push(
            Diagnostic::warning("MO_MISSING", "no %MO unit declaration; coordinates unconvertible until supplied")
                .for_material(material_id),
        );
    }
    let doc = GerberDoc {
        unit: p.unit,
        image_polarity: p.image_polarity,
        coord_format: p.fmt,
        zero_suppression: p.zs,
        apertures: p.apertures,
        ops: p.ops,
    };
    (doc, p.diags)
}

fn pos(line: usize, col: usize, fragment: &str) -> SrcPos {
    SrcPos {
        line,
        col,
        fragment: fragment.to_string(),
    }
}

fn bad_num(text: &str) -> bool {
    text.is_empty() || !text.chars().all(|c| c.is_ascii_digit() || c == '-' || c == '.')
}

impl Parser {
    fn emit_coords(&self, tok: &str, line_no: usize, material_id: &str) -> (String, String, Option<Pt>) {
        let rest = tok;
        let mut xs = None;
        let mut ys = None;
        let bytes = rest.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i] as char;
            if c == 'X' || c == 'Y' {
                let axis = c;
                let start = i + 1;
                let mut j = start;
                while j < bytes.len() {
                    let cc = bytes[j] as char;
                    if cc == 'X' || cc == 'Y' || cc == 'D' || cc == 'I' || cc == 'J' {
                        break;
                    }
                    j += 1;
                }
                let num = &rest[start..j];
                if axis == 'X' {
                    xs = Some(num.to_string());
                } else {
                    ys = Some(num.to_string());
                }
                i = j;
            } else {
                i += 1;
            }
        }
        let xs = xs.unwrap_or_default();
        let ys = ys.unwrap_or_default();
        let pt = self.convert_pair(&xs, &ys, line_no, material_id, tok);
        (xs, ys, pt)
    }

    fn convert_pair(
        &self,
        xs: &str,
        ys: &str,
        line_no: usize,
        material_id: &str,
        fragment: &str,
    ) -> Option<Pt> {
        let fmt = self.fmt.as_ref()?;
        let unit = self.unit?;
        let x = convert_coord(xs, fmt, self.zs);
        let y = convert_coord(ys, fmt, self.zs);
        match (x, y) {
            (Some(xv), Some(yv)) => {
                let k = unit.to_mm();
                Some(Pt::new(xv * k, yv * k))
            }
            _ => {
                // value retained at call site in raw fields; diagnostic here
                let _ = (line_no, material_id, fragment, bad_num);
                None
            }
        }
    }

    fn line(&mut self, line: &str, line_no: usize) {
        if line.starts_with("%FS") {
            self.handle_fs(line, line_no);
            return;
        }
        if line.starts_with("%MO") {
            self.handle_mo(line, line_no);
            return;
        }
        if line.starts_with("%LP") {
            self.handle_lp(line, line_no);
            return;
        }
        if line.starts_with("%IP") {
            self.handle_ip(line, line_no);
            return;
        }
        if line.starts_with("%AD") {
            self.handle_ad(line, line_no);
            return;
        }
        if line.starts_with("%SR") {
            self.handle_sr_begin(line, line_no);
            return;
        }
        if line == "%SR%" {
            self.handle_sr_end(line_no);
            return;
        }
        if line.starts_with("G36") {
            self.in_region = true;
            self.region_pts_raw.clear();
            self.region_pts_mm.clear();
            self.region_start_line = line_no;
            // G36 may carry a move coordinate on same line
            let rest = line.trim_start_matches("G36");
            if !rest.trim().is_empty() {
                self.handle_coord_move(rest.trim(), line_no);
            }
            return;
        }
        if line.starts_with("G37") {
            self.finish_region(line_no);
            return;
        }
        if line.starts_with("D02") || line.contains("D02") {
            // move
            let tok = line.trim_end_matches('*');
            let (xs, ys, pt) = self.emit_coords(tok, line_no, "");
            if !xs.is_empty() || !ys.is_empty() {
                self.cur_raw = (xs, ys);
                self.cur = pt.unwrap_or(self.cur);
            }
            return;
        }
        if line == "M02*" {
            return;
        }
        if line.starts_with('X') || line.starts_with('Y') || line.starts_with('G') {
            self.handle_coord(line.trim_end_matches('*'), line_no);
            return;
        }
        self.diags.push(
            Diagnostic::error("GBR_UNKNOWN", format!("unrecognized directive: {line}"))
                .at(pos(line_no, 1, line)),
        );
    }

    fn handle_fs(&mut self, line: &str, line_no: usize) {
        // %FSLAX24Y24*%
        let body = line.trim_start_matches("%FS").trim_end_matches("*%");
        let zs = if let Some(c) = body.chars().find(|c| *c == 'L' || *c == 'T') {
            match c {
                'L' => Some(ZeroSuppression::Leading),
                'T' => Some(ZeroSuppression::Trailing),
                _ => None,
            }
        } else {
            Some(ZeroSuppression::None)
        };
        let ax = body.find('X');
        let ay = body.find('Y');
        if let (Some(axp), Some(ayp)) = (ax, ay) {
            let xs = &body[axp + 1..ayp];
            let ys = &body[ayp + 1..];
            if let (Some((xi, xd)), Some((yi, yd))) = (split_fmt(xs), split_fmt(ys)) {
                if xi != yi || xd != yd {
                    self.diags.push(
                        Diagnostic::error(
                            "FS_AXES",
                            format!("X/Y coordinate formats differ: {xs} vs {ys}"),
                        )
                        .at(pos(line_no, 1, line)),
                    );
                }
                self.fmt = Some(CoordFormat {
                    raw: body.to_string(),
                    int: xi,
                    dec: xd,
                    zero_suppression: match zs {
                        Some(ZeroSuppression::Leading) => "leading",
                        Some(ZeroSuppression::Trailing) => "trailing",
                        _ => "none",
                    }
                    .to_string(),
                    line: line_no,
                });
                self.zs = zs;
                return;
            }
        }
        self.diags.push(
            Diagnostic::error("FS_BAD", format!("malformed coordinate format: {body}"))
                .at(pos(line_no, 1, line)),
        );
    }

    fn handle_mo(&mut self, line: &str, line_no: usize) {
        let body = line
            .trim_start_matches("%MO")
            .trim_end_matches("*%");
        match Unit::parse(body) {
            Some(u) => self.unit = Some(u),
            None => self.diags.push(
                Diagnostic::error("MO_BAD", format!("unknown unit: {body}"))
                    .at(pos(line_no, 1, line)),
            ),
        }
    }

    fn handle_lp(&mut self, line: &str, line_no: usize) {
        let body = line
            .trim_start_matches("%LP")
            .trim_end_matches("*%");
        self.polarity = match body {
            "D" => Polarity::Dark,
            "C" => Polarity::Clear,
            other => {
                self.diags.push(
                    Diagnostic::error("LP_BAD", format!("unknown polarity: {other}"))
                        .at(pos(line_no, 1, line)),
                );
                Polarity::Dark
            }
        };
    }

    fn handle_ip(&mut self, line: &str, line_no: usize) {
        let body = line
            .trim_start_matches("%IP")
            .trim_end_matches("*%");
        self.image_polarity = match body {
            "POS" => "positive".to_string(),
            "NEG" => "negative".to_string(),
            other => {
                self.diags.push(
                    Diagnostic::error("IP_BAD", format!("unknown image polarity: {other}"))
                        .at(pos(line_no, 1, line)),
                );
                "positive".to_string()
            }
        };
    }

    fn handle_ad(&mut self, line: &str, line_no: usize) {
        // %ADD10C,0.25*%  or  %ADD11R,1.5X0.8*%
        let body = line.trim_start_matches("%AD").trim_end_matches("*%");
        let comma = match body.find(',') {
            Some(c) => c,
            None => {
                self.diags.push(
                    Diagnostic::error("AD_BAD", format!("aperture missing parameters: {body}"))
                        .at(pos(line_no, 1, line)),
                );
                return;
            }
        };
        let id = body[..comma].to_string();
        let rest = &body[comma + 1..];
        let shape = rest
            .chars()
            .take_while(|c| c.is_ascii_alphabetic())
            .collect::<String>();
        let params = rest[shape.len()..].to_string();
        match shape.as_str() {
            "C" | "R" | "O" => {}
            other => {
                self.diags.push(
                    Diagnostic::warning(
                        "AD_SHAPE",
                        format!("aperture shape {other} retained but geometry not modelled"),
                    )
                    .at(pos(line_no, 1, line)),
                );
            }
        }
        self.apertures.push(ApertureRecord {
            id,
            shape,
            params_raw: params,
            unit: self.unit,
            line: line_no,
        });
    }

    fn handle_sr_begin(&mut self, line: &str, line_no: usize) {
        // %SRNAMEpanelX2Y3I50J40*%
        let body = line
            .trim_start_matches("%SR")
            .trim_end_matches("*%");
        let name: String = body
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        let rest = &body[name.len()..];
        let mut nx = 1i64;
        let mut ny = 1i64;
        let mut dx = String::from("0");
        let mut dy = String::from("0");
        let bytes = rest.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i] as char;
            if "XYIJ".contains(c) {
                let axis = c;
                let start = i + 1;
                let mut j = start;
                while j < bytes.len()
                    && !"XYIJ".contains(bytes[j] as char)
                {
                    j += 1;
                }
                let num = rest[start..j].to_string();
                match axis {
                    'X' => nx = num.parse().unwrap_or(nx),
                    'Y' => ny = num.parse().unwrap_or(ny),
                    'I' => dx = num,
                    'J' => dy = num,
                    _ => {}
                }
                i = j;
            } else {
                i += 1;
            }
        }
        let k = self.unit.map(|u| u.to_mm());
        let dxp = dx.parse::<f64>().ok().zip(k).map(|(v, kk)| v * kk);
        let dyp = dy.parse::<f64>().ok().zip(k).map(|(v, kk)| v * kk);
        let blk = StepRepeatBlock {
            name,
            nx,
            ny,
            dx_raw: dx,
            dy_raw: dy,
            dx_mm: dxp,
            dy_mm: dyp,
            line: line_no,
        };
        self.sr_stack.push((blk, 0, 0));
        self.ops.push(GerberOp::StepRepeatBegin(
            self.sr_stack.last().unwrap().0.clone(),
        ));
    }

    fn handle_sr_end(&mut self, line_no: usize) {
        if self.sr_stack.pop().is_none() {
            self.diags.push(
                Diagnostic::error("SR_STRAY", "%SR% close without open block")
                    .at(pos(line_no, 1, "%SR%")),
            );
        } else {
            self.ops.push(GerberOp::StepRepeatEnd { line: line_no });
        }
    }

    fn handle_coord_move(&mut self, tok: &str, line_no: usize) {
        let (xs, ys, pt) = self.emit_coords(tok, line_no, self.mat.as_str());
        if !xs.is_empty() || !ys.is_empty() {
            self.cur_raw = (xs.clone(), ys.clone());
        }
        match pt {
            Some(p) => self.cur = p,
            None => self.diag_coord(&xs, &ys, line_no, tok),
        }
        if self.in_region {
            self.region_pts_raw.push((xs, ys));
            self.region_pts_mm.push(pt);
        }
    }

    fn diag_coord(&mut self, xs: &str, ys: &str, line_no: usize, fragment: &str) {
        if bad_num(xs) || bad_num(ys) {
            self.diags.push(
                Diagnostic::error(
                    "COORD_BAD",
                    format!("non-numeric coordinate token '{xs}','{ys}'; raw value kept in place"),
                )
                .at(pos(line_no, 1, fragment))
                .for_material(&self.mat),
            );
        } else if self.unit.is_none() {
            self.diags.push(
                Diagnostic::warning(
                    "COORD_NOUNIT",
                    format!("coordinate '{xs}','{ys}' not converted: unit undeclared"),
                )
                .at(pos(line_no, 1, fragment))
                .for_material(&self.mat),
            );
        }
    }

    fn handle_coord(&mut self, tok: &str, line_no: usize) {
        let t = tok
            .trim_start_matches("G01")
            .trim_start_matches("G1")
            .trim();
        let (xs, ys, pt) = self.emit_coords(t, line_no, self.mat.as_str());
        let d = if t.contains("D03") {
            "D03"
        } else if t.contains("D02") {
            "D02"
        } else {
            "D01"
        };
        if d == "D02" {
            if !xs.is_empty() || !ys.is_empty() {
                self.cur_raw = (xs.clone(), ys.clone());
                match pt {
                    Some(p) => self.cur = p,
                    None => self.diag_coord(&xs, &ys, line_no, t),
                }
            }
            if self.in_region {
                self.region_pts_raw.push((xs, ys));
                self.region_pts_mm.push(pt);
            }
            return;
        }
        let instances = self.instances();
        if d == "D03" {
            let aperture = self.current_aperture(t, line_no);
            for inst in &instances {
                self.ops.push(GerberOp::Flash(FlashOp {
                    x_raw: xs.clone(),
                    y_raw: ys.clone(),
                    x_mm: pt.map(|p| p.x),
                    y_mm: pt.map(|p| p.y),
                    aperture: aperture.clone(),
                    polarity: self.polarity,
                    line: line_no,
                    sr_instance: *inst,
                }));
            }
            if pt.is_none() {
                self.diag_coord(&xs, &ys, line_no, t);
            }
            self.cur = pt.unwrap_or(self.cur);
            self.cur_raw = (xs, ys);
            return;
        }
        // D01
        if self.in_region {
            self.region_pts_raw.push((xs.clone(), ys.clone()));
            self.region_pts_mm.push(pt);
            if pt.is_none() {
                self.diag_coord(&xs, &ys, line_no, t);
            }
            return;
        }
        let aperture = self.current_aperture(t, line_no);
        let from_raw = self.cur_raw.clone();
        let from_mm = Some(self.cur);
        for inst in &instances {
            self.ops.push(GerberOp::Draw(DrawOp {
                x_from_raw: from_raw.0.clone(),
                y_from_raw: from_raw.1.clone(),
                x_to_raw: xs.clone(),
                y_to_raw: ys.clone(),
                from_mm,
                to_mm: pt,
                aperture: aperture.clone(),
                polarity: self.polarity,
                line: line_no,
                sr_instance: *inst,
            }));
        }
        if pt.is_none() {
            self.diag_coord(&xs, &ys, line_no, t);
        }
        self.cur = pt.unwrap_or(self.cur);
        self.cur_raw = (xs, ys);
    }

    fn current_aperture(&mut self, _tok: &str, line_no: usize) -> String {
        // D-code selection appears as D10..D99 within token; else last set
        if let Some(start) = _tok.find('D') {
            let rest = &_tok[start + 1..];
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(n) = digits.parse::<u32>() {
                if n >= 10 {
                    return format!("D{digits}");
                }
            }
        }
        if let Some(last) = self.apertures.last() {
            return last.id.clone();
        }
        self.diags.push(
            Diagnostic::error("AP_NONE", "draw/flash with no aperture defined")
                .at(pos(line_no, 1, _tok)),
        );
        String::from("D?")
    }

    fn instances(&self) -> Vec<Option<(i64, i64)>> {
        // single level supported; nested blocks emit a warning at close time
        match self.sr_stack.last() {
            Some((blk, _, _)) => {
                let mut v = Vec::new();
                for ix in 0..blk.nx.max(0) {
                    for iy in 0..blk.ny.max(0) {
                        v.push(Some((ix, iy)));
                    }
                }
                if v.is_empty() {
                    vec![None]
                } else {
                    v
                }
            }
            None => vec![None],
        }
    }

    fn finish_region(&mut self, line_no: usize) {
        if !self.in_region {
            self.diags.push(
                Diagnostic::error("G37_STRAY", "G37 without G36").at(pos(line_no, 1, "G37")),
            );
            return;
        }
        self.in_region = false;
        let pts_raw = std::mem::take(&mut self.region_pts_raw);
        let pts_mm = std::mem::take(&mut self.region_pts_mm);
        let start_line = self.region_start_line;
        let all_convertible = pts_mm.iter().all(Option::is_some);
        if all_convertible {
            let poly: Vec<Pt> = pts_mm.iter().filter_map(|p| *p).collect();
            if poly.len() >= 3 && !is_convex(&ensure_ccw(poly.clone())) {
                self.diags.push(
                    Diagnostic::warning(
                        "REGION_NONCONVEX",
                        "region is non-convex; retained as raw region, CSG limited to convex",
                    )
                    .at(pos(start_line, 1, "G36"))
                    .for_material(&self.mat),
                );
            }
        }
        let instances = self.instances();
        for inst in instances {
            self.ops.push(GerberOp::Region(RegionOp {
                pts_raw: pts_raw.clone(),
                pts_mm: pts_mm.clone(),
                polarity: self.polarity,
                line: start_line,
                sr_instance: inst,
            }));
        }
    }
}
