//! The hash every index and every kernel uses for a key (moved from `rerail_blocks`): one value at a time, the same
//! arithmetic in UKL.

/// The hash's seed.
pub const SEED: u64 = 0x243F_6A88_85A3_08D3;

/// The arithmetic is the CLIF's `rel_key_mix` (decision 0325 step 5), which the kernels and the JIT call.
#[inline(always)]
pub fn mix(h: u64, v: i64) -> u64 {
    let h = (h ^ v as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h ^ (h >> 29)
}

/// A key's hash.
#[inline]
pub fn key_hash(vals: &[i64]) -> u64 {
    vals.iter().fold(SEED, |h, &v| mix(h, v))
}
