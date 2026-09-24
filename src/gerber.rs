//! Parser for the Gerber subset used by the offline case pack.
//!
//! The source representation (coordinate tokens, unit, polarity, aperture,
//! step/repeat, raw command) is fully retained. Normalized geometry is a
//! derived layer; unconvertible coordinates stay in place with diagnostics.

use crate::model::{coord_from_token, decimal_size_to_h, Coord, Diag, PosSpec, RawCoord, Shape, Unit};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Polarity {
    Dark,
    Clear,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hole {
    pub diameter: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "aperture", rename_all = "snake_case")]
pub enum Ap {
    Circle { d: i64, hole: Option<Hole> },
    Rect { w: i64, h: i64, hole: Option<Hole> },
    Obround { w: i64, h: i64, hole: Option<Hole> },
    /// Unsupported macro aperture retained verbatim; contributes no geometry
    /// until semantics are resolved.
    Macro { code: String, raw: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Aperture {
    pub dcode: String,
    pub def: Ap,
    pub raw: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum GOp {
    Unit { unit: Unit, raw: String },
    Format { spec: PosSpec, raw: String },
    ApertureDef(Aperture),
    SelectAperture { dcode: String, raw: String },
    Polarity { polarity: Polarity, raw: String },
    StepRepeat { x: i64, y: i64, i: u32, j: u32, raw: String },
    StepRepeatEnd { raw: String },
    /// Flash at (x,y). Either coord may be unconvertible.
    Flash { x: Coord, y: Coord, aperture: String, raw: String },
    /// Linear interpolation draw.
    Draw {
        from: Option<(Coord, Coord)>,
        to: (Coord, Coord),
        aperture: String,
        raw: String,
    },
    /// Move (D02) sets the current point without adding copper.
    Move { x: Coord, y: Coord, raw: String },
    /// Recognized command that is intentionally not part of the subset.
    Unsupported { raw: String, why: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GBlock {
    pub seq: u32,
    pub line: usize,
    pub raw: String,
    pub ops: Vec<GOp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GerberDoc {
    pub file_id: String,
    pub unit: Option<Unit>,
    pub spec: Option<PosSpec>,
    pub blocks: Vec<GBlock>,
    pub diags: Vec<Diag>,
}

fn err(line: usize, code: &str, msg: String, raw: &str) -> Diag {
    Diag::new(code, line, 1, msg, raw.to_string())
}


impl GerberDoc {
    pub fn parse(file_id: &str, text: &str) -> Result<GerberDoc, Vec<Diag>> {
        let mut doc = GerberDoc {
            file_id: file_id.to_string(),
            unit: None,
            spec: None,
            blocks: Vec::new(),
            diags: Vec::new(),
        };
        let mut corrupt: Vec<Diag> = Vec::new();
        let mut seq: u32 = 0;

        for (idx, raw_line) in text.lines().enumerate() {
            let line_no = idx + 1;
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }
            if !line.contains('*') {
                corrupt.push(err(line_no, "fmt.no_terminator", "command block missing '*' terminator".into(), line));
                continue;
            }
            for block_raw in line.split('*').filter(|s| !s.trim().is_empty()) {
                seq += 1;
                let mut block = GBlock { seq, line: line_no, raw: block_raw.trim().to_string(), ops: Vec::new() };
                let body = block.raw.trim_start();
                if let Some(cmd) = body.strip_prefix('%') {
                    Self::parse_extended(&mut doc, &mut block, &mut corrupt, cmd);
                } else {
                    Self::parse_coord_block(&mut doc, &mut block, &mut corrupt);
                }
                doc.blocks.push(block);
            }
        }

        if doc.unit.is_none() {
            doc.diags.push(err(1, "sem.missing_unit", "no %MO*% unit declaration found".into(), ""));
        }
        if doc.spec.is_none() {
            doc.diags.push(err(1, "sem.missing_format", "no %FSLAX...Y...*% coordinate format declared".into(), ""));
        }
        if !corrupt.is_empty() {
            return Err(corrupt);
        }
        Ok(doc)
    }

    fn parse_extended(doc: &mut Self, block: &mut GBlock, corrupt: &mut Vec<Diag>, body: &str) {
        let line = block.line;
        let raw = &block.raw;
        let stripped = body.strip_suffix('%').unwrap_or(body);
        if let Some(f) = stripped.strip_prefix("FSLAX") {
            match parse_fmt_inner(f) {
                Some(spec) => {
                    doc.spec = Some(spec);
                    block.ops.push(GOp::Format { spec, raw: raw.clone() });
                }
                None => corrupt.push(err(line, "fmt.fs", format!("unparseable coordinate format '{f}'"), raw)),
            }
        } else if let Some(u) = stripped.strip_prefix("MO") {
            match u {
                "MM" => { doc.unit = Some(Unit::Mm); block.ops.push(GOp::Unit { unit: Unit::Mm, raw: raw.clone() }); }
                "IN" => { doc.unit = Some(Unit::Inch); block.ops.push(GOp::Unit { unit: Unit::Inch, raw: raw.clone() }); }
                other => corrupt.push(err(line, "fmt.mo", format!("unknown unit '{other}'"), raw)),
            }
        } else if let Some(rest) = stripped.strip_prefix("AD") {
            Self::parse_ad(doc, block, rest);
        } else if let Some(rest) = stripped.strip_prefix("LP") {
            match rest {
                "D" => block.ops.push(GOp::Polarity { polarity: Polarity::Dark, raw: raw.clone() }),
                "C" => block.ops.push(GOp::Polarity { polarity: Polarity::Clear, raw: raw.clone() }),
                other => corrupt.push(err(line, "fmt.lp", format!("unknown polarity '{other}'"), raw)),
            }
        } else if let Some(rest) = stripped.strip_prefix("SR") {
            Self::parse_sr(block, rest, corrupt);
        } else if stripped.starts_with("AM") {
            doc.diags.push(err(line, "sem.aperture_macro", "aperture macros are not part of the supported subset".into(), raw));
            block.ops.push(GOp::Unsupported { raw: raw.clone(), why: "aperture macro".into() });
        } else if stripped == "TF" || stripped.starts_with("TF") || stripped.starts_with("TD") {
            doc.diags.push(err(line, "sem.attribute", "file/object attributes retained but not used".into(), raw));
        } else {
            corrupt.push(err(line, "fmt.unknown_extended", format!("unrecognized extended command '{stripped}'"), raw));
        }
    }

    fn parse_ad(doc: &mut Self, block: &mut GBlock, rest: &str) {
        let raw = block.raw.clone();
        let line = block.line;
        let (dcode, tail) = match rest.split_once(|c: char| c.is_whitespace()) {
            Some(v) => v,
            None => (rest, ""),
        };
        if !(dcode.starts_with('D') && dcode.len() > 1) {
            doc.diags.push(err(line, "sem.ad", format!("bad aperture id '{dcode}'"), &raw));
            return;
        }
        let unit = match doc.unit {
            Some(u) => u,
            None => {
                doc.diags.push(err(line, "sem.ad_before_unit", "aperture defined before unit declaration".into(), &raw));
                Unit::Mm
            }
        };
        // tail examples: "C0.3", "R1.0X0.5", "C0.6X0.3", "O2.0X1.0X0.4M"
        let fields: Vec<&str> = tail.split('X').filter(|s| !s.is_empty()).collect();
        if fields.is_empty() {
            doc.diags.push(err(line, "sem.ad_size", format!("aperture {dcode} has no size"), &raw));
            return;
        }
        let first = fields[0];
        let shape: String = first.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
        let size0_txt = &first[shape.len()..];
        let mut sizes: Vec<i64> = Vec::new();
        match decimal_size_to_h(size0_txt, unit) {
            Ok(v) => sizes.push(v),
            Err(e) => {
                doc.diags.push(err(line, "sem.ad_size", format!("aperture {dcode}: {e}"), &raw));
                return;
            }
        }
        // Remaining numeric fields: rect/obround second size, then optional hole.
        let mut numeric_tail: Vec<i64> = fields[1..]
            .iter()
            .map(|f| f.trim_end_matches('M'))
            .filter_map(|t| decimal_size_to_h(t, unit).ok())
            .collect();
        sizes.append(&mut numeric_tail);
        let hole = match shape.as_str() {
            "R" | "O" => sizes.get(2).copied(),
            _ => sizes.get(1).copied(),
        }
        .map(|diameter| Hole { diameter });
        let def = match shape.as_str() {
            "C" => Ap::Circle { d: sizes[0], hole },
            "R" if sizes.len() >= 2 => Ap::Rect { w: sizes[0], h: sizes[1], hole },
            "O" if sizes.len() >= 2 => Ap::Obround { w: sizes[0], h: sizes[1], hole },
            other => {
                doc.diags.push(err(line, "sem.ad_shape", format!("aperture shape '{other}' not supported"), &raw));
                Ap::Macro { code: dcode.to_string(), raw: raw.clone() }
            }
        };
        block.ops.push(GOp::ApertureDef(Aperture { dcode: dcode.to_string(), def, raw }));
    }

    fn parse_sr(block: &mut GBlock, rest: &str, corrupt: &mut Vec<Diag>) {
        let raw = block.raw.clone();
        if rest.is_empty() || rest == "%".to_string() {
            block.ops.push(GOp::StepRepeatEnd { raw });
            return;
        }
        let mut x = 0i64;
        let mut y = 0i64;
        let mut i = 0u32;
        let mut j = 0u32;
        for part in rest.split('X').filter(|s| !s.is_empty()) {
            let key = part.chars().next().unwrap_or(' ');
            let val: &str = &part[1..];
            match key {
                'X' => x = val.parse().unwrap_or(0),
                'Y' => y = val.parse().unwrap_or(0),
                'I' => i = val.parse().unwrap_or(0),
                'J' => j = val.parse().unwrap_or(0),
                _ => {
                    corrupt.push(err(block.line, "fmt.sr", format!("unknown step/repeat field '{part}'"), &raw));
                    return;
                }
            }
        }
        block.ops.push(GOp::StepRepeat { x, y, i: i.max(1), j: j.max(1), raw });
    }

    fn parse_coord_block(doc: &mut Self, block: &mut GBlock, corrupt: &mut Vec<Diag>) {
        let raw = block.raw.clone();
        let body = raw.trim();
        let mut chars = body.chars().peekable();
        let mut gcode: Option<u8> = None;
        let mut x: Option<String> = None;
        let mut y: Option<String> = None;
        let mut dcode: Option<String> = None;
        let mut neg_x = false;
        let mut neg_y = false;
        while let Some(c) = chars.next() {
            match c {
                'G' => {
                    let mut s = String::from('G');
                    while let Some(&d) = chars.peek() {
                        if d.is_ascii_digit() { s.push(d); chars.next(); } else { break; }
                    }
                    gcode = Some(s.trim_start_matches('G').parse().unwrap_or(0));
                }
                'X' => {
                    if matches!(chars.peek(), Some('-')) { neg_x = true; chars.next(); }
                    let mut s = String::new();
                    while let Some(&d) = chars.peek() {
                        if d.is_ascii_digit() { s.push(d); chars.next(); } else { break; }
                    }
                    x = Some(s);
                }
                'Y' => {
                    if matches!(chars.peek(), Some('-')) { neg_y = true; chars.next(); }
                    let mut s = String::new();
                    while let Some(&d) = chars.peek() {
                        if d.is_ascii_digit() { s.push(d); chars.next(); } else { break; }
                    }
                    y = Some(s);
                }
                'D' => {
                    let mut s = String::from('D');
                    while let Some(&d) = chars.peek() {
                        if d.is_ascii_digit() { s.push(d); chars.next(); } else { break; }
                    }
                    dcode = Some(s);
                }
                'M' => {
                    let mut s = String::from('M');
                    while let Some(&d) = chars.peek() {
                        if d.is_ascii_digit() { s.push(d); chars.next(); } else { break; }
                    }
                    if s == "M02" || s == "M00" { return; }
                }
                other => {
                    corrupt.push(err(block.line, "fmt.coord", format!("unexpected character '{other}' in coordinate block"), &raw));
                    return;
                }
            }
        }
        let g = gcode.unwrap_or(1);
        if g == 2 || g == 3 {
            doc.diags.push(err(block.line, "sem.arc", "G02/G03 circular interpolation is outside the supported subset".into(), &raw));
            block.ops.push(GOp::Unsupported { raw, why: format!("G{g} arc") });
            return;
        }
        if g == 36 || g == 37 {
            doc.diags.push(err(block.line, "sem.region", "G36/G37 region fill is outside the supported subset".into(), &raw));
            block.ops.push(GOp::Unsupported { raw, why: format!("G{g} region") });
            return;
        }
        if g != 1 {
            doc.diags.push(err(block.line, "sem.gcode", format!("unsupported G code G{g}"), &raw));
            block.ops.push(GOp::Unsupported { raw, why: format!("G{g}") });
            return;
        }
        let spec = match doc.spec {
            Some(s) => s,
            None => {
                corrupt.push(err(block.line, "fmt.no_format", "coordinate block before format declaration".into(), &raw));
                return;
            }
        };
        let unit = match doc.unit {
            Some(u) => u,
            None => {
                corrupt.push(err(block.line, "fmt.no_unit", "coordinate block before unit declaration".into(), &raw));
                return;
            }
        };
        let mk = |tok: Option<String>, neg: bool| -> Coord {
            match tok {
                Some(t) => {
                    let c = coord_from_token(&t, neg, spec, unit);
                    if c.is_bad() {
                        doc.diags.push(err(block.line, "sem.coord_unconvertible", "coordinate could not be normalized; kept in place".into(), &raw));
                    }
                    c
                }
                None => Coord::Unconvertible { raw: "<modal>".into(), why: "no value and no modal coordinate".into() },
            }
        };
        let xc = mk(x, neg_x);
        let yc = mk(y, neg_y);
        match dcode.as_deref() {
            Some("D02") => block.ops.push(GOp::Move { x: xc, y: yc, raw }),
            Some("D03") => block.ops.push(GOp::Flash { x: xc, y: yc, aperture: String::new(), raw }),
            Some("D01") | None => {
                if dcode.is_none() {
                    corrupt.push(err(block.line, "fmt.no_dcode", "coordinate block without D code".into(), &raw));
                    return;
                }
                block.ops.push(GOp::Draw { from: None, to: (xc, yc), aperture: String::new(), raw });
            }
            Some(other) => {
                corrupt.push(err(block.line, "fmt.dcode", format!("unknown D code '{other}'"), &raw));
            }
        }
    }
}

fn parse_fmt_inner(f: &str) -> Option<PosSpec> {
    let ypos = f.find('Y')?;
    let xpart = &f[..ypos];
    let ypart = &f[ypos + 1..];
    let parse_two = |s: &str| -> Option<(u8, u8)> {
        if s.len() < 2 { return None; }
        let (a, b) = s.split_at(s.len() - s.chars().last().map(|_| 1).unwrap_or(0));
        let _ = (a, b);
        let digits: Vec<char> = s.chars().collect();
        if digits.len() < 2 { return None; }
        Some((digits[0].to_digit(10)? as u8, digits[1].to_digit(10)? as u8))
    };
    let (xi, xf) = parse_two(xpart)?;
    let (yi, yf) = parse_two(ypart)?;
    if xi == yi && xf == yf {
        Some(PosSpec { digits_int: xi, digits_frac: xf })
    } else {
        None
    }
}
