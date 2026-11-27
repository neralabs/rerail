//! Frozen integer columns' format and reads (moved from `rerail_blocks::col`, whose writers choose and pack them).
//!
//! - `CONST`: every value is `base`. No bytes.
//! - `PACK`: `v - base` in `bits` bits, packed into 64-bit words (frame of reference).
//! - `RUNS`: runs of equal values: each run's value (`v - base`, `bits` bits) and its first position (`rbits`).
//!   A sorted column with repeats, as a canonical order's leading column is.
//! - `LINE`: `v - (base + i * slope / 2^16)`, less `rmin`, in `bits` bits (MLDB's `Predictor`): a column that grows
//!   with its position, as ids in canonical order do.
//! - `GROUPS` (decision 0456 K2): groups of 128 values, each its own minimum (`bits` bits, less `base`) and width;
//!   a value is its group's minimum and its residual, packed at the group's width from the group's bit offset (each
//!   group's width and offset one `rbits`-bit entry). The random-access counterpart of a delta varint: a sorted id
//!   column's 128 values span a small range, and a group that crosses a document costs that group alone.
//!
//! - `DICT` (decision 0622 step 4, MLDB's table): the column's distinct values, sorted, each less `base` in `bits` bits
//!   (`runs`, `nruns` of them), and each row's index among them in `rbits` bits (`data`). A column of few values
//!   spread wide (atoms, kinds, a few documents' ids): its codes take the bits its count needs, not its range. Taken
//!   for a relation's sealed chunks only (`choose_for_reads`), where it is smaller than every other encoding.
//!
//! Reads are arithmetic on the words: the same on a GPU, where a column is read for its bandwidth and the extra
//! instructions to decode it cost nothing.

use crate::refs::Span;
use crate::structure;

pub const CONST: u32 = 0;
pub const PACK: u32 = 1;
pub const RUNS: u32 = 2;
pub const LINE: u32 = 3;
pub const GROUPS: u32 = 4;
pub const DICT: u32 = 5;

/// A `GROUPS` group's values, as a shift.
pub const GROUP_LOG: u32 = 7;
pub const GROUP: usize = 1 << GROUP_LOG;
/// Bits of a group's width in its entry (widths 0..=64), below its offset.
pub const WIDTH_BITS: u32 = 7;

/// The encodings' names, by tag.
pub const TAG_NAMES: [&str; 6] = ["const", "pack", "runs", "line", "groups", "dict"];

structure! {
    pub struct IntCol {
        pub tag: u32,
        /// Bits a value (or a run's value, or a residual) takes.
        pub bits: u32,
        pub n: u32,
        /// `RUNS`: bits a run's first position takes, and the run count.
        pub rbits: u32,
        pub nruns: u32,
        pub pad: u32,
        pub base: i64,
        /// `LINE`: the slope, in 1/65,536ths a row, and the residuals' minimum.
        pub slope: i64,
        pub rmin: i64,
        /// Packed values (one spare word at the end, so a read never branches on a word boundary).
        pub data: Span<u64>,
        /// `RUNS`: packed first positions.
        pub runs: Span<u64>,
    }
}

pub fn bits_for(range: u64) -> u32 {
    64 - range.leading_zeros()
}

#[inline]
pub fn read(words: &[u64], i: usize, bits: u32) -> u64 {
    if bits == 0 {
        return 0;
    }
    let bit = i * bits as usize;
    let (w, sh) = (bit >> 6, bit & 63);
    let two = words[w] as u128 | (words[w + 1] as u128) << 64;
    let mask = if bits == 64 { u64::MAX } else { (1u64 << bits) - 1 };
    (two >> sh) as u64 & mask
}

/// Words `n` values of `bits` bits take, and the spare one.
pub fn words(n: usize, bits: u32) -> usize {
    (n * bits as usize).div_ceil(64) + 1
}

impl IntCol {
    #[inline]
    pub fn len(&self) -> usize {
        self.n as usize
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// The column resolved for reads: its fields and words loaded once.
    #[inline]
    pub fn dec(&self) -> Dec<'_> {
        Dec::of(Fields { tag: self.tag, bits: self.bits, rbits: self.rbits, n: self.n as usize, nruns: self.nruns as usize, base: self.base, slope: self.slope, rmin: self.rmin }, self.data.as_slice(), self.runs.as_slice())
    }

    #[inline]
    fn run_start(&self, k: usize) -> usize {
        read(self.runs.as_slice(), k, self.rbits) as usize
    }

    /// Value `i`, read from the descriptor's fields: no `Dec` made (a sealed chunk's read, decision 0622, is one of
    /// these per row read at random).
    #[inline]
    pub fn get(&self, i: usize) -> i64 {
        match self.tag {
            CONST => self.base,
            // PACK takes at least one bit.
            PACK => self.base.wrapping_add(rd(self.data.as_slice(), i, self.bits, u64::MAX >> (64 - self.bits)) as i64),
            LINE => {
                let r = if self.bits == 0 { 0 } else { rd(self.data.as_slice(), i, self.bits, mask_of(self.bits)) };
                let t = ((i as i128 * self.slope as i128) >> 16) as i64;
                self.base.wrapping_add(t).wrapping_add(self.rmin).wrapping_add(r as i64)
            }
            GROUPS => {
                let (runs, k) = (self.runs.as_slice(), i >> GROUP_LOG);
                let entries = words(self.nruns as usize, self.bits) * 64;
                let min = rd_bit(runs, k * self.bits as usize, mask_of(self.bits));
                let e = rd_bit(runs, entries + k * self.rbits as usize, mask_of(self.rbits));
                let w = (e & ((1 << WIDTH_BITS) - 1)) as u32;
                let r = rd_bit(self.data.as_slice(), (e >> WIDTH_BITS) as usize + (i & (GROUP - 1)) * w as usize, mask_of(w));
                self.base.wrapping_add(min as i64).wrapping_add(r as i64)
            }
            DICT => {
                let k = rd(self.data.as_slice(), i, self.rbits, mask_of(self.rbits));
                self.base.wrapping_add(rd(self.runs.as_slice(), k as usize, self.bits, mask_of(self.bits)) as i64)
            }
            _ => self.dec().get(i),
        }
    }

    /// Every value in order, decoded as it goes (a run's value once per run, not a search per row).
    pub fn for_each(&self, mut f: impl FnMut(i64)) {
        let n = self.len();
        match self.tag {
            CONST => (0..n).for_each(|_| f(self.base)),
            RUNS => {
                let d = self.data.as_slice();
                for k in 0..self.nruns as usize {
                    let end = if k + 1 < self.nruns as usize { self.run_start(k + 1) } else { n };
                    let v = self.base.wrapping_add(read(d, k, self.bits) as i64);
                    for _ in self.run_start(k)..end {
                        f(v);
                    }
                }
            }
            // Resolved once (`dec`), not per value as `get` resolves a grouped or lined column.
            _ => {
                let d = self.dec();
                (0..n).for_each(|i| f(d.get(i)))
            }
        }
    }

    /// The first position in `lo..hi` whose value is not less than `key`, the column sorted over that range.
    pub fn lower_bound(&self, lo: usize, hi: usize, key: i64) -> usize {
        self.dec().lower_bound(lo, hi, key)
    }

    /// `lo..hi`'s positions whose value is `key`, the column sorted over that range.
    pub fn equal_range(&self, lo: usize, hi: usize, key: i64) -> (usize, usize) {
        self.dec().equal_range(lo, hi, key)
    }

    pub fn bytes(&self) -> usize {
        (self.data.len() + self.runs.len()) * 8
    }

    pub fn tag_name(&self) -> &'static str {
        TAG_NAMES[self.tag as usize]
    }
}

/// Value `i` of `bits` bits (`mask` its mask, `bits > 0`) from packed words: two loads and two shifts, no branch.
#[inline(always)]
fn rd(words: &[u64], i: usize, bits: u32, mask: u64) -> u64 {
    let bit = i * bits as usize;
    let (w, sh) = (bit >> 6, (bit & 63) as u32);
    debug_assert!(w + 1 < words.len());
    // SAFETY: a column's words end with a spare one (`words`), so `w + 1` is in bounds for every value it holds.
    let (a, b) = unsafe { (*words.get_unchecked(w), *words.get_unchecked(w + 1)) };
    // `b << 1 << (63 - sh)` is `b << (64 - sh)` without a shift by 64 when `sh` is 0.
    ((a >> sh) | ((b << 1) << (63 - sh))) & mask
}

/// The value of `mask`'s bits at bit `bit` of packed words: `rd` from a bit position.
#[inline(always)]
fn rd_bit(words: &[u64], bit: usize, mask: u64) -> u64 {
    let (w, sh) = (bit >> 6, (bit & 63) as u32);
    debug_assert!(w + 1 < words.len());
    // SAFETY: `GROUPS`' words end with two spare ones, so a read at its last bit (a group of width 0 there) is in
    // bounds; its entries' words end with the spare one `words` gives.
    let (a, b) = unsafe { (*words.get_unchecked(w), *words.get_unchecked(w + 1)) };
    ((a >> sh) | ((b << 1) << (63 - sh))) & mask
}

/// The mask of `w` bits, `w` up to 64: no branch.
#[inline(always)]
fn mask_of(w: u32) -> u64 {
    ((1u128 << w) - 1) as u64
}

/// The first position in `lo..hi` whose value (`f`, non-decreasing) is not less than `key`. One guess by
/// interpolation between the ends, a gallop from it to bracket the answer, then a binary search of the bracket.
/// A sorted column of a packed block is near linear (keys, run values, leading columns, starts), so the guess
/// lands within a few positions and a search reads about six values where a binary one reads `log2 n`. A
/// skewed column costs at most about twice the binary search.
#[inline]
fn search(mut lo: usize, mut hi: usize, key: i64, f: impl Fn(usize) -> i64) -> usize {
    if hi - lo > 8 {
        let (a, b) = (f(lo), f(hi - 1));
        if key <= a {
            return lo;
        }
        if key > b {
            return hi;
        }
        // f(lo) < key <= f(hi - 1): the answer is in lo + 1..=hi - 1.
        let g = lo + ((key as i128 - a as i128) * (hi - 1 - lo) as i128 / (b as i128 - a as i128)) as usize;
        (lo, hi) = (lo + 1, hi - 1);
        let g = g.clamp(lo, hi);
        let mut step = 1;
        if f(g) < key {
            lo = g + 1;
            let mut p = g;
            while p + step < hi {
                let q = p + step;
                if f(q) < key {
                    (lo, p, step) = (q + 1, q, step << 1);
                } else {
                    hi = q;
                    break;
                }
            }
        } else {
            hi = g;
            let mut p = g;
            while p - lo >= step {
                let q = p - step;
                if f(q) >= key {
                    (hi, p, step) = (q, q, step << 1);
                } else {
                    lo = q + 1;
                    break;
                }
            }
        }
    }
    while lo < hi {
        let m = (lo + hi) / 2;
        if f(m) < key { lo = m + 1 } else { hi = m }
    }
    lo
}

/// The end of `key`'s positions from `a` (whose value is `key`) in `a..hi`: a gallop, then a binary search.
#[inline]
fn upper_from(a: usize, hi: usize, key: i64, f: impl Fn(usize) -> i64) -> usize {
    let (mut lo, mut end, mut p, mut step) = (a + 1, hi, a, 1);
    while p + step < hi {
        let q = p + step;
        if f(q) == key {
            (lo, p, step) = (q + 1, q, step << 1);
        } else {
            end = q;
            break;
        }
    }
    while lo < end {
        let m = (lo + end) / 2;
        if f(m) == key { lo = m + 1 } else { end = m }
    }
    lo
}

/// An `IntCol` resolved for reads: its descriptor's fields and its words' slices loaded once, so a probe's reads
/// are arithmetic on registers (a descriptor's spans are atomics, which a compiler does not merge).
#[derive(Clone, Copy)]
pub struct Dec<'a> {
    tag: u32,
    bits: u32,
    mask: u64,
    rbits: u32,
    rmask: u64,
    n: usize,
    nruns: usize,
    /// `GROUPS`: the bit where the groups' entries start in `runs`, past their minima.
    entries: usize,
    base: i64,
    slope: i64,
    rmin: i64,
    data: &'a [u64],
    runs: &'a [u64],
}

/// An encoding's fields, wherever its descriptor keeps them (an `IntCol`'s, or a flat `IntTable`'s body).
#[derive(Clone, Copy, Debug)]
pub struct Fields {
    pub tag: u32,
    pub bits: u32,
    pub rbits: u32,
    pub n: usize,
    pub nruns: usize,
    pub base: i64,
    pub slope: i64,
    pub rmin: i64,
}

impl<'a> Dec<'a> {
    /// An encoding resolved for reads from its fields and its words.
    #[inline]
    pub fn of(f: Fields, data: &'a [u64], runs: &'a [u64]) -> Dec<'a> {
        let mask = |b: u32| if b == 0 { 0 } else if b == 64 { u64::MAX } else { (1u64 << b) - 1 };
        Dec {
            tag: f.tag,
            bits: f.bits,
            mask: mask(f.bits),
            rbits: f.rbits,
            rmask: mask(f.rbits),
            n: f.n,
            nruns: f.nruns,
            entries: if f.tag == GROUPS { words(f.nruns, f.bits) * 64 } else { 0 },
            base: f.base,
            slope: f.slope,
            rmin: f.rmin,
            data,
            runs,
        }
    }
}

impl Dec<'_> {
    #[inline]
    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// Packed value `k` (a value of `PACK`, a run's value of `RUNS`).
    #[inline(always)]
    fn val(&self, k: usize) -> i64 {
        self.base.wrapping_add(rd(self.data, k, self.bits, self.mask) as i64)
    }

    /// `LINE`'s value `i`: the line, its residuals' minimum and the residual, exact modulo 2^64.
    #[inline(always)]
    fn on_line(&self, i: usize) -> i64 {
        let r = if self.bits == 0 { 0 } else { rd(self.data, i, self.bits, self.mask) };
        let t = ((i as i128 * self.slope as i128) >> 16) as i64;
        self.base.wrapping_add(t).wrapping_add(self.rmin).wrapping_add(r as i64)
    }

    /// `GROUPS`' value `i`: its group's minimum, then its residual at the group's width from the group's offset.
    #[inline(always)]
    fn grp(&self, i: usize) -> i64 {
        let k = i >> GROUP_LOG;
        let min = rd_bit(self.runs, k * self.bits as usize, self.mask);
        let e = rd_bit(self.runs, self.entries + k * self.rbits as usize, self.rmask);
        let w = (e & ((1 << WIDTH_BITS) - 1)) as u32;
        let r = rd_bit(self.data, (e >> WIDTH_BITS) as usize + (i & (GROUP - 1)) * w as usize, mask_of(w));
        self.base.wrapping_add(min as i64).wrapping_add(r as i64)
    }

    /// `DICT`'s value `i`: its index among the sorted distinct values, then that value.
    #[inline(always)]
    fn dct(&self, i: usize) -> i64 {
        let k = rd(self.data, i, self.rbits, self.rmask) as usize;
        self.base.wrapping_add(rd(self.runs, k, self.bits, self.mask) as i64)
    }

    /// Run `k`'s first position (`n` past the last run).
    #[inline(always)]
    fn start(&self, k: usize) -> usize {
        if k >= self.nruns { self.n } else { rd(self.runs, k, self.rbits, self.rmask) as usize }
    }

    /// The run holding position `i`.
    #[inline]
    fn run_of(&self, i: usize) -> usize {
        let (mut lo, mut hi) = (0usize, self.nruns);
        while hi - lo > 1 {
            let mid = (lo + hi) / 2;
            if self.start(mid) <= i { lo = mid } else { hi = mid }
        }
        lo
    }

    /// The runs `lo..hi` covers.
    #[inline]
    fn runs_over(&self, lo: usize, hi: usize) -> (usize, usize) {
        let a = if lo == 0 { 0 } else { self.run_of(lo) };
        let b = if hi >= self.n { self.nruns } else { self.run_of(hi - 1) + 1 };
        (a, b)
    }

    #[inline]
    pub fn get(&self, i: usize) -> i64 {
        match self.tag {
            CONST => self.base,
            PACK => self.val(i),
            RUNS => self.val(self.run_of(i)),
            GROUPS => self.grp(i),
            DICT => self.dct(i),
            _ => self.on_line(i),
        }
    }

    /// Values `lo..lo + out.len()` into `out`, in order: a run's value once per run, its first run found by one
    /// search (`get` searches for each value's).
    pub fn fill(&self, lo: usize, out: &mut [i64]) {
        match self.tag {
            CONST => out.fill(self.base),
            PACK => out.iter_mut().enumerate().for_each(|(j, o)| *o = self.val(lo + j)),
            RUNS => {
                let (mut k, mut i) = (if lo == 0 { 0 } else { self.run_of(lo) }, 0);
                while i < out.len() {
                    let end = self.start(k + 1).min(lo + out.len()) - lo;
                    out[i..end].fill(self.val(k));
                    (i, k) = (end, k + 1);
                }
            }
            GROUPS => out.iter_mut().enumerate().for_each(|(j, o)| *o = self.grp(lo + j)),
            DICT => out.iter_mut().enumerate().for_each(|(j, o)| *o = self.dct(lo + j)),
            _ => out.iter_mut().enumerate().for_each(|(j, o)| *o = self.on_line(lo + j)),
        }
    }

    /// The first position in `lo..hi` whose value is not less than `key`, the column sorted over that range.
    #[inline]
    pub fn lower_bound(&self, lo: usize, hi: usize, key: i64) -> usize {
        if lo >= hi {
            return lo;
        }
        match self.tag {
            CONST => if self.base < key { hi } else { lo },
            PACK => search(lo, hi, key, |i| self.val(i)),
            RUNS => {
                // A run at a time: the search reads the runs' values, not the rows.
                let (a, b) = self.runs_over(lo, hi);
                self.start(search(a, b, key, |k| self.val(k))).clamp(lo, hi)
            }
            GROUPS => search(lo, hi, key, |i| self.grp(i)),
            DICT => search(lo, hi, key, |i| self.dct(i)),
            _ => search(lo, hi, key, |i| self.on_line(i)),
        }
    }

    /// `lo..hi`'s positions whose value is `key`, the column sorted over that range: one search, and the end
    /// found from the start (a run's end, or a gallop over a key's few rows).
    #[inline]
    pub fn equal_range(&self, lo: usize, hi: usize, key: i64) -> (usize, usize) {
        if lo >= hi {
            return (lo, lo);
        }
        match self.tag {
            CONST => {
                let p = self.lower_bound(lo, hi, key);
                if self.base == key { (lo, hi) } else { (p, p) }
            }
            PACK => {
                let a = search(lo, hi, key, |i| self.val(i));
                if a == hi || self.val(a) != key { (a, a) } else { (a, upper_from(a, hi, key, |i| self.val(i))) }
            }
            RUNS => {
                let (ra, rb) = self.runs_over(lo, hi);
                let r = search(ra, rb, key, |k| self.val(k));
                let at = self.start(r).clamp(lo, hi);
                if r == rb || self.val(r) != key { (at, at) } else { (at, self.start(r + 1).min(hi)) }
            }
            GROUPS => {
                let a = search(lo, hi, key, |i| self.grp(i));
                if a == hi || self.grp(a) != key { (a, a) } else { (a, upper_from(a, hi, key, |i| self.grp(i))) }
            }
            DICT => {
                let a = search(lo, hi, key, |i| self.dct(i));
                if a == hi || self.dct(a) != key { (a, a) } else { (a, upper_from(a, hi, key, |i| self.dct(i))) }
            }
            _ => {
                let a = search(lo, hi, key, |i| self.on_line(i));
                if a == hi || self.on_line(a) != key { (a, a) } else { (a, upper_from(a, hi, key, |i| self.on_line(i))) }
            }
        }
    }
}

