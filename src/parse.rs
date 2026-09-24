//! Parsers for the small, real subset of RS-274X Gerber, Excellon drill and
//! the stackup manifest used by the lab. Parsing is lossless: every converted
//! value keeps its original token, and values that cannot be converted remain
//! in place with an `UNCONVERTIBLE` diagnostic instead of being replaced by a
//! default.

use crate::domain::*;

fn loc(material_id: &str, line: u32, raw: &str) -> Location {
    Location {
        material_id: Some(material_id.to_string()),
        line: Some(line),
        column: None,
        command: Some(raw.to_string()),
    }
}

fn factor_pm(value: &str, int_digits: u32, dec_digits: u32, raw_unit: Unit) -> Option<Pm> {
    let v = value.trim();
    let neg = v.starts_with('-');
    let digits: String = v.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    // A decimal point overrides format-based digit placement.
    if let Some(pos) = v.find('.') {
        let whole: String = v[..pos].chars().filter(|c| c.is_ascii_digit()).collect();
        let mut frac2: String = v[pos + 1..].chars().filter(|c| c.is_ascii_digit()).collect();
        while frac2.len() < 12 {
            frac2.push('0');
        }
        frac2.truncate(12);
        let f2 = frac2.parse::<i128>().unwrap_or(0);
        let mut pm = whole.parse::<i128>().unwrap_or(0)
            .checked_mul(1_000_000_000_000)?
            .checked_add(f2)?
            .checked_div(1_000_000)?;
        let scale = if raw_unit == Unit::In { PM_PER_IN as i128 } else { PM_PER_MM as i128 };
        pm = pm.checked_mul(scale)?;
        if neg { pm = -pm; }
        return i64::try_from(pm).ok();
    }
    let total = int_digits + dec_digits;
    let padded = if digits.len() < total as usize {
        // RS-274X: LZ = leading zeros omitted -> right aligned;
        // TZ = trailing zeros omitted -> left aligned (pad at the end).
        let mut s = String::new();
        for _ in 0..(total as usize - digits.len()) { s.push('0'); }
        s
    } else {
        String::new()
    };
    // The padding side is determined by the zero omission mode supplied by the
    // caller; `factor_pm` default path assumes leading-zero omission (common
    // case). `factor_pm_mode` handles both modes.
    let mut full = String::new();
    if digits.len() < total as usize {
        // leading-zero omission fallback (kept for aperture-style callers)
        full.push_str(&padded);
        full.push_str(&digits);
    } else {
        full = digits.chars().take(total as usize).collect();
    }
    return factor_pm_decimal_fallback(&full, dec_digits, raw_unit, neg);
}

fn factor_pm_decimal_fallback(digits: &str, dec_digits: u32, raw_unit: Unit, neg: bool) -> Option<Pm> {
    let split = digits.len() as i64 - dec_digits as i64;
    if split <= 0 {
        return None;
    }
    let (whole, frac) = digits.split_at(split as usize);
    let w = whole.parse::<i128>().ok()?;
    let f = frac.parse::<i128>().unwrap_or(0);
    let scale_frac = 10i128.pow(dec_digits);
    // value (in raw unit) = w + f/scale_frac. Multiply by unit->pm first then
    // divide to keep exactness where possible.
    let unit_pm = if raw_unit == Unit::In { PM_PER_IN as i128 } else { PM_PER_MM as i128 };
    let mut pm = w.checked_mul(unit_pm)?
        .checked_add(f.checked_mul(unit_pm)?.checked_div(scale_frac)?)?;
    if neg { pm = -pm; }
    i64::try_from(pm).ok()
}

fn aperture_dims_pm(tail: &str, unit: Unit) -> Option<(Pm, Pm)> {
    // Inputs like "C,0.3" / "R,0.5X0.25" / "O,1.0X0.4", possibly with more
    // fields separated by 'X'.
    let mut it = tail.splitn(3, ',');
    let shape = it.next()?;
    let rest = it.next()?;
    let mut nums = rest.split('X');
    let a = decimal_to_pm(nums.next()?, unit)?;
    let b = nums.next().and_then(|s| decimal_to_pm(s, unit)).unwrap_or(a);
    let _ = shape;
    Some((a, b))
}

fn decimal_to_pm(s: &str, unit: Unit) -> Option<Pm> {
    let s = s.trim();
    let neg = s.starts_with('-');
    let (whole, frac) = match s.find('.') {
        Some(pos) => (&s[..pos], &s[pos + 1..]),
        None => (s, ""),
    };
    let w: i128 = whole.trim_start_matches('-').parse().ok()?;
    let mut frac = frac.chars().filter(|c| c.is_ascii_digit()).collect::<String>();
    while frac.len() < 12 { frac.push('0'); }
    frac.truncate(12);
    let f: i128 = frac.parse().ok()?;
    let unit_pm = if unit == Unit::In { PM_PER_IN as i128 } else { PM_PER_MM as i128 };
    let mut pm = w.checked_mul(unit_pm)?
        .checked_add(f.checked_div(1_000_000)?)?;
    if neg { pm = -pm; }
    i64::try_from(pm).ok()
}

/// Resolve a coordinate token according to the FS format.
/// `leading_omitted` true => LZ (pad zeros on the left), false => TZ.
fn factor_coord(v: &str, int_digits: u32, dec_digits: u32, leading_omitted: bool, unit: Unit) -> Option<Pm> {
    let v = v.trim();
    if v.contains('.') {
        return factor_pm(v, int_digits, dec_digits, unit);
    }
    let neg = v.starts_with('-');
    let digits: String = v.chars().filter(|c| c.is_ascii_digit()).collect();
    let total = (int_digits + dec_digits) as usize;
    let mut full = digits.clone();
    if full.len() < total {
        let pad: String = std::iter::repeat('0').take(total - full.len()).collect();
        full = if leading_omitted {
            format!("{pad}{full}")
        } else {
            format!("{full}{pad}")
        };
    } else if full.len() > total {
        full = full[full.len() - total..].to_string();
    }
    factor_pm_decimal_fallback(&full, dec_digits, unit, neg)
}

fn split_axis(word: &str) -> Option<(char, &str)> {
    let mut chars = word.char_indices();
    let (_, axis) = chars.next()?;
    let start = if word.starts_with('-') || word.starts_with('+') { 1 } else { 0 };
    Some((axis, &word[start..]))
}

pub struct ParseResult {
    pub doc: ParsedDoc,
    pub outcome: ParseOutcome,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn parse_material(id: &str, filename: &str, kind: MaterialKind, content: &str) -> ParseResult {
    match kind {
        MaterialKind::Gerber => {
            let (doc, outcome, diagnostics) = parse_gerber(id, content);
            ParseResult { doc: ParsedDoc::Gerber(doc), outcome, diagnostics }
        }
        MaterialKind::Excellon => {
            let (doc, outcome, diagnostics) = parse_excellon(id, content);
            ParseResult { doc: ParsedDoc::Excellon(doc), outcome, diagnostics }
        }
        MaterialKind::Stackup => {
            let (doc, outcome, diagnostics) = parse_stackup(id, content);
            ParseResult { doc: ParsedDoc::Stackup(doc), outcome, diagnostics }
        }
    }
}
