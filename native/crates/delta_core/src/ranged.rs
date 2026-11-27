//! The format of columns packed per range of rows, and its reads (moved from `rerail_blocks::ranged`, whose `pack`
//! and `writer` write them): one block holding a relation's columns, each cut into ranges of
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

use crate::col::IntCol;
use crate::refs::Span;
use crate::structure;

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
