//! core.rerail's places of nodes and spans (`line`, `end_line`, `col`, `docof`), read off the tape (decision 0353).
//!
//! A node's or a token's place is the `Loc` view's: computed from its byte span and its document's lines where it is
//! read. Only the spans `Extra` numbers (`source`, `src_line`, the comments, the regexes' matches: decision 0353 step
//! 3) have rows, which `Extra` writes once where it makes the spans. A program that read a span relation made every
//! node's places as rows too before: on the M3's 300-file run, 71 MB of the batch's 150 MB block of relations, and
//! the heap that staged them. [`Places`] binds the view and the span rows as one relation: the view's rows first,
//! then the spans'.

use super::super::Source;
use std::collections::BTreeSet;
use std::sync::Arc;

/// The places core.rerail's shape reads as the `Loc` view, with rows only for `Extra`'s spans.
pub(super) const NAMES: [&str; 4] = ["line", "end_line", "col", "docof"];

/// A place relation: the `Loc` view's rows (`view`), then the rows of `Extra`'s spans (`spans`), numbered past them.
pub struct Places {
    view: Arc<dyn Source>,
    spans: Arc<dyn Source>,
}

impl Places {
    pub fn new(view: Arc<dyn Source>, spans: Arc<dyn Source>) -> Self {
        debug_assert_eq!(view.columns().len(), spans.columns().len());
        Places { view, spans }
    }

    /// The view's rows: the first span row's number.
    fn first(&self) -> usize {
        self.view.len()
    }

    /// Both parts' rows, the spans' past the view's: `None` when either cannot answer.
    fn both(&self, of: impl Fn(&dyn Source) -> Option<Vec<usize>>) -> Option<Vec<usize>> {
        let mut rows = of(&*self.view)?;
        let spans = of(&*self.spans)?;
        if !spans.is_empty() {
            let first = self.first();
            rows.extend(spans.into_iter().map(|r| first + r));
        }
        Some(rows)
    }
}

impl Source for Places {
    fn syntax_batch(&self) -> Option<Arc<dyn std::any::Any + Send + Sync>> {
        self.view.syntax_batch()
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
    fn value(&self, row: usize, column: usize) -> i64 {
        let first = self.first();
        if row < first { self.view.value(row, column) } else { self.spans.value(row - first, column) }
    }
    fn row_at(&self, row: usize, columns: &[usize], out: &mut [i64]) {
        let first = self.first();
        if row < first { self.view.row_at(row, columns, out) } else { self.spans.row_at(row - first, columns, out) }
    }
    fn for_rows(&self, columns: &[usize], each: &mut dyn FnMut(usize, &[i64])) {
        self.view.for_rows(columns, each);
        let first = self.first();
        self.spans.for_rows(columns, &mut |r, row| each(first + r, row));
    }
    fn for_rows_in(&self, range: std::ops::Range<usize>, columns: &[usize], each: &mut dyn FnMut(usize, &[i64]) -> bool) {
        let first = self.first();
        let mut going = true;
        if range.start < first {
            self.view.for_rows_in(range.start..range.end.min(first), columns, &mut |r, row| {
                going = each(r, row);
                going
            });
        }
        if going && range.end > first {
            self.spans.for_rows_in(range.start.saturating_sub(first)..range.end - first, columns, &mut |r, row| each(first + r, row));
        }
    }
    fn rows_of(&self, columns: &[usize], out: &mut Vec<i64>) {
        self.view.rows_of(columns, out);
        self.spans.rows_of(columns, out);
    }
    fn documents_of(&self, column: usize, each: &mut dyn FnMut(usize, i64)) {
        self.view.documents_of(column, each);
        let first = self.first();
        self.spans.documents_of(column, &mut |r, d| each(first + r, d));
    }
    fn rows_of_documents(&self, docs: &BTreeSet<u32>) -> Option<Vec<usize>> {
        self.both(|s| s.rows_of_documents(docs))
    }
    fn doc_column(&self) -> Option<usize> {
        self.view.doc_column()
    }
    fn kept_to_doc(&self, column: usize) -> bool {
        self.view.kept_to_doc(column) && self.spans.kept_to_doc(column)
    }
    fn lookup_in(&self, column: usize, values: &[i64], doc: i64) -> Option<Vec<usize>> {
        self.both(|s| s.lookup_in(column, values, doc).or_else(|| s.lookup(column, values)))
    }
    fn indexed(&self, column: usize) -> bool {
        self.view.indexed(column) && self.spans.indexed(column)
    }
    fn lookup_cost(&self, column: usize) -> f64 {
        self.view.lookup_cost(column).max(self.spans.lookup_cost(column))
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        self.view.estimated_matches(column, batch) + self.spans.estimated_matches(column, batch)
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>> {
        self.both(|s| s.lookup(column, values))
    }
    fn lookup_read(&self, column: usize, value: i64, columns: &[usize], each: &mut dyn FnMut(usize, &[i64])) -> bool {
        let Some(rows) = self.lookup(column, &[value]) else { return false };
        let mut row = vec![0; columns.len()];
        for r in rows {
            self.row_at(r, columns, &mut row);
            each(r, &row);
        }
        true
    }
    fn failure(&self) -> Option<String> {
        self.view.failure().or_else(|| self.spans.failure())
    }
    fn failed(&self) -> Option<String> {
        self.view.failed().or_else(|| self.spans.failed())
    }
}

/// `out`'s place relations made as `Extra`'s span rows (core.rerail's shape), each bound with the `Loc` view of its
/// nodes before them: the batch makes no node's place as a row.
pub(super) fn join<N: AsRef<str>>(out: &mut [(N, Arc<dyn Source>)], batch: &Arc<super::Batch>) {
    if batch.shape != super::Shape::Core {
        return;
    }
    for (name, place) in super::named_views(batch.shape) {
        if !NAMES.contains(&name) || !batch.made.contains(name) {
            continue;
        }
        if let Some((_, source)) = out.iter_mut().find(|(n, _)| n.as_ref() == name) {
            let view: Arc<dyn Source> = Arc::new(super::Viewed { batch: Arc::clone(batch), view: place, home: std::sync::OnceLock::new() });
            *source = Arc::new(Places::new(view, Arc::clone(source)));
        }
    }
}
