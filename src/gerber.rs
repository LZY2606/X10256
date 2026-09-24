//! Deterministic subset RS-274X parser.
//!
//! Preserves, in order: coordinate format, units, polarity, apertures,
//! step/repeat (SR block subset), declared file function (layer role), and
//! every draw's raw text. Coordinates that cannot be converted stay in
//! `*_raw` with `*_mm = None` and a positioned diagnostic.

use crate::types::{
    Aperture, CoordFormat, CoordinateBasis, Diagnostic, DrawOp, GerberDraw, LayerRole, ParsedGerber,
    Polarity, StepRepeat,
};

#[derive(Clone)]
struct Ctx {
    unit: crate::types::LengthUnit,
    int_digits: u32,
    dec_digits: u32,
    zero_mode: String,
}

impl Default for Ctx {
    fn default() -> Self {
        Ctx {
            unit: crate::types::LengthUnit::Mm,
            int_digits: 3,
            dec_digits: 6,
            zero_mode: "L".to_string(),
        }
    }
}

pub fn parse_gerber(content: &str) -> (ParsedGerber, Vec<(usize, String)>) {
    let mut ctx = Ctx::default();
    let mut fs_spec = String::new();
    let mut file_function_raw: Option<String> = None;
    let mut image_polarity = "POSITIVE".to_string();
    let mut apertures: Vec<Aperture> = Vec::new();
    let mut step_repeats: Vec<StepRepeat> = Vec::new();
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut draws: Vec<GerberDraw> = Vec::new();

    let mut polarity = Polarity::Dark;
    let mut active_sr: Option<String> = None;
    let mut cur_d: Option<String> = None;
    // current coordinate (mm optionals + raws), last drawn point.
    let mut x_mm: Option<f64> = None;
    let mut y_mm: Option<f64> = None;
    let mut x_raw = String::new();
    let mut y_raw = String::new();
    let mut region_pts: Vec<(String, String)> = Vec::new();
    let mut region_mm: Vec<Option<(f64, f64)>> = Vec::new();
    let mut in_region = false;
    let mut seq = 0usize;

    let mut fatal: Vec<(usize, String)> = Vec::new();

    for (idx, raw_line) in content.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        // Format specification: %FSLAX36*%
        if line.starts_with("%FS") {
            fs_spec = line.trim_start_matches('%').trim_end_matches("*%").to_string();
            apply_fs(&fs_spec, &mut ctx, line_no, &mut diagnostics);
            continue;
        }
        if line.starts_with("%MO") {
            // %MOMIN*% / %MOMM*%
            let body = line.trim_start_matches("%MO").trim_end_matches("*%");
            match body {
                "MM" => ctx.unit = crate::types::LengthUnit::Mm,
                "IN" => ctx.unit = crate::types::LengthUnit::Inch,
                other => diagnostics.push(Diagnostic {
                    code: "G_UNKNOWN_UNIT".into(),
                    line_no,
                    col: 4,
                    raw_fragment: line.into(),
                    message: format!("unknown MO unit {}", other),
                }),
            }
            continue;
        }
        if line.starts_with("%LP") {
            let body = line.trim_start_matches("%LP").trim_end_matches("*%");
            image_polarity = body.to_string();
            continue;
        }
        if line.starts_with("%TF.FileFunction") || line.contains("FileFunction") {
            file_function_raw = Some(line.to_string());
            continue;
        }
        if line.starts_with("%AD") {
            if let Some(ap) = parse_aperture(line, line_no, &ctx) {
                apertures.push(ap);
            } else {
                diagnostics.push(Diagnostic {
                    code: "G_BAD_APERTURE".into(),
                    line_no,
                    col: 1,
                    raw_fragment: line.into(),
                    message: "cannot parse aperture definition".into(),
                });
            }
            continue;
        }
        if line.starts_with("%SR") {
            match parse_sr(line, line_no, &ctx) {
                Ok(sr) => {
                    active_sr = Some(sr.name.clone());
                    step_repeats.push(sr);
                }
                Err(msg) => diagnostics.push(Diagnostic {
                    code: "G_BAD_STEP_REPEAT".into(),
                    line_no,
                    col: 1,
                    raw_fragment: line.into(),
                    message: msg,
                }),
            }
            continue;
        }

        // Polarity: %LPD*% / %LPC*%
        if line.starts_with("%LP") == false && line.starts_with("%LP") {
            continue;
        }
        if line.len() >= 5 && line.starts_with("%LP") {
            continue;
        }
        if line.starts_with("%LPD") {
            polarity = Polarity::Dark;
            continue;
        }
        if line.starts_with("%LPC") {
            polarity = Polarity::Clear;
            continue;
        }

        // G codes
        if line.starts_with("G36") {
            in_region = true;
            region_pts.clear();
            region_mm.clear();
            continue;
        }
        if line.starts_with("G37") {
            in_region = false;
            if region_pts.len() >= 3 {
                seq += 1;
                draws.push(GerberDraw {
                    seq,
                    line_no,
                    raw: line.into(),
                    dcode: "REGION".into(),
                    polarity,
                    step_repeat: active_sr.clone(),
                    op: DrawOp::Region {
                        pts_raw: region_pts.clone(),
                        pts_mm: region_mm.clone(),
                    },
                    valid: region_mm.iter().all(|p| p.is_some()),
                    diagnostic: region_mm
                        .iter()
                        .find(|p| p.is_none())
                        .map(|_| Diagnostic {
                            code: "G_UNCONVERTIBLE_REGION_POINT".into(),
                            line_no,
                            col: 1,
                            raw_fragment: line.into(),
                            message: "a region point could not be converted".into(),
                        }),
                });
            }
            continue;
        }
        if line.starts_with("G01") || line.starts_with("G1") || line.starts_with("G02")
            || line.starts_with("G2") || line.starts_with("G03") || line.starts_with("G3")
        {
            // movement/interpolate; coordinate may follow on same line
            if line.contains('X') || line.contains('Y') {
                handle_coord(
                    line,
                    line_no,
                    &ctx,
                    &mut cur_d,
                    &mut x_mm,
                    &mut y_mm,
                    &mut x_raw,
                    &mut y_raw,
                    &mut polarity,
                    &active_sr,
                    &mut seq,
                    &mut draws,
                    in_region,
                    &mut region_pts,
                    &mut region_mm,
                    &mut apertures,
                );
            }
            continue;
        }

        if line == "M02*" || line == "M00*" {
            active_sr = None;
            continue;
        }

        // D-code select: D10*
        if let Some(rest) = line.strip_suffix('*') {
            if let Some(dsel) = rest.strip_prefix('D') {
                if dsel.chars().all(|c| c.is_ascii_digit()) {
                    let n: u32 = dsel.parse().unwrap_or(0);
                    if n >= 10 {
                        cur_d = Some(format!("D{}", n));
                        continue;
                    }
                }
            }
        }

        if line.contains('X') || line.contains('Y') {
            handle_coord(
                line,
                line_no,
                &ctx,
                &mut cur_d,
                &mut x_mm,
                &mut y_mm,
                &mut x_raw,
                &mut y_raw,
                &mut polarity,
                &active_sr,
                &mut seq,
                &mut draws,
                in_region,
                &mut region_pts,
                &mut region_mm,
                &mut apertures,
            );
            continue;
        }

        // Anything else inside an extended command block we cannot model.
        if line.starts_with('%') {
            diagnostics.push(Diagnostic {
                code: "G_UNMODELED_EXTENDED".into(),
                line_no,
                col: 1,
                raw_fragment: line.into(),
                message: "extended command retained but not modeled".into(),
            });
            continue;
        }

        fatal.push((line_no, line.to_string()));
    }

    let declared_role = file_function_raw.as_ref().and_then(role_from_file_function);

    let parsed = ParsedGerber {
        coord_format: CoordFormat {
            fs_spec,
            integer_digits: ctx.int_digits,
            decimal_digits: ctx.dec_digits,
            declared_unit: Some(ctx.unit),
            zero_mode: ctx.zero_mode.clone(),
        },
        basis: CoordinateBasis {
            x_origin_side: "L".into(),
            y_origin_side: "L".into(),
            offset_mm: ("0".into(), "0".into()),
            axes: "XY".into(),
        },
        declared_role,
        file_function_raw,
        apertures,
        step_repeats,
        draws,
        image_polarity,
        diagnostics,
    };
    (parsed, fatal)
}

fn apply_fs(spec: &str, ctx: &mut Ctx, line_no: usize, diag: &mut Vec<Diagnostic>) {
    // e.g. FSLAX36  -> zero=L, abs, axes X, 3 int, 6 dec
    let bytes: Vec<char> = spec.chars().collect();
    if bytes.len() >= 6 {
        ctx.zero_mode = bytes[2].to_string();
        // bytes[3] == 'A' absolute / 'I' incremental
        let tail: String = bytes[5..].iter().collect();
        let digits: Vec<char> = tail.chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.len() >= 2 {
            ctx.int_digits = digits[0].to_digit(10).unwrap_or(ctx.int_digits);
            ctx.dec_digits = digits[1].to_digit(10).unwrap_or(ctx.dec_digits);
        }
    } else {
        diag.push(Diagnostic {
            code: "G_BAD_FS".into(),
            line_no,
            col: 1,
            raw_fragment: spec.into(),
            message: "malformed FS spec".into(),
        });
    }
}

fn parse_aperture(line: &str, line_no: usize, ctx: &Ctx) -> Option<Aperture> {
    // %ADD10C,0.25*%  or %ADD11R,1.0X0.5*%
    let body = line.trim_start_matches("%AD").trim_end_matches("*%");
    let (dcode, rest) = if let Some(idx) = body.find(|c: char| c == 'C' || c == 'R' || c == 'O' || c == 'P')
    {
        (body[..idx].to_string(), &body[idx..])
    } else {
        return None;
    };
    let shape = rest.chars().next()?.to_string();
    let mods_part = rest.get(1..).unwrap_or("");
    let mods_part = mods_part.strip_prefix(',').unwrap_or(mods_part);
    let modifiers_raw: Vec<String> = mods_part
        .split(['X', 'x'])
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    let modifiers_mm = modifiers_raw
        .iter()
        .map(|raw| parse_length(raw, ctx).map(|v| ctx.unit.mm(v)))
        .collect();
    Some(Aperture {
        dcode: format!("D{}", dcode.trim_start_matches('D')),
        shape,
        modifiers_raw,
        modifiers_mm,
        source_unit: ctx.unit,
        definition_line: line_no,
    })
}

fn parse_sr(line: &str, line_no: usize, ctx: &Ctx) -> Result<StepRepeat, String> {
    // %SRNAME2X3I2.5Y3.0*%  (supported subset: NAME name Xcount Ycount I.. J..)
    let body = line.trim_start_matches("%SR").trim_end_matches("*%");
    let mut name = "SR".to_string();
    let mut x_count = 1u32;
    let mut y_count = 1u32;
    let mut x_step_raw = String::from("0");
    let mut y_step_raw = String::from("0");

    let mut chars = body.chars().peekable();
    let mut token = String::new();
    let mut is_name = true;
    while let Some(ch) = chars.next() {
        match ch {
            'X' | 'Y' | 'I' | 'J' => {
                let key = ch;
                let mut num = String::new();
                while let Some(&n) = chars.peek() {
                    if n == 'X' || n == 'Y' || n == 'I' || n == 'J' {
                        break;
                    }
                    num.push(n);
                    chars.next();
                }
                match key {
                    'X' => {
                        x_count = num.parse().map_err(|_| format!("bad X count {}", num))?
                    }
                    'Y' => {
                        y_count = num.parse().map_err(|_| format!("bad Y count {}", num))?
                    }
                    'I' => x_step_raw = num,
                    'J' => y_step_raw = num,
                    _ => {}
                }
                is_name = false;
                let _ = token;
            }
            _ => {
                if is_name {
                    name.push(ch);
                }
            }
        }
    }
    let x_step_mm = parse_length(&x_step_raw, ctx).map(|v| ctx.unit.mm(v));
    let y_step_mm = parse_length(&y_step_raw, ctx).map(|v| ctx.unit.mm(v));
    Ok(StepRepeat {
        raw: line.to_string(),
        line_no,
        name,
        x_count,
        y_count,
        x_step_raw,
        y_step_raw,
        x_step_mm,
        y_step_mm,
    })
}

fn role_from_file_function(line: &str) -> Option<LayerRole> {
    let upper = line.to_uppercase();
    if upper.contains("COPPER") {
        Some(LayerRole::Copper)
    } else if upper.contains("SOLDERMA") || upper.contains("SOLDERMASK") {
        Some(LayerRole::SolderMask)
    } else if upper.contains("LEGEND") || upper.contains("SILK") {
        Some(LayerRole::Silkscreen)
    } else if upper.contains("PASTE") {
        Some(LayerRole::Paste)
    } else if upper.contains("OUTLINE") || upper.contains("PROFILE") {
        Some(LayerRole::Outline)
    } else if upper.contains("MECHANICAL") {
        Some(LayerRole::Mechanical)
    } else {
        Some(LayerRole::Other)
    }
}

/// Parse a length expressed in `ctx.unit`, returning the value in that unit.
fn parse_length(raw: &str, ctx: &Ctx) -> Result<f64, ()> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(());
    }
    if raw.contains('.') {
        return raw.parse::<f64>().map_err(|_| ());
    }
    // Fixed-point implicit decimal using FS digits.
    let digits: Vec<char> = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return Err(());
    }
    let total_width = (ctx.int_digits + ctx.dec_digits) as usize;
    let mut s: String = digits.into_iter().collect();
    if s.len() < total_width {
        if ctx.zero_mode == "L" {
            // Leading zero suppression: pad on the left.
            while s.len() < total_width {
                s.insert(0, '0');
            }
        } else {
            // Trailing zero suppression: pad on the right.
            while s.len() < total_width {
                s.push('0');
            }
        }
    }
    let dec = ctx.dec_digits as usize;
    if s.len() <= dec {
        while s.len() < dec {
            s.insert(0, '0');
        }
        s.insert(0, '0');
    }
    let split = s.len() - dec;
    let value: f64 = format!("{}.{}", &s[..split], &s[split..]).parse().map_err(|_| ())?;
    Ok(value)
}

#[allow(clippy::too_many_arguments)]
fn handle_coord(
    line: &str,
    line_no: usize,
    ctx: &Ctx,
    cur_d: &mut Option<String>,
    x_mm: &mut Option<f64>,
    y_mm: &mut Option<f64>,
    x_raw: &mut String,
    y_raw: &mut String,
    polarity: &mut Polarity,
    active_sr: &Option<String>,
    seq: &mut usize,
    draws: &mut Vec<GerberDraw>,
    in_region: bool,
    region_pts: &mut Vec<(String, String)>,
    region_mm: &mut Vec<Option<(f64, f64)>>,
    apertures: &[Aperture],
) {
    let (nx_raw, ny_raw) = split_xy(line);
    let prev = (*x_mm, *y_mm, x_raw.clone(), y_raw.clone());

    if let Some(r) = nx_raw {
        *x_raw = r.clone();
        *x_mm = parse_length(&r, ctx).ok().map(|v| ctx.unit.mm(v));
    }
    if let Some(r) = ny_raw {
        *y_raw = r.clone();
        *y_mm = parse_length(&r, ctx).ok().map(|v| ctx.unit.mm(v));
    }

    let flash = line.contains("D03") || line.contains("D3*");
    let draw_stroke = line.contains("D01") || line.contains("D1*");

    if in_region {
        if x_mm.is_some() || y_mm.is_some() {
            region_pts.push((x_raw.clone(), y_raw.clone()));
            region_mm.push(match (*x_mm, *y_mm) {
                (Some(x), Some(y)) => Some((x, y)),
                _ => None,
            });
        }
        return;
    }

    if flash {
        *seq += 1;
        let valid = x_mm.is_some() && y_mm.is_some() && cur_d.is_some();
        draws.push(GerberDraw {
            seq: *seq,
            line_no,
            raw: line.into(),
            dcode: cur_d.clone().unwrap_or_else(|| "D?".into()),
            polarity: *polarity,
            step_repeat: active_sr.clone(),
            op: DrawOp::Flash {
                x_raw: x_raw.clone(),
                y_raw: y_raw.clone(),
                x_mm: *x_mm,
                y_mm: *y_mm,
            },
            valid,
            diagnostic: if valid {
                None
            } else {
                Some(Diagnostic {
                    code: "G_UNCONVERTIBLE_FLASH".into(),
                    line_no,
                    col: 1,
                    raw_fragment: line.into(),
                    message: "flash missing coordinate or aperture".into(),
                })
            },
        });
        return;
    }

    if draw_stroke {
        let (px, py, pxr, pyr) = prev;
        *seq += 1;
        let valid = x_mm.is_some()
            && y_mm.is_some()
            && px.is_some()
            && py.is_some()
            && cur_d.is_some();
        draws.push(GerberDraw {
            seq: *seq,
            line_no,
            raw: line.into(),
            dcode: cur_d.clone().unwrap_or_else(|| "D?".into()),
            polarity: *polarity,
            step_repeat: active_sr.clone(),
            op: DrawOp::Line {
                x1_raw: pxr,
                y1_raw: pyr,
                x2_raw: x_raw.clone(),
                y2_raw: y_raw.clone(),
                x1_mm: px,
                y1_mm: py,
                x2_mm: *x_mm,
                y2_mm: *y_mm,
            },
            valid,
            diagnostic: if valid {
                None
            } else {
                Some(Diagnostic {
                    code: "G_UNCONVERTIBLE_LINE".into(),
                    line_no,
                    col: 1,
                    raw_fragment: line.into(),
                    message: "line missing endpoint or aperture".into(),
                })
            },
        });
        return;
    }

    let _ = apertures;
}

fn split_xy(line: &str) -> (Option<String>, Option<String>) {
    let mut x: Option<String> = None;
    let mut y: Option<String> = None;
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == 'X' || chars[i] == 'Y' {
            let key = chars[i];
            i += 1;
            let start = i;
            while i < chars.len() {
                let c = chars[i];
                if c == 'X' || c == 'Y' || c == 'D' || c == '*' {
                    break;
                }
                i += 1;
            }
            let val: String = chars[start..i].iter().collect();
            if key == 'X' {
                x = Some(val);
            } else {
                y = Some(val);
            }
        } else {
            i += 1;
        }
    }
    (x, y)
}
