//! A number's check: `kernel.inc`'s `number_end`, which is simdjson's `numberparsing::parse_number`
//! with its value not kept (`no_values`), translated with the parts of simdjson 5.0.5's
//! `generic/numberparsing.h` and `arm64/numberparsing_defs.h` it calls. The value is still worked
//! out where it decides the answer: a float past the doubles' range is an error.

use crate::Code;
use crate::tables::{POWER_OF_FIVE_128, STRUCTURAL_OR_WHITESPACE_NEGATED};

const SMALLEST_POWER: i64 = -342;
const LARGEST_POWER: i64 = 308;

/// `jsoncharutils::is_not_structural_or_whitespace`.
#[inline(always)]
pub(crate) fn is_not_structural_or_whitespace(c: u8) -> u32 {
    u32::from(STRUCTURAL_OR_WHITESPACE_NEGATED[c as usize])
}

/// `parse_digit`, for a `u64`: `i` may overflow, checked later.
#[inline(always)]
fn parse_digit(c: u8, i: &mut u64) -> bool {
    let digit = c.wrapping_sub(b'0');
    if digit > 9 {
        return false;
    }
    *i = i.wrapping_mul(10).wrapping_add(u64::from(digit));
    true
}

/// `parse_digit`, for the exponent's `i64`.
#[inline(always)]
fn parse_digit_i64(c: u8, i: &mut i64) -> bool {
    let digit = c.wrapping_sub(b'0');
    if digit > 9 {
        return false;
    }
    *i = i.wrapping_mul(10).wrapping_add(i64::from(digit));
    true
}

/// `is_made_of_eight_digits_fast`.
#[inline(always)]
unsafe fn is_made_of_eight_digits_fast(chars: *const u8) -> bool {
    // SAFETY: reads up to 7 bytes past the digits, which the padding covers.
    let val = unsafe { (chars as *const u64).read_unaligned() };
    ((val & 0xF0F0F0F0F0F0F0F0) | ((val.wrapping_add(0x0606060606060606) & 0xF0F0F0F0F0F0F0F0) >> 4)) == 0x3333333333333333
}

/// `is_made_of_four_digits_fast`.
#[inline(always)]
unsafe fn is_made_of_four_digits_fast(chars: *const u8) -> bool {
    // SAFETY: reads up to 3 bytes past the digits, which the padding covers.
    let val = unsafe { (chars as *const u32).read_unaligned() };
    ((val.wrapping_add(0x46464646) | val.wrapping_sub(0x30303030)) & 0x80808080) == 0
}

/// `parse_four_digits_unrolled`.
#[inline(always)]
unsafe fn parse_four_digits_unrolled(chars: *const u8) -> u32 {
    // SAFETY: four digits.
    let mut val = unsafe { (chars as *const u32).read_unaligned() };
    val = val.wrapping_sub(0x30303030);
    val = val.wrapping_mul(10).wrapping_add(val >> 8);
    ((val & 0x00FF00FF).wrapping_mul(0x00640001) >> 16) & 0xFFFF
}

/// arm64's `parse_eight_digits_unrolled`.
#[inline(always)]
unsafe fn parse_eight_digits_unrolled(chars: *const u8) -> u32 {
    // SAFETY: eight digits.
    let mut val = unsafe { (chars as *const u64).read_unaligned() };
    val = (val & 0x0F0F0F0F0F0F0F0F).wrapping_mul(2561) >> 8;
    val = (val & 0x00FF00FF00FF00FF).wrapping_mul(6553601) >> 16;
    ((val & 0x0000FFFF0000FFFF).wrapping_mul(42949672960001) >> 32) as u32
}

/// `parse_fraction_digits` (`SIMDJSON_SWAR_NUMBER_PARSING`).
#[inline(always)]
unsafe fn parse_fraction_digits(p: &mut *const u8, i: &mut u64) {
    // SAFETY: within the padded source (each read stops at a non-digit, and the padding has none).
    unsafe {
        while is_made_of_eight_digits_fast(*p) {
            *i = i.wrapping_mul(100000000).wrapping_add(u64::from(parse_eight_digits_unrolled(*p)));
            *p = p.add(8);
        }
        if is_made_of_four_digits_fast(*p) {
            *i = i.wrapping_mul(10000).wrapping_add(u64::from(parse_four_digits_unrolled(*p)));
            *p = p.add(4);
        }
        while parse_digit(**p, i) {
            *p = p.add(1);
        }
    }
}

/// `parse_decimal_after_separator`.
#[inline(always)]
unsafe fn parse_decimal_after_separator(p: &mut *const u8, i: &mut u64, exponent: &mut i64) -> Code {
    let first_after_period = *p;
    // SAFETY: as `parse_fraction_digits`.
    unsafe { parse_fraction_digits(p, i) };
    // SAFETY: both in the source.
    *exponent = unsafe { first_after_period.offset_from(*p) } as i64;
    if *exponent == 0 {
        return Code::NumberError;
    }
    Code::Success
}

/// `parse_exponent`.
#[inline(always)]
unsafe fn parse_exponent(p: &mut *const u8, exponent: &mut i64) -> Code {
    // SAFETY: within the padded source.
    unsafe {
        let neg_exp = b'-' == **p;
        if neg_exp || b'+' == **p {
            *p = p.add(1);
        }
        let mut start_exp = *p;
        let mut exp_number = 0i64;
        while parse_digit_i64(**p, &mut exp_number) {
            *p = p.add(1);
        }
        if *p == start_exp {
            return Code::NumberError;
        }
        if *p > start_exp.add(18) {
            while *start_exp == b'0' {
                start_exp = start_exp.add(1);
            }
            if *p > start_exp.add(18) {
                exp_number = 999999999999999999;
            }
        }
        *exponent = exponent.wrapping_add(if neg_exp { exp_number.wrapping_neg() } else { exp_number });
    }
    Code::Success
}

/// `significant_digits`.
#[inline(always)]
unsafe fn significant_digits(start_digits: *const u8, digit_count: usize) -> usize {
    let mut start = start_digits;
    // SAFETY: the digits are followed by a non-digit.
    unsafe {
        while *start == b'0' || *start == b'.' {
            start = start.add(1);
        }
        digit_count.wrapping_sub(start.offset_from(start_digits) as usize)
    }
}

/// `full_multiplication`.
#[inline(always)]
fn full_multiplication(value1: u64, value2: u64) -> (u64, u64) {
    let r = u128::from(value1) * u128::from(value2);
    (r as u64, (r >> 64) as u64)
}

/// `compute_float_64`: whether the value is a finite double (the value itself is not kept).
#[inline(always)]
fn compute_float_64(power: i64, mut i: u64) -> bool {
    if (-22..=22).contains(&power) && i <= 9007199254740991 {
        // `i` times or over a power of ten up to 22: finite (the value, not kept, is not made).
        return true;
    }
    if i == 0 {
        return true;
    }
    let exponent = (((152170 + 65536) * power) >> 16) + 1024 + 63;
    let mut lz = i.leading_zeros() as i64;
    i <<= lz;
    let index = 2 * (power - SMALLEST_POWER) as usize;
    let (mut low, mut high) = full_multiplication(i, POWER_OF_FIVE_128[index]);
    if (high & 0x1FF) == 0x1FF {
        let (_, second_high) = full_multiplication(i, POWER_OF_FIVE_128[index + 1]);
        low = low.wrapping_add(second_high);
        if second_high > low {
            high = high.wrapping_add(1);
        }
    }
    let lower = low;
    let upper = high;
    let upperbit = upper >> 63;
    let mut mantissa = upper >> (upperbit + 9);
    lz += (1 ^ upperbit) as i64;
    let mut real_exponent = exponent - lz;
    if real_exponent <= 0 {
        // A subnormal, or zero: finite.
        return true;
    }
    if lower <= 1 && (-4..=23).contains(&power) && (mantissa & 3) == 1 && (mantissa << (upperbit + 64 - 53 - 2)) == upper {
        mantissa &= !1;
    }
    mantissa += mantissa & 1;
    mantissa >>= 1;
    if mantissa >= (1u64 << 53) {
        real_exponent += 1;
    }
    real_exponent <= 2046
}

/// `parse_float_fallback`: the number as a double, correctly rounded (simdjson's `from_chars`), is
/// finite.
#[cold]
#[inline(never)]
fn parse_float_fallback(src: *const u8, end: *const u8) -> bool {
    // SAFETY: the number's bytes, ASCII.
    let text = unsafe { core::str::from_utf8_unchecked(core::slice::from_raw_parts(src, end.offset_from(src) as usize)) };
    text.parse::<f64>().is_ok_and(f64::is_finite)
}

/// `write_float`, its value not kept.
#[inline(always)]
unsafe fn write_float(src: *const u8, end: *const u8, i: u64, start_digits: *const u8, digit_count: usize, exponent: i64) -> Code {
    // SAFETY: the number's digits.
    if digit_count > 19 && unsafe { significant_digits(start_digits, digit_count) } > 19 {
        return if parse_float_fallback(src, end) { Code::Success } else { Code::NumberError };
    }
    if !(SMALLEST_POWER..=LARGEST_POWER).contains(&exponent) {
        if exponent < SMALLEST_POWER || i == 0 {
            return Code::Success;
        }
        return Code::NumberError;
    }
    if !compute_float_64(exponent, i) && !parse_float_fallback(src, end) {
        return Code::NumberError;
    }
    Code::Success
}

/// `number_end`: a number checked as simdjson checks it, and where it ends.
///
/// # Safety
/// `src` is in the padded source.
#[inline(always)]
pub(crate) unsafe fn number_end(src: *const u8, end: &mut *const u8) -> Code {
    // SAFETY: each read stops at a byte that ends the number, and the padding ends every number.
    unsafe {
        let negative = *src == b'-';
        let mut p = src.add(usize::from(negative));
        let start_digits = p;
        let mut i = 0u64;
        while parse_digit(*p, &mut i) {
            p = p.add(1);
        }
        let mut digit_count = p.offset_from(start_digits) as usize;
        if digit_count == 0 || (b'0' == *start_digits && digit_count > 1) {
            return Code::NumberError;
        }
        let mut exponent = 0i64;
        let mut is_float = false;
        if b'.' == *p {
            is_float = true;
            p = p.add(1);
            let e = parse_decimal_after_separator(&mut p, &mut i, &mut exponent);
            if e != Code::Success {
                return e;
            }
            digit_count = p.offset_from(start_digits) as i32 as usize;
        }
        if b'e' == *p || b'E' == *p {
            is_float = true;
            p = p.add(1);
            let e = parse_exponent(&mut p, &mut exponent);
            if e != Code::Success {
                return e;
            }
        }
        *end = p;
        if is_float {
            let dirty_end = is_not_structural_or_whitespace(*p) != 0;
            let e = write_float(src, p, i, start_digits, digit_count, exponent);
            if e != Code::Success {
                return e;
            }
            return if dirty_end { Code::NumberError } else { Code::Success };
        }
        let longest_digit_count = if negative { 19 } else { 20 };
        if digit_count > longest_digit_count {
            return Code::BigintError;
        }
        if digit_count == longest_digit_count {
            if negative {
                if i > i64::MAX as u64 + 1 {
                    return Code::BigintError;
                }
            } else if *src != b'1' || i <= i64::MAX as u64 {
                return Code::BigintError;
            }
        }
        if is_not_structural_or_whitespace(*p) != 0 { Code::NumberError } else { Code::Success }
    }
}
