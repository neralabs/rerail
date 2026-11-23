//! A kept relation read in place (decision 0456 step 6b): a `Source` over its entries' `Packed`s, where they lie in
//! their space's chain of mapped files (one entry, or a kept input's granules one after another). The run's relation
//! is a view of it (`InRel::over_source`): no row copied, no value decoded but as read. A single entry's leading column
//! is in order, so a probe of it is a binary search of the column (`sorted_range`); another column's probes go through
//! its rows sorted by that column, made when first asked (as a relation's own order is). A relation the run extends is
//! made from its rows then (`Rel::based` takes no view).
use rerail_blocks::vector::Vector;
use super::kept_read::Bound;
use crate::code_rule_dsl::facts::vtab::Source;
use rerail_blocks::packed::Packed;
use std::ops::Range;
use std::sync::{Arc, OnceLock};

pub(super) struct KeptSource {
    /// Holds the mapping the parts lie in.
    _bound: Arc<Bound>,
    /// Its entries, in order, each in `_bound`'s mapping.
    parts: Vec<*const Packed>,
    /// Each part's first row, and the rows' count last.
    starts: Vector<usize>,
    names: Vec<String>,
    /// Each column's rows in its order, made when first asked (a single entry's leading column needs none): its
    /// values sorted, and the rows in that order.
    by: Vec<OnceLock<(Vector<i64>, Vector<u32>)>>,
    /// Each column's distinct values, counted when first asked.
    distinct: Vec<OnceLock<usize>>,
}

// SAFETY: the parts lie in a read-only mapping `_bound` holds, shared by every thread that reads it.
unsafe impl Send for KeptSource {}
unsafe impl Sync for KeptSource {}

impl KeptSource {
    /// `parts` (at least one, of one width), in `bound`'s mapping.
    pub(super) fn new(bound: Arc<Bound>, parts: Vec<*const Packed>) -> KeptSource {
        // SAFETY: as above.
        let k = parts.first().map_or(0, |&p| unsafe { &*p }.arity as usize);
        let mut starts = rerail_blocks::vector::Vector::with_capacity(parts.len() + 1);
        let mut n = 0;
        for &p in &parts {
            starts.push(n);
            // SAFETY: as above.
            n += unsafe { &*p }.len();
        }
        starts.push(n);
        KeptSource { _bound: bound, parts, starts, names: (0..k).map(|c| format!("c{c}")).collect(), by: (0..k).map(|_| OnceLock::new()).collect(), distinct: (0..k).map(|_| OnceLock::new()).collect() }
    }

    fn part(&self, i: usize) -> &Packed {
        // SAFETY: as above: the mapping lives as long as `_bound`.
        unsafe { &*self.parts[i] }
    }

    /// The single entry, when there is one.
    fn single(&self) -> Option<&Packed> {
        (self.parts.len() == 1).then(|| self.part(0))
    }

    fn leads(&self, column: usize) -> bool {
        self.single().is_some_and(|p| p.leads(column))
    }

    /// Row `row`'s part and its place there.
    fn at(&self, row: usize) -> (usize, usize) {
        let i = self.starts.partition_point(|&s| s <= row) - 1;
        (i, row - self.starts[i])
    }

    /// Each value of `column`, in row order.
    fn for_each(&self, column: usize, mut f: impl FnMut(i64)) {
        for i in 0..self.parts.len() {
            self.part(i).col(column).for_each(&mut f);
        }
    }

    /// The rows whose leading `column` is `value` (a single entry's).
    fn range(&self, column: usize, value: i64) -> Range<usize> {
        let rows = self.part(0);
        let (lo, hi) = rows.col(column).dec().equal_range(0, rows.len(), value);
        lo..hi
    }

    fn by(&self, column: usize) -> &(Vector<i64>, Vector<u32>) {
        self.by[column].get_or_init(|| {
            let mut pairs: Vec<(i64, u32)> = Vec::with_capacity(self.len());
            let mut r = 0u32;
            self.for_each(column, |v| {
                pairs.push((v, r));
                r += 1;
            });
            pairs.sort_unstable();
            pairs.into_iter().unzip()
        })
    }

    fn sorted(&self, column: usize, value: i64) -> &[u32] {
        let (values, rows) = self.by(column);
        let lo = values.partition_point(|&v| v < value);
        let hi = values.partition_point(|&v| v <= value);
        &rows[lo..hi]
    }
}

impl Source for KeptSource {
    fn columns(&self) -> Vec<String> {
        self.names.clone()
    }

    fn len(&self) -> usize {
        *self.starts.last().expect("a count")
    }

    fn value(&self, row: usize, column: usize) -> i64 {
        let (i, at) = self.at(row);
        self.part(i).get(at, column)
    }

    fn indexed(&self, _column: usize) -> bool {
        true
    }

    fn estimated_matches(&self, column: usize, _batch: bool) -> usize {
        let distinct = *self.distinct[column].get_or_init(|| {
            if self.leads(column) {
                let (mut n, mut last) = (0usize, None);
                self.for_each(column, |v| {
                    if last != Some(v) {
                        n += 1;
                        last = Some(v);
                    }
                });
                n
            } else if let Some((values, _)) = self.by[column].get() {
                values.chunk_by(|a, b| a == b).count()
            } else {
                // Counted on the column's values sorted for the count alone: its order (`by`, 12 bytes a row) is
                // made for a probe of it, not held for an estimate.
                let mut values = Vec::with_capacity(self.len());
                self.for_each(column, |v| values.push(v));
                values.sort_unstable();
                values.chunk_by(|a, b| a == b).count()
            }
        });
        self.len().div_ceil(distinct.max(1)).max(1)
    }

    fn lookup(&self, column: usize, values: &[i64]) -> Option<rerail_blocks::vector::Vector<usize>> {
        let mut out = rerail_blocks::vector::Vector::new();
        for &v in values {
            if self.leads(column) {
                out.extend(self.range(column, v));
            } else {
                out.extend(self.sorted(column, v).iter().map(|&r| r as usize));
            }
        }
        Some(out)
    }

    fn sorted_rows(&self, column: usize, value: i64) -> Option<&[u32]> {
        (!self.leads(column)).then(|| self.sorted(column, value))
    }

    fn sorted_range(&self, column: usize, value: i64) -> Option<Range<u32>> {
        self.leads(column).then(|| {
            let r = self.range(column, value);
            r.start as u32..r.end as u32
        })
    }

    /// From `at` (`ColDec::equal_range_from`): a probe in the column's order after the last one reads forward.
    fn sorted_range_at(&self, column: usize, value: i64, at: &mut u32) -> Option<Range<u32>> {
        if !self.leads(column) {
            return None;
        }
        let (lo, hi) = self.part(0).col(column).dec().equal_range_from(*at as usize, value);
        *at = lo as u32;
        Some(lo as u32..hi as u32)
    }

    fn sorts_rows(&self, _column: usize) -> bool {
        true
    }

    fn kept(&self) -> bool {
        true
    }

    /// A single entry's canonical order: the order key's columns, then the rest (an entry put as given, or granules
    /// one after another, record none).
    fn order(&self) -> Option<rerail_blocks::vector::Vector<usize>> {
        let rows = self.single()?;
        let key = &rows.okey[..rows.nokey as usize];
        (!key.is_empty()).then(|| key.iter().map(|&c| c as usize).chain((0..rows.arity as usize).filter(|c| !key.contains(&(*c as u16)))).collect())
    }
}
