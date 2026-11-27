//! A packed relation's format and its reads (decision 0456 K2; moved from `rerail_blocks::packed`, whose packer
//! chooses each column's encoding and writes it): its columns, whole or split at a bit, relative to column 0 where
//! that is smaller, its permutation indexes, and its rows' levels, each read in place.

use crate::col::{Dec, IntCol};
use crate::refs::Span;
use crate::structure;

structure! {
    /// A permutation index on one column that does not lead the order.
    pub struct PIndex {
        pub col: u32,
        pub pad: u32,
        /// The column's distinct values, sorted.
        pub keys: IntCol,
        /// Where each key's positions start in `perm` (one more entry than keys).
        pub starts: IntCol,
        /// Positions, grouped by key, in canonical order within a key.
        pub perm: IntCol,
    }
}

/// The bit a node id is split at (decision 0456 K2): an id is `document << 32 | slot << 18 | index` (decisions 0083
/// §3, 0094 §2), so above it is the id's granule, below it its index there. The engine's `granules::INDEX_BITS`,
/// which asserts they are equal.
pub const ID_SPLIT: u32 = 18;

structure! {
    /// A column of a packed relation (decision 0456 K2). `split` 0: its values whole, in `hi`. Else split at bit
    /// `split`: `hi` holds each value's `v >> split` and `lo` its `v & (2^split - 1)`, so a sorted id column is runs
    /// of granules and small indexes where a group of 128 ids would span documents. `rel` 1: each part is less
    /// column 0's same part at the row (an id near the row's first id, as a child is near its parent); column 0 is
    /// never relative, nor a column of an order column 0 does not lead (`Rel`).
    pub struct PCol {
        pub split: u32,
        pub rel: u32,
        pub hi: IntCol,
        pub lo: IntCol,
    }
}

structure! {
    pub struct Packed {
        pub arity: u32,
        pub n: u32,
        pub nokey: u32,
        pub nindex: u32,
        pub okey: [u16; 8],
        pub cols: Span<PCol>,
        pub indexes: Span<PIndex>,
        /// Each row's level (decision 0090), in the rows' order; empty when they have none (decision 0456 K1).
        pub levels: IntCol,
    }
}

impl Packed {
    #[inline]
    pub fn len(&self) -> usize {
        self.n as usize
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    #[inline]
    pub fn get(&self, pos: usize, c: usize) -> i64 {
        self.col(c).get(pos)
    }

    /// Column `c`, as it reads: its parts, and column 0 where it is relative to it.
    #[inline]
    pub fn col(&self, c: usize) -> Col<'_> {
        let cols = self.cols.as_slice();
        Col { col: &cols[c], base: (cols[c].rel != 0).then(|| &cols[0]) }
    }

    /// Its rows' levels, in their order; `None` when they have none.
    pub fn levels(&self) -> Option<&IntCol> {
        (self.n > 0 && self.levels.len() == self.len()).then_some(&self.levels)
    }

    /// Whether column `c` leads its canonical order: its rows equal to a value are one range (`col(c).equal_range`).
    pub fn leads(&self, c: usize) -> bool {
        self.nokey > 0 && self.okey[0] as usize == c
    }

    /// The positions whose columns `cols` equal `key`, in order: a range of the order's leading columns the key holds
    /// (each of the key's other columns checked within it: a relation's entry records its one sort column), else
    /// through the column's permutation index.
    pub fn probe(&self, cols: &[u16], key: &[i64]) -> Probe<'_> {
        let ok = &self.okey[..self.nokey as usize];
        let lead = ok.iter().take_while(|c| cols.contains(c)).count();
        if lead > 0 {
            let (mut lo, mut hi) = (0usize, self.len());
            for &oc in &ok[..lead] {
                let k = key[cols.iter().position(|&c| c == oc).unwrap()];
                (lo, hi) = self.col(oc as usize).dec().equal_range(lo, hi, k);
            }
            let mut rest = [(0u16, 0i64); crate::relation::MAX_KEY];
            let mut m = 0;
            for (&c, &k) in cols.iter().zip(key) {
                if !ok[..lead].contains(&c) {
                    rest[m] = (c, k);
                    m += 1;
                }
            }
            if m == 0 {
                return Probe::Range(lo..hi);
            }
            return Probe::Checked(self, lo..hi, rest, m);
        }
        assert_eq!(cols.len(), 1, "a key of several columns leads with the order's first");
        let ix = self.indexes.as_slice().iter().find(|ix| ix.col == cols[0] as u32).expect("an index on the column");
        let keys = ix.keys.dec();
        let k = keys.lower_bound(0, keys.len(), key[0]);
        if k >= keys.len() || keys.get(k) != key[0] {
            return Probe::Range(0..0);
        }
        let starts = ix.starts.dec();
        Probe::Perm(ix.perm.dec(), starts.get(k) as usize..starts.get(k + 1) as usize)
    }

    /// A digest of the rows: the same as the view's it was packed from (`View::digest`: a sum of each row's hash).
    pub fn digest(&self) -> u64 {
        let mut sum = 0u64;
        for p in 0..self.len() {
            sum = sum.wrapping_add(crate::hash::row_digest((0..self.arity as usize).map(|c| self.get(p, c))));
        }
        crate::hash::mix(sum, self.len() as i64)
    }
}

/// A column of a `Packed`, as it reads (`Packed::col`): its values whole or from their two parts, and column 0's
/// added where it is relative to it.
#[derive(Clone, Copy)]
pub struct Col<'a> {
    col: &'a PCol,
    base: Option<&'a PCol>,
}

impl<'a> Col<'a> {
    #[inline]
    pub fn len(&self) -> usize {
        self.col.hi.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn get(&self, i: usize) -> i64 {
        if self.col.split == 0 && self.base.is_none() { self.col.hi.get(i) } else { self.dec().get(i) }
    }

    /// The column resolved for reads.
    #[inline]
    pub fn dec(&self) -> ColDec<'a> {
        let parts = |c: &'a PCol| (c.hi.dec(), (c.split > 0).then(|| c.lo.dec()), c.split);
        ColDec { own: parts(self.col), base: self.base.map(parts) }
    }

    /// Every value in order.
    pub fn for_each(&self, mut f: impl FnMut(i64)) {
        if self.col.split == 0 && self.base.is_none() {
            self.col.hi.for_each(f)
        } else {
            let (d, n, mut buf) = (self.dec(), self.len(), [0i64; FILL]);
            for at in (0..n).step_by(FILL) {
                let m = (n - at).min(FILL);
                d.fill(at, &mut buf[..m]);
                buf[..m].iter().for_each(|&v| f(v));
            }
        }
    }

    /// Its parts' bytes.
    pub fn bytes(&self) -> usize {
        self.col.hi.bytes() + if self.col.split > 0 { self.col.lo.bytes() } else { 0 }
    }

    /// Its form: the encoding's name when whole, else `split`; `rel` after it when relative.
    pub fn form(&self) -> ColForm {
        ColForm { name: if self.col.split > 0 { "split" } else { self.col.hi.tag_name() }, rel: self.base.is_some() }
    }
}

/// A part pair resolved for reads: the high (or whole) part, the low when split, and the split.
type Parts<'a> = (Dec<'a>, Option<Dec<'a>>, u32);

/// The values `ColDec::fill` decodes at once: a chunk of rows read column by column, then row by row.
pub const FILL: usize = 256;

/// A `Col` resolved for reads: its parts' and column 0's loaded once.
#[derive(Clone, Copy)]
pub struct ColDec<'a> {
    own: Parts<'a>,
    base: Option<Parts<'a>>,
}

impl ColDec<'_> {
    #[inline]
    pub fn len(&self) -> usize {
        self.own.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline(always)]
    fn whole((hi, lo, s): &Parts<'_>, i: usize) -> i64 {
        match lo {
            None => hi.get(i),
            Some(lo) => hi.get(i) << s | lo.get(i),
        }
    }

    #[inline]
    pub fn get(&self, i: usize) -> i64 {
        let Some(base) = &self.base else { return Self::whole(&self.own, i) };
        let b = Self::whole(base, i);
        let (hi, lo, s) = &self.own;
        match lo {
            None => hi.get(i).wrapping_add(b),
            Some(lo) => hi.get(i).wrapping_add(b >> s) << s | (lo.get(i) + (b & ((1i64 << s) - 1))),
        }
    }

    /// `whole`'s values `at..at + out.len()`, `low` the low part's scratch.
    fn whole_fill((hi, lo, s): &Parts<'_>, at: usize, out: &mut [i64], low: &mut [i64]) {
        hi.fill(at, out);
        if let Some(lo) = lo {
            lo.fill(at, low);
            out.iter_mut().zip(low.iter()).for_each(|(o, l)| *o = *o << s | l);
        }
    }

    /// Values `at..at + out.len()` into `out`, in order, as `get` gives them: each part decoded as it goes
    /// (`Dec::fill`), at most `FILL` at once.
    pub fn fill(&self, at: usize, out: &mut [i64]) {
        let n = out.len();
        assert!(n <= FILL, "at most FILL values at once");
        let mut low = [0i64; FILL];
        let Some(base) = &self.base else { return Self::whole_fill(&self.own, at, out, &mut low[..n]) };
        let mut b = [0i64; FILL];
        Self::whole_fill(base, at, &mut b[..n], &mut low[..n]);
        let (hi, lo, s) = &self.own;
        hi.fill(at, out);
        match lo {
            None => out.iter_mut().zip(&b[..n]).for_each(|(o, b)| *o = o.wrapping_add(*b)),
            Some(lo) => {
                lo.fill(at, &mut low[..n]);
                let m = (1i64 << s) - 1;
                for ((o, b), l) in out.iter_mut().zip(&b[..n]).zip(&low[..n]) {
                    *o = o.wrapping_add(b >> s) << s | (l + (b & m));
                }
            }
        }
    }

    /// `key` as the column holds it over `lo..hi`: its high (or whole) and low parts, less column 0's when
    /// relative. A relative column is searched only where column 0 is one value (it follows column 0 in the order,
    /// `Rel::Searched`), so its values there are the column's less a constant, in the column's order. `Err(at)`:
    /// no value there can be `key`, which would go at `at`.
    #[inline]
    fn held(&self, lo: usize, hi: usize, key: i64) -> Result<(i64, i64), usize> {
        let (_, low, s) = &self.own;
        let m = (1i64 << s) - 1;
        let Some(base) = &self.base else { return Ok(if low.is_some() { (key >> s, key & m) } else { (key, 0) }) };
        if lo >= hi {
            return Err(lo);
        }
        let b = Self::whole(base, lo);
        debug_assert_eq!(Self::whole(base, hi - 1), b, "a relative column searched where column 0 varies");
        match low {
            Some(_) => Ok(((key >> s) - (b >> s), (key & m) - (b & m))),
            // Every value less `b` fits (`plan_col`), so a key whose difference does not lies past them all.
            None => key.checked_sub(b).map(|d| (d, 0)).ok_or(if key > b { hi } else { lo }),
        }
    }

    /// `lo..hi`'s positions whose value is `key`, the column sorted over that range: its high part's range, then
    /// the low part's within it.
    pub fn equal_range(&self, lo: usize, hi: usize, key: i64) -> (usize, usize) {
        let (kh, kl) = match self.held(lo, hi, key) {
            Ok(k) => k,
            Err(at) => return (at, at),
        };
        match &self.own {
            (h, None, _) => h.equal_range(lo, hi, kh),
            (h, Some(l), _) => {
                let (a, b) = h.equal_range(lo, hi, kh);
                l.equal_range(a, b, kl)
            }
        }
    }

    /// `equal_range` over the whole column, searched from `at` where every position before it is below `key`: a
    /// gallop forward from `at` (`at`, `at + 1`, `at + 2`, `at + 4`, …) to past `key`'s positions, then a search of
    /// that stretch. Keys probed in the column's order, each from the last's start, read it forward once (a merge
    /// join's inner side). Any `at` is correct: one past `key`'s positions, or past the end, takes the whole search.
    pub fn equal_range_from(&self, at: usize, key: i64) -> (usize, usize) {
        let n = self.len();
        if at > n || (at > 0 && self.get(at - 1) >= key) {
            return self.equal_range(0, n, key);
        }
        let (mut end, mut step) = (at, 1usize);
        while end < n && self.get(end) <= key {
            end = at + step;
            step <<= 1;
        }
        self.equal_range(at, end.min(n), key)
    }

    /// The first position in `lo..hi` whose value is not less than `key`, the column sorted over that range.
    pub fn lower_bound(&self, lo: usize, hi: usize, key: i64) -> usize {
        let (kh, kl) = match self.held(lo, hi, key) {
            Ok(k) => k,
            Err(at) => return at,
        };
        match &self.own {
            (h, None, _) => h.lower_bound(lo, hi, kh),
            (h, Some(l), _) => {
                let (a, b) = h.equal_range(lo, hi, kh);
                if a == b { a } else { l.lower_bound(a, b, kl) }
            }
        }
    }
}

pub enum Probe<'a> {
    Range(core::ops::Range<usize>),
    Perm(crate::col::Dec<'a>, core::ops::Range<usize>),
    /// A range of the order's leading columns, each position's other key columns checked (the first `usize`).
    Checked(&'a Packed, core::ops::Range<usize>, [(u16, i64); crate::relation::MAX_KEY], usize),
}

impl Iterator for Probe<'_> {
    type Item = usize;

    #[inline]
    fn next(&mut self) -> Option<usize> {
        match self {
            Probe::Range(r) => r.next(),
            Probe::Perm(p, r) => r.next().map(|i| p.get(i) as usize),
            Probe::Checked(p, r, rest, m) => r.find(|&i| rest[..*m].iter().all(|&(c, k)| p.get(i, c as usize) == k)),
        }
    }
}

/// A column's form (`Col::form`), written by `Display`: its encoding's name, or `split`, and ` rel` when relative.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColForm {
    pub name: &'static str,
    pub rel: bool,
}

impl core::fmt::Display for ColForm {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name)?;
        if self.rel { f.write_str(" rel") } else { Ok(()) }
    }
}
