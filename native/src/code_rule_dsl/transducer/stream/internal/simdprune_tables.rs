//! simdjson's `include/simdjson/internal/simdprune_tables.h` and
//! `src/internal/simdprune_tables.cpp`: the tables of `Simd8<u8>::compress`.
//!
//! simdjson lists their values; here each is computed by a `const fn` from its definition, at
//! compile time, and `arm64/simd_tests.rs` checks entries against simdjson's listing.

/// For each mask byte, the indexes of its clear bits, in order, one per byte, then zeros.
pub(in crate::code_rule_dsl) const THINTABLE_EPI8: [u64; 256] = thintable_epi8();

/// Twice the number of set bits of each byte.
pub(in crate::code_rule_dsl) const BITS_SET_TABLE_256_MUL2: [u8; 256] = bits_set_table_256_mul2();

/// Row `k` (16 bytes at `16 k`, for `k` in 0..=8) joins two compressed halves: bytes
/// `0 .. 8 - k`, then `8 .. 16`, then 0xff. The rest is zero, as in simdjson's 272 entries.
pub(in crate::code_rule_dsl) const PSHUFB_COMBINE_TABLE: [u8; 272] = pshufb_combine_table();

const fn thintable_epi8() -> [u64; 256] {
    let mut out = [0u64; 256];
    let mut m = 0;
    while m < 256 {
        let (mut w, mut k, mut i) = (0u64, 0, 0);
        while i < 8 {
            if m >> i & 1 == 0 {
                w |= (i as u64) << (8 * k);
                k += 1;
            }
            i += 1;
        }
        out[m] = w;
        m += 1;
    }
    out
}

const fn bits_set_table_256_mul2() -> [u8; 256] {
    let mut out = [0u8; 256];
    let mut m = 0;
    while m < 256 {
        out[m] = 2 * (m as u8).count_ones() as u8;
        m += 1;
    }
    out
}

const fn pshufb_combine_table() -> [u8; 272] {
    let mut out = [0u8; 272];
    let mut k = 0;
    while k <= 8 {
        let mut j = 0;
        while j < 16 {
            let kept_low = 8 - k;
            out[16 * k + j] = if j < kept_low {
                j as u8
            } else if j < kept_low + 8 {
                (8 + j - kept_low) as u8
            } else {
                0xff
            };
            j += 1;
        }
        k += 1;
    }
    out
}
