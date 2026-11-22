//! A place relation of core.rerail's shape (`line`, `end_line`, `col`, `docof`) where a program reads spans (Q216,
//! decision 0353 step 3): the `Loc` view's rows, every node's, token's and position's place read off the tape, then
//! the places of the spans `Extra` made (a regex's matches, a comment no node is, `source` and `src_line`). A span is
//! a derived fact, and its place is written once where it is made (`LineTable::place`). No node's or token's place is
//! made as a row.
//!
//! A lookup by id goes to the side that holds the id. A node's id never reaches the spans, so a probe of a node's
//! place makes no document (the made rows' probes made a node's document and every relation of its group).

use super::granules;
use crate::code_rule_dsl::facts::vtab::Source;
use std::ops::Range;
use std::sync::Arc;

pub(super) struct Placed {
    /// The `Loc` view.
    pub view: Arc<dyn Source>,
    /// `Extra`'s rows of the place, its spans' alone.
    pub spans: Arc<dyn Source>,
}

/// Whether id `id` is one of `Extra`'s spans (`granules::ExtraSpan`), not a node, token or position.
fn is_span(id: i64) -> bool {
    granules::is_extra(id as u32)
}

impl Placed {
    /// The spans' first row: past the view's.
    fn at(&self) -> usize {
        self.view.len()
    }

    /// `rows` of the spans, as this relation's rows.
    fn after(&self, rows: Vec<usize>) -> impl Iterator<Item = usize> {
        let at = self.at();
        rows.into_iter().map(move |r| r + at)
    }
}

impl Source for Placed {
    fn python_batch(&self) -> Option<Arc<super::Batch>> {
        self.view.python_batch()
    }
    fn columns(&self) -> Vec<String> {
        self.view.columns()
    }
    fn len(&self) -> usize {
        self.view.len() + self.spans.len()
    }
    fn estimated_len(&self) -> usize {
        self.view.estimated_len() + self.spans.estimated_len()
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        // A row's id is one row, the view's or a span's.
        if column == 0 { self.view.estimated_matches(column, batch) } else { self.view.estimated_matches(column, batch) + self.spans.estimated_matches(column, batch) }
    }
    fn lookup_cost(&self, column: usize) -> f64 {
        self.view.lookup_cost(column).max(self.spans.lookup_cost(column))
    }
    fn indexed(&self, column: usize) -> bool {
        self.view.indexed(column) && self.spans.indexed(column)
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>> {
        let (spans, others): (Vec<i64>, Vec<i64>) = if column == 0 { values.iter().partition(|&&v| is_span(v)) } else { (values.to_vec(), values.to_vec()) };
        let mut rows = if others.is_empty() { Vec::new() } else { self.view.lookup(column, &others)? };
        if !spans.is_empty() {
            let found = self.spans.lookup(column, &spans)?;
            rows.extend(self.after(found));
        }
        Some(rows)
    }
    fn keys_rows(&self, column: usize) -> bool {
        column == 0 && self.view.keys_rows(0)
    }
    fn key_row(&self, column: usize, value: i64) -> Option<usize> {
        if is_span(value) {
            let found = self.spans.lookup(column, &[value])?;
            return self.after(found).next();
        }
        self.view.key_row(column, value)
    }
    fn lookup_read(&self, column: usize, value: i64, columns: &[usize], each: &mut dyn FnMut(usize, &[i64])) -> bool {
        // A node's place is the view's read; a span's is looked up (`false`: nothing given).
        column == 0 && !is_span(value) && self.view.lookup_read(column, value, columns, each)
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        let at = self.at();
        if row < at { self.view.value(row, column) } else { self.spans.value(row - at, column) }
    }
    fn row_at(&self, row: usize, columns: &[usize], out: &mut [i64]) {
        let at = self.at();
        if row < at { self.view.row_at(row, columns, out) } else { self.spans.row_at(row - at, columns, out) }
    }
    fn for_rows(&self, columns: &[usize], each: &mut dyn FnMut(usize, &[i64])) {
        let at = self.at();
        self.view.for_rows(columns, each);
        self.spans.for_rows(columns, &mut |r, row| each(r + at, row));
    }
    fn for_rows_in(&self, range: Range<usize>, columns: &[usize], each: &mut dyn FnMut(usize, &[i64]) -> bool) {
        let at = self.at();
        let mut going = true;
        if range.start < at {
            self.view.for_rows_in(range.start..range.end.min(at), columns, &mut |r, row| {
                going = each(r, row);
                going
            });
        }
        if going && range.end > at {
            self.spans.for_rows_in(range.start.max(at) - at..range.end - at, columns, &mut |r, row| each(r + at, row));
        }
    }
    fn documents_of(&self, column: usize, each: &mut dyn FnMut(usize, i64)) {
        let at = self.at();
        self.view.documents_of(column, each);
        self.spans.documents_of(column, &mut |r, d| each(r + at, d));
    }
    fn rows_of(&self, columns: &[usize], out: &mut Vec<i64>) {
        self.view.rows_of(columns, out);
        self.spans.rows_of(columns, out);
    }
    fn rows_of_documents(&self, docs: &std::collections::BTreeSet<u32>) -> Option<Vec<usize>> {
        let mut rows = self.view.rows_of_documents(docs)?;
        let found = self.spans.rows_of_documents(docs)?;
        rows.extend(self.after(found));
        Some(rows)
    }
    fn filter_eq(&self, eqs: &[(usize, i64)]) -> Vec<u32> {
        let at = self.at() as u32;
        let mut rows = self.view.filter_eq(eqs);
        rows.extend(self.spans.filter_eq(eqs).into_iter().map(|r| r + at));
        rows
    }
    fn made_on_lookup(&self) -> bool {
        self.spans.made_on_lookup()
    }
    fn complete_rows(&self) -> bool {
        self.spans.complete_rows()
    }
    fn expect_lookups(&self, column: usize, keys: f64) {
        self.spans.expect_lookups(column, keys);
    }
    fn failure(&self) -> Option<String> {
        self.view.failure().or_else(|| self.spans.failure())
    }
    fn failed(&self) -> Option<String> {
        self.view.failed().or_else(|| self.spans.failed())
    }
}
