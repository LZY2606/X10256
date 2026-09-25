//! Stackup / layer-role description parser.
//!
//! Line oriented, `#` comments. Each non-comment line describes one layer:
//!   index, role, name, thickness(optional) = mapped-file(optional)
//! Example:
//!   1, copper, Top_Cu, 0.035mm = top.gbr
//!   2, dielectric, Core, 0.5mm
//!   3, copper, L2_Cu, 0.035mm = inner.gbr

use crate::model::*;

pub fn parse(name: &str, text: &str) -> Result<StackupDoc, Vec<Diagnostic>> {
    let mut errs = Vec::new();
    let mut layers = Vec::new();
    for (raw_line_no, raw_line) in text.lines().enumerate() {
        let line_no = raw_line_no + 1;
        let t = raw_line.trim();
        if t.is_empty() || t.starts_with('#') || t.starts_with(';') {
            continue;
        }
        if t.to_ascii_uppercase().starts_with("STACKUP") {
            continue;
        }
        let (left, mapped) = match t.split_once('=') {
            Some((l, r)) => (l.trim(), Some(r.trim().to_string())),
            None => (t, None),
        };
        let fields: Vec<&str> = left.split(',').map(|s| s.trim()).collect();
        if fields.len() < 3 {
            errs.push(Diagnostic {
                code: "S_FIELDS".into(),
                severity: "error".into(),
                file: name.into(),
                line: line_no,
                column: 0,
                message: "layer line needs index, role, name at minimum".into(),
                token: raw_line.into(),
            });
            continue;
        }
        let index = match fields[0].parse::<usize>() {
            Ok(v) => v,
            Err(_) => {
                errs.push(Diagnostic {
                    code: "S_INDEX".into(),
                    severity: "error".into(),
                    file: name.into(),
                    line: line_no,
                    column: 0,
                    message: format!("layer index '{}' not an integer", fields[0]),
                    token: fields[0].into(),
                });
                continue;
            }
        };
        let role = LayerRole::parse(fields[1]);
        if role == LayerRole::Unknown {
            errs.push(Diagnostic {
                code: "S_ROLE_UNKNOWN".into(),
                severity: "error".into(),
                file: name.into(),
                line: line_no,
                column: 0,
                message: format!("layer role '{}' is not one of copper/solder_mask/legend/drill/plate/dielectric", fields[1]),
                token: fields[1].into(),
            });
            continue;
        }
        let thickness = fields.get(3).map(|s| parse_dim(s));
        let thickness = match thickness {
            Some(Ok(d)) => Some(d),
            Some(Err(msg)) => {
                errs.push(Diagnostic {
                    code: "S_THICKNESS".into(),
                    severity: "error".into(),
                    file: name.into(),
                    line: line_no,
                    column: 0,
                    message: msg,
                    token: fields[3].into(),
                });
                None
            }
            None => None,
        };
        layers.push(StackLayer {
            line_no,
            raw: raw_line.to_string(),
            index,
            name: fields[2].to_string(),
            role,
            thickness,
            mapped_file: mapped,
        });
    }
    if !errs.is_empty() {
        return Err(errs);
    }
    Ok(StackupDoc { layers })
}

fn parse_dim(s: &str) -> Result<Dim, String> {
    let s = s.trim();
    if let Some(num) = s.strip_suffix("mm") {
        let v = num.trim().parse::<f64>().map_err(|_| format!("bad thickness '{}'", s))?;
        Ok(Dim::new(s, LengthUnit::Mm, Some(v)))
    } else if let Some(num) = s.strip_suffix("mil") {
        let v = num.trim().parse::<f64>().map_err(|_| format!("bad thickness '{}'", s))?;
        Ok(Dim::new(s, LengthUnit::Inch, Some(v * 0.0254)))
    } else if let Some(num) = s.strip_suffix("in") {
        let v = num.trim().parse::<f64>().map_err(|_| format!("bad thickness '{}'", s))?;
        Ok(Dim::new(s, LengthUnit::Inch, Some(v * 25.4)))
    } else {
        Err(format!("thickness '{}' has no unit; value left unconverted", s))
    }
}
