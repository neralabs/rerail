//! Read-only eponymous virtual tables over integer relations (decision 0008; 05 Phase 7 row 3).
//!
//! A table is registered once per connection and rebound per wave without DDL or a reprepare: its
//! [`Binding`] swaps the [`Source`] that later filters read, while open cursors keep the one they
//! started with. `xBestIndex` takes an `EQ` or `IN` constraint on an indexed column (the whole `IN`
//! list in one filter) or a range on a range-indexed column. Several `EQ` are one filter: a source's
//! sorted order over them all (`composite`), else a probe of the most selective column with the rest
//! checked here; estimates come from the source's distinct counts, so SQL written by hand plans well.
//! SQLite rechecks every constraint, so a source may answer with a superset. Every column is an integer: kinds, fields and symbols are atoms.
use rerail_blocks::vector::Vector;
use fallible_iterator::FallibleIterator;
use rusqlite::{
    Connection, Result,
    types::Value,
    vtab::{
        Context, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig, VTabConnection,
        VTabCursor,
    },
};
use std::{
    borrow::Cow,
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    ffi::{CStr, CString, c_int},
    ops::Range,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};
pub mod schema;
// cratify: `syntax` is rerail-facts' (it mounts it, over the base)
// cratify: `python` is rerail-lang-python's (it mounts it)
pub mod demand;

/// `idx_num` of a filter on several `=` columns, listed in `idx_str`.
const MULTI: c_int = 1 << 22;
const ORDERED: c_int = 1 << 23;
const PRIMARY_BATCH: c_int = 1 << 24;

pub fn failure(message: &str) -> rusqlite::Error {
    rusqlite::Error::UserFunctionError(message.to_owned().into())
}

/// A native relation with integer lookup columns, stable while SQL cursors read it.
/// Text result columns stay in SQLite's filters and never enter integer probes.
/// A mutable owner must close every reader before changing rows, then refresh its binding
/// and planner estimates. Most producers expose immutable snapshots.
pub trait RowCells: Send+Sync {
    fn cell(&self,column:usize)->Value;
}
pub struct ReadRow {pub id:i64,pub cells:Arc<dyn RowCells>}
pub trait RowStream:Send {fn next(&mut self)->Option<ReadRow>;}
/// A view the device reads in place (`Source::device_view`, decision 0353 step 4): nothing makes its rows.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
pub struct DeviceView {
    pub view: u32,
    pub rows: usize,
    /// Where a stage reads it: its batch's directory, the buffers its reads reach.
    pub in_place: ViewInPlace,
    /// The view's relation of no columns, whose block its probes' indexes go in (decision 0478 step 3, R4): made once
    /// by the source, which holds it (its batch's life), so every stratum on any thread finds them.
    pub home: crate::code_rule_dsl::facts::rows::Part,
}

/// A view a stage reads in place (decision 0353 step 4d): its batch's directory's GPU address, the view's number in
/// the kernels' reads (1 + `device_chunk::VIEW_*`), the buffers its reads reach by address (the device layer's, as
/// the stages bind them), and what keeps them. Made where the process's device is Metal: the directory is of Metal's
/// addresses (`device_chunk::views_in_place`).
/// A 16z batch's views read their blocks through its cache (`caches`, decision 0353 step 4b), which its reads write:
/// a stage that reads the view is refused when the caches' failures rise past `failed` (no room found). A layered
/// source's view reads each of its batches' caches.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
#[derive(Clone)]
pub struct ViewInPlace {
    pub dir: u64,
    pub kind: u32,
    pub buffers: Vec<rerail_device::device::Buffer>,
    pub caches: Vec<ZCache>,
    pub failed: u32,
    /// Its batch's tape format (`ViewsInPlace::format`; 0: several), which the stages reading it are compiled for.
    pub format: u32,
    pub keep: Arc<dyn std::any::Any + Send + Sync>,
}

/// A `has` axis read off the tape (`Source::has_walk`): its batch's `node` view in place, of `nodes` rows, through which
/// a stage climbs a node's ancestors or reads a node's subtree, and its set's kind atoms, ascending.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
pub struct HasWalk {
    pub nodes: usize,
    pub view: ViewInPlace,
    pub kinds: Arc<[i64]>,
}

/// A batch's 16z cache as its views' directory names it (decision 0353 step 4b, `view::cache`): the batch's one cache
/// (`rerail_tape_format::f16::ZCache`, decision 0628 §6), which its host's reads decode into, a home of its views'
/// directory that the host's and the device's reads of its views share; and its device buffer where the cache is in
/// the device's memory.
#[derive(Clone)]
pub struct ZCache {
    pub cache: Arc<rerail_tape_format::f16::ZCache>,
    pub buffer: Option<rerail_device::device::Buffer>,
    /// The most bytes an entry takes (`Spec::entry_bytes`): a view's entry size where its reads share the cache.
    pub entry: usize,
}

impl ZCache {
    /// Its counts, as its reads left them.
    pub fn counts(&self) -> rerail_clif::view::cache::Counts {
        // SAFETY: the cache's bytes, which this holds.
        rerail_clif::view::cache::Counts::checked(unsafe { std::slice::from_raw_parts(self.cache.at(), self.cache.bytes()) })
    }
}

#[cfg(any(target_os = "macos", target_os = "linux", windows))]
impl ViewInPlace {
    /// Its batches' caches' counts (none without one): for tuning, in the statistics mode (`RERAIL_ZCACHE_STATS`).
    pub fn cache_counts(&self) -> Vec<rerail_clif::view::cache::Counts> {
        self.caches.iter().map(ZCache::counts).collect()
    }

    /// The failures its batches' caches have counted (0 without one).
    pub fn cache_failed(&self) -> u32 {
        self.cache_counts().iter().map(|c| c.failed).sum()
    }
}

/// A join step's probe cursor (`Source::lookup_read_at`), kept by the step across its probes: where the last probe
/// ended (`hint`: a view of the tape's document and granule), and the source's plan of what a match reads (`plan`),
/// made on the step's first probe. A hint is checked, never trusted; a plan is kept only for the column and columns
/// it was made for.
#[derive(Clone, Debug, Default)]
pub struct ProbeCursor {
    pub hint: [u32; 2],
    pub plan: ReadPlan,
    /// A second key column's value of this probe, `(column, value)`, which the source may keep its matches to (a
    /// child by its place: `child(p, c, i)` by `p` and `i`). A hint too: the caller checks every key column.
    pub also: Option<(usize, i64)>,
    /// Where the last probe's id was found in its document (a view of the tape): its slot's entry in the granule
    /// table, and its select's group. Hints, checked before use; any value gives the same rows.
    pub near: [u32; 2],
    /// The read's memo of the node id a probe last found, where the source reads nodes (`NodeMemo`): one cell the
    /// join gives each of its steps' cursors, as a rule's body probes several views by the same node in turn, each
    /// from inside the step before's read. None: the probe's own (a lookup alone).
    pub node: Option<std::rc::Rc<std::cell::Cell<NodeMemo>>>,
}

/// A source's memo of the node id it last found (`Batch::node_hint`): its documents (their address), the id, its
/// document, local id and tape node (`u32::MAX` until read), and the document's projection (its address). Checked
/// against all of these, never trusted: a probe of `kind(n, k)` then `text(n, s)` then `docof(n, d)` by the same `n`
/// searches the id's granule and selects its tape node once, where each searched and selected again. The default
/// matches no documents.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NodeMemo {
    pub docs: usize,
    pub id: i64,
    pub doc: u32,
    pub local: u32,
    pub x: u32,
    pub at: usize,
}

/// A source's plan of a probe's reads (`ProbeCursor::plan`): for the key column and output columns it was made for, one
/// op a slot and the inputs a match needs, in the source's own codes. Decided once a step, not once a match.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReadPlan {
    pub made: bool,
    pub column: u8,
    pub len: u8,
    pub columns: [u8; 8],
    pub ops: [u8; 8],
    pub needs: u8,
}

impl ReadPlan {
    /// Whether it was made for key `column` and output `columns`.
    #[inline]
    pub fn is_for(&self, column: usize, columns: &[usize]) -> bool {
        self.made && usize::from(self.column) == column && usize::from(self.len) == columns.len() && columns.iter().zip(&self.columns).all(|(&c, &k)| c == usize::from(k))
    }
    /// An empty plan for key `column` and output `columns` (at most 8, each below 256), its ops to fill.
    pub fn new(column: usize, columns: &[usize]) -> Option<Self> {
        if columns.len() > 8 || column > 255 || columns.iter().any(|&c| c > 255) {
            return None;
        }
        let mut cols = [0u8; 8];
        for (k, &c) in columns.iter().enumerate() {
            cols[k] = c as u8;
        }
        Some(ReadPlan { made: true, column: column as u8, len: columns.len() as u8, columns: cols, ops: [0; 8], needs: 0 })
    }
}


/// A reader's memo of a source's search before its first read (`Source::row_at_memo`): none found yet.
pub const NO_MEMO: u32 = rerail_clif::view_reads::NO_DOC;
/// The relation of the nodes whose text `@rr_node_text` reads, bound with the batch every node read goes through
/// (the syntax table's `LongTexts`).
pub const LONG_TEXT_NODES: &str = "rr_long_text_node";

/// The functions that read a node's text, value or order from the run's batch (`Source::node_read`): their values are
/// the run's, so `functions::eval` gives none. `rr_node_order` is a node's preorder key, `node_order`'s `k`: what a
/// comparison of two nodes reads (`lowering_tail::OrderNodes`), with no join. `name`, `comment`, `strprefix`, `op_at`
/// and T's `text`, `intval` and `op` are core.rerail's and T's per-node values, read where a rule binds the node
/// (decision 0353 step 3).
pub const NODE_READS: [&str; 12] = ["rr_node_text", "rr_node_text_any", "rr_node_value", "rr_node_str", "rr_node_name", "rr_node_comment", "rr_node_strprefix", "rr_node_op_at", "rr_node_intval", "rr_node_op_t", "rr_node_t_text", "rr_node_order"];

/// A row a producer hands a relation (`Relation::from_rows`, `staged`): its values on the stack, as the relation
/// copies them into its block (decision 0687); a row past 8 columns goes on the heap.
pub type Row = rerail_blocks::frame::InlineVec<i64, 8>;

/// A `Row` of its values: `row![a, b]`.
#[macro_export]
macro_rules! row {
    ($($v:expr),* $(,)?) => {{
        let mut r = $crate::code_rule_dsl::facts::vtab::Row::new();
        $(r.push($v);)*
        r
    }};
}

/// A lookup's rows read by position (`Source::lookup_seq`): how many, and the `i`th, where they lie (a list the source
/// holds, its runs of rows), or a list made of them.
pub struct Seq<'a> {
    len: usize,
    nth: Box<dyn Fn(usize) -> usize + 'a>,
}

impl<'a> Seq<'a> {
    /// `len` rows, the `i`th `nth(i)`.
    pub fn new(len: usize, nth: impl Fn(usize) -> usize + 'a) -> Self {
        Seq { len, nth: Box::new(nth) }
    }
    /// The rows `rows`, in order.
    pub fn rows(rows: rerail_blocks::vector::Vector<usize>) -> Self {
        Seq::new(rows.len(), move |i| rows[i])
    }
    /// The rows of `runs`, in order: the `i`th's run by bisection of their starts.
    pub fn runs(runs: Vec<std::ops::Range<usize>>) -> Self {
        let mut starts = Vec::with_capacity(runs.len());
        let len = runs.iter().fold(0, |at, r| {
            starts.push(at);
            at + r.len()
        });
        Seq::new(len, move |i| {
            let j = starts.partition_point(|&s| s <= i) - 1;
            runs[j].start + (i - starts[j])
        })
    }
    /// `self`'s rows, then `more`'s.
    pub fn then(self, more: Seq<'a>) -> Self {
        let at = self.len;
        Seq::new(self.len + more.len, move |i| if i < at { self.nth(i) } else { more.nth(i - at) })
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    /// The `i`th row (`i < len`).
    pub fn nth(&self, i: usize) -> usize {
        (self.nth)(i)
    }
    /// Its rows, in order.
    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.len).map(|i| self.nth(i))
    }
}

/// A node read (`Source::node_read_ref`): a text the source lends, or a value made.
pub enum NodeRead<'a> {
    Text(&'a str),
    Value(rusqlite::types::Value),
}

pub trait Source: Send + Sync {
    /// The syntax batch it reads (the syntax table's `Batch`, rerail-facts'), which its readers take as their own
    /// type (`syntax::PythonBatch`).
    fn syntax_batch(&self) -> Option<Arc<dyn std::any::Any + Send + Sync>> { None }
    /// Node `n`'s text, value or order as the function `name` reads it (`NODE_READS`), its further arguments `more`
    /// (`op_at`'s place), for a binding of `LONG_TEXT_NODES` (the syntax table's `LongTexts`); none for another source
    /// or function.
    fn node_read(&self, _name: &str, _n: i64, _more: &[i64]) -> Option<rusqlite::types::Value> { None }
    /// `node_read`, a text lent where the source holds it, with none made: what a join's walk reads
    /// (`NodeRead::Text`).
    fn node_read_ref(&self, name: &str, n: i64, more: &[i64]) -> Option<NodeRead<'_>> { self.node_read(name, n, more).map(NodeRead::Value) }
    /// Column `c`'s rows a value, where the source knows them without reading its rows (`docof`'s documents: each
    /// one's nodes); `None` otherwise.
    fn column_counts(&self, _c: usize) -> Option<Vec<(i64, u64)>> { None }
    fn row_stream(&self,_range:Range<usize>,_reverse:bool)->Option<Box<dyn RowStream>> {None}
    fn direct_all(&self,_equal:&[(usize,i64)])->Option<Vec<ReadRow>> {None}
    fn direct_one(&self,_column:usize,_values:&[i64])->Option<Vec<ReadRow>> {None}
    fn prepare_lookup(&self,_columns:&[usize]) {}
    fn caches_rows(&self)->bool {false}
    fn row_cells(&self,_row:usize)->Option<(i64,Arc<dyn RowCells>)> {None}
    fn prepare_index(&self, _columns:&[usize]) {}
    fn primary_key_len(&self)->Option<usize> {None}
    fn lookup_primary_batch(&self, _values:&[Vector<i64>])->Option<rerail_blocks::vector::Vector<usize>> {None}
    /// Pin a stable version before a SQL cursor starts. Immutable sources need no copy.
    fn frozen(&self) -> Option<Arc<dyn Source>> { None }
    /// A stable data revision for this source identity; None disables cross-gate reuse.
    fn revision(&self) -> Option<u64> { None }
    fn cell(&self,row:usize,column:usize)->Value { Value::Integer(self.value(row,column)) }
    fn row_id(&self,row:usize)->i64 { row as i64+1 }
    fn writable(&self)->bool {false}
    fn ordered(&self,_equal:&[usize],_order:&[(usize,bool)])->bool {false}
    fn ordered_range(&self,_equal:&[(usize,i64)])->Option<Range<usize>> {None}
    /// SQL writes are available only to explicitly mutable native relations.
    fn write(&self,_old:Option<i64>,_new:Option<i64>,_values:Option<Vec<Option<i64>>>,_replace:bool,_ignore:bool)->Result<i64> {
        Err(rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_READONLY),None))
    }
    fn text_column(&self, _column: usize) -> bool { false }
    /// The source as itself, for a caller that reads a kind's own parts (a derived relation's rows).
    fn as_any(&self) -> Option<&dyn std::any::Any> { None }
    fn columns(&self) -> Vec<String>;
    fn len(&self) -> usize;
    /// Whether it has no row: `len() == 0` by default; a source that counts its rows by a walk (an axis) stops at the
    /// first.
    fn holds_none(&self) -> bool {
        self.len() == 0
    }
    /// Row `row`'s values of `columns`, into `out`: one value at a time by default; a source that finds a row's
    /// place first (a view of a batch's documents) finds it once.
    fn row_at(&self, row: usize, columns: &[usize], out: &mut [i64]) {
        for (o, &c) in out.iter_mut().zip(columns) {
            *o = self.value(row, c);
        }
    }
    /// `row_at`, with the caller's memo: what the source's search for a row last found (the document a views
    /// directory's search found, `ViewDir::doc_of_row`), handed back on the caller's next read (decision 0353 step 3
    /// on the CPU). Any value is correct, a stale one costs a comparison: a reader starts one at `NO_MEMO` and keeps
    /// it across its reads of this source. A source with no search ignores it.
    fn row_at_memo(&self, row: usize, columns: &[usize], out: &mut [i64], memo: &mut u32) {
        let _ = memo;
        self.row_at(row, columns, out)
    }
    /// Each row's values of `columns`, row after row, given to `each` with the row's number: `row_at` a row at a time
    /// by default; a view of a batch's documents walks each document's nodes once, finding no row's place and
    /// selecting no node.
    fn for_rows(&self, columns: &[usize], each: &mut dyn FnMut(usize, &[i64])) {
        let (mut row, mut memo) = (vec![0i64; columns.len()], NO_MEMO);
        for r in 0..self.len() {
            self.row_at_memo(r, columns, &mut row, &mut memo);
            each(r, &row);
        }
    }
    /// The rows whose `column` is `value`, each with its values of `columns` (at most 8), given to `each`, as
    /// `lookup` and `row_at` would; `false` (nothing given) where the source has no such read. A view of the tape
    /// gives each match's values from what the lookup found it by (its tape node, its own or its parent's id), not
    /// a point read a row (decision 0353 step 2).
    fn lookup_read(&self, _column: usize, _value: i64, _columns: &[usize], _each: &mut dyn FnMut(usize, &[i64])) -> bool {
        false
    }
    /// `lookup_read` from a probe cursor (`hint`, the step's, zeroed at its start): where the last probe of the step
    /// ended, which a source may keep its place in (a view of the tape: the document and granule). Probes in key
    /// order (a scan's rows sorted by the next key, decision 0112) then find their place without a search. A hint is
    /// checked, never trusted: any value gives the same rows.
    fn lookup_read_at(&self, column: usize, value: i64, columns: &[usize], _cursor: &mut ProbeCursor, each: &mut dyn FnMut(usize, &[i64])) -> bool {
        self.lookup_read(column, value, columns, each)
    }
    /// Whether some row with `value` in `column` has `want` in `columns` (a membership step's test, `exists`), read
    /// as `lookup_read_at` would find it, without a row or its values handed on: `None` where the source has no such
    /// test (the caller reads the matches instead).
    fn test_at(&self, _column: usize, _value: i64, _columns: &[usize], _want: &[i64], _cursor: &mut ProbeCursor) -> Option<bool> {
        None
    }
    /// `for_rows` over rows `range` alone, until `each` returns false: a join step's scan of a view reads its rows
    /// so (decision 0353 step 2), a view of a batch's documents walking the documents the range holds once, each row's
    /// node, id and parent coming from the walk, not a point read a row.
    fn for_rows_in(&self, range: Range<usize>, columns: &[usize], each: &mut dyn FnMut(usize, &[i64]) -> bool) {
        let (mut row, mut memo) = (vec![0i64; columns.len()], NO_MEMO);
        for r in range {
            self.row_at_memo(r, columns, &mut row, &mut memo);
            if !each(r, &row) {
                return;
            }
        }
    }
    /// The rows whose `column` holds one of `keys`, each with its values of `columns`, in row order, until `each`
    /// returns false; `false` (nothing given) where the source has no such read. A view of a batch's documents walks
    /// the documents the keys name, reading each row's own column and the other columns of the rows kept alone:
    /// the walk carries a node's depth and place, where a lookup a key computes them again (a semi-join's rows,
    /// decision 0458).
    fn for_rows_keyed(&self, _column: usize, _keys: &rustc_hash::FxHashSet<i64>, _columns: &[usize], _each: &mut dyn FnMut(usize, &[i64]) -> bool) -> bool {
        false
    }
    /// Each row's document: the high 32 bits of the id in `column` (a value below `1 << 32` is its own document), row
    /// after row with the row's number. A view of a batch's documents gives each document's rows its key, reading no
    /// row's id.
    fn documents_of(&self, column: usize, each: &mut dyn FnMut(usize, i64)) {
        self.for_rows(&[column], &mut |r, row| each(r, if row[0] >= 1 << 32 { row[0] >> 32 } else { row[0] }));
    }
    fn estimated_len(&self) -> usize {
        self.len()
    }
    /// Whether it may hold a row: `false` only where it holds none. A test for an empty input (a rule over one is
    /// not run) asks this, not `len`, so a source whose rows are made when read (a batch's, decision 0494) answers
    /// without making them, and is taken as holding rows.
    fn may_hold_rows(&self) -> bool {
        self.len() > 0
    }
    /// The rows whose column `g` holds a node `owned` names, where the source finds them without reading every row
    /// (a view over a batch's documents, by its nodes): `None` when it cannot, and the caller reads them all.
    fn rows_owned(&self, _g: usize, _owned: &dyn Fn(i64) -> bool) -> Option<rerail_blocks::vector::Vector<usize>> {
        None
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize;
    /// A lookup on `column`'s CPU, as a multiple of a derived relation's (placement's `CPU_PER_PROBE` a row in and
    /// `CPU_PER_ROW_OUT` a row out; decision 0312): a view whose lookup walks the tree or searches the documents
    /// costs several. Fitted from the profile's `sources.tsv`.
    fn lookup_cost(&self, _column: usize) -> f64 {
        1.0
    }
    /// A scan's work a row it gives, as a multiple of a derived relation's: one, but where the rows a scan gives are
    /// a few of what it reads (a text test of a batch not yet made: every string of every document, made and
    /// tested). The planner prices a scan of it so (`Stats::scan_cost`), and leads with what probes it.
    fn scan_cost(&self) -> f64 {
        1.0
    }
    /// The rows estimated to match one value on each of `columns` at once. By default the columns are taken
    /// as independent (each keeps its share of the rows), bounded by the most selective one.
    fn estimated_matches_all(&self, columns: &[usize]) -> usize {
        let n = self.estimated_len().max(1) as f64;
        let each: Vector<usize> = columns.iter().map(|&c| self.estimated_matches(c, false)).collect();
        let independent = each.iter().fold(n, |rows, &m| rows * (m as f64 / n).min(1.0));
        (independent.ceil() as usize).max(1).min(each.into_iter().min().unwrap_or(usize::MAX))
    }
    /// Whether `lookup_all` answers from one sorted order over several columns.
    fn composite(&self) -> bool {
        false
    }
    /// The rows whose columns equal the values (`(column, value)` by column, at least two), from one sorted
    /// order over them; `None` probes one column and filters the rest.
    fn lookup_all(&self, _equal: &[(usize, i64)]) -> Option<rerail_blocks::vector::Vector<usize>> {
        None
    }
    /// The rows of the documents `docs` (by key), ascending, where the source knows where they lie (decision 0494: a
    /// batch's, by group or by offset); `None` where it does not, and they are found by a read of its rows.
    fn rows_of_documents(&self, _docs: &std::collections::BTreeSet<u32>) -> Option<rerail_blocks::vector::Vector<usize>> {
        None
    }
    /// Whether the relation holds the row of these values, every column given (`(column, value)`), answered by a
    /// test and not by its rows: a join step that reads no column of the row asks only this (`within(n, a)` with
    /// both bound is an interval test on the tape, t50 step 4). `None`: the source has no such test; `lookup_all`.
    fn holds(&self, _equal: &[(usize, i64)]) -> Option<bool> {
        None
    }
    /// The rows whose every column `columns[i]` is `values[i]` (a probe binding several key columns), found where the
    /// source can keep the probe to the rows the columns together name: a made relation's to the documents every
    /// bound column routes to (`docof(n, d)` by both: none where `n`'s document is not `d`'s), none made for the
    /// others. `None`: no such probe; the caller probes one column and checks the rest.
    fn lookup_columns(&self, _columns: &[usize], _values: &[i64]) -> Option<rerail_blocks::vector::Vector<usize>> {
        None
    }
    /// Whether `lookup_columns` may answer: a probe of several columns is then that lookup, not one column's
    /// streamed read with the rest checked.
    fn looks_up_columns(&self) -> bool {
        false
    }
    /// The column holding a row's document, where a lookup can be kept to one document (`lookup_in`).
    fn doc_column(&self) -> Option<usize> {
        None
    }
    /// The rows whose `column` is one of `values` in document `doc` (by its key): a lookup over a column that
    /// spans the documents (a kind) where the statement binds the document too. `None`: not kept to one.
    fn lookup_in(&self, _column: usize, _values: &[i64], _doc: i64) -> Option<rerail_blocks::vector::Vector<usize>> {
        None
    }
    /// Whether a lookup on `column` can be kept to one document (`lookup_in`).
    fn kept_to_doc(&self, _column: usize) -> bool {
        false
    }
    /// The rows whose columns hold every `(column, value)` of `eqs`, ascending: a scan, which a source may do faster
    /// than a `value` call per row.
    fn filter_eq(&self, eqs: &[(usize, i64)]) -> Vector<u32> {
        (0..self.len()).filter(|&r| eqs.iter().all(|&(c, v)| self.value(r, c) == v)).map(|r| r as u32).collect()
    }
    /// A `has` axis the device reads off the tape (decision 0353 step 5): a stage probes it by one column, each
    /// binding's rows walked there (`metal_join::Batch::by_has`). `None` for any other source.
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    fn has_walk(&self) -> Option<HasWalk> {
        None
    }
    /// The `node` view of the batch whose nodes' texts this source reads (`node_read`), where the device reads every
    /// one of its documents' sources in place (decision 0287 §2, `ViewsInPlace::sources`): its rows and directory,
    /// through which a stage reads a node's text where it lies. `None` for any other source.
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    fn node_view(&self) -> Option<(usize, ViewInPlace)> {
        None
    }
    /// A view the device reads in place (decision 0353 step 4): its code (`syntax::device_chunk::VIEW_*`), its rows,
    /// its directory. `None`: the device does not read it where it lies.
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    fn device_view(&self) -> Option<DeviceView> {
        None
    }
    /// Its rows as one relation of `rerail_blocks` in one block, every source column at its own place, which the
    /// device reads where they lie (decision 0325 step 3), a probe reads through the relation's own index, and the CPU's
    /// index of a whole source finds or builds in its block (setfix), on every target. `None`: the device copies them
    /// (`device_view`, `row_at`).
    fn in_block(&self) -> Option<super::rows::InBlock> {
        None
    }
    /// Its own index on `columns` (decision 0325 step 5), where the source holds its rows to be probed so (`held`):
    /// found or built in its block, its number with the relation it indexes. A probe reads a bucket's rows, candidates
    /// whose key it checks. `None` by default: a source answers its lookups, and a whole one in a block is given an
    /// index there only when nothing else answers (`InRel::index`).
    fn own_index(&self, _columns: &[usize]) -> Option<(super::rows::InBlock, usize)> {
        None
    }
    /// Rows are made per looked-up value, so reading them all (a scan, a hash index of every row)
    /// makes every one: a join probes such a source through `lookup`, whatever its match estimate.
    fn made_on_lookup(&self) -> bool {
        false
    }
    /// For a reader that takes the whole relation where a join would look it up (the device's keyed read, which
    /// sorts the rows it is given, decision 0277): whether its rows can be made at once, at the cost of a solve the
    /// source runs, rather than one lookup each. A source that answers so makes them (and its lookups then read
    /// them). Every source not made on lookup has them.
    fn complete_rows(&self) -> bool {
        !self.made_on_lookup()
    }
    /// The planner expects `keys` lookups on `column` (decision 0143): a source that can make every row at once for
    /// less than that many lookups cost chooses to.
    fn expect_lookups(&self, _column: usize, _keys: f64) {}
    /// The planner's statistics (`syntax::statistics`'s rows) where the source knows them without
    /// reading every row: a source whose rows are made on lookup.
    fn statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        None
    }
    /// How many rows hold each value `v` in column `c`, for each `(c, v)` of `wanted`, where the source keeps them
    /// (its rows do not change): `None` when the caller counts them.
    fn value_counts(&self, _wanted: &[(usize, i64)]) -> Option<Vector<i64>> {
        None
    }
    /// Why rows could not be made, where a source makes them on demand and making them failed (a
    /// stored frame that does not decode). The statement reading it fails with this message rather
    /// than reading the rows as absent.
    fn failure(&self) -> Option<String> {
        None
    }
    /// `failure`, as known now: never makes rows to find it (a reader that read some asks after it read them).
    fn failed(&self) -> Option<String> {
        None
    }
    fn indexed(&self, column: usize) -> bool;
    /// Whether an upper bound on `upper` also bounds `lower` (an end bounds its start).
    fn range_upper_compatible(&self, _lower: usize, _upper: usize) -> bool {
        false
    }
    fn range_indexed(&self, _column: usize) -> bool {
        false
    }
    /// The rows whose `column` lies between the bounds (each with its inclusiveness); `None` scans.
    fn range_lookup(&self, _column: usize, _lower: Option<(i64, bool)>, _upper: Option<(i64, bool)>) -> Option<rerail_blocks::vector::Vector<usize>> {
        None
    }
    /// The rows whose `column` is one of `values`; `None` scans.
    fn lookup(&self, column: usize, values: &[i64]) -> Option<rerail_blocks::vector::Vector<usize>>;
    /// `lookup`'s rows, each given to `each` as it is found, in `lookup`'s order, with no list made (decision 0687: a
    /// probe's rows are the caller's to keep, in its own memory); false where it scans (`lookup`'s `None`). The
    /// default reads `lookup`'s list; a source probed often gives its rows as it finds them.
    fn lookup_each(&self, column: usize, values: &[i64], each: &mut dyn FnMut(usize)) -> bool {
        match self.lookup(column, values) {
            Some(rows) => {
                rows.iter().for_each(|&r| each(r));
                true
            }
            None => false,
        }
    }
    /// `lookup`'s rows for `value`, read by position (`Seq`), in its order; `None` where it scans. A source whose
    /// lookups are runs of rows (a view's document) or a list it holds (a kind's nodes) gives them made into no list
    /// of its own: what a probe's candidates number (`setfix::InRel::probe_size`) and are drawn from (`probe_sample`)
    /// is read there. By default, `lookup`'s rows.
    fn lookup_seq(&self, column: usize, value: i64) -> Option<Seq<'_>> {
        self.lookup(column, &[value]).map(Seq::rows)
    }
    /// Whether each value of `column` is at most one row, which `key_row` finds without allocating (a node's own
    /// column): a view of the source's columns held in place probes by it (`setfix::hold_view_in_place`).
    fn keys_rows(&self, _column: usize) -> bool {
        false
    }
    /// Whether `column`'s values are document keys (`node`'s and `core_node`'s `d`, `docof`'s `d`): a key names the
    /// documents its rows are of, as a node id's high bits do.
    fn keys_documents(&self, _column: usize) -> bool {
        false
    }
    /// Whether every row is of the source's own documents: each node id it holds, and each document key, one of
    /// theirs (a view of a batch's tapes), never a host's that a place names. An admitted value's part of such a
    /// relation is found by its own documents' keys alone (`values::Values::add`).
    fn rows_of_own_documents(&self) -> bool {
        false
    }
    /// The row whose `column` (a `keys_rows` column) is `value`, if there is one: `lookup`'s answer, not allocated.
    fn key_row(&self, _column: usize, _value: i64) -> Option<usize> {
        None
    }
    /// The rows whose `column` is `value` and whose columns hold every `(column, value)` of `eqs`, in `lookup`'s
    /// order, where the source tests `eqs` as it finds them (a view's children by parent and field, off the tape)
    /// rather than each row's values read back; `None`: looked up, then filtered.
    fn lookup_eq(&self, _column: usize, _value: i64, _eqs: &[(usize, i64)]) -> Option<rerail_blocks::vector::Vector<usize>> {
        None
    }
    fn value(&self, row: usize, column: usize) -> i64;
    /// `value`, with the caller's memo (`row_at_memo`).
    fn value_memo(&self, row: usize, column: usize, memo: &mut u32) -> i64 {
        let _ = memo;
        self.value(row, column)
    }
    /// The rows whose `column` is `value`, as a slice of the source's own sorted index (no allocation); `None`
    /// where the source keeps none.
    fn sorted_rows(&self, _column: usize, _value: i64) -> Option<&[u32]> {
        None
    }
    /// The rows whose `column` is `value` as a range of row ids, where the column is in row order (a sorted
    /// source's leading column): found by a search of the column itself, no index of its rows built. `None`: the
    /// column is not in order (ask `sorted_rows`). Where it answers, `sorted_rows` gives the same rows.
    fn sorted_range(&self, _column: usize, _value: i64) -> Option<std::ops::Range<u32>> {
        None
    }
    /// `sorted_range`, searched from `at` where it can be: a sequence of probes in the key's order (a scan in the
    /// column's order, the other side of a merge join, decision 0456 K7) reads the column forward once, not a
    /// search of all of it a probe. `at` is set to the range's start; any value is correct, a wrong one is slower.
    fn sorted_range_at(&self, column: usize, value: i64, _at: &mut u32) -> Option<std::ops::Range<u32>> {
        self.sorted_range(column, value)
    }
    /// Whether `sorted_rows` serves `column`, asked without building its index: a plan choosing among a source's
    /// columns built every candidate's sorted form to ask, where it reads one.
    fn sorts_rows(&self, column: usize) -> bool {
        self.sorted_rows(column, 0).is_some()
    }
    /// The columns its rows lie in order on, in turn (a kept relation's canonical order, decision 0456 K7), when
    /// they do: a scan of its rows is in that order, with no sort.
    fn order(&self) -> Option<rerail_blocks::vector::Vector<usize>> {
        None
    }
    /// Whether its rows are a kept relation's, read where they lie in the store's mapped chain (decision 0456 step
    /// 6b): rows that are work, which a run keeps as it keeps a relation it holds, where a view of an input's columns
    /// or an index's relation is made again in less than reading it back costs.
    fn kept(&self) -> bool {
        false
    }
}

#[derive(Default)]
pub struct Counters {
    pub scans: AtomicU64,
    pub batches: AtomicU64,
    pub candidates: AtomicU64,
    pub columns: AtomicU64,
}

/// `source`'s columns as relation `rel`'s table names them: a derived relation named like a base one is read by the
/// base names (`core::input_column_name`: `value`, handed on from the chunks as `c0`, `c1`, is read as `value(n, v)`).
/// Its first binding names the table so; every later one is compared so (`Binding::set`), and a reader of the bound
/// source finds a column so (`setfix::view`).
pub fn named(rel: &str, source: &dyn Source) -> Vec<String> {
    source.columns().into_iter().map(|c| crate::code_rule_dsl::facts::core::input_column_name(rel, &c).map_or(c, str::to_owned)).collect()
}

#[derive(Clone)]
pub struct Binding {
    /// The relation's name: its table's columns are its source's as `named` reads them.
    name: Arc<str>,
    current: Arc<parking_lot::RwLock<Arc<dyn Source>>>,
    schema: Arc<Metadata>,
    stats: Arc<Counters>,
}
impl Binding {
    pub fn clear(&self) {
        *self.current.write() = self.schema.clone();
    }
    pub fn set(&self, source: Arc<dyn Source>) -> Result<()> {
        if self.schema.columns != named(&self.name, source.as_ref()) || self.schema.writable!=source.writable() || self.schema.text.iter().enumerate().any(|(c,t)| *t!=source.text_column(c)) {
            return Err(failure(&format!("native binding schema changed: bound with columns {:?} (writable {}), now {:?} (writable {})", self.schema.columns, self.schema.writable, source.columns(), source.writable())));
        }
        *self.current.write() = source;
        Ok(())
    }
    pub fn counters(&self) -> &Counters {
        &self.stats
    }
    pub fn current(&self) -> Arc<dyn Source> {
        self.current.read().clone()
    }
}
struct Metadata {
    text: Vec<bool>,
    columns: Vec<String>,
    writable: bool,
}
impl Source for Metadata {
    fn text_column(&self,c:usize)->bool { self.text[c] }
    fn writable(&self)->bool {self.writable}
    fn columns(&self) -> Vec<String> {
        self.columns.clone()
    }
    fn len(&self) -> usize {
        0
    }
    fn estimated_matches(&self, _: usize, _: bool) -> usize {
        0
    }
    fn indexed(&self, _: usize) -> bool {
        false
    }
    fn lookup(&self, _: usize, _: &[i64]) -> Option<rerail_blocks::vector::Vector<usize>> {
        None
    }
    fn value(&self, _: usize, _: usize) -> i64 {
        unreachable!("an empty binding has no rows")
    }
}

/// A hash over each source's size and index shape, in buckets: statements prepared under one
/// signature keep their plans while sources of the same shape are rebound.
pub fn plan_signature(sources: &BTreeMap<String, Arc<dyn Source>>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    fn bucket(n: usize) -> u32 {
        usize::BITS - n.leading_zeros()
    }
    for (name, source) in sources {
        name.hash(&mut hash);
        bucket(source.estimated_len()).hash(&mut hash);
        for column in 0..source.columns().len() {
            source.indexed(column).hash(&mut hash);
            source.range_indexed(column).hash(&mut hash);
            if source.indexed(column) {
                bucket(source.estimated_matches(column, false)).hash(&mut hash);
                bucket(source.estimated_matches(column, true)).hash(&mut hash);
            }
        }
    }
    hash.finish()
}

/// The tables registered on one connection, by name. The connection's owner holds it.
#[derive(Default)]
pub struct Modules {
    bindings: RefCell<HashMap<String, Binding>>,
    /// The connections it bound tables on (weakly: a closed one's are gone with it).
    on: RefCell<Vec<std::sync::Weak<ConnData>>>,
}

/// What this module keeps for one connection (decision 0442: on the connection, `ConnData`), dropped as it closes.
#[derive(Default)]
struct OnVtab {
    /// The tables bound now, by name.
    bound: parking_lot::Mutex<HashMap<String, Binding>>,
    /// The tables its last run bound, emptied (`reuse`): their modules stay registered until it closes, and the next
    /// run's `bind` of a table of the same shape takes one back.
    registered: parking_lot::Mutex<HashMap<String, Binding>>,
    /// The views noted on it (`note_view`).
    views: parking_lot::Mutex<HashMap<String, UnmadeView>>,
    /// The tables registered on it that no statement has resolved yet (`register`).
    unresolved: parking_lot::Mutex<std::collections::BTreeSet<String>>,
    /// Its deferred set-up (`defer_setup`).
    setup: parking_lot::Mutex<Setup>,
}

/// What this module keeps for `db` (`OnVtab`).
fn on(db: &Connection) -> Arc<OnVtab> {
    connection_data(db).get::<OnVtab>()
}
impl Modules {
    /// Binds `source` to the table `name`, registering it on `db` the first time.
    pub fn bind(&self, db: &Connection, name: &str, source: Arc<dyn Source>) -> Result<()> {
        self.bind_inner(db,name,source,false)
    }
    /// SQL can restore a dropped virtual-table schema before native rollback runs.
    pub(crate) fn restore(&self, db: &Connection, name: &str, source: Arc<dyn Source>) -> Result<()> {
        self.bind_inner(db,name,source,true)
    }
    /// Keep the same binding cell when SQL rollback revives an earlier table instance.
    pub fn restore_binding(&self,db:&Connection,name:&str,binding:Binding)->Result<()> {
        register(db,name,binding.clone(),true)?;
        self.bound_on(db).bound.lock().insert(name.to_owned(),binding.clone());
        self.bindings.borrow_mut().insert(name.into(),binding);
        Ok(())
    }
    fn bind_inner(&self, db:&Connection, name:&str, source:Arc<dyn Source>, restored:bool)->Result<()> {
        let mut bindings = self.bindings.borrow_mut();
        if let Some(binding) = bindings.get(name) {
            return binding.set(source);
        }
        // A kept connection's table of this shape from its last run (`reuse`): its module is registered still.
        let on = self.bound_on(db);
        let columns = named(name, source.as_ref());
        let shape = (columns.clone(), (0..source.columns().len()).map(|c| source.text_column(c)).collect::<Vec<_>>(), source.writable());
        let kept = on.registered.lock().remove(name);
        // (A writable table's `temp` virtual table is there still: the run's clean-up drops none it binds, and one it
        // drops is no longer kept, `dropped_temp`.)
        if let Some(binding) = kept.filter(|b| (b.schema.columns.clone(), b.schema.text.clone(), b.schema.writable) == shape) {
            binding.set(source)?;
            on.bound.lock().insert(name.to_owned(), binding.clone());
            bindings.insert(name.into(), binding);
            return Ok(());
        }
        let binding = Binding {
            name: name.into(),
            schema: Arc::new(Metadata { text:(0..source.columns().len()).map(|c|source.text_column(c)).collect(), columns, writable:source.writable() }),
            current: Arc::new(parking_lot::RwLock::new(source)),
            stats: Arc::new(Counters::default()),
        };
        register(db, name, binding.clone(), restored)?;
        if name == "node" { for register in node_tables() { register(db, &binding)?; } demand::register(db)?; }
        on.bound.lock().insert(name.to_owned(), binding.clone());
        bindings.insert(name.into(), binding);
        Ok(())
    }
    /// `db`'s `OnVtab`, with `db` noted as a connection this binds on.
    fn bound_on(&self, db: &Connection) -> Arc<OnVtab> {
        let data = connection_data(db);
        let mut on = self.on.borrow_mut();
        if !on.iter().any(|w| std::ptr::eq(w.as_ptr(), Arc::as_ptr(&data))) {
            on.retain(|w| w.strong_count() > 0);
            on.push(Arc::downgrade(&data));
        }
        data.get::<OnVtab>()
    }
    pub fn get(&self, name: &str) -> Option<Binding> {
        self.bindings.borrow().get(name).cloned()
    }
    /// Releases the table without a schema write or a retained row source.
    pub fn unregister(&self, db: &Connection, name: &str) -> Result<()> {
        if self.bindings.borrow().get(name).is_some_and(|b|b.schema.writable)
            && db.query_row("SELECT EXISTS(SELECT 1 FROM temp.sqlite_schema WHERE name=?1)",[name],|r|r.get::<_,bool>(0))? {
            db.execute_batch(&format!("DROP TABLE IF EXISTS temp.{}",quote(name)))?;
        }
        self.bindings.borrow_mut().remove(name);
        on(db).bound.lock().remove(name);
        let name = CString::new(name).map_err(|_| failure("invalid native module name"))?;
        // SAFETY: the live connection owns its module. SQLite releases its auxiliary
        // data and keeps any already-open cursor's Arc ownership intact.
        let code = unsafe {
            rusqlite::ffi::sqlite3_create_module_v2(db.handle(), name.as_ptr(), std::ptr::null(), std::ptr::null_mut(), None)
        };
        if code != rusqlite::ffi::SQLITE_OK {
            return Err(rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), None));
        }
        Ok(())
    }
}
impl Drop for Modules {
    fn drop(&mut self) {
        let mine=self.bindings.get_mut();
        if mine.is_empty(){return;}
        // Their modules stay registered on the connection, which may still read them: a later run on it takes one
        // of the same shape back (`REGISTERED`; `reuse` empties them first).
        for data in self.on.get_mut().iter().filter_map(std::sync::Weak::upgrade) {
            let Some(on) = data.get_if_any::<OnVtab>() else { continue };
            let mut registered = on.registered.lock();
            on.bound.lock().retain(|name,binding| {
                let owned = mine.get(name).is_some_and(|owned|Arc::ptr_eq(&owned.current,&binding.current));
                if owned {
                    registered.insert(name.clone(), binding.clone());
                }
                !owned
            });
        }
    }
}

/// A run's connection, or only its scope (`connection_scope`) where the work runs off the connection's thread: a
/// stratum run as a job on the pool (decision 0113). A connection is not `Sync`, but what the engine keeps per
/// connection (bound sources, native relations, components, symbols) is keyed by its scope alone, so a job reads and
/// writes it from any thread. Only SQL needs the connection: [`Db::sql`] is an error without one.
#[derive(Clone, Copy)]
pub struct Db<'a> {
    at: usize,
    conn: Option<&'a Connection>,
    /// What the engine keeps for the connection (`connection_data`): alive while the connection is borrowed, or, for
    /// a detached one, while its holder keeps it (`detached`).
    data: DataRef,
}

/// A pointer to a connection's `ConnData`, the `Arc`'s.
#[derive(Clone, Copy)]
struct DataRef(std::ptr::NonNull<ConnData>);
// SAFETY: `ConnData` is `Send + Sync`; the pointer is read only while the data lives (`Db`).
unsafe impl Send for DataRef {}
unsafe impl Sync for DataRef {}

impl<'a> Db<'a> {
    pub fn of(conn: &'a Connection) -> Self {
        // SAFETY: the client data of the open `conn` (`client`), alive while `conn` is borrowed.
        let client = unsafe { &*client(conn) };
        Db { at: client.0, conn: Some(conn), data: DataRef(std::ptr::NonNull::from(&*client.1)) }
    }
    /// The scope alone, for a job off the connection's thread, and the connection's data, which the job holds while
    /// it runs.
    ///
    /// # Safety
    /// The returned `Db` is used only while the returned `Arc` lives.
    pub unsafe fn detached(self) -> (Db<'static>, Arc<ConnData>) {
        // SAFETY: `data` points into an `Arc` alive while `self` is (the connection's client data).
        let keep = unsafe {
            Arc::increment_strong_count(self.data.0.as_ptr());
            Arc::from_raw(self.data.0.as_ptr())
        };
        (Db { at: self.at, conn: None, data: self.data }, keep)
    }
    pub fn at(&self) -> usize {
        self.at
    }
    /// The connection's data, held weakly (a plan step's, made before its run binds what it reads).
    pub fn data_weak(&self) -> std::sync::Weak<ConnData> {
        // SAFETY: `data` points into an `Arc` alive while `self` may be used.
        let held = unsafe {
            Arc::increment_strong_count(self.data.0.as_ptr());
            Arc::from_raw(self.data.0.as_ptr())
        };
        Arc::downgrade(&held)
    }
    /// What the engine keeps for the connection (decision 0442).
    pub fn data(&self) -> &ConnData {
        // SAFETY: alive while `self` may be used (`Db::of`, `Db::detached`).
        unsafe { self.data.0.as_ref() }
    }
    pub fn sql(&self) -> rusqlite::Result<&'a Connection> {
        self.conn.ok_or_else(|| rusqlite::Error::ModuleError("SQL read by a stratum off the run's thread (decision 0113): it has only its connection's scope".into()))
    }
}

/// A run's SQL, where the caller is on the connection's thread: a detached scope here is a stratum the issuer should
/// not have sent off its thread (`plan::strand`), a bug, so it panics with what it is.
impl std::ops::Deref for Db<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.conn.expect("SQL read by a stratum off the run's thread (decision 0113)")
    }
}

/// What a function of the engine's per-connection state takes: a connection or a [`Db`].
pub trait Scoped {
    fn db(&self) -> Db<'_>;
    fn at(&self) -> usize {
        self.db().at()
    }
}
impl Scoped for Connection {
    fn db(&self) -> Db<'_> {
        Db::of(self)
    }
}
impl Scoped for Db<'_> {
    fn db(&self) -> Db<'_> {
        *self
    }
}
impl Scoped for rusqlite::functions::ConnectionRef<'_> {
    fn db(&self) -> Db<'_> {
        Db::of(self)
    }
}
impl<T: Scoped + ?Sized> Scoped for &T {
    fn db(&self) -> Db<'_> {
        (**self).db()
    }
}

/// The tables bound on each live connection (by its handle), for readers on that connection that read a source in
/// place rather than through SQL: the native engine's base inputs (`logic::native`).
/// The scope of what this process keeps per connection (its handle, an address), with a hook that drops all of it
/// when the connection closes: the vtab bindings here and the native engine's relations
/// (`logic::native::forget_scope`). A closed connection's address is reused by the next one opened.
/// What registers a family's tables over the bound `node` relation (Python's semantic producers, `vtab::python`).
pub(crate) type NodeTables = fn(&Connection, &Binding) -> Result<()>;

static NODE_TABLES: std::sync::OnceLock<Vec<NodeTables>> = std::sync::OnceLock::new();

/// Registers the families' tables over `node` (once, at load).
pub fn install_node_tables(tables: Vec<NodeTables>) {
    let _ = NODE_TABLES.set(tables);
}

fn node_tables() -> &'static [NodeTables] {
    NODE_TABLES.get().map_or(&[], |v| v.as_slice())
}

/// What the engine keeps for one connection (decision 0442): its own values, by type, each made as it is first asked
/// for (`get`). It lives in the connection's client data and is dropped as the connection closes, after `Forget::scope`
/// has taken what it frees elsewhere. A job off the connection's thread holds it through its [`Db`].
#[derive(Default)]
pub struct ConnData {
    values: parking_lot::Mutex<HashMap<std::any::TypeId, Arc<dyn std::any::Any + Send + Sync>>>,
}

impl ConnData {
    /// The connection's value of type `T`, made the first time it is asked for.
    pub fn get<T: Default + Send + Sync + 'static>(&self) -> Arc<T> {
        let v = Arc::clone(self.values.lock().entry(std::any::TypeId::of::<T>()).or_insert_with(|| Arc::new(T::default())));
        v.downcast::<T>().expect("a connection's value's type is its key")
    }
    /// The connection's value of type `T`, if it was made.
    pub fn get_if_any<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        let v = self.values.lock().get(&std::any::TypeId::of::<T>()).cloned()?;
        v.downcast::<T>().ok()
    }
}

/// The connection's client data: its scope (its handle when first seen) and its values.
type Client = (usize, Arc<ConnData>);

const KEY: &std::ffi::CStr = c"rerail.connection-scope";

/// The client data of `db`, set the first time it is asked for: the connection's scope, and its values.
fn client(db: &Connection) -> *const Client {
    // Each close bumps the generation: a thread's last connection with its client data set (`LAST`) is known to have
    // it until some connection closes (whose address the next one may take), without asking SQLite on every lookup.
    static CLOSES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    thread_local!(static LAST: std::cell::Cell<(usize, u64, usize)> = const { std::cell::Cell::new((0, u64::MAX, 0)) });
    unsafe extern "C" fn closed(client: *mut std::os::raw::c_void) {
        CLOSES.fetch_add(1, Ordering::AcqRel);
        // SAFETY: the pointer `client` set, given back once as the connection closes.
        let client = unsafe { Box::from_raw(client.cast::<Client>()) };
        let at = client.0;
        if let Some(forget) = FORGET.get() {
            (forget.scope)(at, &client.1);
        }
        drop(client);
    }
    let handle = unsafe { db.handle() };
    let closes = CLOSES.load(Ordering::Acquire);
    let last = LAST.get();
    if (last.0, last.1) == (handle as usize, closes) {
        return last.2 as *const Client;
    }
    // SAFETY: `handle` is `db`'s, open; the client data set here is a `Client` box, freed by `closed` alone.
    let client = unsafe {
        let mut p = rusqlite::ffi::sqlite3_get_clientdata(handle, KEY.as_ptr());
        if p.is_null() {
            p = Box::into_raw(Box::new((handle as usize, Arc::new(ConnData::default())) as Client)).cast();
            rusqlite::ffi::sqlite3_set_clientdata(handle, KEY.as_ptr(), p, Some(closed));
        }
        p as *const Client
    };
    LAST.set((handle as usize, closes, client as usize));
    client
}

/// The scope of what the engine keeps for `db`: its handle when first seen, reused by a connection opened after it
/// closes, whose own values (`connection_data`) start empty.
pub fn connection_scope(db: &Connection) -> usize {
    // SAFETY: the client data of the open `db` (`client`), freed only as it closes.
    unsafe { (*client(db)).0 }
}

/// What the engine keeps for `db` (decision 0442): its own values, dropped as it closes.
pub fn connection_data(db: &Connection) -> Arc<ConnData> {
    // SAFETY: as `connection_scope`.
    unsafe { Arc::clone(&(*client(db)).1) }
}

/// A view of one bound input that a run read in place and did not make in SQL (`plan`): the input, the input's columns
/// it keeps (in the view's order), and its (column, value) filters. A reader after the run reads it from the input.
#[derive(Clone, Debug)]
pub struct UnmadeView {
    pub base: String,
    pub cols: Vector<usize>,
    pub eqs: Vec<(usize, i64)>,
}


/// Whether `name` is a table of `db`'s connection's own, bound now or kept for a later bind (`REGISTERED`): its
/// `temp` virtual table (a writable one's) is the binding's, not what an earlier run left. (A run binds some of
/// its tables as it goes: the kept strata's `load` hook.)
pub fn is_bound(db: &Connection, name: &str) -> bool {
    let on = on(db);
    on.bound.lock().contains_key(name) || on.registered.lock().contains_key(name)
}

/// A run dropped `temp.<name>` on `db`'s connection: a kept binding of that name is gone with it.
pub fn dropped_temp(db: &Connection, name: &str) {
    on(db).registered.lock().remove(name);
}

/// A connection's deferred set-up (`defer_setup`): the statements still to run, each with what undoes it, and what
/// undoes those already run (`reuse`).
#[derive(Default)]
struct Setup {
    pending: Vec<(String, Option<String>)>,
    undo: Vec<String>,
}

/// Defers `sql` (statements, run as one batch) until SQL first runs on `db`'s connection (`setup_now`): a run whose
/// readers are native runs none of it.
pub fn defer_setup(db: &Connection, sql: &str) {
    on(db).setup.lock().pending.push((sql.to_owned(), None));
}

/// `defer_setup` of a run's own set-up on a connection a later run reuses: once run, `undo` undoes it when the
/// connection is reused (`reuse`; `DETACH x` for an `ATTACH`).
pub fn defer_setup_undone(db: &Connection, sql: &str, undo: &str) {
    on(db).setup.lock().pending.push((sql.to_owned(), Some(undo.to_owned())));
}

/// Runs `db`'s deferred set-up (`defer_setup`), once: before a statement that needs it (a table made, a table of
/// `x` written).
pub fn setup_now(db: &Connection) -> Result<()> {
    let on = on(db);
    let pending = std::mem::take(&mut on.setup.lock().pending);
    for (sql, undo) in pending {
        db.execute_batch(&sql)?;
        if let Some(undo) = undo {
            on.setup.lock().undo.push(undo);
        }
    }
    Ok(())
}

/// Readies a connection an earlier run worked on for the next run (its owner keeps it, `corpus::Connections`): what
/// the earlier run set up of its own is undone (SQL only where its set-up ran), and what the process kept for that
/// run on it (its views, the engine's relations) is dropped; its bound tables are emptied, kept for the next run's
/// `bind` (`REGISTERED`). What the connection itself holds stays: its functions and modules, its symbols, which of
/// its tables SQL has yet to connect (`UNRESOLVED`), what its runs left in SQL (`plan::Left`, which the next run drops).
pub fn reuse(db: &Connection) -> Result<()> {
    let on = on(db);
    let undo = std::mem::take(&mut *on.setup.lock()).undo;
    for sql in undo.iter().rev() {
        db.execute_batch(sql)?;
    }
    // SQLite keeps each module's binding until the connection closes: the last run's sources are let go, its tables
    // left empty (a `Modules` may drop while its connection still reads, so this is the connection's next run's).
    // The next run's `bind` takes a table of the same shape back (`REGISTERED`): its module stays registered.
    let mut registered = on.registered.lock();
    for (name, binding) in std::mem::take(&mut *on.bound.lock()) {
        registered.insert(name, binding);
    }
    for binding in registered.values() {
        binding.clear();
    }
    drop(registered);
    on.views.lock().clear();
    if let Some(forget) = FORGET.get() {
        (forget.engine)(&connection_data(db));
    }
    Ok(())
}

/// What the evaluator keeps per connection, dropped with it: `scope` when the connection closes (its symbols,
/// paths and plans with its relations), `engine` when a run reuses it (its relations and components alone).
pub struct Forget {
    pub scope: fn(usize, &ConnData),
    pub engine: fn(&ConnData),
}

static FORGET: std::sync::OnceLock<Forget> = std::sync::OnceLock::new();

/// Registers what the evaluator drops with a connection (once, at load).
pub fn install_forget(forget: Forget) {
    let _ = FORGET.set(forget);
}

/// Resolves every table registered on `db` and not yet resolved, by preparing a read of it: an authorizer installed
/// after this does not see SQLite connect them (an authored query's authorizer refuses what it does not name).
pub fn resolve_tables(db: &Connection) -> Result<()> {
    let on = on(db);
    let names = std::mem::take(&mut *on.unresolved.lock());
    for name in names {
        if on.bound.lock().contains_key(&name) {
            db.prepare(&format!("SELECT * FROM {}", quote(&name)))?;
        }
    }
    Ok(())
}


/// Records `view` as the view `name` on `db`'s connection, until the connection closes or `forget_views`.
pub(crate) fn note_view(db: &Connection, name: &str, view: UnmadeView) {
    on(db).views.lock().insert(name.to_owned(), view);
}

/// Drops the views noted on `db`'s connection: a run starts with none.
pub fn forget_views(db: &Connection) {
    on(db).views.lock().clear();
}

/// The rows of the noted view `name` (`note_view`), read from its bound input: each once, as SQL's view gives them
/// (a set), column by column.
pub fn view_columns(db: &Connection, name: &str) -> Option<Vec<Vector<i64>>> {
    let view = on(db).views.lock().get(name).cloned()?;
    let src = bound_source(db, &view.base)?;
    let mut rows: Vec<Vector<i64>> = src.filter_eq(&view.eqs).into_iter().map(|id| view.cols.iter().map(|&c| src.value(id as usize, c)).collect()).collect();
    rows.sort_unstable();
    rows.dedup();
    let mut columns: Vec<Vector<i64>> = (0..view.cols.len()).map(|_| Vector::with_capacity(rows.len())).collect();
    for row in rows {
        for (c, v) in row.into_iter().enumerate() {
            columns[c].push(v);
        }
    }
    Some(columns)
}

/// The source bound now to the table `name` on `db`'s connection, if one is.
pub fn bound_source(db: impl Scoped, name: &str) -> Option<Arc<dyn Source>> {
    bound_source_in(db.db().data(), name)
}

/// `bound_source` by the connection's data (`connection_data`), for a callback that holds it, not the connection.
pub fn bound_source_in(data: &ConnData, name: &str) -> Option<Arc<dyn Source>> {
    data.get_if_any::<OnVtab>()?.bound.lock().get(name).map(Binding::current)
}

/// Rebind through the existing cell so its owning Modules and prepared cursors agree.
pub fn replace_bound_source(db:&Connection,name:&str,source:Arc<dyn Source>)->Result<bool>{
    let binding=on(db).bound.lock().get(name).cloned();
    if let Some(binding)=binding{binding.set(source)?;Ok(true)}else{Ok(false)}
}

/// SQL cursor work, by bound input. Snapshot differences attribute fused query work to its leaves.
pub fn counters(db:&Connection)->BTreeMap<String,[u64;4]> {
    on(db).bound.lock().iter().map(|(name,b)|(name.clone(),[b.stats.scans.load(Ordering::Relaxed),b.stats.batches.load(Ordering::Relaxed),b.stats.candidates.load(Ordering::Relaxed),b.stats.columns.load(Ordering::Relaxed)])).collect()
}

#[repr(C)]
struct Table {
    base: rusqlite::ffi::sqlite3_vtab,
    binding: Binding,
    connection: *mut rusqlite::ffi::sqlite3,
}
fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
fn register(db: &Connection, name: &str, binding: Binding, restored:bool) -> Result<()> {
    if name.is_empty() || name.len() > 120 || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err(failure("invalid native table name"));
    }
    let module = CString::new(name).map_err(|_| failure("invalid module name"))?;
    const MODULE: Module<Table> = Module::eponymous_only_module();
    const WRITABLE: Module<Table> = Module::update_module();
    let writable=binding.current().writable();
    db.create_module(module.as_c_str(), if writable { &WRITABLE } else { &MODULE }, Some(binding))?;
    // Mutable execution state is temporary. An eponymous table in main would make
    // SQLite classify volatile writes as durable store writes (including its atom guard).
    let result=(|| {
    if writable {
        let exists=restored&&db.query_row("SELECT EXISTS(SELECT 1 FROM temp.sqlite_schema WHERE name=?1)",[name],|r|r.get::<_,bool>(0))?;
        if !exists {db.execute_batch(&format!("CREATE VIRTUAL TABLE temp.{} USING {}",quote(name),quote(name)))?;}
    }
    // xConnect is resolved before an authored-query authorizer is installed (`resolve_tables`), not here: a run whose
    // readers are native prepares no statement over its tables.
    on(db).unresolved.lock().insert(name.to_owned());
    Ok(())
    })();
    if result.is_err() {
        // Registration precedes schema preparation. A budget interruption can fail the
        // latter before the owner records this module, so release its auxiliary source here.
        unsafe {rusqlite::ffi::sqlite3_create_module_v2(db.handle(),module.as_ptr(),std::ptr::null(),std::ptr::null_mut(),None);}
    }
    result
}

/// A constant's integer under the column's INTEGER affinity, if it has one: only such constants can
/// equal an integer cell.
fn integer(value: &Value) -> Option<i64> {
    let real = |r: f64| (r.fract() == 0.0 && r >= i64::MIN as f64 && r < i64::MAX as f64).then_some(r as i64);
    match value {
        Value::Integer(i) => Some(*i),
        Value::Real(r) => real(*r),
        Value::Text(t) => t.trim().parse().ok().or_else(|| t.trim().parse().ok().and_then(real)),
        _ => None,
    }
}
/// A range bound widened to an inclusive integer one; `None` when only a scan is safe.
fn bound(value: &Value, inclusive: bool, lower: bool) -> Option<(i64, bool)> {
    if let Some(i) = integer(value) {
        return Some((i, inclusive));
    }
    match value {
        Value::Real(r) if r.is_finite() => {
            let r = if lower { r.floor() } else { r.ceil() };
            (r >= i64::MIN as f64 && r < i64::MAX as f64).then_some((r as i64, true))
        }
        _ => None,
    }
}

// SAFETY: SQLite's base is first; auxiliary data and cursors own Arc references.
unsafe impl<'a> VTab<'a> for Table {
    type Aux = Binding;
    type Cursor = Cursor;
    fn connect(
        db: &mut VTabConnection,
        aux: Option<&Binding>,
        _: &[u8],
        _: &[u8],
        _: &[u8],
        _: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::Innocuous)?;
        let binding = aux.ok_or_else(|| failure("missing native row source"))?.clone();
        if binding.current().writable() {
            // This option takes a third vararg. VTabConnection::config in the pinned
            // rusqlite version supplies only the option code, so pass the flag explicitly.
            let code=unsafe {rusqlite::ffi::sqlite3_vtab_config(db.handle(),rusqlite::ffi::SQLITE_VTAB_CONSTRAINT_SUPPORT,1)};
            if code!=rusqlite::ffi::SQLITE_OK {return Err(rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code),None));}
        }
        let columns: Vec<String> = binding.schema.columns.iter().enumerate().map(|(i,c)| format!("{} {}", quote(c), if binding.schema.text[i] {"TEXT"} else {"INTEGER"})).collect();
        let sql = CString::new(format!("CREATE TABLE x({})", columns.join(","))).map_err(|_| failure("invalid native columns"))?;
        Ok((Cow::Owned(sql), Self { base: rusqlite::ffi::sqlite3_vtab::default(), binding, connection:unsafe {db.handle()} }))
    }
    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let source = self.binding.current();
        let equal=info.constraints().enumerate().filter(|(i,c)|c.is_usable()&&c.column()>=0&&!source.text_column(c.column() as usize)&&c.operator()==IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ&&info.collation(*i).is_ok_and(|v|v=="BINARY")).map(|(i,c)|(i,c.column() as usize)).collect::<Vec<_>>();
        let order=info.order_bys().map(|o|(o.column() as usize,o.is_order_by_desc())).collect::<Vec<_>>();
        if !order.is_empty()&&equal.iter().all(|(i,_)|info.is_in_constraint(*i).is_ok_and(|b|!b))&&source.ordered(&equal.iter().map(|(_,c)|*c).collect::<Vec<_>>(),&order) {
            for (arg,&(i,_)) in equal.iter().enumerate(){info.constraint_usage(i).set_argv_index((arg+1) as i32);}
            info.set_idx_num(ORDERED);
            info.set_idx_str(&format!("{}:{}",if order[0].1{"d"}else{"a"},equal.iter().map(|(_,c)|c.to_string()).collect::<Vec<_>>().join(",")));
            info.set_order_by_consumed(true);
            let rows=source.estimated_matches_all(&equal.iter().map(|(_,c)|*c).collect::<Vec<_>>());
            info.set_estimated_rows(rows as i64);info.set_estimated_cost(rows.max(1) as f64);
            return Ok(true);
        }
        // Batch every primary-key field together. SQLite rechecks tuple-IN correlation.
        if let Some(keys)=source.primary_key_len().filter(|&n|n>=2) {
            let chosen=(0..keys).map(|c|equal.iter().filter(|(_,col)|*col==c).min_by_key(|(i,_)|info.is_in_constraint(*i).unwrap_or(false)).copied()).collect::<Option<Vec<_>>>();
            if let Some(chosen)=chosen && chosen.iter().any(|(i,_)|info.is_in_constraint(*i).unwrap_or(false)) {
                let mut tags=String::new();
                for (arg,(i,_)) in chosen.into_iter().enumerate() {
                    let batch=info.is_in_constraint(i)?;
                    if batch {info.set_in_constraint(i,true)?;}
                    tags.push(if batch {'b'} else {'s'});
                    info.constraint_usage(i).set_argv_index(arg as c_int+1);
                }
                info.set_idx_num(PRIMARY_BATCH);info.set_idx_str(&tags);
                info.set_estimated_rows(8);info.set_estimated_cost(8.0);
                return Ok(true);
            }
        }
        // The document bound by an `EQ`, where the source can keep a lookup to it (`lookup_in`).
        let doc = source.doc_column().filter(|&d| {
            info.constraints().enumerate().any(|(j, c)| {
                c.is_usable() && c.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ && c.column() == d as i32 && info.collation(j).is_ok_and(|v| v == "BINARY")
            })
        });
        // A lookup kept to one document reads that document's share of its matches.
        let in_doc = |col: usize, matches: usize| match doc {
            Some(d) if source.kept_to_doc(col) => (matches * source.estimated_matches(d, false) / source.estimated_len().max(1)).max(1),
            _ => matches,
        };
        // Every usable `=` on a column, one value each; an `IN` list only as the probed column.
        let mut equal: Vec<(usize, usize)> = Vec::new();
        let mut probe: Option<(usize, usize, bool, usize)> = None;
        for (i, c) in info.constraints().enumerate() {
            if !c.is_usable()
                || c.operator() != IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ
                || c.column() < 0
                || source.text_column(c.column() as usize)
                || !info.collation(i).is_ok_and(|v| v == "BINARY")
            {
                continue;
            }
            let col = c.column() as usize;
            let batch = info.is_in_constraint(i)?;
            if !batch && !equal.iter().any(|&(_, e)| e == col) {
                equal.push((i, col));
            }
            if source.indexed(col) {
                let matches = in_doc(col, source.estimated_matches(col, batch));
                if probe.is_none_or(|p| matches < p.3) {
                    probe = Some((i, col, batch, matches));
                }
            }
        }
        let columns: Vector<usize> = equal.iter().map(|&(_, c)| c).collect();
        let all = (columns.len() >= 2).then(|| source.estimated_matches_all(&columns));
        let selected = probe.map(|(i, col, _, matches)| (i, col as c_int, matches));
        let range = info.constraints().enumerate().find_map(|(i, c)| {
            let col = c.column();
            if !c.is_usable() || col < 0 || !source.range_indexed(col as usize) || !info.collation(i).is_ok_and(|v| v == "BINARY") {
                return None;
            }
            if !matches!(
                c.operator(),
                IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_GT
                    | IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_GE
                    | IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_LT
                    | IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_LE
            ) {
                return None;
            }
            Some(col)
        });
        if let Some(col) = range
            && all.or(selected.map(|s| s.2)).is_none_or(|cost| cost > 32)
        {
            let mut lower = None;
            let mut upper = None;
            for (i, c) in info.constraints().enumerate() {
                let upper_equivalent = c.column() >= 0
                    && source.range_upper_compatible(col as usize, c.column() as usize)
                    && matches!(
                        c.operator(),
                        IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_LT | IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_LE
                    );
                if !c.is_usable() || (c.column() != col && !upper_equivalent) || !info.collation(i).is_ok_and(|v| v == "BINARY") {
                    continue;
                }
                // An end bound also bounds the start. SQLite rechecks the original
                // end predicate; this only narrows the candidate start range.
                match c.operator() {
                    IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_GE => lower = Some((i, true)),
                    IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_GT => lower = Some((i, false)),
                    IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_LE => upper = Some((i, true)),
                    IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_LT => upper = Some((i, false)),
                    _ => {}
                }
            }
            let mut flags = 1 << 20 | ((col + 1) << 4);
            let mut arg = 1;
            if let Some((i, inclusive)) = lower {
                info.constraint_usage(i).set_argv_index(arg);
                arg += 1;
                flags |= 1;
                if inclusive {
                    flags |= 4;
                }
            }
            if let Some((i, inclusive)) = upper {
                info.constraint_usage(i).set_argv_index(arg);
                flags |= 2;
                if inclusive {
                    flags |= 8;
                }
            }
            info.set_idx_num(flags);
            info.set_estimated_rows(32);
            info.set_estimated_cost(32.0);
            return Ok(true);
        }
        if let Some(rows) = all {
            // Several `=`: one probe of a sorted order over them all (`composite`), else a probe of the most
            // selective column (or a scan) with the rest filtered here, before SQLite sees a row. `idx_str` is
            // a tag, `c` (composite), `p` / `b` (probe, of an `IN` list) or `s` (scan), then the columns in
            // argument order, the probed one first.
            let tag = match probe {
                _ if source.composite() && probe.is_none_or(|p| !p.2 || rows <= p.3) => 'c',
                Some((_, _, true, _)) => 'b',
                Some(_) => 'p',
                None => 's',
            };
            let mut order: Vec<(usize, usize)> = Vec::new();
            if let (Some((i, col, batch, _)), 'p' | 'b') = (probe, tag) {
                if batch {
                    info.set_in_constraint(i, true)?;
                }
                order.push((i, col));
            }
            let first = order.first().copied();
            order.extend(equal.iter().copied().filter(|e| Some(*e) != first));
            for (arg, &(i, _)) in order.iter().enumerate() {
                info.constraint_usage(i).set_argv_index(arg as c_int + 1);
            }
            let list: Vec<String> = order.iter().map(|&(_, c)| c.to_string()).collect();
            info.set_idx_num(MULTI);
            info.set_idx_str(&format!("{tag}{}", list.join(",")));
            // The cost is the rows read: the order's run, the probed column's run, or every row.
            let read = match (tag, probe) {
                ('s', _) => source.estimated_len(),
                ('p' | 'b', Some(p)) => p.3,
                _ => rows,
            };
            info.set_estimated_rows(rows.max(1) as i64);
            info.set_estimated_cost((read.max(1) + rows) as f64);
        } else if let Some((i, col, matches)) = selected {
            let batch = info.is_in_constraint(i)?;
            if batch {
                info.set_in_constraint(i, true)?;
            }
            info.constraint_usage(i).set_argv_index(1);
            // No affinity or collation assumptions: SQLite rechecks every filter.
            info.set_idx_num((col + 1) * 2 + i32::from(batch));
            info.set_estimated_rows(matches as i64);
            info.set_estimated_cost(matches.max(1) as f64);
        } else {
            info.set_idx_num(0);
            info.set_estimated_rows(source.estimated_len() as i64);
            info.set_estimated_cost(source.estimated_len().max(1) as f64);
        }
        Ok(true)
    }
    fn open(&'a mut self) -> Result<Cursor> {
        Ok(Cursor {
            base: rusqlite::ffi::sqlite3_vtab_cursor::default(),
            binding: self.binding.clone(),
            source: self.binding.current(),
            range: 0..0,
            selected: None,
            position: 0,
            reverse:false,
            lazy:false,
            cache_rows:false,
            cached:RefCell::new(None),
            direct:None,
            stream:None,
            stream_row:None,
        })
    }
}
#[repr(C)]
struct Cursor {
    base: rusqlite::ffi::sqlite3_vtab_cursor,
    binding: Binding,
    source: Arc<dyn Source>,
    range: Range<usize>,
    selected: Option<rerail_blocks::vector::Vector<usize>>,
    position: usize,
    reverse: bool,
    lazy: bool,
    cache_rows:bool,
    cached:RefCell<Option<(usize,i64,Arc<dyn RowCells>)>>,
    direct:Option<Vec<ReadRow>>,
    stream:Option<Box<dyn RowStream>>,
    stream_row:Option<ReadRow>,
}
impl Cursor {
    fn start_stream(&mut self) {
        if !self.cache_rows||self.direct.is_some() {return;}
        if let Some(rows)=&self.selected
            && rows.len()>128&&rows.windows(2).all(|w|w[1]==w[0]+1) {
                self.range=rows[0]..rows[rows.len()-1]+1;self.selected=None;
            }
        if self.selected.is_none()&&self.range.len()>128 {
            self.stream=self.source.row_stream(self.range.clone(),self.reverse);
            if let Some(stream)=&mut self.stream {self.stream_row=stream.next();}
        }
    }
    fn with_cells<T>(&self,f:impl FnOnce(i64,&dyn RowCells)->T)->T {
        let row=self.row();
        let mut cache=self.cached.borrow_mut();
        if cache.as_ref().is_none_or(|(at,_,_)|*at!=row) {
            let(id,cells)=self.source.row_cells(row).expect("source promised cached rows");
            *cache=Some((row,id,cells));
        }
        let(_,id,cells)=cache.as_ref().unwrap();f(*id,cells.as_ref())
    }
    fn row(&self) -> usize {
        self.selected.as_ref().map_or(if self.reverse {self.range.end.saturating_sub(1)}else{self.range.start}, |r| r[self.position])
    }
}
impl<'vtab> rusqlite::vtab::CreateVTab<'vtab> for Table {
    const KIND:rusqlite::vtab::VTabKind=rusqlite::vtab::VTabKind::Default;
}
impl<'vtab> rusqlite::vtab::UpdateVTab<'vtab> for Table {
    fn delete(&mut self,arg:rusqlite::types::ValueRef<'_>)->Result<()> {
        self.binding.current().write(Some(arg.as_i64()?),None,None,false,false).map(|_|())
    }
    fn insert(&mut self,args:&rusqlite::vtab::Inserts<'_>)->Result<i64> {
        crate::code_rule_dsl::sql_frame::frame(crate::code_rule_dsl::sql_frame::FrameEvent::Insert,1);
        use rusqlite::vtab::ConflictMode;
        let conflict=unsafe {args.on_conflict(self.connection)};
        let values=(2..args.len()).map(|i|args.get::<Option<i64>>(i)).collect::<Result<Vec<_>>>()?;
        self.binding.current().write(None,args.get(1)?,Some(values),conflict==ConflictMode::Replace,conflict==ConflictMode::Ignore)
    }
    fn update(&mut self,args:&rusqlite::vtab::Updates<'_>)->Result<()> {
        use rusqlite::vtab::ConflictMode;
        let conflict=unsafe {args.on_conflict(self.connection)};
        let values=(2..args.len()).map(|i|args.get::<Option<i64>>(i)).collect::<Result<Vec<_>>>()?;
        self.binding.current().write(args.get(0)?,args.get(1)?,Some(values),conflict==ConflictMode::Replace,conflict==ConflictMode::Ignore).map(|_|())
    }
}
// SAFETY: SQLite's cursor base is first. No borrowed SQLite value escapes filter.
unsafe impl VTabCursor for Cursor {
    fn filter(&mut self, index: c_int, list: Option<&str>, args: &Filters<'_>) -> Result<()> {
        use crate::code_rule_dsl::sql_frame::{frame,FrameEvent};
        frame(if index==0 {FrameEvent::Scan} else {FrameEvent::Lookup},1);
        let stats = &self.binding.stats;
        stats.scans.fetch_add(1, Ordering::Relaxed);
        self.source = self.binding.current();
        if index==MULTI {
            if let Some(list)=list {
                let(tag,cols)=list.split_at(1);
                let columns=cols.split(',').filter_map(|c|c.parse::<usize>().ok()).collect::<Vec<_>>();
                let full=self.source.primary_key_len().is_some_and(|n|(0..n).all(|c|columns.contains(&c)));
                let mut sorted=columns.clone();sorted.sort_unstable();sorted.dedup();
                if matches!(tag,"p"|"b") {self.source.prepare_lookup(&columns[..1]);}
                else if !full&&!sorted.iter().copied().eq(0..sorted.len()) {self.source.prepare_lookup(&columns);}

            }
        } else if index>0&&index< (1<<20) {self.source.prepare_lookup(&[(index/2-1) as usize]);}
        if let Some(version)=self.source.frozen() {self.source=version;}
        self.cache_rows=self.source.caches_rows();
        *self.cached.get_mut()=None;
        self.position = 0;
        self.reverse=false;
        self.lazy=false;
        self.selected = None;
        self.direct=None;
        self.stream=None;self.stream_row=None;
        self.range = 0..0;
        if index==ORDERED {
            self.lazy=true;
            let (direction,columns)=list.ok_or_else(||failure("missing ordered columns"))?.split_once(':').ok_or_else(||failure("invalid ordered columns"))?;
            let mut equal=Vec::new();
            for (arg,column) in columns.split(',').filter(|c|!c.is_empty()).enumerate() {
                let Some(value)=integer(&args.get::<Value>(arg)?) else {self.range=0..0;return Ok(());};
                equal.push((column.parse().map_err(|_|failure("invalid ordered field"))?,value));
            }
            self.range=self.source.ordered_range(&equal).ok_or_else(||failure("ordered source lost its index"))?;
            self.reverse=direction=="d";
            if !self.range.is_empty(){stats.candidates.fetch_add(1,Ordering::Relaxed);}
            self.start_stream();
            return Ok(());
        }
        if index == PRIMARY_BATCH {
            let mut columns:Vec<Vector<i64>>=Vec::new();
            for (arg,tag) in list.ok_or_else(||failure("missing primary batch columns"))?.chars().enumerate() {
                let mut values=Vector::new();
                if tag=='b' {
                    stats.batches.fetch_add(1,Ordering::Relaxed);
                    let mut iter=args.in_values(arg)?;
                    while let Some(value)=iter.next()? {values.extend(integer(&Value::try_from(value).map_err(|e|failure(&e.to_string()))?));}
                } else {values.extend(integer(&args.get::<Value>(arg)?));}
                values.sort_unstable();values.dedup();columns.push(values);
            }
            self.selected=Some(self.source.lookup_primary_batch(&columns).ok_or_else(||failure("primary batch source lost its index"))?);
        } else if index == MULTI {
            let list = list.ok_or_else(|| failure("missing index columns"))?;
            let (tag, list) = list.split_at(1);
            let columns: Vector<usize> = list.split(',').map(|c| c.parse::<usize>().map_err(|_| failure("bad index columns"))).collect::<Result<_>>()?;
            let mut values = rerail_blocks::vector::Vector::new();
            let mut equal: Vec<(usize, i64)> = Vec::new();
            let mut matchless = false;
            for (arg, &c) in columns.iter().enumerate() {
                if arg == 0 && tag == "b" {
                    stats.batches.fetch_add(1, Ordering::Relaxed);
                    let mut iter = args.in_values(0)?;
                    while let Some(value) = iter.next()? {
                        values.extend(integer(&Value::try_from(value).map_err(|e| failure(&e.to_string()))?));
                    }
                    values.sort_unstable();
                    values.dedup();
                    continue;
                }
                // A value that is not an integer matches no cell (as a single-column probe answers).
                match integer(&args.get::<Value>(arg)?) {
                    Some(v) => equal.push((c, v)),
                    None => matchless = true,
                }
            }
            let source = &self.source;
            let keep = |r: &usize| equal.iter().all(|&(c, v)| source.value(*r, c) == v);
            self.selected = if matchless {
                Some(rerail_blocks::vector::Vector::new())
            } else {
                match tag {
                    "c" => {
                        let mut by = equal.clone();
                        by.sort_unstable();
                        self.direct=source.direct_all(&by);
                        if self.direct.is_some() {Some(rerail_blocks::vector::Vector::new())} else {
                            source.lookup_all(&by).or_else(|| source.lookup(by[0].0, &[by[0].1]).map(|r| r.into_iter().filter(keep).collect()))
                        }
                    }
                    // A probe over a column that spans the documents (a kind), where the document is bound as
                    // well: kept to that document, rather than every document's rows rechecked against it.
                    "p" | "b" => {
                        let probed: Vector<i64> = if tag == "p" { rerail_blocks::vector![equal[0].1] } else { values.clone() };
                        let doc = source.doc_column().filter(|_| source.kept_to_doc(columns[0])).and_then(|d| equal.iter().find(|&&(c, _)| c == d).map(|&(_, v)| v));
                        doc.and_then(|d| source.lookup_in(columns[0], &probed, d))
                            .or_else(|| source.lookup(columns[0], &probed))
                            .map(|r| r.into_iter().filter(keep).collect())
                    }
                    _ => Some(source.filter_eq(&equal).into_iter().map(|r| r as usize).collect()),
                }
            };
        } else if index & (1 << 20) != 0 {
            let column = ((index & ((1 << 20) - 1)) >> 4) - 1;
            let mut arg = 0;
            let lower = if index & 1 != 0 {
                let v = args.get::<Value>(arg)?;
                arg += 1;
                Some(bound(&v, index & 4 != 0, true))
            } else {
                None
            };
            let upper = if index & 2 != 0 { Some(bound(&args.get::<Value>(arg)?, index & 8 != 0, false)) } else { None };
            // A bound that is not a number leaves the scan to SQLite's recheck.
            if lower.is_none_or(|b| b.is_some()) && upper.is_none_or(|b| b.is_some()) {
                self.selected = self.source.range_lookup(column as usize, lower.flatten(), upper.flatten());
            }
        } else if index != 0 {
            let batch = index % 2 == 1;
            let column = (index / 2 - 1) as usize;
            let mut values = Vec::new();
            if batch {
                #[cfg(any(test, feature = "test-hooks"))]
                let _span=crate::code_rule_dsl::logic::profile::span(|| "SQL IN key decode".into());
                stats.batches.fetch_add(1, Ordering::Relaxed);
                let mut iter = args.in_values(0)?;
                while let Some(value) = iter.next()? {
                    values.extend(integer(&Value::try_from(value).map_err(|e| failure(&e.to_string()))?));
                }
                values.sort_unstable();
                values.dedup();
            } else {
                values.extend(integer(&args.get::<Value>(0)?));
            }
            #[cfg(any(test, feature = "test-hooks"))]
            let _span=batch.then(||crate::code_rule_dsl::logic::profile::span(|| "SQL IN index lookup".into()));
            self.direct=self.source.direct_one(column,&values);
            if self.direct.is_none() {self.selected = self.source.lookup(column, &values);}
        }
        // Only a scan takes the length: a source whose rows are made on lookup makes all of them for it.
        if self.selected.is_none() && self.direct.is_none() {
            self.range = 0..self.source.len();
        }
        if let Some(message) = self.source.failure() {
            return Err(failure(&message));
        }
        let count = self.direct.as_ref().map_or_else(||self.selected.as_ref().map_or(self.range.len(), |v| v.len()),|v| v.len());
        stats.candidates.fetch_add(count as u64, Ordering::Relaxed);
        self.start_stream();
        Ok(())
    }
    fn next(&mut self) -> Result<()> {
        if let Some(stream)=&mut self.stream {self.stream_row=stream.next();}
        crate::code_rule_dsl::sql_frame::frame(crate::code_rule_dsl::sql_frame::FrameEvent::Next,1);
        if self.selected.is_some()||self.direct.is_some() {
            self.position += 1;
        } else {
            if self.reverse {self.range.end-=1;}else{self.range.start+=1;}
        }
        if self.lazy&&!self.range.is_empty(){self.binding.stats.candidates.fetch_add(1,Ordering::Relaxed);}
        Ok(())
    }
    fn eof(&self) -> bool {
        self.direct.as_ref().map_or_else(||self.selected.as_ref().map_or(self.range.is_empty(), |r| self.position >= r.len()),|r|self.position>=r.len())
    }
    fn column(&self, ctx: &mut Context, col: c_int) -> Result<()> {
        crate::code_rule_dsl::sql_frame::frame(crate::code_rule_dsl::sql_frame::FrameEvent::Column,1);
        self.binding.stats.columns.fetch_add(1, Ordering::Relaxed);
        let value=if let Some(row)=&self.stream_row {row.cells.cell(col as usize)} else if let Some(rows)=&self.direct {rows[self.position].cells.cell(col as usize)} else if self.cache_rows {self.with_cells(|_,cells|cells.cell(col as usize))} else {self.source.cell(self.row(),col as usize)};
        ctx.set_result(&value)
    }
    fn rowid(&self) -> Result<i64> {
        Ok(if let Some(row)=&self.stream_row {row.id} else if let Some(rows)=&self.direct {rows[self.position].id} else if self.cache_rows {self.with_cells(|id,_|id)} else {self.source.row_id(self.row())})
    }
}

/// Integer cells with every column indexed: each column's rows sorted by value, built on first use. Producers'
/// compact relations (S `relation.rs`) and small derived relations. The cells are rows in a block the CPU and GPU
/// read in place (`facts::rows`, decision 0325): a device stage binds them where they lie (`Source::in_block`).
/// Rows pushed are staged and written into their block whole at the first read, exactly sized: built a row at a time
/// in a block shared with others, a relation's growth and moves held 70 MB more at the peak on posthog/hogql.
/// A column whose values were pushed in order (a document's lines, columns and texts, by node) is its own index:
/// searched in the cells, with no sort and no row list. Its distinct values are counted as it is pushed.
pub struct Relation {
    /// Its columns' names, interned once a schema (`column_names`): a relation makes none of its own.
    columns: &'static [&'static str],
    /// Rows pushed and not yet read, row-major: the first read moves them into `cells`.
    staged: std::sync::Mutex<Vector<i64>>,
    rows: usize,
    cells: OnceLock<super::rows::Rows>,
    /// Per column, its index and its order, in one list.
    cols: Box<[Col]>,
    /// Every row's id in order, made when a column pushed in order is first probed for a slice of its rows
    /// (`sorted_rows`): such a column's rows of one value are consecutive, and need no index of their own.
    identity: OnceLock<Vector<u32>>,
    /// Rows a later relation goes on from (`grown`, decision 0431 §6): frozen when they are written into their block,
    /// so that one takes their writer and its rows follow these in the same block relation.
    grows: bool,
}
/// A column's sorted order, in the relation's block (t56 step 3), searched by the orders' one search
/// (`rerail_blocks::order`): its rows in value order (then row), the value at every `FENCE`th, and each row's print of
/// its value. A probe of one value reads the fences, then its span's prints, eight a word, and only the rows whose
/// print is the value's: about one row read where a bisection read eight, each a decode of its sealed chunk.
/// A relation's column: its index (made when it is first probed), and its distinct values while its values have come
/// in nondecreasing order, else `None`.
struct Col {
    index: OnceLock<ColumnIndex>,
    ordered: Option<usize>,
}
impl Col {
    fn of(ordered: Option<usize>) -> Self {
        Col { index: OnceLock::new(), ordered }
    }
}

/// `columns` interned: one list a schema, made once in the process and never freed (an append-only interner of the
/// engine's own names, as its atoms' are). A relation's columns were a `Vec` of a `String` each, `1 + k` allocations
/// a relation, and a chunk makes millions of relations.
fn column_names<S: AsRef<str>>(columns: &[S]) -> &'static [&'static str] {
    use std::hash::{BuildHasher, Hasher};
    type Names = &'static [&'static str];
    thread_local! { static NEAR: std::cell::RefCell<rustc_hash::FxHashMap<u64, Names>> = Default::default(); }
    static ALL: std::sync::Mutex<Option<rustc_hash::FxHashMap<u64, Vec<Names>>>> = std::sync::Mutex::new(None);
    let same = |n: Names| n.len() == columns.len() && n.iter().zip(columns).all(|(a, b)| *a == b.as_ref());
    let mut h = rustc_hash::FxBuildHasher.build_hasher();
    for c in columns {
        h.write(c.as_ref().as_bytes());
        h.write_u8(0xff);
    }
    let key = h.finish();
    // The thread's list of the names' hash, else the process's (a hash two lists share: the process's alone).
    if let Ok(Some(n)) = NEAR.try_with(|m| m.borrow().get(&key).copied().filter(|&n| same(n))) {
        return n;
    }
    let n = {
        let mut all = ALL.lock().unwrap_or_else(|e| e.into_inner());
        let lists = all.get_or_insert_with(Default::default).entry(key).or_default();
        match lists.iter().copied().find(|&n| same(n)) {
            Some(n) => n,
            None => {
                let n: Names = Box::leak(columns.iter().map(|c| &*Box::leak(Box::<str>::from(c.as_ref()))).collect::<Vec<&'static str>>().into_boxed_slice());
                lists.push(n);
                n
            }
        }
    };
    let _ = NEAR.try_with(|m| m.borrow_mut().insert(key, n));
    n
}

struct ColumnIndex {
    /// The rows in value order; empty for a column pushed in order (`Relation::ordered`): its rows are their order.
    ids: Vector<u32>,
    distinct: usize,
    fences: Vector<i64>,
    prints: Vector<u8>,
}
impl ColumnIndex {
    /// The row at position `p` of the order.
    #[inline(always)]
    fn row(&self, p: usize) -> u32 {
        if self.ids.is_empty() { p as u32 } else { self.ids[p] }
    }
}
use rerail_blocks::order::FENCE;
impl Relation {
    /// The bytes its rows' chunks hold in their block (none before its first read writes them there).
    pub fn block_bytes(&self) -> usize {
        self.cells.get().map_or(0, super::rows::Rows::capacity_bytes)
    }

    pub fn new(columns: &[&str]) -> Self {
        assert!(!columns.is_empty());
        Self {
            columns: column_names(columns),
            staged: Default::default(),
            rows: 0,
            cells: OnceLock::new(),
            cols: columns.iter().map(|_| Col::of(Some(0))).collect(),
            identity: OnceLock::new(),
            grows: false,
        }
    }

    /// This relation as one a later relation goes on from (`grown`): its rows frozen in their block (`Rows::freeze`),
    /// now if they are written there, else when they are.
    pub fn growable(mut self) -> Self {
        self.grows = true;
        if let Some(cells) = self.cells.take() {
            let _ = self.cells.set(cells.freeze(self.rows));
        }
        self
    }

    /// Whether its rows lie in a part `other`'s do (decision 0616's tests: read where they lie, not copied).
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn shares_rows(&self, other: &Relation) -> bool {
        self.cells().shares(other.cells())
    }
    /// How many rows a reader binding it whole in one block reads (`rows::Rows::in_block`), parts merged.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn bound_rows(&self) -> Option<u32> {
        self.cells().in_block().map(|b| b.rows)
    }

    /// How many parts hold its rows `range` (`rows::Rows::runs`): the segments a relation of them has.
    pub fn parts_in(&self, range: std::ops::Range<usize>) -> usize {
        if range.is_empty() {
            return 0;
        }
        self.cells().runs(range.start as u32, range.end as u32).len()
    }

    /// A relation of `runs`' rows, in order: each run rows `lo..hi` of a relation of the same columns, read where
    /// they lie (`rows::Part::range`), no row copied (decision 0616). The rows a later `grown` adds go on in a
    /// relation of their own after them. Its orders are not known.
    pub fn of_runs(columns: &[String], runs: &[(&Relation, std::ops::Range<usize>)]) -> Self {
        let k = columns.len();
        let mut parts = Vec::new();
        for (r, range) in runs {
            assert!(r.columns.len() == columns.len() && r.columns.iter().zip(columns).all(|(a, b)| a == b), "a run of a relation of other columns");
            parts.extend(r.cells().runs(range.start as u32, range.end as u32));
        }
        let cells = super::rows::Rows::of_parts(k, super::rows::Life::Source, parts);
        let rows = cells.rows();
        Relation {
            columns: column_names(columns),
            staged: Default::default(),
            rows,
            cells: OnceLock::from(cells),
            cols: columns.iter().map(|_| Col::of(None)).collect(),
            identity: OnceLock::new(),
            grows: true,
        }
    }

    /// These rows and then `more`'s, as a batch grows by a round's documents (decision 0431 §6): this relation's rows
    /// shared, not rewritten (`Rows::shared`: their writer taken, so the rows that follow go on in the same block
    /// relation, which the device binds whole), and `more`'s written after them. This relation still reads its own
    /// rows (a prefix), as the round before read them. Its indexes are made again, over all the rows, when probed.
    pub fn grown(&self, more: Relation) -> Relation {
        assert_eq!(self.columns, more.columns, "a relation grown by rows of other columns");
        let k = self.columns.len();
        let base = self.cells();
        let mut r = Relation {
            columns: self.columns,
            staged: Default::default(),
            rows: self.rows,
            cells: OnceLock::from(base.shared(self.rows)),
            cols: self.cols.iter().map(|c| Col::of(c.ordered)).collect(),
            identity: OnceLock::new(),
            grows: true,
        };
        let Relation { staged, cells, rows, .. } = more;
        match cells.into_inner() {
            Some(cells) => {
                let mut row = vec![0; k];
                for i in 0..rows as u32 {
                    row.iter_mut().enumerate().for_each(|(c, v)| *v = cells.get(i, c));
                    r.push(&row);
                }
            }
            None => staged.into_inner().unwrap_or_else(|e| e.into_inner()).chunks_exact(k).for_each(|row| r.push(row)),
        }
        let cells = r.cells.take().expect("the grown rows");
        let _ = r.cells.set(cells.freeze(r.rows));
        r
    }
    /// A relation of `rows`, staged (`push`), not yet written into a block: rows a relation that goes on from another
    /// writes once, past that one's (`grown`: a tape value's rows admitted inside a chunk's fixpoint, decision 0431 §8).
    pub fn staged(columns: &[&str], rows: &mut dyn Iterator<Item = Row>) -> Self {
        let mut r = Relation::new(columns);
        rows.for_each(|row| r.push(&row));
        r
    }
    /// A relation of `rows`: written straight into its block, at their number, when the iterator knows it exactly
    /// (no staging, no growth or move); else staged, as `push` stages them.
    pub fn from_rows(columns: &[&str], rows: &mut dyn Iterator<Item = Row>) -> Self {
        let mut r = Relation::new(columns);
        if let (n, Some(m)) = rows.size_hint()
            && n == m
            && n > 0
        {
            let mut cells = super::rows::Rows::living(r.columns.len(), super::rows::Life::Source);
            cells.reserve(n * r.columns.len());
            let _ = r.cells.set(cells);
        }
        rows.for_each(|row| r.push(&row));
        r
    }
    pub fn push(&mut self, row: &[i64]) {
        assert_eq!(row.len(), self.columns.len());
        assert!(self.len() < u32::MAX as usize);
        let last = self.len().checked_sub(1);
        let k = row.len();
        // The last row read where it is: staged, or (pushed after a read) in the block.
        let staged = self.staged.get_mut().unwrap_or_else(|e| e.into_inner());
        let cells = self.cells.get();
        for (c, o) in self.cols.iter_mut().map(|c| &mut c.ordered).enumerate() {
            let prev = last.map(|l| cells.map_or_else(|| staged[l * k + c], |r| r.get(l as u32, c)));
            *o = match (*o, prev) {
                (Some(d), None) => Some(d + 1),
                (Some(d), Some(p)) if row[c] > p => Some(d + 1),
                (Some(d), Some(p)) if row[c] == p => Some(d),
                _ => None,
            };
        }
        match self.cells.get_mut() {
            Some(cells) => cells.extend_from_slice(row),
            None => staged.extend_from_slice(row),
        }
        self.rows += 1;
        self.cols.iter_mut().for_each(|c| c.index = OnceLock::new());
        self.identity = OnceLock::new();
    }
    /// Column `c` of rows `range`, resolved once where they lie in one part (`Rows::range_reader`).
    #[inline]
    pub fn range_reader(&self, range: Range<usize>, c: usize) -> Option<super::rows::RangeRead<'_>> {
        self.cells().range_reader(range.start as u32, range.end as u32, c)
    }
    /// The rows in their block: the staged ones written there at the first read, in one relation of their size.
    #[inline]
    fn cells(&self) -> &super::rows::Rows {
        crate::code_rule_dsl::logic::profile::once(&self.cells, "relation: staged rows written", || {
            let staged = std::mem::take(&mut *self.staged.lock().unwrap_or_else(|e| e.into_inner()));
            let mut cells = super::rows::Rows::living(self.columns.len(), super::rows::Life::Source);
            cells.reserve(staged.len());
            cells.extend_from_slice(&staged);
            if self.grows { cells.freeze(self.rows) } else { cells }
        })
    }
    fn index(&self, column: usize) -> &ColumnIndex {
        crate::code_rule_dsl::logic::profile::once(&self.cols[column].index, "relation: a column's index built", || {
            use super::rows::collect_in;
            use rerail_blocks::order::print_vals;
            let home = self.cells().home();
            let print = |v: i64| print_vals(std::iter::once(v));
            if let Some(distinct) = self.cols[column].ordered {
                // Its rows are its order: the fences and prints of its values as they lie, each read once.
                let vals = (0..self.len()).map(|r| self.value(r, column));
                let prints = collect_in(home.as_ref(), vals.map(print));
                let fences = collect_in(home.as_ref(), (0..self.len()).step_by(FENCE).map(|r| self.value(r, column)));
                return ColumnIndex { ids: collect_in(home.as_ref(), std::iter::empty()), distinct, fences, prints };
            }
            // Each value read once, and the pairs sorted (ties by row, as a stable sort by value): a sort reading the
            // rows' values at every comparison read each of them about 2 log n times.
            let mut pairs: Vec<(i64, u32)> = (0..self.len()).map(|r| (self.value(r, column), r as u32)).collect();
            pairs.sort_unstable();
            let distinct = 1 + pairs.windows(2).filter(|w| w[0].0 != w[1].0).count();
            let fences = collect_in(home.as_ref(), pairs.iter().step_by(FENCE).map(|p| p.0));
            let prints = collect_in(home.as_ref(), pairs.iter().map(|p| print(p.0)));
            let ids = collect_in(home.as_ref(), pairs.into_iter().map(|p| p.1));
            ColumnIndex { ids, distinct, fences, prints }
        })
    }
    /// Whether `column`'s values were pushed in order, so that it is its own index.
    pub fn ordered(&self, column: usize) -> bool {
        self.cols[column].ordered.is_some()
    }
    /// The positions in `column`'s order of the rows holding `value` (`rerail_blocks::order::find`: the fences, then
    /// the span's prints, then the rows whose print is the value's).
    fn equal(&self, column: usize, value: i64) -> Range<usize> {
        let ix = self.index(column);
        let (s, e) = rerail_blocks::order::find(self.len(), &ix.fences, &[], &ix.prints, 1, &[value], |p, _| self.value(ix.row(p) as usize, column).cmp(&value));
        s..e
    }
    /// The rows with values in `lo..=hi`, in value order.
    fn rows_between(&self, column: usize, lo: i64, hi: i64) -> rerail_blocks::vector::Vector<usize> {
        let ix = self.index(column);
        self.between(column, lo, hi).map(|p| ix.row(p) as usize).collect()
    }
    /// The positions in `column`'s index of the rows with values in `lo..=hi`.
    fn between(&self, column: usize, lo: i64, hi: i64) -> Range<usize> {
        let index = self.index(column);
        let (n, at) = (self.len(), |p: usize| self.value(index.row(p) as usize, column));
        // The first position not below `lo`: the fence's entry past it bounds the rows to search to one fence's span.
        let j = index.fences.partition_point(|&k| k < lo);
        let (from, to) = (j.saturating_sub(1) * FENCE, (j * FENCE).min(n));
        let start = rerail_blocks::order::split_at_pos(from, to, |p| at(p) < lo);
        // The end from the start (`first_from`): no second search of the fence and its span.
        start..first_from(n, start, |m| at(m) <= hi).max(start)
    }
}

/// The first of `0..n` for which `below` is false (`below` true before it, false from it; `n` where none is),
/// galloping from `from` (any of `0..=n`): forward when `from` is below, else back. A search near `from` reads a few
/// rows, one far off about 2 log n.
fn first_from(n: usize, from: usize, below: impl Fn(usize) -> bool) -> usize {
    let (mut a, mut b) = if from < n && below(from) {
        // Rows `..=from` are below: double forward until a row is not.
        let (mut a, mut step) = (from + 1, 1);
        while a + step <= n && below(a + step - 1) {
            a += step;
            step *= 2;
        }
        (a, (a + step).min(n))
    } else {
        // Rows `from..` are not below: double back until a row is.
        let (mut b, mut step) = (from.min(n), 1);
        while b >= step && !below(b - step) {
            b -= step;
            step *= 2;
        }
        (b.saturating_sub(step), b)
    };
    while a < b {
        let m = a + (b - a) / 2;
        if below(m) { a = m + 1 } else { b = m }
    }
    a
}
impl Source for Relation {
    fn revision(&self)->Option<u64> {Some(0)}
    fn columns(&self) -> Vec<String> {
        self.columns.iter().map(|c| (*c).to_owned()).collect()
    }
    fn len(&self) -> usize {
        self.rows
    }
    fn in_block(&self) -> Option<super::rows::InBlock> {
        self.cells().in_block()
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        let distinct = self.cols[column].ordered.unwrap_or_else(|| self.index(column).distinct);
        let one = self.len().div_ceil(distinct.max(1));
        if batch { one * 8 } else { one }
    }
    fn indexed(&self, _: usize) -> bool {
        true
    }
    fn range_indexed(&self, _: usize) -> bool {
        true
    }
    fn range_lookup(&self, column: usize, lower: Option<(i64, bool)>, upper: Option<(i64, bool)>) -> Option<rerail_blocks::vector::Vector<usize>> {
        let lo = match lower {
            None => i64::MIN,
            Some((v, true)) => v,
            Some((v, false)) => match v.checked_add(1) {
                Some(v) => v,
                None => return Some(rerail_blocks::vector::Vector::new()),
            },
        };
        let hi = match upper {
            None => i64::MAX,
            Some((v, true)) => v,
            Some((v, false)) => match v.checked_sub(1) {
                Some(v) => v,
                None => return Some(rerail_blocks::vector::Vector::new()),
            },
        };
        Some(self.rows_between(column, lo, hi))
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<rerail_blocks::vector::Vector<usize>> {
        #[cfg(any(test, feature = "test-hooks"))]
        if let Some(rows) = lookup_spike::selected(self, column, values) { return Some(rows); }
        let ix = self.index(column);
        Some(values.iter().flat_map(|&v| self.equal(column, v).map(|p| ix.row(p) as usize)).collect())
    }
    /// A slice of the column's sorted index, or for a column pushed in order of the rows' ids: a probe makes no list.
    /// Through `lookup`, each probe of a relation built for a chunk (the scopes' `rr_nearest_*`, the texts)
    /// collected its rows into a list, and the join copied it into another: 6,400 samples of a cold PostHog run.
    fn sorted_rows(&self, column: usize, value: i64) -> Option<&[u32]> {
        let range = self.equal(column, value);
        if self.cols[column].ordered.is_some() {
            let ids = self.identity.get_or_init(|| (0..self.len() as u32).collect());
            return Some(&ids[range]);
        }
        Some(&self.index(column).ids[range])
    }
    /// The rows of `value` with their values of `columns`, read from the column's order where they lie: no list of
    /// ids made a probe (`lookup`'s), which the join's view probe made for every relation a chunk binds (t50 step 6).
    fn lookup_read(&self, column: usize, value: i64, columns: &[usize], each: &mut dyn FnMut(usize, &[i64])) -> bool {
        if columns.len() > 8 {
            return false;
        }
        let mut row = [0i64; 8];
        let cells = self.cells();
        let mut give = |r: usize| {
            for (j, &c) in columns.iter().enumerate() {
                row[j] = cells.get(r as u32, c);
            }
            each(r, &row[..columns.len()]);
        };
        let ix = self.index(column);
        self.equal(column, value).for_each(|p| give(ix.row(p) as usize));
        true
    }
    /// A column pushed in order of the rows' ids: its rows a range, no identity list made.
    fn sorted_range(&self, column: usize, value: i64) -> Option<Range<u32>> {
        self.cols[column].ordered.is_some().then(|| {
            let r = self.equal(column, value);
            r.start as u32..r.end as u32
        })
    }
    fn sorts_rows(&self, _column: usize) -> bool {
        true
    }
    /// A column pushed in order whose values are each one row (`ordered`: as many distinct values as rows) is a key:
    /// a view of the relation's rows probes it here (`setfix::hold_view_in_place`), with no index of its own.
    fn keys_rows(&self, column: usize) -> bool {
        self.cols[column].ordered == Some(self.len())
    }
    /// The row of `value` in a key column (`keys_rows`): a search of the cells, as `lookup`'s.
    fn key_row(&self, column: usize, value: i64) -> Option<usize> {
        debug_assert!(self.keys_rows(column));
        self.equal(column, value).next()
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        self.cells().get(row as u32, column)
    }
}

/// S's `query/plan/relation.rs` as copied (0689eb25); merged into this module (decision 0008).
#[allow(dead_code)]
pub mod s_relation;
// cratify: `p_table` is rerail-workspace's (it mounts it)
// cratify: `tests` is rerail-logic's (it mounts it)

#[cfg(any(test, feature = "test-hooks"))]
pub mod lookup_spike;

#[cfg(test)]
mod first_from_tests {
    /// `first_from` finds `partition_point`'s answer from any start, for every split of every short run.
    #[test]
    fn first_from_is_partition_point_from_anywhere() {
        for n in 0..40usize {
            let values: Vec<usize> = (0..n).map(|i| i / 3).collect();
            for t in 0..=n / 3 + 1 {
                let want = values.partition_point(|&v| v < t);
                for from in 0..=n {
                    assert_eq!(super::first_from(n, from, |m| values[m] < t), want, "n {n}, t {t}, from {from}");
                }
            }
        }
    }
}
