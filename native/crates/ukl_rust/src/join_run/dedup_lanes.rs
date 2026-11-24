//! A dedup's and an index's per-row work at `N` lanes (decision 0567): `N` rows' hashes an operation (their columns
//! `N` consecutive values a load, `crate::lanes::key_mix`), their probes of a table's places gathers and compares
//! of `N` places, each lane's probe masked past its end. A claim's compare-and-swap is each lane's (no CPU has a
//! vector one): its places come from the lanes' hashes.
//!
//! Each lane's result is `dedup_mark_held`'s, `dedup_claim_one`'s, `dedup_place`'s and `ix_key`'s for its row.

use super::*;
use crate::lanes as ln;
use rerail_simd::prelude::*;
use rerail_simd::{LaneCount, SupportedLaneCount};

type I<const N: usize> = Simd<i64, N>;
type U<const N: usize> = Simd<u64, N>;
type R<const N: usize> = Simd<u32, N>;
type W<const N: usize> = Mask<i32, N>;

#[inline(always)]
fn sr<const N: usize>(v: u32) -> R<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    Simd::splat(v)
}

/// The first `n` values at `p` (`n` at most `N`), 0 past them.
#[inline(always)]
unsafe fn load<const N: usize>(p: *const i64, n: u32) -> I<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    if n as usize >= N {
        return Simd::from_array((p as *const [i64; N]).read_unaligned());
    }
    let mut a = [0i64; N];
    core::ptr::copy_nonoverlapping(p, a.as_mut_ptr(), n as usize);
    Simd::from_array(a)
}

/// The lanes' numbers.
#[inline(always)]
fn iota<const N: usize>() -> R<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    let mut v = [0u32; N];
    for (i, v) in v.iter_mut().enumerate() {
        *v = i as u32;
    }
    Simd::from_array(v)
}

/// The lanes `0..n`.
fn first<const N: usize>(n: u32) -> W<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    Mask::from_bitmask(if n as usize >= N { u64::MAX } else { (1u64 << n) - 1 })
}

/// `N` rows `x..x + n` of a dedup's table: each column's values, and their hashes (`row_hash`, unmasked).
struct Rows<const N: usize>
where
    LaneCount<N>: SupportedLaneCount,
{
    v: [I<N>; 16],
    h: R<N>,
}

impl<const N: usize> Rows<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    unsafe fn of(d: &Dedup, t: *const i64, x: u32, n: u32) -> Self {
        let mut v = [I::splat(0); 16];
        let mut h = U::splat(KEY_SEED);
        for (c, v) in v.iter_mut().enumerate().take(d.ncols.min(16) as usize) {
            *v = load(colp(t, d.cols[c], d.cap, x), n);
            h = ln::key_mix(h, v.cast::<u64>());
        }
        Rows { v, h: h.cast::<u32>() }
    }

    /// The lanes whose row is table row `y`'s (each lane's own `y`).
    unsafe fn same(&self, d: &Dedup, t: *const i64, y: R<N>, on: W<N>) -> W<N> {
        // Every lane's `y` is a row of the table (a lane off reads its own).
        let mut m = on;
        for c in 0..d.ncols.min(16) as usize {
            m &= ln::gather64_ix(t.add(d.cols[c] as usize * d.cap as usize).cast::<u8>(), y, 3).cast::<i64>().simd_eq(self.v[c]).cast::<i32>();
        }
        m
    }

    /// The lanes whose row the relation holds (`dedup_held`): each lane's walk of the map from its hash's place.
    unsafe fn held(&self, d: &Dedup, rel: *const u8, hb: *const u8, on: W<N>) -> W<N> {
        let m = hb.add(d.h_at as usize) as *const u32;
        let rd = rel.wrapping_add(d.r_at as usize);
        let (mut at, mut go, mut held) = (self.h & sr(d.hmask), on, Mask::splat(false));
        for _ in 0..=d.hmask {
            if !go.any() {
                break;
            }
            // Each lane's place, walking or not, is one of the map's.
            let y = go.select(ln::gather32_ix(m.cast::<u8>(), at * sr(3), 2), sr(NONE));
            go &= y.simd_ne(sr(NONE));
            let Some(f) = go.first_set() else { break };
            // A lane done reads the first walking lane's row: every read is of a row there is.
            let y = go.select(y, sr(y[f]));
            let row = ln::rel_at(rd, y);
            let mut same = go;
            for c in 0..d.ncols.min(16) as usize {
                same &= row.get(c as u32).cast::<i64>().simd_eq(self.v[c]).cast::<i32>();
            }
            held |= same;
            go &= !same;
            at = (at + sr(1)) & sr(d.hmask);
        }
        held
    }
}

/// Rows `x0..x1` of a dedup (all below its rows): each kept (1) where it is the first of its rows in the table's
/// places and, less a relation's (`Dedup::seen`), the relation holds no equal row.
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn dedup_mark_lanes<const N: usize>(t: *const i64, p: *const u32, counts: *mut u32, rel: *const u8, hb: *const u8, d: &Dedup, mask: u32, x0: u32, x1: u32)
where
    LaneCount<N>: SupportedLaneCount,
{
    let mut x = x0;
    while x < x1 {
        let n = (x1 - x).min(N as u32);
        let rows = Rows::<N>::of(d, t, x, n);
        let me = sr::<N>(x) + iota::<N>();
        let (mut at, mut go, mut keep) = (rows.h & sr(mask), first::<N>(n), Mask::splat(false));
        for _ in 0..=mask {
            if !go.any() {
                break;
            }
            let y = go.select(ln::gather32_ix(p.cast::<u8>(), at, 2), sr(NONE));
            go &= y.simd_ne(sr(NONE));
            let mine = go & y.simd_eq(me);
            keep |= mine;
            go &= !mine;
            // Another row's place: the walk ends where it is this row's (an earlier equal row holds it).
            go &= !rows.same(d, t, go.select(y, me), go);
            at = (at + sr(1)) & sr(mask);
        }
        if d.seen != 0 && keep.any() {
            keep &= !rows.held(d, rel, hb, keep);
        }
        let c: [u32; N] = keep.select(sr(1), sr(0)).to_array();
        core::ptr::copy_nonoverlapping(c.as_ptr(), counts.add(x as usize), n as usize);
        x += n;
    }
}

/// Rows `x0..x1` of a dedup (all below its rows) claim their places: each lane's compare-and-swap from its hash's
/// place, as `dedup_claim_one`.
pub(super) unsafe fn dedup_claim_lanes<const N: usize>(t: *const i64, p: *mut u32, d: &Dedup, mask: u32, x0: u32, x1: u32)
where
    LaneCount<N>: SupportedLaneCount,
{
    let mut x = x0;
    while x < x1 {
        let n = (x1 - x).min(N as u32);
        let at0 = (Rows::<N>::of(d, t, x, n).h & sr(mask)).to_array();
        for (i, &a0) in at0.iter().enumerate().take(n as usize) {
            let me = x + i as u32;
            let mut a = a0;
            for _ in 0..=mask {
                let place = at(p.add(a as usize));
                match place.compare_exchange(NONE, me, SeqCst, SeqCst) {
                    Ok(_) => break,
                    Err(seen) if same_row(d, t, seen, me) => {
                        place.fetch_min(me, SeqCst);
                        break;
                    }
                    Err(_) => a = (a + 1) & mask,
                }
            }
        }
        x += n;
    }
}

/// Kept rows `x..x + n` of a dedup placed in its relation's map (`Dedup::seen`) as rows `id..id + n`: each lane's
/// compare-and-swap from its hash's place, as `dedup_place`.
pub(super) unsafe fn dedup_place_lanes<const N: usize>(t: *const i64, hb: *mut u8, d: &Dedup, x: u32, n: u32, id: u32)
where
    LaneCount<N>: SupportedLaneCount,
{
    let m = hb.add(d.h_at as usize) as *mut u32;
    let mut k = 0;
    while k < n {
        let c = (n - k).min(N as u32);
        let at0 = (Rows::<N>::of(d, t, x + k, c).h & sr(d.hmask)).to_array();
        for (i, &a0) in at0.iter().enumerate().take(c as usize) {
            let mut a = a0;
            for _ in 0..=d.hmask {
                if at(m.add(3 * a as usize)).compare_exchange(NONE, id + k + i as u32, SeqCst, SeqCst).is_ok() {
                    break;
                }
                a = (a + 1) & d.hmask;
            }
        }
        k += c;
    }
}

/// Rows `t0..t1` of an index's relation (rows in its block, no view): each one's key hash, masked, plus `kb`, at
/// `eb + t` of `out`, as `ix_key`'s.
pub(super) unsafe fn ix_keys_lanes<const N: usize>(bb: *const u8, out: *mut u32, p: &IxP, kb: u32, eb: u32, t0: u32, t1: u32)
where
    LaneCount<N>: SupportedLaneCount,
{
    let d = bb.wrapping_add(p.at as usize);
    let mut t = t0;
    while t < t1 {
        let n = (t1 - t).min(N as u32);
        // Lanes past the rows read the first one's: every read is of a row there is.
        let row = first::<N>(n).select(sr(p.from.wrapping_add(t)) + iota::<N>(), sr(p.from.wrapping_add(t)));
        let at = ln::rel_at(d, row);
        let mut h = U::splat(KEY_SEED);
        for i in 0..(p.ncols & !IX_LIVE).min(8) as usize {
            h = ln::key_mix(h, at.get(p.cols[i]));
        }
        let k: [u32; N] = ((h & U::splat(p.mask)).cast::<u32>() + sr(kb)).to_array();
        core::ptr::copy_nonoverlapping(k.as_ptr(), out.add((eb + t) as usize), n as usize);
        t += n;
    }
}
