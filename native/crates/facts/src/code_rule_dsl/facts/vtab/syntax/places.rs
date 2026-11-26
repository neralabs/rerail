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
use rerail_blocks::vector::Vector;
use super::granules;
use std::collections::BTreeSet;
use std::sync::Arc;

/// The places core.rerail's shape reads as the `Loc` view, with rows only for `Extra`'s spans; and `text`, the `Text`
/// view's likewise (decision 0656 step 4: a node's or position's text is read off the tape, a span's is made with it).
pub(super) const NAMES: [&str; 5] = ["line", "end_line", "col", "docof", "text"];

/// A place relation: the `Loc` view's rows (`view`), then the rows of `Extra`'s spans (`spans`), numbered past them.
pub struct Places {
    view: Arc<dyn Source>,
    spans: Arc<dyn Source>,
    /// Where the device reads it, made on the first ask (`device_view`); `None` where it cannot.
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    device: std::sync::OnceLock<Option<OnDevice>>,
    /// Where the host's join programs read it (`rerail_accel::gpu::shared::on_host`), made on the first ask there.
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    host: std::sync::OnceLock<Option<OnHost>>,
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
    /// Held by a ledger at its address (decision 0491 step 2), as a pooled lease is: a launch that binds it holds it.
    dir: rerail_blocks::ledger::Tracked<rerail_device::device::Buffer>,
    tail: rerail_device::device::Buffer,
    _spans: crate::code_rule_dsl::facts::rows::InBlock,
    home: crate::code_rule_dsl::facts::rows::Part,
}

/// A placed relation as the host reads it: its `PlacedDir` at host addresses (the view's directory's, the spans'
/// block's), the spans' relation held while it is read, and its relation of no columns, as `OnDevice`'s.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
struct OnHost {
    dir: Box<PlacedDir>,
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
            #[cfg(any(target_os = "macos", target_os = "linux", windows))]
            host: std::sync::OnceLock::new(),
        }
    }

    /// The view's rows: the first span row's number.
    fn first(&self) -> usize {
        self.view.len()
    }

    /// The rows of the ids `values` on `column`, each id from the side that holds it on column 0 (a node's from the
    /// view, a span's from the spans), from both on another column: `None` when a part asked cannot answer.
    fn by_side(&self, column: usize, values: &[i64], of: impl Fn(&dyn Source, &[i64]) -> Option<Vector<usize>>) -> Option<Vector<usize>> {
        let (spans, nodes): (Vec<i64>, Vec<i64>) = if column == 0 { values.iter().partition(|&&v| is_span(v)) } else { (values.to_vec(), values.to_vec()) };
        let mut rows = if nodes.is_empty() { Vector::new() } else { of(&*self.view, &nodes)? };
        if !spans.is_empty() {
            let first = self.first();
            rows.extend(of(&*self.spans, &spans)?.into_iter().map(|r| first + r));
        }
        Some(rows)
    }

    /// Both parts' rows, the spans' past the view's: `None` when either cannot answer.
    fn both(&self, of: impl Fn(&dyn Source) -> Option<Vector<usize>>) -> Option<Vector<usize>> {
        let mut rows = of(&*self.view)?;
        let spans = of(&*self.spans)?;
        if !spans.is_empty() {
            let first = self.first();
            rows.extend(spans.iter().map(|&r| first + r));
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
        let at = dir.contents().cast::<u8>();
        let dir = rerail_blocks::ledger::Tracked::new(dir, at, std::mem::size_of::<PlacedDir>());
        let home = crate::code_rule_dsl::facts::rows::Part::for_device(0, self.len()).ok()?;
        Some(OnDevice { dir, tail, _spans: spans, home })
    }

    /// `on_device`'s `PlacedDir` at the host's addresses (`view_dir`: the view's directory's host address), for the
    /// host's join programs: the spans' block where it lies here, no device buffer.
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    fn on_host(&self, view_dir: u64) -> Option<OnHost> {
        let spans = self.spans.in_block()?;
        let tail = spans.facts.block.base_ptr() as u64;
        let dir = Box::new(PlacedDir { dir: view_dir, tail, at: spans.at as u64, rows: self.first() as u32, pad: 0 });
        let home = crate::code_rule_dsl::facts::rows::Part::for_device(0, self.len()).ok()?;
        Some(OnHost { dir, _spans: spans, home })
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
        // Where the join programs run on the host, the view's directory is at its host address (`device_view` of
        // the syntax source), and so is this one.
        if super::device_chunk::ViewsInPlace::read_on_host() {
            let on = self.host.get_or_init(|| self.on_host(v.dir)).as_ref()?;
            v.dir = std::ptr::from_ref::<PlacedDir>(&on.dir) as u64;
            v.kind |= VIEW_PLACED;
            d.rows = self.len();
            d.home = on.home.clone();
            return Some(d);
        }
        let on = self.device.get_or_init(|| self.on_device(v.dir)).as_ref()?;
        v.dir = on.dir.device_address()?;
        v.kind |= VIEW_PLACED;
        v.buffers.push((*on.dir).clone());
        v.buffers.push(on.tail.clone());
        d.rows = self.len();
        d.home = on.home.clone();
        Some(d)
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
    fn known_len(&self) -> Option<usize> {
        Some(self.view.known_len()? + self.spans.known_len()?)
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
    fn documents_of(&self, column: usize, each: &mut dyn FnMut(usize, i64)) {
        self.view.documents_of(column, each);
        let first = self.first();
        self.spans.documents_of(column, &mut |r, d| each(first + r, d));
    }
    fn rows_of_documents(&self, docs: &BTreeSet<u32>) -> Option<Vector<usize>> {
        self.both(|s| s.rows_of_documents(docs))
    }
    fn filter_eq(&self, eqs: &[(usize, i64)]) -> Vector<u32> {
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
    fn lookup_in(&self, column: usize, values: &[i64], doc: i64) -> Option<Vector<usize>> {
        self.by_side(column, values, |s, v| s.lookup_in(column, v, doc).or_else(|| s.lookup(column, v)))
    }
    fn indexed(&self, column: usize) -> bool {
        self.view.indexed(column) && self.spans.indexed(column)
    }
    fn keys_documents(&self, column: usize) -> bool {
        self.view.keys_documents(column)
    }
    fn lookup_cost(&self, column: usize) -> f64 {
        self.view.lookup_cost(column).max(self.spans.lookup_cost(column))
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        // A row's id is one row, the view's or a span's.
        if column == 0 { self.view.estimated_matches(column, batch) } else { self.view.estimated_matches(column, batch) + self.spans.estimated_matches(column, batch) }
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vector<usize>> {
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
        self.lookup_read_at(column, value, columns, &mut super::super::ProbeCursor::default(), each)
    }
    /// The view's matches read as found (its cursor), then the spans' (read as found, else looked up, else
    /// filtered, and read): a place relation's probe reads no row of the view a point read each. By a row's id, the
    /// side that holds it alone (`is_span`). Only the view may refuse, before any row is given; the spans always answer.
    fn lookup_read_at(&self, column: usize, value: i64, columns: &[usize], cursor: &mut super::super::ProbeCursor, each: &mut dyn FnMut(usize, &[i64])) -> bool {
        let width = columns.len();
        if width > 8 {
            return false;
        }
        let mut row = [0i64; 8];
        let routed = column == 0;
        if !(routed && is_span(value)) {
            if !self.view.lookup_read_at(column, value, columns, cursor, each) {
                let Some(rows) = self.view.lookup(column, &[value]) else { return false };
                for r in rows {
                    self.view.row_at(r, columns, &mut row[..width]);
                    each(r, &row[..width]);
                }
            }
            // A node's place is the view's read alone.
            if routed {
                return true;
            }
        }
        let first = self.first();
        if self.spans.lookup_read(column, value, columns, &mut |r, got| each(first + r, got)) {
            return true;
        }
        let rows = self.spans.lookup(column, &[value]).unwrap_or_else(|| (0..self.spans.len()).filter(|&r| self.spans.value(r, column) == value).collect());
        for r in rows {
            self.spans.row_at(r, columns, &mut row[..width]);
            each(first + r, &row[..width]);
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
    /// Counted in an array in a block of the run's (`place_statistics`). Counted as a view's rows by `RelStats`, its counts took 48 bytes a row on the heap (`line`'s and `col`'s 49.5 MB each at the
    /// M3's 300-file peak, ab160).
    fn statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        place_statistics(self, false)
    }
    /// `statistics`, or with its spans not made, their sample's added (`place_statistics`).
    fn estimated_statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        place_statistics(self, true)
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
struct Pairs(rerail_blocks::vector::Vector<[i64; 2]>);

impl Pairs {
    fn new() -> Pairs {
        Pairs(crate::code_rule_dsl::facts::rows::vector_in(None, 0))
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
/// With its spans not made, only an estimate (`estimate`) counts them, on their sample: the exact statistics are
/// counted where the spans lie (the chunks'), and a walk of them here would make them.
fn place_statistics(p: &Places, estimate: bool) -> Option<Vec<(String, i64, i64, i64, i64)>> {
    let names = p.columns();
    // `text`'s second column is a text: counted by value, every text of the batch would be interned to measure it.
    if names.len() != 2 || names[1] == "s" {
        return None;
    }
    let mut view = Pairs::new();
    let unmade = p.spans.as_any().and_then(|a| a.downcast_ref::<super::made::MadeRows>()).is_some_and(|s| !s.is_done());
    if unmade && !estimate {
        return None;
    }
    let sampled = if unmade { p.spans.estimated_statistics() } else { None };
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
