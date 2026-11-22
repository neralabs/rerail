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
    /// The view's walk of the keyed rows, then the spans' (their own keyed walk, else every span row with a key: the
    /// spans are few beside the nodes).
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
        if self.spans.for_rows_keyed(column, keys, columns, &mut |r, row| each(first + r, row)) {
            return true;
        }
        let mut all = columns.to_vec();
        all.push(column);
        let k = columns.len();
        self.spans.for_rows_in(0..self.spans.len(), &all, &mut |r, row| !keys.contains(&row[k]) || each(first + r, &row[..k]));
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
    /// Counted in dense arrays in a block of the run's (`dense_statistics`): a place is a small number. Counted as a
    /// view's rows by `RelStats`, its counts took 48 bytes a row on the heap (`line`'s and `col`'s 49.5 MB each at the
    /// M3's 300-file peak, ab160).
    fn statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        dense_statistics(self)
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

/// The largest value `dense_statistics` counts: past it, a relation is counted as rows (`None`).
const DENSE: i64 = 1 << 24;

/// Each value's count over all rows (`all`) and within the document open (`doc`, its values `touched`), in a block.
struct Counts {
    all: crate::code_rule_dsl::facts::rows::BlockVec<i64>,
    doc: crate::code_rule_dsl::facts::rows::BlockVec<i64>,
    touched: crate::code_rule_dsl::facts::rows::BlockVec<u32>,
    sqd: i64,
    ok: bool,
}

impl Counts {
    fn add(&mut self, v: i64) {
        if !self.ok || !(0..DENSE).contains(&v) {
            self.ok = false;
            return;
        }
        let v = v as usize;
        if self.all.len() <= v {
            let grown = (v + 1).next_power_of_two();
            self.all.resize(grown, 0);
            self.doc.resize(grown, 0);
        }
        self.all[v] += 1;
        if self.doc[v] == 0 {
            self.touched.push(v as u32);
        }
        self.doc[v] += 1;
    }
    /// The open document's counts into `sqd` (each value's count within it, squared), and cleared.
    fn close(&mut self) {
        for &v in self.touched.iter() {
            let n = std::mem::take(&mut self.doc[v as usize]);
            self.sqd += n * n;
        }
        self.touched.resize(0, 0);
    }
}

/// A place relation's statistics, as `syntax::measured` gives them: each column's `(name, rows, distinct values, the
/// sum of their counts' squares, that sum within each document)`. Its first column is a node or span, one row each;
/// its second a small number, counted by value in arrays in a block, none on the heap. A document's rows are the
/// view's and then its spans' (`Places` puts every span after every node), so a document's span rows are counted
/// while the view's rows of it are open. `None` for a value negative or past `DENSE`, or rows out of order.
fn dense_statistics(p: &Places) -> Option<Vec<(String, i64, i64, i64, i64)>> {
    use crate::code_rule_dsl::facts::rows::BlockVec;
    let names = p.columns();
    if names.len() != 2 {
        return None;
    }
    // The spans' runs of one document: (document, first row, end), three values a run.
    let mut runs: BlockVec<i64> = BlockVec::new();
    let mut increasing = true;
    let mut last = None::<i64>;
    p.spans.for_rows(&[0], &mut |r, row| {
        increasing &= last.is_none_or(|l| l < row[0]);
        last = Some(row[0]);
        let d = row[0] >> 32;
        let n = runs.len();
        if n >= 3 && runs[n - 3] == d && runs[n - 1] == r as i64 {
            runs[n - 1] += 1;
        } else {
            runs.extend_from_slice(&[d, r as i64, r as i64 + 1]);
        }
    });
    let mut used: BlockVec<u8> = BlockVec::new();
    used.resize(runs.len() / 3, 0);
    let mut c = Counts { all: BlockVec::new(), doc: BlockVec::new(), touched: BlockVec::new(), sqd: 0, ok: true };
    let mut next = 0usize;
    // Document `d`'s span rows into the counts: the run after the last one taken, else any.
    let spans_of = |d: i64, c: &mut Counts, used: &mut BlockVec<u8>, next: &mut usize| {
        let k = runs.len() / 3;
        let Some(i) = (*next..k).chain(0..*next).find(|&i| used[i] == 0 && runs[3 * i] == d) else { return };
        used[i] = 1;
        *next = i + 1;
        let (lo, hi) = (runs[3 * i + 1] as usize, runs[3 * i + 2] as usize);
        p.spans.for_rows_in(lo..hi, &[1], &mut |_, row| {
            c.add(row[0]);
            true
        });
    };
    let mut at = None::<i64>;
    last = None;
    p.view.for_rows(&[0, 1], &mut |_, row| {
        increasing &= last.is_none_or(|l| l < row[0]);
        last = Some(row[0]);
        let d = row[0] >> 32;
        if at != Some(d) {
            if let Some(prev) = at {
                spans_of(prev, &mut c, &mut used, &mut next);
                c.close();
            }
            at = Some(d);
        }
        c.add(row[1]);
    });
    if let Some(prev) = at {
        spans_of(prev, &mut c, &mut used, &mut next);
        c.close();
    }
    // The documents with spans and no node: each its own.
    for i in 0..runs.len() / 3 {
        if used[i] == 0 {
            let d = runs[3 * i];
            spans_of(d, &mut c, &mut used, &mut next);
            c.close();
        }
    }
    if !c.ok || !increasing {
        return None;
    }
    let r = p.len() as i64;
    let (ndv, sq) = c.all.iter().filter(|&&n| n > 0).fold((0i64, 0i64), |(k, s), &n| (k + 1, s + n * n));
    Some(vec![(names[0].clone(), r, r, r, r), (names[1].clone(), r, ndv, sq, c.sqd)])
}
