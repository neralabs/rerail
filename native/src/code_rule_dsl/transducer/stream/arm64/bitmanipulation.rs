//! simdjson's `include/simdjson/arm64/bitmanipulation.h`. Each function is one instruction:
//! `rbit` + `clz` for trailing zeroes, `clz`, `cnt` + `addv`, `rbit`.

/// The result is unspecified when `input_num` is zero; as in simdjson, it is then not used.
#[inline(always)]
pub(in crate::code_rule_dsl) fn trailing_zeroes(input_num: u64) -> i32 {
    input_num.trailing_zeros() as i32
}

/// The result is unspecified when `input_num` is zero.
#[inline(always)]
pub(in crate::code_rule_dsl) fn clear_lowest_bit(input_num: u64) -> u64 {
    input_num & input_num.wrapping_sub(1)
}

/// The result is unspecified when `input_num` is zero.
#[inline(always)]
pub(in crate::code_rule_dsl) fn leading_zeroes(input_num: u64) -> i32 {
    input_num.leading_zeros() as i32
}

#[inline(always)]
pub(in crate::code_rule_dsl) fn count_ones(input_num: u64) -> i32 {
    input_num.count_ones() as i32
}

/// `rbit`. simdjson sets `SIMDJSON_PREFER_REVERSE_BITS` on arm64 for the algorithms that use it.
#[inline(always)]
pub(in crate::code_rule_dsl) fn reverse_bits(input_num: u64) -> u64 {
    input_num.reverse_bits()
}

/// Flips the bit at index `63 - leading_zeroes`: with `leading_zeroes` from `rev_bits`, it
/// clears the leading bit. As in simdjson, the result for 64 or more is not used; the shift
/// wraps rather than being undefined.
#[inline(always)]
pub(in crate::code_rule_dsl) fn zero_leading_bit(rev_bits: u64, leading_zeroes: i32) -> u64 {
    rev_bits ^ 0x8000_0000_0000_0000u64.wrapping_shr(leading_zeroes as u32)
}
