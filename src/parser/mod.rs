//! Offline parsers for the Gerber subset, Excellon drill files and the
//! project's stackup manifest. Original tokens are retained on every parsed
//! value; unconvertible values stay in place with diagnostics.

pub mod excellon;
pub mod gerber;
pub mod stackup;

pub use excellon::parse_excellon;
pub use gerber::parse_gerber;
pub use stackup::parse_stackup;

use crate::error::{Diagnostic, Failure, FailureKind};

/// Reconstruct a Gerber coordinate integer (in local decimal units) from a
/// token under the declared coordinate format. Leading/trailing zero
/// suppression change how the fixed-width field is padded.
pub fn reconstruct_fixed(
    token: &str,
    int_digits: u32,
    dec_digits: u32,
    leading_zero_suppression: bool,
) -> Result<i64, String> {
    let neg = token.starts_with('-');
    let body: String = token.chars().filter(|c| c.is_ascii_digit()).collect();
    if body.is_empty() {
        return Err(format!("no digits in coordinate token '{token}'"));
    }
    let total = (int_digits + dec_digits) as usize;
    if body.len() > total {
        return Err(format!("coordinate token '{token}' has more than {total} digits"));
    }
    let padded = if leading_zero_suppression {
        format!("{:0>width$}", body, width = total)
    } else {
        // Trailing zero suppression: pad on the right.
        format!("{:0<width$}", body, width = total)
    };
    let (i, d) = padded.split_at(int_digits as usize);
    let whole: i64 = i.parse().map_err(|_| format!("bad integer part '{i}'"))?;
    let frac: i64 = d.parse().map_err(|_| format!("bad fraction part '{d}'"))?;
    let value = whole * 10i64.pow(dec_digits) + frac;
    Ok(if neg { -value } else { value })
}

pub fn fail(diags: Vec<Diagnostic>) -> Failure {
    let kind = diags
        .iter()
        .map(|d| d.kind)
        .find(|k| *k == FailureKind::CorruptFormat)
        .or_else(|| diags.first().map(|d| d.kind))
        .unwrap_or(FailureKind::SemanticUnresolved);
    Failure { kind, diagnostics: diags }
}
