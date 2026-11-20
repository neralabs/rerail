//! Columns packed per range of rows: one block holding a relation's columns, each cut into ranges of
//! `RANGE` rows, each range an `IntCol` (MLDB's `MappedIntTable` ranges over its `RawMappedIntTable` choices).
//!
//! A column chosen whole takes the bits its widest stretch needs everywhere. Per range, a sorted column's values
//! are close within a range and its runs short, so each range takes only what it needs. A column may also be split
//! at a bit (`split`): its value is `hi << split | lo`, the two parts packed apart, each per range. A node id
//! (`document << 32 | slot << 18 | index`) split at 18 is runs of its granule and a small index.
//!
//! Reading a value is a shift to its range, then the range's extract: in place, as on a device.
//!
//! ```text
//! Columns { arity, n, cols: [RangedCol; arity] }
//! RangedCol { n, split, parts, ranges: [IntCol; ranges × parts] }   // parts: 1, or 2 (hi, lo) when split
//! value(c, i) = parts == 1 ? ranges[i >> 12].get(i & 4095)
//!                          : ranges[2k].get(j) << split | ranges[2k + 1].get(j)    // k = i >> 12, j = i & 4095
//! ```

use crate::block::Block;
use crate::col::{Choice, IntCol, choose, write};
use crate::refs::Span;
use crate::structure;
use rayon::prelude::*;

/// Rows a range holds: a power of two, so a row's range is a shift.
pub const RANGE_BITS: u32 = 12;
pub const RANGE: usize = 1 << RANGE_BITS;

structure! {
    /// One column, packed per range of `RANGE` rows.
    pub struct RangedCol {
        pub n: u32,
        /// 0: the value packed whole; else the bit it is split at, its parts packed apart.
        pub split: u32,
        /// 1, or 2 when split: the high part's range, then the low part's.
        pub parts: u32,
        pub pad: u32,
        pub ranges: Span<IntCol>,
    }
}

structure! {
    /// A relation's columns, the block's root.
    pub struct Columns {
        pub arity: u32,
        pub n: u32,
        pub cols: Span<RangedCol>,
    }
}

/// Column `v`'s part `p` of its range `r` (`split` 0: the value whole), into `out`.
fn part(v: &[i64], r: usize, split: u32, p: usize, out: &mut Vec<i64>) {
    let range = &v[r * RANGE..((r + 1) * RANGE).min(v.len())];
    out.clear();
    match (split, p) {
        (0, _) => out.extend_from_slice(range),
        (s, 0) => out.extend(range.iter().map(|&x| x >> s)),
        (s, _) => out.extend(range.iter().map(|&x| x & ((1i64 << s) - 1))),
    }
}

/// Packs `cols` (each `n` values, rows in the order given) into a new block of its exact size; column `c` split at
/// `split[c]` (0: whole). The block's root is a `Columns`.
pub fn pack(cols: &[&[i64]], split: &[u32], kind: u32) -> Block {
    let arity = cols.len();
    let n = cols.first().map_or(0, |c| c.len());
    assert!(cols.iter().all(|c| c.len() == n), "columns of one length");
    assert!(split.len() == arity && split.iter().all(|&s| s < 64), "a split bit per column");
    let nranges = n.div_ceil(RANGE);
    let parts = |c: usize| if split[c] == 0 { 1 } else { 2 };
    // Each column's choices, range by range (parts in turn), in parallel by column.
    let choices: Vec<Vec<Choice>> = rerail_context::on_cores(|| {
        (0..arity)
            .into_par_iter()
            .map(|c| {
                let mut buf = Vec::with_capacity(RANGE);
                (0..nranges)
                    .flat_map(|r| (0..parts(c)).map(move |p| (r, p)))
                    .map(|(r, p)| {
                        part(cols[c], r, split[c], p, &mut buf);
                        choose(&buf)
                    })
                    .collect()
            })
            .collect()
    });
    let words: usize = choices.iter().flatten().map(|ch| ch.bytes() + 2 * 64).sum();
    let descr = size_of::<Columns>() + arity * size_of::<RangedCol>() + choices.iter().map(Vec::len).sum::<usize>() * size_of::<IntCol>();
    let block = crate::alloc::host(kind, descr + words + 512);
    let root: *mut Columns = block.alloc_t(1);
    let rc: *mut RangedCol = block.alloc_t(arity.max(1));
    let mut jobs: Vec<(usize, usize, &Choice, &mut [u64], &mut [u64])> = Vec::new();
    unsafe {
        (*root).arity = arity as u32;
        (*root).n = n as u32;
        for (c, ch) in choices.iter().enumerate() {
            let col = rc.add(c);
            (*col).n = n as u32;
            (*col).split = split[c];
            (*col).parts = parts(c) as u32;
            let ic: *mut IntCol = block.alloc_t(ch.len().max(1));
            for (k, choice) in ch.iter().enumerate() {
                let (d, r) = IntCol::lay_out(ic.add(k), &block, choice);
                jobs.push((c, k, choice, d, r));
            }
            Span::set(&raw mut (*col).ranges, ic, ch.len());
        }
        Span::set(&raw mut (*root).cols, rc, arity);
    }
    rerail_context::on_cores(|| {
        jobs.into_par_iter().for_each_init(
            || Vec::with_capacity(RANGE),
            |buf, (c, k, choice, d, r)| {
                let p = parts(c);
                part(cols[c], k / p, split[c], k % p, buf);
                write(choice, buf, d, r);
            },
        )
    });
    block.set_root(root);
    block
}

impl Columns {
    #[inline]
    pub fn len(&self) -> usize {
        self.n as usize
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    #[inline]
    pub fn col(&self, c: usize) -> &RangedCol {
        &self.cols.as_slice()[c]
    }

    #[inline]
    pub fn get(&self, row: usize, c: usize) -> i64 {
        self.col(c).get(row)
    }
}

impl RangedCol {
    #[inline]
    pub fn len(&self) -> usize {
        self.n as usize
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    #[inline]
    pub fn get(&self, i: usize) -> i64 {
        let (k, j) = (i >> RANGE_BITS, i & (RANGE - 1));
        let ranges = self.ranges.as_slice();
        if self.parts == 1 {
            ranges[k].get(j)
        } else {
            ranges[2 * k].get(j) << self.split | ranges[2 * k + 1].get(j)
        }
    }

    /// Every value from row `from` in order, into `out` (as many as it holds, or to the end): a range's words read
    /// once in turn.
    pub fn read(&self, from: usize, out: &mut [i64]) -> usize {
        let end = (from + out.len()).min(self.len());
        let ranges = self.ranges.as_slice();
        let mut at = from;
        while at < end {
            let (k, j) = (at >> RANGE_BITS, at & (RANGE - 1));
            let stop = ((k + 1) * RANGE).min(end);
            let o = &mut out[at - from..stop - from];
            if self.parts == 1 {
                let d = ranges[k].dec();
                o.iter_mut().enumerate().for_each(|(i, v)| *v = d.get(j + i));
            } else {
                let (h, l) = (ranges[2 * k].dec(), ranges[2 * k + 1].dec());
                o.iter_mut().enumerate().for_each(|(i, v)| *v = h.get(j + i) << self.split | l.get(j + i));
            }
            at = stop;
        }
        end - from.min(end)
    }

    /// The bytes its ranges' words and descriptors take.
    pub fn bytes(&self) -> usize {
        size_of::<RangedCol>() + self.ranges.as_slice().iter().map(|r| r.bytes() + size_of::<IntCol>()).sum::<usize>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Node ids in canonical order: documents, a few granules each (slot << 18), a few nodes in each.
    fn nodes(n: usize) -> Vec<i64> {
        let mut v: Vec<i64> = (0..n as i64).map(|i| (100_000 + i / 900) << 32 | ((i * 7919) % 31) << 18 | (i % 40)).collect();
        v.sort_unstable();
        v
    }

    #[test]
    fn columns_read_back_whole_split_and_across_ranges() {
        let n = 3 * RANGE + 17;
        let a = nodes(n);
        let b: Vec<i64> = (0..n as i64).map(|i| (i * 2_654_435_761) % 1_000_003 - 500_000).collect();
        let c = vec![7i64; n];
        let block = pack(&[&a, &b, &c], &[18, 0, 0], crate::block::kind::FACTS);
        let cols: &Columns = block.root();
        assert_eq!((cols.len(), cols.arity), (n, 3));
        for i in 0..n {
            assert_eq!([cols.get(i, 0), cols.get(i, 1), cols.get(i, 2)], [a[i], b[i], c[i]], "row {i}");
        }
        let mut out = vec![0i64; RANGE + 100];
        let got = cols.col(0).read(RANGE - 50, &mut out);
        assert_eq!(got, out.len());
        assert_eq!(&out[..got], &a[RANGE - 50..RANGE - 50 + got]);
        let got = cols.col(1).read(n - 10, &mut out);
        assert_eq!(&out[..got], &b[n - 10..]);
    }

    /// Split at the granule, a sorted node column takes a few bits a row: its granule in runs, its index small.
    #[test]
    fn a_sorted_node_column_split_at_its_granule_is_small() {
        let n = 16 * RANGE;
        let a = nodes(n);
        let whole = pack(&[&a], &[0], crate::block::kind::FACTS);
        let split = pack(&[&a], &[18], crate::block::kind::FACTS);
        let (w, s) = (whole.root::<Columns>().col(0).bytes(), split.root::<Columns>().col(0).bytes());
        assert!(s * 2 < w, "split {s} B against whole {w} B");
        assert!(s < n * 2, "{s} B for {n} rows");
    }

    #[test]
    fn no_rows_pack_and_read() {
        let block = pack(&[&[], &[]], &[0, 18], crate::block::kind::FACTS);
        let cols: &Columns = block.root();
        assert!(cols.is_empty());
        assert_eq!(cols.col(1).read(0, &mut [0; 4]), 0);
    }
}
