//! simdjson's `include/simdjson/arm64/bitmask.h`.

/// A cumulative XOR, flipping each bit after a 1: `prefix_xor(00100100) == 00011100`.
///
/// simdjson uses six shifts, not PMULL, on arm64: the XORs run on the integer units, which are
/// idle while the vector code runs, and PMULL needs two extra moves between register files.
#[inline(always)]
pub(in crate::code_rule_dsl) fn prefix_xor(bitmask: u64) -> u64 {
    let mut bitmask = bitmask;
    bitmask ^= bitmask << 1;
    bitmask ^= bitmask << 2;
    bitmask ^= bitmask << 4;
    bitmask ^= bitmask << 8;
    bitmask ^= bitmask << 16;
    bitmask ^= bitmask << 32;
    bitmask
}
