//! Gerber RS-274X subset parser.
//!
//! Preserves the original coordinate representation (`raw` strings), declared
//! unit/format, polarity, aperture definitions, step/repeat blocks and layer
//! role hints. Unconvertible values remain attached with a diagnostic rather
//! than being replaced by defaults.

use crate::domain::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Polar {
    Dark,
    Clear,
}

impl Polar {
    pub fn as_str(self) -> &'static str {
        match self {
            Polar::Dark => "dark",
            Polar::Clear => "clear",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GerberFormat {
    pub int_x: u32,
    pub frac_x: u32,
    pub int_y: u32,
    pub frac_y: u32,
    /// Coordinate notation: digits omitted at the end of a word are zero.
    pub absolute: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "shape")]
pub enum ApertureShape {
    Circle { diameter: RawNum },
    Rect { width: RawNum, height: RawNum },
    Obround { width: RawNum, height: RawNum },
    Unsupported { raw: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Aperture {
    pub code: u32,
    pub shape: ApertureShape,
    pub pos: SourcePos,
    /// Raw `%ADD...*%` block retained for the audit trail.
    pub raw: String,
}

/// A single paint operation in strict source order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaintOp {
    pub seq: u64,
    pub polar: Polar,
    pub aperture: u32,
    /// Flash, line segment (rendered as an obround/rect), or aperture outline.
    #[serde(rename = "kind")]
    pub op_kind: PaintKind,
    pub pos: SourcePos,
    pub raw: String,
    /// Step-and-repeat instance (0,0 is the base instance).
    pub sr: SrInstance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SrInstance {
    pub ix: u32,
    pub iy: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum PaintKind {
    Flash { at: RawPoint },
    Segment { from: RawPoint, to: RawPoint },
    Outline { points: Vec<RawPoint> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SrSpec {
    pub nx: u32,
    pub ny: u32,
    pub dx: RawNum,
    pub dy: RawNum,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GerberDoc {
    pub unit: Unit,
    pub format: Option<GerberFormat>,
    pub apertures: Vec<Aperture>,
    pub ops: Vec<PaintOp>,
    /// Active step/repeat specification while scanning, with raw declaration.
    pub sr_events: Vec<SrEvent>,
    pub diags: Vec<Diag>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SrEvent {
    pub pos: SourcePos,
    pub raw: String,
    pub spec: Option<SrSpec>,
    /// index of first / last op emitted inside this step-repeat region.
    pub first_op: Option<u64>,
    pub last_op: Option<u64>,
}

struct Record<'a> {
    text: &'a str,
    start: usize,
    line: u32,
    col: u32,
}

/// Split Gerber stream into `*`-terminated records, tracking source position.
///
/// Extended command blocks (`%...*%`) are returned as one record with the
/// percent signs stripped; plain `*`-terminated words are returned verbatim.
fn records(src: &str) -> Vec<Record<'_>> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    let mut line = 1u32;
    let mut col = 1u32;
    let mut start_off = 0usize;
    let mut start_line = 1u32;
    let mut start_col = 1u32;
    let mut emit = |lo: usize, hi: usize, ln: u32, co: u32| {
        let mut text = &src[lo..hi];
        if let Some(t) = text.strip_prefix('%') {
            text = t.strip_suffix('%').unwrap_or(t);
        }
        text = text.trim_matches('\r');
        out.push(Record { text, start: lo, line: ln, col: co });
    };
    while i < b.len() {
        match b[i] {
            b'%' => {
                start_off = i;
                start_line = line;
                start_col = col;
                i += 1;
                col += 1;
                while i < b.len() && b[i] != b'%' {
                    if b[i] == b'*' && i + 1 < b.len() && b[i + 1] == b'%' {
                        // ends immediately: include closing percent
                    }
                    if b[i] == b'\n' { line += 1; col = 1; } else if b[i] != b'\r' { col += 1; }
                    i += 1;
                }
                let hi = (i + 1).min(b.len());
                emit(start_off, hi, start_line, start_col);
                i = hi;
                if i < b.len() {
                    if b[i] == b'\n' { line += 1; col = 1; i += 1; }
                    else if b[i] == b'\r' { i += 1; }
                    else { col += 1; }
                }
                start_off = i;
                start_line = line;
                start_col = col;
            }
            b'*' => {
                emit(start_off, i, start_line, start_col);
                i += 1;
                col += 1;
                start_off = i;
                start_line = line;
                start_col = col;
            }
            b'\n' => { line += 1; col = 1; i += 1; }
            b'\r' => { i += 1; }
            _ => { col += 1; i += 1; }
        }
    }
    if start_off < b.len() && src[start_off..].trim().is_empty() == false {
        emit(start_off, b.len(), start_line, start_col);
    }
    out
}

struct Parser<'a> {
    src: &'a str,
    doc: GerberDoc,
    polar: Polar,
    cur_ap: Option<u32>,
    pos: Option<(i128, i128)>,
    sr: Option<(SrSpec, SourcePos, String)>,
    sr_first_op: Option<u64>,
    sr_last_op: Option<u64>,
    sr_index: u32,
    seq: u64,
    done: bool,
}

pub fn parse(src: &str) -> GerberDoc {
    let mut p = Parser {
        src,
        doc: GerberDoc::default(),
        polar: Polar::Dark,
        cur_ap: None,
        pos: None,
        sr: None,
        sr_first_op: None,
        sr_last_op: None,
        sr_index: 0,
        seq: 0,
        done: false,
    };
    for rec in records(src) {
        let text = rec.text.trim();
        if text.is_empty() {
            continue;
        }
        if p.done {
            p.doc.diags.push(
                Diag::warning("GBR_TRAILING_DATA", "records after M02 are ignored", Some(SourcePos::new(rec.start, rec.line, rec.col)))
                    .with_raw(text),
            );
            continue;
        }
        p.dispatch(text, SourcePos::new(rec.start, rec.line, rec.col));
    }
    if let Some((spec, spos, raw)) = p.sr.take() {
        p.doc.sr_events.push(SrEvent {
            pos: spos,
            raw,
            spec: Some(spec),
            first_op: p.sr_first_op,
            last_op: p.sr_last_op,
        });
    }
    p.doc
}

impl<'a> Parser<'a> {
    fn unit(&self) -> Unit {
        self.doc.unit
    }

    fn dispatch(&mut self, text: &str, pos: SourcePos) {
        if text == "M02" {
            self.done = true;
            return;
        }
        if let Some(rest) = text.strip_prefix("G04") {
            let _ = rest;
            return; // comment
        }
        if text.starts_with("FS") {
            self.handle_fs(text, pos);
            return;
        }
        if text == "MOIN" {
            self.doc.unit = Unit::In;
            return;
        }
        if text == "MOMM" {
            self.doc.unit = Unit::Mm;
            return;
        }
        if text.starts_with("AD") {
            self.handle_ad(text, pos);
            return;
        }
        if text.starts_with("AM") {
            self.doc
                .diags
                .push(Diag::warning("GBR_APERTURE_MACRO_KEPT", "aperture macro is retained but not expanded", Some(pos)).with_raw(text));
            return;
        }
        if text.starts_with("LP") {
            self.close_sr(pos);
            match text {
                "LPD" => self.polar = Polar::Dark,
                "LPC" => self.polar = Polar::Clear,
                _ => self.doc.diags.push(Diag::error("GBR_BAD_POLARITY", "invalid LP polarity", Some(pos)).with_raw(text)),
            }
            return;
        }
        if text.starts_with("SR") {
            self.handle_sr(text, pos);
            return;
        }
        if text.starts_with("D") && !text.contains('X') && !text.contains('Y') && !text.contains('I') && !text.contains('J') {
            self.handle_d_select(text, pos);
            return;
        }
        if text.starts_with('G') || text.contains('X') || text.contains('Y') || text.contains('I') || text.contains('J') {
            self.handle_coord(text, pos);
            return;
        }
        self.doc.diags.push(Diag::warning("GBR_UNKNOWN_RECORD", "unrecognized record retained", Some(pos)).with_raw(text));
    }

    fn handle_fs(&mut self, text: &str, pos: SourcePos) {
        // %FSLAX24Y24*%  or %FSLIX34Y34*%
        let mut absolute = true;
        let mut chars = text.chars();
        let mut fmt_str = String::new();
        while let Some(c) = chars.next() {
            match c {
                'L' => absolute = true,
                'I' => absolute = false,
                'A' => {}
                'F' | 'S' | 'T' | 'X' | 'Y' => {}
                '0'..='9' => fmt_str.push(c),
                _ => {
                    self.doc.diags.push(Diag::error("GBR_BAD_FORMAT", "invalid FS coordinate format", Some(pos)).with_raw(text));
                    return;
                }
            }
        }
        if fmt_str.len() != 4 {
            self.doc.diags.push(Diag::error("GBR_BAD_FORMAT", "FS must declare X and Y int/frac digits", Some(pos)).with_raw(text));
            return;
        }
        let d: Vec<u32> = fmt_str.chars().map(|c| c.to_digit(10).unwrap()).collect();
        if d[1] == 0 || d[3] == 0 {
            self.doc.diags.push(Diag::warning("GBR_ZERO_FRACTION", "zero fractional digits declared", Some(pos)).with_raw(text));
        }
        self.doc.format = Some(GerberFormat {
            int_x: d[0],
            frac_x: d[1],
            int_y: d[2],
            frac_y: d[3],
            absolute,
        });
    }

    fn parse_length(&self, s: &str, frac_hint: Option<u32>) -> RawNum {
        let unit = self.unit();
        if unit == Unit::Unspecified {
            return RawNum::new(s, unit, None);
        }
        if s.contains('.') {
            return RawNum::new(s, unit, decimal_to_nm(s, unit));
        }
        let frac = frac_hint.unwrap_or_else(|| self.doc.format.map(|f| f.frac_x).unwrap_or(0));
        match fixed_digits_to_int(s) {
            Some(n) => RawNum::new(s, unit, fixed_to_nm(n, frac, unit)),
            None => RawNum::new(s, unit, None),
        }
    }

    fn handle_ad(&mut self, text: &str, pos: SourcePos) {
        // ADD10C,0.3* / ADD11R,0.6X0.4* / ADD12O,...
        let body = &text[2..];
        let (code_s, rest) = match body.split_once(|c: char| c.is_alphabetic()) {
            Some(_) => split_code_body(body),
            None => {
                self.doc.diags.push(Diag::error("GBR_BAD_APERTURE", "malformed aperture definition", Some(pos)).with_raw(text));
                return;
            }
        };
        let code: u32 = match code_s.parse() {
            Ok(c) => c,
            Err(_) => {
                self.doc.diags.push(Diag::error("GBR_BAD_APERTURE", "aperture code is not numeric", Some(pos)).with_raw(text));
                return;
            }
        };
        let kind = rest.chars().next();
        let params = rest.get(kind.map(|c| c.len_utf8()).unwrap_or(0)..).unwrap_or("");
        let shape = match kind {
            Some('C') => {
                let dia = params.trim_start_matches(',').split(['X', 'x']).next().unwrap_or("").trim();
                ApertureShape::Circle { diameter: self.parse_length(dia, None) }
            }
            Some('R') | Some('O') => {
                let mut it = params.trim_start_matches(',').split(['X', 'x']);
                let w = it.next().unwrap_or("").trim();
                let h = it.next().unwrap_or("").trim();
                let width = self.parse_length(w, None);
                let height = self.parse_length(h, None);
                if kind == Some('R') {
                    ApertureShape::Rect { width, height }
                } else {
                    ApertureShape::Obround { width, height }
                }
            }
            Some('P') => ApertureShape::Unsupported { raw: rest.to_string() },
            _ => ApertureShape::Unsupported { raw: rest.to_string() },
        };
        self.doc.apertures.push(Aperture { code, shape, pos, raw: text.to_string() });
    }

    fn handle_d_select(&mut self, text: &str, pos: SourcePos) {
        let code_s = &text[1..];
        match code_s.parse::<u32>() {
            Ok(code) if code >= 10 => {
                self.close_sr(pos);
                self.cur_ap = Some(code);
            }
            Ok(_) => {
                self.doc.diags.push(Diag::error("GBR_RESERVED_DCODE", "D codes below 10 are handled via graphic records", Some(pos)).with_raw(text));
            }
            Err(_) => {
                self.doc.diags.push(Diag::error("GBR_BAD_DCODE", "invalid D code", Some(pos)).with_raw(text));
            }
        }
    }

    fn handle_sr(&mut self, text: &str, pos: SourcePos) {
        self.close_sr(pos);
        if text == "SR" {
            self.sr = None;
            return;
        }
        let body = &text[2..];
        let mut nx = 1u32;
        let mut ny = 1u32;
        let mut dx = RawNum::new("0", self.unit(), Some(0));
        let mut dy = RawNum::new("0", self.unit(), Some(0));
        for (key, val_s) in tokenize_sr_fields(body) {
            match key {
                'X' => nx = val_s.parse().unwrap_or(1),
                'Y' => ny = val_s.parse().unwrap_or(1),
                'I' => dx = self.parse_length(&val_s, None),
                'J' => dy = self.parse_length(&val_s, None),
                _ => {}
            }
        }
        let spec = SrSpec { nx, ny, dx, dy };
        self.sr = Some((spec, pos, text.to_string()));
        self.sr_first_op = None;
        self.sr_last_op = None;
        self.sr_index = 0;
    }

    fn close_sr(&mut self, _pos: SourcePos) {
        if let Some((spec, spos, raw)) = self.sr.take() {
            self.doc.sr_events.push(SrEvent {
                pos: spos,
                raw,
                spec: Some(spec),
                first_op: self.sr_first_op,
                last_op: self.sr_last_op,
            });
        }
    }

fn tokenize_sr_fields(body: &str) -> Vec<(char, String)> {
    let mut out = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let key = bytes[i] as char;
        if !matches!(key, 'X' | 'Y' | 'I' | 'J') {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < bytes.len() && !matches!(bytes[j], b'X' | b'Y' | b'I' | b'J') {
            j += 1;
        }
        out.push((key, body[i + 1..j].to_string()));
        i = j;
    }
    out
}

fn split_code_body(body: &str) -> (String, String) {
    let pos = body.char_indices().find(|(_, c)| c.is_alphabetic()).map(|(i, _)| i).unwrap_or(body.len());
    (body[..pos].to_string(), body[pos..].to_string())
}

fn fixed_digits_to_int(s: &str) -> Option<i128> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    s.parse::<i128>().ok().or_else(|| {
        // allow leading sign embedded in nothing else
        None
    })
}

fn parse_coord_word(word: &str, frac: u32, unit: Unit, pos: SourcePos, diags: &mut Vec<Diag>) -> RawNum {
    let nano = if unit == Unit::Unspecified {
        None
    } else if word.contains('.') {
        decimal_to_nm(word, unit)
    } else {
        word.parse::<i128>().ok().and_then(|n| fixed_to_nm(n, frac, unit))
    };
    if nano.is_none() {
        diags.push(
            Diag::error("GBR_UNCONVERTIBLE_COORD", "coordinate cannot be converted to the common basis; retained raw and excluded from drawing", Some(pos))
                .with_raw(word),
        );
    }
    RawNum::new(word, unit, nano)
}


impl<'a> Parser<'a> {
    fn parse_coords(&mut self, text: &str, pos: SourcePos) -> (Option<RawNum>, Option<RawNum>, u32) {
        let fmt = self.doc.format;
        let (fx, fy) = match fmt {
            Some(f) => (f.frac_x, f.frac_y),
            None => {
                self.doc.diags.push(Diag::error("GBR_NO_FORMAT", "coordinate record without FS format; retained raw", Some(pos)).with_raw(text));
                (0, 0)
            }
        };
        let bytes = text.as_bytes();
        let keys = [b'X', b'Y'];
        let mut vals: [Option<RawNum>; 2] = [None, None];
        let mut dcode: Option<u32> = None;
        let mut i = 0usize;
        while i < bytes.len() {
            if let Some(k) = keys.iter().position(|k| *k == bytes[i]) {
                let mut j = i + 1;
                while j < bytes.len() && !matches!(bytes[j], b'X' | b'Y' | b'D') {
                    j += 1;
                }
                let word = &text[i + 1..j];
                let frac = if k == 0 { fx } else { fy };
                vals[k] = Some(parse_coord_word(word, frac, self.unit(), pos, &mut self.doc.diags));
                i = j;
            } else if bytes[i] == b'D' {
                let mut j = i + 1;
                while j < bytes.len() && bytes[j].is_ascii_digit() {
                    j += 1;
                }
                dcode = text[i + 1..j].parse().ok();
                i = j;
            } else {
                i += 1;
            }
        }
        (vals[0].clone(), vals[1].clone(), dcode.unwrap_or(1))
    }

    fn handle_coord(&mut self, text: &str, pos: SourcePos) {
        // G36 region outlines are not supported geometrically; retained raw.
        if text.contains('G36') || text.contains('G37') {
            self.doc.diags.push(Diag::warning("GBR_REGION_KEPT", "G36/G37 region retained but not rasterized", Some(pos)).with_raw(text));
            return;
        }
        if text.contains('I') || text.contains('J') {
            self.doc.diags.push(Diag::warning("GBR_ARC_KEPT", "arc interpolation retained but flattened as a straight segment", Some(pos)).with_raw(text));
        }
        let (xn, yn, dcode) = self.parse_coords(text, pos);
        let rp = match (xn, yn) {
            (Some(x), Some(y)) => Some(RawPoint { x, y }),
            _ => {
                if dcode == 3 {
                    self.doc.diags.push(Diag::error("GBR_FLASH_WITHOUT_XY", "D03 flash missing coordinates", Some(pos)).with_raw(text));
                }
                None
            }
        };
        let Some(rp) = rp else {
            // retain modal position; unconvertible value stays recorded above
            return;
        };
        let nano = |r: &RawNum| r.nano;
        let now = (nano(&rp.x), nano(&rp.y));
        let valid = now.0.zip(now.1);
        let fmt = self.doc.format;
        let absolute = fmt.map(|f| f.absolute).unwrap_or(true);
        match dcode {
            3 => {
                let at = if absolute || self.pos.is_none() {
                    rp
                } else {
                    self.add_incremental(rp)
                };
                self.emit_flash(at, pos, text);
                if let Some(p) = valid {
                    self.pos = Some(p);
                }
            }
            2 | 1 => {
                let to = if absolute || self.pos.is_none() {
                    rp
                } else {
                    self.add_incremental(rp)
                };
                if self.pos.is_none() {
                    self.doc
                        .diags
                        .push(Diag::warning("GBR_SEGMENT_NO_ORIGIN", "D01/D02 without prior position; treated as move only", Some(pos)).with_raw(text));
                } else if dcode == 1 {
                    let from = self.raw_from_modal();
                    self.emit_segment(from, to.clone(), pos, text);
                }
                if let Some(p) = valid {
                    let origin = self.pos.unwrap_or((0, 0));
                    let target = if absolute { p } else { (origin.0 + p.0, origin.1 + p.1) };
                    self.pos = Some(target);
                }
            }
            other => {
                self.doc.diags.push(Diag::error("GBR_BAD_DCODE", format!("unsupported drawing code D{other:02}"), Some(pos)).with_raw(text));
            }
        }
    }

    fn raw_from_modal(&self) -> RawPoint {
        let unit = self.unit();
        RawPoint {
            x: RawNum::new(self.pos.map(|(x, _)| x.to_string()).unwrap_or_default(), unit, self.pos.map(|(x, _)| x)),
            y: RawNum::new(self.pos.map(|(_, y)| y.to_string()).unwrap_or_default(), unit, self.pos.map(|(_, y)| y)),
        }
    }

    fn add_incremental(&self, rp: RawPoint) -> RawPoint {
        let (bx, by) = self.pos.unwrap_or((0, 0));
        let add = |base: i64, r: RawNum| {
            let raw = format!("{}+{}", base, r.raw);
            RawNum::new(raw, r.unit, r.nano.map(|n| base + n))
        };
        RawPoint { x: add(bx, rp.x), y: add(by, rp.y) }
    }

    fn sr_offsets(&self) -> Vec<(SrInstance, i64, i64)> {
        match &self.sr {
            Some((spec, _, _)) => {
                let mut out = Vec::new();
                for iy in 0..spec.ny {
                    for ix in 0..spec.nx {
                        let ox = spec.dx.nano.unwrap_or(0) * ix as i64;
                        let oy = spec.dy.nano.unwrap_or(0) * iy as i64;
                        out.push((SrInstance { ix, iy }, ox, oy));
                    }
                }
                out
            }
            None => vec![(SrInstance::default(), 0, 0)],
        }
    }

    fn shift_point(&self, rp: &RawPoint, ox: i64, oy: i64) -> RawPoint {
        let shift = |r: &RawNum, off: i64| RawNum::new(r.raw.clone(), r.unit, r.nano.map(|n| n + off));
        RawPoint { x: shift(&rp.x, ox), y: shift(&rp.y, oy) }
    }

    fn emit_flash(&mut self, at: RawPoint, pos: SourcePos, raw: &str) {
        let Some(ap) = self.cur_ap else {
            self.doc.diags.push(Diag::error("GBR_NO_APERTURE", "flash without selected aperture", Some(pos)).with_raw(raw));
            return;
        };
        let polar = self.polar;
        for (sr, ox, oy) in self.sr_offsets() {
            self.seq += 1;
            self.doc.ops.push(PaintOp {
                seq: self.seq,
                polar,
                aperture: ap,
                op_kind: PaintKind::Flash { at: self.shift_point(&at, ox, oy) },
                pos,
                raw: raw.to_string(),
                sr,
            });
            self.note_sr_op();
        }
    }

    fn emit_segment(&mut self, from: RawPoint, to: RawPoint, pos: SourcePos, raw: &str) {
        let Some(ap) = self.cur_ap else {
            self.doc.diags.push(Diag::error("GBR_NO_APERTURE", "segment without selected aperture", Some(pos)).with_raw(raw));
            return;
        };
        let polar = self.polar;
        for (sr, ox, oy) in self.sr_offsets() {
            self.seq += 1;
            self.doc.ops.push(PaintOp {
                seq: self.seq,
                polar,
                aperture: ap,
                op_kind: PaintKind::Segment {
                    from: self.shift_point(&from, ox, oy),
                    to: self.shift_point(&to, ox, oy),
                },
                pos,
                raw: raw.to_string(),
                sr,
            });
            self.note_sr_op();
        }
    }

    fn note_sr_op(&mut self) {
        note_sr(&mut self.sr_first_op, &mut self.sr_last_op, self.seq);
    }
}

fn note_sr(first: &mut Option<u64>, last: &mut Option<u64>, seq: u64) {
    if first.is_none() {
        *first = Some(seq);
    }
    *last = Some(seq);
}
