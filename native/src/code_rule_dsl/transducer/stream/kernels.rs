//! Block kernels: each operation of `ops.rs` on one 64-byte block and its carry.
//!
//! Every kernel does the same work whatever the block holds. Mask kernels are a few word
//! instructions; lane kernels are fixed loops over the 64 lanes of a block, the width of
//! simdjson's `simd8x64` (4 NEON registers), with no branch on a lane's value. These are the
//! semantics the interpreter runs and the code generators and Metal reproduce.

use super::ops::{Membership, SINK, STATES};

pub(in crate::code_rule_dsl) type Lanes = [u8; 64];

#[inline]
pub(in crate::code_rule_dsl) fn prev(w: u64, before: u64, k: u32) -> u64 {
    if k >= 64 { before } else { (w << k) | (before >> (64 - k)) }
}

#[inline]
pub(in crate::code_rule_dsl) fn next(w: u64, after: u64, k: u32) -> u64 {
    if k >= 64 { after } else { (w >> k) | (after << (64 - k)) }
}

/// Prefix XOR within the word: six shifts (simdjson's arm64 `prefix_xor`).
#[inline]
pub(in crate::code_rule_dsl) fn prefix_xor(m: u64) -> u64 {
    let mut p = m;
    p ^= p << 1;
    p ^= p << 2;
    p ^= p << 4;
    p ^= p << 8;
    p ^= p << 16;
    p ^= p << 32;
    p
}

/// `carry` is all ones when the parity before the block is odd.
#[inline]
pub(in crate::code_rule_dsl) fn toggle(m: u64, carry: &mut u64) -> u64 {
    let p = prefix_xor(m) ^ *carry;
    *carry = ((p as i64) >> 63) as u64;
    p
}

/// From each start up to, not including, the next end: one add, its carry the borrow passed on.
#[inline]
pub(in crate::code_rule_dsl) fn fill(s: u64, e: u64, carry: &mut u64) -> u64 {
    let (a, b) = (s, !e | s);
    let sum = a.wrapping_add(b).wrapping_add(*carry);
    let c = sum ^ a ^ b;
    let f = s | (!e & c);
    *carry = f >> 63;
    f
}

/// `fill` mirrored: called on blocks last to first.
#[inline]
pub(in crate::code_rule_dsl) fn back_fill(s: u64, e: u64, carry: &mut u64) -> u64 {
    fill(s.reverse_bits(), e.reverse_bits(), carry).reverse_bits()
}

/// Bits of `k` mod residues: the word of positions `64 b + i` with `(64 b + i) % k == c`, for each
/// `r = 64 b % k`.
pub(in crate::code_rule_dsl) fn pattern_words(k: u32, c: u32) -> Vec<u64> {
    (0..k).map(|r| (0..64u32).fold(0u64, |w, i| w | (((r + i) % k == c) as u64) << i)).collect()
}

/// A mask from a predicate over the 64 lanes (NEON: compare, then simdjson's `to_bitmask`).
#[inline]
pub(in crate::code_rule_dsl) fn bits(f: impl Fn(usize) -> bool) -> u64 {
    let mut w = 0u64;
    for l in 0..64 {
        w |= (f(l) as u64) << l;
    }
    w
}

/// 0xff in the lanes of `m`'s set bits.
#[inline]
pub(in crate::code_rule_dsl) fn expand(m: u64) -> Lanes {
    let mut out = [0u8; 64];
    for (l, o) in out.iter_mut().enumerate() {
        *o = 0u8.wrapping_sub((m >> l & 1) as u8);
    }
    out
}

#[inline]
pub(in crate::code_rule_dsl) fn classify(bytes: &[u8], table: &[u8; 256]) -> Lanes {
    let mut out = [0u8; 64];
    for (o, &b) in out.iter_mut().zip(bytes) {
        *o = table[b as usize];
    }
    out
}

#[inline]
pub(in crate::code_rule_dsl) fn lane_select(m: u64, a: &Lanes, b: &Lanes) -> Lanes {
    let x = expand(m);
    let mut out = [0u8; 64];
    for l in 0..64 {
        out[l] = (a[l] & x[l]) | (b[l] & !x[l]);
    }
    out
}

#[inline]
pub(in crate::code_rule_dsl) fn lane_map(a: &Lanes, table: &[u8; 256]) -> Lanes {
    classify(a, table)
}

#[inline]
pub(in crate::code_rule_dsl) fn lane_prev(a: &Lanes, before: u8) -> Lanes {
    let mut out = [0u8; 64];
    out[0] = before;
    out[1..].copy_from_slice(&a[..63]);
    out
}

/// Maps over the states composed by a Hillis–Steele scan: six steps, each composing every lane's
/// map with the map `d` lanes before (NEON: one `vqtbl1q_u8` per lane per step).
pub(in crate::code_rule_dsl) fn map_scan(entries: &[&Lanes], carry: &mut u8) -> Lanes {
    let mut maps = [[0u8; STATES]; 64];
    for (l, m) in maps.iter_mut().enumerate() {
        for (s, v) in m.iter_mut().enumerate() {
            *v = if s < entries.len() { entries[s][l].min(SINK) } else { s as u8 };
        }
    }
    for d in [1usize, 2, 4, 8, 16, 32] {
        for l in (d..64).rev() {
            let (before, here) = (maps[l - d], maps[l]);
            for s in 0..STATES {
                maps[l][s] = here[before[s] as usize];
            }
        }
    }
    let mut out = [0u8; 64];
    for l in 0..64 {
        out[l] = maps[l][*carry as usize];
    }
    *carry = out[63];
    out
}

/// A count with a floor at 0 and a cap of `max + 1`: per byte a map `x ↦ clamp(x + a, lo, hi)`,
/// composed by the same scan as `map_scan`.
pub(in crate::code_rule_dsl) fn count(up: u64, down: u64, reset: u64, first: u64, max: u8, carry: &mut u8) -> Lanes {
    let cap = max as i16 + 1;
    let (u, d, r, f) = (expand(up), expand(down), expand(reset), expand(first));
    let mut a = [0i16; 64];
    let mut lo = [0i16; 64];
    let mut hi = [cap; 64];
    for l in 0..64 {
        // Priority: reset, then up, then down; `first` applies the step to 0.
        let (is_r, is_u, is_d) = ((r[l] & 1) as i16, (u[l] & 1 & !r[l]) as i16, (d[l] & 1 & !u[l] & !r[l]) as i16);
        let step = is_u - is_d;
        a[l] = step;
        hi[l] = cap * (1 - is_r);
        // At `first`: the constant `action(0)`: 1 after an up, 0 otherwise.
        let is_f = (f[l] & 1) as i16;
        let at0 = is_u;
        a[l] = a[l] * (1 - is_f);
        lo[l] = is_f * at0;
        hi[l] = hi[l] * (1 - is_f) + is_f * at0;
    }
    for dd in [1usize, 2, 4, 8, 16, 32] {
        for l in (dd..64).rev() {
            let (a1, lo1, hi1) = (a[l - dd], lo[l - dd], hi[l - dd]);
            let (a2, lo2, hi2) = (a[l], lo[l], hi[l]);
            a[l] = a1 + a2;
            lo[l] = (lo1 + a2).clamp(lo2, hi2);
            hi[l] = (hi1 + a2).clamp(lo2, hi2);
        }
    }
    let mut out = [0u8; 64];
    let x0 = *carry as i16;
    for l in 0..64 {
        out[l] = (x0 + a[l]).clamp(lo[l], hi[l]) as u8;
    }
    *carry = out[63];
    out
}

/// Per block, what `ucode` shifts: for each `d` in 1..=3 the bytes whose character covers the
/// byte `d` after it, and for each class and each `d` in 0..=3 the starts of characters in the
/// class that cover the byte `d` after it.
pub(in crate::code_rule_dsl) struct UcodeBlock {
    pub covers: [u64; 3],
    pub hits: Vec<[u64; 4]>,
}

/// One block of `ucode` before its shifts. `bytes` is the block and the 3 bytes after it;
/// `starts` are the block's character starts, which need the previous block's covers.
/// simdjson's ASCII path: a block with no byte of 0x80 or above has one-byte characters only.
pub(in crate::code_rule_dsl) fn ucode_block(bytes: &[u8], over: u64, cover_in: u64, classes: &[Membership]) -> (UcodeBlock, u64) {
    let high = bits(|l| bytes[l] >= 0x80);
    if high == 0 {
        // Code point = byte, length 1: nothing covers a later byte.
        let starts = over & !cover_in;
        let hits = classes.iter().map(|c| [starts & bits(|l| c.has(bytes[l] as u32)), 0, 0, 0]).collect();
        return (UcodeBlock { covers: [0; 3], hits }, starts);
    }
    let cont = |l: usize| (bytes[l] & 0xc0 == 0x80) as u32;
    let mut cp = [0u32; 64];
    let mut len = [1u8; 64];
    for l in 0..64 {
        let b = bytes[l] as u32;
        let n = 1 + (b >= 0xc0) as u32 + (b >= 0xe0) as u32 + (b >= 0xf0) as u32;
        let lead_ok = (b < 0x80 || (0xc0..=0xf7).contains(&b)) as u32;
        let c1 = cont(l + 1);
        let c2 = cont(l + 2);
        let c3 = cont(l + 3);
        let conts_ok = (n < 2 || c1 == 1) as u32 & (n < 3 || c2 == 1) as u32 & (n < 4 || c3 == 1) as u32;
        let ok = lead_ok & conts_ok;
        let init = [b, b & 0x1f, b & 0x0f, b & 0x07][(n - 1) as usize];
        let t1 = bytes[l + 1] as u32 & 0x3f;
        let t2 = bytes[l + 2] as u32 & 0x3f;
        let t3 = bytes[l + 3] as u32 & 0x3f;
        let v = [init, init << 6 | t1, init << 12 | t1 << 6 | t2, init << 18 | t1 << 12 | t2 << 6 | t3][(n - 1) as usize];
        let ascii = (b < 0x80) as u32;
        cp[l] = ok * v + (1 - ok) * (ascii * b + (1 - ascii) * 0xfffd);
        len[l] = (ok * n + (1 - ok)) as u8;
    }
    let covers = [bits(|l| len[l] > 1) & over, bits(|l| len[l] > 2) & over, bits(|l| len[l] > 3) & over];
    let contm = bits(|l| cont(l) == 1);
    // A continuation byte covered by an earlier character (from this block or the one before)
    // starts nothing.
    let covered = contm & (cover_in | prev(covers[0], 0, 1) | prev(covers[1], 0, 2) | prev(covers[2], 0, 3));
    let starts = over & !covered;
    let hits = classes
        .iter()
        .map(|c| {
            let h = starts & bits(|l| c.has(cp[l]));
            [h, h & bits(|l| len[l] > 1), h & bits(|l| len[l] > 2), h & bits(|l| len[l] > 3)]
        })
        .collect();
    (UcodeBlock { covers, hits }, starts)
}
