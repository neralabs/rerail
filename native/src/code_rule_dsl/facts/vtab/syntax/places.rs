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
    fn python_batch(&self) -> Option<Arc<super::Batch>> {
        self.view.python_batch()
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
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
    /// The view's walk of the keyed rows, then the spans': only at the keys that are `Extra` spans' ids (no node's is:
    /// `granules::is_extra`), their own keyed walk, else the rows of those keys' documents, else every span row with a
    /// key. A walk of every span row made the whole batch, every relation of every group: the cross part's count of
    /// `docof` at the exports' nodes, 85 MB of the 300-file batch at the M3's peak (ab200), though no node it was asked
    /// for is a span.
    fn for_rows_keyed(&self, column: usize, keys: &rustc_hash::FxHashSet<i64>, columns: &[usize], each: &mut dyn FnMut(usize, &[i64]) -> bool) -> bool {
        let mut going = true;
        if !self.view.for_rows_keyed(column, keys, columns, &mut |r, row| {
            going = each(r, row);
            going
        }) {
            return false;
        }
        if !going {
            return true;
        }
        let first = self.first();
        let spans: rustc_hash::FxHashSet<i64> = keys.iter().copied().filter(|&k| super::granules::is_extra(k as u32)).collect();
        if spans.is_empty() {
            return true;
        }
        if self.spans.for_rows_keyed(column, &spans, columns, &mut |r, row| each(first + r, row)) {
            return true;
        }
        let mut all = columns.to_vec();
        all.push(column);
        let k = columns.len();
        let docs: BTreeSet<u32> = spans.iter().map(|&s| (s >> 32) as u32).collect();
        if let Some(rows) = self.spans.rows_of_documents(&docs) {
            let mut row = vec![0; all.len()];
            for r in rows {
                self.spans.row_at(r, &all, &mut row);
                if spans.contains(&row[k]) && !each(first + r, &row[..k]) {
                    break;
                }
            }
            return true;
        }
        self.spans.for_rows_in(0..self.spans.len(), &all, &mut |r, row| !spans.contains(&row[k]) || each(first + r, &row[..k]));
        true
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
    /// Counted in an array in a block of the run's (`place_statistics`). Counted as a view's rows by `RelStats`, its counts took 48 bytes a row on the heap (`line`'s and `col`'s 49.5 MB each at the
    /// M3's 300-file peak, ab160).
    fn statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        place_statistics(self)
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
pub fn join<N: AsRef<str>>(out: &mut [(N, Arc<dyn Source>)], batch: &Arc<super::Batch>) {
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

/// A place relation's second column's values with each row's document (`value`, `document`), in an array in the
/// run's blocks, none on the heap: sorted, a run of one value is its postings, and within it a run of one document
/// that document's, whatever the rows' order or the values' range.
struct Pairs(crate::code_rule_dsl::facts::rows::BlockVec<[i64; 2]>);

impl Pairs {
    fn new() -> Pairs {
        Pairs(crate::code_rule_dsl::facts::rows::BlockVec::new())
    }
    /// A row of `id` (the first column: its document is the high half) and `value`.
    fn add(&mut self, id: i64, value: i64) {
        self.0.push([value, id >> 32]);
    }
    /// `(rows, distinct values, the sum of their postings' squares, that sum within each document)`.
    fn measure(mut self) -> (i64, i64, i64, i64) {
        self.0.sort_unstable();
        let (mut ndv, mut sq, mut sqd) = (0i64, 0i64, 0i64);
        for value in self.0.chunk_by(|a, b| a[0] == b[0]) {
            ndv += 1;
            sq += (value.len() * value.len()) as i64;
            for doc in value.chunk_by(|a, b| a[1] == b[1]) {
                sqd += (doc.len() * doc.len()) as i64;
            }
        }
        (self.0.len() as i64, ndv, sq, sqd)
    }
}

/// A place relation's statistics, as `syntax::measured` gives them: each column's `(name, rows, distinct values, the
/// sum of their counts' squares, that sum within each document)`. Its first column is a node or span, one row each;
/// its second counted by value (`Pairs`). Its spans' rows not made yet, the view's are counted on the tape and the
/// spans' measured on a sample of their documents (`MadeRows::sampled`), so the measure makes no document's rows (a
/// walk of the spans made the whole batch: `col`'s measure did, ab175). Rows add; the second column's values (lines,
/// columns, documents) are the view's mostly, the spans' among them, so its distinct values are the more of the two's,
/// and its postings' squares add with their cross term at its most (each value's postings in both, in proportion).
fn place_statistics(p: &Places) -> Option<Vec<(String, i64, i64, i64, i64)>> {
    let names = p.columns();
    if names.len() != 2 {
        return None;
    }
    let mut view = Pairs::new();
    let unmade = p.spans.as_any().and_then(|a| a.downcast_ref::<super::made::MadeRows>()).is_some_and(|s| !s.is_done());
    let sampled = if unmade { p.spans.statistics() } else { None };
    p.view.for_rows(&[0, 1], &mut |_, row| view.add(row[0], row[1]));
    if sampled.is_none() {
        p.spans.for_rows(&[0, 1], &mut |_, row| view.add(row[0], row[1]));
    }
    let (rows, ndv, sq, sqd) = view.measure();
    let (r, ndv, sq, sqd) = match sampled.as_ref().and_then(|s| s.get(1)) {
        Some(&(_, s_rows, s_ndv, s_sq, s_sqd)) => {
            let cross = |a: i64, b: i64| 2 * ((a as f64) * (b as f64)).sqrt() as i64;
            (rows + s_rows, ndv.max(s_ndv), sq + s_sq + cross(sq, s_sq), sqd + s_sqd + cross(sqd, s_sqd))
        }
        None => (rows, ndv, sq, sqd),
    };
    Some(vec![(names[0].clone(), r, r, r, r), (names[1].clone(), r, ndv, sq, sqd)])
}
