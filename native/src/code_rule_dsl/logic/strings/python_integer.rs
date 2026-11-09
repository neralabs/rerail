//! Python 3.13's decimal integer syntax, including a bounded-allocation text projection.
//! Unicode Nd and whitespace follow Python's Unicode 15.1 database. The caller
//! supplies the digit limit so the result does not depend on hidden global state.

const DECIMAL_ZEROES: &[u32] = &[
    48,1632,1776,1984,2406,2534,2662,2790,2918,3046,3174,3302,
    3430,3558,3664,3792,3872,4160,4240,6112,6160,6470,6608,6784,
    6800,6992,7088,7232,7248,42528,43216,43264,43472,43504,43600,
    44016,65296,66720,68912,69734,69872,69942,70096,70384,70736,
    70864,71248,71360,71472,71904,72016,72784,73040,73120,73552,
    92768,92864,93008,120782,120792,120802,120812,120822,123200,
    123632,124144,125264,130032,
];

fn whitespace(c: char) -> bool {
    matches!(c as u32, 9..=13 | 32 | 133 | 160 | 5760 | 8192..=8202 | 8232 | 8233 | 8239 | 8287 | 12288)
}

fn digit(c: char) -> Option<u32> {
    let code = c as u32;
    let position = DECIMAL_ZEROES.partition_point(|&zero| zero <= code);
    let zero = *DECIMAL_ZEROES.get(position.checked_sub(1)?)?;
    (code - zero < 10).then_some(code - zero)
}

/// Canonical decimal text, with Python's default 4300-digit conversion limit.
pub(super) fn canonical(text: &str) -> Result<String, String> {
    let invalid = || format!("invalid literal for int() with base 10: {}", crate::code_rule_dsl::facts::vtab::syntax::python_print::string_repr(text));
    let trimmed = text.trim_matches(whitespace);
    let (negative, digits) = if let Some(rest) = trimmed.strip_prefix('-') { (true, rest) }
        else { (false, trimmed.strip_prefix('+').unwrap_or(trimmed)) };
    let mut normalized = String::new();
    let mut previous_digit = false;
    for c in digits.chars() {
        if c == '_' && previous_digit { previous_digit = false; continue; }
        let Some(d) = digit(c) else { return Err(invalid()); };
        normalized.push(char::from(b'0' + d as u8));
        previous_digit = true;
    }
    if !previous_digit { return Err(invalid()); }
    if normalized.len() > 4300 {
        return Err(format!("Exceeds the limit (4300 digits) for integer string conversion: value has {} digits; use sys.set_int_max_str_digits() to increase the limit", normalized.len()));
    }
    let magnitude = normalized.trim_start_matches('0');
    Ok(if magnitude.is_empty() { "0".into() } else { format!("{}{magnitude}", if negative { "-" } else { "" }) })
}

pub(super) fn clamped(text: &str, lower: i64, upper: i64, max_digits: i64) -> Option<i64> {
    if lower > upper || max_digits < 0 { return None; }
    let text = text.trim_matches(whitespace);
    let (negative, digits) = if let Some(rest) = text.strip_prefix('-') { (true, rest) }
        else { (false, text.strip_prefix('+').unwrap_or(text)) };
    let mut magnitude = 0_u128;
    let mut count = 0_u64;
    let mut previous_digit = false;
    for c in digits.chars() {
        if c == '_' && previous_digit {
            previous_digit = false;
            continue;
        }
        let value = digit(c)?;
        magnitude = magnitude.saturating_mul(10).saturating_add(u128::from(value));
        count += 1;
        if max_digits != 0 && count > max_digits as u64 { return None; }
        previous_digit = true;
    }
    if !previous_digit { return None; }
    let magnitude = magnitude.min(i128::MAX as u128) as i128;
    let value = if negative { -magnitude } else { magnitude };
    Some(value.clamp(i128::from(lower), i128::from(upper)) as i64)
}
