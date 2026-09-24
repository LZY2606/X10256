//! 单位与坐标基准。
//!
//! 原始文件中的所有量在导入阶段都保留“原文表示 + 单位/坐标基准”。
//! 只有派生几何层才规范化为内部整数单位（皮米，`pm`），避免英寸小数误差。

use serde::{Deserialize, Serialize};

/// 可识别长度单位。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LengthUnit {
    /// 英寸（Gerber 默认、Excellon 常见）。
    Inch,
    /// 毫米（`%MOIN*%` / `METRIC,000.000` 的对立面）。
    Mm,
    /// 内部规范化单位：皮米。
    Pm,
}

impl LengthUnit {
    pub fn as_str(self) -> &'static str {
        match self {
            LengthUnit::Inch => "inch",
            LengthUnit::Mm => "mm",
            LengthUnit::Pm => "pm",
        }
    }

    /// 1 单位等于多少皮米。英寸取 25_400_000_000，故 `2:5`/`2:6` 英寸可精确表示。
    pub fn pm_per_unit(self) -> i128 {
        match self {
            LengthUnit::Inch => 25_400_000_000,
            LengthUnit::Mm => 1_000_000_000,
            LengthUnit::Pm => 1,
        }
    }
}

/// 坐标基准（drawing datum）。偏移坐标基准下派生几何平移，但原值与基准都保留。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Datum {
    pub x_raw: String,
    pub y_raw: String,
    pub unit: LengthUnit,
    pub x_pm: i128,
    pub y_pm: i128,
}

impl Datum {
    pub fn zero(unit: LengthUnit) -> Self {
        Datum {
            x_raw: "0".to_string(),
            y_raw: "0".to_string(),
            unit,
            x_pm: 0,
            y_pm: 0,
        }
    }
}

/// 带单位的原始量：保留原文，派生时给出规范化值。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawQuantity {
    pub raw: String,
    pub unit: LengthUnit,
    pub pm: Option<i128>,
    pub diagnostic_id: Option<String>,
}

/// 把有限十进制数（可带符号、小数点）按 `value * 10^exponent` 精确换算成皮米。
/// 非有限或溢出时返回 `None`，调用方必须保留原值并附诊断，而不是使用默认值。
pub fn decimal_to_pm(raw: &str, unit: LengthUnit) -> Option<i128> {
    let s = raw.trim();
    let (negative, digits) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let mut int_part: i128 = 0;
    let mut frac_digits = 0i32;
    let mut seen_dot = false;
    let mut any_digit = false;
    for ch in digits.chars() {
        if ch == '.' {
            if seen_dot {
                return None;
            }
            seen_dot = true;
        } else if let Some(d) = ch.to_digit(10) {
            any_digit = true;
            if seen_dot {
                frac_digits += 1;
                int_part = int_part.checked_mul(10)? + d as i128;
            } else {
                int_part = int_part.checked_mul(10)? + d as i128;
            }
        } else {
            return None;
        }
    }
    if !any_digit {
        return None;
    }
    // 实际整数 = int_part / 10^frac_digits（int_part 已把小数位并入）。
    // pm = value * pm_per_unit = int_part * pm_per_unit / 10^frac_digits，必须整除才接受。
    let scale = unit.pm_per_unit();
    let mut numerator = int_part.checked_mul(scale)?;
    let mut divisor: i128 = 1;
    for _ in 0..frac_digits {
        divisor = divisor.checked_mul(10)?;
    }
    if numerator % divisor != 0 {
        return None;
    }
    numerator /= divisor;
    if negative {
        numerator = numerator.checked_neg()?;
    }
    Some(numerator)
}

/// Gerber/Excellon 的隐式小数点定点数：`digits` 个整数位、`decimals` 个小数位。
/// 例如英寸 `2:5` 下的 `01000` 表示 0.1 英寸。
pub fn fixed_point_to_pm(raw_digits: &str, unit: LengthUnit, int_digits: u32, dec_digits: u32) -> Option<i128> {
    let negative = raw_digits.starts_with('-');
    let body: String = raw_digits.chars().filter(|c| c.is_ascii_digit()).collect();
    if body.is_empty() {
        return None;
    }
    let total = int_digits + dec_digits;
    if body.len() > total as usize {
        return None;
    }
    let padded = format!("{:0>width$}", body, width = total as usize);
    let (whole, frac) = padded.split_at(int_digits as usize);
    let composed = format!("{}.{}", whole, frac);
    let mut value = decimal_to_pm(&composed, unit)?;
    if negative {
        value = value.checked_neg()?;
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inch_metric_conversions_are_exact() {
        assert_eq!(decimal_to_pm("1", LengthUnit::Inch), Some(25_400_000_000));
        assert_eq!(decimal_to_pm("25.4", LengthUnit::Mm), Some(25_400_000_000));
        assert_eq!(fixed_point_to_pm("01000", LengthUnit::Inch, 2, 5), Some(2_540_000_000));
        // 0.000001 英寸 = 25.4pm 非整数：不可精确转换，必须拒绝。
        assert_eq!(fixed_point_to_pm("000001", LengthUnit::Inch, 2, 6), None);
        assert_eq!(decimal_to_pm("1.5", LengthUnit::Mm), Some(1_500_000_000));
        assert_eq!(decimal_to_pm("not-a-number", LengthUnit::Mm), None);
    }
}
