//! Gerber RS-274X subset parser.
//!
//! Preserved from the source file (never normalised away):
//! coordinate format (`%FSLAX...`), unit (`%MO`), polarity (`%LP`),
//! aperture definitions (`%ADD`), step/repeat blocks (`%SR...%` ... `%SR%`),
//! raw directive text and byte-anchored positions.
//! Conversion to canonical mm geometry happens in [`crate::engine`].

use crate::model::{CoordFormat, Diagnostic, LengthUnit, Severity, SrcPos};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Polarity {
    Dark,
    Clear,
}

impl Polarity {
    pub fn label(self) -> &'static str {
        match self {
            Polarity::Dark => "D",
            Polarity::Clear => "C",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApertureShape {
    Circle,
    Rect,
    Obround,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ApertureGeom {
    Circle {
        diameter_raw: f64,
    },
    Rect {
        x_size_raw: f64,
        y_size_raw: f64,
    },
    Obround {
        x_size_raw: f64,
        y_size_raw: f64,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Aperture {
    pub code: String,
    pub shape: ApertureShape,
    pub geom: ApertureGeom,
    /// Raw declaration, e.g. `%ADD10C,0.25*%`.
    pub raw: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FeatureShape {
    /// Flash at `(x, y)` with the given aperture.
    Flash {
        aperture: String,
        x_raw: f64,
        y_raw: f64,
    },
    /// Linear draw between two points (G01) with the current aperture.
    Draw {
        aperture: String,
        x1_raw: f64,
        y1_raw: f64,
        x2_raw: f64,
        y2_raw: f64,
    },
    /// Filled axis-aligned rectilinear region (G36...G37).
    RegionRect {
        x_raw: f64,
        y_raw: f64,
        w_raw: f64,
        h_raw: f64,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Feature {
    /// Zero-based operation order. Polarity overlay is resolved in this order.
    pub order: usize,
    pub polarity: Polarity,
    pub shape: FeatureShape,
    /// Step/repeat instance identity ("base" or `sr:<n>:<i>:<j>`), base order.
    pub sr_instance: String,
    pub sr_base_order: Option<usize>,
    pub raw: String,
    pub pos: Option<SrcPos>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct StepRepeat {
    pub x_repeats: u32,
    pub y_repeats: u32,
    pub dx_raw: f64,
    pub dy_raw: f64,
    /// Template directives, in source order, kept for replay/provenance.
    pub template: Vec<RawOp>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RawOp {
    pub text: String,
    pub line: usize,
    pub col: usize,
    pub byte_start: usize,
    pub byte_end: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct GerberDoc {
    pub format: Option<CoordFormat>,
    pub unit: Option<LengthUnit>,
    pub polarity_initial: Option<Polarity>,
    pub apertures: Vec<Aperture>,
    /// Fully expanded features (including step/repeat instances), ordered.
    pub features: Vec<Feature>,
    pub step_repeats: Vec<StepRepeat>,
    pub raw_ops: Vec<RawOp>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn parse(input: &str, file_id: &str) -> Result<GerberDoc, (GerberDoc, Vec<Diagnostic>)> {
    let mut p = Parser::new(input, file_id);
    p.run();
    if p.doc.diagnostics.iter().any(|d| d.severity == Severity::Error) {
        Err((p.doc, p.doc.diagnostics.clone()))
    } else {
        Ok(p.doc)
    }
}

struct Parser<'a> {
    src: &'a str,
    file_id: String,
    byte_cursor: usize,
    doc: GerberDoc,
    cur_polarity: Polarity,
    cur_aperture: Option<String>,
    cur_x: f64,
    cur_y: f64,
    pen_x: f64,
    pen_y: f64,
    in_region: bool,
    region_points: Vec<(f64, f64)>,
    in_sr: bool,
    sr_template: Vec<RawOp>,
    sr: Option<StepRepeat>,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str, file_id: &str) -> Self {
        Parser {
            src,
            file_id: file_id.to_string(),
            byte_cursor: 0,
            doc: GerberDoc::default(),
            cur_polarity: Polarity::Dark,
            cur_aperture: None,
            cur_x: 0.0,
            cur_y: 0.0,
            pen_x: 0.0,
            pen_y: 0.0,
            in_region: false,
            region_points: Vec::new(),
            in_sr: false,
            sr_template: Vec::new(),
            sr: None,
        }
    }

    fn pos_for(&self, byte_start: usize, byte_end: usize, line: usize, col: usize) -> SrcPos {
        let snippet: String = self
            .src
            .get(byte_start..byte_end)
            .unwrap_or("")
            .chars()
            .take(80)
            .collect();
        SrcPos {
            file_id: self.file_id.clone(),
            line,
            col,
            byte_start,
            byte_end,
            snippet,
        }
    }

    fn err(&mut self, code: &str, msg: impl Into<String>, op: &RawOp) {
        self.doc.diagnostics.push(
            Diagnostic::err(code, msg).at(self.pos_for(op.byte_start, op.byte_end, op.line, op.col)),
        );
    }

    fn warn(&mut self, code: &str, msg: impl Into<String>, op: &RawOp) {
        self.doc.diagnostics.push(
            Diagnostic::warn(code, msg).at(self.pos_for(op.byte_start, op.byte_end, op.line, op.col)),
        );
    }

    fn run(&mut self) {
        let ops = split_ops(self.src, &mut self.byte_cursor);
        let mut saw_end = false;
        for op in ops {
            self.doc.raw_ops.push(op.clone());
            let text = op.text.trim();
            if text.is_empty() {
                continue;
            }
            if text == "M02" {
                saw_end = true;
                break;
            }
            if self.in_sr && !is_sr_boundary(text) {
                self.sr_template.push(op.clone());
                if text.starts_with('%') {
                    // Polarity inside SR is part of the template; skip top-level handling.
                    self.dispatch_directive(text, &op, /*template=*/ true);
                } else {
                    self.dispatch_plot(text, &op, /*template=*/ true);
                }
                continue;
            }
            if text.starts_with('%') {
                self.dispatch_directive(text, &op, false);
            } else {
                self.dispatch_plot(text, &op, false);
            }
        }
        if self.in_sr {
            self.err("GERBER_SR_UNCLOSED", "step/repeat block opened with %SR% but never closed", self.doc.raw_ops.last().unwrap());
        }
        if !saw_end {
            // Not fatal: many real files omit M02, but it indicates possible truncation.
            let op = self
                .doc
                .raw_ops
                .last()
                .cloned()
                .unwrap_or(RawOp { text: String::new(), line: 1, col: 1, byte_start: 0, byte_end: 0 });
            self.warn("GERBER_NO_END", "file ends without M02; possible truncation", &op);
        }
        if self.doc.format.is_none() {
            self.err(
                "GERBER_NO_FORMAT",
                "missing %FSLAX... coordinate format specification; coordinates cannot be decoded",
                self.doc.raw_ops.first().cloned().unwrap_or(RawOp {
                    text: String::new(),
                    line: 1,
                    col: 1,
                    byte_start: 0,
                    byte_end: 0,
                })
                .as_ref_op(),
            );
        }
        if self.doc.unit.is_none() {
            let op = self.doc.raw_ops.first().cloned().unwrap_or(RawOp {
                text: String::new(),
                line: 1,
                col: 1,
                byte_start: 0,
                byte_end: 0,
            });
            self.err("GERBER_NO_UNIT", "missing %MO unit specification", &op);
        }
    }

    fn dispatch_directive(&mut self, text: &str, op: &RawOp, template: bool) {
        let inner = text.trim_start_matches('%').trim_end_matches('%');
        if let Some(rest) = inner.strip_prefix("FSLAX") {
            if let Some((i, d)) = parse_fs(rest) {
                self.doc.format = Some(CoordFormat { int_digits: i, dec_digits: d });
            } else {
                self.err("GERBER_BAD_FORMAT", format!("unparseable coordinate format `{inner}`"), op);
            }
        } else if let Some(rest) = inner.strip_prefix("MO") {
            match rest {
                "MM" => self.doc.unit = Some(LengthUnit::Mm),
                "IN" => self.doc.unit = Some(LengthUnit::In),
                other => self.err("GERBER_BAD_UNIT", format!("unknown unit `{other}`"), op),
            }
        } else if let Some(rest) = inner.strip_prefix("LP") {
            match rest {
                "D" => {
                    self.cur_polarity = Polarity::Dark;
                    if self.doc.polarity_initial.is_none() {
                        self.doc.polarity_initial = Some(Polarity::Dark);
                    }
                }
                "C" => self.cur_polarity = Polarity::Clear,
                other => self.err("GERBER_BAD_POLARITY", format!("unknown polarity `{other}`"), op),
            }
        } else if let Some(rest) = inner.strip_prefix("AD") {
            match parse_aperture(rest) {
                Some(ap) => self.doc.apertures.push(ap),
                None => self.err("GERBER_BAD_APERTURE", format!("unparseable aperture `%{inner}%`"), op),
            }
        } else if inner == "LPD" || inner == "LPC" {
            // already handled above
        } else if let Some(rest) = inner.strip_prefix("SR") {
            self.handle_sr(rest, op, template);
        } else if inner.starts_with("TF") || inner.starts_with("TA") || inner.starts_with("TD") {
            // Layer attributes: retained as raw ops, not interpreted as geometry.
        } else {
            self.warn("GERBER_UNSUPPORTED_DIRECTIVE", format!("directive `%{inner}%` retained but not interpreted"), op);
        }
    }

    fn handle_sr(&mut self, rest: &str, op: &RawOp, template: bool) {
        let _ = template;
        if rest.is_empty() {
            // %SR% closes an open block.
            if self.in_sr {
                self.expand_sr();
                self.in_sr = false;
                self.sr_template.clear();
                self.sr = None;
            } else {
                self.err("GERBER_SR_STRRAY", "stray %SR% with no open step/repeat block", op);
            }
            return;
        }
        if self.in_sr {
            self.err("GERBER_SR_NESTED", "nested %SR% step/repeat blocks are not supported", op);
            return;
        }
        match parse_sr(rest) {
            Some(sr) => {
                self.in_sr = true;
                self.sr = Some(sr);
            }
            None => self.err("GERBER_BAD_SR", format!("unparseable step/repeat `%SR{rest}%`"), op),
        }
    }

    fn expand_sr(&mut self) {
        let Some(sr) = self.sr.take() else { return };
        let nx = sr.x_repeats.max(1);
        let ny = sr.y_repeats.max(1);
        let template = std::mem::take(&mut self.sr_template);
        // Replay template into temporary plot features, offset per cell.
        let mut tfeats: Vec<(FeatureShape, Polarity, RawOp)> = Vec::new();
        let saved_polarity = self.cur_polarity;
        let saved_ap = self.cur_aperture.clone();
        let saved_x = self.cur_x;
        let saved_y = self.cur_y;
        let saved_pen_x = self.pen_x;
        let saved_pen_y = self.pen_y;
        self.cur_polarity = Polarity::Dark;
        self.cur_aperture = None;
        self.cur_x = 0.0;
        self.cur_y = 0.0;
        self.pen_x = 0.0;
        self.pen_y = 0.0;
        for top in template {
            let text = top.text.trim();
            if text.starts_with('%') {
                self.dispatch_directive(text, &top, true);
            } else {
                if let Some(f) = self.plot_feature(text, &top) {
                    tfeats.push((f.0, f.1, top));
                }
            }
        }
        self.cur_polarity = saved_polarity;
        self.cur_aperture = saved_ap;
        self.cur_x = saved_x;
        self.cur_y = saved_y;
        self.pen_x = saved_pen_x;
        self.pen_y = saved_pen_y;
        let sr_index = self.doc.step_repeats.len();
        for (shape, pol, rawop) in tfeats {
            let base_order = self.doc.features.len();
            for j in 0..ny {
                for i in 0..nx {
                    let off_x = (i as f64) * sr.dx_raw;
                    let off_y = (j as f64) * sr.dy_raw;
                    let shape2 = offset_shape(&shape, off_x, off_y);
                    let order = self.doc.features.len();
                    self.doc.features.push(Feature {
                        order,
                        polarity: pol,
                        shape: shape2,
                        sr_instance: format!("sr:{sr_index}:{i}:{j}"),
                        sr_base_order: Some(base_order),
                        raw: rawop.text.clone(),
                        pos: Some(self.pos_for(rawop.byte_start, rawop.byte_end, rawop.line, rawop.col)),
                    });
                }
            }
        }
        self.doc.step_repeats.push(sr);
    }

    fn dispatch_plot(&mut self, text: &str, op: &RawOp, template: bool) {
        if let Some(feat) = self.plot_feature(text, op) {
            if template {
                // Only occurs for stray plot ops captured oddly; ignore.
                return;
            }
            let (shape, pol) = feat;
            let order = self.doc.features.len();
            self.doc.features.push(Feature {
                order,
                polarity: pol,
                shape,
                sr_instance: "base".to_string(),
                sr_base_order: None,
                raw: op.text.clone(),
                pos: Some(self.pos_for(op.byte_start, op.byte_end, op.line, op.col)),
            });
        }
    }

    /// Parse one plot command; returns None on handled non-geometry / errors.
    fn plot_feature(&mut self, text: &str, op: &RawOp) -> Option<(FeatureShape, Polarity)> {
        // G-code prefix
        let body = strip_gcode(text);
        if body.starts_with("G36") {
            self.in_region = true;
            self.region_points.clear();
            return None;
        }
        if body.starts_with("G37") {
            self.close_region(op);
            return None;
        }
        if body.starts_with("G02") || body.starts_with("G03") {
            self.warn("GERBER_ARC_UNSUPPORTED", "arcs (G02/G03) are retained as raw ops but not rasterised", op);
            // consume coordinates so the cursor still moves
            let (x, y, _) = parse_coords(body, self.doc.format, self.doc.unit);
            if let Some(x) = x { self.cur_x = x; }
            if let Some(y) = y { self.cur_y = y; }
            return None;
        }
        if let Some(rest) = body.strip_prefix('D') {
            // Aperture select D10..D99 without coordinates (legacy D-plot select).
            if rest.chars().all(|c| c.is_ascii_digit()) {
                let code = format!("D{rest}");
                if code != "D00" {
                    self.cur_aperture = Some(code);
                }
                return None;
            }
        }
        let (x, y, dcode) = parse_coords(body, self.doc.format, self.doc.unit);
        if let Some(x) = x { self.cur_x = x; }
        if let Some(y) = y { self.cur_y = y; }
        let d = dcode.as_deref().unwrap_or("");
        if self.in_region {
            if d == "D02" {
                self.region_points.clear();
            }
            if let (Some(x), Some(y)) = (x, y) {
                self.region_points.push((x, y));
            }
            return None;
        }
        match d {
            "D03" => {
                let ap = self.require_ap(op)?;
                let (x, y) = (self.cur_x, self.cur_y);
                self.pen_x = x;
                self.pen_y = y;
                Some((FeatureShape::Flash { aperture: ap, x_raw: x, y_raw: y }, self.cur_polarity))
            }
            "D01" => {
                let ap = self.require_ap(op)?;
                let (x1, y1, x2, y2) = (self.pen_x, self.pen_y, self.cur_x, self.cur_y);
                self.pen_x = x2;
                self.pen_y = y2;
                Some((
                    FeatureShape::Draw {
                        aperture: ap,
                        x1_raw: x1,
                        y1_raw: y1,
                        x2_raw: x2,
                        y2_raw: y2,
                    },
                    self.cur_polarity,
                ))
            }
            "D02" => {
                self.pen_x = self.cur_x;
                self.pen_y = self.cur_y;
                None
            }
            _ => {
                self.warn("GERBER_DCODE_UNKNOWN", format!("plot op `{}` without usable D-code", op.text.trim()), op);
                None
            }
        }
    }

    fn require_ap(&mut self, op: &RawOp) -> Option<String> {
        match &self.cur_aperture {
            Some(ap) => Some(ap.clone()),
            None => {
                self.err("GERBER_NO_APERTURE", "plot command issued without a selected aperture", op);
                None
            }
        }
    }

    fn close_region(&mut self, op: &RawOp) {
        self.in_region = false;
        let pts = std::mem::take(&mut self.region_points);
        match rect_from_points(&pts) {
            Some(r) => {
                let order = self.doc.features.len();
                self.doc.features.push(Feature {
                    order,
                    polarity: self.cur_polarity,
                    shape: FeatureShape::RegionRect { x_raw: r.0, y_raw: r.1, w_raw: r.2, h_raw: r.3 },
                    sr_instance: "base".to_string(),
                    sr_base_order: None,
                    raw: op.text.clone(),
                    pos: Some(self.pos_for(op.byte_start, op.byte_end, op.line, op.col)),
                });
            }
            None => self.err(
                "GERBER_REGION_NOT_RECT",
                "only axis-aligned rectilinear rectangular regions are supported by the geometry engine",
                op,
            ),
        }
    }
}

fn is_sr_boundary(text: &str) -> bool {
    let inner = text.trim_start_matches('%').trim_end_matches('%');
    inner == "SR" || inner.starts_with("SR")
}

/// Split a Gerber stream on `*` into raw operations with byte/line anchors.
fn split_ops(src: &str, _cursor: &mut usize) -> Vec<RawOp> {
    let mut ops = Vec::new();
    let mut start = 0usize;
    let mut line = 1usize;
    let mut col = 1usize;
    let mut seg_start_line = 1usize;
    let mut seg_start_col = 1usize;
    for (idx, ch) in src.char_indices() {
        if ch == '*' {
            ops.push(RawOp {
                text: src[start..idx].to_string(),
                line: seg_start_line,
                col: seg_start_col,
                byte_start: start,
                byte_end: idx,
            });
            start = idx + ch.len_utf8();
            seg_start_line = line;
            seg_start_col = col + 1;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    if start < src.len() {
        ops.push(RawOp {
            text: src[start..].to_string(),
            line: seg_start_line,
            col: seg_start_col,
            byte_start: start,
            byte_end: src.len(),
        });
    }
    ops
}

fn parse_fs(rest: &str) -> Option<(u8, u8)> {
    // Expect e.g. `34Y34*%...` but separators stripped: 34Y34
    let (a, b) = rest.split_once('Y')?;
    let a = a.trim();
    let b = b.trim();
    // a is int+dec digits counts concatenated, e.g. "34" => int 3, dec 4.
    if a.len() < 2 {
        return None;
    }
    let int_d = a.as_bytes()[a.len() - 2] - b'0';
    let dec_d = a.as_bytes()[a.len() - 1] - b'0';
    // b may be followed by extra chars; first two chars are counts.
    if b.len() < 2 || int_d > 9 || dec_d > 9 {
        return None;
    }
    Some((int_d, dec_d))
}

fn parse_aperture(rest: &str) -> Option<Aperture> {
    // e.g. `D10C,0.25` or `D11R,1.0X0.5`
    let (code, body) = rest.split_at(rest.find(|c: char| c.is_ascii_alphabetic())?);
    let code = code.to_string();
    let mut chars = body.chars();
    let shape_ch = chars.next()?;
    let params: String = chars.collect();
    let params = params.trim_start_matches(',').trim();
    match shape_ch {
        'C' => {
            let diameter = params.split_whitespace().next()?.parse::<f64>().ok()?;
            Some(Aperture {
                code,
                shape: ApertureShape::Circle,
                geom: ApertureGeom::Circle { diameter_raw: diameter },
                raw: format!("%AD{rest}%"),
            })
        }
        'R' | 'O' => {
            let (x, y) = params.split_once('X')?;
            let x = x.parse::<f64>().ok()?;
            let y = y
                .split(|c: char| c.is_whitespace() || c == '*')
                .next()?
                .parse::<f64>()
                .ok()?;
            Some(Aperture {
                code,
                shape: if shape_ch == 'R' { ApertureShape::Rect } else { ApertureShape::Obround },
                geom: ApertureGeom::Rect { x_size_raw: x, y_size_raw: y },
                raw: format!("%AD{rest}%"),
            })
        }
        _ => None,
    }
}

fn parse_sr(rest: &str) -> Option<StepRepeat> {
    // e.g. `X3Y2I5.0J3.0`
    let mut x: u32 = 1;
    let mut y: u32 = 1;
    let mut dx = 0.0;
    let mut dy = 0.0;
    for part in rest.split(|c: char| c.is_whitespace() || c == '*') {
        if part.is_empty() {
            continue;
        }
        let key = part.as_bytes()[0] as char;
        let val = &part[1..];
        match key {
            'X' => x = val.parse().ok()?,
            'Y' => y = val.parse().ok()?,
            'I' => dx = val.parse().ok()?,
            'J' => dy = val.parse().ok()?,
            _ => return None,
        }
    }
    Some(StepRepeat {
        x_repeats: x,
        y_repeats: y,
        dx_raw: dx,
        dy_raw: dy,
        template: Vec::new(),
    })
}

fn offset_shape(s: &FeatureShape, ox: f64, oy: f64) -> FeatureShape {
    match s {
        FeatureShape::Flash { aperture, x_raw, y_raw } => FeatureShape::Flash {
            aperture: aperture.clone(),
            x_raw: x_raw + ox,
            y_raw: y_raw + oy,
        },
        FeatureShape::Draw { aperture, x1_raw, y1_raw, x2_raw, y2_raw } => FeatureShape::Draw {
            aperture: aperture.clone(),
            x1_raw: x1_raw + ox,
            y1_raw: y1_raw + oy,
            x2_raw: x2_raw + ox,
            y2_raw: y2_raw + oy,
        },
        FeatureShape::RegionRect { x_raw, y_raw, w_raw, h_raw } => FeatureShape::RegionRect {
            x_raw: x_raw + ox,
            y_raw: y_raw + oy,
            w_raw: *w_raw,
            h_raw: *h_raw,
        },
    }
}

fn strip_gcode(text: &str) -> String {
    let mut s = text.trim();
    for code in ["G54", "G01", "G1", "G02", "G2", "G03", "G3", "G36", "G37", "G70", "G71", "G74", "G75", "G90", "G91"] {
        if let Some(rest) = s.strip_prefix(code) {
            s = rest.trim();
        }
    }
    s.to_string()
}

fn parse_coords(
    body: &str,
    format: Option<CoordFormat>,
    _unit: Option<LengthUnit>,
) -> (Option<f64>, Option<f64>, Option<String>) {
    let mut x: Option<f64> = None;
    let mut y: Option<f64> = None;
    let mut d: Option<String> = None;
    let mut i = 0;
    let bytes = body.as_bytes();
    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
            'X' | 'Y' | 'I' | 'J' | 'D' => {
                let key = c;
                let start = i + 1;
                let mut j = start;
                while j < bytes.len() {
                    let cj = bytes[j] as char;
                    if cj.is_ascii_alphabetic() {
                        break;
                    }
                    j += 1;
                }
                let raw = body[start..j].trim();
                match key {
                    'X' | 'Y' => {
                        if let (Some(fmt), Ok(n)) = (format, raw.parse::<i64>()) {
                            let v = (n as f64) / 10f64.powi(fmt.dec_digits as i32);
                            if key == 'X' { x = Some(v); } else { y = Some(v); }
                        }
                    }
                    'D' => d = Some(format!("D{raw}")),
                    _ => {}
                }
                i = j;
            }
            _ => i += 1,
        }
    }
    (x, y, d)
}

fn rect_from_points(pts: &[(f64, f64)]) -> Option<(f64, f64, f64, f64)> {
    if pts.len() < 4 {
        return None;
    }
    let x0 = pts.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let x1 = pts.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
    let y0 = pts.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let y1 = pts.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    // Every point must sit on the rectangle boundary (axis-aligned rectilinear).
    let on = |p: &(f64, f64)| {
        (p.0 == x0 || p.0 == x1) && (y0..=y1).contains(&p.1)
            || (p.1 == y0 || p.1 == y1) && (x0..=x1).contains(&p.0)
    };
    if pts.iter().all(on) {
        Some((x0, y0, x1 - x0, y1 - y0))
    } else {
        None
    }
}
