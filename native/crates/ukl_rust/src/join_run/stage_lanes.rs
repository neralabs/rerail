//! A relational stage's count and scatter at `N` lanes (decision 0567): `N` bindings an operation, each step of their
//! candidate walks one vector read (`crate::lanes`), their keys, tests, binds and row program vectors of the lanes'
//! values. A lane's walk is its own length; the walk runs to the longest, each lane masked past its own. The stages it
//! takes (`runs_at_lanes`) probe a relation by keys (or read all its rows): no view, text or aggregate.
//!
//! Each lane's result is `stage_count_one`'s and `stage_scatter_one`'s for its binding, row for row (the tests of
//! `setfix::device` run every stage through both).

use super::*;
use crate::lanes as ln;
use rerail_simd::prelude::*;
use rerail_simd::{LaneCount, SupportedLaneCount};

/// The lanes a step: 8 (four NEON registers of 64-bit lanes).
pub(super) const LANES: usize = 8;

type I<const N: usize> = Simd<i64, N>;
type U<const N: usize> = Simd<u64, N>;
type R<const N: usize> = Simd<u32, N>;
type W<const N: usize> = Mask<i64, N>;

/// Whether stage `s` runs at lanes: a join, a semi-join or an anti-join of a relation in its block, probed by its keys
/// through its index (or every row), with no view, text, aggregate or text test.
pub(super) fn runs_at_lanes(s: &Stage) -> bool {
    s.v_has == 0 && s.r_view == 0 && s.v_parent == 0 && matches!(s.mode, 0 | 1 | 2) && s.nkeys <= 8
}

/// Bindings `t..t + n` of a stage (`n` at most `N`): its reads at `N` lanes.
struct Lanes<'a, const N: usize> {
    st: &'a St<'a>,
    /// The relation's descriptor (`r + r_at`), its dead rows' (`r + r_dead`).
    d: *const u8,
    dead: *const u8,
    t: u32,
    n: u32,
}

#[inline(always)]
fn si<const N: usize>(v: i64) -> I<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    Simd::splat(v)
}

#[inline(always)]
fn sr<const N: usize>(v: u32) -> R<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    Simd::splat(v)
}

/// The first `n` values at `p` (`n` at most `N`), 0 past them: one load where all `N` are there.
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

/// `lane_of`: the low `w` bits, sign-extended (16, 32), or the whole value.
#[inline(always)]
fn lanes_of<const N: usize>(v: I<N>, w: u32) -> I<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    match w {
        16 => (v << si(48)) >> si(48),
        32 => (v << si(32)) >> si(32),
        _ => v,
    }
}

/// Lanes as 0 or 1.
#[inline(always)]
fn ones<const N: usize>(m: W<N>) -> I<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    m.select(si(1), si(0))
}

impl<const N: usize> Lanes<'_, N>
where
    LaneCount<N>: SupportedLaneCount,
{
    /// The lanes that hold a binding.
    fn held(&self) -> W<N> {
        Mask::from_bitmask(if self.n as usize >= N { u64::MAX } else { (1u64 << self.n) - 1 })
    }

    unsafe fn slot(&self, k: u32) -> I<N> {
        load(colp(self.st.l, k, self.st.s.cap_in, self.t), self.n)
    }

    unsafe fn rget(&self, row: R<N>, c: u32) -> I<N> {
        ln::rel_get(self.d, row, c).cast::<i64>()
    }

    unsafe fn colval(&self, k: u32, row: R<N>) -> I<N> {
        let (s, k) = (self.st.s, k as usize);
        let v = if s.col_kind[k] == 0 { self.rget(row, s.col_idx[k]) } else { self.slot(s.col_idx[k]) };
        if s.col_doc[k] != 0 { v >> si(32) } else { v }
    }

    unsafe fn parval(&self, k: u32) -> I<N> {
        let (s, k) = (self.st.s, k as usize);
        let v = if s.par_kind[k] == 0 { si(s.par_const[k]) } else { self.slot(s.par_slot[k]) };
        if s.par_doc[k] != 0 { v >> si(32) } else { v }
    }

    unsafe fn key(&self, k: u32) -> I<N> {
        let (s, k) = (self.st.s, k as usize);
        if s.key_kind[k] == 1 { self.slot(s.key_slot[k]) } else { si(s.key_const[k]) }
    }

    /// Arithmetic bind `j`'s values and the lanes where they are defined (no overflow, no zero divisor), as `arith`.
    /// A product's, a quotient's and a remainder's overflow has no vector test on NEON: their lanes each.
    unsafe fn arith(&self, j: u32, row: R<N>) -> (I<N>, W<N>) {
        let a = &self.st.s.arith[j as usize];
        let x = if a[3] & 1 != 0 { self.parval(a[1]) } else { self.colval(a[1], row) };
        let y = if a[3] & 2 != 0 { self.parval(a[2]) } else { self.colval(a[2], row) };
        let each = |f: fn(i64, i64) -> Option<i64>| {
            let (xs, ys) = (x.to_array(), y.to_array());
            let r: [Option<i64>; N] = core::array::from_fn(|i| f(xs[i], ys[i]));
            (Simd::from_array(r.map(|v| v.unwrap_or(0))), Mask::from_array(r.map(|v| v.is_some())))
        };
        match a[0] {
            0 => {
                let r = x + y;
                (r, !((x ^ r) & (y ^ r)).is_negative())
            }
            1 => {
                let r = x - y;
                (r, !((x ^ y) & (x ^ r)).is_negative())
            }
            2 => each(i64::checked_mul),
            3 => each(i64::checked_div),
            4 => each(i64::checked_rem),
            5 => (x.simd_max(y), Mask::splat(true)),
            _ => (x.simd_min(y), Mask::splat(true)),
        }
    }

    unsafe fn defined(&self, row: R<N>) -> W<N> {
        let s = self.st.s;
        let mut ok = Mask::splat(true);
        for b in 0..s.nbinds.min(16) as usize {
            if s.bind_kind[b] == 3 {
                ok &= self.arith(s.bind_idx[b], row).1;
            }
        }
        ok
    }

    /// The row program: bit x of each lane's value is operation x's.
    unsafe fn eval(&self, row: R<N>) -> U<N> {
        let s = self.st.s;
        let mut v = U::splat(0);
        for x in 0..s.nops.min(64) {
            let o = &s.ops[x as usize];
            let bit = |k: u32| (v >> U::splat(u64::from(k & 63))) & U::splat(1);
            let b: U<N> = match o[0] {
                0 => {
                    let w = (o[3] >> 8) & 255;
                    let a = lanes_of(self.colval(o[1], row), w);
                    let c = if o[2] & 0x8000_0000 != 0 { self.parval(o[2] & 0xffff) } else { self.colval(o[2], row) };
                    let c = lanes_of(c, w);
                    let m = match o[3] & 255 {
                        0 => a.simd_eq(c),
                        1 => a.simd_ne(c),
                        2 => a.simd_lt(c),
                        _ => a.simd_le(c),
                    };
                    m.select(U::splat(1), U::splat(0))
                }
                1 => U::splat(u64::from(o[1] != 0)),
                2 => bit(o[1]) ^ U::splat(1),
                3 => bit(o[1]) & bit(o[2]),
                4 => bit(o[1]) | bit(o[2]),
                5 => bit(o[1]) ^ bit(o[2]),
                _ => bit(o[1]) & !bit(o[2]) & U::splat(1),
            };
            v |= b << U::splat(u64::from(x));
        }
        v
    }

    fn out_bit(&self, v: U<N>, k: u32) -> W<N> {
        ((v >> U::splat(u64::from(self.st.s.outputs[k as usize] & 63))) & U::splat(1)).simd_ne(U::splat(0))
    }

    /// The lanes whose row passes the stage's last output (or all, with none).
    unsafe fn passes(&self, row: R<N>) -> W<N> {
        let s = self.st.s;
        let ok = self.defined(row);
        if s.last == NONE {
            return ok;
        }
        ok & self.out_bit(self.eval(row), s.last)
    }

    /// The bindings' candidate walks: `f` of each step's rows and the lanes whose row matches (the binding's keys, the
    /// relation's tests, a live row), its result the lanes that go on. `go`: the lanes walking.
    unsafe fn walk(&self, f: *const u8, mut go: W<N>, mut each: impl FnMut(R<N>, W<N>) -> W<N>) {
        let s = self.st.s;
        let nk = s.nkeys.min(8) as usize;
        let mut kv = [si::<N>(0); 8];
        for (k, v) in kv.iter_mut().enumerate().take(nk) {
            *v = self.key(k as u32);
        }
        if s.screen != 0 && s.screen <= s.nkeys && s.screen <= 8 {
            let (lo, hi) = ln::bloom_hash(kv[s.screen as usize - 1].cast::<u64>());
            go &= ln::bloom_may(f.add(s.screen_at as usize), lo, hi).cast::<i64>();
        }
        let (start, len) = if nk == 0 {
            (sr(s.r_from), sr(self.st.rows()))
        } else {
            let mut h = U::splat(KEY_SEED);
            for v in &kv[..nk] {
                h = ln::key_mix(h, v.cast::<u64>());
            }
            ln::rel_group(self.d, s.r_index, h)
        };
        let len = go.cast::<i32>().select(len, sr(0));
        for i in 0..len.reduce_max() {
            let on = go & sr::<N>(i).simd_lt(len).cast::<i64>();
            let Some(first) = on.first_set() else { break };
            // A lane past its walk reads the first walking lane's row: every lane's read is of a row there is.
            let q = start + sr(i);
            let q = on.cast::<i32>().select(q, sr(q[first]));
            let row = if nk == 0 { q } else { ln::rel_group_row(self.d, s.r_index, q) };
            let mut m = on;
            if s.r_dead != 0 {
                m &= !ln::rel_dead(self.dead, row).cast::<i64>();
            }
            for (k, v) in kv[..nk].iter().enumerate() {
                m &= self.rget(row, s.key_rcol[k]).simd_eq(*v);
            }
            for k in 0..s.ntests.min(8) as usize {
                m &= self.rget(row, s.test_a[k]).simd_eq(self.rget(row, s.test_b[k]));
            }
            go &= each(row, m);
        }
    }

    /// Each lane's row's slots and binds out at its place `a` of `o`, the lanes `w`.
    unsafe fn write_rows(&self, row: R<N>, o: *mut i64, a: R<N>, w: W<N>) {
        let s = self.st.s;
        let on = w.cast::<isize>();
        let at = |c: u32| Simd::<*mut i64, N>::splat(o.add(c as usize * s.cap_out as usize)).wrapping_add(a.cast::<usize>());
        for k in 0..s.nslots {
            self.slot(k).scatter_select_ptr(at(k), on);
        }
        for b in 0..s.nbinds.min(16) as usize {
            let i = s.bind_idx[b];
            let v = match s.bind_kind[b] {
                3 => {
                    let (v, ok) = self.arith(i, row);
                    ok.select(v, si(0))
                }
                0 => self.colval(i, row),
                1 => self.parval(i),
                _ => self.rget(row, i),
            };
            v.scatter_select_ptr(at(s.bind_slot[b]), on);
        }
    }
}

/// Bindings `t0..t1` of a relational stage (`runs_at_lanes`), all below its input's rows: each one's count of rows
/// out (a semi-join's and an anti-join's 0 or 1) into `counts`, its rows matched and passed into `acc`.
pub(super) unsafe fn stage_count_lanes<const N: usize>(st: &St, j: &Job, counts: *mut u32, t0: u32, t1: u32, acc: &mut [u32; 17])
where
    LaneCount<N>: SupportedLaneCount,
{
    let s = st.s;
    let d = (st.r as *const u8).wrapping_add(s.r_at as usize);
    let dead = (st.r as *const u8).wrapping_add(s.r_dead as usize);
    let passed = s.npassed.min(16) as usize;
    let exists = s.mode != 0;
    let mut t = t0;
    while t < t1 {
        let lanes = Lanes::<N> { st, d, dead, t, n: (t1 - t).min(N as u32) };
        let mut c = si::<N>(0);
        if exists {
            let mut found: W<N> = Mask::splat(false);
            lanes.walk(j.f, lanes.held(), |_, m| {
                found |= m;
                !m
            });
            c = ones(if s.mode == 2 { !found } else { found });
        } else {
            let (mut p0, mut q) = (si::<N>(0), [si::<N>(0); 16]);
            lanes.walk(j.f, lanes.held(), |row, m| {
                if !m.any() {
                    return Mask::splat(true);
                }
                p0 += ones(m);
                let ok = m & lanes.defined(row);
                let v = if s.nops > 0 { lanes.eval(row) } else { U::splat(0) };
                for (q, &bit) in q.iter_mut().zip(&s.passed).take(passed) {
                    *q += ones(if bit == NONE { ok } else { ok & lanes.out_bit(v, bit) });
                }
                c += ones(if s.last == NONE { ok } else { ok & lanes.out_bit(v, s.last) });
                Mask::splat(true)
            });
            acc[0] += p0.reduce_sum() as u32;
            for (a, q) in acc[1..].iter_mut().zip(&q).take(passed) {
                *a += q.reduce_sum() as u32;
            }
        }
        let c: [u32; N] = c.cast::<u32>().to_array();
        core::ptr::copy_nonoverlapping(c.as_ptr(), counts.add(t as usize), lanes.n as usize);
        t += lanes.n;
    }
}

/// Bindings `t0..t1` of a join (`runs_at_lanes`, mode 0), all below its input's rows: each one's rows out at its
/// offset (`offsets`, less `o0`), below the stage's room.
pub(super) unsafe fn stage_scatter_lanes<const N: usize>(st: &St, j: &Job, offsets: *const u32, o: *mut i64, t0: u32, t1: u32)
where
    LaneCount<N>: SupportedLaneCount,
{
    let s = st.s;
    let d = (st.r as *const u8).wrapping_add(s.r_at as usize);
    let dead = (st.r as *const u8).wrapping_add(s.r_dead as usize);
    let o0 = *offsets;
    let cap = sr::<N>(s.cap_out);
    let mut t = t0;
    while t < t1 {
        let lanes = Lanes::<N> { st, d, dead, t, n: (t1 - t).min(N as u32) };
        let mut off = [0u32; N];
        core::ptr::copy_nonoverlapping(offsets.add(t as usize), off.as_mut_ptr(), lanes.n as usize);
        let mut a = Simd::from_array(off) - sr(o0);
        let go = lanes.held() & a.simd_lt(cap).cast::<i64>();
        lanes.walk(j.f, go, |row, m| {
            if !m.any() {
                return Mask::splat(true);
            }
            let w = m & lanes.passes(row);
            lanes.write_rows(row, o, a, w);
            a += w.cast::<i32>().select(sr(1), sr(0));
            a.simd_lt(cap).cast::<i64>()
        });
        t += lanes.n;
    }
}
