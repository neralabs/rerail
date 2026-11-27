//! A relational stage's count and scatter at `N` lanes (decision 0567), as a GPU's threads expand a join's work.
//!
//! A tile of bindings (`TILE`) first finds its groups `N` bindings an operation: their keys read from their slots'
//! columns (one load each), hashed, screened and looked up in the relation's index together. Their groups' places
//! are then expanded into one stream of candidates (`CANDS` a chunk), every lane a candidate of some binding, so no
//! lane waits on another's walk; consecutive candidates of a group are consecutive places of the index's rows, read
//! with one load. Each `N` candidates' rows are found once in their relation, each column read once, their keys,
//! tests, binds and row program vectors of the lanes' values. A count is each binding's sum of its candidates'
//! verdicts (prefix sums over the stream); a scatter writes the rows that pass in their order, a stream compaction
//! from the range's first offset (each binding's rows follow the one before's: its offset is their scan). The
//! stages it takes (`runs_at_lanes`) probe a relation by keys (or read all its rows): no view, text or aggregate.
//!
//! Each binding's result is `stage_count_one`'s and `stage_scatter_one`'s, row for row (the tests of
//! `setfix::device` run every stage through both).

use super::*;
use crate::lanes as ln;
use core::cell::{Cell, UnsafeCell};
use core::mem::MaybeUninit;
use rerail_simd::prelude::*;
use rerail_simd::{LaneCount, SupportedLaneCount};

/// The lanes a step: 8 (four NEON registers of 64-bit lanes).
pub(super) const LANES: usize = 8;

/// Bindings a tile, candidates a chunk; `SLACK`: room past each for a last step's whole vector (`N` at most 64).
const TILE: usize = 256;
const CANDS: usize = 512;
const SLACK: usize = 64;

type I<const N: usize> = Simd<i64, N>;
type U<const N: usize> = Simd<u64, N>;
type R<const N: usize> = Simd<u32, N>;
type W<const N: usize> = Mask<i64, N>;

/// Whether stage `s` runs at lanes: a join, a semi-join or an anti-join of a relation in its block, probed by its keys
/// through its index (or every row), with no view, text, aggregate or text test.
pub(super) fn runs_at_lanes(s: &Stage) -> bool {
    s.v_has == 0 && s.r_view == 0 && s.v_parent == 0 && s.mode <= 2 && s.nkeys <= 8 && s.nslots <= 16
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

/// The lanes' numbers.
#[inline(always)]
fn lane<const N: usize>() -> R<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    let mut v = [0u32; N];
    for (i, v) in v.iter_mut().enumerate() {
        *v = i as u32;
    }
    Simd::from_array(v)
}

/// Each lane's count of the set lanes of `bits` below it (an exclusive prefix sum of one bit a lane).
#[inline(always)]
fn ranks<const N: usize>(bits: u64) -> R<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    let below = (sr::<N>(1) << lane::<N>()) - sr(1);
    (sr::<N>(bits as u32) & below).count_ones()
}

/// `N` words at `p`.
#[inline(always)]
unsafe fn ld<const N: usize>(p: *const u32) -> R<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    Simd::from_array((p as *const [u32; N]).read_unaligned())
}

/// `v` as `N` words at `p`.
#[inline(always)]
unsafe fn st<const N: usize>(p: *mut u32, v: R<N>)
where
    LaneCount<N>: SupportedLaneCount,
{
    (p as *mut [u32; N]).write_unaligned(v.to_array());
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

/// A stage's reads at `N` lanes.
struct Lanes<'a> {
    st: &'a St<'a>,
    /// The relation's descriptor (`r + r_at`), its dead rows' (`r + r_dead`).
    d: *const u8,
    dead: *const u8,
    /// The block its far base lies in (`far_block`).
    far: *const u8,
}

/// Each lane's binding (`t`), its slots read once: a slot's values at its first use, one load where the lanes are `N`
/// consecutive bindings (`run`), else a gather.
struct BindAt<const N: usize>
where
    LaneCount<N>: SupportedLaneCount,
{
    t: R<N>,
    run: bool,
    l: *const i64,
    cap: u32,
    have: Cell<u32>,
    v: UnsafeCell<[MaybeUninit<I<N>>; 16]>,
}

impl<const N: usize> BindAt<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    #[inline(always)]
    fn new(l: &Lanes, t: R<N>, run: bool) -> Self {
        BindAt { t, run, l: l.st.l, cap: l.st.s.cap_in, have: Cell::new(0), v: UnsafeCell::new([MaybeUninit::uninit(); 16]) }
    }

    /// Slot `k` (below 16) of each lane's binding.
    #[inline(always)]
    fn slot(&self, k: u32) -> I<N> {
        let k = k & 15;
        // SAFETY: the bindings' own cache, a slot's place written before its bit is set; each lane's binding below
        // the stage's input rows, its slots in their columns.
        unsafe {
            let v = &mut *self.v.get();
            if self.have.get() >> k & 1 != 0 {
                return v[k as usize].assume_init();
            }
            let x = if self.run {
                Simd::from_array((colp(self.l, k, self.cap, self.t[0]) as *const [i64; N]).read_unaligned())
            } else {
                ln::gather64_ix(colp(self.l, k, self.cap, 0).cast::<u8>(), self.t, 3).cast::<i64>()
            };
            v[k as usize].write(x);
            self.have.set(self.have.get() | 1 << k);
            x
        }
    }
}

/// A step's rows, found once in their relation (`RowAt`), each column read once.
struct Row<const N: usize>
where
    LaneCount<N>: SupportedLaneCount,
{
    at: ln::RowAt<N>,
    have: Cell<u32>,
    v: UnsafeCell<[MaybeUninit<I<N>>; 16]>,
}

impl<const N: usize> Row<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    #[inline(always)]
    unsafe fn new(d: *const u8, id: R<N>, far: *const u8, sealed: ln::Sealed) -> Self {
        Row { at: ln::rel_at(d, id, far, sealed), have: Cell::new(0), v: UnsafeCell::new([MaybeUninit::uninit(); 16]) }
    }

    /// Column `c` of each lane's row.
    #[inline(always)]
    fn get(&self, c: u32) -> I<N> {
        // SAFETY: the row's own cache, a column's place written before its bit is set; each lane's row one the
        // relation has.
        unsafe {
            if c >= 16 {
                return self.at.get(c).cast::<i64>();
            }
            let v = &mut *self.v.get();
            if self.have.get() >> c & 1 != 0 {
                return v[c as usize].assume_init();
            }
            let x = self.at.get(c).cast::<i64>();
            v[c as usize].write(x);
            self.have.set(self.have.get() | 1 << c);
            x
        }
    }
}

/// What a walk does with its candidates (`Lanes::walk`).
trait Visit<const N: usize>
where
    LaneCount<N>: SupportedLaneCount,
{
    /// A tile of `nt` bindings begins: each one's group's first place and length.
    fn tile(&mut self, _nt: u32, _first: &[u32], _len: &[u32]) {}
    /// Candidates `k..k + N` of a chunk: each lane's binding and row, `m` the lanes whose row matches.
    unsafe fn each(&mut self, l: &Lanes, b: &BindAt<N>, row: &Row<N>, m: W<N>, k: u32);
    /// A chunk's `n` candidates are done: the tile's bindings `lo..hi` had theirs at `sk[i]..ek[i]` of it.
    unsafe fn chunk(&mut self, _n: u32, _lo: u32, _hi: u32, _sk: &[u32], _ek: &[u32]) {}
    /// The tile of `nt` bindings from `ta` is done.
    unsafe fn tile_end(&mut self, _ta: u32, _nt: u32) {}
}

impl Lanes<'_> {
    #[inline(always)]
    fn colval<const N: usize>(&self, b: &BindAt<N>, row: &Row<N>, k: u32) -> I<N>
    where
        LaneCount<N>: SupportedLaneCount,
    {
        let (s, k) = (self.st.s, k as usize);
        let v = if s.col_kind[k] == 0 { row.get(s.col_idx[k]) } else { b.slot(s.col_idx[k]) };
        if s.col_doc[k] != 0 { v >> si(32) } else { v }
    }

    #[inline(always)]
    fn parval<const N: usize>(&self, b: &BindAt<N>, k: u32) -> I<N>
    where
        LaneCount<N>: SupportedLaneCount,
    {
        let (s, k) = (self.st.s, k as usize);
        let v = if s.par_kind[k] == 0 { si(s.par_const[k]) } else { b.slot(s.par_slot[k]) };
        if s.par_doc[k] != 0 { v >> si(32) } else { v }
    }

    #[inline(always)]
    fn key<const N: usize>(&self, b: &BindAt<N>, k: u32) -> I<N>
    where
        LaneCount<N>: SupportedLaneCount,
    {
        let (s, k) = (self.st.s, k as usize);
        if s.key_kind[k] == 1 { b.slot(s.key_slot[k]) } else { si(s.key_const[k]) }
    }

    /// Arithmetic bind `j`'s values and the lanes where they are defined (no overflow, no zero divisor), as `arith`.
    /// A product's, a quotient's and a remainder's overflow has no vector test on NEON: their lanes each.
    fn arith<const N: usize>(&self, b: &BindAt<N>, row: &Row<N>, j: u32) -> (I<N>, W<N>)
    where
        LaneCount<N>: SupportedLaneCount,
    {
        let a = &self.st.s.arith[j as usize];
        let x = if a[3] & 1 != 0 { self.parval(b, a[1]) } else { self.colval(b, row, a[1]) };
        let y = if a[3] & 2 != 0 { self.parval(b, a[2]) } else { self.colval(b, row, a[2]) };
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

    #[inline(always)]
    fn defined<const N: usize>(&self, b: &BindAt<N>, row: &Row<N>) -> W<N>
    where
        LaneCount<N>: SupportedLaneCount,
    {
        let s = self.st.s;
        let mut ok = Mask::splat(true);
        for k in 0..s.nbinds.min(16) as usize {
            if s.bind_kind[k] == 3 {
                ok &= self.arith(b, row, s.bind_idx[k]).1;
            }
        }
        ok
    }

    /// The row program: bit x of each lane's value is operation x's.
    #[inline(always)]
    fn eval<const N: usize>(&self, b: &BindAt<N>, row: &Row<N>) -> U<N>
    where
        LaneCount<N>: SupportedLaneCount,
    {
        let s = self.st.s;
        let mut v = U::splat(0);
        for x in 0..s.nops.min(64) {
            let o = &s.ops[x as usize];
            let bit = |k: u32| (v >> U::splat(u64::from(k & 63))) & U::splat(1);
            let r: U<N> = match o[0] {
                0 => {
                    let w = (o[3] >> 8) & 255;
                    let a = lanes_of(self.colval(b, row, o[1]), w);
                    let c = if o[2] & 0x8000_0000 != 0 { self.parval(b, o[2] & 0xffff) } else { self.colval(b, row, o[2]) };
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
            v |= r << U::splat(u64::from(x));
        }
        v
    }

    #[inline(always)]
    fn out_bit<const N: usize>(&self, v: U<N>, k: u32) -> W<N>
    where
        LaneCount<N>: SupportedLaneCount,
    {
        ((v >> U::splat(u64::from(self.st.s.outputs[k as usize] & 63))) & U::splat(1)).simd_ne(U::splat(0))
    }

    /// The lanes whose row passes the stage's last output (or all, with none).
    #[inline(always)]
    fn passes<const N: usize>(&self, b: &BindAt<N>, row: &Row<N>) -> W<N>
    where
        LaneCount<N>: SupportedLaneCount,
    {
        let s = self.st.s;
        let ok = self.defined(b, row);
        if s.last == NONE {
            return ok;
        }
        ok & self.out_bit(self.eval(b, row), s.last)
    }

    /// Each lane's binding's group of candidate places (where `live`): its keys' hash's group of the relation's index
    /// (none where the screen holds no key), or every row with no keys; its first place and its end.
    #[inline(always)]
    unsafe fn group<const N: usize>(&self, f: *const u8, b: &BindAt<N>, live: W<N>) -> (R<N>, R<N>)
    where
        LaneCount<N>: SupportedLaneCount,
    {
        let s = self.st.s;
        let nk = s.nkeys.min(8);
        if nk == 0 {
            let first = sr(s.r_from);
            return (first, first + live.cast::<i32>().select(sr(self.st.rows()), sr(0)));
        }
        let mut h = U::splat(KEY_SEED);
        for k in 0..nk {
            h = ln::key_mix(h, self.key(b, k).cast::<u64>());
        }
        let mut go = live;
        if s.screen != 0 && s.screen <= nk {
            let (lo, hi) = ln::bloom_hash(self.key(b, s.screen - 1).cast::<u64>());
            go &= ln::bloom_may(f.add(s.screen_at as usize), lo, hi).cast::<i64>();
        }
        let (first, len) = ln::rel_group(self.d, s.r_index, h);
        (first, first + go.cast::<i32>().select(len, sr(0)))
    }

    /// Bindings `t0..t1`' candidates, a tile of bindings at a time: their groups found `N` bindings a step, their
    /// places expanded into chunks of candidates, `N` candidates a step for `v`.
    unsafe fn walk<const N: usize>(&self, f: *const u8, t0: u32, t1: u32, v: &mut impl Visit<N>)
    where
        LaneCount<N>: SupportedLaneCount,
    {
        let s = self.st.s;
        let nk = s.nkeys.min(8) as usize;
        let iota = lane::<N>();
        let (mut first, mut len) = ([0u32; TILE + SLACK], [0u32; TILE + SLACK]);
        let (mut cb, mut cq) = ([0u32; CANDS + SLACK], [0u32; CANDS + SLACK]);
        let (mut sk, mut ek) = ([0u32; TILE + SLACK], [0u32; TILE + SLACK]);
        let mut ta = t0;
        while ta < t1 {
            let nt = (t1 - ta).min(TILE as u32);
            // The tile's groups, `N` bindings a step: their keys one load each, hashed, screened and found together.
            let mut g = 0;
            while g < nt {
                let t = sr::<N>(ta + g) + iota;
                let live = t.simd_lt(sr(ta + nt));
                let b = BindAt::new(self, live.select(t, sr(ta + g)), live.all());
                let (a, e) = self.group(f, &b, live.cast::<i64>());
                st(first.as_mut_ptr().add(g as usize), a);
                st(len.as_mut_ptr().add(g as usize), e - a);
                g += N as u32;
            }
            v.tile(nt, &first, &len);
            // The candidates, `CANDS` a chunk: each binding's group's places in order (a group longer than the room
            // left goes on in the next chunk).
            let (mut i, mut j) = (0u32, 0u32);
            while i < nt {
                let lo = i;
                let mut n = 0u32;
                while i < nt && n < CANDS as u32 {
                    let take = (len[i as usize] - j).min(CANDS as u32 - n);
                    sk[i as usize] = n;
                    let mut c = 0;
                    while c < take {
                        st(cb.as_mut_ptr().add((n + c) as usize), sr::<N>(i));
                        st(cq.as_mut_ptr().add((n + c) as usize), sr::<N>(first[i as usize] + j + c) + iota);
                        c += N as u32;
                    }
                    n += take;
                    j += take;
                    ek[i as usize] = n;
                    if j < len[i as usize] {
                        break;
                    }
                    i += 1;
                    j = 0;
                }
                let hi = if j > 0 { i + 1 } else { i };
                let mut k = 0;
                while k < n {
                    let valid = (sr::<N>(k) + iota).simd_lt(sr(n));
                    let (bi, q) = (ld::<N>(cb.as_ptr().add(k as usize)), ld::<N>(cq.as_ptr().add(k as usize)));
                    let (bi, q) = (valid.select(bi, sr(bi[0])), valid.select(q, sr(q[0])));
                    let b = BindAt::new(self, sr::<N>(ta) + bi, false);
                    // A whole step of one group's consecutive places: its rows one load.
                    let id = if nk == 0 {
                        q
                    } else if valid.all() && q.simd_eq(sr::<N>(q[0]) + iota).all() {
                        ln::rel_group_rows(self.d, s.r_index, q[0])
                    } else {
                        ln::rel_group_row(self.d, s.r_index, q)
                    };
                    let row = Row::new(self.d, id, self.far, self.st.rd.col_sealed());
                    let mut m = valid.cast::<i64>();
                    if s.r_dead != 0 {
                        m &= !ln::rel_dead(self.dead, id).cast::<i64>();
                    }
                    for k in 0..nk {
                        m &= row.get(s.key_rcol[k]).simd_eq(self.key(&b, k as u32));
                    }
                    for k in 0..s.ntests.min(8) as usize {
                        m &= row.get(s.test_a[k]).simd_eq(row.get(s.test_b[k]));
                    }
                    v.each(self, &b, &row, m, k);
                    k += N as u32;
                }
                v.chunk(n, lo, hi, &sk, &ek);
            }
            v.tile_end(ta, nt);
            ta += nt;
        }
    }

    /// Each lane's row's slots and binds, each column's values given to `out` with the column.
    #[inline(always)]
    unsafe fn write_rows<const N: usize>(&self, b: &BindAt<N>, row: &Row<N>, mut out: impl FnMut(I<N>, u32))
    where
        LaneCount<N>: SupportedLaneCount,
    {
        let s = self.st.s;
        for k in 0..s.nslots.min(16) {
            out(b.slot(k), k);
        }
        for k in 0..s.nbinds.min(16) as usize {
            let i = s.bind_idx[k];
            let v = match s.bind_kind[k] {
                3 => {
                    let (v, ok) = self.arith(b, row, i);
                    ok.select(v, si(0))
                }
                0 => self.colval(b, row, i),
                1 => self.parval(b, i),
                _ => row.get(i),
            };
            out(v, s.bind_slot[k]);
        }
    }
}

/// A count's walk: each binding's rows out (a semi-join's and an anti-join's 0 or 1) into `counts`, the rows matched
/// and passed into `p0` and `q`. A candidate's verdict is a bit of the chunk's exclusive prefix sums (`pre`); a
/// binding's count, the sums at its candidates' end less at their start.
struct Count<const N: usize>
where
    LaneCount<N>: SupportedLaneCount,
{
    counts: *mut u32,
    cnt: [u32; TILE + SLACK],
    pre: [u32; CANDS + SLACK + 1],
    run: u32,
    p0: I<N>,
    q: [I<N>; 16],
    /// A join's finds (`V`, `stage_count_one`'s; null: none kept): the tile's bindings' first places (`NONE` where
    /// their groups are longer than `FINDS_MAX`), their bits, their candidates in the chunks before, and the chunk's
    /// verdicts, a bit a candidate.
    v: *mut i64,
    lo: [u32; TILE + SLACK],
    bits: [u32; TILE + SLACK],
    seen: [u32; TILE + SLACK],
    pb: [u64; (CANDS + SLACK) / 64 + 2],
}

impl<const N: usize> Visit<N> for Count<N>
where
    LaneCount<N>: SupportedLaneCount,
{
    fn tile(&mut self, nt: u32, first: &[u32], len: &[u32]) {
        self.cnt[..nt as usize].fill(0);
        if self.v.is_null() {
            return;
        }
        let mut i = 0;
        while i < nt as usize {
            // SAFETY: the tile's arrays, `SLACK` past its bindings.
            unsafe {
                let short = ld::<N>(len.as_ptr().add(i)).simd_le(sr(FINDS_MAX));
                st(self.lo.as_mut_ptr().add(i), short.select(ld::<N>(first.as_ptr().add(i)), sr(NONE)));
                st(self.bits.as_mut_ptr().add(i), sr(0));
                st(self.seen.as_mut_ptr().add(i), sr(0));
            }
            i += N;
        }
    }

    unsafe fn each(&mut self, l: &Lanes, b: &BindAt<N>, row: &Row<N>, m: W<N>, k: u32) {
        let s = l.st.s;
        let pass = if s.mode != 0 || !m.any() {
            m
        } else {
            self.p0 += ones(m);
            let ok = m & l.defined(b, row);
            let v = if s.nops > 0 { l.eval(b, row) } else { U::splat(0) };
            for (q, &bit) in self.q.iter_mut().zip(&s.passed).take(s.npassed.min(16) as usize) {
                *q += ones(if bit == NONE { ok } else { ok & l.out_bit(v, bit) });
            }
            if s.last == NONE { ok } else { ok & l.out_bit(v, s.last) }
        };
        let bits = pass.to_bitmask();
        st(self.pre.as_mut_ptr().add(k as usize), sr::<N>(self.run) + ranks::<N>(bits));
        self.run += bits.count_ones();
        if !self.v.is_null() {
            // `k` is a multiple of `N`, a power of two below 64: its bits in one word.
            self.pb[k as usize / 64] |= bits << (k % 64);
        }
    }

    unsafe fn chunk(&mut self, n: u32, lo: u32, hi: u32, sk: &[u32], ek: &[u32]) {
        self.pre[n as usize] = self.run;
        self.run = 0;
        let mut i = lo;
        while i < hi {
            let on = (sr::<N>(i) + lane::<N>()).simd_lt(sr(hi)).cast::<isize>();
            let (a, e) = (ld::<N>(sk.as_ptr().add(i as usize)), ld::<N>(ek.as_ptr().add(i as usize)));
            let got = R::gather_select(&self.pre, on, e.cast::<usize>(), sr(0)) - R::gather_select(&self.pre, on, a.cast::<usize>(), sr(0));
            let c = self.cnt.as_mut_ptr().add(i as usize);
            st(c, ld::<N>(c) + got);
            if !self.v.is_null() {
                // Each binding's verdicts here, its candidates `a..e` of the chunk: a field of at most `FINDS_MAX`
                // bits (a longer group's, unread), past the bits of its candidates in the chunks before.
                let w = ln::gather64_ix(self.pb.as_ptr().cast::<u8>(), a >> sr(3), 0);
                let n = (e - a).cast::<u64>();
                let field = (w >> (a & sr(7)).cast::<u64>()) & ((Simd::splat(1u64) << n) - Simd::splat(1));
                let (bp, sp) = (self.bits.as_mut_ptr().add(i as usize), self.seen.as_mut_ptr().add(i as usize));
                let seen = ld::<N>(sp);
                let bits = ld::<N>(bp) | (field << seen.cast::<u64>()).cast::<u32>();
                st(bp, on.cast::<i32>().select(bits, ld::<N>(bp)));
                st(sp, on.cast::<i32>().select(seen + (e - a), seen));
            }
            i += N as u32;
        }
        if !self.v.is_null() {
            self.pb.fill(0);
        }
    }

    unsafe fn tile_end(&mut self, ta: u32, nt: u32) {
        core::ptr::copy_nonoverlapping(self.cnt.as_ptr(), self.counts.add(ta as usize), nt as usize);
        if self.v.is_null() {
            return;
        }
        for i in 0..nt as usize {
            *self.v.add(ta as usize + i) = if self.lo[i] == NONE { FINDS_WALK } else { (u64::from(self.lo[i]) | u64::from(self.bits[i]) << 32) as i64 };
        }
    }
}

/// Bindings `t0..t1` of a relational stage (`runs_at_lanes`), all below its input's rows: each one's count of rows
/// out (a semi-join's and an anti-join's 0 or 1) into `counts`, its rows matched and passed into `acc`.
pub(super) unsafe fn stage_count_lanes<const N: usize>(st: &St, j: &Job, counts: *mut u32, v: *mut i64, t0: u32, t1: u32, acc: &mut [u32; 17])
where
    LaneCount<N>: SupportedLaneCount,
{
    let s = st.s;
    let l = Lanes { st, d: (st.r as *const u8).wrapping_add(s.r_at as usize), dead: (st.r as *const u8).wrapping_add(s.r_dead as usize), far: st.far as *const u8 };
    let v = if s.mode == 0 { v } else { core::ptr::null_mut() };
    let mut v = Count::<N> {
        counts,
        cnt: [0; TILE + SLACK],
        pre: [0; CANDS + SLACK + 1],
        run: 0,
        p0: si(0),
        q: [si(0); 16],
        v,
        lo: [0; TILE + SLACK],
        bits: [0; TILE + SLACK],
        seen: [0; TILE + SLACK],
        pb: [0; (CANDS + SLACK) / 64 + 2],
    };
    if s.mode != 0 {
        // A semi-join's binding is 1 where its walk found a row, an anti-join's where it found none.
        struct Exists<const N: usize>(Count<N>, bool)
        where
            LaneCount<N>: SupportedLaneCount;
        impl<const N: usize> Visit<N> for Exists<N>
        where
            LaneCount<N>: SupportedLaneCount,
        {
            fn tile(&mut self, nt: u32, first: &[u32], len: &[u32]) {
                self.0.tile(nt, first, len);
            }
            unsafe fn each(&mut self, l: &Lanes, b: &BindAt<N>, row: &Row<N>, m: W<N>, k: u32) {
                self.0.each(l, b, row, m, k);
            }
            unsafe fn chunk(&mut self, n: u32, lo: u32, hi: u32, sk: &[u32], ek: &[u32]) {
                self.0.chunk(n, lo, hi, sk, ek);
            }
            unsafe fn tile_end(&mut self, ta: u32, nt: u32) {
                let anti = self.1;
                for c in &mut self.0.cnt[..nt as usize] {
                    *c = u32::from((*c > 0) != anti);
                }
                self.0.tile_end(ta, nt);
            }
        }
        let mut e = Exists(v, s.mode == 2);
        l.walk(j.f, t0, t1, &mut e);
        return;
    }
    l.walk(j.f, t0, t1, &mut v);
    acc[0] += v.p0.reduce_sum() as u32;
    for (a, q) in acc[1..].iter_mut().zip(&v.q).take(s.npassed.min(16) as usize) {
        *a += q.reduce_sum() as u32;
    }
}

/// A join's scatter's walk: the rows that pass, in their order, each at the next place from the range's first
/// offset (a stream compaction), below the stage's room.
struct Scatter {
    o: *mut i64,
    run: u32,
    /// The range's last place and the stage's room, the lesser: the places this range's rows may take.
    end: u32,
}

/// For each 8 lanes' bits, the bytes of 8 64-bit lanes that move the lanes of the bits to the front, in their order
/// (the rest, lane 0's): a stream compaction's table lookup (`swizzle_dyn`).
const PACK64: [[u8; 64]; 256] = {
    let mut t = [[0u8; 64]; 256];
    let mut bits = 0;
    while bits < 256 {
        let (mut k, mut i) = (0, 0);
        while i < 8 {
            if bits >> i & 1 != 0 {
                let mut b = 0;
                while b < 8 {
                    t[bits][8 * k + b] = (8 * i + b) as u8;
                    b += 1;
                }
                k += 1;
            }
            i += 1;
        }
        bits += 1;
    }
    t
};

/// The 8 values of `v` (`N` is 8) of the lanes of `bits`, in their order, at `p`, and values past them up to `p + 8`
/// (places a later row takes).
#[inline(always)]
unsafe fn pack8<const N: usize>(v: I<N>, ix: Simd<u8, 64>, p: *mut i64)
where
    LaneCount<N>: SupportedLaneCount,
{
    let b = Simd::<u8, 64>::from_array(core::mem::transmute_copy::<I<N>, [u8; 64]>(&v));
    p.cast::<[u8; 64]>().write_unaligned(b.swizzle_dyn(ix).to_array());
}

impl<const N: usize> Visit<N> for Scatter
where
    LaneCount<N>: SupportedLaneCount,
{
    unsafe fn each(&mut self, l: &Lanes, b: &BindAt<N>, row: &Row<N>, m: W<N>, _: u32) {
        if !m.any() {
            return;
        }
        self.put(l, b, row, m & l.passes(b, row));
    }
}

impl Scatter {
    /// The rows of lanes `w` out, in their order, from the next place.
    #[inline(always)]
    unsafe fn put<const N: usize>(&mut self, l: &Lanes, b: &BindAt<N>, row: &Row<N>, w: W<N>)
    where
        LaneCount<N>: SupportedLaneCount,
    {
        let bits = w.to_bitmask();
        if bits == 0 {
            return;
        }
        let (o, cap) = (self.o, l.st.s.cap_out as usize);
        if N == 8 && self.run + 8 <= self.end {
            // Room for all 8 in this range: each column's lanes packed to the front and stored, 8 places at once.
            let ix = Simd::<u8, 64>::from_array(PACK64[bits as usize]);
            let at = self.run as usize;
            l.write_rows(b, row, |v, c| pack8(v, ix, o.add(c as usize * cap + at)));
        } else {
            let a = sr::<N>(self.run) + ranks::<N>(bits);
            let on = (w & a.simd_lt(sr(l.st.s.cap_out)).cast::<i64>()).to_bitmask();
            l.write_rows(b, row, |v, c| ln::scatter64_ix(v.cast::<u64>(), o.add(c as usize * cap).cast::<u8>(), a, 3, on));
        }
        self.run += bits.count_ones();
    }
}

/// Bindings `t0..t1` of a join (`runs_at_lanes`, mode 0), all its live bindings from `t0`: their rows out from `t0`'s
/// offset (`offsets`, less `o0`; each binding's follow the one before's), below the stage's room.
pub(super) unsafe fn stage_scatter_lanes<const N: usize>(st: &St, j: &Job, offsets: *const u32, o: *mut i64, finds: *const i64, t0: u32, t1: u32)
where
    LaneCount<N>: SupportedLaneCount,
{
    let s = st.s;
    let l = Lanes { st, d: (st.r as *const u8).wrapping_add(s.r_at as usize), dead: (st.r as *const u8).wrapping_add(s.r_dead as usize), far: st.far as *const u8 };
    let o0 = *offsets;
    let end = (*offsets.add(t1 as usize)).wrapping_sub(o0).min(s.cap_out);
    let mut v = Scatter { o, run: (*offsets.add(t0 as usize)).wrapping_sub(o0), end };
    if N != 8 {
        l.walk::<N>(j.f, t0, t1, &mut v);
        return;
    }
    // A tile at a time: its bindings' finds (`stage_count_one`'s) where each has them, else its walk again.
    let mut ta = t0;
    while ta < t1 {
        let nt = (t1 - ta).min(TILE as u32);
        let f = finds.add(ta as usize);
        if (0..nt as usize).any(|i| *f.add(i) as u32 == NONE) {
            l.walk::<N>(j.f, ta, ta + nt, &mut v);
        } else {
            l.found::<N>(f, ta, nt, &mut v);
        }
        ta += nt;
    }
}

/// For each byte, the places of its set bits in order (the rest 0): a binding's finds' candidates, 8 bits a read.
const PLACES8: [[u32; 8]; 256] = {
    let mut t = [[0u32; 8]; 256];
    let mut bits = 0;
    while bits < 256 {
        let (mut k, mut i) = (0, 0);
        while i < 8 {
            if bits >> i & 1 != 0 {
                t[bits][k] = i as u32;
                k += 1;
            }
            i += 1;
        }
        bits += 1;
    }
    t
};

impl Lanes<'_> {
    /// A tile's `nt` bindings from `ta` out by their finds (`f`, every one's kept): their candidates that are rows
    /// out, expanded into chunks in their order, `N` (8) rows a step, none probed or tested again.
    unsafe fn found<const N: usize>(&self, f: *const i64, ta: u32, nt: u32, v: &mut Scatter)
    where
        LaneCount<N>: SupportedLaneCount,
    {
        let s = self.st.s;
        let iota = lane::<N>();
        let (mut cb, mut cq) = ([0u32; CANDS + SLACK], [0u32; CANDS + SLACK]);
        let mut i = 0u32;
        while i < nt {
            // The rows out of bindings from `i`, while a chunk has room for another's `FINDS_MAX`.
            let mut n = 0u32;
            while i < nt && n <= (CANDS as u32) - FINDS_MAX {
                let w = *f.add(i as usize) as u64;
                let (mut at, mut bits) = (w as u32, (w >> 32) as u32);
                while bits != 0 {
                    let byte = (bits & 255) as usize;
                    st(cq.as_mut_ptr().add(n as usize), sr::<N>(at) + Simd::from_slice(&PLACES8[byte]));
                    st(cb.as_mut_ptr().add(n as usize), sr::<N>(i));
                    n += byte.count_ones();
                    bits >>= 8;
                    at += 8;
                }
                i += 1;
            }
            let mut k = 0;
            while k < n {
                let valid = (sr::<N>(k) + iota).simd_lt(sr(n));
                let (bi, q) = (ld::<N>(cb.as_ptr().add(k as usize)), ld::<N>(cq.as_ptr().add(k as usize)));
                let (bi, q) = (valid.select(bi, sr(bi[0])), valid.select(q, sr(q[0])));
                let b = BindAt::new(self, sr::<N>(ta) + bi, false);
                let id = if s.nkeys == 0 {
                    q
                } else if valid.all() && q.simd_eq(sr::<N>(q[0]) + iota).all() {
                    ln::rel_group_rows(self.d, s.r_index, q[0])
                } else {
                    ln::rel_group_row(self.d, s.r_index, q)
                };
                v.put(self, &b, &Row::new(self.d, id, self.far, self.st.rd.col_sealed()), valid.cast::<i64>());
                k += N as u32;
            }
        }
    }
}
