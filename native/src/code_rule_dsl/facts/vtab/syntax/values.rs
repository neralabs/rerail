//! The tape values a chunk's run admits inside its fixpoint (decision 0431 §8).
//!
//! A string a rule parses is answered inside the fixpoint (`rr_embed_req`'s answer, decision 0065 §2a): its tape is
//! found by its text's sha or parsed, and admitted once a chunk. Admitted, it is projected alone into the chunk's
//! block (`Grower::admit`), and each relation and view the batch makes has the value's rows as a part of its own.
//! The chunk's sources are bound as [`Layered`]: the batch's source (its base) and the parts admitted since, whose
//! rows are numbered past the base's. Every read of a base row is the base's; a lookup is the base's and those of the
//! parts that hold the looked-up node's key (a value's own, or a host's its places name); a scan reads the base and
//! then the parts admitted so far.
//!
//! A value's node is reached through a guest relation, which binds it (0058's hiding), so a rule looks it up after the
//! answer that admitted it: its rows are there when they are read. A read the base answers from an index of its own
//! rows (`sorted_rows`, `own_index`, a keyed read) cannot hold a part's, so a layered source offers none of those.

#[cfg(target_os = "macos")]
use super::super::DeviceView;
use super::super::Source;
use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// One admitted value's rows of a relation: its source (a batch of the value alone), from row `start` of the parts.
struct Part {
    start: usize,
    src: Arc<dyn Source>,
}

/// A relation's parts, in the order admitted, each found by the keys of the nodes it holds rows of.
#[derive(Default)]
struct Parts {
    list: Vec<Part>,
    total: usize,
    by_key: FxHashMap<u32, Vec<usize>>,
}

impl Parts {
    /// The part holding row `row` of the parts.
    fn at(&self, row: usize) -> Option<(&Part, usize)> {
        let i = self.list.partition_point(|p| p.start <= row).checked_sub(1)?;
        let p = &self.list[i];
        (row < p.start + p.src.len()).then(|| (p, row - p.start))
    }
    /// The parts that may hold rows whose value is `v`: those that hold node ids of `v`'s key, or every part for a
    /// value no key names (an atom, a number).
    fn of(&self, v: i64, each: &mut dyn FnMut(&Part)) {
        if !(1 << 32..).contains(&v) {
            self.list.iter().for_each(each);
            return;
        }
        for &i in self.by_key.get(&((v >> 32) as u32)).into_iter().flatten() {
            each(&self.list[i]);
        }
    }
}

/// The chunk's admitted values: each relation's parts, shared by its layered sources.
#[derive(Default)]
pub struct Values {
    parts: RwLock<FxHashMap<String, Parts>>,
    admitted: AtomicUsize,
}

impl Values {
    /// Value `keys[0]`'s sources: each relation's rows of it, a part of that relation from now on. `keys` are the
    /// keys of the nodes its rows hold: its own, and its places' hosts'.
    pub fn add(&self, keys: &[u32], sources: Vec<(String, Arc<dyn Source>)>) {
        let mut parts = self.parts.write();
        for (name, src) in sources {
            // A relation made on lookup (containment, an axis) has its rows only once a lookup asks.
            if src.len() == 0 && !src.made_on_lookup() {
                continue;
            }
            let p = parts.entry(name).or_default();
            let i = p.list.len();
            p.list.push(Part { start: p.total, src: Arc::clone(&src) });
            p.total += src.len();
            for &k in keys {
                let at = p.by_key.entry(k).or_default();
                if at.last() != Some(&i) {
                    at.push(i);
                }
            }
        }
        self.admitted.fetch_add(1, Ordering::Relaxed);
    }
    /// The values admitted.
    pub fn admitted(&self) -> usize {
        self.admitted.load(Ordering::Relaxed)
    }
    /// The source of relation `name` that holds rows of node `n`'s document: an admitted value's.
    pub fn source_of(&self, name: &str, n: i64) -> Option<Arc<dyn Source>> {
        let parts = self.parts.read();
        let p = parts.get(name)?;
        let i = *p.by_key.get(&((n >> 32) as u32))?.first()?;
        Some(Arc::clone(&p.list[i].src))
    }
    fn total(&self, name: &str) -> usize {
        self.parts.read().get(name).map_or(0, |p| p.total)
    }
}

/// Where a layered source made on lookup numbers the rows it makes of a value's (`Made`): past every row containment
/// makes (`within`'s outer rows start at `1 << 31`, and a chunk makes far fewer than `1 << 30` of them), within the
/// `u32` the engine reads a row by.
const MADE_AT: usize = 0xF000_0000;
const MADE_ROOM: usize = (1 << 32) - MADE_AT;

/// The rows a layered source made on lookup made of the values' parts (containment's, an axis's answers): each
/// looked-up value's rows written once, where they are computed, as `within` writes its own.
#[derive(Default)]
struct Made {
    rows: Vec<i64>,
    /// Each looked-up value's run of rows, and how many parts there were when it was made: a part admitted since (a
    /// value given a place more) makes it again.
    runs: FxHashMap<(usize, i64), ((usize, usize), usize)>,
}

/// A chunk's source of relation `name`: the batch's (`base`), and the values admitted since, past its rows.
pub struct Layered {
    name: String,
    base: Arc<dyn Source>,
    base_len: usize,
    values: Arc<Values>,
    /// For a base made on lookup, whose rows grow as it is read: the parts' rows, made as they are looked up.
    made: Option<parking_lot::Mutex<Made>>,
    width: usize,
}

impl Layered {
    pub fn new(name: &str, base: Arc<dyn Source>, values: Arc<Values>) -> Self {
        let base_len = base.len();
        let made = base.made_on_lookup().then(|| parking_lot::Mutex::new(Made::default()));
        let width = base.columns().len();
        Layered { name: name.to_owned(), base, base_len, values, made, width }
    }
    /// The batch's source.
    pub fn base(&self) -> &Arc<dyn Source> {
        &self.base
    }
    /// The source that holds node `n`'s document's rows: an admitted value's, else the batch's.
    pub fn source_of(&self, n: i64) -> Arc<dyn Source> {
        self.values.source_of(&self.name, n).unwrap_or_else(|| Arc::clone(&self.base))
    }
    fn parts<T>(&self, f: impl FnOnce(&Parts) -> T) -> Option<T> {
        self.values.parts.read().get(&self.name).filter(|p| !p.list.is_empty()).map(f)
    }
    fn has_parts(&self) -> bool {
        self.values.total(&self.name) > 0
    }
}

impl Source for Layered {
    fn python_batch(&self) -> Option<Arc<super::Batch>> {
        self.base.python_batch()
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn columns(&self) -> Vec<String> {
        self.base.columns()
    }
    fn text_column(&self, column: usize) -> bool {
        self.base.text_column(column)
    }
    fn len(&self) -> usize {
        // Made on lookup, the base's rows grow as it is read; a value's are past them (`MADE_AT`), never scanned.
        if self.made.is_some() {
            return self.base.len();
        }
        self.base_len + self.values.total(&self.name)
    }
    fn estimated_len(&self) -> usize {
        self.base.estimated_len()
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        self.base.estimated_matches(column, batch)
    }
    fn lookup_cost(&self, column: usize) -> f64 {
        self.base.lookup_cost(column)
    }
    fn statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        self.base.statistics()
    }
    fn indexed(&self, column: usize) -> bool {
        self.base.indexed(column)
    }
    fn made_on_lookup(&self) -> bool {
        self.base.made_on_lookup()
    }
    fn complete_rows(&self) -> bool {
        self.base.complete_rows()
    }
    fn expect_lookups(&self, column: usize, keys: f64) {
        self.base.expect_lookups(column, keys)
    }
    fn failure(&self) -> Option<String> {
        self.base.failure()
    }
    fn doc_column(&self) -> Option<usize> {
        self.base.doc_column()
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>> {
        let mut out = self.base.lookup(column, values)?;
        if let Some(made) = &self.made {
            self.parts(|p| {
                let mut made = made.lock();
                for &v in values {
                    let run = match made.runs.get(&(column, v)) {
                        Some(&(run, parts)) if parts == p.list.len() => run,
                        _ => {
                            let at = made.rows.len() / self.width.max(1);
                            p.of(v, &mut |part| {
                                for r in part.src.lookup(column, &[v]).unwrap_or_default() {
                                    for c in 0..self.width {
                                        let x = part.src.value(r, c);
                                        made.rows.push(x);
                                    }
                                }
                            });
                            let run = (at, made.rows.len() / self.width.max(1) - at);
                            assert!(at + run.1 <= MADE_ROOM, "{}: more rows made of the tape values than its room", self.name);
                            made.runs.insert((column, v), (run, p.list.len()));
                            run
                        }
                    };
                    out.extend((run.0..run.0 + run.1).map(|r| MADE_AT + r));
                }
            });
            return Some(out);
        }
        self.parts(|p| {
            for &v in values {
                p.of(v, &mut |part| {
                    let at = self.base_len + part.start;
                    match part.src.lookup(column, &[v]) {
                        Some(rows) => out.extend(rows.into_iter().map(|r| at + r)),
                        // A part that keeps no index of `column` is read whole: a value's rows are few.
                        None => out.extend((0..part.src.len()).filter(|&r| part.src.value(r, column) == v).map(|r| at + r)),
                    }
                });
            }
        });
        Some(out)
    }
    fn lookup_read(&self, column: usize, value: i64, columns: &[usize], each: &mut dyn FnMut(usize, &[i64])) -> bool {
        !self.has_parts() && self.base.lookup_read(column, value, columns, each)
    }
    fn lookup_eq(&self, column: usize, value: i64, eqs: &[(usize, i64)]) -> Option<Vec<usize>> {
        if self.has_parts() { None } else { self.base.lookup_eq(column, value, eqs) }
    }
    fn lookup_in(&self, column: usize, values: &[i64], doc: i64) -> Option<Vec<usize>> {
        if self.has_parts() { None } else { self.base.lookup_in(column, values, doc) }
    }
    fn kept_to_doc(&self, column: usize) -> bool {
        self.base.kept_to_doc(column)
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        if let Some(made) = &self.made {
            if row < MADE_AT {
                return self.base.value(row, column);
            }
            return made.lock().rows.get((row - MADE_AT) * self.width + column).copied().unwrap_or(0);
        }
        if row < self.base_len {
            return self.base.value(row, column);
        }
        self.parts(|p| p.at(row - self.base_len).map(|(part, r)| part.src.value(r, column))).flatten().unwrap_or(0)
    }
    fn row_at(&self, row: usize, columns: &[usize], out: &mut [i64]) {
        if self.made.is_some() {
            if row < MADE_AT {
                return self.base.row_at(row, columns, out);
            }
            for (o, &c) in out.iter_mut().zip(columns) {
                *o = self.value(row, c);
            }
            return;
        }
        if row < self.base_len {
            return self.base.row_at(row, columns, out);
        }
        self.parts(|p| {
            if let Some((part, r)) = p.at(row - self.base_len) {
                part.src.row_at(r, columns, out);
            }
        });
    }
    fn for_rows(&self, columns: &[usize], each: &mut dyn FnMut(usize, &[i64])) {
        self.base.for_rows(columns, each);
        if self.made.is_some() {
            return;
        }
        self.parts(|p| {
            for part in &p.list {
                let at = self.base_len + part.start;
                part.src.for_rows(columns, &mut |r, row| each(at + r, row));
            }
        });
    }
    fn for_rows_in(&self, range: Range<usize>, columns: &[usize], each: &mut dyn FnMut(usize, &[i64]) -> bool) {
        if self.made.is_some() {
            return self.base.for_rows_in(range, columns, each);
        }
        let base = range.start.min(self.base_len)..range.end.min(self.base_len);
        let mut going = true;
        if !base.is_empty() {
            self.base.for_rows_in(base, columns, &mut |r, row| {
                going = each(r, row);
                going
            });
        }
        if !going || range.end <= self.base_len {
            return;
        }
        let mut row = vec![0i64; columns.len()];
        for r in range.start.max(self.base_len)..range.end.min(self.len()) {
            self.row_at(r, columns, &mut row);
            if !each(r, &row) {
                return;
            }
        }
    }
    fn documents_of(&self, column: usize, each: &mut dyn FnMut(usize, i64)) {
        self.base.documents_of(column, each);
        if self.made.is_some() {
            return;
        }
        self.parts(|p| {
            for part in &p.list {
                let at = self.base_len + part.start;
                part.src.documents_of(column, &mut |r, d| each(at + r, d));
            }
        });
    }
    #[cfg(target_os = "macos")]
    fn device_view(&self) -> Option<DeviceView<'_>> {
        None
    }
}
