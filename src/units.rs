//! Coordinate formats and units.
//!
//! Canonical computation unit is the nanometre (f64 nm for geometry; input
//! representations are always preserved verbatim). Gerber inch coordinates in
//! common formats convert exactly to an integer multiple of 25.4 nm, which we
//! represent without information loss for the formats used in the bench.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Unit {
    Inch,
    Mm,
}

impl Unit {
    pub fn code(self) -> &'static str {
        match self {
            Unit::Inch => "in",
            Unit::Mm => "mm",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Unit::Inch => "inch",
            Unit::Mm => "millimetre",
        }
    }
}

/// Exactly what the source file said for one scalar, before normalization.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RawNumber {
    /// Original lexical token, e.g. "002500" for 002500 format or "2.5".
    pub text: String,
    /// Gerber-style integer/decimal split (9 is unset), e.g. (2,4).
    pub format: (u8, u8),
    pub unit: Unit,
    /// 1-based source line, 0-based column of the first token character.
    pub line: u32,
    pub col: u32,
}

impl RawNumber {
    pub fn new(text: impl Into<String>, int: u8, dec: u8, unit: Unit, line: u32, col: u32) -> Self {
        Self { text: text.into(), format: (int, dec), unit, line, col }
    }

    /// Interpret the token under its coordinate format. Returns None when the
    /// token cannot be converted (illegal digits / overflow); callers must keep
    /// the raw value and emit a positioned diagnostic instead of substituting
    /// a default.
    pub fn interpreted_value(&self) -> Option<f64> {
        let t = self.text.trim();
        let (neg, body) = match t.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, t),
        };
        let body = body.strip_prefix('+').unwrap_or(body);
        if body.is_empty() {
            return None;
        }
        let (_int_digits, dec_digits) = self.format;
        let digits: String = body.chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.len() != body.chars().filter(|c| c.is_ascii_digit() || *c == '.').count() {
            return None;
        }
        let has_dot = body.contains('.');
        let value = if has_dot || dec_digits == 0 {
            body.parse::<f64>().ok()?
        } else {
            if digits.is_empty() {
                return None;
            }
            let pad = (dec_digits as usize).saturating_sub(digits.len());
            let mut s = String::new();
            for ch in digits.chars() {
                if s.len() == pad {
                    s.push('.');
                }
                s.push(ch);
            }
            if pad >= digits.len() {
                s.insert(0, '0');
                if pad == digits.len() {
                    s.insert(0, '.');
                }
            }
            s.parse::<f64>().ok()?
        };
        Some(if neg { -value } else { value })
    }

    /// Nanometres, or None when conversion is impossible. Inch values that land
    /// on the 25.4 nm lattice are exact by construction; millimetres are x1e6.
    pub fn to_nm(&self) -> Option<f64> {
        let v = self.interpreted_value()?;
        Some(match self.unit {
            Unit::Mm => v * 1_000_000.0,
            Unit::Inch => v * 25_400_000.0,
        })
    }
}

/// Convert a bare numeric string at a known unit into nanometres.
pub fn scalar_to_nm(text: &str, unit: Unit) -> Option<f64> {
    let v: f64 = text.parse().ok()?;
    Some(match unit {
        Unit::Mm => v * 1_000_000.0,
        Unit::Inch => v * 25_400_000.0,
    })
}

/// True when a nm value sits on the exact 25.4 nm inch lattice within the
/// numeric noise of f64 (only used for display hints, never for decisions).
pub fn is_inch_lattice(nm: f64) -> bool {
    let q = nm / 25.4;
    (q - q.round()).abs() < 1e-9
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inch_254_grid_is_exact() {
        // 0.0010 inch in 2:4 format token "000100" -> 25400 nm exactly.
        let r = RawNumber::new("000100", 2, 4, Unit::Inch, 1, 1);
        assert_eq!(r.to_nm(), Some(25_400.0));
        let r = RawNumber::new("002500", 2, 4, Unit::Inch, 1, 1);
        assert_eq!(r.to_nm(), Some(635_000.0));
    }

    #[test]
    fn mm_token_converts() {
        let r = RawNumber::new("2500", 2, 3, Unit::Mm, 1, 1);
        assert_eq!(r.interpreted_value(), Some(2.5));
        assert_eq!(r.to_nm(), Some(2_500_000.0));
    }

    #[test]
    fn explicit_decimal_token() {
        let r = RawNumber::new("0.001", 2, 5, Unit::Inch, 1, 1);
        assert_eq!(r.to_nm(), Some(25_400.0));
    }

    #[test]
    fn garbage_token_is_unconvertible_not_default() {
        let r = RawNumber::new("12OZ", 2, 4, Unit::Inch, 3, 7);
        assert_eq!(r.to_nm(), None);
    }
}
