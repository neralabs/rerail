//! An edit's input deltas, read from the documents that changed (decision 0090, 0458): an input's version before the
//! edit is its rows now but the changed documents', then the rows those documents had (`Before`), each read where it
//! lies; the rows the changed documents own are found through the input's own index (`rows_of_documents`).

use rerail_blocks::vector::Vector;
use super::memo::Content;
use std::collections::HashMap;
use std::sync::Arc;

/// Each document a run read: its content as the memo knows it, and its document key, by path.
pub(in crate::code_rule_dsl) type Docs = HashMap<String, (Content, i64)>;

/// Rows of `now` that `docs` own, ascending: through `now`'s own index of its first column, a document key (`d`) or a
/// node id (`key << 32 | …`), where it has one; else (`None`) the caller reads `now` through. Decision 0458: an
/// edit reads its changed documents' rows, not every row.
pub(super) fn rows_of_documents(now: &dyn crate::code_rule_dsl::facts::vtab::Source, docs: &std::collections::BTreeSet<i64>) -> Option<Vector<u32>> {
    let first = now.columns().into_iter().next()?;
    let mut rows: Vector<u32> = rerail_blocks::vector::Vector::new();
    if first == "d" {
        if !now.indexed(0) {
            return None;
        }
        rows.extend(now.lookup(0, &docs.iter().copied().collect::<Vec<_>>())?.into_iter().map(|r| r as u32));
    } else {
        if !now.range_indexed(0) {
            return None;
        }
        for d in docs {
            rows.extend(now.range_lookup(0, Some((d << 32, true)), Some(((d + 1) << 32, false)))?.into_iter().map(|r| r as u32));
        }
    }
    rows.sort_unstable();
    rows.dedup();
    Some(rows)
}

/// A changed base relation's version before an edit (decision 0458, 0364's `Old`), read where its rows lie: its rows
/// now but the changed documents' (`skip`, ascending), then the rows those documents had before the edit (`was`, a
/// batch of their old documents). No row is copied.
pub(super) struct Before {
    now: Arc<dyn crate::code_rule_dsl::facts::vtab::Source>,
    skip: Vector<u32>,
    was: Option<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>>,
    kept: std::sync::OnceLock<usize>,
}

impl Before {
    pub(super) fn new(now: Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, skip: Vector<u32>, was: Option<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>>) -> Before {
        Before { now, skip, was, kept: std::sync::OnceLock::new() }
    }

    /// Its rows of `now`, the skipped ones but: read when it is first read, not when it is made, as the corpus batch
    /// counts its rows by the totals it is given after the versions are made (decision 0494), and before them made
    /// every document to count them (Q332).
    fn kept(&self) -> usize {
        *self.kept.get_or_init(|| self.now.len() - self.skip.len())
    }

    /// The id in `now` of its `r`-th row not skipped: `r` plus the skipped ids before it (`skip[j] - j` counts the rows
    /// kept before `skip[j]`, and does not decrease).
    fn now_row(&self, r: usize) -> usize {
        let (mut lo, mut hi) = (0usize, self.skip.len());
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.skip[mid] as usize - mid <= r { lo = mid + 1 } else { hi = mid }
        }
        r + lo
    }

    /// The row of `now`'s id `id`: its rank among the rows not skipped, `None` for a skipped one.
    fn before_row(&self, id: usize) -> Option<usize> {
        let at = self.skip.partition_point(|&s| (s as usize) < id);
        (self.skip.get(at).copied() != Some(id as u32)).then_some(id - at)
    }
}

impl crate::code_rule_dsl::facts::vtab::Source for Before {
    fn columns(&self) -> Vec<String> {
        self.now.columns()
    }
    fn len(&self) -> usize {
        self.kept() + self.was.as_ref().map_or(0, |w| w.len())
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        self.now.estimated_matches(column, batch)
    }
    fn lookup_cost(&self, column: usize) -> f64 {
        self.now.lookup_cost(column)
    }
    fn scan_cost(&self) -> f64 {
        self.now.scan_cost()
    }
    fn indexed(&self, column: usize) -> bool {
        self.now.indexed(column)
    }
    /// The corpus batch's lookup, the changed documents' rows dropped, then the old documents' rows: by their own
    /// lookup, else read (they are the changed documents' alone).
    fn lookup(&self, column: usize, values: &[i64]) -> Option<rerail_blocks::vector::Vector<usize>> {
        let mut out: rerail_blocks::vector::Vector<usize> = self.now.lookup(column, values)?.into_iter().filter_map(|id| self.before_row(id)).collect();
        if let Some(was) = &self.was {
            match was.lookup(column, values) {
                Some(ids) => out.extend(ids.into_iter().map(|r| r + self.kept())),
                None => out.extend((0..was.len()).filter(|&r| values.contains(&was.value(r, column))).map(|r| r + self.kept())),
            }
        }
        out.sort_unstable();
        Some(out)
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        if row < self.kept() {
            self.now.value(self.now_row(row), column)
        } else {
            self.was.as_ref().expect("a row past the kept ones is an old document's").value(row - self.kept(), column)
        }
    }
}

#[cfg(test)]
mod before_tests {
    use super::Before;
    use crate::code_rule_dsl::facts::vtab::Source;
    use std::sync::Arc;

    /// The old version reads the rows now that the changed documents do not own, in order, then their old rows.
    #[test]
    fn before_reads_the_kept_rows_then_the_old_documents_rows() {
        let col = |v: rerail_blocks::vector::Vector<i64>| crate::code_rule_dsl::logic::native::component::columns_source(vec!["n".to_owned()], vec![Some(v)]);
        let now = col(rerail_blocks::vector![10, 11, 12, 13, 14, 15]);
        let was = col(rerail_blocks::vector![99, 98]);
        let before = Before::new(now, rerail_blocks::vector![0, 2, 3], Some(was));
        let rows: rerail_blocks::vector::Vector<i64> = (0..before.len()).map(|r| before.value(r, 0)).collect();
        assert_eq!(rows, vec![11, 14, 15, 99, 98]);
        // Looked up, the same rows by their place in the old version.
        for v in [10, 11, 14, 99, 7] {
            let want: rerail_blocks::vector::Vector<usize> = (0..before.len()).filter(|&r| before.value(r, 0) == v).collect();
            assert_eq!(before.lookup(0, &[v]).expect("the rows now are looked up"), want, "{v}");
        }
        let none = Before::new(col(rerail_blocks::vector![1, 2]), rerail_blocks::vector![], None);
        assert_eq!((0..none.len()).map(|r| none.value(r, 0)).collect::<Vec<_>>(), vec![1, 2]);
        let _ = Arc::new(none);
    }
}
