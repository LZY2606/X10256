//! Units and coordinate bases. Domain quantities carry their unit/basis;
//! original representations live on source operations, normalized `Nano`
//! values only appear on derived layers.

use serde::{Deserialize, Serialize};

/// Nanometres in the project coordinate basis (right-handed, Gerber orientation).
pub type Nano = i64;

pub const NM_PER_MM: Nano = 1_000_000;
pub const NM_PER_IN: Nano = 25_400_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LengthUnit {
    Inch,
    Mm,
    /// Unit could not be converted; the raw token stays at its origin.
    Unknown,
}

impl LengthUnit {
    pub fn tag(self) -> &'static str {
        match self {
            LengthUnit::Inch => "in",
            LengthUnit::Mm => "mm",
            LengthUnit::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ZeroSuppression {
    Leading,
    Trailing,
    None,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CoordMode {
    Absolute,
    Incremental,
    Unknown,
}

/// Gerber coordinate format, e.g. `FSLAX35Y35` -> 3 integer / 5 decimal digits,
/// leading-zero suppression, absolute coordinates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GerberFormat {
    pub raw: String,
    pub unit: LengthUnit,
    pub int_digits: u32,
    pub dec_digits: u32,
    pub zero: ZeroSuppression,
    pub mode: CoordMode,
}

impl Default for GerberFormat {
    fn default() -> Self {
        GerberFormat {
            raw: String::new(),
            unit: LengthUnit::Unknown,
            int_digits: 0,
            dec_digits: 0,
            zero: ZeroSuppression::Unknown,
            mode: CoordMode::Unknown,
        }
    }
}

impl GerberFormat {
    /// Convert a raw fixed-point coordinate token to project nanometres.
    /// Returns `None` when the format/unit does not permit conversion; the
    /// caller must keep the raw token and attach a diagnostic in that case.
    pub fn coord_to_nano(&self, token: &str, negative: bool) -> Option<Nano> {
        if self.int_digits == 0 && self.dec_digits == 0 {
            return None;
        }
        let digits: String = token.chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            return None;
        }
        let total = (self.int_digits + self.dec_digits) as usize;
        let normalized = match self.zero {
            ZeroSuppression::Leading => {
                if digits.len() > total {
                    return None;
                }
                format!("{:0>width$}", digits, width = total)
            }
            ZeroSuppression::Trailing => {
                if digits.len() > total {
                    return None;
                }
                format!("{:0<width$}", digits, width = total)
            }
            ZeroSuppression::None | ZeroSuppression::Unknown => {
                if digits.len() != total {
                    return None;
                }
                digits
            }
        };
        let int_part: i64 = normalized[..self.int_digits as usize].parse().ok()?;
        let dec_part: i64 = normalized[self.int_digits as usize..].parse().ok()?;
        let factor = match self.unit {
            LengthUnit::Inch => NM_PER_IN,
            LengthUnit::Mm => NM_PER_MM,
            LengthUnit::Unknown => return None,
        };
        let ten_d = 10i64.checked_pow(self.dec_digits)?;
        let scaled = int_part.checked_mul(factor)?
            + dec_part.checked_mul(factor)? / ten_d;
        Some(if negative { -scaled } else { scaled })
    }

    /// Convert an aperture modifier (decimal number in file units) to nanometres.
    pub fn measure_to_nano(&self, text: &str) -> Option<Nano> {
        measure_to_nano(text, self.unit)
    }
}

pub fn measure_to_nano(text: &str, unit: LengthUnit) -> Option<Nano> {
    let factor = match unit {
        LengthUnit::Inch => NM_PER_IN,
        LengthUnit::Mm => NM_PER_MM,
        LengthUnit::Unknown => return None,
    };
    let text = text.trim();
    let neg = text.starts_with('-');
    let body = text.trim_start_matches('-');
    let (a, b) = body.split_once('.').unwrap_or((body, ""));
    let int_part: i64 = if a.is_empty() { 0 } else { a.parse().ok()? };
    let dec_part: i64 = if b.is_empty() { 0 } else { b.parse().ok()? };
    let ten_d = 10i64.checked_pow(b.len() as u32)?;
    let scaled = int_part.checked_mul(factor)? + dec_part.checked_mul(factor)? / ten_d;
    Some(if neg { -scaled } else { scaled })
}
