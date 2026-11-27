//! `IntCol`'s encoder (moved under the flat layer from `rerail_blocks::col`, *Flat Views and Lifecycles* step 4): the
//! encoding chosen by exact size before anything is packed (MLDB's `columnSize`), then packed into words of exactly
//! that size. With no heap: a pass is over pieces of `PIECE` values ([`Pieces`]: one after another here, in parallel
//! by the host's driver in `rerail_blocks::col`), and `DICT`'s distinct values are counted and kept in a work arena
//! ([`distinct_in`]). The format and its reads are `crate::col`'s.

use super::arena::{Arena, Full};
use super::map::{Map, hash_word};
use crate::col::{CONST, DICT, GROUP, GROUP_LOG, GROUPS, LINE, PACK, RUNS, WIDTH_BITS, bits_for, words};
use core::ops::Range;
use core::sync::atomic::{AtomicU64, Ordering::Relaxed};

/// Values a piece of a pass holds. A multiple of 64 (and of `GROUP`), so a piece's values pack into whole words of
/// their own and hold whole groups.
pub const PIECE: usize = 1 << 16;

/// The pieces `0..n` is passed over in: at least one.
#[inline]
pub fn pieces(n: usize) -> usize {
    n.div_ceil(PIECE).max(1)
}

/// Piece `p` of `0..n`.
#[inline]
pub fn piece(p: usize, n: usize) -> Range<usize> {
    p * PIECE..((p + 1) * PIECE).min(n)
}

/// How a pass runs over its pieces: `f` on each piece of `0..n`, the results joined in order.
pub trait Pieces: Sync {
    fn fold<T: Send>(&self, n: usize, f: impl Fn(Range<usize>) -> T + Sync, join: impl Fn(T, T) -> T) -> T;

    /// A writer's pass that needs where each piece starts in its output: `each` writes a piece from its start (the
    /// pieces before it's counts, summed) and gives back its own count; `count` counts a piece without writing it, for
    /// a driver that writes the pieces at once (their counts first, then their sum before each: a scan).
    fn scan(&self, n: usize, count: impl Fn(Range<usize>) -> usize + Sync, each: impl Fn(Range<usize>, usize) -> usize + Sync);
}

/// One piece after another: a twin's, whose items are the parallel work, and a kernel's lane.
pub struct Serial;

impl Pieces for Serial {
    fn fold<T: Send>(&self, n: usize, f: impl Fn(Range<usize>) -> T + Sync, join: impl Fn(T, T) -> T) -> T {
        (1..pieces(n)).fold(f(piece(0, n)), |acc, p| join(acc, f(piece(p, n))))
    }

    /// One piece after another, each from the sum of the counts its writes gave: none counted twice.
    fn scan(&self, n: usize, _count: impl Fn(Range<usize>) -> usize + Sync, each: impl Fn(Range<usize>, usize) -> usize + Sync) {
        (0..pieces(n)).fold(0, |at, p| at + each(piece(p, n), at));
    }
}

/// A column's values as the encoder reads them, in place (decision 0353 step 8): `len` values, value `i` by `at`.
/// A slice is one; a relation's column read where it lies, in an order, is another (`packed::ColRead`).
pub trait Vals: Sync {
    fn len(&self) -> usize;
    fn at(&self, i: usize) -> i64;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Vals for [i64] {
    #[inline]
    fn len(&self) -> usize {
        <[i64]>::len(self)
    }
    #[inline]
    fn at(&self, i: usize) -> i64 {
        self[i]
    }
}

/// Anything that is a slice of values: a reference to one, a vector, a box.
impl<T: core::ops::Deref<Target = [i64]> + Sync> Vals for T {
    #[inline]
    fn len(&self) -> usize {
        (**self).len()
    }
    #[inline]
    fn at(&self, i: usize) -> i64 {
        (**self)[i]
    }
}

/// `u32`s as values (a string table's ends, a CSR's starts).
pub struct U32s<'a>(pub &'a [u32]);

impl Vals for U32s<'_> {
    #[inline]
    fn len(&self) -> usize {
        self.0.len()
    }
    #[inline]
    fn at(&self, i: usize) -> i64 {
        self.0[i] as i64
    }
}

/// What a column's packed values are taken from: `v[i] - base - floor(i * slope / 2^16) - rmin`, modulo 2^64
/// (exact, as each is under `2^bits`). PACK is the line of slope 0.
#[derive(Clone, Copy)]
pub struct Line {
    pub base: i64,
    pub slope: i64,
    pub rmin: i64,
}

/// Words a writer of disjoint words of one slice reaches from every piece: the slice's address and length.
#[derive(Clone, Copy)]
struct Words(*mut u64, usize);

// SAFETY: each piece writes only its own words (`pack_resid`), so sharing the address across threads is sound.
unsafe impl Sync for Words {}
unsafe impl Send for Words {}

impl Words {
    /// Words `from..` (to the end for the last piece, else `to`).
    #[allow(clippy::mut_from_ref)]
    unsafe fn part(&self, from: usize, to: usize) -> &mut [u64] {
        debug_assert!(from <= to && to <= self.1);
        unsafe { core::slice::from_raw_parts_mut(self.0.add(from), to - from) }
    }
}

/// Packs `v`'s values less `line` into `out` (zero, `words(v.len(), bits)` long), the residuals computed as they
/// are packed (no pass to a temporary). A piece of `PIECE` values is `PIECE * bits / 64` whole words, so pieces
/// share no word.
pub fn pack_resid<V: Vals + ?Sized, P: Pieces>(out: &mut [u64], v: &V, line: Line, bits: u32, p: &P) {
    if bits == 0 {
        return;
    }
    // A line of slope 0 is its base and minimum together: the packer with no line to subtract.
    let line = if line.slope == 0 { Line { base: line.base.wrapping_add(line.rmin), slope: 0, rmin: 0 } } else { line };
    let n = v.len();
    let (per, k) = (PIECE * bits as usize / 64, pieces(n));
    let w = Words(out.as_mut_ptr(), out.len());
    p.fold(
        n,
        |r| {
            let at = r.start / PIECE;
            let to = if at + 1 == k { w.1 } else { (at + 1) * per };
            // SAFETY: piece `at`'s own words (the last piece's run to the end, the spare word with them).
            pack_piece(unsafe { w.part(at * per, to) }, v, r, line, bits)
        },
        |(), ()| (),
    );
}

/// Packs `vals` (each under `2^bits`) into `out`, which is zero and `words(vals.len(), bits)` long.
pub fn pack_into<P: Pieces>(out: &mut [u64], vals: &[u64], bits: u32, p: &P) {
    // SAFETY: `u64` and `i64` have one layout; the packer reads `v - 0` modulo 2^64, the same bits.
    let v = unsafe { core::slice::from_raw_parts(vals.as_ptr().cast::<i64>(), vals.len()) };
    pack_resid(out, v, Line { base: 0, slope: 0, rmin: 0 }, bits, p);
}

/// Packs `v`'s values at `r` (`r.start` a multiple of 64) into `out` from its first bit: a packer per width, and
/// per whether there is a line to subtract.
pub fn pack_piece<V: Vals + ?Sized>(out: &mut [u64], v: &V, r: Range<usize>, line: Line, bits: u32) {
    macro_rules! widths {
        ($($b:literal)*) => {
            match (bits, line.slope != 0) {
                $(($b, false) => pack_fixed::<$b, false, V>(out, v, r, line),
                  ($b, true) => pack_fixed::<$b, true, V>(out, v, r, line),)*
                _ => unreachable!("{bits} bits"),
            }
        };
    }
    widths!(1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32
            33 34 35 36 37 38 39 40 41 42 43 44 45 46 47 48 49 50 51 52 53 54 55 56 57 58 59 60 61 62 63 64);
}

/// 64 values of `B` bits make `B` whole words, so a group of 64 is packed by a loop whose shifts and word
/// boundaries are constants once unrolled; the tail goes value by value.
#[inline(never)]
fn pack_fixed<const B: usize, const L: bool, V: Vals + ?Sized>(out: &mut [u64], v: &V, r: Range<usize>, line: Line) {
    let from = r.start;
    // The line at the next value, `(from + j) * slope`, kept as a sum (the values go in order).
    let mut acc = from as i128 * line.slope as i128;
    let mut res = |x: i64| -> u64 {
        let x = x.wrapping_sub(line.base);
        if L {
            let t = (acc >> 16) as i64;
            acc += line.slope as i128;
            x.wrapping_sub(t).wrapping_sub(line.rmin) as u64
        } else {
            x as u64
        }
    };
    let n = r.len();
    let groups = n / 64;
    for g in 0..groups {
        let o: &mut [u64] = &mut out[g * B..g * B + B];
        for j in 0..64 {
            let x = res(v.at(from + g * 64 + j));
            let bit = j * B;
            let (w, sh) = (bit / 64, bit % 64);
            o[w] |= x << sh;
            if sh + B > 64 {
                o[w + 1] |= x >> (64 - sh);
            }
        }
    }
    for i in groups * 64..n {
        let x = res(v.at(from + i));
        let bit = i * B;
        let (w, sh) = (bit / 64, bit % 64);
        out[w] |= x << sh;
        if sh + B > 64 {
            out[w + 1] |= x >> (64 - sh);
        }
    }
}

/// Appends values of `bits` bits to zeroed words from a bit position, a word stored once it is full. Words are
/// ORed in atomically, so writers starting at other positions in the same words (a parallel pass's pieces, whose
/// first and last words they may share) never lose bits.
pub struct BitW<'a> {
    out: &'a [AtomicU64],
    w: usize,
    fill: u32,
    acc: u64,
    bits: u32,
}

impl BitW<'_> {
    pub fn new(out: &[AtomicU64], bit: usize, bits: u32) -> BitW<'_> {
        BitW { out, w: bit / 64, fill: (bit % 64) as u32, acc: 0, bits }
    }

    #[inline]
    pub fn push(&mut self, x: u64) {
        self.acc |= x << self.fill;
        let end = self.fill + self.bits;
        if end >= 64 {
            self.out[self.w].fetch_or(self.acc, Relaxed);
            self.w += 1;
            self.acc = if self.fill == 0 { 0 } else { x >> (64 - self.fill) };
            self.fill = end - 64;
        } else {
            self.fill = end;
        }
    }

    pub fn flush(self) {
        if self.fill > 0 {
            self.out[self.w].fetch_or(self.acc, Relaxed);
        }
    }
}

/// `words` as atomics, for writers that may share a word.
pub fn atomics(words: &mut [u64]) -> &[AtomicU64] {
    // SAFETY: `AtomicU64` has `u64`'s size and alignment, and the borrow is exclusive.
    unsafe { &*(core::ptr::from_mut(words) as *const [AtomicU64]) }
}

/// A column's encoding, chosen by exact size before anything is packed: so a block can be made exactly the size
/// of what it will hold, and each column packed straight into it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Choice {
    pub tag: u32,
    pub bits: u32,
    pub rbits: u32,
    pub nruns: u32,
    pub base: i64,
    pub slope: i64,
    pub rmin: i64,
    pub n: usize,
    pub data_words: usize,
    pub runs_words: usize,
}

impl Choice {
    /// The bytes it takes in a block, besides its descriptor.
    pub fn bytes(&self) -> usize {
        if self.tag == CONST { 0 } else { (self.data_words + self.runs_words) * 8 }
    }
}

/// `v[i]`'s residual from the line through the first value at `slope`: `v[i] - (v[0] + i * slope / 2^16)`.
#[inline]
pub fn resid<V: Vals + ?Sized>(v: &V, slope: i64, i: usize) -> i128 {
    v.at(i) as i128 - (v.at(0) as i128 + ((i as i128 * slope as i128) >> 16))
}

/// The least and greatest of `v`'s values at `r` (not empty).
#[inline]
pub fn range_of<V: Vals + ?Sized>(v: &V, r: Range<usize>) -> (i64, i64) {
    let first = v.at(r.start);
    r.fold((first, first), |(a, b), i| {
        let x = v.at(i);
        (a.min(x), b.max(x))
    })
}

/// The groups of `GROUP` values in `r` (`r.start` a multiple of `GROUP`), each its range of positions.
pub fn groups_of(r: Range<usize>) -> impl Iterator<Item = Range<usize>> {
    let end = r.end;
    r.step_by(GROUP).map(move |g| g..(g + GROUP).min(end))
}

/// The runs that start in `r`: a position whose value differs from the one before it, and position 0.
pub fn runs_in<V: Vals + ?Sized>(v: &V, r: Range<usize>) -> usize {
    let from = r.start.max(1);
    (r.start == 0) as usize + (from..r.end).filter(|&i| v.at(i) != v.at(i - 1)).count()
}

/// The line's residuals over `v`: the words they pack in, their bits and their minimum, or `usize::MAX` words
/// when the line cannot beat `bits` a value. `sample` is the residuals' range at every 64th row (taken in the
/// range's read): it is within the full one, so when it already needs `bits`, the full pass is skipped. The pass
/// runs in pieces, in `i64` when `slope * n` and the values (`lo..=hi`) cannot overflow it (vectorized), else in
/// `i128`.
pub fn line_fit<V: Vals + ?Sized, P: Pieces>(v: &V, slope: i64, bits: u32, lo: i64, hi: i64, sample: (i128, i128), p: &P) -> (usize, u32, i64) {
    let n = v.len();
    let no = (usize::MAX, 64, 0);
    let (slo, shi) = sample;
    if shi - slo > u64::MAX as i128 || bits_for((shi - slo) as u64) >= bits {
        return no;
    }
    let small = (slope as i128).abs() * n as i128 <= (1i128 << 62) && lo > -(1 << 61) && hi < 1 << 61;
    let (rlo, rhi) = p.fold(
        n,
        |r| {
            if small {
                let v0 = v.at(0);
                let (mut lo, mut hi, mut t) = (i64::MAX, i64::MIN, r.start as i64 * slope);
                for x in r.map(|i| v.at(i)) {
                    let d = x - (v0 + (t >> 16));
                    lo = lo.min(d);
                    hi = hi.max(d);
                    t += slope;
                }
                (lo as i128, hi as i128)
            } else {
                r.map(|i| resid(v, slope, i)).fold((i128::MAX, i128::MIN), |(a, b), d| (a.min(d), b.max(d)))
            }
        },
        |(a, b), (c, d)| (a.min(c), b.max(d)),
    );
    let range = rhi - rlo;
    if range > u64::MAX as i128 || rlo < i64::MIN as i128 {
        return no;
    }
    let lbits = bits_for(range as u64);
    (words(n, lbits), lbits, rlo as i64)
}

/// The most distinct values a `DICT` of `n` values `bits` wide (their range's) may have and still take fewer words
/// than `best`: over each code width `cb` below `bits`, the values its codes can name and the dictionary's words can
/// hold. 0 where none would.
pub fn dict_limit(n: usize, bits: u32, best: usize) -> usize {
    (1..bits.min(32))
        .filter(|&cb| words(n, cb) < best)
        .map(|cb| (1usize << cb).min((best - words(n, cb) - 1) * 64 / bits as usize))
        .max()
        .unwrap_or(0)
}

/// What a column's pass found, from which every encoding's exact size follows.
#[derive(Clone, Copy, Debug)]
pub struct Scan {
    pub n: usize,
    /// The first value and the line's slope (through the first and last), in 1/65,536ths a row.
    pub first: i64,
    pub slope: i64,
    pub lo: i64,
    pub hi: i64,
    pub nruns: usize,
    /// `GROUPS`: the groups' summed widths, and the greatest of their minima.
    pub gbits: usize,
    pub ghi: i64,
    /// `LINE`: its words, bits and residuals' minimum (`usize::MAX` words: the line cannot win).
    pub line: (usize, u32, i64),
}

/// Every encoding's size for a scan: the smallest picked by `pick`, `DICT` tried against it by `dict`.
#[derive(Clone, Copy, Debug)]
pub struct Sizes {
    s: Scan,
    /// A constant column's choice, once `lo == hi` or there are no values.
    done: Option<Choice>,
    bits: u32,
    pack: usize,
    rbits: u32,
    runs: (usize, usize),
    gb: u32,
    eb: u32,
    groups: (usize, usize),
    /// The fewest words of PACK, RUNS, LINE and GROUPS.
    pub best: usize,
}

impl Scan {
    /// Each encoding's exact size; `RUNS` only with `max_runs` runs or fewer.
    pub fn sizes(&self, max_runs: usize) -> Sizes {
        let n = self.n;
        let none = Choice { tag: CONST, bits: 0, rbits: 0, nruns: 0, base: 0, slope: 0, rmin: 0, n, data_words: 1, runs_words: 0 };
        let mut z = Sizes { s: *self, done: None, bits: 0, pack: 0, rbits: 0, runs: (0, 0), gb: 0, eb: 0, groups: (0, 0), best: 0 };
        if n == 0 {
            z.done = Some(none);
            return z;
        }
        if self.lo == self.hi {
            z.done = Some(Choice { base: self.lo, ..none });
            return z;
        }
        let bits = bits_for((self.hi as i128 - self.lo as i128) as u64);
        let pack = words(n, bits);
        let rbits = bits_for(n as u64);
        let runs = if self.nruns > max_runs { (usize::MAX / 4, usize::MAX / 4) } else { (words(self.nruns, bits), words(self.nruns, rbits)) };
        // `GROUPS`: the residuals (two spare words: a group of width 0 at the end reads at the last bit), each group's
        // minimum less the column's (`lo`, the least of them), and each group's entry (width, offset).
        let ngroups = n.div_ceil(GROUP);
        let (gb, eb) = (bits_for((self.ghi as i128 - self.lo as i128) as u64), WIDTH_BITS + bits_for(self.gbits as u64));
        let groups = (self.gbits / 64 + 2, words(ngroups, gb) + words(ngroups, eb));
        let best = pack.min(runs.0 + runs.1).min(self.line.0).min(groups.0 + groups.1);
        Sizes { s: *self, done: None, bits, pack, rbits, runs, gb, eb, groups, best }
    }
}

impl Sizes {
    /// A constant column's choice (no values, or one value): nothing else to weigh.
    pub fn constant(&self) -> Option<Choice> {
        self.done
    }

    /// The most distinct values a `DICT` could have and win (0: it cannot).
    pub fn dict_limit(&self) -> usize {
        if self.done.is_some() { 0 } else { dict_limit(self.s.n, self.bits, self.best) }
    }

    /// `DICT` of `distinct` values, where it is smaller than every other encoding.
    pub fn dict(&self, distinct: usize) -> Option<Choice> {
        if self.done.is_some() || distinct < 2 {
            return None;
        }
        let (n, bits) = (self.s.n, self.bits);
        let cb = bits_for(distinct as u64 - 1);
        let (dw, vw) = (words(n, cb), words(distinct, bits));
        (dw + vw < self.best).then_some(Choice { tag: DICT, bits, rbits: cb, nruns: distinct as u32, base: self.s.lo, slope: 0, rmin: 0, n, data_words: dw, runs_words: vw })
    }

    /// The smallest of PACK, RUNS, LINE and GROUPS (or the constant).
    pub fn pick(&self) -> Choice {
        if let Some(c) = self.done {
            return c;
        }
        let (s, n, bits, best) = (&self.s, self.s.n, self.bits, self.best);
        let (line_words, lbits, rlo) = s.line;
        if best == self.groups.0 + self.groups.1 && best < self.pack.min(self.runs.0 + self.runs.1).min(line_words) {
            return Choice { tag: GROUPS, bits: self.gb, rbits: self.eb, nruns: n.div_ceil(GROUP) as u32, base: s.lo, slope: 0, rmin: 0, n, data_words: self.groups.0, runs_words: self.groups.1 };
        }
        if best == self.runs.0 + self.runs.1 {
            return Choice { tag: RUNS, bits, rbits: self.rbits, nruns: s.nruns as u32, base: s.lo, slope: 0, rmin: 0, n, data_words: self.runs.0, runs_words: self.runs.1 };
        }
        if best == line_words && line_words < self.pack {
            return Choice { tag: LINE, bits: lbits, rbits: 0, nruns: 0, base: s.first, slope: s.slope, rmin: rlo, n, data_words: line_words, runs_words: 0 };
        }
        Choice { tag: PACK, bits, rbits: 0, nruns: 0, base: s.lo, slope: 0, rmin: 0, n, data_words: self.pack, runs_words: 0 }
    }
}

/// The runs past which a column read at random takes no `RUNS`: a read's search of their starts is then 5 steps at
/// most.
pub const RUNS_FOR_READS: usize = 32;

/// `v`'s pass: the range and the run count in one branch-free pass a piece (vectorized), and the line's residuals at
/// every 64th row (LINE's sample) in the same read; the range a group at a time (a piece is whole groups), so
/// `GROUPS`' widths, summed, and its minima's range come from the same read; then the line's pass where the sample
/// says it may win.
pub fn scan<V: Vals + ?Sized, P: Pieces>(v: &V, p: &P) -> Scan {
    let n = v.len();
    if n == 0 {
        return Scan { n, first: 0, slope: 0, lo: 0, hi: 0, nruns: 0, gbits: 0, ghi: 0, line: (usize::MAX, 64, 0) };
    }
    // The line through the first and last values (LINE's slope), known before any pass.
    let slope = if n > 1 { (((v.at(n - 1) as i128 - v.at(0) as i128) << 16) / (n as i128 - 1)) as i64 } else { 0 };
    let (lo, hi, nruns, sample, gbits, ghi) = p.fold(
        n,
        |r| {
            let (mut a, mut b, mut gbits, mut ghi) = (i64::MAX, i64::MIN, 0usize, i64::MIN);
            for g in groups_of(r.clone()) {
                let len = g.len();
                let (x, y) = range_of(v, g);
                (a, b, ghi) = (a.min(x), b.max(y), ghi.max(x));
                gbits += len * bits_for((y as i128 - x as i128) as u64) as usize;
            }
            let runs = runs_in(v, r.clone());
            let sample = (r.start.div_ceil(64) * 64..r.end).step_by(64).map(|i| resid(v, slope, i)).fold((i128::MAX, i128::MIN), |(a, b), d| (a.min(d), b.max(d)));
            (a, b, runs, sample, gbits, ghi)
        },
        |(a, b, r, s, g, h), (c, d, t, u, gb, gh)| (a.min(c), b.max(d), r + t, (s.0.min(u.0), s.1.max(u.1)), g + gb, h.max(gh)),
    );
    let line = if lo == hi { (usize::MAX, 64, 0) } else { line_fit(v, slope, bits_for((hi as i128 - lo as i128) as u64), lo, hi, sample, p) };
    Scan { n, first: v.at(0), slope, lo, hi, nruns, gbits, ghi, line }
}

/// The smallest encoding of `v`: `RUNS` only with `max_runs` runs or fewer, and `DICT` where `distinct(limit)`
/// counts `v`'s distinct values up to `limit` (`None` past it) and it is smallest.
pub fn choose<V: Vals + ?Sized, P: Pieces>(v: &V, max_runs: usize, p: &P, distinct: Option<&mut dyn FnMut(usize) -> Option<usize>>) -> Choice {
    let z = scan(v, p).sizes(max_runs);
    if let Some(count) = distinct {
        let limit = z.dict_limit();
        if limit >= 2
            && let Some(d) = count(limit)
            && let Some(c) = z.dict(d)
        {
            return c;
        }
    }
    z.pick()
}

/// `v`'s distinct values, sorted, where there are `limit` or fewer (`None` past it: the count stops there), counted
/// in a map in `work` and kept there. The map's keys are row indexes, hashed and compared by their values
/// (`Map::claim_by`): 32-bit words, as a kernel claims them. Its claims are at most `distinct_bytes(v.len())`.
pub fn distinct_in<'a, V: Vals + ?Sized>(v: &V, limit: usize, work: &'a Arena<'_>, part: u32) -> Result<Option<&'a [i64]>, Full> {
    let n = v.len();
    if limit < 2 || n == 0 {
        return Ok(None);
    }
    let limit = limit.min(n);
    let m = Map::<u32, u32>::make(work, part, limit + 1, u32::MAX, 0)?;
    for i in 0..n {
        let x = v.at(i);
        m.claim_by(i as u32, hash_word(x), |j| v.at(j as usize) == x)?;
        if m.len() > limit {
            return Ok(None);
        }
    }
    let d = work.claim::<i64>(part, m.len())?;
    for (k, (j, _)) in m.iter().enumerate() {
        d[k].write(v.at(j as usize));
    }
    // SAFETY: every item written.
    let d = unsafe { &mut *(core::ptr::from_mut(d) as *mut [i64]) };
    d.sort_unstable();
    Ok(Some(&*d))
}

/// The most bytes `distinct_in` claims for `n` values, whatever its limit: its map's header and slots, and the
/// values kept, each claim's room as the arena rounds it.
pub fn distinct_bytes(n: usize) -> usize {
    let slots = super::map::slots_for(n + 1);
    size_of::<Map<u32, u32>>().next_multiple_of(64) + 2 * (4 * slots).next_multiple_of(64) + (8 * n).next_multiple_of(64) + 64
}

/// Packs `v` as `c` chose into `data` and `runs` (clear, `c.data_words` and `c.runs_words` long), in pieces as `p`
/// runs them (one after another, or at once on the host's cores). `dict`: `DICT`'s distinct values, sorted
/// (`distinct_in`); empty for every other encoding. Pieces share no word but at their edges, where their writers OR
/// their bits in (`BitW`); `RUNS` and `GROUPS` write a piece from where the pieces before it end (`Pieces::scan`).
pub fn write<V: Vals + ?Sized, P: Pieces>(c: &Choice, v: &V, data: &mut [u64], runs: &mut [u64], dict: &[i64], p: &P) {
    let n = v.len();
    match c.tag {
        CONST => {}
        PACK => pack_resid(data, v, Line { base: c.base, slope: 0, rmin: 0 }, c.bits, p),
        LINE => pack_resid(data, v, Line { base: c.base, slope: c.slope, rmin: c.rmin }, c.bits, p),
        RUNS => {
            let (d, s) = (atomics(data), atomics(runs));
            // A piece's runs start at the runs before it.
            p.scan(n, |r| runs_in(v, r), |r, k| {
                let (mut wv, mut ws) = (BitW::new(d, k * c.bits as usize, c.bits), BitW::new(s, k * c.rbits as usize, c.rbits));
                let mut m = 0;
                for i in r {
                    let x = v.at(i);
                    if i == 0 || x != v.at(i - 1) {
                        wv.push(x.wrapping_sub(c.base) as u64);
                        ws.push(i as u64);
                        m += 1;
                    }
                }
                wv.flush();
                ws.flush();
                m
            });
        }
        GROUPS => {
            // Each group's minimum and width, its residuals at the running offset, its minimum and entry after; a
            // piece's groups (whole ones: `PIECE` is a multiple of `GROUP`) from the bits the groups before it take.
            let entries = words(n.div_ceil(GROUP), c.bits) * 64;
            let (d, s) = (atomics(data), atomics(runs));
            let width = |g: Range<usize>| {
                let (x, y) = range_of(v, g);
                (x, bits_for((y as i128 - x as i128) as u64))
            };
            let count = |r: Range<usize>| groups_of(r).map(|g| width(g.clone()).1 as usize * g.len()).sum();
            p.scan(n, count, |r, from| {
                let k0 = r.start >> GROUP_LOG;
                let (mut mins, mut ents) = (BitW::new(s, k0 * c.bits as usize, c.bits), BitW::new(s, entries + k0 * c.rbits as usize, c.rbits));
                let mut at = from;
                for g in groups_of(r) {
                    let (gmin, w) = width(g.clone());
                    if c.bits > 0 {
                        mins.push(gmin.wrapping_sub(c.base) as u64);
                    }
                    ents.push((at as u64) << WIDTH_BITS | w as u64);
                    if w > 0 {
                        let mut out = BitW::new(d, at, w);
                        for i in g.clone() {
                            out.push(v.at(i).wrapping_sub(gmin) as u64);
                        }
                        out.flush();
                    }
                    at += w as usize * g.len();
                }
                mins.flush();
                ents.flush();
                at - from
            });
        }
        DICT => {
            assert_eq!(dict.len(), c.nruns as usize, "a DICT column's distinct values");
            pack_resid(runs, dict, Line { base: c.base, slope: 0, rmin: 0 }, c.bits, p);
            let out = atomics(data);
            p.fold(
                n,
                |r| {
                    let mut w = BitW::new(out, r.start * c.rbits as usize, c.rbits);
                    for i in r {
                        w.push(dict.binary_search(&v.at(i)).expect("a value among the column's distinct ones") as u64);
                    }
                    w.flush();
                },
                |(), ()| (),
            );
        }
        t => panic!("an IntCol of tag {t}"),
    }
}

/// What `choose` reads of a column, gathered a value at a time in one pass (`push`), for a reader that reads several
/// forms of one column at once (`packed`'s candidates: whole, split, less column 0's), each value read once
/// (decision 0353 step 8: the values are read where they lie). A pass runs in pieces, each started with the value
/// before it (`piece`), and the pieces are joined in order (`join`). `choice` is `choose`'s.
#[derive(Clone, Copy, Debug)]
pub struct Stats {
    first: i64,
    slope: i64,
    lo: i64,
    hi: i64,
    runs: usize,
    prev: Option<i64>,
    /// The groups' summed widths, and the greatest of their minima.
    gbits: usize,
    ghi: i64,
    /// The open group's range and size.
    glo: i64,
    gmax: i64,
    gn: usize,
    /// The residuals' range from the line through the first value at `slope`.
    rlo: i128,
    rhi: i128,
}

impl Stats {
    /// The statistics of a piece from position `start` of a column of `n` values whose first and last are `first`
    /// and `last`; `before`: the value at `start - 1` (none at 0).
    pub fn piece(first: i64, last: i64, n: usize, before: Option<i64>) -> Stats {
        let slope = if n > 1 { (((last as i128 - first as i128) << 16) / (n as i128 - 1)) as i64 } else { 0 };
        Stats { first, slope, lo: i64::MAX, hi: i64::MIN, runs: 0, prev: before, gbits: 0, ghi: i64::MIN, glo: i64::MAX, gmax: i64::MIN, gn: 0, rlo: i128::MAX, rhi: i128::MIN }
    }

    /// Value `x` at position `i`, the next after those pushed (a piece starts at a group's start).
    #[inline]
    pub fn push(&mut self, i: usize, x: i64) {
        (self.lo, self.hi) = (self.lo.min(x), self.hi.max(x));
        if self.prev != Some(x) {
            self.runs += 1;
        }
        self.prev = Some(x);
        (self.glo, self.gmax, self.gn) = (self.glo.min(x), self.gmax.max(x), self.gn + 1);
        if self.gn == GROUP {
            self.close_group();
        }
        let d = x as i128 - (self.first as i128 + ((i as i128 * self.slope as i128) >> 16));
        (self.rlo, self.rhi) = (self.rlo.min(d), self.rhi.max(d));
    }

    fn close_group(&mut self) {
        if self.gn > 0 {
            self.gbits += self.gn * bits_for((self.gmax as i128 - self.glo as i128) as u64) as usize;
            self.ghi = self.ghi.max(self.glo);
            (self.glo, self.gmax, self.gn) = (i64::MAX, i64::MIN, 0);
        }
    }

    /// This piece's statistics and the next one's, joined (a piece holds whole groups but the last).
    pub fn join(mut self, mut o: Stats) -> Stats {
        self.close_group();
        o.close_group();
        Stats {
            lo: self.lo.min(o.lo),
            hi: self.hi.max(o.hi),
            runs: self.runs + o.runs,
            prev: o.prev.or(self.prev),
            gbits: self.gbits + o.gbits,
            ghi: self.ghi.max(o.ghi),
            rlo: self.rlo.min(o.rlo),
            rhi: self.rhi.max(o.rhi),
            ..self
        }
    }

    /// The least and greatest value pushed.
    pub fn range(&self) -> (i64, i64) {
        (self.lo, self.hi)
    }

    /// The scan of the `n` values pushed: the line's residuals exact.
    pub fn scan(mut self, n: usize) -> Scan {
        self.close_group();
        // No residual pushed (a constant column's statistics, read for its range alone): no line.
        let range = self.rhi.checked_sub(self.rlo).filter(|&r| r >= 0).unwrap_or(i128::MAX);
        let line = if n == 0 || range > u64::MAX as i128 || self.rlo < i64::MIN as i128 {
            (usize::MAX, 64, 0)
        } else {
            let lbits = bits_for(range as u64);
            (words(n, lbits), lbits, self.rlo as i64)
        };
        Scan { n, first: self.first, slope: self.slope, lo: self.lo, hi: self.hi, nruns: self.runs, gbits: self.gbits, ghi: self.ghi, line }
    }

    /// The encoding `choose` takes for the `n` values pushed.
    pub fn choice(self, n: usize) -> Choice {
        self.scan(n).sizes(usize::MAX).pick()
    }
}
