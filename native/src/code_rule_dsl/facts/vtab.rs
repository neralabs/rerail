//! Read-only eponymous virtual tables over integer relations (decision 0008; 05 Phase 7 row 3).
//!
//! A table is registered once per connection and rebound per wave without DDL or a reprepare: its
//! [`Binding`] swaps the [`Source`] that later filters read, while open cursors keep the one they
//! started with. `xBestIndex` takes an `EQ` or `IN` constraint on an indexed column (the whole `IN`
//! list in one filter) or a range on a range-indexed column. Several `EQ` are one filter: a source's
//! sorted order over them all (`composite`), else a probe of the most selective column with the rest
//! checked here; estimates come from the source's distinct counts, so SQL written by hand plans well.
//! SQLite rechecks every constraint, so a source may answer with a superset. Every column is an integer: kinds, fields and symbols are atoms.
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
pub mod syntax;
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
pub trait Source: Send + Sync {
    fn python_batch(&self) -> Option<Arc<syntax::Batch>> { None }
    fn row_stream(&self,_range:Range<usize>,_reverse:bool)->Option<Box<dyn RowStream>> {None}
    fn direct_all(&self,_equal:&[(usize,i64)])->Option<Vec<ReadRow>> {None}
    fn direct_one(&self,_column:usize,_values:&[i64])->Option<Vec<ReadRow>> {None}
    fn prepare_lookup(&self,_columns:&[usize]) {}
    fn caches_rows(&self)->bool {false}
    fn row_cells(&self,_row:usize)->Option<(i64,Arc<dyn RowCells>)> {None}
    fn prepare_index(&self, _columns:&[usize]) {}
    fn primary_key_len(&self)->Option<usize> {None}
    fn lookup_primary_batch(&self, _values:&[Vec<i64>])->Option<Vec<usize>> {None}
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
    /// The source as itself, for a caller that carries state between two of one kind (`carry_sorted`).
    fn as_any(&self) -> Option<&dyn std::any::Any> { None }
    fn columns(&self) -> Vec<String>;
    fn len(&self) -> usize;
    /// Row `row`'s values of `columns`, into `out`: one value at a time by default; a source that finds a row's
    /// place first (a view of a batch's documents) finds it once.
    fn row_at(&self, row: usize, columns: &[usize], out: &mut [i64]) {
        for (o, &c) in out.iter_mut().zip(columns) {
            *o = self.value(row, c);
        }
    }
    fn estimated_len(&self) -> usize {
        self.len()
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize;
    /// The rows estimated to match one value on each of `columns` at once. By default the columns are taken
    /// as independent (each keeps its share of the rows), bounded by the most selective one.
    fn estimated_matches_all(&self, columns: &[usize]) -> usize {
        let n = self.estimated_len().max(1) as f64;
        let each: Vec<usize> = columns.iter().map(|&c| self.estimated_matches(c, false)).collect();
        let independent = each.iter().fold(n, |rows, &m| rows * (m as f64 / n).min(1.0));
        (independent.ceil() as usize).max(1).min(each.into_iter().min().unwrap_or(usize::MAX))
    }
    /// Whether `lookup_all` answers from one sorted order over several columns.
    fn composite(&self) -> bool {
        false
    }
    /// The rows whose columns equal the values (`(column, value)` by column, at least two), from one sorted
    /// order over them; `None` probes one column and filters the rest.
    fn lookup_all(&self, _equal: &[(usize, i64)]) -> Option<Vec<usize>> {
        None
    }
    /// The column holding a row's document, where a lookup can be kept to one document (`lookup_in`).
    fn doc_column(&self) -> Option<usize> {
        None
    }
    /// The rows whose `column` is one of `values` in document `doc` (by its key): a lookup over a column that
    /// spans the documents (a kind) where the statement binds the document too. `None`: not kept to one.
    fn lookup_in(&self, _column: usize, _values: &[i64], _doc: i64) -> Option<Vec<usize>> {
        None
    }
    /// Whether a lookup on `column` can be kept to one document (`lookup_in`).
    fn kept_to_doc(&self, _column: usize) -> bool {
        false
    }
    /// The rows whose columns hold every `(column, value)` of `eqs`, ascending: a scan, which a source may do faster
    /// than a `value` call per row.
    fn filter_eq(&self, eqs: &[(usize, i64)]) -> Vec<u32> {
        (0..self.len()).filter(|&r| eqs.iter().all(|&(c, v)| self.value(r, c) == v)).map(|r| r as u32).collect()
    }
    /// Rows are made per looked-up value, so reading them all (a scan, a hash index of every row)
    /// makes every one: a join probes such a source through `lookup`, whatever its match estimate.
    fn made_on_lookup(&self) -> bool {
        false
    }
    /// The planner expects `keys` lookups on `column` (decision 0105): a source that can make every row at once for
    /// less than that many lookups cost chooses to.
    fn expect_lookups(&self, _column: usize, _keys: f64) {}
    /// The planner's statistics (`syntax::statistics`'s rows) where the source knows them without
    /// reading every row: a source whose rows are made on lookup.
    fn statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        None
    }
    /// How many rows hold each value `v` in column `c`, for each `(c, v)` of `wanted`, where the source keeps them
    /// (its rows do not change): `None` when the caller counts them.
    fn value_counts(&self, _wanted: &[(usize, i64)]) -> Option<Vec<i64>> {
        None
    }
    /// Why rows could not be made, where a source makes them on demand and making them failed (a
    /// stored frame that does not decode). The statement reading it fails with this message rather
    /// than reading the rows as absent.
    fn failure(&self) -> Option<String> {
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
    fn range_lookup(&self, _column: usize, _lower: Option<(i64, bool)>, _upper: Option<(i64, bool)>) -> Option<Vec<usize>> {
        None
    }
    /// The rows whose `column` is one of `values`; `None` scans.
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>>;
    fn value(&self, row: usize, column: usize) -> i64;
    /// The rows whose `column` is `value`, as a slice of the source's own sorted index (no allocation); `None`
    /// where the source keeps none.
    fn sorted_rows(&self, _column: usize, _value: i64) -> Option<&[u32]> {
        None
    }
    /// Whether `sorted_rows` serves `column`, asked without building its index: a plan choosing among a source's
    /// columns built every candidate's sorted form to ask, where it reads one.
    fn sorts_rows(&self, column: usize) -> bool {
        self.sorted_rows(column, 0).is_some()
    }
}

#[derive(Default)]
pub struct Counters {
    pub scans: AtomicU64,
    pub batches: AtomicU64,
    pub candidates: AtomicU64,
    pub columns: AtomicU64,
}

#[derive(Clone)]
pub struct Binding {
    current: Arc<parking_lot::RwLock<Arc<dyn Source>>>,
    schema: Arc<Metadata>,
    stats: Arc<Counters>,
}
impl Binding {
    pub fn clear(&self) {
        *self.current.write() = self.schema.clone();
    }
    pub fn set(&self, source: Arc<dyn Source>) -> Result<()> {
        if self.schema.columns != source.columns() || self.schema.writable!=source.writable() || self.schema.text.iter().enumerate().any(|(c,t)| *t!=source.text_column(c)) {
            return Err(failure("native binding schema changed"));
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
    fn lookup(&self, _: usize, _: &[i64]) -> Option<Vec<usize>> {
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
        BOUND.lock().insert((connection_scope(db),name.to_owned()),binding.clone());
        self.bindings.borrow_mut().insert(name.into(),binding);
        Ok(())
    }
    fn bind_inner(&self, db:&Connection, name:&str, source:Arc<dyn Source>, restored:bool)->Result<()> {
        let mut bindings = self.bindings.borrow_mut();
        if let Some(binding) = bindings.get(name) {
            return binding.set(source);
        }
        // A kept connection's table of this shape from its last run (`reuse`): its module is registered still.
        let at = connection_scope(db);
        let shape = (source.columns(), (0..source.columns().len()).map(|c| source.text_column(c)).collect::<Vec<_>>(), source.writable());
        let kept = REGISTERED.lock().remove(&(at, name.to_owned()));
        // (A writable table's `temp` virtual table is there still: the run's clean-up drops none it binds, and one it
        // drops is no longer kept, `dropped_temp`.)
        if let Some(binding) = kept.filter(|b| (b.schema.columns.clone(), b.schema.text.clone(), b.schema.writable) == shape) {
            binding.set(source)?;
            BOUND.lock().insert((at, name.to_owned()), binding.clone());
            bindings.insert(name.into(), binding);
            return Ok(());
        }
        let binding = Binding {
            schema: Arc::new(Metadata { text:(0..source.columns().len()).map(|c|source.text_column(c)).collect(), columns: source.columns(), writable:source.writable() }),
            current: Arc::new(parking_lot::RwLock::new(source)),
            stats: Arc::new(Counters::default()),
        };
        register(db, name, binding.clone(), restored)?;
        if name == "node" { for register in node_tables() { register(db, &binding)?; } demand::register(db)?; }
        BOUND.lock().insert((connection_scope(db), name.to_owned()), binding.clone());
        bindings.insert(name.into(), binding);
        Ok(())
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
        BOUND.lock().remove(&(unsafe { db.handle() } as usize, name.to_owned()));
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
        let mut registered = REGISTERED.lock();
        BOUND.lock().retain(|(at,name),binding| {
            let owned = mine.get(name).is_some_and(|owned|Arc::ptr_eq(&owned.current,&binding.current));
            if owned {
                registered.insert((*at, name.clone()), binding.clone());
            }
            !owned
        });
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
}

impl<'a> Db<'a> {
    pub fn of(conn: &'a Connection) -> Self {
        Db { at: connection_scope(conn), conn: Some(conn) }
    }
    /// The scope alone: a job off the connection's thread.
    pub fn detached(self) -> Db<'static> {
        Db { at: self.at, conn: None }
    }
    pub fn at(&self) -> usize {
        self.at
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
    NODE_TABLES.get().map_or(&[], Vec::as_slice)
}

pub fn connection_scope(db: &Connection) -> usize {
    const KEY: &std::ffi::CStr = c"rerail.connection-scope";
    // Each close bumps the generation: a thread's last connection with its hook set (`LAST`) is known to have it
    // until some connection closes (whose address the next one may take), without asking SQLite on every lookup.
    static CLOSES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    thread_local!(static LAST: std::cell::Cell<(usize, u64)> = const { std::cell::Cell::new((0, u64::MAX)) });
    unsafe extern "C" fn closed(at: *mut std::os::raw::c_void) {
        CLOSES.fetch_add(1, Ordering::AcqRel);
        let at = at as usize;
        BOUND.lock().retain(|(h, _), _| *h != at);
        REGISTERED.lock().retain(|(h, _), _| *h != at);
        VIEWS.lock().retain(|(h, _), _| *h != at);
        UNRESOLVED.lock().remove(&at);
        SETUP.lock().remove(&at);
        if let Some(forget) = FORGET.get() {
            (forget.scope)(at);
        }
    }
    let handle = unsafe { db.handle() };
    let closes = CLOSES.load(Ordering::Acquire);
    if LAST.get() == (handle as usize, closes) {
        return handle as usize;
    }
    unsafe {
        if rusqlite::ffi::sqlite3_get_clientdata(handle, KEY.as_ptr()).is_null() {
            rusqlite::ffi::sqlite3_set_clientdata(handle, KEY.as_ptr(), handle.cast(), Some(closed));
        }
    }
    LAST.set((handle as usize, closes));
    handle as usize
}

/// A view of one bound input that a run read in place and did not make in SQL (`plan`): the input, the input's columns
/// it keeps (in the view's order), and its (column, value) filters. A reader after the run reads it from the input.
#[derive(Clone, Debug)]
pub struct UnmadeView {
    pub base: String,
    pub cols: Vec<usize>,
    pub eqs: Vec<(usize, i64)>,
}

/// The tables a kept connection's last run bound, emptied (`reuse`), by connection and name: their modules stay
/// registered until the connection closes, and the next run's `bind` of a table of the same shape takes one back.
static REGISTERED: std::sync::LazyLock<parking_lot::Mutex<HashMap<(usize, String), Binding>>> = std::sync::LazyLock::new(Default::default);

/// Whether `name` is a table of `db`'s connection's own, bound now or kept for a later bind (`REGISTERED`): its
/// `temp` virtual table (a writable one's) is the binding's, not what an earlier run left. (A run binds some of
/// its tables as it goes: the kept strata's `load` hook.)
pub fn is_bound(db: &Connection, name: &str) -> bool {
    let key = (connection_scope(db), name.to_owned());
    BOUND.lock().contains_key(&key) || REGISTERED.lock().contains_key(&key)
}

/// A run dropped `temp.<name>` on `db`'s connection: a kept binding of that name is gone with it.
pub fn dropped_temp(db: &Connection, name: &str) {
    REGISTERED.lock().remove(&(connection_scope(db), name.to_owned()));
}

/// A connection's deferred set-up (`defer_setup`): the statements still to run, each with what undoes it, and what
/// undoes those already run (`reuse`).
#[derive(Default)]
struct Setup {
    pending: Vec<(String, Option<String>)>,
    undo: Vec<String>,
}
/// Set-up statements a connection runs only once SQL is about to run on it (`defer_setup`, `setup_now`).
static SETUP: std::sync::LazyLock<parking_lot::Mutex<HashMap<usize, Setup>>> = std::sync::LazyLock::new(Default::default);

/// Defers `sql` (statements, run as one batch) until SQL first runs on `db`'s connection (`setup_now`): a run whose
/// readers are native runs none of it.
pub fn defer_setup(db: &Connection, sql: &str) {
    SETUP.lock().entry(connection_scope(db)).or_default().pending.push((sql.to_owned(), None));
}

/// `defer_setup` of a run's own set-up on a connection a later run reuses: once run, `undo` undoes it when the
/// connection is reused (`reuse`; `DETACH x` for an `ATTACH`).
pub fn defer_setup_undone(db: &Connection, sql: &str, undo: &str) {
    SETUP.lock().entry(connection_scope(db)).or_default().pending.push((sql.to_owned(), Some(undo.to_owned())));
}

/// Runs `db`'s deferred set-up (`defer_setup`), once: before a statement that needs it (a table made, a table of
/// `x` written).
pub fn setup_now(db: &Connection) -> Result<()> {
    let at = connection_scope(db);
    let pending = std::mem::take(&mut SETUP.lock().entry(at).or_default().pending);
    for (sql, undo) in pending {
        db.execute_batch(&sql)?;
        if let Some(undo) = undo {
            SETUP.lock().entry(at).or_default().undo.push(undo);
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
    let at = connection_scope(db);
    let undo = SETUP.lock().remove(&at).map(|s| s.undo).unwrap_or_default();
    for sql in undo.iter().rev() {
        db.execute_batch(sql)?;
    }
    // SQLite keeps each module's binding until the connection closes: the last run's sources are let go, its tables
    // left empty (a `Modules` may drop while its connection still reads, so this is the connection's next run's).
    // The next run's `bind` takes a table of the same shape back (`REGISTERED`): its module stays registered.
    let mut registered = REGISTERED.lock();
    BOUND.lock().retain(|(h, name), binding| {
        if *h == at {
            registered.insert((at, name.clone()), binding.clone());
        }
        *h != at
    });
    for ((h, _), binding) in registered.iter() {
        if *h == at {
            binding.clear();
        }
    }
    drop(registered);
    VIEWS.lock().retain(|(h, _), _| *h != at);
    if let Some(forget) = FORGET.get() {
        (forget.engine)(at);
    }
    Ok(())
}

/// What the evaluator keeps per connection, dropped with it: `scope` when the connection closes (its symbols,
/// paths and plans with its relations), `engine` when a run reuses it (its relations and components alone).
pub struct Forget {
    pub scope: fn(usize),
    pub engine: fn(usize),
}

static FORGET: std::sync::OnceLock<Forget> = std::sync::OnceLock::new();

/// Registers what the evaluator drops with a connection (once, at load).
pub fn install_forget(forget: Forget) {
    let _ = FORGET.set(forget);
}

/// The tables registered on each connection that no statement has resolved yet (`register`).
static UNRESOLVED: std::sync::LazyLock<parking_lot::Mutex<HashMap<usize, std::collections::BTreeSet<String>>>> = std::sync::LazyLock::new(Default::default);

/// Resolves every table registered on `db` and not yet resolved, by preparing a read of it: an authorizer installed
/// after this does not see SQLite connect them (an authored query's authorizer refuses what it does not name).
pub fn resolve_tables(db: &Connection) -> Result<()> {
    let names = UNRESOLVED.lock().remove(&connection_scope(db)).unwrap_or_default();
    for name in names {
        if BOUND.lock().contains_key(&(connection_scope(db), name.clone())) {
            db.prepare(&format!("SELECT * FROM {}", quote(&name)))?;
        }
    }
    Ok(())
}

static VIEWS: std::sync::LazyLock<parking_lot::Mutex<HashMap<(usize, String), UnmadeView>>> = std::sync::LazyLock::new(Default::default);

/// Records `view` as the view `name` on `db`'s connection, until the connection closes or `forget_views`.
pub(crate) fn note_view(db: &Connection, name: &str, view: UnmadeView) {
    VIEWS.lock().insert((connection_scope(db), name.to_owned()), view);
}

/// Drops the views noted on `db`'s connection: a run starts with none.
pub fn forget_views(db: &Connection) {
    let at = connection_scope(db);
    VIEWS.lock().retain(|(h, _), _| *h != at);
}

/// The rows of the noted view `name` (`note_view`), read from its bound input: each once, as SQL's view gives them
/// (a set), column by column.
pub fn view_columns(db: &Connection, name: &str) -> Option<Vec<Vec<i64>>> {
    let view = VIEWS.lock().get(&(connection_scope(db), name.to_owned())).cloned()?;
    let src = bound_source(db, &view.base)?;
    let mut rows: Vec<Vec<i64>> = src.filter_eq(&view.eqs).into_iter().map(|id| view.cols.iter().map(|&c| src.value(id as usize, c)).collect()).collect();
    rows.sort_unstable();
    rows.dedup();
    let mut columns = vec![Vec::with_capacity(rows.len()); view.cols.len()];
    for row in rows {
        for (c, v) in row.into_iter().enumerate() {
            columns[c].push(v);
        }
    }
    Some(columns)
}

static BOUND: std::sync::LazyLock<parking_lot::Mutex<HashMap<(usize, String), Binding>>> = std::sync::LazyLock::new(Default::default);

/// The source bound now to the table `name` on `db`'s connection, if one is.
pub fn bound_source(db: impl Scoped, name: &str) -> Option<Arc<dyn Source>> {
    BOUND.lock().get(&(db.db().at(), name.to_owned())).map(Binding::current)
}

/// Rebind through the existing cell so its owning Modules and prepared cursors agree.
pub fn replace_bound_source(db:&Connection,name:&str,source:Arc<dyn Source>)->Result<bool>{
    let binding=BOUND.lock().get(&(unsafe{db.handle()} as usize,name.to_owned())).cloned();
    if let Some(binding)=binding{binding.set(source)?;Ok(true)}else{Ok(false)}
}

/// SQL cursor work, by bound input. Snapshot differences attribute fused query work to its leaves.
pub fn counters(db:&Connection)->BTreeMap<String,[u64;4]> {
    let scope=unsafe{db.handle()} as usize;
    BOUND.lock().iter().filter(|((s,_),_)|*s==scope).map(|((_,name),b)|(name.clone(),[b.stats.scans.load(Ordering::Relaxed),b.stats.batches.load(Ordering::Relaxed),b.stats.candidates.load(Ordering::Relaxed),b.stats.columns.load(Ordering::Relaxed)])).collect()
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
    UNRESOLVED.lock().entry(connection_scope(db)).or_default().insert(name.to_owned());
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
        let columns: Vec<usize> = equal.iter().map(|&(_, c)| c).collect();
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
    selected: Option<Vec<usize>>,
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
        crate::code_rule_dsl::sql_profile::frame(crate::code_rule_dsl::sql_profile::FrameEvent::Insert,1);
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
        use crate::code_rule_dsl::sql_profile::{frame,FrameEvent};
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
            let mut columns=Vec::new();
            for (arg,tag) in list.ok_or_else(||failure("missing primary batch columns"))?.chars().enumerate() {
                let mut values=Vec::new();
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
            let columns: Vec<usize> = list.split(',').map(|c| c.parse().map_err(|_| failure("bad index columns"))).collect::<Result<_>>()?;
            let mut values = Vec::new();
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
                Some(Vec::new())
            } else {
                match tag {
                    "c" => {
                        let mut by = equal.clone();
                        by.sort_unstable();
                        self.direct=source.direct_all(&by);
                        if self.direct.is_some() {Some(Vec::new())} else {
                            source.lookup_all(&by).or_else(|| source.lookup(by[0].0, &[by[0].1]).map(|r| r.into_iter().filter(keep).collect()))
                        }
                    }
                    // A probe over a column that spans the documents (a kind), where the document is bound as
                    // well: kept to that document, rather than every document's rows rechecked against it.
                    "p" | "b" => {
                        let probed: Vec<i64> = if tag == "p" { vec![equal[0].1] } else { values.clone() };
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
        let count = self.direct.as_ref().map_or_else(||self.selected.as_ref().map_or(self.range.len(), Vec::len),Vec::len);
        stats.candidates.fetch_add(count as u64, Ordering::Relaxed);
        self.start_stream();
        Ok(())
    }
    fn next(&mut self) -> Result<()> {
        if let Some(stream)=&mut self.stream {self.stream_row=stream.next();}
        crate::code_rule_dsl::sql_profile::frame(crate::code_rule_dsl::sql_profile::FrameEvent::Next,1);
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
        crate::code_rule_dsl::sql_profile::frame(crate::code_rule_dsl::sql_profile::FrameEvent::Column,1);
        self.binding.stats.columns.fetch_add(1, Ordering::Relaxed);
        let value=if let Some(row)=&self.stream_row {row.cells.cell(col as usize)} else if let Some(rows)=&self.direct {rows[self.position].cells.cell(col as usize)} else if self.cache_rows {self.with_cells(|_,cells|cells.cell(col as usize))} else {self.source.cell(self.row(),col as usize)};
        ctx.set_result(&value)
    }
    fn rowid(&self) -> Result<i64> {
        Ok(if let Some(row)=&self.stream_row {row.id} else if let Some(rows)=&self.direct {rows[self.position].id} else if self.cache_rows {self.with_cells(|id,_|id)} else {self.source.row_id(self.row())})
    }
}

/// Row-major integer cells with every column indexed: each column's rows sorted by value, built on
/// first use. Producers' compact relations (S `relation.rs`) and small derived relations.
pub struct Relation {
    columns: Vec<String>,
    cells: Vec<i64>,
    indexes: Box<[OnceLock<ColumnIndex>]>,
}
struct ColumnIndex {
    rows: Vec<u32>,
    distinct: usize,
}
impl Relation {
    pub fn new(columns: &[&str]) -> Self {
        assert!(!columns.is_empty());
        Self {
            columns: columns.iter().map(|c| (*c).to_owned()).collect(),
            cells: Vec::new(),
            indexes: columns.iter().map(|_| OnceLock::new()).collect(),
        }
    }
    pub fn push(&mut self, row: &[i64]) {
        assert_eq!(row.len(), self.columns.len());
        assert!(self.len() < u32::MAX as usize);
        self.cells.extend_from_slice(row);
        self.indexes.iter_mut().for_each(|i| *i = OnceLock::new());
    }
    fn index(&self, column: usize) -> &ColumnIndex {
        self.indexes[column].get_or_init(|| {
            let mut rows: Vec<u32> = (0..self.len() as u32).collect();
            rows.sort_by_key(|&r| self.value(r as usize, column));
            let distinct = 1 + rows.windows(2).filter(|w| self.value(w[0] as usize, column) != self.value(w[1] as usize, column)).count();
            ColumnIndex { rows, distinct }
        })
    }
    /// The positions in `column`'s index of the rows with values in `lo..=hi`.
    fn between(&self, column: usize, lo: i64, hi: i64) -> Range<usize> {
        let rows = &self.index(column).rows;
        let start = rows.partition_point(|&r| self.value(r as usize, column) < lo);
        let end = rows.partition_point(|&r| self.value(r as usize, column) <= hi);
        start..end.max(start)
    }
}
impl Source for Relation {
    fn revision(&self)->Option<u64> {Some(0)}
    fn columns(&self) -> Vec<String> {
        self.columns.clone()
    }
    fn len(&self) -> usize {
        self.cells.len() / self.columns.len()
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        let one = self.len().div_ceil(self.index(column).distinct.max(1));
        if batch { one * 8 } else { one }
    }
    fn indexed(&self, _: usize) -> bool {
        true
    }
    fn range_indexed(&self, _: usize) -> bool {
        true
    }
    fn range_lookup(&self, column: usize, lower: Option<(i64, bool)>, upper: Option<(i64, bool)>) -> Option<Vec<usize>> {
        let lo = match lower {
            None => i64::MIN,
            Some((v, true)) => v,
            Some((v, false)) => match v.checked_add(1) {
                Some(v) => v,
                None => return Some(Vec::new()),
            },
        };
        let hi = match upper {
            None => i64::MAX,
            Some((v, true)) => v,
            Some((v, false)) => match v.checked_sub(1) {
                Some(v) => v,
                None => return Some(Vec::new()),
            },
        };
        let rows = &self.index(column).rows;
        Some(rows[self.between(column, lo, hi)].iter().map(|&r| r as usize).collect())
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>> {
        #[cfg(any(test, feature = "test-hooks"))]
        if let Some(rows) = lookup_spike::selected(self, column, values) { return Some(rows); }
        let rows = &self.index(column).rows;
        Some(values.iter().flat_map(|&v| rows[self.between(column, v, v)].iter().map(|&r| r as usize)).collect())
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        self.cells[row * self.columns.len() + column]
    }
}

/// S's `query/plan/relation.rs` as copied (0689eb25); merged into this module (decision 0008).
#[allow(dead_code)]
pub mod s_relation;
// cratify: `p_table` is rerail-w's (it mounts it)
// cratify: `tests` is rerail-logic's (it mounts it)

#[cfg(any(test, feature = "test-hooks"))]
pub mod lookup_spike;
