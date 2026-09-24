//! Gerber RS-274X parser (focused deterministic subset).
//!
//! Preserved verbatim for provenance: unit directive, coordinate format directive,
//! aperture definitions, polarity changes, step/repeat blocks, and every raw command.
//! Numeric parsing uses the declared FS specification; unconvertible tokens stay in
//! place as diagnostics rather than becoming implicit defaults.

use crate::geometry::{DrawnShape, LayerDraw, Pt, Shape};
use crate::model::{CoordBasis, Diagnostic, Polarity, Quantity, Unit};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApertureDef {
    pub code: String,
    pub shape: String, // C / R / O / P
    pub params_raw: Vec<String>,
    pub raw: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub modifiers: Vec<Quantity>,
    pub d_mm: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepRepeatRec {
    pub nr: u32,
    pub ni: u32,
    pub dx_mm: f64,
    pub dy_mm: f64,
    pub raw: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawCommand {
    pub order: usize,
    pub line: usize,
    pub text: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GerberDoc {
    pub source_id: String,
    pub basis: CoordBasis,
    pub apertures: Vec<ApertureDef>,
    pub polarity_trace: Vec<(usize, Polarity)>,
    pub step_repeat: Option<StepRepeatRec>,
    pub raw_commands: Vec<RawCommand>,
    pub raw_text: String,
    pub declared_filename: Option<String>,
    pub diagnostics: Vec<Diagnostic>,
}

fn err(doc: &str, line: usize, code: &str, msg: impl Into<String>) -> Diagnostic {
    Diagnostic::err(code, doc, Some(line), msg)
}

/// Parse an FS directive body, e.g. `FSLAX34Y34`.
fn parse_fs(body: &str) -> Option<(String, u8, u8, u8, u8)> {
    let s = body.strip_prefix("FS")?;
    let mut chars = s.chars();
    let l = chars.next()?; // L
    let _a = chars.next()?; // A
    let xn = chars.next()?;
    if xn != 'X' {
        return None;
    }
    let xi = chars.next()?.to_digit(10)? as u8;
    let xd = chars.next()?.to_digit(10)? as u8;
    let yn = chars.next()?;
    if yn != 'Y' {
        return None;
    }
    let yi = chars.next()?.to_digit(10)? as u8;
    let yd = chars.next()?.to_digit(10)? as u8;
    Some((l.to_string(), xi, xd, yi, yd))
}

/// Decode one coordinate token (`X..Y..I..J..`) under the declared FS specification.
fn decode_coord(
    tok: &str,
    basis: &CoordBasis,
) -> Result<(Option<f64>, Option<f64>, Option<f64>, Option<f64>), String> {
    let mut x: Option<f64> = None;
    let mut y: Option<f64> = None;
    let mut i: Option<f64> = None;
    let mut j: Option<f64> = None;
    let key = |c: char| matches!(c, 'X' | 'Y' | 'I' | 'J');
    let mut idx = 0;
    let bytes: Vec<char> = tok.chars().collect();
    while idx < bytes.len() {
        if !key(bytes[idx]) {
            idx += 1;
            continue;
        }
        let name = bytes[idx];
        idx += 1;
        let start = idx;
        while idx < bytes.len() && !key(bytes[idx]) {
            idx += 1;
        }
        let digits: String = bytes[start..idx].iter().collect();
        let (int_d, dec_d) = match name {
            'X' => (basis.x_int, basis.x_dec),
            _ => (basis.y_int, basis.y_dec),
        };
        if digits.is_empty() {
            continue;
        }
        let neg = digits.starts_with('-');
        let core = digits.trim_start_matches('-');
        if core.chars().any(|c| !c.is_ascii_digit()) {
            return Err(format!("non-numeric coordinate digits `{digits}`"));
        }
        let padded: String = if (core.len() as u8) < int_d + dec_d {
            "0".repeat((int_d + dec_d - core.len() as u8) as usize) + core
        } else {
            core.to_string()
        };
        let value = if dec_d == 0 {
            padded.parse::<f64>().map_err(|_| "bad coordinate")?
        } else {
            let split = padded.len().saturating_sub(dec_d as usize);
            let (a, b) = padded.split_at(split);
            format!("{a}.{b}")
                .parse::<f64>()
                .map_err(|_| "bad coordinate")?
        };
        let value = if neg { -value } else { value };
        match name {
            'X' => x = Some(value),
            'Y' => y = Some(value),
            'I' => i = Some(value),
            'J' => j = Some(value),
            _ => {}
        }
    }
    Ok((x, y, i, j))
}

/// Convert a value already expressed in source units into mm, when possible.
fn to_mm(v: f64, unit: Unit) -> Option<f64> {
    unit.to_mm_factor().map(|f| v * f)
}

impl GerberDoc {
    pub fn parse(source_id: &str, text: &str) -> GerberDoc {
        let mut basis = CoordBasis::unknown();
        let mut apertures: Vec<ApertureDef> = Vec::new();
        let mut polarity_trace: Vec<(usize, Polarity)> = vec![(0, Polarity::Dark)];
        let mut raw_commands: Vec<RawCommand> = Vec::new();
        let mut step_repeat: Option<StepRepeatRec> = None;
        let mut declared_filename: Option<String> = None;
        let mut diagnostics: Vec<Diagnostic> = Vec::new();
        let mut order = 0usize;
        let mut in_sr = false;

        for (line_no, raw_line) in text.lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with("G04 ") || line == "G04" {
                raw_commands.push(RawCommand {
                    order,
                    line: line_no + 1,
                    text: line.into(),
                    kind: "comment".into(),
                });
                order += 1;
                continue;
            }
            if let Some(body) = line.strip_prefix('%') {
                let body = body.strip_suffix('%').unwrap_or(body);
                let kind = if body.starts_with("MO") {
                    "mode_unit"
                } else if body.starts_with("FS") {
                    "format"
                } else if body.starts_with("AD") {
                    "aperture_def"
                } else if body.starts_with("AM") {
                    "aperture_macro"
                } else if body.starts_with("LP") {
                    "polarity"
                } else if body.starts_with("SR") {
                    "step_repeat"
                } else if body.starts_with("TF") {
                    "file_attribute"
                } else if body.starts_with("TD") {
                    "aperture_attribute"
                } else {
                    "extended_unknown"
                };
                raw_commands.push(RawCommand {
                    order,
                    line: line_no + 1,
                    text: line.into(),
                    kind: kind.into(),
                });
                match kind {
                    "mode_unit" => {
                        let (u, raw) = match body {
                            "MOMM" => (Unit::Mm, "%MOMM*%".to_string()),
                            "MOIN" => (Unit::In, "%MOIN*%".to_string()),
                            other => {
                                diagnostics.push(
                                    err(source_id, line_no + 1, "G100", format!("unknown MO directive `{other}`"))
                                        .with_snippet(line, other),
                                );
                                (Unit::Unknown, line.to_string())
                            }
                        };
                        basis.unit = u;
                        basis.unit_raw = Some(raw);
                    }
                    "format" => {
                        if let Some((notation, xi, xd, yi, yd)) = parse_fs(body) {
                            basis.notation = notation;
                            basis.x_int = xi;
                            basis.x_dec = xd;
                            basis.y_int = yi;
                            basis.y_dec = yd;
                            basis.format_raw = Some(line.to_string());
                        } else {
                            diagnostics.push(
                                err(source_id, line_no + 1, "G101", "invalid FS coordinate format")
                                    .with_snippet(line, body),
                            );
                            basis.format_raw = Some(line.to_string());
                        }
                    }
                    "aperture_def" => {
                        // ADD10C,0.25  or  ADR12R,1.0X0.5
                        let body2 = body.strip_prefix("AD").unwrap_or(body);
                        let mut split = body2.splitn(2, ',');
                        let head = split.next().unwrap_or("");
                        let tail = split.next().unwrap_or("");
                        let digits: String = head.chars().take_while(|c| c.is_ascii_digit()).collect();
                        let shape: String = head.chars().skip_while(|c| c.is_ascii_digit()).collect();
                        let params_raw: Vec<String> =
                            tail.split('X').map(|s| s.trim().to_string()).collect();
                        let mut mods = Vec::new();
                        let mut d_mm = None;
                        for p in &params_raw {
                            if p.is_empty() {
                                continue;
                            }
                            let q = Quantity::new(p.clone(), basis.unit);
                            if !q.convertible {
                                diagnostics.push(
                                    err(
                                        source_id,
                                        line_no + 1,
                                        "G103",
                                        format!("aperture parameter `{p}` cannot be normalised"),
                                    )
                                    .with_snippet(line, p),
                                );
                            } else if shape == "C" && d_mm.is_none() {
                                d_mm = q.value_mm;
                            }
                            mods.push(q);
                        }
                        apertures.push(ApertureDef {
                            code: format!("D{digits}"),
                            shape,
                            params_raw,
                            raw: line.to_string(),
                            modifiers: mods,
                            d_mm,
                        });
                    }
                    "polarity" => {
                        let pol = match body {
                            "LPD" => Polarity::Dark,
                            "LPC" => Polarity::Clear,
                            other => {
                                diagnostics.push(
                                    err(source_id, line_no + 1, "G104", format!("unknown polarity `{other}`"))
                                        .with_snippet(line, other),
                                );
                                Polarity::Dark
                            }
                        };
                        polarity_trace.push((order, pol));
                    }
                    "step_repeat" => {
                        // SRX3Y2I2.0J1.5
                        let parsed = decode_coord(body, &CoordBasis {
                            unit: basis.unit,
                            unit_raw: basis.unit_raw.clone(),
                            format_raw: None,
                            notation: basis.notation.clone(),
                            x_int: 6,
                            x_dec: 4,
                            y_int: 6,
                            y_dec: 4,
                        });
                        match parsed {
                            Ok((x, y, i, j)) => {
                                let nr = x.unwrap_or(1.0) as u32;
                                let ni = y.unwrap_or(1.0) as u32;
                                let dx = to_mm(i.unwrap_or(0.0), basis.unit).unwrap_or(0.0);
                                let dy = to_mm(j.unwrap_or(0.0), basis.unit).unwrap_or(0.0);
                                step_repeat = Some(StepRepeatRec {
                                    nr,
                                    ni,
                                    dx_mm: dx,
                                    dy_mm: dy,
                                    raw: line.to_string(),
                                });
                                in_sr = !(nr <= 1 && ni <= 1);
                            }
                            Err(e) => diagnostics.push(
                                err(source_id, line_no + 1, "G105", format!("bad SR directive: {e}"))
                                    .with_snippet(line, body),
                            ),
                        }
                    }
                    "file_attribute" => {
                        if let Some(rest) = body.strip_prefix("TF.GenerationSoftware,") {
                            declared_filename = Some(rest.to_string());
                        }
                    }
                    "aperture_macro" => {
                        diagnostics.push(Diagnostic {
                            code: "G200".into(),
                            severity: crate::model::Severity::Info,
                            message: "aperture macros retained as raw input; macro expansion not supported"
                                .into(),
                            source_id: source_id.into(),
                            line: Some(line_no + 1),
                            column: None,
                            snippet: Some(line.into()),
                            raw_token: Some(line.into()),
                        });
                    }
                    _ => {}
                }
                order += 1;
                continue;
            }
            // standard command
            let kind = if line.starts_with('D') || line.contains("D0") {
                "draw"
            } else if line.starts_with("G") {
                "g_code"
            } else if line == "M02*" || line == "M00*" {
                "end"
            } else {
                "other"
            };
            raw_commands.push(RawCommand {
                order,
                line: line_no + 1,
                text: line.into(),
                kind: kind.into(),
            });
            order += 1;
        }
        let _ = in_sr;
        if basis.unit == Unit::Unknown {
            diagnostics.push(err(
                source_id,
                0,
                "G102",
                "no %MO..*% unit directive; coordinates retained in original representation only",
            ));
        }
        GerberDoc {
            source_id: source_id.into(),
            basis,
            apertures,
            polarity_trace,
            step_repeat,
            raw_commands,
            raw_text: text.into(),
            declared_filename,
            diagnostics,
        }
    }

    /// Replay raw draw commands in order, producing millimetre shapes. Polarity is
    /// attached per shape so compositing can apply it strictly in operation order.
    /// Step/repeat is expanded deterministically (row then column, index in the id).
    pub fn to_shapes(&self, diag: &mut Vec<Diagnostic>) -> LayerDraw {
        let mut shapes: Vec<DrawnShape> = Vec::new();
        let mut current_ap: Option<&ApertureDef> = None;
        let mut current_pol = Polarity::Dark;
        let mut pos = Pt::new(0.0, 0.0);
        let mut arc_mode: u8 = 1; // G01 linear default
        let mut op_index = 0usize;
        let mut sr_open = false;

        for rc in &self.raw_commands {
            let line = rc.text.trim_end_matches('*');
            // polarity lookup by command order
            for (at, pol) in &self.polarity_trace {
                if *at <= rc.order {
                    current_pol = *pol;
                }
            }
            if rc.kind == "step_repeat" {
                sr_open = true;
                continue;
            }
            if line == "SR" || rc.text == "%SR%*" {
                sr_open = false;
                continue;
            }
            if line.starts_with("G01") {
                arc_mode = 1;
            } else if line.starts_with("G02") {
                arc_mode = 2;
            } else if line.starts_with("G03") {
                arc_mode = 3;
            }
            if line.starts_with('D') && line.trim_end_matches('*').chars().all(|c| c.is_ascii_digit() || c == 'D') {
                let code = line.trim_end_matches('*');
                current_ap = self.apertures.iter().find(|a| a.code == code);
                if current_ap.is_none() {
                    diag.push(
                        err(&self.source_id, rc.line, "G110", format!("aperture {code} selected but never defined"))
                            .with_snippet(rc.text.clone(), code),
                    );
                }
                continue;
            }
            if !line.chars().any(|c| c == 'X' || c == 'Y') {
                continue;
            }
            let coord_part: String = line
                .chars()
                .take_while(|c| matches!(c, 'X' | 'Y' | 'I' | 'J' | '-' | '0'..='9'))
                .collect();
            if coord_part.is_empty() {
                continue;
            }
            let decoded = decode_coord(&coord_part, &self.basis);
            let (x, y, i, j) = match decoded {
                Ok(v) => v,
                Err(e) => {
                    diag.push(
                        err(&self.source_id, rc.line, "G111", format!("coordinate decode failed: {e}"))
                            .with_snippet(rc.text.clone(), coord_part.clone()),
                    );
                    continue;
                }
            };
            let term: String = line
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_digit() || *c == 'D')
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            // movement
            let nx = x.and_then(|v| to_mm(v, self.basis.unit)).unwrap_or(pos.x);
            let ny = y.and_then(|v| to_mm(v, self.basis.unit)).unwrap_or(pos.y);
            let target = Pt::new(nx, ny);
            let ap = match current_ap {
                Some(a) => a,
                None => {
                    diag.push(err(
                        &self.source_id,
                        rc.line,
                        "G112",
                        "draw without a selected aperture; operation retained but not rendered",
                    ));
                    pos = target;
                    continue;
                }
            };
            let width = ap.d_mm.unwrap_or_else(|| {
                diag.push(
                    err(&self.source_id, rc.line, "G113", format!("aperture {} has no normalised diameter", ap.code))
                        .with_snippet(rc.text.clone(), ap.code.clone()),
                );
                0.0
            });

            let mut base: Vec<Shape> = Vec::new();
            if term.ends_with("D03") {
                // flash
                match ap.shape.as_str() {
                    "C" => base.push(Shape::Circle(crate::geometry::Circle {
                        c: target,
                        d: width,
                    })),
                    "R" | "O" => {
                        let w = ap.modifiers.first().and_then(|q| q.value_mm).unwrap_or(0.0);
                        let h = ap
                            .modifiers
                            .get(1)
                            .and_then(|q| q.value_mm)
                            .unwrap_or(w);
                        base.push(Shape::Rect(crate::geometry::Rect {
                            min: Pt::new(target.x - w / 2.0, target.y - h / 2.0),
                            max: Pt::new(target.x + w / 2.0, target.y + h / 2.0),
                        }));
                    }
                    other => {
                        diag.push(err(
                            &self.source_id,
                            rc.line,
                            "G201",
                            format!("flash of unsupported aperture shape `{other}` retained, not rendered"),
                        ));
                    }
                }
            } else if term.ends_with("D01") || term.ends_with("D02") {
                if term.ends_with("D01") && width > 0.0 {
                    match arc_mode {
                        1 => base.push(Shape::Segment {
                            a: pos,
                            b: target,
                            width,
                        }),
                        2 | 3 => {
                            let io = i.and_then(|v| to_mm(v, self.basis.unit)).unwrap_or(0.0);
                            let jo = j.and_then(|v| to_mm(v, self.basis.unit)).unwrap_or(0.0);
                            let center = Pt::new(pos.x + io, pos.y + jo);
                            arc_segments(&center, &pos, &target, arc_mode == 2, &mut base, width);
                        }
                        _ => {}
                    }
                }
            }
            pos = target;

            if let Some(sr) = &self.step_repeat {
                if sr_open {
                    for row in 0..sr.ni {
                        for col in 0..sr.nr {
                            for shape in &base {
                                shapes.push(DrawnShape {
                                    shape: translate_shape(shape, col as f64 * sr.dx_mm, row as f64 * sr.dy_mm),
                                    aperture: ap.code.clone(),
                                    op_index,
                                    polarity: current_pol,
                                });
                            }
                        }
                    }
                    op_index += 1;
                    continue;
                }
            }
            for shape in base {
                shapes.push(DrawnShape {
                    shape,
                    aperture: ap.code.clone(),
                    op_index,
                    polarity: current_pol,
                });
            }
            op_index += 1;
        }
        LayerDraw {
            source_id: self.source_id.clone(),
            shapes,
        }
    }
}

fn translate_shape(s: &Shape, dx: f64, dy: f64) -> Shape {
    let t = |p: &Pt| Pt::new(p.x + dx, p.y + dy);
    match s {
        Shape::Circle(c) => Shape::Circle(crate::geometry::Circle {
            c: t(&c.c),
            d: c.d,
        }),
        Shape::Rect(r) => Shape::Rect(crate::geometry::Rect {
            min: t(&r.min),
            max: t(&r.max),
        }),
        Shape::Polygon { pts } => Shape::Polygon {
            pts: pts.iter().map(t).collect(),
        },
        Shape::Segment { a, b, width } => Shape::Segment {
            a: t(a),
            b: t(b),
            width: *width,
        },
    }
}

fn arc_segments(center: &Pt, a: &Pt, b: &Pt, clockwise: bool, out: &mut Vec<Shape>, width: f64) {
    let ang = |p: &Pt| (p.y - center.y).atan2(p.x - center.x);
    let mut a0 = ang(a);
    let a1 = ang(b);
    let mut delta = if clockwise {
        let mut d = a0 - a1;
        while d <= 0.0 {
            d += std::f64::consts::TAU;
        }
        -d
    } else {
        let mut d = a1 - a0;
        while d <= 0.0 {
            d += std::f64::consts::TAU;
        }
        d
    };
    let radius = ((a.x - center.x).powi(2) + (a.y - center.y).powi(2)).sqrt();
    let steps = ((delta.abs() / 0.12).ceil() as i64).max(4);
    let step = delta / steps as f64;
    for k in 0..steps {
        let p0 = Pt::new(
            center.x + radius * (a0 + step * k as f64).cos(),
            center.y + radius * (a0 + step * k as f64).sin(),
        );
        let p1 = Pt::new(
            center.x + radius * (a0 + step * (k + 1) as f64).cos(),
            center.y + radius * (a0 + step * (k + 1) as f64).sin(),
        );
        out.push(Shape::Segment {
            a: p0,
            b: p1,
            width,
        });
    }
    let _ = a1;
}
