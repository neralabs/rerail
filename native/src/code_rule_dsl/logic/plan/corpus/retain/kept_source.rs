//! A kept relation read in place (decision 0417 step 6): a `Source` over its entry's `Packed`, where it lies in its
//! space's chain of mapped files. The run's relation is a view of it (`InRel::over_source`): no row copied, no frame
//! decoded. Its leading column is in order, so a probe of it is a binary search of the column (`sorted_range`);
//! another column's probes go through its rows sorted by that column, made when first asked (as a relation's own
//! order is). A relation the run extends is made from its rows then (`Rel::based` takes no view).
use crate::code_rule_dsl::facts::vtab::Source;
use crate::content_store::KeptRows;
use std::ops::Range;
use std::sync::OnceLock;

pub(super) struct KeptSource {
    kept: KeptRows,
    names: Vec<String>,
    /// Each column's rows in its order, made when first asked (the leading column needs none): its values sorted,
    /// and the rows in that order.
    by: Vec<OnceLock<(Vec<i64>, Vec<u32>)>>,
    /// Each column's distinct values, counted when first asked.
    distinct: Vec<OnceLock<usize>>,
}

impl KeptSource {
    pub(super) fn new(kept: KeptRows) -> KeptSource {
        let k = kept.rows().arity as usize;
        KeptSource { kept, names: (0..k).map(|c| format!("c{c}")).collect(), by: (0..k).map(|_| OnceLock::new()).collect(), distinct: (0..k).map(|_| OnceLock::new()).collect() }
    }

    fn leads(&self, column: usize) -> bool {
        self.kept.rows().leads(column)
    }

    /// The rows whose leading `column` is `value`.
    fn range(&self, column: usize, value: i64) -> Range<usize> {
        let rows = self.kept.rows();
        let (lo, hi) = rows.col(column).dec().equal_range(0, rows.len(), value);
        lo..hi
    }

    fn by(&self, column: usize) -> &(Vec<i64>, Vec<u32>) {
        self.by[column].get_or_init(|| {
            let mut pairs: Vec<(i64, u32)> = Vec::with_capacity(self.len());
            let mut r = 0u32;
            self.kept.rows().col(column).for_each(|v| {
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
        self.kept.rows().len()
    }

    fn value(&self, row: usize, column: usize) -> i64 {
        self.kept.rows().get(row, column)
    }

    fn indexed(&self, _column: usize) -> bool {
        true
    }

    fn estimated_matches(&self, column: usize, _batch: bool) -> usize {
        let distinct = *self.distinct[column].get_or_init(|| {
            if self.leads(column) {
                let (mut n, mut last) = (0usize, None);
                self.kept.rows().col(column).for_each(|v| {
                    if last != Some(v) {
                        n += 1;
                        last = Some(v);
                    }
                });
                n
            } else {
                let values = &self.by(column).0;
                values.chunk_by(|a, b| a == b).count()
            }
        });
        self.len().div_ceil(distinct.max(1)).max(1)
    }

    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>> {
        let mut out = Vec::new();
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
        let (lo, hi) = self.kept.rows().col(column).dec().equal_range_from(*at as usize, value);
        *at = lo as u32;
        Some(lo as u32..hi as u32)
    }

    fn sorts_rows(&self, _column: usize) -> bool {
        true
    }

    /// Its entry's canonical order: the order key's columns, then the rest (an entry put as given records none).
    fn order(&self) -> Option<Vec<usize>> {
        let rows = self.kept.rows();
        let key = &rows.okey[..rows.nokey as usize];
        (!key.is_empty()).then(|| key.iter().map(|&c| c as usize).chain((0..rows.arity as usize).filter(|c| !key.contains(&(*c as u16)))).collect())
    }
}
