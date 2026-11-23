//! core.rerail's places of nodes and spans (`line`, `end_line`, `col`, `docof`), read off the tape (decision 0353
//! step 3, Q216).
//!
//! A node's or a token's place is the `Loc` view's: computed from its byte span and its document's lines where it is
//! read. Only the spans `Extra` numbers (`source`, `src_line`, the comments, the regexes' matches) have rows, which
//! `Extra` writes once where it makes the spans (`LineTable::place`). A program that read a span relation made every
//! node's places as rows too before: on the M3's 300-file run, 71 MB of the batch's 150 MB block of relations, and
//! the heap that staged them. [`Places`] binds the view and the span rows as one relation: the view's rows first,
//! then the spans'.
//!
//! A lookup by id goes to the side that holds the id (`granules::is_extra`). A node's id never reaches the spans, so a
//! probe of a node's place makes no document (the made rows' probes made a node's document and every relation of its
//! group).
//!
//! The device reads it in place too (`device_view`): the view through the views' reads, the spans' rows in their
//! block, one relation to a stage (`PlacedDir`, the kernels' `vread` of a kind with `VIEW_PLACED`).

use super::super::Source;
use super::granules;
use std::collections::BTreeSet;
use std::sync::Arc;

/// The places core.rerail's shape reads as the `Loc` view, with rows only for `Extra`'s spans.
pub(super) const NAMES: [&str; 4] = ["line", "end_line", "col", "docof"];

/// A place relation: the `Loc` view's rows (`view`), then the rows of `Extra`'s spans (`spans`), numbered past them.
pub struct Places {
    view: Arc<dyn Source>,
    spans: Arc<dyn Source>,
    /// Where the device reads it, made on the first ask (`device_view`); `None` where it cannot.
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    device: std::sync::OnceLock<Option<OnDevice>>,
}

/// Whether id `id` is one of `Extra`'s spans (`granules::ExtraSpan`), not a node, token or position.
fn is_span(id: i64) -> bool {
    granules::is_extra(id as u32)
}

/// The kernels' `PlacedDir`: the view's directory, the spans' relation where it lies (its block's address and its
/// descriptor's offset), and the view's rows, which the spans' follow.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
#[repr(C)]
#[derive(Clone, Copy)]
struct PlacedDir {
    dir: u64,
    tail: u64,
    at: u64,
    rows: u32,
    pad: u32,
}

/// The kernels' `VIEW_PLACED`: a kind's bit for a placed relation, whose directory is a `PlacedDir`.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
pub const VIEW_PLACED: u32 = 16;

/// A placed relation on the device: its `PlacedDir` (a buffer of its own: addresses, no rows), the buffer its reads
/// reach past the view's (the spans' block), the spans' relation held while it is read, and its relation of no
/// columns, which its indexes go in (decision 0478 step 3).
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
struct OnDevice {
    dir: rerail_device::device::Buffer,
    tail: rerail_device::device::Buffer,
    _spans: crate::code_rule_dsl::facts::rows::InBlock,
    home: crate::code_rule_dsl::facts::rows::Part,
}

impl Places {
    pub fn new(view: Arc<dyn Source>, spans: Arc<dyn Source>) -> Self {
        debug_assert_eq!(view.columns().len(), spans.columns().len());
        Places {
            view,
            spans,
            #[cfg(any(target_os = "macos", target_os = "linux", windows))]
            device: std::sync::OnceLock::new(),
        }
    }

    /// The view's rows: the first span row's number.
    fn first(&self) -> usize {
        self.view.len()
    }

    /// The rows of the ids `values` on `column`, each id from the side that holds it on column 0 (a node's from the
    /// view, a span's from the spans), from both on another column: `None` when a part asked cannot answer.
    fn by_side(&self, column: usize, values: &[i64], of: impl Fn(&dyn Source, &[i64]) -> Option<Vec<usize>>) -> Option<Vec<usize>> {
        let (spans, nodes): (Vec<i64>, Vec<i64>) = if column == 0 { values.iter().partition(|&&v| is_span(v)) } else { (values.to_vec(), values.to_vec()) };
        let mut rows = if nodes.is_empty() { Vec::new() } else { of(&*self.view, &nodes)? };
        if !spans.is_empty() {
            let first = self.first();
            rows.extend(of(&*self.spans, &spans)?.into_iter().map(|r| first + r));
        }
        Some(rows)
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

    /// The view's directory and the spans' rows where they lie, in a `PlacedDir` (`view_dir`: the view's, read in
    /// place). `None` where the spans' rows are in no block, or the device's buffers have no addresses.
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    fn on_device(&self, view_dir: u64) -> Option<OnDevice> {
        let device = crate::code_rule_dsl::tape::placed::device()?;
        let spans = self.spans.in_block()?;
        let tail = spans.facts.block.device_buffer(device).ok()?;
        let base = tail.device_address()?;
        let dir = rerail_device::device::alloc_uninitialized_array_sync::<u8>(device, std::mem::size_of::<PlacedDir>());
        let placed = PlacedDir { dir: view_dir, tail: base, at: spans.at as u64, rows: self.first() as u32, pad: 0 };
        // SAFETY: the buffer's bytes, shared with the host, `PlacedDir`'s size, read by no stage yet.
        unsafe { dir.contents().cast::<PlacedDir>().write_unaligned(placed) };
        let home = crate::code_rule_dsl::facts::rows::Part::for_device(0, self.len()).ok()?;
        Some(OnDevice { dir, tail, _spans: spans, home })
    }
}

impl Source for Places {
    fn syntax_batch(&self) -> Option<Arc<dyn std::any::Any + Send + Sync>> {
        self.view.syntax_batch()
    }
    /// The view read in place, its rows then the spans' where they lie (`OnDevice`). With no spans, the view's. Where
    /// the view is not read in place, none: the stage reads its rows from the host (0353 step 6's upload).
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    fn device_view(&self) -> Option<crate::code_rule_dsl::facts::vtab::DeviceView> {
        let mut d = self.view.device_view()?;
        if self.spans.len() == 0 {
            return Some(d);
        }
        let v = &mut d.in_place;
        let on = self.device.get_or_init(|| self.on_device(v.dir)).as_ref()?;
        v.dir = on.dir.device_address()?;
        v.kind |= VIEW_PLACED;
        v.buffers.push(on.dir.clone());
        v.buffers.push(on.tail.clone());
        d.rows = self.len();
        d.home = on.home.clone();
        Some(d)
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
    fn filter_eq(&self, eqs: &[(usize, i64)]) -> Vec<u32> {
        let first = self.first() as u32;
        let mut rows = self.view.filter_eq(eqs);
        rows.extend(self.spans.filter_eq(eqs).into_iter().map(|r| r + first));
        rows
    }
    fn doc_column(&self) -> Option<usize> {
        self.view.doc_column()
    }
    fn kept_to_doc(&self, column: usize) -> bool {
        self.view.kept_to_doc(column) && self.spans.kept_to_doc(column)
    }
    fn lookup_in(&self, column: usize, values: &[i64], doc: i64) -> Option<Vec<usize>> {
        self.by_side(column, values, |s, v| s.lookup_in(column, v, doc).or_else(|| s.lookup(column, v)))
    }
    fn indexed(&self, column: usize) -> bool {
        self.view.indexed(column) && self.spans.indexed(column)
    }
    fn lookup_cost(&self, column: usize) -> f64 {
        self.view.lookup_cost(column).max(self.spans.lookup_cost(column))
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        // A row's id is one row, the view's or a span's.
        if column == 0 { self.view.estimated_matches(column, batch) } else { self.view.estimated_matches(column, batch) + self.spans.estimated_matches(column, batch) }
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>> {
        self.by_side(column, values, |s, v| s.lookup(column, v))
    }
    fn keys_rows(&self, column: usize) -> bool {
        column == 0 && self.view.keys_rows(0)
    }
    fn key_row(&self, column: usize, value: i64) -> Option<usize> {
        if is_span(value) {
            let first = self.first();
            return self.spans.lookup(column, &[value])?.first().map(|r| first + r);
        }
        self.view.key_row(column, value)
    }
    fn lookup_read(&self, column: usize, value: i64, columns: &[usize], each: &mut dyn FnMut(usize, &[i64])) -> bool {
        // A node's place is the view's read; a span's, its rows.
        if column == 0 && !is_span(value) {
            return self.view.lookup_read(column, value, columns, each);
        }
        let Some(rows) = self.lookup(column, &[value]) else { return false };
        let mut row = vec![0; columns.len()];
        for r in rows {
            self.row_at(r, columns, &mut row);
            each(r, &row);
        }
        true
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
