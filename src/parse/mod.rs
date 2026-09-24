pub mod gerber;
pub mod excellon;
pub mod stackup;

use crate::model::{CoordPoint, Diagnostic, LengthUnit};

/// Parse a coordinate body given the FS format, e.g. `X010200Y008000`.
/// Returns raw substrings and normalised mm values. On any malformed digit
/// run the mm value stays `None` and a positional diagnostic is returned.
pub fn decode_coord(
    body: &str,
    int_digits: usize,
    dec_digits: usize,
    zero: char, // 'L' leading, 'T' trailing, 'N' none
    unit: LengthUnit,
    line: u32,
) -> (CoordPoint, Vec<Diagnostic>) {
    let mut diags = Vec::new();
    let mut point = CoordPoint { x_raw: None, y_raw: None, x_mm: None, y_mm: None, line };
    let mut i = 0;
    let bytes = body.as_bytes();
    while i < bytes.len() {
        let axis = bytes[i] as char;
        if axis != 'X' && axis != 'Y' && axis != 'I' && axis != 'J' {
            i += 1;
            continue;
        }
        let start = i + 1;
        let mut j = start;
        while j < bytes.len() {
            let c = bytes[j] as char;
            if c == 'X' || c == 'Y' || c == 'I' || c == 'J' || c == 'D' || c == '*' {
                break;
            }
            j += 1;
        }
        let raw = &body[start..j];
        if axis == 'X' || axis == 'Y' {
            let val = decode_number(raw, int_digits, dec_digits, zero);
            match val {
                Some(v) => {
                    let mm = v * unit.per_unit_mm();
                    if axis == 'X' {
                        point.x_raw = Some(raw.to_string());
                        point.x_mm = Some(mm);
                    } else {
                        point.y_raw = Some(raw.to_string());
                        point.y_mm = Some(mm);
                    }
                }
                None => {
                    diags.push(Diagnostic::error(
                        "GERBER_BAD_COORD",
                        Some(line),
                        Some(raw.to_string()),
                        format!("coordinate for {axis} is not a valid digit run"),
                    ));
                    if axis == 'X' {
                        point.x_raw = Some(raw.to_string());
                    } else {
                        point.y_raw = Some(raw.to_string());
                    }
                }
            }
        }
        i = j;
    }
    (point, diags)
}

fn decode_number(raw: &str, int_digits: usize, dec_digits: usize, zero: char) -> Option<f64> {
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit() || b == b'-' || b == b'+') {
        return None;
    }
    let neg = raw.starts_with('-');
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let total = int_digits + dec_digits;
    let padded = match zero {
        'L' => {
            // leading zeros suppressed: pad on the left
            if digits.len() > total {
                return None;
            }
            format!("{:0>width$}", digits, width = total)
        }
        'T' => {
            if digits.len() > total {
                return None;
            }
            format!("{:0<width$}", digits, width = total)
        }
        _ => {
            if digits.len() != total {
                return None;
            }
            digits
        }
    };
    let (int_part, dec_part) = padded.split_at(int_digits);
    let int_part: i64 = int_part.parse().ok()?;
    let dec_part: i64 = dec_part.parse().ok()?;
    let scale = 10f64.powi(dec_digits as i32);
    let mut value = int_part as f64 + dec_part as f64 / scale;
    if neg {
        value = -value;
    }
    Some(value)
}
