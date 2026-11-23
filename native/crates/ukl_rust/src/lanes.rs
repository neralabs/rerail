//! The relation reads at four lanes: each a function of registers, four rows or keys an operation, no call in it and
//! none per lane. They are `relation.h`'s reads (decision 0310's, written once in CLIF) as NEON code, field for field;
//! `tests` holds each to the compiled scalar read on every row of a relation of many chunks, a frozen base and a
//! delta over it. A relation (its descriptor `d`) and a column are the same in every lane, as a join step reads one
//! relation; rows, keys and hashes are each lane's.
//!
//! NEON has no gather: a lane's own address is one lane load (`ld1 {v.s}[i]`), four to a register.
#![allow(unsafe_op_in_unsafe_fn)]

use core::arch::aarch64::*;

/// Four 32-bit lanes: rows, counts, masks (all ones or zero).
pub type U = uint32x4_t;

/// Four 64-bit lanes: values, hashes, addresses; lanes 0 and 1, then 2 and 3.
#[derive(Clone, Copy)]
pub struct W(pub uint64x2_t, pub uint64x2_t);

impl W {
    #[inline(always)]
    pub unsafe fn splat(x: u64) -> W {
        W(vdupq_n_u64(x), vdupq_n_u64(x))
    }
    #[inline(always)]
    pub unsafe fn load(p: *const u64) -> W {
        W(vld1q_u64(p), vld1q_u64(p.add(2)))
    }
    #[inline(always)]
    pub unsafe fn store(self, p: *mut u64) {
        vst1q_u64(p, self.0);
        vst1q_u64(p.add(2), self.1);
    }
    #[inline(always)]
    pub unsafe fn add(self, o: W) -> W {
        W(vaddq_u64(self.0, o.0), vaddq_u64(self.1, o.1))
    }
    #[inline(always)]
    pub unsafe fn and(self, o: W) -> W {
        W(vandq_u64(self.0, o.0), vandq_u64(self.1, o.1))
    }
    #[inline(always)]
    pub unsafe fn xor(self, o: W) -> W {
        W(veorq_u64(self.0, o.0), veorq_u64(self.1, o.1))
    }
    /// The low and high halves of each lane, as four lanes each.
    #[inline(always)]
    pub unsafe fn halves(self) -> (U, U) {
        let (a, b) = (vreinterpretq_u32_u64(self.0), vreinterpretq_u32_u64(self.1));
        (vuzp1q_u32(a, b), vuzp2q_u32(a, b))
    }
    /// Lanes of `lo` below, `hi` above.
    #[inline(always)]
    pub unsafe fn join(lo: U, hi: U) -> W {
        W(vreinterpretq_u64_u32(vzip1q_u32(lo, hi)), vreinterpretq_u64_u32(vzip2q_u32(lo, hi)))
    }
    /// Each lane's low 32 bits.
    #[inline(always)]
    pub unsafe fn low(self) -> U {
        vmovn_high_u64(vmovn_u64(self.0), self.1)
    }
    /// Lanes where `m` is set from `a`, else from `self`.
    #[inline(always)]
    pub unsafe fn pick(self, m: U, a: W) -> W {
        let m = wide(m);
        W(vbslq_u64(m.0, a.0, self.0), vbslq_u64(m.1, a.1, self.1))
    }
    /// The lanes that are zero, as a mask of four.
    #[inline(always)]
    pub unsafe fn is_zero(self) -> U {
        W(vceqzq_u64(self.0), vceqzq_u64(self.1)).low()
    }
    #[inline(always)]
    pub unsafe fn lane(self, i: usize) -> u64 {
        let mut a = [0u64; 4];
        self.store(a.as_mut_ptr());
        a[i]
    }
}

/// `u`'s lanes zero-extended (a mask's stay all ones: sign-extended).
#[inline(always)]
pub unsafe fn widen(u: U) -> W {
    W(vmovl_u32(vget_low_u32(u)), vmovl_high_u32(u))
}
#[inline(always)]
unsafe fn wide(m: U) -> W {
    let s = vreinterpretq_s32_u32(m);
    W(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(s))), vreinterpretq_u64_s64(vmovl_high_s32(s)))
}
/// `u * k`, each lane's product whole.
#[inline(always)]
unsafe fn mul_wide(u: U, k: u32) -> W {
    W(vmull_n_u32(vget_low_u32(u), k), vmull_high_n_u32(u, k))
}
/// `p + u * k` as each lane's address.
#[inline(always)]
unsafe fn at(p: *const u8, u: U, k: u32) -> W {
    mul_wide(u, k).add(W::splat(p as u64))
}
#[inline(always)]
unsafe fn sel(m: U, a: U, b: U) -> U {
    vbslq_u32(m, a, b)
}

/// The word at each lane's address.
#[inline(always)]
pub unsafe fn gather32(a: W) -> U {
    let v = vdupq_n_u32(0);
    let v = vld1q_lane_u32::<0>(vgetq_lane_u64::<0>(a.0) as *const u32, v);
    let v = vld1q_lane_u32::<1>(vgetq_lane_u64::<1>(a.0) as *const u32, v);
    let v = vld1q_lane_u32::<2>(vgetq_lane_u64::<0>(a.1) as *const u32, v);
    vld1q_lane_u32::<3>(vgetq_lane_u64::<1>(a.1) as *const u32, v)
}
/// The double word at each lane's address.
#[inline(always)]
pub unsafe fn gather64(a: W) -> W {
    let z = vdupq_n_u64(0);
    let l = vld1q_lane_u64::<0>(vgetq_lane_u64::<0>(a.0) as *const u64, z);
    let l = vld1q_lane_u64::<1>(vgetq_lane_u64::<1>(a.0) as *const u64, l);
    let h = vld1q_lane_u64::<0>(vgetq_lane_u64::<0>(a.1) as *const u64, z);
    W(l, vld1q_lane_u64::<1>(vgetq_lane_u64::<1>(a.1) as *const u64, h))
}

#[inline(always)]
unsafe fn r64(p: *const u8, o: u64) -> u64 {
    p.wrapping_add(o as usize).cast::<u64>().read_unaligned()
}
#[inline(always)]
unsafe fn r32(p: *const u8, o: u64) -> u32 {
    p.wrapping_add(o as usize).cast::<u32>().read_unaligned()
}
/// The address an offset field at `p + o` names (its own place plus the offset), as the reads' `p + o + *(p + o)`.
#[inline(always)]
unsafe fn follow(p: *const u8, o: u64) -> *const u8 {
    p.wrapping_add((o as usize).wrapping_add(r64(p, o) as usize))
}

/// `x * k` in each 64-bit lane (NEON multiplies 32 bits at most): the low halves' whole product, then the cross
/// products a word up.
#[inline(always)]
unsafe fn mul64(x: W, k: u64) -> W {
    let (lo, hi) = x.halves();
    let (kl, kh) = (k as u32, (k >> 32) as u32);
    let cross = vaddq_u32(vmulq_n_u32(lo, kh), vmulq_n_u32(hi, kl));
    let c = widen(cross);
    mul_wide(lo, kl).add(W(vshlq_n_u64::<32>(c.0), vshlq_n_u64::<32>(c.1)))
}

/// `clif_rel_key_mix`: a key hash `h` mixed with value `v`.
#[inline(always)]
pub unsafe fn key_mix(h: W, v: W) -> W {
    let x = mul64(h.xor(v), 0x9E37_79B9_7F4A_7C15);
    W(veorq_u64(x.0, vshrq_n_u64::<29>(x.0)), veorq_u64(x.1, vshrq_n_u64::<29>(x.1)))
}

/// `clif_bloom_hash` of each lane's key, as its low and high halves (the probe's bits, then its word's choice).
#[inline(always)]
pub unsafe fn bloom_hash(key: W) -> (U, U) {
    let (lo, hi) = key.halves();
    let v6 = veorq_u32(lo, vmulq_n_u32(hi, 18446744072069020081u64 as u32));
    let v8 = veorq_u32(v6, vshrq_n_u32::<16>(v6));
    let v9 = vmulq_n_u32(v8, 18446744071661406827u64 as u32);
    let v11 = veorq_u32(v9, vshrq_n_u32::<13>(v9));
    let v12 = vmulq_n_u32(v11, 18446744072681074229u64 as u32);
    let v14 = veorq_u32(v12, vshrq_n_u32::<16>(v12));
    let v15 = vmulq_n_u32(v14, 668265263);
    (v14, veorq_u32(v15, vshrq_n_u32::<15>(v15)))
}

/// `clif_bloom_may` of filter `d` for hashes `(lo, hi)` (`bloom_hash`): the lanes whose key the filter may hold.
#[inline(always)]
pub unsafe fn bloom_may(d: *const u8, lo: U, hi: U) -> U {
    let n = r64(d, 8);
    if n == 0 {
        return vdupq_n_u32(!0);
    }
    let word = widen(hi).and(W::splat(n.wrapping_sub(1)));
    let words = follow(d, 0);
    let w = gather64(W(vshlq_n_u64::<3>(word.0), vshlq_n_u64::<3>(word.1)).add(W::splat(words as u64)));
    // Five bits of the word, each by six bits of the hash's low half.
    let one = vdupq_n_u64(1);
    let mut m = W(vdupq_n_u64(0), vdupq_n_u64(0));
    let mut b = lo;
    for _ in 0..5 {
        let s = widen(vandq_u32(b, vdupq_n_u32(63)));
        m = W(vorrq_u64(m.0, vshlq_u64(one, vreinterpretq_s64_u64(s.0))), vorrq_u64(m.1, vshlq_u64(one, vreinterpretq_s64_u64(s.1))));
        b = vshrq_n_u32::<6>(b);
    }
    W(vceqq_u64(vandq_u64(w.0, m.0), m.0), vceqq_u64(vandq_u64(w.1, m.1), m.1)).low()
}

/// `clif_rel_dead` of relation `d`: the lanes whose row is dead.
#[inline(always)]
pub unsafe fn rel_dead(d: *const u8, r: U) -> U {
    let n = r64(d, 128);
    let w = vshrq_n_u32::<6>(r);
    let inside = if n > u32::MAX as u64 { vdupq_n_u32(!0) } else { vcltq_u32(w, vdupq_n_u32(n as u32)) };
    // A lane past the bits reads word 0's place (the descriptor's, when there are none), and is masked.
    let w = vandq_u32(w, inside);
    let base = if n == 0 { d } else { follow(d, 120) };
    let word = gather64(at(base, w, 8));
    let s = widen(vandq_u32(r, vdupq_n_u32(63)));
    let neg = |x: uint64x2_t| vnegq_s64(vreinterpretq_s64_u64(x));
    let bit = W(vshlq_u64(word.0, neg(s.0)), vshlq_u64(word.1, neg(s.1))).and(W::splat(1));
    vandq_u32(vmvnq_u32(bit.is_zero()), inside)
}

/// Index `index` of relation `d`: its descriptor.
#[inline(always)]
unsafe fn index_of(d: *const u8, index: u32) -> *const u8 {
    follow(d, 136).wrapping_add(index as usize * 152)
}

/// `clif_rel_group` of relation `d`'s index `index` for key hashes `h`: each lane's group's first place in the index's
/// rows and its length (the read's `start << 32 | len`, as two registers).
#[inline(always)]
pub unsafe fn rel_group(d: *const u8, index: u32, h: W) -> (U, U) {
    let ix = index_of(d, index);
    if r64(ix, 40) == 0 {
        return (vdupq_n_u32(0), vdupq_n_u32(0));
    }
    let slot = h.and(W::splat(r64(ix, 24)));
    let a = W(vshlq_n_u64::<2>(slot.0), vshlq_n_u64::<2>(slot.1)).add(W::splat(follow(ix, 32) as u64));
    let lo = gather32(a);
    let hi = gather32(a.add(W::splat(4)));
    (lo, vsubq_u32(hi, lo))
}

/// `clif_rel_group_row` of relation `d`'s index `index`: the row at each lane's place `q` of its rows.
#[inline(always)]
pub unsafe fn rel_group_row(d: *const u8, index: u32, q: U) -> U {
    gather32(at(follow(index_of(d, index), 48), q, 4))
}

/// `clif_rel_len` of relation `d`.
#[inline(always)]
pub unsafe fn rel_len(d: *const u8) -> u32 {
    r32(d, 40)
}

/// Each lane's place in a column of chunks doubling from `1 << shift` rows: the chunk (`seg`) and the row in it, as
/// `clif_col_at` finds them.
#[inline(always)]
unsafe fn chunk(i: U, shift: U) -> (U, U) {
    let k1 = vaddq_u32(vshlq_u32(i, vnegq_s32(vreinterpretq_s32_u32(shift))), vdupq_n_u32(1));
    let s = vsubq_u32(vdupq_n_u32(31), vclzq_u32(k1));
    let one = vdupq_n_u32(1);
    let first = vshlq_u32(vsubq_u32(vshlq_u32(one, vreinterpretq_s32_u32(s)), one), vreinterpretq_s32_u32(shift));
    let off = vsubq_u32(i, first);
    // The chunk's half: `shift + s - 2` bits (none, below two: the whole offset is the row).
    let e = vsubq_s32(vreinterpretq_s32_u32(vaddq_u32(shift, s)), vdupq_n_s32(2));
    let e = vmaxq_s32(e, vdupq_n_s32(-1));
    let e = vbslq_s32(vcltzq_s32(e), vdupq_n_s32(32), e);
    let seg = vaddq_u32(vshlq_n_u32::<2>(s), vshlq_u32(off, vnegq_s32(e)));
    let within = vandq_u32(off, vsubq_u32(vshlq_u32(one, e), one));
    (seg, within)
}

/// A 4-byte column's document high words (`doc_high`, at `tab`, `n` of them): each lane's row's, by a search of four
/// in step, each lane's bounds its own.
#[inline(always)]
unsafe fn doc_high(tab: W, n: U, i: U) -> U {
    let mut lo = vdupq_n_u32(0);
    let mut hi = n;
    loop {
        let go = vcltq_u32(lo, hi);
        if vmaxvq_u32(go) == 0 {
            break;
        }
        let mid = vaddq_u32(lo, vshrq_n_u32::<1>(vsubq_u32(hi, lo)));
        let key = gather32(mul_wide(mid, 8).add(tab).add(W::splat(8)));
        let le = vandq_u32(go, vcleq_u32(key, i));
        let gt = vbicq_u32(go, le);
        lo = sel(le, vaddq_u32(mid, vdupq_n_u32(1)), lo);
        hi = sel(gt, mid, hi);
    }
    gather32(mul_wide(lo, 8).add(tab).add(W::splat(4)))
}

/// `clif_col_get` of column `c` (the same in every lane) of chunks from `1 << shift` rows: row `i`'s value.
#[inline(always)]
pub unsafe fn col_get(c: *const u8, shift: u32, i: U) -> W {
    let (seg, within) = chunk(i, vdupq_n_u32(shift));
    let width = r32(c, 0);
    let chunks = gather64(at(c.wrapping_add(8), seg, 8));
    let a = mul_wide(within, width).add(chunks).add(W::splat(c as u64));
    if width != 4 {
        return gather64(a);
    }
    let low = gather32(a);
    let dh = r32(c, 4);
    if dh == 0 {
        return widen(low);
    }
    let tab = c.wrapping_add((dh as i32 as i64 as usize).wrapping_mul(8));
    W::join(low, doc_high(W::splat(tab as u64), vdupq_n_u32(r32(tab, 0)), i))
}

/// `clif_col_get` of each lane's own column `c`, chunks from `1 << shift` rows.
#[inline(always)]
unsafe fn col_get_each(c: W, shift: U, i: U) -> W {
    let (seg, within) = chunk(i, shift);
    let width = gather32(c);
    let chunks = gather64(mul_wide(seg, 8).add(c).add(W::splat(8)));
    let a = W(vmull_u32(vget_low_u32(within), vget_low_u32(width)), vmull_high_u32(within, width)).add(chunks).add(c);
    let four = vceqq_u32(width, vdupq_n_u32(4));
    let low = gather32(a);
    // The high words: an 8-byte lane's next word, a 4-byte lane's document's (none: zero).
    let high8 = gather32(a.add(W::splat(4)).pick(four, a));
    let dh = gather32(c.add(W::splat(4)));
    let searched = vandq_u32(four, vmvnq_u32(vceqzq_u32(dh)));
    let high4 = if vmaxvq_u32(searched) == 0 {
        vdupq_n_u32(0)
    } else {
        let off = vreinterpretq_s32_u32(vandq_u32(dh, searched));
        let tab = W(vreinterpretq_u64_s64(vshlq_n_s64::<3>(vmovl_s32(vget_low_s32(off)))), vreinterpretq_u64_s64(vshlq_n_s64::<3>(vmovl_high_s32(off)))).add(c);
        let n = vandq_u32(gather32(tab), searched);
        vandq_u32(doc_high(tab, n, i), searched)
    };
    W::join(low, sel(four, high4, high8))
}

/// A relation's rows (`rows`, at `d + 40` of its descriptor): column `c`'s directory.
#[inline(always)]
unsafe fn rows_col(rows: *const u8, c: u32) -> *const u8 {
    follow(rows, 24).wrapping_add(c as usize * 776)
}

/// `clif_rel_get` of relation `d`: column `c` of each lane's row `r` (a delta's, its base's under it, by the chain of
/// blocks each lane walks to its row's).
#[inline(always)]
pub unsafe fn rel_get(d: *const u8, r: U, c: u32) -> W {
    if r64(d, 192) == 0 {
        let rows = d.wrapping_add(40);
        return col_get(rows_col(rows, c), r32(rows, 8), vsubq_u32(r, vdupq_n_u32(r32(d, 8))));
    }
    // Each lane walks from the newest block to the one its row is in: down while its row is below the block's first.
    let mut b = W::splat(d as u64);
    let mut first;
    loop {
        first = gather32(b.add(W::splat(8)));
        let next = gather64(b.add(W::splat(192)));
        let go = vandq_u32(vcltq_u32(r, first), vmvnq_u32(next.is_zero()));
        if vmaxvq_u32(go) == 0 {
            break;
        }
        b = b.pick(go, b.add(next));
    }
    let rows = b.add(W::splat(40));
    let shift = gather32(rows.add(W::splat(8)));
    let cols = gather64(rows.add(W::splat(24))).add(rows).add(W::splat(24 + c as u64 * 776));
    col_get_each(cols, shift, vsubq_u32(r, first))
}

#[cfg(test)]
mod tests;
