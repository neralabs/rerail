//! A native semi-naive fixpoint for one recursive component of set-valued relations.
//!
//! `set_fixpoint_open(spec)` reads the relations' current rows (their seeds, which SQL has inserted) and the edge
//! tables. `set_fixpoint_run(name)` runs the waves and inserts the new tuples into the relations' tables.
//! `set_fixpoint_close(name)` frees the component. Between open and close, `set_rel` virtual tables expose each
//! relation's tuples (`full`) or the previous wave's new tuples (`delta`), with a hash index on every column.
//!
//! A rule is one of two kinds:
//! - **Edge.** Linear (one recursive atom), with its non-recursive rest evaluated once by SQL into an edge table
//!   keyed by the variables the rest shares with the recursive atom. A wave joins the atom's delta with the edges
//!   natively.
//! - **Probe.** Anything else: several recursive atoms, or a rest too costly to evaluate on its own (a
//!   demand-guarded rule's rest is the undemanded relation). Its SQL runs once per wave for each recursive atom
//!   whose delta is non-empty, reading that atom from the `delta` vtab and the others from `full`. So the joins
//!   stay set-at-a-time in SQL, and the native side does the deduplication, the delta bookkeeping and the
//!   scheduling. There is no DDL per round, and a statement whose delta is empty does not run.
//!
//! The spec has one item per line, tokens separated by spaces. Symbols and SQL are hex, so they can hold anything:
//!   name <component name>
//!   rel <table> <types> [empty]                types: one char per column, `i` (int or node) or `s` (symbol)
//!   rule <head rel> <src rel> <edge table|-> <edge types|-> key <slot..> ; ecols <slot..> ; pat <tok..> ; head <tok..>
//!   probe <head rel> <delta rel> <hex SQL>     the SQL's columns are the head's
//!   seed <head rel> <hex join>                 a native join run once, in the first wave
//!   join <head rel> <delta rel> <hex join> [<hex join>]   a native probe; the second, if given, runs instead in the
//!                                              wave whose delta is the whole relation (any order, read in full)
//!   request <rel>                              a relation whose new rows the caller answers: a wave that adds to it
//!                                              ends the run (done 2) before any gate opens (`pending`, `add`)
//!   produced <rel>                             a relation only the caller adds to (`add`); not written
//!   gate <done rel> <src rel|-> <quiet rels|-> <gates|->   after a wave in which no quiet relation changed and the
//!                                              listed (earlier) gates have nothing left to pass, the src rows not yet
//!                                              passed join done as its delta (`-`: one row, 0); comma lists
//! Rels are numbered in order. A pattern token is `_`, `v<slot>` (binds the slot, or tests it if already bound),
//! `i:<int>` or `x:<hex symbol>`. The edge table's columns are the key slots, then the ecols slots. A head token is
//! `v<slot>`, `i:<int>`, `x:<hex>` or `cat:<slot>:<slot>` (`@dot` of the symbols: joined by ".", `""` first gives the second).
//! `set_fixpoint_run(name[, max_waves])` returns "new_rows waves visits probe_runs probe_skips solve_ms total_ms
//! per_probe done": per_probe is a comma-separated "ms:rows" for each probe line in spec order, and done is 1 at the
//! fixpoint (new rows are written then), 0 when max_waves stopped it. `set_fixpoint_stats(name)` and
//! `set_fixpoint_reprobe(name, i, hex SQL)` let the caller replan probes between bounded runs.
pub mod scalars;
pub mod liveness;
/// The index-cost spike: column orders as sorted runs of row ids (`RERAIL_INDEX_SPIKE`).
pub mod ordered;
/// A relation's indexes (decision 0112): sorted orders of row ids for the key sets probes bind, one run a wave.
pub mod runs;
use runs::Index;
/// A column's statistics in a fixed size (decision 0112): no index is kept for them.
pub mod sketch;
use sketch::Sketch;
pub mod rowcode;
use crate::code_rule_dsl::logic::analysis::scalars::Kind as ScalarKind;
use rusqlite::ffi;
use rusqlite::types::{Value, ValueRef};
use rusqlite::vtab::{Context as VContext, CreateVTab, Filters, IndexConstraintOp, IndexInfo, VTab, VTabConnection, VTabCursor, VTabKind};
use rusqlite::{Connection, Error, Result};
use crate::code_rule_dsl::facts::vtab::Scoped;
use std::borrow::Cow;
use hashbrown::HashTable;
use rustc_hash::{FxHashMap as HashMap, FxHasher};
use std::ffi::{CStr, CString};
use std::hash::Hasher;
use std::os::raw::c_int;
use std::sync::{Arc, LazyLock, Mutex, RwLock};
use std::time::Instant;

/// Strata captured and run again on their own (decision 0095, step 0).
pub mod lab;
pub mod spec;
#[cfg(test)]
mod survivor_tests;
use spec::{ComponentSpec, InputKey, JoinItem, JoinSpec, Line, Reprobe, Source, Start, Tok};
pub mod closure;
pub mod table;
pub mod level;
// cratify: `device` is rerail-device-metal's (it mounts it)
pub use crate::code_rule_dsl::logic::device::{Report as DeviceReport, take_report as device_report};
use crate::code_rule_dsl::logic::device::{JoinPlan, solver};

fn err(m: impl Into<String>) -> Error { Error::UserFunctionError(m.into().into()) }

/// Symbols are the store's atoms (`content_store::atoms`, C12), so a symbol column holds atom ids and joins
/// directly with fact families' atom columns. T numbered symbols per process and stored them as text.
/// A run interns the symbols it creates (`cat`) in the process's arena of the database, the one the store uses
/// (decision 0049): they are pending until a write transaction or a due flush appends them, and SQL sees them
/// through the `atoms` virtual table meanwhile. `buf`: scratch for `cat`, so joining two symbols allocates only when
/// the result is new. `failed`: the first interning that failed (the atoms lease not obtained), returned by `check`
/// at the run's next boundary; its symbol is -1, which matches nothing.
pub struct Interner { atoms: Arc<crate::content_store::atoms::Atoms>, buf: String, failed: Option<String> }
impl Interner {
    /// A loader's scratch cursor over the same atom arena. Loading SQL may call sym_text/sym_atom,
    /// so it must not hold the registered cursor's lock across SQLite execution.
    fn for_loading(shared: &RwLock<Self>) -> Self {
        Self { atoms: Arc::clone(&shared.read().unwrap().atoms), buf: String::new(), failed: None }
    }
    fn id(&mut self, s: &str) -> i64 {
        match self.atoms.intern(s) {
            Ok(atom) => i64::from(atom),
            Err(e) => {
                self.failed.get_or_insert_with(|| e.to_string());
                -1
            }
        }
    }
    /// The atom arena the symbols are in (the device interns the texts it makes into it).
    pub fn atoms(&self) -> &Arc<crate::content_store::atoms::Atoms> { &self.atoms }
    /// The atom of `s` in this process's arena (`symbols` read the database's atoms into it).
    fn get(&self, s: &str) -> Option<i64> { self.atoms.get(s).map(i64::from) }
    /// Atom `v`'s text, reading the database's atoms written since `symbols` when `v` is past the arena; "" if there
    /// is no such atom or they could not be read (a join step has no error to return: it matches as it did before).
    pub fn name(&self, v: i64) -> String {
        let Ok(atom) = crate::content_store::atoms::Sym::try_from(v) else { return String::new() };
        self.atoms.with_str(atom, str::to_owned).or_else(|| self.atoms.value(atom).ok().flatten()).unwrap_or_default()
    }
    /// Whether `v` is an atom of the database, read as `name` reads it.
    fn known(&self, v: i64) -> Result<bool> {
        let Ok(atom) = crate::content_store::atoms::Sym::try_from(v) else { return Ok(false) };
        Ok(self.atoms.with_str(atom, |_| ()).is_some() || self.atoms.value(atom).map_err(|e| err(e.to_string()))?.is_some())
    }
    /// The first interning that failed since the last check.
    fn check(&mut self) -> Result<()> {
        match self.failed.take() { Some(e) => Err(err(e)), None => Ok(()) }
    }
}

/// The process's symbols of each database, kept from run to run with their arena: a run need not read a large
/// store's atoms again. They hold no file open: the store's last handle closes them (`ContentStore`'s drop).
/// `RERAIL_LEVELS_ON_CYCLES`: a leveled component keeps levels only on its relations on a cycle (`open`).
static LEVELS_ON_CYCLES: LazyLock<bool> = LazyLock::new(|| std::env::var_os("RERAIL_LEVELS_ON_CYCLES").is_some());
static SYMS: LazyLock<Mutex<HashMap<String, Arc<RwLock<Interner>>>>> = LazyLock::new(Default::default);
/// The symbols of `conn`'s database: the process's arena of it.
fn symbols(conn: impl Scoped) -> Result<Arc<RwLock<Interner>>> {
    let conn = conn.db();
    // A connection's symbols are found once (by its path) and kept for it until it closes (`forget_scope`).
    let at = conn.at();
    if let Some(s) = SCOPE_SYMS.lock().unwrap().get(&at) {
        return Ok(Arc::clone(s));
    }
    let s = symbols_by_path(conn)?;
    SCOPE_SYMS.lock().unwrap().insert(at, Arc::clone(&s));
    Ok(s)
}
/// The path of `conn`'s main database (`""` in memory), found once per connection and kept until it closes
/// (`forget_scope`): SQLite's `sqlite3_db_filename` takes the connection's mutex at every call.
fn conn_path(conn: impl Scoped) -> Arc<str> {
    let conn = conn.db();
    let at = conn.at();
    if let Some(p) = SCOPE_PATHS.lock().unwrap().get(&at) {
        return Arc::clone(p);
    }
    // Off the run's thread (a strand, decision 0113) the path is not known unless the run's thread found it
    // (`prime`): not kept, so the run's own lookup finds it.
    let Ok(sql) = conn.sql() else { return Arc::from("") };
    let p: Arc<str> = Arc::from(sql.path().unwrap_or_default());
    SCOPE_PATHS.lock().unwrap().insert(at, Arc::clone(&p));
    p
}

/// What a stratum off the run's thread reads of its connection, found here first (decision 0113): its path and its
/// symbols, kept by its scope.
pub fn prime(conn: &Connection) -> Result<Arc<crate::content_store::atoms::Atoms>> {
    conn_path(conn);
    let syms = symbols(conn)?;
    let arena = Arc::clone(&syms.read().unwrap().atoms);
    Ok(arena)
}
static SCOPE_PATHS: std::sync::LazyLock<Mutex<HashMap<usize, Arc<str>>>> = std::sync::LazyLock::new(Default::default);
/// Each live connection's symbols (`symbols`), by its scope.
static SCOPE_SYMS: std::sync::LazyLock<Mutex<HashMap<usize, Arc<RwLock<Interner>>>>> = std::sync::LazyLock::new(Default::default);
fn symbols_by_path(conn: impl Scoped) -> Result<Arc<RwLock<Interner>>> {
    let conn = conn.db();
    let key = conn_path(conn).to_string();
    let mut syms = SYMS.lock().unwrap();
    let atoms = crate::content_store::atoms::Atoms::for_connection(conn.sql()?);
    // A store created again at the same path has new atoms (`Atoms::for_database`, checked by `Atoms::attach`).
    if let Some(s) = syms.get(&key) && Arc::ptr_eq(&s.read().unwrap().atoms, &atoms) {
        return Ok(Arc::clone(s));
    }
    // An arena made after the store was written holds none of its atoms yet: the run's SQL and joins read them.
    atoms.read_stored().map_err(|e| err(e.to_string()))?;
    let s = Arc::new(RwLock::new(Interner { atoms, buf: String::new(), failed: None }));
    syms.insert(key, Arc::clone(&s));
    Ok(s)
}

/// Intern a whole producer result without SQLite callbacks or a lock held across SQL.
pub fn intern_batch<'a>(conn: &Connection, texts: impl IntoIterator<Item = &'a str>) -> Result<Vec<i64>> {
    let shared = symbols(conn)?;
    let mut cursor = Interner::for_loading(&shared);
    let ids = texts.into_iter().map(|text| cursor.id(text)).collect();
    cursor.check()?;
    Ok(ids)
}

/// `sym_text(a)` read from `conn`'s symbols in memory, without a statement: the text of a request's atom, answered in a
/// wave (decision 0104).
pub fn sym_text_of(conn: &Connection, a: i64) -> Result<String> {
    let syms = symbols(conn)?;
    let s = syms.read().unwrap();
    if !s.known(a)? {
        return Err(err(format!("no symbol {a}")));
    }
    Ok(s.name(a))
}

/// `sym_text(a)`, `sym_atom(text)` and `sym_cat(a, b)` (`@dot`: `a.b`, `"" . b` is `b`) over `conn`'s symbols: the planner's SQL holds symbols
/// as atoms (decision 0009), and these are the only places it reads or makes their text. NULL gives NULL. Also
/// registers `atoms` on `conn` (decision 0049; its opener registered the store's commit hook, `Atoms::install`).
pub fn register_symbols(conn: &Connection) -> Result<()> {
    use rusqlite::functions::FunctionFlags;
    crate::content_store::atoms::Atoms::attach(conn)?;
    let det = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC;
    let syms = symbols(conn)?;
    let s = Arc::clone(&syms);
    conn.create_scalar_function(c"sym_text", 1, det, move |ctx| {
        let Some(a) = ctx.get::<Option<i64>>(0)? else { return Ok(None) };
        let s = s.read().unwrap();
        if !s.known(a)? {
            return Err(err(format!("no symbol {a}")));
        }
        Ok(Some(s.name(a)))
    })?;
    let s = Arc::clone(&syms);
    conn.create_scalar_function(c"sym_atom", 1, det, move |ctx| {
        let Some(t) = ctx.get::<Option<String>>(0)? else { return Ok(None) };
        let mut s = s.write().unwrap();
        let id = s.id(&t);
        s.check()?;
        Ok(Some(id))
    })?;
    conn.create_scalar_function(c"sym_cat", 2, det, move |ctx| {
        let (Some(a), Some(b)) = (ctx.get::<Option<i64>>(0)?, ctx.get::<Option<i64>>(1)?) else { return Ok(None) };
        let mut s = syms.write().unwrap();
        for v in [a, b] {
            if !s.known(v)? {
                return Err(err(format!("no symbol {v}")));
            }
        }
        let id = cat(&mut s, a, b);
        s.check()?;
        Ok(Some(id))
    })?;
    Ok(())
}

/// Decode a composed output through the shared atom arena, without a SQL query per row.
pub fn symbol_text(conn: &Connection, atom: i64) -> Result<String> {
    let symbols=symbols(conn)?;
    let s=symbols.read().unwrap();
    if !s.known(atom)? {return Err(err(format!("no symbol {atom}")))}
    Ok(s.name(atom).to_owned())
}

/// Whether a maintained component's first wave is its input changes' seeds, not its kept rows (`plan::reads_change`,
/// decision 0122 §2): the default; `RERAIL_WAVE0=kept` makes the kept rows the first wave's delta, as before (a
/// measurement aid). Read at each stratum planned, so a test can set it; the spec carries the choice (`KeptOld`).
pub fn wave0_changes() -> bool {
    !std::env::var("RERAIL_WAVE0").is_ok_and(|v| v == "kept")
}

/// Appends the symbols pending in `conn`'s database if a flush is due (decision 0049 §4.2): a run's end.
pub fn flush_symbols(conn: &Connection) -> Result<()> {
    crate::content_store::atoms::Atoms::for_connection(conn).flush_if_due(conn).map_err(|e| err(e.to_string()))
}

enum Pat { Any, Slot(usize), Int(i64), Sym(i64) }
pub enum Term { Slot(usize), Const(i64), Cat(usize, usize) }

fn hash_vals(vals: impl Iterator<Item = i64>) -> u64 {
    let mut h = FxHasher::default();
    for v in vals { h.write_i64(v); }
    h.finish()
}

/// `RERAIL_INDEX_REPORT` (the index-cost spike): every probe of a relation, by its name and the key columns it binds
/// (`+delta` for a probe of the delta alone), and the planner's reads of its per-column indexes (`[stats]`,
/// `[postings]`, `[lead]`). Measurement only: nothing reads it but the report.
pub mod probed {
    use std::collections::HashMap;
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    static PROBES: std::sync::Mutex<Option<HashMap<(String, Vec<usize>), u64>>> = std::sync::Mutex::new(None);
    #[inline]
    pub(crate) fn on() -> bool {
        *ON.get_or_init(|| std::env::var_os("RERAIL_INDEX_REPORT").is_some())
    }
    #[inline]
    pub(crate) fn note(rel: &str, cols: impl Iterator<Item = usize>) {
        if on() {
            add(rel, cols.collect());
        }
    }
    #[cold]
    fn add(rel: &str, mut cols: Vec<usize>) {
        cols.sort_unstable();
        cols.dedup();
        let mut p = PROBES.lock().unwrap_or_else(|e| e.into_inner());
        *p.get_or_insert_with(HashMap::new).entry((rel.to_owned(), cols)).or_insert(0) += 1;
    }
    /// The probes since the last `take`.
    pub(crate) fn take() -> HashMap<(String, Vec<usize>), u64> {
        PROBES.lock().unwrap_or_else(|e| e.into_inner()).take().unwrap_or_default()
    }
}

/// Whether row `id` is dead in bitset `dead` (a relation started from a kept one, its deleted rows marked, not
/// removed: decision 0090 step 4). Empty: none is.
#[inline]
pub fn is_dead(dead: &[u64], id: u32) -> bool {
    !dead.is_empty() && dead.get(id as usize >> 6).is_some_and(|w| w >> (id & 63) & 1 == 1)
}
fn kill(dead: &mut Vec<u64>, id: u32) -> bool {
    let w = id as usize >> 6;
    if dead.len() <= w {
        dead.resize(w + 1, 0);
    }
    let was = dead[w] >> (id & 63) & 1 == 1;
    dead[w] |= 1 << (id & 63);
    !was
}
/// A relation's tuples in insertion order, flat (`k` columns each); `lo..hi` is the previous wave's delta, the rows
/// added in wave `wave`. `idx`: its orders (decision 0112), sorted row ids, one for each chain of key sets its probes
/// bind (`want`); the first holds every column and is the dedup test. Each is a list of runs: a wave's rows become one
/// run when its delta is set (`set_delta`), so rows `indexed..n`, pushed since, are in none yet: `pending` holds them
/// for the dedup test until then. `sketches`: each column's statistics (`Sketch`), over rows `..indexed`.
/// `dead`: rows deleted but kept in place (`based`), which every reader skips; `ndead` of them.
/// `level`: each row's level (decision 0090, levels for recursive strata): `lbase` plus the wave that pushed it, or
/// the kept row's level for a row started from a kept relation. A row's level is above the levels of the rows it was
/// first derived from, as they were pushed in earlier waves; `lbase` is above every kept row's level in the
/// component, so a maintained run's new rows are above the rows they read.
pub struct Rel { table: String, types: Vec<u8>, pub k: usize, pub n: usize, data: Vec<i64>, stored: usize, lo: usize, hi: usize,
             wave: u32, idx: Vec<Index>, serves: Vec<Vec<Vec<usize>>>, indexed: usize, pending: HashTable<u32>, sketches: Vec<Sketch>,
             dead: Vec<u64>, ndead: usize, summaries: scalars::Pool,
             /// The kept relation it started as (`InRel::delta_since`).
             from: Option<Origin>,
             /// Each row's level (decision 0090: `lbase` plus the wave that pushed it), while every row has one.
             level: Vec<u32>, lbase: u32,
             /// Whether its rows get levels: only a relation of a recursive stratum reads them (`open`), and at 4
             /// bytes a row they are not held for the others.
             leveled: bool,
             /// Its statistics kept as its rows come (`kept_stats`), the planner's form of them kept by delta
             /// (decision 0096): rows `..indexed` counted in, and those `stats_dead` marks counted out.
             stats: Option<KeptStats>, stats_dead: Vec<u64> }

/// The id of a live row equal to `row` in the relation of `orders` (its first holds every column), not `skip`.
fn find_row(orders: &[Index], data: &[i64], k: usize, row: &[i64], skip: impl Fn(u32) -> bool) -> Option<u32> {
    let o = orders.first()?;
    let mut key = [0i64; MAX_KEY];
    let key = permuted(&o.cols, |c| row[c], &mut key);
    o.find(data, k, key, None).parts().flatten().copied().find(|&id| !skip(id) && &data[id as usize * k..][..k] == row)
}

/// The widest key an order is probed with inline; a relation of more columns is not indexed (`Rel::new`).
const MAX_KEY: usize = 32;

/// The values of `cols`, in order, from `value` (by column), into `out`.
#[inline]
fn permuted<'a>(cols: &[usize], value: impl Fn(usize) -> i64, out: &'a mut [i64; MAX_KEY]) -> &'a [i64] {
    for (j, &c) in cols.iter().enumerate() {
        out[j] = value(c);
    }
    &out[..cols.len()]
}

/// The columns of a chain of key sets (each a subset of the next) as an order: each set's new columns in turn, then
/// the rest of `k`'s.
fn chained(sets: &[Vec<usize>], k: usize) -> Vec<usize> {
    let mut cols: Vec<usize> = Vec::with_capacity(k);
    let mut by: Vec<&Vec<usize>> = sets.iter().collect();
    by.sort_by_key(|s| s.len());
    for set in by {
        cols.extend(set.iter().filter(|c| !cols.contains(c)).copied().collect::<Vec<_>>());
    }
    cols.extend((0..k).filter(|c| !cols.contains(c)).collect::<Vec<_>>());
    cols
}

/// The kept relation (by address) a maintained relation started as, and how its rows follow from that one's.
#[derive(Clone)]
enum Origin {
    /// Its first `n` rows are the old relation's (`Rel::based`), less those it marked dead; it added the rest.
    Based { at: usize, n: usize },
    /// Made from the old relation's survivors (too many dead to keep in place): its first `kept` rows, then those
    /// it added; `removed`, the old rows it left out, row-major.
    Compacted { at: usize, old_n: usize, kept: usize, removed: Vec<i64> },
}
impl Rel {
    fn new(table: &str, types: &[u8]) -> Rel {
        let k = types.len();
        assert!(k <= MAX_KEY, "{table}: {k} columns, past the {MAX_KEY} an order is probed with");
        Rel { table: table.to_string(), types: types.to_vec(), k, n: 0, data: Vec::new(), stored: 0, lo: 0, hi: 0, wave: 0,
              idx: vec![Index::new((0..k).collect())], serves: vec![Vec::new()], indexed: 0, pending: HashTable::new(),
              sketches: vec![Sketch::default(); k], dead: Vec::new(), ndead: 0, summaries: Default::default(), from: None,
              level: Vec::new(), lbase: 0, leveled: true, stats: None, stats_dead: Vec::new() }
    }
    /// A relation that starts as `old` (a kept one: its rows copied as memory; its orders' runs and its sketches
    /// shared, not built again) less the rows of `gone`, marked dead in place (decision 0090 step 4). Its rows are
    /// stamped the first wave's delta, as the survivors rule it replaced derived them (b3d4958c3); a component whose
    /// spec says `KeptOld` (the default, decision 0122 §2) stamps them no wave's. `None` when `old` has no orders to start from,
    /// or when too many of its rows would be dead (then it is made from its rows, compacted). No `gone`: all of
    /// `old`'s rows.
    fn based(table: &str, types: &[u8], old: &InRel, gone: Option<&InRel>) -> Option<Rel> {
        let none = InRel::empty(old.k);
        let gone = gone.unwrap_or(&none);
        let k = old.k;
        if old.view.is_some() || k != types.len() || gone.k != k || old.chains.first().is_none_or(|o| o.cols.len() != k || o.rows() != old.n)
            || old.sketches.len() != k {
            return None;
        }
        let data = &old.rows;
        let mut dead = old.dead.clone();
        let mut ndead = old.ndead;
        let mut row = vec![0i64; k];
        for g in 0..gone.n {
            if is_dead(&gone.dead, g as u32) {
                continue;
            }
            let r = gone.at(g);
            for (c, v) in row.iter_mut().enumerate() {
                *v = r.at(c);
            }
            if let Some(id) = find_row(&old.chains, data, k, &row, |id| is_dead(&dead, id)) && kill(&mut dead, id) {
                ndead += 1;
            }
        }
        if ndead * 8 > old.n.max(1) {
            return None;
        }
        let mut idx: Vec<Index> = old.chains.clone();
        if idx.iter().any(|ix| ix.rows() != old.n) {
            return None;
        }
        // Every row is the first wave's (`open` makes it the delta): its runs are stamped so.
        for ix in &mut idx {
            ix.restamp(1);
        }
        let serves = if old.serves.len() == idx.len() { old.serves.clone() } else { idx.iter().map(|_| Vec::new()).collect() };
        let level = if old.has_levels() { old.level.clone() } else { Vec::new() };
        let from = Some(Origin::Based { at: old as *const InRel as usize, n: old.n });
        Some(Rel { table: table.to_string(), types: types.to_vec(), k, n: old.n, data: data.clone(), stored: 0, lo: 0, hi: 0, wave: 0, idx, serves,
                   indexed: old.n, pending: HashTable::new(), sketches: old.sketches.clone(), dead, ndead, summaries: Default::default(), from, level,
                   lbase: 0, leveled: true, stats: old.stats.clone(), stats_dead: old.dead.clone() })
    }
    /// `based`'s relation, made from `old`'s rows less those of `gone` rather than in place (compacted).
    fn compacted(table: &str, types: &[u8], old: &InRel, gone: Option<&InRel>) -> Rel {
        let mut rel = Rel::new(table, types);
        let k = rel.k;
        let mut gone_rows: rustc_hash::FxHashSet<Vec<i64>> = Default::default();
        if let Some(gone) = gone {
            for id in 0..gone.len() { if !is_dead(&gone.dead, id as u32) { let r = gone.at(id); gone_rows.insert((0..k).map(|c| r.at(c)).collect()); } }
        }
        // Each row keeps its kept level; without levels, none are recorded (`Rel::push`).
        let levels = old.has_levels();
        if !levels { rel.level = vec![0; 1]; }
        let mut row = vec![0i64; k];
        let mut removed = Vec::new();
        for id in 0..old.len() {
            if is_dead(&old.dead, id as u32) { continue; }
            let r = old.at(id);
            for (c, v) in row.iter_mut().enumerate() { *v = r.at(c); }
            if gone_rows.contains(&row) { removed.extend_from_slice(&row); } else if rel.push(&row) && levels { *rel.level.last_mut().expect("a level pushed") = old.level[id]; }
        }
        if !levels { rel.level.clear(); }
        if old.view.is_none() && old.k == k {
            rel.from = Some(Origin::Compacted { at: old as *const InRel as usize, old_n: old.n, kept: rel.n, removed });
        }
        rel
    }
    #[inline]
    fn dead(&self, id: u32) -> bool {
        is_dead(&self.dead, id)
    }
    fn row(&self, id: usize) -> &[i64] { &self.data[id * self.k..][..self.k] }
    /// A live row equal to `row`, not one of `skip`: in the first order, or pushed since the last wave (`pending`).
    fn find(&self, row: &[i64], skip: impl Fn(u32) -> bool) -> Option<u32> {
        let (data, k) = (&self.data, self.k);
        if let Some(&id) = self.pending.find(hash_vals(row.iter().copied()), |&id| !skip(id) && &data[id as usize * k..][..k] == row) {
            return Some(id);
        }
        find_row(&self.idx, data, k, row, skip)
    }
    pub fn contains(&self, row: &[i64]) -> bool {
        self.find(row, |id| is_dead(&self.dead, id)).is_some()
    }
    /// Probes will bind the key set `keys` (columns): an order whose leading columns are that set answers them. An
    /// order whose key sets are a chain with it is reordered to answer it too (re-sorted, as rarely as a probe is
    /// planned); otherwise a new order is made, 4 bytes a row.
    fn want(&mut self, keys: &[usize]) {
        let mut set = keys.to_vec();
        set.sort_unstable();
        set.dedup();
        if set.is_empty() || set.len() > self.k {
            return;
        }
        // Answered already: the order keeps answering it (a later reorder keeps every set it serves a prefix).
        if let Some(i) = self.idx.iter().position(|ix| ix.answers(&set)) {
            if !self.serves[i].contains(&set) { self.serves[i].push(set); }
            return;
        }
        let chain = |sets: &[Vec<usize>]| sets.iter().all(|t| t.iter().all(|c| set.contains(c)) || set.iter().all(|c| t.contains(c)));
        let (data, k) = (&self.data, self.k);
        if let Some(i) = (0..self.idx.len()).find(|&i| chain(&self.serves[i])) {
            self.serves[i].push(set);
            let cols = chained(&self.serves[i], k);
            charge(Unit::IndexEntry, self.idx[i].rows() as u64);
            self.idx[i].reorder(data, k, cols);
        } else {
            let cols = chained(std::slice::from_ref(&set), k);
            let mut ix = Index::new(cols);
            charge(Unit::IndexEntry, self.indexed as u64);
            // The rows already in runs: those before the delta as of wave 0, the delta's as of its wave; the rest are
            // indexed with the others' at the next `set_delta`.
            let lo = self.lo.min(self.indexed);
            ix.add_run(data, k, 0, lo, 0);
            ix.add_run(data, k, lo, self.indexed, self.wave);
            self.idx.push(ix);
            self.serves.push(vec![set]);
        }
    }
    /// Room for `n` more rows, so a wave's rows are not copied as the rows grow (the pack's joins spent 18 s of 155 s
    /// growing tables). The orders take a wave's rows at once (`set_delta`).
    fn reserve(&mut self, n: usize) {
        if n < 64 {
            return;
        }
        let k = self.k;
        self.data.reserve(n * k);
        let data = &self.data;
        self.pending.reserve(n, |&i| hash_vals(data[i as usize * k..][..k].iter().copied()));
    }
    /// Adds a row (of the next wave, its delta) unless it is there.
    fn push(&mut self, row: &[i64]) -> bool {
        // A dead row derived again is a new row (its old id stays dead).
        if self.contains(row) { return false; }
        let k = self.k;
        let id = self.n as u32;
        self.data.extend_from_slice(row);
        // A relation started from a kept one without levels has none (`has_levels` then says so).
        if self.leveled && self.level.len() == self.n {
            self.level.push(self.lbase + self.wave + 1);
        }
        self.n += 1;
        let data = &self.data;
        self.pending.insert_unique(hash_vals(row.iter().copied()), id, |&i| hash_vals(data[i as usize * k..][..k].iter().copied()));
        true
    }
    /// A wave's derived rows added as the next wave's, each once, unless held: checked as one sorted batch
    /// (`Fresh::settle`), so the dedup test reads each of the first order's runs once, in order, not row by row
    /// (decision 0112). `pend`: the rows stay in no order past this wave's end (a gate's relation, its delta set
    /// later), so they are also put in `pending` for the dedup tests until then. Whether any was new.
    fn push_fresh(&mut self, mut f: Fresh, pend: bool) -> bool {
        f.settle(self);
        let (k, m) = (self.k, f.n);
        if m == 0 {
            return false;
        }
        let from = self.n;
        self.data.extend_from_slice(&f.data[..m * k]);
        for _ in 0..m {
            if self.leveled && self.level.len() == self.n {
                self.level.push(self.lbase + self.wave + 1);
            }
            self.n += 1;
        }
        if pend {
            let data = &self.data;
            for id in from..self.n {
                self.pending.insert_unique(hash_vals(data[id * k..][..k].iter().copied()), id as u32, |&i| hash_vals(data[i as usize * k..][..k].iter().copied()));
            }
        }
        true
    }
    /// Rows `indexed..n` counted in the sketches; `pending` emptied (they are in the orders now).
    fn indexed_to_n(&mut self) {
        for id in self.indexed..self.n {
            for (c, sk) in self.sketches.iter_mut().enumerate() {
                sk.add(self.data[id * self.k + c]);
            }
        }
        if let Some(stats) = &mut self.stats {
            stats.0.delta(&self.data[self.indexed * self.k..self.n * self.k], &[], self.k);
        }
        self.indexed = self.n;
        self.pending = HashTable::new();
    }
    /// The rows dead now and not when the statistics last counted them out taken out of them, and those revived
    /// (a level's rows dead while checked, `Levels`) put back: a word of the dead marks at a time, so a relation with
    /// none changed costs its marks' words.
    fn count_dead(&mut self) {
        let Some(stats) = &mut self.stats else { return };
        let (k, words) = (self.k, self.dead.len().max(self.stats_dead.len()));
        let (mut plus, mut minus) = (Vec::new(), Vec::new());
        for w in 0..words {
            let (now, was) = (self.dead.get(w).copied().unwrap_or(0), self.stats_dead.get(w).copied().unwrap_or(0));
            let mut changed = now ^ was;
            while changed != 0 {
                let id = w * 64 + changed.trailing_zeros() as usize;
                changed &= changed - 1;
                if id < self.indexed {
                    let row = &self.data[id * k..(id + 1) * k];
                    if now >> (id % 64) & 1 == 1 { minus.extend_from_slice(row) } else { plus.extend_from_slice(row) }
                }
            }
        }
        if !plus.is_empty() || !minus.is_empty() {
            stats.0.delta(&plus, &minus, k);
        }
        self.stats_dead.clone_from(&self.dead);
    }
    /// The next wave: its delta is rows `lo..hi`. The rows pushed since the last wave become each order's runs: those
    /// before `lo` as of the wave ending, the rest as the new wave's (its delta). First, each order's older runs merge
    /// if its probes paid for it (`Index::settle`).
    fn set_delta(&mut self, lo: usize, hi: usize) {
        let w = self.wave + 1;
        let (from, n, k) = (self.indexed, self.n, self.k);
        let split = lo.clamp(from, n);
        for ix in &mut self.idx {
            ix.settle(&self.data, k, w);
            ix.add_run(&self.data, k, from, split, self.wave);
            ix.add_run(&self.data, k, split, n, w);
        }
        self.indexed_to_n();
        self.lo = lo;
        self.hi = hi;
        self.wave = w;
    }
    /// The rows pushed since the last wave as runs of no wave's delta (a relation frozen, `InRel::moved`), then at
    /// most `FROZEN_RUNS` runs an order: its probes cannot merge them later.
    fn index_rest(&mut self) {
        let (from, n, k) = (self.indexed, self.n, self.k);
        for ix in &mut self.idx {
            ix.settle(&self.data, k, u32::MAX);
            ix.add_run(&self.data, k, from, n, 0);
            ix.tier(&self.data, k, FROZEN_RUNS);
        }
        self.indexed_to_n();
    }
    /// The rows whose columns `keys` hold the values `key` (in that order), in the delta only or in all:
    /// (candidates, count). The order answering the most of them leads (each is checked by the caller); with none,
    /// every row of the delta or of all. Rows pushed since the last wave (in no order yet; none on the engine's own
    /// path, which probes after `set_delta`) are candidates of all, as they are of no delta.
    /// `probe`, each run of the order searched from where this sequence of probes last was in it (`hints`).
    fn probe_at(&self, keys: &[usize], key: &[i64], delta: bool, hints: &mut [u32; runs::HINTS]) -> (Cands<'_>, usize) {
        let Some((ix, m)) = best_order(&self.idx, keys) else { return self.probe(keys, key, delta) };
        let mut buf = [0i64; MAX_KEY];
        let at = |c: usize| key[keys.iter().position(|&x| x == c).expect("a key column")];
        let found = ix.find_at(&self.data, self.k, permuted(&ix.cols[..m], at, &mut buf), delta.then_some(self.wave), hints);
        let n = found.len();
        if !delta && self.indexed < self.n {
            return (Cands::Pending(found, self.indexed as u32, self.n as u32), n + self.n - self.indexed);
        }
        (Cands::Runs(found), n)
    }
    fn probe(&self, keys: &[usize], key: &[i64], delta: bool) -> (Cands<'_>, usize) {
        let Some((ix, m)) = best_order(&self.idx, keys) else {
            let (lo, hi) = if delta { (self.lo, self.hi) } else { (0, self.n) };
            return (Cands::Range(lo as u32, hi as u32), hi - lo);
        };
        let mut buf = [0i64; MAX_KEY];
        let at = |c: usize| key[keys.iter().position(|&x| x == c).expect("a key column")];
        let found = ix.find(&self.data, self.k, permuted(&ix.cols[..m], at, &mut buf), delta.then_some(self.wave));
        let n = found.len();
        if !delta && self.indexed < self.n {
            return (Cands::Pending(found, self.indexed as u32, self.n as u32), n + self.n - self.indexed);
        }
        (Cands::Runs(found), n)
    }
}

/// The order of `orders` whose leading columns hold the most of the columns `keys` (all of them leading), with how
/// many: `None` when none leads with one of them.
fn best_order<'a>(orders: &'a [Index], keys: &[usize]) -> Option<(&'a Index, usize)> {
    orders.iter().map(|ix| (ix, ix.cols.iter().take_while(|c| keys.contains(c)).count())).filter(|&(_, m)| m > 0).max_by_key(|&(_, m)| m)
}

/// A frozen relation's runs an index at most (`Rel::index_rest`): one, as its probes cannot merge it later, and
/// each probe of a frozen order otherwise read every run (O125: 9 G instructions of the cold run's 346 G cheaper
/// than four, and 3 G of step 01's 181 G).
const FROZEN_RUNS: usize = 1;

/// Candidate row ids: a range, a list, or a key's ranges of an index's runs.
#[derive(Clone)]
enum Cands<'a> { Range(u32, u32), Slice(&'a [u32]), Owned(Vec<u32>), Runs(runs::Found<'a>), Pending(runs::Found<'a>, u32, u32) }
impl Cands<'_> {
    /// Calls `f` on each id until it returns true; returns whether one did.
    fn any(self, mut f: impl FnMut(u32) -> bool) -> bool {
        match self {
            Cands::Range(lo, hi) => (lo..hi).any(f),
            Cands::Slice(ids) => ids.iter().any(|&id| f(id)),
            Cands::Owned(ids) => ids.into_iter().any(f),
            Cands::Runs(found) => found.parts().any(|ids| ids.iter().any(|&id| f(id))),
            Cands::Pending(found, lo, hi) => found.parts().any(|ids| ids.iter().any(|&id| f(id))) || (lo..hi).any(f),
        }
    }
}

/// The tuples of one relation derived in a wave, flat, not yet checked against it (decision 0112: the dedup test a
/// batch at a time). Rows `..settled` are distinct, not held by the relation, and sorted by its first order; the rest
/// are as derived. `emitted`: every row a rule derived this wave, new or not (`RERAIL_SPIKE_DERIVATIONS`: the
/// derivations a counting scheme would count).
#[derive(Clone, Default)]
pub struct Fresh { data: Vec<i64>, n: usize, emitted: u64, settled: usize }

/// Rows a wave's derived tuples hold before they are checked against their relation (`Fresh::offer`): a wave that
/// derives many rows already held keeps at most about this many more.
const FRESH_BATCH: usize = 1 << 16;

/// `RERAIL_SPIKE_DERIVATIONS`: rows derived (new or already held) and rows held new, by relation, over the process.
pub static DERIVATIONS: std::sync::Mutex<std::collections::BTreeMap<String, (u64, u64)>> = std::sync::Mutex::new(std::collections::BTreeMap::new());
impl Fresh {
    pub fn add(&mut self, row: &[i64]) { self.data.extend_from_slice(row); self.n += 1; }
    /// A row a rule derived for `rel`: kept unchecked, the rows past `settled` checked a batch at a time.
    #[inline]
    fn offer(&mut self, row: &[i64], rel: &Rel) {
        self.add(row);
        if self.n - self.settled >= FRESH_BATCH.max(self.settled) {
            self.settle(rel);
        }
    }
    /// Drops the rows `rel` holds and the repeats: the rest, distinct, sorted by `rel`'s first order (so a wave's rows
    /// are checked against it as one sorted batch, each run searched once in order, not row by row).
    fn settle(&mut self, rel: &Rel) {
        let (k, n, settled) = (rel.k, self.n, self.settled);
        if settled == n {
            return;
        }
        // The first order's sort (`runs::cmp_vals`: tagged, by the leading value's tag first, then the columns).
        let cols = &rel.idx[0].cols;
        let tagged = runs::tagged(cols, k);
        let data = &self.data;
        let cmp = |a: u32, b: u32| -> std::cmp::Ordering {
            runs::cmp_vals(cols, tagged, &data[a as usize * k..][..k], &data[b as usize * k..][..k])
        };
        let mut order: Vec<u32> = (settled as u32..n as u32).collect();
        runs::sort_keyed(&mut order, tagged, cols.len(), |q, i| data[q as usize * k + cols[i]], &cmp);
        order.dedup_by(|a, b| cmp(*a, *b).is_eq());
        // The new rows' check: rows pushed since the last wave (in no order yet), then the first order.
        let mut held = vec![false; order.len()];
        if rel.indexed < rel.n {
            for (h, &q) in held.iter_mut().zip(&order) {
                *h = rel.contains(&data[q as usize * k..][..k]);
            }
        }
        rel.idx[0].held_sorted(&rel.data, k, data, &order, |id| !rel.dead(id), &mut held);
        // Merged with the settled rows (sorted, distinct, not held), less those the new rows repeat.
        let mut out: Vec<i64> = Vec::with_capacity((settled + order.len()) * k);
        let (mut s, mut kept) = (0, 0);
        for (&q, h) in order.iter().zip(held) {
            while s < settled && cmp(s as u32, q).is_lt() {
                out.extend_from_slice(&data[s * k..][..k]);
                (s, kept) = (s + 1, kept + 1);
            }
            if !h && !(s < settled && cmp(s as u32, q).is_eq()) {
                out.extend_from_slice(&data[q as usize * k..][..k]);
                kept += 1;
            }
        }
        out.extend_from_slice(&data[s * k..settled * k]);
        self.n = kept + settled - s;
        self.settled = self.n;
        self.data = out;
    }
}
struct EdgeRule { head: usize, src: usize, key: Vec<usize>, ecols: Vec<usize>, pat: Vec<Pat>, out: Vec<Term>,
                  edges: HashMap<Vec<i64>, Vec<Vec<i64>>>, has_edge: bool, nslots: usize }
/// A probe's body: SQL over the `set_rel` vtabs, or a native join pipeline (`join.rs` spec).
enum Body { Sql(String), Join(JoinRule) }
/// `whole`: a join probe's plan for the wave whose delta is its whole relation (the relation was empty before it), in
/// any literal order, reading that relation in full. `wkey`: its spec, the same for each probe of one rule: in a wave
/// the store does not change, so a whole plan run once derives what each of its rule's probes would (it is not run
/// again).
struct ProbeRule { head: usize, delta: usize, body: Body, whole: Option<JoinRule>, wkey: Option<Arc<JoinSpec>> }

pub struct Store { pub rels: Vec<Rel>, syms: Arc<RwLock<Interner>> }
/// `seeded`: the seed joins have run (in the first wave). `write`: at the fixpoint, insert the new rows into the
/// relations' tables (else they stay native, in `DERIVED`, until `set_fixpoint_materialize`).
/// `requests`: relations whose new rows the caller answers (`pending`, `add`), each with the rows answered so far.
/// `produced`: relations only the caller adds to, which the fixpoint does not write.
/// `device`: the seed joins' device plans, by probe, when the spec asked for the device and it takes every one of them
/// (`device::compile`); `seed_specs`, their specs, to parse them again with the CPU's indexes when the device refuses
/// one at run time.
struct Component { store: Arc<RwLock<Store>>, edges: Vec<EdgeRule>, probes: Vec<ProbeRule>, gates: Vec<Gate>, seeded: bool, write: bool, done: bool,
                   requests: Vec<(usize, usize)>, produced: Vec<usize>, closure: Option<closure::Closure>, levels: Levels,
                   /// Request rows the caller held back (`defer`): asked again once the run is otherwise quiet. Until
                   /// then they are as rows not yet asked (they keep no gate shut).
                   deferred: Vec<(usize, Vec<Vec<i64>>)>,
                   /// The run paused quiet with rows held back: `pending` gives them again.
                   presenting: bool,
                   /// A closure whose launches wait in this thread's device wave (`lanes::wave`), with their tickets.
                   waiting: Option<(closure::Closure, closure::Prepared, Vec<u64>)>,
                   device: Option<Vec<(usize, JoinPlan)>>,
                   /// Device joins waiting in this thread's wave (`submit`): their ticket.
                   device_wait: Option<u64>,
                   seed_specs: Vec<(usize, Arc<JoinSpec>)> }

/// Decision 0090's levels for a recursive stratum: which of its kept rows are gone, decided in level order. Per
/// relation `h` of the stratum, a line names five relations of the component:
/// - `cand`: rows of `h` that lost a derivation (the rules: an old derivation with a premise gone);
/// - `chk`: the candidates of the level being checked, which this driver writes;
/// - `held`: those of them with a derivation from rows below them (the rules, reading `live`);
/// - `gone`: those without, which this driver writes;
/// - `live`: `h`'s kept rows, each with its level, of which a rule reads only those below the level checked and not
///   gone: every other row is dead while it is checked.
///
/// Each time the component is quiet, the checked level's candidates not held go to `gone` (whose rules make more
/// candidates, above them), then the lowest level with candidates is checked. A candidate at or below a level
/// already checked lost no derivation from below (its premises from below were decided already): it stays.
#[derive(Default)]
struct Levels { lines: Vec<LevelLine>, cur: Option<u32>, last: Option<u32>, started: bool,
                heap: std::collections::BinaryHeap<std::cmp::Reverse<(u32, usize, u32)>> }
struct LevelLine { cand: usize, chk: usize, held: usize, gone: usize, live: usize,
                   /// Candidate rows seen (a prefix of `cand`), those checked or passed, and those of the level checked.
                   seen: usize, decided: Vec<bool>, batch: Vec<u32>,
                   /// `live`'s rows by level, and how many of them are shown; its rows gone.
                   order: Vec<u32>, shown: usize, gone_rows: Vec<u64>,
                   /// `live`'s rows dead before it was checked (deleted in an earlier run, a later copy live): never
                   /// shown, and never a candidate's row.
                   was_dead: Vec<u64> }

fn revive(dead: &mut [u64], id: u32) -> bool {
    let w = id as usize >> 6;
    let Some(word) = dead.get_mut(w) else { return false };
    let was = *word >> (id & 63) & 1 == 1;
    *word &= !(1 << (id & 63));
    was
}

impl Rel {
    /// A row's id, hidden or not, but not one of `was_dead` (a copy deleted before: the row's live copy, if any,
    /// is a later one, with its own level).
    fn id_of(&self, row: &[i64], was_dead: &[u64]) -> Option<u32> {
        self.find(row, |id| is_dead(was_dead, id))
    }
}

/// `RERAIL_LEVELS_TRACE=<value>`: the candidates holding the value, and how each is decided.
fn levels_traced(row: &[i64]) -> bool {
    static V: std::sync::LazyLock<Option<i64>> = std::sync::LazyLock::new(|| std::env::var("RERAIL_LEVELS_TRACE").ok().and_then(|v| v.parse().ok()));
    V.is_some_and(|v| row.contains(&v))
}

impl Levels {
    /// Every `live` row dead until its level is below the one checked.
    fn start(&mut self, store: &mut Store) {
        self.started = true;
        for ln in &mut self.lines {
            let live = &mut store.rels[ln.live];
            ln.was_dead = live.dead.clone();
            let mut order: Vec<u32> = (0..live.n as u32).filter(|&id| !is_dead(&ln.was_dead, id)).collect();
            if live.level.len() == live.n {
                order.sort_by_key(|&i| live.level[i as usize]);
            }
            for id in 0..live.n as u32 {
                if kill(&mut live.dead, id) { live.ndead += 1; }
            }
            ln.order = order;
        }
    }
    /// One step at a quiet point: whether it gave the component more to derive.
    fn step(&mut self, store: &mut Store) -> bool {
        if self.lines.is_empty() { return false; }
        if !self.started { self.start(store); }
        if let Some(l) = self.cur.take() {
            let mut any = false;
            for ln in &mut self.lines {
                let lo = store.rels[ln.gone].n;
                for &id in &ln.batch {
                    let row = store.rels[ln.cand].row(id as usize).to_vec();
                    if levels_traced(&row) { eprintln!("RERAIL_LEVELS_TRACE {} {row:?} at level {l}: {}", store.rels[ln.cand].table, if store.rels[ln.held].contains(&row) { "held" } else { "gone" }); }
                    if store.rels[ln.held].contains(&row) { continue; }
                    any |= store.rels[ln.gone].push(&row);
                    if let Some(lid) = store.rels[ln.live].id_of(&row, &ln.was_dead) { kill(&mut ln.gone_rows, lid); }
                }
                ln.batch.clear();
                let hi = store.rels[ln.gone].n;
                if hi > lo { store.rels[ln.gone].set_delta(lo, hi); }
            }
            self.last = Some(l);
            if any { return true; }
        }
        // New candidates, by their kept level.
        for (li, ln) in self.lines.iter_mut().enumerate() {
            let (cand, live) = (&store.rels[ln.cand], &store.rels[ln.live]);
            ln.decided.resize(cand.n, false);
            for id in ln.seen..cand.n {
                let level = live.id_of(cand.row(id), &ln.was_dead).filter(|_| live.level.len() == live.n).map(|lid| live.level[lid as usize]);
                if levels_traced(cand.row(id)) { eprintln!("RERAIL_LEVELS_TRACE {} {:?}: candidate, level {level:?}, last {:?}", cand.table, cand.row(id), self.last); }
                match level {
                    Some(l) if self.last.is_none_or(|last| l > last) => self.heap.push(std::cmp::Reverse((l, li, id as u32))),
                    _ => ln.decided[id] = true,
                }
            }
            ln.seen = cand.n;
        }
        let Some(&std::cmp::Reverse((l, _, _))) = self.heap.peek() else { return false };
        // The rows below `l` not gone are shown; the level's candidates are checked.
        for ln in &mut self.lines {
            let live = &mut store.rels[ln.live];
            while ln.shown < ln.order.len() && live.level[ln.order[ln.shown] as usize] < l {
                let id = ln.order[ln.shown];
                if !is_dead(&ln.gone_rows, id) && revive(&mut live.dead, id) { live.ndead -= 1; }
                ln.shown += 1;
            }
        }
        let mut los: Vec<usize> = self.lines.iter().map(|ln| store.rels[ln.chk].n).collect();
        while let Some(&std::cmp::Reverse((l2, li, id))) = self.heap.peek() {
            if l2 != l { break; }
            self.heap.pop();
            let ln = &mut self.lines[li];
            if ln.decided[id as usize] { continue; }
            ln.decided[id as usize] = true;
            let row = store.rels[ln.cand].row(id as usize).to_vec();
            store.rels[ln.chk].push(&row);
            ln.batch.push(id);
        }
        for (ln, lo) in self.lines.iter().zip(los.drain(..)) {
            let hi = store.rels[ln.chk].n;
            if hi > lo { store.rels[ln.chk].set_delta(lo, hi); }
        }
        self.cur = Some(l);
        true
    }
}
/// A gated negation's `done` relation, fed from `src` (its demand) once `quiet` is: `passed` src rows so far.
struct Gate { done: usize, src: Option<usize>, quiet: Vec<usize>, after: Vec<usize>, passed: usize }

/// The connection a component or a derived relation belongs to (its handle): two runs in one process (two corpora,
/// or tests in parallel) name their components and relations alike, and must not see each other's.
///
/// A handle is an address, and a closed connection's address is given to the next one opened: what a connection
/// left here is dropped when it closes (`sqlite3_set_clientdata`'s destructor runs in `sqlite3_close`), so a new
/// connection at the same address starts with none of it. Without this a second run over the same store read the
/// first run's fixpoint as its own (a cone's loaded documents counted as the workspace's).
pub fn scope(conn: &Connection) -> usize {
    crate::code_rule_dsl::facts::vtab::connection_scope(conn)
}

/// Drops what the connection at `at` left: its components and derived relations (`scope`).
pub fn forget_scope(at: usize) {
    if let Ok(mut s) = SCOPE_SYMS.lock() {
        s.remove(&at);
    }
    if let Ok(mut p) = SCOPE_PATHS.lock() {
        p.remove(&at);
    }
    crate::code_rule_dsl::logic::plan::forget_connection(at);
    forget_engine(at);
}

/// Drops the engine's relations and components a run left on the connection at `at` (when it closes, or when the
/// next run reuses it, `vtab::reuse`). They are freed on a thread of their own: no run's work.
pub fn forget_engine(at: usize) {
    let mut gone: (Vec<Arc<Mutex<Component>>>, Vec<Arc<InRel>>) = Default::default();
    if let Ok(mut c) = COMPONENTS.lock() {
        c.retain(|((s, _), comp)| *s != at || { gone.0.push(Arc::clone(comp)); false });
    }
    if let Ok(mut d) = DERIVED.lock() {
        d.retain(|(s, _), rel| *s != at || { gone.1.push(Arc::clone(rel)); false });
    }
    if let Ok(mut a) = ADOPTED.lock() {
        a.retain(|(s, _)| *s != at);
    }
    if !gone.0.is_empty() || !gone.1.is_empty() {
        rerail_accel::exec::drop_beside(gone);
    }
}
static COMPONENTS: Mutex<Vec<((usize, String), Arc<Mutex<Component>>)>> = Mutex::new(Vec::new());
fn component(conn: impl Scoped, name: &str) -> Result<Arc<Mutex<Component>>> {
    let conn = conn.db();
    component_in(conn.at(), name)
}
fn component_in(scope: usize, name: &str) -> Result<Arc<Mutex<Component>>> {
    COMPONENTS.lock().unwrap().iter().find(|((s, n), _)| *s == scope && n == name).map(|c| c.1.clone()).ok_or_else(|| err(format!("no set fixpoint named {name:?}")))
}

fn unhex(s: &str) -> Result<String> { spec::unhex(s).map_err(err) }

fn read_value(v: ValueRef<'_>, ty: u8, syms: &mut Interner) -> Result<i64> {
    match (v, ty) {
        (ValueRef::Integer(i), b'i') => Ok(i),
        (ValueRef::Text(t), b's') => Ok(syms.id(std::str::from_utf8(t).map_err(|e| err(e.to_string()))?)),
        (ValueRef::Integer(i), b's') => { let _ = syms; Ok(i) }
        (other, _) => Err(err(format!("unexpected value {other:?} for a column of type {}", ty as char))),
    }
}

use crate::code_rule_dsl::logic::profile::{Unit, work as charge};

fn read_rows(conn: impl Scoped, table: &str, types: &[u8], syms: &mut Interner) -> Result<Vec<Vec<i64>>> {
    let conn = conn.db();
    let cols: Vec<String> = (0..types.len()).map(|i| format!("c{i}")).collect();
    read_sql(conn, &format!("SELECT {} FROM {}", cols.join(", "), crate::code_rule_dsl::logic::sql::table(table)), types, syms).map_err(|e| err(format!("{table}: {e}")))
}

fn read_sql(conn: impl Scoped, sql: &str, types: &[u8], syms: &mut Interner) -> Result<Vec<Vec<i64>>> {
    let conn = conn.db();
    let table = sql;
    let mut st = conn.sql()?.prepare(sql)?;
    let mut rows = st.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let mut row = Vec::with_capacity(types.len());
        for (i, &t) in types.iter().enumerate() { row.push(read_value(r.get_ref(i)?, t, syms).map_err(|e| err(format!("{table}: {e}")))?); }
        out.push(row);
    }
    drop(rows);
    // The roofline's work: the statement's VM steps, and each row entered in a component relation or edge map.
    charge(Unit::VmStep, st.get_status(rusqlite::StatementStatus::VmStep) as u64);
    charge(Unit::IndexEntry, out.len() as u64);
    Ok(out)
}


/// Whether an environment setting is on, read once per process (these are asked per index and per join).
macro_rules! env_set {
    ($name:literal) => {{
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ON.get_or_init(|| std::env::var_os($name).is_some())
    }};
}

/// Opens a component from its spec's text (the SQL function's form, `set_fixpoint_open`).
pub fn open(conn: &Connection, spec: &str) -> Result<String> {
    open_spec(conn, &ComponentSpec::parse(spec).map_err(err)?)
}

/// Opens a component from its spec: the planner's, in process, or one read from text.
pub fn open_spec(conn: impl Scoped, spec: &ComponentSpec) -> Result<String> {
    let conn = conn.db();
    let symbols = symbols(conn)?;
    let mut syms = Interner::for_loading(&symbols);
    let (mut rels, mut edges, mut probes, mut gates) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let name = spec.name.clone();
    let mut write = true;
    let (mut requests, mut produced) = (Vec::new(), Vec::new());
    let mut levels = Levels::default();
    let joins = |j: &JoinSpec, syms: &mut Interner| -> Result<JoinRule> {
        let _j = crate::code_rule_dsl::logic::profile::span(|| "open: joins".into());
        join_rule(conn, j, syms)
    };
    // `device`: the placement put the stratum on the device (decision 0095 §7); its seeds are parsed without the
    // CPU's indexes, and compiled for the device below.
    let mut device_wanted = false;
    // The delta joins' specs, by probe, for the device's plans of a recursive component (their rules' keys).
    let mut delta_specs: Vec<(usize, String)> = Vec::new();
    let mut leveled = false;
    // The relations whose statistics are kept as their rows come (`Line::Stats`).
    let mut counted: Vec<usize> = Vec::new();
    let mut cyclic: Vec<usize> = Vec::new();
    let mut kept_old = false;
    let mut seed_specs: Vec<(usize, Arc<JoinSpec>)> = Vec::new();
    // Each relation's kept rows (`Start::Base`), by index: under `wave0_changes` they are no wave's delta.
    let mut kept_n: Vec<usize> = Vec::new();
    for line in &spec.lines {
        match line {
            Line::Rel { name: table, types, start } => {
                let mut rel = Rel::new(table, types.as_bytes());
                // `Empty`: the table was made this run and nothing wrote it (no statement). `From`: its rows are a
                // query's (an input's, read where it is).
                // `Base`: the relation starts as a kept native relation less another's rows (the survivors of a
                // maintained stratum, `h__old` less `h__del`), dead in place; else, made from those rows. `seeded`:
                // rows a SQL seed or the program's facts wrote to the table, added to it.
                let what: &'static str = match start { Start::Base { .. } => "open: survivors", Start::From(_) => "open: rows from SQL", Start::Rows => "open: rows from its table", Start::Empty => "open: empty" };
                let _rows = crate::code_rule_dsl::logic::profile::span(move || what.into());
                match start {
                    Start::Base { old, gone, seeded } => {
                        let (o, g) = { let d = DERIVED.lock().unwrap(); (d.get(&(conn.at(), old.clone())).cloned(), if gone == "-" { None } else { d.get(&(conn.at(), gone.clone())).cloned() }) };
                        // `-`: no rows gone (a copy of the kept relation, its levels with it).
                        let o = o.ok_or_else(|| err(format!("rel: no native relation {old}")))?;
                        // The rows gone, from the stratum that made them: a native relation, or a table where a SQL
                        // stratum wrote them (an overdelete reading an input's delta); they are the delta's size.
                        let g = match g {
                            Some(g) => Some(g),
                            None if gone == "-" => None,
                            None => Some(InRel::flat(&read_rows(conn, gone, types.as_bytes(), &mut syms)?, types.as_bytes().to_vec())),
                        };
                        match Rel::based(table, types.as_bytes(), &o, g.as_deref()) {
                            Some(b) => rel = b,
                            None => rel = Rel::compacted(table, types.as_bytes(), &o, g.as_deref()),
                        }
                        kept_n.resize(rels.len(), 0);
                        kept_n.push(rel.n);
                        if *seeded {
                            for row in read_rows(conn, &rel.table, &rel.types.clone(), &mut syms)? { rel.push(&row); }
                        }
                    }
                    Start::From(sql) => {
                        let rows = match bound_rows(conn, sql, rel.types.as_slice(), &mut syms)? { Some(rows) => rows, None => read_sql(conn, sql, &rel.types.clone(), &mut syms)? };
                        rel.reserve(rows.len());
                        for row in rows { rel.push(&row); }
                    }
                    Start::Rows => {
                        let rows = read_rows(conn, &rel.table, &rel.types.clone(), &mut syms)?;
                        rel.reserve(rows.len());
                        for row in rows { rel.push(&row); }
                    }
                    Start::Empty => {}
                }
                // A relation started from a kept one has none of its rows in its table (made this run).
                rel.stored = if matches!(start, Start::Base { .. }) { 0 } else { rel.n };
                rels.push(rel);
            }
            Line::Probe { head, delta, sql } => probes.push(ProbeRule { head: *head, delta: *delta, body: Body::Sql(sql.clone()), whole: None, wkey: None }),
            Line::Device => device_wanted = solver().is_some(),
            Line::Leveled(rels) => { leveled = true; cyclic.extend(rels.iter().copied()); }
            Line::KeptOld => kept_old = true,
            Line::Seed { head, join } => {
                let j = { let _j = crate::code_rule_dsl::logic::profile::span(|| "open: joins".into()); join_rule_with(conn, join, &mut syms, !device_wanted)? };
                if device_wanted { seed_specs.push((probes.len(), join.clone())); }
                probes.push(ProbeRule { head: *head, delta: SEED, body: Body::Join(j), whole: None, wkey: None });
            }
            Line::NoWrite => write = false,
            Line::Request(r) => requests.push((*r, 0)),
            Line::Produced(r) => produced.push(*r),
            Line::Stats(r) => counted.push(*r),
            Line::Gate { done, src, quiet, after } => gates.push(Gate { done: *done, src: *src, quiet: quiet.clone(), after: after.clone(), passed: 0 }),
            Line::Join { head, delta, join, whole } => {
                let w = match whole { Some(w) => Some(joins(w, &mut syms)?), None => None };
                if device_wanted { delta_specs.push((probes.len(), join.to_text())); }
                probes.push(ProbeRule { head: *head, delta: *delta, body: Body::Join(joins(join, &mut syms)?), whole: w, wkey: whole.clone() });
            }
            Line::Levels { cand, chk, held, gone, live } => levels.lines.push(LevelLine { cand: *cand, chk: *chk, held: *held, gone: *gone, live: *live,
                                                       seen: 0, decided: Vec::new(), batch: Vec::new(), order: Vec::new(), shown: 0, gone_rows: Vec::new(), was_dead: Vec::new() }),
            Line::Edge(e) => {
                let mut pat = Vec::new();
                for t in &e.pat {
                    pat.push(match t { Tok::Wild => Pat::Any, Tok::Int(i) => Pat::Int(*i), Tok::Sym(x) => Pat::Sym(syms.id(x)), Tok::Slot(s) => Pat::Slot(*s) });
                }
                let mut out = Vec::new();
                for t in &e.out {
                    out.push(match t {
                        spec::HeadTok::Tok(Tok::Int(i)) => Term::Const(*i),
                        spec::HeadTok::Tok(Tok::Sym(x)) => Term::Const(syms.id(x)),
                        spec::HeadTok::Tok(Tok::Slot(s)) => Term::Slot(*s),
                        spec::HeadTok::Tok(Tok::Wild) => return Err(err("bad slot _")),
                        spec::HeadTok::Cat(a, b) => Term::Cat(*a, *b),
                    });
                }
                let (key, ecols) = (e.key.clone(), e.ecols.clone());
                let nslots = key.iter().chain(&ecols).copied().chain(pat.iter().filter_map(|p| if let Pat::Slot(s) = p { Some(*s) } else { None }))
                    .max().map_or(0, |m| m + 1);
                let mut map: HashMap<Vec<i64>, Vec<Vec<i64>>> = HashMap::default();
                if let Some((table, types)) = &e.edge {
                    for row in read_rows(conn, table, types.as_bytes(), &mut syms)? {
                        let (k, v) = row.split_at(key.len());
                        map.entry(k.to_vec()).or_default().push(v.to_vec());
                    }
                }
                edges.push(EdgeRule { head: e.head, src: e.src, key, ecols, pat, out, edges: map, has_edge: e.edge.is_some(), nslots });
            }
        }
    }
    // The device's plans of the seeds, or why it takes none of them: then they are parsed again with the CPU's indexes.
    let device_plans = if let Some(dev) = solver().filter(|_| device_wanted) {
        let _plan = crate::code_rule_dsl::logic::profile::span(|| "open: device plans".into());
        let recursive = probes.iter().any(|p| p.delta != SEED);
        let refused = if !edges.is_empty() || !gates.is_empty() || !requests.is_empty() || !produced.is_empty() {
            Some("a component with gates, requests or edge rules".to_owned())
        } else if recursive && (leveled || !levels.lines.is_empty()) {
            Some("a recursive component that keeps levels (its maintenance runs on the CPU)".to_owned())
        } else if recursive && probes.iter().any(|p| p.delta != SEED && matches!(p.body, Body::Sql(_))) {
            Some("a recursive component with a SQL probe".to_owned())
        } else {
            None
        };
        let plans = match refused {
            Some(r) => Err(r),
            None => seed_specs.iter().map(|(pi, js)| (*pi, js.to_text())).chain(delta_specs.iter().cloned()).map(|(pi, text)| match &probes[pi].body {
                Body::Join(j) => dev.compile_join(j, &text).map(|p| (pi, p)),
                Body::Sql(_) => Err("a SQL seed".to_owned()),
            }).collect::<std::result::Result<Vec<_>, String>>(),
        };
        match plans {
            Ok(p) if !p.is_empty() && dev.join_available() => Some(p),
            other => {
                let why = match other { Err(e) => e, Ok(p) if p.is_empty() => "no seed join".to_owned(), Ok(_) => "no Metal device".to_owned() };
                dev.refuse(why);
                for (pi, js) in &seed_specs {
                    probes[*pi].body = Body::Join(join_rule(conn, js, &mut syms)?);
                }
                seed_specs.clear();
                None
            }
        }
    } else {
        None
    };
    let _multi = crate::code_rule_dsl::logic::profile::span(|| "open: indexes".into());
    // Levels only where they are read: in a component that keeps them (`leveled`, a session's), a relation on a
    // cycle of the component's rules (a recursive stratum's); one of a levels line, or one started from a kept
    // relation with levels. The others hold none.
    {
        let n = rels.len();
        let mut next: Vec<Vec<usize>> = vec![Vec::new(); n];
        // A seed probe (`SEED`) reads no relation of the component.
        for p in probes.iter().filter(|p| p.delta < n) { next[p.delta].push(p.head); }
        for e in edges.iter().filter(|e| e.src < n) { next[e.src].push(e.head); }
        let mut keep = vec![false; n];
        for ln in &levels.lines { for r in [ln.cand, ln.chk, ln.held, ln.gone, ln.live] { keep[r] = true; } }
        // A relation that reaches itself through the stratum's rules as written keeps levels though this run left out
        // the rules that close its cycle (they joined an empty relation): the next run may run them, and a session
        // maintains a stratum by levels only when every relation of it has them.
        for &r in cyclic.iter().filter(|&&r| r < n && leveled) { keep[r] = true; }
        for r in 0..n {
            if keep[r] || (rels[r].n > 0 && rels[r].level.len() == rels[r].n) { keep[r] = true; continue; }
            if !leveled { continue; }
            // In a component that keeps levels, every relation keeps them: a session maintains a recursive stratum by
            // levels only when every relation of it has them (`maintain`), and a relation of the stratum that its
            // rules here leave off every cycle (`garg`, `gcall` once `gpar`'s rules joined empty relations) held
            // none, so the whole stratum (174 relations, O125 `acls`) was overdeleted and derived again instead. A
            // row's wave orders its supports whether or not a cycle reads it. `RERAIL_LEVELS_ON_CYCLES` keeps them
            // only on cycles, as before.
            if !*LEVELS_ON_CYCLES { keep[r] = true; continue; }
            // Whether `r` reaches itself.
            let mut seen = vec![false; n];
            let mut stack = next[r].clone();
            while let Some(x) = stack.pop() {
                if x == r { keep[r] = true; break; }
                if !std::mem::replace(&mut seen[x], true) { stack.extend(next[x].iter().copied()); }
            }
        }
        for (r, keep) in rels.iter_mut().zip(keep) {
            if !keep { r.leveled = false; r.level = Vec::new(); }
        }
    }
    // Rows this run pushes go above every kept row's level in the component (`Rel::level`).
    let lbase = rels.iter().filter(|r| r.level.len() == r.n).flat_map(|r| r.level.iter().copied()).max().unwrap_or(0);
    for r in &mut rels { r.lbase = lbase; }
    // Counted from their first rows (a relation started from a kept one has its statistics already).
    for &r in &counted {
        if let Some(rel) = rels.get_mut(r).filter(|rel| rel.stats.is_none() && rel.indexed == 0) {
            rel.stats = Some(KeptStats::of(rel.k));
        }
    }
    // The first wave reads the seeds as its delta; under `KeptOld`, not a relation's kept rows (`Start::Base`): their
    // orders' runs (`Rel::based` stamps them the first wave's) are of no wave, so a keyed delta read skips them too.
    for (i, r) in rels.iter_mut().enumerate() {
        let n = r.n;
        let kept = if kept_old { kept_n.get(i).copied().unwrap_or(0) } else { 0 };
        if kept > 0 {
            for ix in &mut r.idx {
                ix.restamp(0);
            }
        }
        r.set_delta(kept, n);
    }
    // Levels (decision 0090): `live`'s rows are shown a level at a time, never a delta, so no probe reads it as
    // one; the other levels relations get small batches from row 0, which a whole plan (one for a delta that is
    // its whole relation) would join by scanning `live` or `old`.
    for ln in &levels.lines {
        let n = rels[ln.live].n;
        rels[ln.live].set_delta(n, n);
        for p in probes.iter_mut().filter(|p| [ln.cand, ln.chk, ln.held, ln.gone].contains(&p.delta)) {
            p.whole = None;
            p.wkey = None;
        }
    }
    for p in &probes {
        if let Body::Join(j) = &p.body { for (r, cols) in &j.multi { rels[*r].want(cols); } }
        if let Some(j) = &p.whole { for (r, cols) in &j.multi { rels[*r].want(cols); } }
    }
    syms.check()?;
    drop(syms);
    let store = Arc::new(RwLock::new(Store { rels, syms: symbols }));
    let mut reg = COMPONENTS.lock().unwrap();
    let key = (conn.at(), name.clone());
    reg.retain(|(k, _)| *k != key);
    let mut comp = Component { store, edges, probes, gates, seeded: false, write, done: false, requests, produced, closure: None, waiting: None, device_wait: None,
                               device: device_plans, seed_specs, levels, deferred: Vec::new(), presenting: false };
    // A stratum placed on the device runs there whole: a recursive one as the device's rounds (`fixpoint`), not as
    // a closure (decision 0098 §1's form, for a stratum on the CPU).
    let on_device_rounds = match (solver(), comp.device.as_ref()) {
        (Some(dev), Some(d)) => d.iter().any(|(_, p)| dev.reads_own(p)),
        _ => false,
    };
    // A levels stratum is run by its driver, step by step (`Levels`), not as a closure.
    if comp.levels.lines.is_empty() && !on_device_rounds {
        comp.closure = {
            let _h = crate::code_rule_dsl::logic::lanes::prof::host("closure", "plan: recognize the form");
            closure::recognize(&comp)
        };
    }
    reg.push((key, Arc::new(Mutex::new(comp))));
    Ok(name)
}

/// At the fixpoint the component's relations move to `DERIVED`, the input of later strata (`r:` sources), with the
/// hash indexes the waves kept (`InRel::chains`): nothing is copied or indexed again.
pub fn close(conn: impl Scoped, name: &str) -> Result<()> {
    let conn = conn.db();
    let at_scope = conn.at();
    let mut reg = COMPONENTS.lock().unwrap();
    let Some(at) = reg.iter().position(|((s, n), _)| *s == at_scope && n == name) else { return Ok(()) };
    let (_, comp) = reg.remove(at);
    drop(reg);
    let comp = comp.lock().unwrap();
    if comp.done {
        let rels = std::mem::take(&mut comp.store.write().unwrap().rels);
        let mut derived = DERIVED.lock().unwrap();
        for rel in rels {
            let moved = InRel::moved(rel);
            let table = moved.name.get().cloned().unwrap_or_default();
            derived.insert((at_scope, table), Arc::new(moved));
        }
    }
    Ok(())
}

/// Runs waves until the fixpoint, or until `max_waves` more have run (0: no bound). Rows are written to the
/// relations' tables only at the fixpoint, so a bounded run can be resumed after the caller replans probes from
/// `stats`. A bounded run also returns when a multi-relation delta join outgrows the statistics available at
/// entry. This prevents a later wave's first substantial demand from running several times before replanning.
pub fn run(conn: impl Scoped, name: &str, max_waves: u32) -> Result<String> {
    let conn = conn.db();
    let t0 = Instant::now();
    let comp = component(conn, name)?;
    let mut comp = comp.lock().unwrap();
    let (nrels, symbols) = { let s = comp.store.read().unwrap(); (s.rels.len(), Arc::clone(&s.syms)) };
    // This run's own cursor over the shared atoms (decision 0114 §3): the atoms lock themselves, so a stratum need
    // not hold the connection's symbols for its run, and strata on other threads intern at once.
    let mut cursor = Interner::for_loading(&symbols);
    {
        let store=comp.store.read().unwrap();
        for probe in &comp.probes {
            if let Body::Join(join)=&probe.body {scalars::register(&join.steps,Some(&store));}
            if let Some(join)=&probe.whole {scalars::register(&join.steps,Some(&store));}
        }
    }
    let mut stmts = Vec::new();
    for p in &comp.probes { stmts.push(match &p.body { Body::Sql(sql) => Some(conn.sql()?.prepare(sql)?), Body::Join(_) => None }); }
    let (mut waves, mut visits, mut runs, mut skips) = (0u32, 0u64, 0u64, 0u64);
    let mut done = false;
    let mut paused = false;
    let mut seeded = comp.seeded;
    // Per probe: seconds spent in its statement, rows it returned, and, while profiling, this thread's CPU seconds
    // in it (the profile's `native.tsv`, per join plan).
    let mut per: Vec<(f64, u64, f64)> = vec![(0.0, 0, 0.0); comp.probes.len()];
    let cpu_timed = crate::code_rule_dsl::logic::profile::on();
    let cpu_now = || if cpu_timed { crate::code_rule_dsl::logic::profile::now() } else { 0.0 };
    let mut row: Vec<i64> = Vec::new();
    let wave_trace = std::env::var_os("RDL_WAVE_TRACE").is_some();
    // Whole-plan probes can start at a selective external input. Maintenance's delta-only probes cannot:
    // their order depends on the delta's measured domain. Watch the existing index counts, without scanning
    // rows or computing full statistics on every wave. Unbounded callers retain run-to-fixpoint semantics.
    let planning_counts: Vec<(usize, Vec<usize>)> = if max_waves == 0 { Vec::new() } else {
        let mut watched = vec![false; nrels];
        for p in &comp.probes {
            if p.delta == SEED || p.whole.is_some() { continue; }
            if let Body::Join(j) = &p.body
                && j.steps.iter().filter(|s| matches!(s, Step::Atom { neg: false, .. })).count() > 1 {
                watched[p.delta] = true;
            }
        }
        let store = comp.store.read().unwrap();
        store.rels.iter().enumerate().filter(|(r,_)| watched[*r]).map(|(r,rel)| {
            (r, std::iter::once(rel.n).chain(rel.sketches.iter().map(Sketch::ndv)).collect())
        }).collect()
    };
    let mut gates = std::mem::take(&mut comp.gates);
    let mut levels = std::mem::take(&mut comp.levels);
    let (held_back, mut present) = (!comp.deferred.is_empty(), false);
    let gates_done: Vec<usize> = gates.iter().map(|g| g.done).collect();
    // The closure form (decision 0098 §1): seeds once, then the walk's reach over distinct nodes, in two halves around
    // its launches. When the caller can wait (`lanes::wave::can_wait`: the stratum driver, which runs a level's
    // strata to their launches first), the launches join this thread's device wave and the run returns `3`; the run
    // after the wave ran finishes it. Otherwise they run here, as a batch of their own.
    let mut closure_ran: Option<Option<closure::Ran>> = None;
    let tp = Instant::now();
    if let Some((cl, prep, tickets)) = comp.waiting.take() {
        let outs = crate::code_rule_dsl::logic::lanes::wave::take(&tickets).map_err(|e| err(format!("{name}: {e}")))?;
        let mut guard = comp.store.write().unwrap();
        closure_ran = Some(closure::finish(&cl, prep, outs, &mut guard));
        drop(guard);
        comp.closure = Some(cl);
    } else if !seeded && let Some(cl) = comp.closure.take() {
        let mut guard = comp.store.write().unwrap();
        let store = &mut *guard;
        let mut syms = &mut cursor;
        let mut fresh: Vec<Fresh> = vec![Fresh::default(); nrels];
        for (pi, p) in comp.probes.iter().enumerate() {
            let (Body::Join(j), SEED) = (&p.body, p.delta) else { continue };
            runs += 1;
            let n = run_seed(j, p.head, store, &mut syms, &mut fresh)?;
            visits += n;
            per[pi].1 += n;
        }
        let k = store.rels[0].k;
        store.rels[0].reserve(fresh[0].n);
        for i in 0..fresh[0].n { store.rels[0].push(&fresh[0].data[i * k..][..k]); }
        let prep = closure::prepare(&cl, store, &mut syms);
        drop(syms);
        match prep {
            None => closure_ran = Some(None),
            Some(mut prep) if !prep.launches.is_empty() && crate::code_rule_dsl::logic::lanes::wave::can_wait() => {
                let tickets = crate::code_rule_dsl::logic::lanes::wave::queue("closure", std::mem::take(&mut prep.launches));
                drop(guard);
                comp.waiting = Some((cl, prep, tickets));
                comp.seeded = true;
                let per: Vec<String> = per.iter().map(|(t, n, c)| format!("{:.1}:{n}:{:.3}", t * 1e3, c * 1e3)).collect();
                let ms = t0.elapsed().as_secs_f64() * 1e3;
                return Ok(format!("0 1 {visits} {runs} {skips} {ms:.1} {ms:.1} {} 3", per.join(",")));
            }
            Some(mut prep) => {
                let launches = std::mem::take(&mut prep.launches);
                let outs = if launches.is_empty() {
                    Vec::new()
                } else {
                    let batch = rerail_accel::Batch(launches.into_iter().map(rerail_accel::Launch::new).collect());
                    let (done, _) = crate::code_rule_dsl::logic::lanes::prof::run("closure", batch).map_err(|e| err(format!("{name}: {e}")))?;
                    done.into_iter().map(|d| d.out).collect()
                };
                closure_ran = Some(closure::finish(&cl, prep, outs, store));
            }
        }
        drop(guard);
        comp.closure = Some(cl);
    }
    if let Some(ran) = closure_ran {
        seeded = true;
        waves = 1;
        let mut guard = comp.store.write().unwrap();
        let store = &mut *guard;
        let n = store.rels[0].n;
        match ran {
            Some(r) => {
                visits += r.nodes as u64 + r.rows as u64;
                crate::code_rule_dsl::logic::profile::decision("closure (decision 0098 §1)", || format!("a walk along an edge run as its closure, by {}", r.how));
                if wave_trace { eprintln!("      {name}: closure over {} nodes by {}, {} rows, {:.1} ms", r.nodes, r.how, r.rows, tp.elapsed().as_secs_f64() * 1e3); }
                store.rels[0].set_delta(n, n);
                done = true;
            }
            None => {
                crate::code_rule_dsl::logic::lanes::prof::count("closure", "closures run semi-naive (the edge search failed)", 1);
                crate::code_rule_dsl::logic::profile::decision("closure (decision 0098 §1)", || format!("a walk's edge search failed: semi-naive from its seeds ({name})"));
                store.rels[0].set_delta(0, n)
            }
        }
    }
    // Decision 0113 §3 for the device joins: when the caller can wait for its level's wave, a stratum placed on the
    // device encodes its seed joins into the wave's one command buffer and returns `3`; the run after the wave
    // finishes them (`SolverDevice::resume`). A level's small strata so share one commit and one wait. A recursive one
    // runs its rounds here.
    if !seeded && comp.device_wait.is_none() && crate::code_rule_dsl::logic::lanes::wave::can_wait()
        && let Some(dev) = solver()
        && let Some(plans) = comp.device.as_ref()
        && !plans.iter().any(|(_, p)| dev.reads_own(p))
        && std::env::var("RERAIL_DEVICE_WAVES").map_or(true, |v| v != "off")
    {
        let rules: Vec<(&JoinPlan, &JoinRule, usize)> = plans.iter().map(|(pi, plan)| match &comp.probes[*pi].body {
            Body::Join(j) => (plan, j, comp.probes[*pi].head),
            Body::Sql(_) => unreachable!("a device plan is a join's"),
        }).collect();
        let submitted = { let syms = symbols.read().unwrap(); dev.submit(&rules, &syms) };
        if let Ok(Some(t)) = submitted {
            comp.device_wait = Some(t);
            let per: Vec<String> = per.iter().map(|(t, n, c)| format!("{:.1}:{n}:{:.3}", t * 1e3, c * 1e3)).collect();
            let ms = t0.elapsed().as_secs_f64() * 1e3;
            return Ok(format!("0 1 {visits} {runs} {skips} {ms:.1} {ms:.1} {} 3", per.join(",")));
        }
    }
    // A stratum back from its wave: its device joins are finished below, in their place.
    let mut device_ticket = comp.device_wait.take();
    while !done {
        let mut fresh: Vec<Fresh> = vec![Fresh::default(); nrels];
        // Probe rules: SQL over the vtabs, while no lock is held (the vtab cursors take read locks).
        let mut raw: Vec<(usize, Vec<Vec<Value>>)> = Vec::new();
        for (pi, (p, st)) in comp.probes.iter().zip(stmts.iter_mut()).enumerate() {
            let Some(st) = st else { continue };
            let tp = Instant::now();
            let empty = { let s = comp.store.read().unwrap(); s.rels[p.delta].lo == s.rels[p.delta].hi };
            if empty { skips += 1; continue; }
            runs += 1;
            let c0 = cpu_now();
            let k = comp.store.read().unwrap().rels[p.head].types.len();
            let mut rows = st.query([])?;
            let mut got = Vec::new();
            while let Some(r) = rows.next()? { got.push((0..k).map(|i| r.get::<_, Value>(i)).collect::<Result<Vec<_>>>()?); }
            per[pi].0 += tp.elapsed().as_secs_f64();
            per[pi].1 += got.len() as u64;
            per[pi].2 += cpu_now() - c0;
            raw.push((p.head, got));
        }
        let mut guard = comp.store.write().unwrap();
        let mut store = &mut *guard;
        let mut syms = &mut cursor;
        for (head, got) in raw {
            let types = store.rels[head].types.clone();
            for vals in got {
                visits += 1;
                row.clear();
                // A function with no value (`rx_group` with no match) gives no row, as the SQL strata's inserts.
                if vals.iter().any(|v| matches!(v, Value::Null)) { continue; }
                for (v, &ty) in vals.iter().zip(&types) {
                    row.push(read_value(ValueRef::from(v), ty, &mut syms).map_err(|e| err(format!("{}: {e}", store.rels[head].table)))?);
                }
                fresh[head].emitted += 1;
                fresh[head].offer(&row, &store.rels[head]);
            }
        }
        // The seed joins on the device, in one batch, when the stratum was placed there (decision 0095, step 4).
        let dev = solver();
        let tried = !seeded && comp.device.is_some() && dev.is_some();
        // The component's fixpoint ran on the device (a recursive one): its rows are all of them, and it is done.
        let mut device_fixed = false;
        let on_device = tried && {
            let dev = dev.expect("a device");
            let plans = comp.device.as_ref().expect("device plans");
            let tp = Instant::now();
            let c0 = cpu_now();
            let rules: Vec<(&JoinPlan, &JoinRule, usize)> = plans.iter().map(|(pi, plan)| match &comp.probes[*pi].body {
                Body::Join(j) => (plan, j, comp.probes[*pi].head),
                Body::Sql(_) => unreachable!("a device plan is a join's"),
            }).collect();
            let mut dev_fresh: Vec<Fresh> = vec![Fresh::default(); nrels];
            let (seeds, deltas): (Vec<_>, Vec<_>) = rules.iter().copied().partition(|r| !dev.reads_own(r.0));
            device_fixed = !deltas.is_empty();
            let got = if device_fixed {
                let mut heads: Vec<usize> = rules.iter().map(|r| r.2).collect();
                heads.sort_unstable();
                heads.dedup();
                // Each rule's rows in `plans`' order, from the fixpoint's seeds-then-deltas order.
                dev.fixpoint(&seeds, &deltas, &heads, store, &syms, &mut dev_fresh).map(|made| {
                    let (mut s, mut d) = (made[..seeds.len()].iter(), made[seeds.len()..].iter());
                    rules.iter().map(|r| *if dev.reads_own(r.0) { d.next() } else { s.next() }.expect("a count per rule")).collect::<Vec<u64>>()
                })
            } else if let Some(t) = device_ticket.take() {
                dev.resume(t, &rules, store, &syms, &mut dev_fresh)
            } else {
                dev.run_joins(&rules, store, &syms, &mut dev_fresh)
            };
            match got {
                Ok(made) => {
                    let cpu = cpu_now() - c0;
                    let wall = tp.elapsed().as_secs_f64();
                    for ((pi, _), n) in plans.iter().zip(made) {
                        runs += 1;
                        visits += n;
                        per[*pi].1 += n;
                        per[*pi].0 += wall / plans.len() as f64;
                        per[*pi].2 += cpu / plans.len() as f64;
                    }
                    for (f, d) in fresh.iter_mut().zip(dev_fresh) {
                        f.data.extend_from_slice(&d.data);
                        f.n += d.n;
                    }
                    true
                }
                Err(why) => {
                    // Refused at run time (a table past the device's budget): the seeds run on the CPU, named.
                    dev.refuse(why);
                    device_fixed = false;
                    false
                }
            }
        };
        if tried && !on_device {
            // The seeds were parsed without the CPU's indexes: parse them again with them.
            drop(syms);
            drop(guard);
            let specs = std::mem::take(&mut comp.seed_specs);
            {
                let mut syms = Interner::for_loading(&symbols);
                for (pi, js) in &specs {
                    let j = join_rule(conn, js, &mut syms)?;
                    comp.probes[*pi].body = Body::Join(j);
                }
                syms.check()?;
            }
            comp.device = None;
            guard = comp.store.write().unwrap();
            store = &mut *guard;
            syms = &mut cursor;
        }
        // Join probes, natively; seed joins in the first wave only (none when the device ran the fixpoint).
        // The whole plans run this wave, by head: a spec says what a body derives, not where, so two rules with one
        // body and different heads (a magic relation and its reader's, decision 0108's demand) are two runs.
        let mut wholes_run: Vec<(usize, &Arc<JoinSpec>)> = Vec::new();
        for (pi, p) in comp.probes.iter().enumerate() {
            let Body::Join(j) = &p.body else { continue };
            let tp = Instant::now();
            if device_fixed { break; }
            if p.delta == SEED { if seeded || on_device { continue } }
            else if store.rels[p.delta].lo == store.rels[p.delta].hi { skips += 1; continue; }
            let j = match (&p.whole, &p.wkey) {
                (Some(w), Some(k)) if p.delta != SEED && store.rels[p.delta].lo == 0 => {
                    if wholes_run.iter().any(|&(h, w)| h == p.head && (Arc::ptr_eq(w, k) || **w == **k)) { skips += 1; continue; }
                    wholes_run.push((p.head, k));
                    w
                }
                _ => j,
            };
            runs += 1;
            if wave_trace {
                let d = if p.delta == SEED { 0 } else { store.rels[p.delta].hi - store.rels[p.delta].lo };
                eprintln!("      wave {waves} probe {pi} of {name} starts, head {}, delta {d}", store.rels[p.head].table);
            }
            let c0 = cpu_now();
            let n = if p.delta == SEED { run_seed(j, p.head, store, &mut syms, &mut fresh)? } else { run_join(j, p.head, store, &mut syms, &mut fresh)? };
            visits += n;
            per[pi].0 += tp.elapsed().as_secs_f64();
            per[pi].1 += n;
            per[pi].2 += cpu_now() - c0;
            // RDL_WAVE_TRACE: each probe of a wave as it runs (probe lines in spec order, seeds first).
            if wave_trace {
                let d = if p.delta == SEED { 0 } else { store.rels[p.delta].hi - store.rels[p.delta].lo };
                let steps: Vec<String> = j.counts.iter().map(|[a, b]| format!("{}:{}", a.get(), b.get())).collect();
                eprintln!("      wave {waves} probe {pi}: {:.0} ms, delta {d}, {n} derived, steps {}", tp.elapsed().as_secs_f64() * 1e3, steps.join(","));
            }
        }
        // Edge rules, natively.
        let (mut slots, mut bound, mut key): (Vec<i64>, Vec<bool>, Vec<i64>) = (Vec::new(), Vec::new(), Vec::new());
        for rule in &comp.edges {
            slots.clear();
            slots.resize(rule.nslots, 0);
            let (lo, hi) = (store.rels[rule.src].lo, store.rels[rule.src].hi);
            for ti in lo..hi {
                if store.rels[rule.src].dead(ti as u32) { continue; }
                visits += 1;
                bound.clear();
                bound.resize(rule.nslots, false);
                let ok = {
                    let t = store.rels[rule.src].row(ti);
                    rule.pat.iter().zip(t.iter()).all(|(p, &v)| match p {
                        Pat::Any => true,
                        Pat::Int(c) | Pat::Sym(c) => *c == v,
                        Pat::Slot(s) => if bound[*s] { slots[*s] == v } else { bound[*s] = true; slots[*s] = v; true },
                    })
                };
                if !ok { continue; }
                let mut emit = |slots: &[i64], store: &Store, syms: &mut Interner, fresh: &mut [Fresh]| {
                    row.clear();
                    row.extend(rule.out.iter().map(|o| match o {
                        Term::Slot(s) => slots[*s],
                        Term::Const(c) => *c,
                        Term::Cat(a, b) => cat(syms, slots[*a], slots[*b]),
                    }));
                    fresh[rule.head].emitted += 1;
                    fresh[rule.head].offer(&row, &store.rels[rule.head]);
                };
                if !rule.has_edge { emit(&slots, store, &mut syms, &mut fresh); continue; }
                key.clear();
                key.extend(rule.key.iter().map(|&s| slots[s]));
                if let Some(es) = rule.edges.get(&key[..]) {
                    for e in es {
                        for (&s, &v) in rule.ecols.iter().zip(e) { slots[s] = v; }
                        emit(&slots, store, &mut syms, &mut fresh);
                    }
                }
            }
        }
        waves += 1;
        seeded = true;
        let mut any = false;
        let mut lo_of = vec![0; nrels];
        let counting = std::env::var_os("RERAIL_SPIKE_DERIVATIONS").is_some();
        for (r, f) in fresh.into_iter().enumerate() {
            let rel = &mut store.rels[r];
            let lo = rel.n;
            let emitted = f.emitted;
            any |= rel.push_fresh(f, gates_done.contains(&r));
            if counting && emitted > 0 {
                let mut all = DERIVATIONS.lock().unwrap();
                let e = all.entry(rel.table.clone()).or_default();
                e.0 += emitted;
                e.1 += (rel.n - lo) as u64;
            }
            lo_of[r] = lo;
            if !gates_done.contains(&r) { let hi = rel.n; rel.set_delta(lo, hi); }
        }
        // A request relation that gained rows pauses the run before any gate opens: the caller answers them
        // (`pending`, `add`) and runs again. A gate opens only once no request is unanswered.
        if comp.requests.iter().any(|&(r, answered)| store.rels[r].n > answered) {
            for &r in &gates_done { let (lo, hi) = (lo_of[r], store.rels[r].n); store.rels[r].set_delta(lo, hi); }
            paused = true;
            break;
        }
        // Gates, lowest first: one whose quiet relations got nothing this wave, and whose earlier gates have nothing
        // left to pass, passes its source's new rows. Its `done` relation's delta is set after, so they are this
        // wave's rows.
        let mut busy = vec![false; gates.len()];
        for gi in 0..gates.len() {
            let g = &gates[gi];
            let n = g.src.map_or(1, |s| store.rels[s].n);
            let quiet = g.quiet.iter().all(|&r| store.rels[r].lo == store.rels[r].hi) && g.after.iter().all(|&a| !busy[a]);
            busy[gi] = n > g.passed;
            if !quiet || n <= g.passed { continue; }
            for id in g.passed..n {
                let row: Vec<i64> = match g.src { Some(s) => store.rels[s].row(id).to_vec(), None => vec![0] };
                any |= store.rels[g.done].push(&row);
            }
            gates[gi].passed = n;
        }
        for &r in &gates_done { let (lo, hi) = (lo_of[r], store.rels[r].n); store.rels[r].set_delta(lo, hi); }
        if device_fixed {
            for r in 0..nrels { let n = store.rels[r].n; store.rels[r].set_delta(n, n); }
            done = true;
            break;
        }
        // Quiet: a recursive stratum's levels decide the next rows gone, or check the next level (`Levels`).
        if !any { any = levels.step(store); }
        // Quiet, with request rows held back: the caller is asked them again (`defer`).
        if !any && held_back { present = true; paused = true; break; }
        if !any { done = true; break; }
        if max_waves > 0 && waves >= max_waves { break; }
        if planning_counts.iter().any(|(r, before)| {
            let rel = &store.rels[*r];
            std::iter::once(rel.n).chain(rel.sketches.iter().map(Sketch::ndv)).zip(before)
                .any(|(now,then)| (*then == 0 && now > 0) || now as f64 > (*then).max(1) as f64 * crate::code_rule_dsl::logic::plan::PRICE_DRIFT)
        }) {
            if wave_trace { eprintln!("      {name}: return for planning after {waves} waves; delta statistics grew"); }
            break;
        }
    }
    comp.seeded = seeded;
    comp.presenting = present;
    comp.gates = gates;
    comp.levels = levels;
    // A symbol a wave could not intern fails the run.
    cursor.check()?;
    symbols.write().unwrap().check()?;
    let solve = t0.elapsed();
    let per: Vec<String> = per.iter().map(|(t, n, c)| format!("{:.1}:{n}:{:.3}", t * 1e3, c * 1e3)).collect();
    if paused {
        return Ok(format!("0 {waves} {visits} {runs} {skips} {:.1} {:.1} {} 2", solve.as_secs_f64() * 1e3, t0.elapsed().as_secs_f64() * 1e3, per.join(",")));
    }
    if !done {
        return Ok(format!("0 {waves} {visits} {runs} {skips} {:.1} {:.1} {} 0", solve.as_secs_f64() * 1e3, t0.elapsed().as_secs_f64() * 1e3, per.join(",")));
    }
    comp.done = true;
    let mut new_rows = 0usize;
    if comp.write {
        // Each relation's new rows into its table, many rows per statement (SQLite binds at most 32766 values).
        let store = comp.store.read().unwrap();
        for (ri, rel) in store.rels.iter().enumerate() {
            if comp.produced.contains(&ri) { continue; }
            let per = (32000 / rel.k.max(1)).max(1);
            let mut id = rel.stored;
            while id < rel.n {
                let end = (id + per).min(rel.n);
                let holes = vec![format!("({})", vec!["?"; rel.k].join(", ")); end - id].join(", ");
                conn.sql()?.execute(&format!("INSERT INTO {} VALUES {holes}", crate::code_rule_dsl::logic::sql::table(&rel.table)), rusqlite::params_from_iter(rel.data[id * rel.k..end * rel.k].iter()))?;
                new_rows += end - id;
                id = end;
            }
        }
    }
    Ok(format!("{new_rows} {waves} {visits} {runs} {skips} {:.1} {:.1} {} 1", solve.as_secs_f64() * 1e3, t0.elapsed().as_secs_f64() * 1e3, per.join(",")))
}

/// The request rows not yet answered, per request relation (its table), now counted as answered.
pub fn pending(conn: impl Scoped, name: &str) -> Result<Vec<(String, Vec<Vec<i64>>)>> {
    let conn = conn.db();
    let comp = component(conn, name)?;
    let mut comp = comp.lock().unwrap();
    let store = Arc::clone(&comp.store);
    let store = store.read().unwrap();
    let mut out: Vec<(String, Vec<Vec<i64>>)> = Vec::new();
    for (r, answered) in &mut comp.requests {
        let rel = &store.rels[*r];
        if rel.n > *answered {
            out.push((rel.table.clone(), (*answered..rel.n).filter(|&i| !rel.dead(i as u32)).map(|i| rel.row(i).to_vec()).collect()));
            *answered = rel.n;
        }
    }
    // Quiet with rows held back: those rows, asked again.
    if std::mem::take(&mut comp.presenting) {
        for (r, rows) in std::mem::take(&mut comp.deferred) {
            let table = store.rels[r].table.clone();
            match out.iter_mut().find(|(t, _)| *t == table) {
                Some((_, got)) => got.extend(rows),
                None => out.push((table, rows)),
            }
        }
    }
    Ok(out)
}

/// Holds back request rows the caller does not answer yet (a load past the budget's current level, decision 0108):
/// they are asked again once the run is otherwise quiet. Until then they are as rows not yet asked.
pub fn defer(conn: impl Scoped, name: &str, table: &str, rows: Vec<Vec<i64>>) -> Result<()> {
    let conn = conn.db();
    let comp = component(conn, name)?;
    let mut comp = comp.lock().unwrap();
    let r = comp.store.read().unwrap().rels.iter().position(|rel| rel.table == table).ok_or_else(|| err(format!("{name}: no relation {table}")))?;
    if !comp.requests.iter().any(|(q, _)| *q == r) { return Err(err(format!("{name}: {table} is not a request"))); }
    if !rows.is_empty() { comp.deferred.push((r, rows)); }
    Ok(())
}

/// Adds an answer's rows to a produced relation (by table), as its delta for the next wave: once per relation per
/// pause. Returns how many were new.
pub fn add(conn: &Connection, name: &str, table: &str, rows: &[Vec<i64>]) -> Result<usize> {
    let comp = component(conn, name)?;
    let comp = comp.lock().unwrap();
    let mut store = comp.store.write().unwrap();
    let r = store.rels.iter().position(|rel| rel.table == table).ok_or_else(|| err(format!("{name}: no relation {table}")))?;
    if !comp.produced.contains(&r) { return Err(err(format!("{name}: {table} is not a produced relation"))); }
    let rel = &mut store.rels[r];
    if rel.lo != rel.hi { return Err(err(format!("{name}: {table} was added to twice in one pause"))); }
    let lo = rel.n;
    rel.reserve(rows.len());
    let new = rows.iter().filter(|row| rel.push(row)).count();
    let hi = rel.n;
    rel.set_delta(lo, hi);
    Ok(new)
}

/// Live statistics of a component's relations, one line per relation in spec order: its row count, then each
/// column's distinct count (its hash index's key count), then each column's sum of squared posting lengths (the
/// skew: sum/rows is the expected postings of a row's value, the planner's `fan`). What the planner lacks for a relation still being derived.
pub fn stats(conn: &Connection, name: &str) -> Result<String> {
    let comp = component(conn, name)?;
    let comp = comp.lock().unwrap();
    let store = comp.store.read().unwrap();
    let syms = store.syms.read().unwrap();
    Ok(store.rels.iter().map(|r| {
        probed::note(&format!("{} [stats]", r.table), std::iter::empty());
        let cols = &r.sketches;
        let mut line = std::iter::once(r.n).chain(cols.iter().map(Sketch::ndv))
            .chain(cols.iter().map(|sk| sk.sq() as usize))
            .map(|n| n.to_string()).collect::<Vec<_>>().join(" ");
        // Each value's postings while the bounded histogram remains exact, as `|col:hexvalue=n,..`: a
        // constant there is priced exactly, as stat_val prices an input's.
        for (c, sk) in cols.iter().enumerate() {
            let Some(small) = sk.exact_counts() else { continue };
            let vals: Vec<String> = small.iter().map(|&(v, n)| {
                let text = if r.types[c] == b's' { syms.name(v) } else { v.to_string() };
                format!("{}={}", text.bytes().map(|b| format!("{b:02x}")).collect::<String>(), n)
            }).collect();
            line.push_str(&format!(" |{c}:{}", vals.join(",")));
        }
        line
    }).collect::<Vec<_>>().join("\n"))
}

/// A fused filter over an input relation that a device can run alone (`scans`): its row program, its inputs, its
/// atom's gathered rows, its parameters, and the rows the CPU passes with the CPU it took.
pub struct ScanCase {
    pub rule: String,
    pub program: rerail_compiler::block::BlockProgram,
    pub inputs: Vec<rowcode::Word>,
    pub gathered: Vec<(usize, Vec<i64>)>,
    pub params: Vec<i64>,
    pub passed: Vec<u32>,
    pub cpu_seconds: f64,
}

/// Each fused filter of an open component's rules whose atom reads an input relation with constant keys and whose
/// parameters are constants: the whole of what it filters is known before the run, so a device scan of it can be
/// checked against the CPU's (the relational lab).
pub fn scans(conn: &Connection, name: &str) -> Result<Vec<ScanCase>> {
    let comp = component(conn, name)?;
    let comp = comp.lock().unwrap();
    let mut out = Vec::new();
    let rules = comp.probes.iter().enumerate().flat_map(|(p, probe)| {
        let body = match &probe.body { Body::Join(j) => Some((format!("probe{p}"), j)), Body::Sql(_) => None };
        body.into_iter().chain(probe.whole.as_ref().map(|j| (format!("probe{p}.whole"), j)))
    });
    for (label, j) in rules {
        for (i, f) in j.fused.iter().enumerate() {
            let Some(f) = f.as_deref() else { continue };
            let Step::Atom { src: src @ Src::In(..), keys, tests, .. } = &j.steps[i] else { continue };
            if keys.iter().any(|(_, k)| matches!(k, Key::Slot(_))) || f.params.iter().any(|p| matches!(p.key, Key::Slot(_))) { continue; }
            let key: Vec<i64> = keys.iter().map(|&(_, k)| match k { Key::Const(c) => c, Key::Slot(_) => unreachable!("constants") }).collect();
            let Src::In(rel, _) = src else { unreachable!("an input") };
            // Every row, its keys checked (the index finds the same ones).
            let keep: Vec<u32> = (0..rel.len() as u32).filter(|&id| !is_dead(&rel.dead, id)).filter(|&id| {
                let row = rel.at(id as usize).read();
                keys.iter().zip(&key).all(|(&(c, _), &v)| row.at(c) == v) && tests.iter().all(|&(c, c0)| row.at(c) == row.at(c0))
            }).collect();
            let gathered: Vec<(usize, Vec<i64>)> = f.gather.iter().map(|&(_, c)| (c, keep.iter().map(|&id| rel.at(id as usize).read().at(c)).collect())).collect();
            let params: Vec<i64> = f.params.iter().map(|p| { let Key::Const(v) = p.key else { unreachable!("constants") }; if p.doc { v >> 32 } else { v } }).collect();
            // The CPU: the chunks as the run takes them.
            let c0 = thread_cpu();
            let mut s = rowcode::Scratch { columns: vec![Vec::new(); f.columns.len()], params: params.clone(), ..Default::default() };
            let mut passed = Vec::new();
            for start in (0..keep.len()).step_by(rowcode::CHUNK) {
                let end = (start + rowcode::CHUNK).min(keep.len());
                for (g, (_, col)) in f.gather.iter().zip(&gathered) {
                    s.columns[g.0].clear();
                    s.columns[g.0].extend_from_slice(&col[start..end]);
                }
                s.rows = end - start;
                let words = f.run(&mut s, |_, _| {});
                for (w, &word) in words.iter().enumerate() {
                    let mut m = word;
                    while m != 0 { passed.push((start + 64 * w + m.trailing_zeros() as usize) as u32); m &= m - 1; }
                }
            }
            let cpu_seconds = thread_cpu() - c0;
            out.push(ScanCase { rule: format!("{label} step {i}"), program: f.program.clone(), inputs: f.columns.clone(), gathered, params, passed, cpu_seconds });
        }
    }
    Ok(out)
}

/// Two atoms of a rule over input relations, the second's keys all bound by the first (`tiles`): the first's rows (its
/// constant keys checked) by the columns the second's keys read, the second's rows by its key columns, and the pairs
/// a CPU hash join gives (left row, right row id) with the CPU it took.
pub struct TileCase {
    pub rule: String,
    pub left: Vec<Vec<i64>>,
    pub right: Vec<Vec<i64>>,
    pub(in crate::code_rule_dsl) right_ids: Vec<u32>,
    /// The right side's identity: its relation, key columns and column tests (one sort serves every case of it).
    pub right_key: String,
    /// Whether each key holds symbols.
    pub(in crate::code_rule_dsl) symbols: Vec<bool>,
    pub pairs: Vec<(u32, u32)>,
    pub cpu_seconds: f64,
}

/// Each rule's first two steps when both are atoms over input relations and the second's keys are slots the first
/// binds: a join whose whole input is known before the run (the relational lab's `tile`).
pub fn tiles(conn: &Connection, name: &str) -> Result<Vec<TileCase>> {
    let comp = component(conn, name)?;
    let comp = comp.lock().unwrap();
    let mut out = Vec::new();
    let rules = comp.probes.iter().enumerate().flat_map(|(p, probe)| {
        let body = match &probe.body { Body::Join(j) => Some((format!("probe{p}"), j)), Body::Sql(_) => None };
        body.into_iter().chain(probe.whole.as_ref().map(|j| (format!("probe{p}.whole"), j)))
    });
    for (label, j) in rules {
        let (Some(Step::Atom { src: Src::In(a, _), neg: false, keys: ak, binds, tests: at }), Some(Step::Atom { src: Src::In(b, _), neg: false, keys: bk, tests: bt, .. })) = (j.steps.first(), j.steps.get(1)) else { continue };
        if bk.is_empty() || ak.iter().any(|(_, k)| matches!(k, Key::Slot(_))) { continue; }
        let Some(lcols) = bk.iter().map(|&(_, k)| match k { Key::Slot(s) => binds.iter().find(|b| b.1 == s).map(|b| b.0), Key::Const(_) => None }).collect::<Option<Vec<usize>>>() else { continue };
        let rcols: Vec<usize> = bk.iter().map(|&(c, _)| c).collect();
        let akey: Vec<(usize, i64)> = ak.iter().map(|&(c, k)| (c, match k { Key::Const(v) => v, Key::Slot(_) => unreachable!("constants") })).collect();
        let lrows: Vec<u32> = (0..a.len() as u32).filter(|&id| !is_dead(&a.dead, id)).filter(|&id| {
            let row = a.at(id as usize).read();
            akey.iter().all(|&(c, v)| row.at(c) == v) && at.iter().all(|&(c, c0)| row.at(c) == row.at(c0))
        }).collect();
        let right_ids: Vec<u32> = (0..b.len() as u32).filter(|&id| !is_dead(&b.dead, id)).filter(|&id| {
            let row = b.at(id as usize).read();
            bt.iter().all(|&(c, c0)| row.at(c) == row.at(c0))
        }).collect();
        let left: Vec<Vec<i64>> = lcols.iter().map(|&c| lrows.iter().map(|&id| a.at(id as usize).read().at(c)).collect()).collect();
        let right: Vec<Vec<i64>> = rcols.iter().map(|&c| right_ids.iter().map(|&id| b.at(id as usize).read().at(c)).collect()).collect();
        // The CPU: a hash join, built on the right and probed by each left row.
        let c0 = thread_cpu();
        let mut table: HashMap<Vec<i64>, Vec<u32>> = HashMap::default();
        for r in 0..right_ids.len() { table.entry(right.iter().map(|c| c[r]).collect()).or_default().push(r as u32); }
        let mut pairs = Vec::new();
        let mut key = Vec::new();
        for l in 0..lrows.len() {
            key.clear();
            key.extend(left.iter().map(|c| c[l]));
            if let Some(rs) = table.get(&key) { pairs.extend(rs.iter().map(|&r| (l as u32, r))); }
        }
        let cpu_seconds = thread_cpu() - c0;
        let symbols = rcols.iter().map(|&c| b.types.get(c) == Some(&b's')).collect();
        let right_key = format!("{:p} {rcols:?} {bt:?}", Arc::as_ptr(b));
        out.push(TileCase { rule: label, left, right, right_ids, right_key, symbols, pairs, cpu_seconds });
    }
    Ok(out)
}

/// `set_fixpoint_steps(name)`: per probe line (spec order), its join steps' "in:out" counts since the last call,
/// comma-separated (empty for a SQL probe); the counts restart.
/// Per probe (spec order), its join steps' (in, out) binding counts since the last call (empty for a SQL probe); the
/// counts restart.
pub fn step_counts(conn: impl Scoped, name: &str) -> Result<Vec<Vec<(i64, i64)>>> {
    let conn = conn.db();
    let comp = component(conn, name)?;
    let comp = comp.lock().unwrap();
    Ok(comp.probes.iter().map(|p| match &p.body {
        Body::Join(j) => j.counts.iter().map(|[a, b]| (a.replace(0) as i64, b.replace(0) as i64)).collect(),
        Body::Sql(_) => Vec::new(),
    }).collect())
}

/// A relation's live statistics (`stats`): rows, then per column its distinct values and sum of squared postings,
/// and each value's postings when a bounded exact histogram is available.
pub struct Live { pub n: i64, pub delta: i64, pub ndv: Vec<i64>, pub sq: Vec<i64>, pub freq: Vec<(usize, Vec<(i64, i64)>)> }
pub fn live(conn: impl Scoped, name: &str) -> Result<Vec<Live>> {
    let conn = conn.db();
    let comp = component(conn, name)?;
    let comp = comp.lock().unwrap();
    let store = comp.store.read().unwrap();
    Ok(store.rels.iter().map(|r| {
        probed::note(&format!("{} [stats]", r.table), std::iter::empty());
        let cols = &r.sketches;
        let freq = cols.iter().enumerate().filter_map(|(c, sk)| sk.exact_counts().map(|s| (c, s.iter().map(|&(v, n)| (v, i64::from(n))).collect())))
            .collect();
        Live {
            n: r.n as i64,
            delta: (r.hi - r.lo) as i64,
            ndv: cols.iter().map(|sk| sk.ndv() as i64).collect(),
            sq: cols.iter().map(|sk| sk.sq() as i64).collect(),
            freq,
        }
    }).collect())
}

pub fn steps(conn: &Connection, name: &str) -> Result<String> {
    let comp = component(conn, name)?;
    let comp = comp.lock().unwrap();
    Ok(comp.probes.iter().map(|p| match &p.body {
        Body::Join(j) => j.counts.iter().map(|[a, b]| format!("{}:{}", a.replace(0), b.replace(0))).collect::<Vec<_>>().join(","),
        Body::Sql(_) => String::new(),
    }).collect::<Vec<_>>().join("\n"))
}

/// Replaces probe `i`'s body, replanned from `stats`: `kind` is `sql` (hex SQL, prepared by the next `run`) or `join`
/// (a hex join spec).
pub fn reprobe(conn: &Connection, name: &str, i: usize, kind: &str, body: &str) -> Result<()> {
    reprobe_spec(conn, name, i, &Reprobe::parse(kind, body).map_err(err)?)
}

/// `reprobe` of the planner's values, in process.
pub fn reprobe_spec(conn: impl Scoped, name: &str, i: usize, new: &Reprobe) -> Result<()> {
    let conn = conn.db();
    let comp = component(conn, name)?;
    let mut comp = comp.lock().unwrap();
    let n = comp.probes.len();
    let joined = |comp: &Component, spec: &JoinSpec| -> Result<JoinRule> {
        let symbols = Arc::clone(&comp.store.read().unwrap().syms);
        let mut syms = Interner::for_loading(&symbols);
        let j = join_rule(conn, spec, &mut syms)?;
        syms.check()?;
        let mut store = comp.store.write().unwrap();
        for (r, cols) in &j.multi { store.rels[*r].want(cols); }
        Ok(j)
    };
    let body = match new {
        Reprobe::Sql(sql) => Body::Sql(sql.clone()),
        Reprobe::Join(spec) => Body::Join(joined(&comp, spec)?),
        // The probe's whole plan (`ProbeRule::whole`), planned again from live statistics.
        Reprobe::Whole(spec) => {
            let j = joined(&comp, spec)?;
            let levels: Vec<usize> = comp.levels.lines.iter().flat_map(|l| [l.cand, l.chk, l.held, l.gone, l.live]).collect();
            let p = comp.probes.get_mut(i).ok_or_else(|| err(format!("no probe {i} of {n}")))?;
            // A levels relation's probes take no whole plan (`open`).
            if levels.contains(&p.delta) { return Ok(()); }
            p.whole = Some(j);
            p.wkey = Some(Arc::clone(spec));
            return Ok(());
        }
    };
    comp.probes.get_mut(i).ok_or_else(|| err(format!("no probe {i} of {n}")))?.body = body;
    Ok(())
}

// ---- set_rel(component, rel, full|delta): a relation's tuples while its component runs -------------------------

#[repr(C)]
pub struct RelTab { base: ffi::sqlite3_vtab, store: Arc<RwLock<Store>>, rel: usize, delta: bool }

unsafe impl<'vtab> VTab<'vtab> for RelTab {
    type Aux = ();
    type Cursor = RelCursor;
    fn connect(db: &mut VTabConnection, _: Option<&()>, _: &[u8], _: &[u8], _: &[u8], args: &[&[u8]]) -> Result<(Cow<'static, CStr>, Self)> {
        let arg = |i: usize| std::str::from_utf8(args.get(i).copied().unwrap_or(b"")).unwrap_or("").trim().trim_matches('\'').to_string();
        let comp = component_in(unsafe { db.handle() } as usize, &arg(0)).map_err(|e| Error::ModuleError(e.to_string()))?;
        let store = comp.lock().unwrap().store.clone();
        let rel: usize = arg(1).parse().map_err(|_| Error::ModuleError("bad rel".into()))?;
        let types = store.read().unwrap().rels[rel].types.clone();
        let cols: Vec<String> = types.iter().enumerate().map(|(i, &t)| format!("c{i} {}", if t == b's' { "TEXT" } else { "INTEGER" })).collect();
        let schema = CString::new(format!("CREATE TABLE x({})", cols.join(", "))).map_err(|e| Error::ModuleError(e.to_string()))?;
        Ok((Cow::Owned(schema), RelTab { base: ffi::sqlite3_vtab::default(), store, rel, delta: arg(2) == "delta" }))
    }
    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        // Every usable equality, one per column: the cursor picks the column whose posting list for the actual values
        // is shortest and checks the others itself. The plan is fixed when the probe is prepared, while the relation
        // is still empty, so no choice made here could know which column is selective (taking the first one made
        // `pv($o, "inst", $c)` read every instance row through the constant).
        let rows = { let s = self.store.read().unwrap(); let r = &s.rels[self.rel]; if self.delta { r.hi - r.lo } else { r.n } };
        let mut cols: Vec<(usize, i32)> = Vec::new();
        for (i, c) in info.constraints().enumerate() {
            if c.is_usable() && c.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ && c.column() < 31
                && !cols.iter().any(|&(_, col)| col == c.column()) { cols.push((i, c.column())); }
        }
        cols.sort_by_key(|&(_, col)| col);
        let mut mask = 0;
        for (arg, &(i, col)) in cols.iter().enumerate() {
            let mut u = info.constraint_usage(i);
            u.set_argv_index(arg as c_int + 1);
            u.set_omit(true);
            mask |= 1 << col;
        }
        info.set_idx_num(mask);
        if cols.is_empty() {
            info.set_estimated_cost(rows.max(1) as f64);
            info.set_estimated_rows(rows.max(1) as i64);
        } else {
            info.set_estimated_cost(2.0 / cols.len() as f64);
            info.set_estimated_rows(2);
        }
        Ok(true)
    }
    fn open(&mut self) -> Result<RelCursor> {
        Ok(RelCursor { base: ffi::sqlite3_vtab_cursor::default(), store: self.store.clone(), rel: self.rel, delta: self.delta, ids: Vec::new(), pos: 0 })
    }
}
impl CreateVTab<'_> for RelTab { const KIND: VTabKind = VTabKind::Default; }

#[repr(C)]
pub struct RelCursor { base: ffi::sqlite3_vtab_cursor, store: Arc<RwLock<Store>>, rel: usize, delta: bool, ids: Vec<u32>, pos: usize }
unsafe impl VTabCursor for RelCursor {
    fn filter(&mut self, idx_num: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        let s = self.store.read().unwrap();
        let r = &s.rels[self.rel];
        self.pos = 0;
        self.ids.clear();
        if idx_num == 0 {
            if self.delta { self.ids.extend((r.lo as u32..r.hi as u32).filter(|&id| !r.dead(id))) } else { self.ids.extend((0..r.n as u32).filter(|&id| !r.dead(id))) }
            return Ok(());
        }
        // Bit c of idx_num: column c is constrained, the constraints' values in column order.
        let mut keys: Vec<(usize, i64)> = Vec::new();
        let mut arg = 0;
        for col in 0..r.types.len() {
            if idx_num & (1 << col) == 0 { continue; }
            let key = match (args.get::<Value>(arg)?, r.types[col]) {
                (Value::Integer(i), b'i') => Some(i),
                (Value::Text(t), b'i') => t.parse().ok(),
                (Value::Text(t), b's') => s.syms.read().unwrap().get(&t),
                (Value::Integer(i), b's') => Some(i),
                _ => None,
            };
            arg += 1;
            match key { Some(k) => keys.push((col, k)), None => return Ok(()) }
        }
        if probed::on() && !keys.is_empty() { probed::note(&format!("{}{}", r.table, if self.delta { "+delta" } else { "" }), keys.iter().map(|&(c, _)| c)); }
        let cols: Vec<usize> = keys.iter().map(|&(c, _)| c).collect();
        let vals: Vec<i64> = keys.iter().map(|&(_, k)| k).collect();
        let (cands, _) = r.probe(&cols, &vals, self.delta);
        let ids = &mut self.ids;
        cands.any(|id| { if !r.dead(id) && keys.iter().all(|&(c, k)| r.row(id as usize)[c] == k) { ids.push(id) } false });
        Ok(())
    }
    fn next(&mut self) -> Result<()> { self.pos += 1; Ok(()) }
    fn eof(&self) -> bool { self.pos >= self.ids.len() }
    fn column(&self, ctx: &mut VContext, i: c_int) -> Result<()> {
        let s = self.store.read().unwrap();
        let r = &s.rels[self.rel];
        let v = r.row(self.ids[self.pos] as usize)[i as usize];
        ctx.set_result(v)
    }
    fn rowid(&self) -> Result<i64> { Ok(self.ids[self.pos] as i64) }
}

// ---- native join probes ---------------------------------------------------------------------------------------
//
// A join probe is the planned body of a probe rule, run natively: the planner's literal order, with the delta atom
// reading the previous wave's tuples. Its spec (hex in the `join` line) has one literal per line, then the head:
//   pos <src> <tok..>      a positive atom; <src> is `d<rel>` (delta), `c<rel>` (full), or `t:<hex table>:<hex
//                          columns>:<types>`, an input relation (a facts table, a view or a lower stratum's table)
//   neg <src> <tok..>      a negated atom (every variable bound)
//   cat <tok> <tok> <tok>  first = `@dot(second, third)`, binding the first or testing it; with the first bound and a
//                          part unbound, splitting the first (binding the unbound parts, testing a bound one)
//   eq <tok> <tok>, ne <tok> <tok> equal (binding the first if unbound) or not
//   lt <tok> <tok>, le <tok> <tok> integer order, both bound
//   count <src> <tok..> <tok>      the atom's matching tuples (its variables bound), binding or testing the last
//   head <tok..>           the head's terms (`cat:v<a>:v<b>` too)
// Tokens are `_`, `v<slot>`, `i:<int>`, `x:<hex symbol>`. Which columns of an atom are keys (bound) or bind is fixed
// here, in literal order, so each step looks up one index and never re-derives the binding pattern.
// An input relation is read once per process (`INPUTS`) with a hash index per key-column set, built on first use.
// T's `near`/`down` steps (ancestor climbs over a loaded tape) are not kept: tree axes run in the transducer.

/// `@dot(a, b)` (`core.rr`): `a.b`, and `b` when `a` is "".
/// `cat(a, b)` as a rule's head computes it, with `conn`'s symbols (the recursive-maintenance spike's evaluator).
pub fn cat_symbols(conn: &Connection, a: i64, b: i64) -> i64 {
    let syms = symbols(conn).expect("the connection's symbols");
    let mut syms = syms.write().unwrap();
    cat(&mut syms, a, b)
}

fn cat(syms: &mut Interner, a: i64, b: i64) -> i64 {
    if syms.name(a).is_empty() {
        return b;
    }
    let mut s = std::mem::take(&mut syms.buf);
    s.clear();
    syms.atoms.with_str(a as crate::content_store::atoms::Sym, |a| s.push_str(a));
    s.push('.');
    syms.atoms.with_str(b as crate::content_store::atoms::Sym, |b| s.push_str(b));
    let id = syms.id(&s);
    syms.buf = s;
    id
}

/// This thread's CPU seconds (wasm32: `prof::single_thread_cpu`).
#[cfg(target_family = "wasm")]
fn thread_cpu() -> f64 {
    crate::code_rule_dsl::prof::single_thread_cpu().as_secs_f64()
}

/// This thread's CPU seconds.
#[cfg(unix)]
pub fn thread_cpu() -> f64 {
    let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
    t.tv_sec as f64 + t.tv_nsec as f64 / 1e9
}

/// This thread's CPU seconds (user + kernel).
#[cfg(windows)]
fn thread_cpu() -> f64 {
    let (user, kernel) = crate::code_rule_dsl::prof::windows_cpu(true);
    (user + kernel).as_secs_f64()
}

/// CPU seconds and rows of input loads and index builds: [base facts (construction), lower strata's tables].
static LOADS: Mutex<[(f64, u64); 2]> = Mutex::new([(0.0, 0); 2]);

/// `set_fixpoint_loads()`: "base_cpu_ms base_rows derived_cpu_ms derived_rows". Base-fact relations and their
/// indexes are construction (the product's tape and anchor index hold them), reported apart from execution.
pub fn loads() -> String {
    let l = LOADS.lock().unwrap();
    format!("{:.1} {} {:.1} {}", l[0].0 * 1e3, l[0].1, l[1].0 * 1e3, l[1].1)
}

/// An input relation: `n` rows flat (`k` columns each), and hash indexes by key-column bitmask. `base`: read from
/// the facts (or a view of them), not from a derived relation.
/// `set`, `dead`, `ndead`: a moved relation's dedup set and dead rows (`Rel`), kept so a later run's relation can
/// start from it (`Rel::based`).
pub struct InRel { pub k: usize, n: usize, rows: Vec<i64>, pub view: Option<View>, base: bool, types: Vec<u8>, stored: usize, idx: Mutex<HashMap<u64, Arc<Csr>>>, chains: Vec<Index>,
                   /// Its columns' statistics and its orders' key sets, as its relation had them (`Rel`).
                   sketches: Vec<Sketch>, serves: Vec<Vec<Vec<usize>>>, pub dead: Vec<u64>, pub ndead: usize, summaries: scalars::Pool,
                   /// Each column's statistics (`syntax::measured`), measured when first asked: a kept relation's rows
                   /// do not change, so a session's later runs read them, not measure them again (0094 §4).
                   measured: std::sync::OnceLock<Vec<[i64; 4]>>,
                   /// Rows per value of the columns the rules test by constants (`Source::value_counts`), counted when
                   /// first asked, a column at a time, for the same reason.
                   counted: Mutex<HashMap<usize, HashMap<i64, i64>>>,

                   /// Its count tables by key columns (decision 0098 §1, `table`), built when a count step's walks have
                   /// cost as much, or ahead in a level's device wave; shared by every step that counts it so.
                   tables: Mutex<HashMap<Vec<usize>, Arc<table::Table>>>,
                   /// The relation's table, once the run holds it by name (`r:` sources).
                   name: std::sync::OnceLock<String>,
                   /// The kept relation a maintained one started as (`Rel::from`).
                   from: Option<Origin>,
                   /// Its id for the run: the executor keeps what a lane built from it by this (decision 0100 §4).
                   uid: level::Uid,
                   /// Each row's level, when the native engine derived it (`Rel::level`); empty otherwise.
                   level: Vec<u32>,
                   /// Its statistics, kept as its relation's rows came and went (`Rel::stats`).
                   stats: Option<KeptStats> }
/// A base input read in place: the bound vtab source's columns `cols`, at its rows `ids` (all rows when `None`, the
/// rows passing the atom's constant filters otherwise). Nothing of the source is copied.
pub struct View { pub src: Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, cols: Vec<usize>, ids: Option<Vec<u32>>, eqs: Vec<(usize, i64)> }
impl View {
    /// The source's rows whose view column `c` is `v` and which pass the filters, through the source's own index.
    fn find(&self, c: usize, v: i64) -> Vec<u32> {
        let rows = self.src.lookup(self.cols[c], &[v]).unwrap_or_default();
        rows.into_iter().filter(|&r| self.eqs.iter().all(|&(c, v)| self.src.value(r, c) == v)).map(|r| r as u32).collect()
    }
}
/// One row of an input relation: a slice of its flat rows, or a row of its view.
#[derive(Clone, Copy)]
pub enum RowRef<'a> { Flat(&'a [i64]), View(&'a View, usize), Source(&'a View, usize), Read([i64; READ], usize) }
/// The widest view row read whole (`RowRef::read`); a wider one is read a value at a time.
const READ: usize = 8;
impl<'a> RowRef<'a> {
    #[inline]
    pub fn at(self, c: usize) -> i64 {
        match self {
            RowRef::Flat(r) => r[c],
            RowRef::View(v, id) => v.src.value(v.ids.as_ref().map_or(id, |ids| ids[id] as usize), v.cols[c]),
            RowRef::Source(v, row) => v.src.value(row, v.cols[c]),
            RowRef::Read(vals, _) => vals[c],
        }
    }
    /// The row with its view's columns read at once (`Source::row_at`): a view of a batch finds the row's document
    /// once, not once per column a step reads.
    #[inline]
    pub fn read(self) -> RowRef<'a> {
        let (v, row) = match self {
            RowRef::View(v, id) => (v, v.ids.as_ref().map_or(id, |ids| ids[id] as usize)),
            RowRef::Source(v, row) => (v, row),
            other => return other,
        };
        if v.cols.len() > READ {
            return self;
        }
        let mut vals = [0i64; READ];
        v.src.row_at(row, &v.cols, &mut vals[..v.cols.len()]);
        RowRef::Read(vals, row)
    }
}
/// A hash index of an immutable relation on `cols`: each key's row ids are one run of `ids`, found from the key's
/// first row (`rep`) and its run (`start`, `len`). Two passes over the rows, no allocation per key.
/// Over a view whose source indexes the one key column, the source's index serves (`source`), and nothing is built.
/// Over an unfiltered view of a source with a sorted index (`sorted`), a probe is a slice of that index.
/// Over a moved relation whose waves kept an index on the key columns (`chain`), a probe walks that index's chain.
/// An input's index on key columns: its own hash table, or (`sorted`, `source`) the source's index on one key column,
/// `cols[lead]`, whose candidates the step checks against every key.
/// Over a view of a source answering several columns at once (`Source::composite`, the reach index), a probe is the
/// source's `lookup_all` (`composite`).
pub struct Csr { cols: Vec<usize>, table: HashTable<(u32, u32, u32)>, ids: Vec<u32>, source: bool, sorted: bool, chain: Option<usize>, lead: usize, composite: bool }
impl Csr {
    fn build(rel: &InRel, cols: &[usize]) -> Csr {
        // Each row's key read once (a view's values go through its source), then hashed once.
        let (k, n) = (cols.len(), rel.len());
        let mut keys: Vec<i64> = Vec::with_capacity(n * k);
        for id in 0..n {
            let row = rel.at(id);
            keys.extend(cols.iter().map(|&c| row.at(c)));
        }
        let key = |id: usize| &keys[id * k..][..k];
        let hashes: Vec<u64> = (0..n).map(|id| hash_vals(key(id).iter().copied())).collect();
        // First pass: each key's group (its entry's `.1`, for now) and size; each row's group.
        let mut table: HashTable<(u32, u32, u32)> = HashTable::new();
        let mut group = vec![0u32; n];
        let mut groups = 0u32;
        for id in 0..n {
            match table.find_mut(hashes[id], |&(rep, _, _)| key(rep as usize) == key(id)) {
                Some(e) => {
                    e.2 += 1;
                    group[id] = e.1;
                }
                None => {
                    table.insert_unique(hashes[id], (id as u32, groups, 1), |&(rep, _, _)| hashes[rep as usize]);
                    group[id] = groups;
                    groups += 1;
                }
            }
        }
        // Each group's start; then the rows placed by their group, in row order.
        let mut start = vec![0u32; groups as usize];
        for e in table.iter() {
            start[e.1 as usize] = e.2;
        }
        let mut at = 0;
        for s in &mut start {
            let len = *s;
            *s = at;
            at += len;
        }
        for e in table.iter_mut() {
            e.1 = start[e.1 as usize];
        }
        let mut ids = vec![0u32; n];
        for id in 0..n {
            let g = &mut start[group[id] as usize];
            ids[*g as usize] = id as u32;
            *g += 1;
        }
        // A dead row (`Rel::based`) is in no key's run: each run keeps its live rows, in order.
        if !rel.dead.is_empty() {
            for e in table.iter_mut() {
                let (s, l) = (e.1 as usize, e.2 as usize);
                let mut w = s;
                for r in s..s + l {
                    if !is_dead(&rel.dead, ids[r]) {
                        ids[w] = ids[r];
                        w += 1;
                    }
                }
                e.2 = (w - s) as u32;
            }
        }
        Csr { cols: cols.to_vec(), table, ids, source: false, sorted: false, chain: None, lead: 0, composite: false }
    }
    /// `key` holds the values of `cols`, in order.
    fn get<'a>(&'a self, rel: &InRel, key: &[i64]) -> &'a [u32] {
        self.table.find(hash_vals(key.iter().copied()), |&(rep, _, _)| self.cols.iter().zip(key).all(|(&c, &v)| rel.at(rep as usize).at(c) == v))
            .map_or(&[], |&(_, start, len)| &self.ids[start as usize..][..len as usize])
    }
}
/// Drops native relations `tables` of `conn`'s run: no later stratum reads them (`plan::last_reads`), so their rows
/// and the indexes the waves kept are freed now, not when the run ends.
pub fn release(conn: &Connection, tables: &[String]) {
    let at = conn.at();
    let mut derived = DERIVED.lock().unwrap();
    for t in tables {
        derived.remove(&(at, t.clone()));
    }
}

/// Drops the indexes of native relations `tables` that stay to the run's end (outputs, kept relations) but that no
/// later stratum reads: their rows are read once more, in order, and their per-column indexes were most of their
/// size. A relation shared with a reader still open keeps them.
pub fn unindex(conn: &Connection, tables: &[String]) {
    let at = conn.at();
    let mut derived = DERIVED.lock().unwrap();
    for t in tables {
        if let Some(rel) = derived.get_mut(&(at, t.clone())).and_then(Arc::get_mut) {
            rel.chains = Vec::new();
            *rel.idx.get_mut().unwrap() = HashMap::default();
        }
    }
}

/// Drops the input relations loaded on `conn`'s database that read any of `tables` (their views and indexes), and
/// the stored tables of those names: no later stratum reads them.
pub fn forget_inputs_of(conn: &Connection, tables: &[String]) {
    let path = conn_path(conn).to_string();
    let reads = |key: &InputKey| key.sql.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).any(|w| tables.iter().any(|t| t == w));
    if let Some(m) = INPUTS.lock().unwrap().get_mut(&path) { m.retain(|key, _| !reads(key)); }
    STORED.lock().unwrap().retain(|(p, t), _| *p != path || !t.rsplit('.').next().is_some_and(|n| tables.iter().any(|x| x == n)));
}

/// Drops every native relation of `conn`'s runs: a chunk's, once its exports are written to the cross-file part.
pub fn forget_derived(conn: &Connection) {
    let at = conn.at();
    DERIVED.lock().unwrap().retain(|(s, _), _| *s != at);
}

/// A native relation of `conn`'s run, for a caller that keeps it past the run with the indexes it has (a session's
/// relations, decision 0090 step 3).
pub fn held(conn: impl Scoped, table: &str) -> Option<Arc<InRel>> {
    let conn = conn.db();
    DERIVED.lock().unwrap().get(&(conn.at(), table.to_owned())).cloned()
}

/// A native relation of `conn`'s run as the caller reads an output: each live row, its columns `text` gives as the
/// symbols' text (what `sym_text` gives in SQL), the others as integers. `None`: the run holds no such relation. The
/// rows are read where the engine holds them: an output is not written to a table to be read back.
pub fn output_rows(conn: &Connection, table: &str, text: &[bool]) -> Result<Option<Vec<Vec<Value>>>> {
    let Some(rel) = held(conn, table) else { return Ok(None) };
    let syms = symbols(conn)?;
    let syms = syms.read().unwrap();
    let mut out = Vec::with_capacity(rel.rows());
    for id in 0..rel.len() {
        if is_dead(&rel.dead, id as u32) {
            continue;
        }
        let row = rel.values(id);
        let mut vals = Vec::with_capacity(row.len());
        for (c, &v) in row.iter().enumerate() {
            vals.push(if text.get(c).copied().unwrap_or(false) {
                if !syms.known(v)? {
                    return Err(err(format!("{table}: no symbol {v}")));
                }
                Value::Text(syms.name(v))
            } else {
                Value::Integer(v)
            });
        }
        out.push(vals);
    }
    Ok(Some(out))
}

/// The rows of `src`, an input bound on `conn`, as an output is read (`output_rows`): its columns `text` gives as the
/// symbols' text, the others as integers. An input the run hands on as an output is read where it is bound.
pub fn source_rows(conn: &Connection, src: &dyn crate::code_rule_dsl::facts::vtab::Source, text: &[bool]) -> Result<Vec<Vec<Value>>> {
    let syms = symbols(conn)?;
    let syms = syms.read().unwrap();
    let k = src.columns().len();
    let mut out = Vec::with_capacity(src.len());
    for row in 0..src.len() {
        let mut vals = Vec::with_capacity(k);
        for c in 0..k {
            let v = src.value(row, c);
            vals.push(if text.get(c).copied().unwrap_or(false) {
                if !syms.known(v)? {
                    return Err(err(format!("no symbol {v}")));
                }
                Value::Text(syms.name(v))
            } else {
                Value::Integer(v)
            });
        }
        out.push(vals);
    }
    Ok(out)
}

/// `finding` decoded where the engine holds it: each (check, node) with the node's document's path and the node's
/// start, from the bound `node` and `doc` inputs, as SQL's join of `finding` with them gives (each distinct).
/// `None`: the run holds no `finding`, or `node` or `doc` is not bound in place.
pub fn finding_rows(conn: &Connection) -> Result<Option<Vec<(String, String, i64)>>> {
    let Some(rel) = held(conn, "finding") else { return Ok(None) };
    let (Some(node), Some(doc)) = (crate::code_rule_dsl::facts::vtab::bound_source(conn, "node"), crate::code_rule_dsl::facts::vtab::bound_source(conn, "doc")) else { return Ok(None) };
    let col = |src: &dyn crate::code_rule_dsl::facts::vtab::Source, name: &str| src.columns().iter().position(|c| c == name);
    let (Some(nn), Some(nd), Some(ns), Some(dd), Some(dp)) = (col(node.as_ref(), "n"), col(node.as_ref(), "d"), col(node.as_ref(), "start"), col(doc.as_ref(), "d"), col(doc.as_ref(), "path")) else { return Ok(None) };
    let syms = symbols(conn)?;
    let syms = syms.read().unwrap();
    let name = |v: i64| if !syms.known(v)? { Err(err(format!("finding: no symbol {v}"))) } else { Ok(syms.name(v)) };
    let mut out = Vec::with_capacity(rel.rows());
    for id in 0..rel.len() {
        if is_dead(&rel.dead, id as u32) {
            continue;
        }
        let row = rel.values(id);
        let Some(ks) = node.lookup(nn, &[row[1]]) else { return Ok(None) };
        for k in ks {
            let Some(ds) = doc.lookup(dd, &[node.value(k, nd)]) else { return Ok(None) };
            for d in ds {
                out.push((name(row[0])?, name(doc.value(d, dp))?, node.value(k, ns)));
            }
        }
    }
    out.sort();
    out.dedup();
    Ok(Some(out))
}

/// Puts a relation kept from an earlier run into `conn`'s next run as the input `table`: its strata read it in place,
/// with the indexes it has (`r:` sources), not copied into a source and indexed again. The run's planner takes the
/// names (`adopted`).
pub fn adopt(conn: &Connection, table: &str, rel: Arc<InRel>) {
    let at = conn.at();
    let _ = rel.name.set(table.to_owned());
    DERIVED.lock().unwrap().insert((at, table.to_owned()), rel);
    ADOPTED.lock().unwrap().insert((at, table.to_owned()));
}

/// Holds `table` in `conn`'s run as `rel`, a relation it already holds under another name: a quiet stratum's
/// relation is its kept one, unchanged (decision 0101 §1), shared with its indexes.
pub fn hold_as(conn: impl Scoped, table: &str, rel: Arc<InRel>) {
    let conn = conn.db();
    DERIVED.lock().unwrap().insert((conn.at(), table.to_owned()), rel);
}

/// A native fact operator's completed set. No rows are published until a SQL consumer materializes it.
/// Unlike `adopt`, this is a result of the current run, not an input scheduled for the next one.
pub fn retain(conn: impl Scoped, table: &str, mut rows: Vec<Vec<i64>>, types: Vec<u8>) -> Result<()> {
    let conn = conn.db();
    if rows.iter().any(|r| r.len() != types.len()) { return Err(err(format!("{table}: native result width differs from its declaration"))); }
    rows.sort_unstable();
    rows.dedup();
    let mut rel = InRel::flat(&rows, types);
    Arc::get_mut(&mut rel).unwrap().stored = 0;
    DERIVED.lock().unwrap().insert((conn.at(), table.to_owned()), rel);
    Ok(())
}

/// Holds `table` in `conn`'s run as `src`'s rows, read in place (a view of every row): a relation an index holds
/// (the reach index, decision 0119) is probed through the source's own lookups, never copied into rows.
pub fn hold_source(conn: impl Scoped, table: &str, src: Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, types: Vec<u8>) {
    let conn = conn.db();
    let k = types.len();
    let view = View { src, cols: (0..k).collect(), ids: None, eqs: Vec::new() };
    let rel = Arc::new(InRel { k, n: 0, rows: Vec::new(), view: Some(view), base: false, types, stored: 0, idx: Mutex::new(HashMap::default()), chains: Vec::new(), sketches: Vec::new(), serves: Vec::new(), dead: Vec::new(), ndead: 0, summaries: Default::default(), measured: Default::default(), counted: Default::default(), tables: Default::default(), name: Default::default(), from: None, uid: Default::default(), level: Vec::new(), stats: None });
    let _ = rel.name.set(table.to_owned());
    DERIVED.lock().unwrap().insert((conn.at(), table.to_owned()), rel);
}

/// The source a relation of `conn`'s run is held as (`hold_source`), for a caller that reads the index itself.
pub fn held_source(conn: impl Scoped, table: &str) -> Option<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>> {
    let conn = conn.db();
    let rel = DERIVED.lock().unwrap().get(&(conn.at(), table.to_owned())).cloned()?;
    rel.held_source()
}

/// Holds `table` in `conn`'s run as a relation with no rows, as a component that derived none would leave it: for a
/// stratum with nothing to run, which opens no component.
pub fn hold_empty(conn: impl Scoped, table: &str, types: Vec<u8>) {
    let conn = conn.db();
    DERIVED.lock().unwrap().insert((conn.at(), table.to_owned()), InRel::flat(&[], types));
}

/// Holds `table` in `conn`'s run as a relation of `rows` (flat, `types.len()` values each): one a run kept from an
/// earlier run of the same stratum over the same inputs (decision 0077 §4), read as if this run had derived it.
pub fn hold_rows(conn: &Connection, table: &str, types: Vec<u8>, rows: Vec<i64>, n: usize) {
    let rel = InRel::kept(types, rows, n);
    let _ = rel.name.set(table.to_owned());
    DERIVED.lock().unwrap().insert((conn.at(), table.to_owned()), rel);
}

/// A native relation of `conn`'s run as it can be kept and held again (`hold_rows`): its columns' types and its live
/// rows, flat.
pub fn kept_rows(conn: &Connection, table: &str) -> Option<(Vec<u8>, Vec<i64>, usize)> {
    let rel = DERIVED.lock().unwrap().get(&(conn.at(), table.to_owned())).cloned()?;
    if rel.view.is_some() {
        return None;
    }
    let k = rel.k;
    let mut out = Vec::with_capacity(rel.rows() * k);
    for id in 0..rel.len() as u32 {
        if rel.ndead > 0 && is_dead(&rel.dead, id) {
            continue;
        }
        out.extend_from_slice(&rel.rows[id as usize * k..id as usize * k + k]);
    }
    Some((rel.types.clone(), out, rel.rows()))
}

/// The relations `adopt`ed into `conn`'s next run, taken by its planner.
pub fn adopted(conn: &Connection) -> std::collections::BTreeSet<String> {
    let at = conn.at();
    let mut a = ADOPTED.lock().unwrap();
    let mine: std::collections::BTreeSet<String> = a.iter().filter(|(s, _)| *s == at).map(|(_, t)| t.clone()).collect();
    a.retain(|(s, _)| *s != at);
    mine
}
static ADOPTED: Mutex<std::collections::BTreeSet<(usize, String)>> = Mutex::new(std::collections::BTreeSet::new());

impl InRel {
    /// A relation of `rows` (flat, `types.len()` values each) an earlier run kept (decision 0077 §4), in no table.
    pub fn kept(types: Vec<u8>, rows: Vec<i64>, n: usize) -> Arc<InRel> {
        let k = types.len();
        Arc::new(InRel { k, n, rows, view: None, base: false, types, stored: 0, idx: Mutex::new(HashMap::default()), chains: Vec::new(), sketches: Vec::new(), serves: Vec::new(), dead: Vec::new(), ndead: 0, summaries: Default::default(), measured: Default::default(), counted: Default::default(), tables: Default::default(), name: Default::default(), from: None, uid: Default::default(), level: Vec::new(), stats: None })
    }
    /// Its columns' value types (the engine's codes).
    pub fn types(&self) -> &[u8] {
        &self.types
    }
    /// A relation of `rows` (each of `types.len()` values), for one a run left in a table rather than in the engine.
    pub fn flat(rows: &[Vec<i64>], types: Vec<u8>) -> Arc<InRel> {
        let k = types.len();
        let n = rows.len();
        let flat: Vec<i64> = rows.iter().flat_map(|r| r.iter().copied()).collect();
        Arc::new(InRel { k, n, rows: flat, view: None, base: false, types, stored: n, idx: Mutex::new(HashMap::default()), chains: Vec::new(), sketches: Vec::new(), serves: Vec::new(), dead: Vec::new(), ndead: 0, summaries: Default::default(), measured: Default::default(), counted: Default::default(), tables: Default::default(), name: Default::default(), from: None, uid: Default::default(), level: Vec::new(), stats: None })
    }
    /// A relation of `n` rows, flat (`types.len()` values each), as a stored frame gives them (t33 K4): read as a
    /// relation the run derived.
    pub fn from_flat(rows: Vec<i64>, n: usize, types: Vec<u8>) -> Arc<InRel> {
        let k = types.len();
        Arc::new(InRel { k, n, rows, view: None, base: false, types, stored: 0, idx: Mutex::new(HashMap::default()), chains: Vec::new(), sketches: Vec::new(), serves: Vec::new(), dead: Vec::new(), ndead: 0, summaries: Default::default(), measured: Default::default(), counted: Default::default(), tables: Default::default(), name: Default::default(), from: None, uid: Default::default(), level: Vec::new(), stats: None })
    }
    /// `from_flat`'s relation with each row's level (decision 0090), as a frame of its levels gives them: `None` when
    /// they are not one per row.
    pub fn from_flat_levels(rows: Vec<i64>, n: usize, types: Vec<u8>, level: Vec<u32>) -> Option<Arc<InRel>> {
        if level.len() != n {
            return None;
        }
        let mut rel = Arc::into_inner(Self::from_flat(rows, n, types)).expect("a new relation");
        rel.level = level;
        Some(Arc::new(rel))
    }
    /// Its live rows' levels, in `live_rows`' order: `None` unless every row has one (`has_levels`).
    pub fn live_levels(&self) -> Option<Vec<u32>> {
        if !self.has_levels() {
            return None;
        }
        Some((0..self.n).filter(|&id| self.ndead == 0 || !is_dead(&self.dead, id as u32)).map(|id| self.level[id]).collect())
    }
    /// Its rows, as a source of columns `names`.
    pub fn source(self: &Arc<Self>, names: Vec<String>) -> Arc<dyn crate::code_rule_dsl::facts::vtab::Source> {
        Derived::new(Arc::clone(self), Some(names))
    }
    /// Its rows' bytes and their levels' (decision 0090, 4 bytes a row); its indexes not counted.
    pub fn bytes(&self) -> usize {
        self.rows.len() * 8 + self.level.len() * 4
    }
    /// What it holds in memory, by part: its values, its rows' levels, and its dedup set, dead bits and hash
    /// indexes (each by capacity, the bytes allocated).
    /// The index-cost spike (`RERAIL_INDEX_SPIKE`): its probed key-column sets `shapes` answered by column orders
    /// (`ordered`) and by today's per-column chains, timed on keys of its own rows (every `stride`th), with the
    /// counts checked equal; and its dedup test by the set and by the order holding every column. A line, and the
    /// orders' bytes; `None` for a view.
    /// The index-cost spike's growth (`RERAIL_INDEX_GROW`): the relation's rows again, wave by wave (by the level each
    /// row was pushed at; 4096 rows a wave without levels), with as many duplicates offered as its derivations had
    /// (`RERAIL_SPIKE_DERIVATIONS`), into today's dedup set and chains (`Rel::push`, the engine's own) and into the
    /// mixed shape grown (`Growing` for the whole row and `hot`, `Runs` for the `orders`, each wave's run its delta).
    fn grow_spike(&self, name: &str, hot: &[Vec<usize>], orders: Vec<Vec<usize>>, rest: &[Vec<usize>], rest_counts: &[u64]) -> String {
        let (k, n, rows) = (self.k, self.n, &self.rows);
        // Waves: runs of one level, or of 4096 rows.
        // The rows a maintained run added to the kept relation it started as (`Origin::Based`), else the last wave:
        // the delta path, timed apart.
        let based = match &self.from { Some(Origin::Based { n: old, .. }) if *old < n => Some(*old), _ => None };
        let mut waves: Vec<(usize, usize)> = Vec::new();
        let mut lo = 0;
        for id in 1..=n {
            let cut = id == n || Some(id) == based || if self.level.len() == n { self.level[id] != self.level[lo] } else { id - lo >= 4096 };
            if cut {
                waves.push((lo, id));
                lo = id;
            }
        }
        // Duplicates offered: (derived - new) / new of the relation's derivations, spread evenly, each a row of an
        // earlier wave.
        let dup = DERIVATIONS.lock().unwrap().get(name).map_or(0.0, |&(d, nw)| if nw > 0 { d.saturating_sub(nw) as f64 / nw as f64 } else { 0.0 });
        let mut offers: Vec<Vec<u32>> = Vec::with_capacity(waves.len());
        let mut owed = 0.0f64;
        let mut seed = 0x9e37_79b9u64;
        for &(lo, hi) in &waves {
            let mut o: Vec<u32> = (lo as u32..hi as u32).collect();
            owed += dup * (hi - lo) as f64;
            while owed >= 1.0 && hi > 0 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                o.push(((seed >> 33) % hi as u64) as u32);
                owed -= 1.0;
            }
            offers.push(o);
        }
        let offered: usize = offers.iter().map(Vec::len).sum();
        let split = based.unwrap_or_else(|| waves.last().map_or(0, |w| w.0));
        let in_delta = |wi: usize| waves[wi].0 >= split;
        let delta_offers: usize = offers.iter().enumerate().filter(|(i, _)| in_delta(*i)).map(|(_, o)| o.len()).sum();
        // Today: a fresh relation with the kept one's composite chains, the engine's own push.
        let t = Instant::now();
        let mut now = Rel::new(name, &self.types);
        now.leveled = false;
        for ix in &self.chains {
            now.want(&ix.cols);
        }
        let mut held_now = 0usize;
        let (mut now_delta, mut now_copy) = (0.0f64, 0.0f64);
        for (wi, o) in offers.iter().enumerate() {
            if wi > 0 && in_delta(wi) && !in_delta(wi - 1) {
                // Today a maintained run starts from a copy of the kept relation's set and chains (`Rel::based`).
                let t = Instant::now();
                std::hint::black_box(now.idx.clone());
                now_copy = t.elapsed().as_secs_f64() * 1e3;
            }
            let tw = Instant::now();
            let lo = now.n;
            now.reserve(o.len());
            for &id in o {
                held_now += usize::from(now.push(&rows[id as usize * k..][..k]));
            }
            let hi = now.n;
            now.set_delta(lo, hi);
            if in_delta(wi) {
                now_delta += tw.elapsed().as_secs_f64() * 1e3;
            }
        }
        let now_ms = t.elapsed().as_secs_f64() * 1e3;
        let now_bytes = now.idx.iter().map(Index::bytes).sum::<usize>();
        drop(now);
        // The mixed shape. Ids are the kept relation's: a duplicate offer is an earlier row's id again.
        let id_bits = 32 - (n as u32 + 1).leading_zeros();
        let t = Instant::now();
        let mut set = ordered::Growing::new((0..k).collect(), id_bits);
        let mut placed: Vec<ordered::Growing> = hot.iter().map(|c| ordered::Growing::new(c.clone(), id_bits)).collect();
        let fanout: usize = std::env::var("RERAIL_RUNS_FANOUT").ok().and_then(|v| v.parse().ok()).unwrap_or(2);
        let prefix: Vec<usize> = orders.iter().map(|o| rest.iter().filter(|s| o[..s.len()].iter().all(|c| s.contains(c))).map(Vec::len).max().unwrap_or(1)).collect();
        let mut runs: Vec<ordered::Runs> = orders.iter().cloned().map(|o| ordered::Runs::with_fanout(o, fanout)).collect();
        let mut waves_fresh: Vec<Vec<u32>> = Vec::new();
        let mut held_new = 0usize;
        let (mut t_set, mut t_placed, mut t_runs) = (0.0f64, 0.0f64, 0.0f64);
        let mut seen = vec![0u64; n.div_ceil(64)];
        let (mut new_delta, mut new_copy) = (0.0f64, 0.0f64);
        for (wi, o) in offers.iter().enumerate() {
            if wi > 0 && in_delta(wi) && !in_delta(wi - 1) {
                let t = Instant::now();
                std::hint::black_box((set.clone_slots(), placed.iter().map(ordered::Growing::clone_slots).collect::<Vec<_>>(), runs.iter().map(|r| r.runs.clone()).collect::<Vec<_>>()));
                new_copy = t.elapsed().as_secs_f64() * 1e3;
            }
            let t0 = Instant::now();
            // The wave's rows by hash, so the inserts walk the table forward.
            let mut hs: Vec<(u64, u32)> = o.iter().map(|&id| (ordered::key_hash(rows[id as usize * k..][..k].iter().copied()), id)).collect();
            hs.sort_unstable();
            set.reserve(rows, k, hs.len());
            let mut fresh: Vec<u32> = Vec::with_capacity(hs.len());
            for &(h, id) in &hs {
                // A duplicate offer names an earlier row; its own id was placed then.
                let first = seen[id as usize / 64] & (1 << (id % 64)) == 0;
                if first && set.insert(rows, k, id, h, true) {
                    seen[id as usize / 64] |= 1 << (id % 64);
                    fresh.push(id);
                } else if !first {
                    // Probe only: found, so not inserted (`insert` returns false before placing a duplicate).
                    std::hint::black_box(set.insert_probe(rows, k, id, h));
                }
            }
            held_new += fresh.len();
            let t1 = Instant::now();
            for p in &mut placed {
                let mut hs: Vec<(u64, u32)> = fresh.iter().map(|&id| (ordered::key_hash(p.cols.iter().map(|&c| rows[id as usize * k + c])), id)).collect();
                hs.sort_unstable();
                p.reserve(rows, k, hs.len());
                for &(h, id) in &hs {
                    p.insert(rows, k, id, h, false);
                }
            }
            let t2 = Instant::now();
            for r in &mut runs {
                r.add_wave(rows, k, fresh.clone());
            }
            waves_fresh.push(fresh);
            t_runs += t2.elapsed().as_secs_f64();
            t_placed += (t2 - t1).as_secs_f64();
            t_set += (t1 - t0).as_secs_f64();
            if in_delta(wi) {
                new_delta += t0.elapsed().as_secs_f64() * 1e3;
            }
        }
        let new_ms = t.elapsed().as_secs_f64() * 1e3;
        // The orders grown again four ways, each timed, with a probe's time over what it ends as (each order's
        // longest covered key, at sampled rows) and the step's probes of the order (its covered sets' counts):
        // merged as runs come (fanout 2), the same with each id's tag, never merged (a run a wave), and that too
        // with tags. A run never merged is merged once (timed) where the step's probes would cost more over its
        // runs than over one run and the merge.
        let mut variants = String::new();
        if !orders.is_empty() {
            let live: Vec<u32> = (0..n as u32).filter(|&id| !is_dead(&self.dead, id)).collect();
            let sample: Vec<u32> = live.iter().step_by((live.len() / 2048).max(1)).copied().collect();
            // Each covered set's probes go to the first order it is a prefix of.
            let mut probes = vec![0u64; orders.len()];
            for (s, &c) in rest.iter().zip(rest_counts) {
                if let Some(i) = orders.iter().position(|o| o[..s.len()].iter().all(|c| s.contains(c))) {
                    probes[i] += c;
                }
            }
            let keys = |order: &[usize], len: usize| -> Vec<Vec<i64>> {
                sample.iter().map(|&id| { let r = self.row(id as usize); order[..len].iter().map(|&c| r[c]).collect() }).collect()
            };
            let best_ns = |f: &dyn Fn() -> ()| -> f64 {
                let mut best = f64::MAX;
                for _ in 0..3 {
                    let t = Instant::now();
                    f();
                    best = best.min(t.elapsed().as_nanos() as f64 / sample.len().max(1) as f64);
                }
                best
            };
            // (grow ms, probe-weighted ms over what it ends as, merge-once ms or 0, probe ns, rows read by merges)
            let mut sums = [(0.0f64, 0.0f64, 0.0f64, 0.0f64, 0usize); 6];
            for (oi, order) in orders.iter().enumerate() {
                let len = prefix[oi];
                let ks = keys(order, len);
                let w = probes[oi] as f64;
                // 0: merged as they come.
                let t = Instant::now();
                let mut eager = ordered::Runs::with_fanout(order.clone(), 2);
                for f in &waves_fresh { eager.add_wave(rows, k, f.clone()); }
                let grow = t.elapsed().as_secs_f64() * 1e3;
                let ns = best_ns(&|| for key in &ks { std::hint::black_box(eager.count(rows, k, key)); });
                sums[0] = (sums[0].0 + grow, sums[0].1 + w * ns / 1e6, 0.0, sums[0].3 + ns, sums[0].4 + eager.merged);
                // 1: the same with tags.
                let t = Instant::now();
                let mut tagged = ordered::TaggedRuns::with_fanout(order.clone(), 2);
                for f in &waves_fresh { tagged.add_wave(rows, k, f.clone()); }
                let grow = t.elapsed().as_secs_f64() * 1e3;
                let ns = best_ns(&|| for key in &ks { std::hint::black_box(tagged.count(rows, k, key)); });
                sums[1] = (sums[1].0 + grow, sums[1].1 + w * ns / 1e6, 0.0, sums[1].3 + ns, sums[1].4 + tagged.row_compares);
                // 4: the same, its probes from the tag's interpolated place (mldb's PathIndex); the merges' tag
                // collisions (ties of different leading values).
                let ns = best_ns(&|| for key in &ks { std::hint::black_box(tagged.count_interp(rows, k, key)); });
                sums[4] = (sums[4].0 + grow, sums[4].1 + w * ns / 1e6, 0.0, sums[4].3 + ns, sums[4].4 + tagged.collisions);
                // 2: never merged; merged once where its probes pay for it.
                let t = Instant::now();
                let mut lazy = ordered::Runs::with_fanout(order.clone(), usize::MAX);
                for f in waves_fresh.iter().filter(|f| !f.is_empty()) { lazy.add_wave(rows, k, f.clone()); }
                let grow = t.elapsed().as_secs_f64() * 1e3;
                let ns_runs = best_ns(&|| for key in &ks { std::hint::black_box(lazy.count(rows, k, key)); });
                let t = Instant::now();
                let one = lazy.merged_all(rows, k);
                let merge_ms = t.elapsed().as_secs_f64() * 1e3;
                let ns_one = best_ns(&|| for key in &ks { std::hint::black_box(one.count(rows, k, key)); });
                let (over_runs, merged_then) = (w * ns_runs / 1e6, merge_ms + w * ns_one / 1e6);
                let (probe_ms, merge_ms, ns) = if merged_then < over_runs { (w * ns_one / 1e6, merge_ms, ns_one) } else { (over_runs, 0.0, ns_runs) };
                sums[2] = (sums[2].0 + grow, sums[2].1 + probe_ms, sums[2].2 + merge_ms, sums[2].3 + ns, sums[2].4 + usize::from(merge_ms > 0.0));
                // 3: never merged, tagged; merged once where its probes pay for it.
                let t = Instant::now();
                let mut lazy_t = ordered::TaggedRuns::with_fanout(order.clone(), usize::MAX);
                for f in waves_fresh.iter().filter(|f| !f.is_empty()) { lazy_t.add_wave(rows, k, f.clone()); }
                let grow = t.elapsed().as_secs_f64() * 1e3;
                let ns_runs = best_ns(&|| for key in &ks { std::hint::black_box(lazy_t.count(rows, k, key)); });
                let t = Instant::now();
                let one = lazy_t.merged_all(rows, k);
                let merge_ms = t.elapsed().as_secs_f64() * 1e3;
                let merge_ms_t = merge_ms;
                let ns_one = best_ns(&|| for key in &ks { std::hint::black_box(one.count(rows, k, key)); });
                let (over_runs, merged_then) = (w * ns_runs / 1e6, merge_ms + w * ns_one / 1e6);
                let (probe_ms, merge_ms, ns) = if merged_then < over_runs { (w * ns_one / 1e6, merge_ms, ns_one) } else { (over_runs, 0.0, ns_runs) };
                // 5: the same, probed from the tags' interpolated places.
                let merge_once = merge_ms_t;
                let ns_runs = best_ns(&|| for key in &ks { std::hint::black_box(lazy_t.count_interp(rows, k, key)); });
                let ns_one = best_ns(&|| for key in &ks { std::hint::black_box(one.count_interp(rows, k, key)); });
                let (over_runs, merged_then) = (w * ns_runs / 1e6, merge_once + w * ns_one / 1e6);
                let (probe_i, merge_i, ns_i) = if merged_then < over_runs { (w * ns_one / 1e6, merge_once, ns_one) } else { (over_runs, 0.0, ns_runs) };
                sums[5] = (sums[5].0 + grow, sums[5].1 + probe_i, sums[5].2 + merge_i, sums[5].3 + ns_i, sums[5].4 + usize::from(merge_i > 0.0));
                sums[3] = (sums[3].0 + grow, sums[3].1 + probe_ms, sums[3].2 + merge_ms, sums[3].3 + ns, sums[3].4 + usize::from(merge_ms > 0.0));
            }
            let unprobed = probes.iter().filter(|&&p| p == 0).count();
            VARIANTS.with(|v| {
                let mut v = v.borrow_mut();
                for (i, x) in sums.iter().enumerate() {
                    v.0[i].0 += x.0;
                    v.0[i].1 += x.1;
                    v.0[i].2 += x.2;
                    v.0[i].3 += x.3;
                    v.0[i].4 += x.4;
                }
                v.1 += orders.len();
                v.2 += unprobed;
            });
            variants = format!(
                "; orders: eager {:.1}+{:.1} ms, tagged {:.1}+{:.1} ms, lazy {:.1}+{:.1}+{:.1} ms, lazy tagged {:.1}+{:.1}+{:.1} ms (grow + probes [+ merge once]; {unprobed} of {} orders unprobed)",
                sums[0].0, sums[0].1, sums[1].0, sums[1].1, sums[2].0, sums[2].2, sums[2].1, sums[3].0, sums[3].2, sums[3].1, orders.len()
            );
        }
        let new_bytes = set.bytes() + placed.iter().map(ordered::Growing::bytes).sum::<usize>() + runs.iter().map(ordered::Runs::bytes).sum::<usize>();
        GROWN.with(|g| { let mut g = g.borrow_mut(); g.0 += now_ms; g.1 += new_ms; g.2 += offered; g.3 += held_now; g.4 += now_bytes; g.5 += new_bytes; g.6 += t_set * 1e3; g.7 += t_placed * 1e3; g.8 += t_runs * 1e3; g.9 += delta_offers; g.10 += now_delta; g.11 += now_copy; g.12 += new_delta; g.13 += new_copy; g.14 += set.rehashed + placed.iter().map(|p| p.rehashed).sum::<usize>(); g.15 += usize::from(based.is_some()); });
        format!(
            "grown over {} waves, {offered} offers ({dup:.2} duplicates a row): today {now_ms:.1} ms {:.1} B/row ({held_now} held), mixed {new_ms:.1} ms = set {:.1} + {} placed {:.1} + {} orders {:.1} ms, {:.1} B/row ({held_new} held; set grew {}x, {} rows read again, runs {:?}); delta {delta_offers} offers{}: today {now_delta:.2} ms + copy {now_copy:.2}, mixed {new_delta:.2} ms + copy {new_copy:.2}",
            waves.len(),
            now_bytes as f64 / held_now.max(1) as f64,
            t_set * 1e3,
            placed.len(),
            t_placed * 1e3,
            runs.len(),
            t_runs * 1e3,
            new_bytes as f64 / held_new.max(1) as f64,
            set.grew,
            set.rehashed,
            runs.iter().map(|r| r.runs.len()).collect::<Vec<_>>(),
            if based.is_some() { " (a maintained run's)" } else { " (the last wave)" }
        ) + &variants
    }
    /// The spike's growth totals since the last take: today's ms, the mixed ms, offers, rows held, today's bytes, the
    /// mixed bytes, and the mixed ms by part (set, placed, orders).
    /// The orders' totals by variant since the last take (`VARIANTS`).
    pub fn index_spike_variants() -> ([(f64, f64, f64, f64, usize); 6], usize, usize) {
        VARIANTS.with(|f| std::mem::take(&mut *f.borrow_mut()))
    }
    pub fn index_spike_grown() -> Grown {
        GROWN.with(|g| std::mem::replace(&mut *g.borrow_mut(), GROWN_ZERO))
    }
    /// The spike's mixed shape's bytes and probe-weighted ns, summed over the relations measured since the last take.
    pub fn index_spike_mixed() -> (usize, f64) {
        MIXED.with(|m| std::mem::take(&mut *m.borrow_mut()))
    }
    pub fn index_spike(&self, name: &str, shapes: &[Vec<usize>], counts: &[u64]) -> Option<(String, usize, usize, usize, f64, f64, f64)> {
        if self.view.is_some() || self.k == 0 || self.n == 0 {
            return None;
        }
        let (k, n, rows) = (self.k, self.n, &self.rows);
        let t = Instant::now();
        let cover = ordered::cover(shapes, k);
        let orders: Vec<ordered::Ordered> = cover.iter().map(|o| ordered::Ordered::build(rows, k, n, |id| is_dead(&self.dead, id), o.clone(), false)).collect();
        let build_ms = t.elapsed().as_secs_f64() * 1e3;
        let load: f64 = std::env::var("RERAIL_PLACED_LOAD").ok().and_then(|v| v.parse().ok()).unwrap_or(0.75);
        let t = Instant::now();
        let mut placed_sets: Vec<Vec<usize>> = shapes.to_vec();
        placed_sets.push((0..k).collect());
        let placed: Vec<ordered::Placed> = placed_sets.iter().map(|c| ordered::Placed::build(rows, k, n, |id| is_dead(&self.dead, id), c.clone(), load)).collect();
        let hashed_ms = t.elapsed().as_secs_f64() * 1e3;
        let placed_bytes: usize = placed.iter().map(ordered::Placed::bytes).sum();
        // Placed for dedup only, the orders for the probes.
        let dedup_placed = placed.last().map_or(0, ordered::Placed::bytes);
        let bytes: usize = orders.iter().map(ordered::Ordered::bytes).sum();
        let now = self.chains.iter().map(Index::bytes).sum::<usize>();
        let live: Vec<u32> = (0..n as u32).filter(|&id| !is_dead(&self.dead, id)).collect();
        let stride = (live.len() / 4096).max(1);
        let sample: Vec<u32> = live.iter().step_by(stride).copied().collect();
        let mut parts = Vec::new();
        let (mut w_now, mut w_new, mut w_hash) = (0.0f64, 0.0f64, 0.0f64);
        let mut checks = shapes.to_vec();
        checks.push((0..k).collect());
        let mut measured: Vec<(Vec<usize>, bool, f64, f64, u64)> = Vec::new();
        for (si, shape) in checks.iter().enumerate() {
            let dedup = si == shapes.len();
            let Some((oi, len)) = orders.iter().enumerate().find_map(|(i, o)| o.prefix(shape).map(|l| (i, l))) else { continue };
            let o = &orders[oi];
            // The new: the order's range on the key in the order's column order.
            let time = |o: &ordered::Ordered| -> (f64, usize) {
                let (mut ns, mut got) = (f64::MAX, 0usize);
                for _ in 0..3 {
                    let t = Instant::now();
                    got = 0;
                    let mut key = vec![0i64; len];
                    for &id in &sample {
                        let r = self.row(id as usize);
                        for (j, &c) in o.order[..len].iter().enumerate() { key[j] = r[c]; }
                        got += std::hint::black_box(o.range(rows, k, &key)).len();
                    }
                    ns = ns.min(t.elapsed().as_nanos() as f64 / sample.len() as f64);
                }
                (ns, got)
            };
            let (new_ns, got_new) = time(o);
            let pl = &placed[si];
            let (mut hash_ns, mut got_hash) = (f64::MAX, 0usize);
            for _ in 0..3 {
                let t = Instant::now();
                got_hash = 0;
                let mut key = vec![0i64; pl.cols.len()];
                for &id in &sample {
                    let r = self.row(id as usize);
                    for (j, &c) in pl.cols.iter().enumerate() { key[j] = r[c]; }
                    pl.each(rows, k, &key, |x| { std::hint::black_box(x); got_hash += 1 });
                }
                hash_ns = hash_ns.min(t.elapsed().as_nanos() as f64 / sample.len() as f64);
            }
            // Today's: the dedup set for the whole row; else a chain on exactly the key's columns, or the shortest
            // single-column chain, walked and filtered on the key.
            let (mut now_ns, mut got_now) = (f64::MAX, 0usize);
            for _ in 0..3 {
            let t = Instant::now();
            got_now = 0;
            for &id in &sample {
                let r = self.row(id as usize);
                if dedup {
                    got_now += usize::from(find_row(&self.chains, rows, k, r, |x| is_dead(&self.dead, x)).is_some());
                    continue;
                }
                let exact = self.chains.iter().find(|ix| ix.cols == *shape);
                let pick = exact.map(|ix| ix.find(rows, k, &shape.iter().map(|&c| r[c]).collect::<Vec<_>>(), None)).or_else(|| {
                    shape.iter().filter_map(|&c| self.chains.iter().find(|ix| ix.cols == [c])).map(|ix| ix.find(rows, k, &[r[ix.cols[0]]], None)).min_by_key(runs::Found::len)
                });
                let Some(found) = pick else { continue };
                for at in found.parts().flatten().copied() {
                    let x = self.row(at as usize);
                    if !is_dead(&self.dead, at) && shape.iter().all(|&c| x[c] == r[c]) { got_now += 1; }
                }
            }
            now_ns = now_ns.min(t.elapsed().as_nanos() as f64 / sample.len() as f64);
            }
            measured.push((shape.clone(), dedup, new_ns, hash_ns, counts.get(si).copied().unwrap_or(0)));
            let same = got_now == got_new && got_now == got_hash;
            let w = counts.get(si).copied().unwrap_or(0) as f64;
            (w_now, w_new, w_hash) = (w_now + w * now_ns, w_new + w * new_ns, w_hash + w * hash_ns);
            parts.push(format!("{}{shape:?}: {now_ns:.0} ns now, {new_ns:.0} fenced, {hash_ns:.0} placed{}", if dedup { "dedup " } else { "" }, if same { String::new() } else { format!(" (COUNTS DIFFER: {got_now} vs {got_new} vs {got_hash})") }));
        }
        // The mixed shape: the whole row placed (dedup); each probed set probed at least once a live row a step
        // whose placed index is faster than its order placed too; the rest covered by orders.
        let live_n = live.len().max(1);
        let mut mixed_bytes = dedup_placed;
        let mut mixed_ns = 0.0f64;
        let mut rest: Vec<Vec<usize>> = Vec::new();
        for (shape, dedup, fenced, placed_ns, count) in &measured {
            if *dedup {
                continue;
            }
            let hot = *count as usize >= live_n && placed_ns < fenced;
            if hot {
                mixed_bytes += placed.iter().find(|p| p.cols == *shape).map_or(0, ordered::Placed::bytes);
                mixed_ns += *count as f64 * placed_ns;
            } else {
                rest.push(shape.clone());
                mixed_ns += *count as f64 * fenced;
            }
        }
        if !rest.is_empty() {
            let mut chains = ordered::cover(&rest, k);
            // The whole row is placed already: an order for it alone is not needed.
            chains.retain(|o| rest.iter().any(|s| o[..s.len()].iter().all(|c| s.contains(c))));
            mixed_bytes += chains.len() * live_n * 4 + chains.len() * (live_n / ordered::FENCE + 1) * 8;
        }
        MIXED.with(|m| { let mut m = m.borrow_mut(); m.0 += mixed_bytes; m.1 += mixed_ns; });
        if std::env::var_os("RERAIL_INDEX_GROW").is_some() {
            let hot: Vec<Vec<usize>> = measured.iter().filter(|(_, d, f, p, c)| !*d && *c as usize >= live_n && p < f).map(|m| m.0.clone()).collect();
            let mut chains = ordered::cover(&rest, k);
            chains.retain(|o| rest.iter().any(|s| o[..s.len()].iter().all(|c| s.contains(c))));
            let rest_counts: Vec<u64> = rest.iter().map(|r| measured.iter().find(|m| !m.1 && m.0 == *r).map_or(0, |m| m.4)).collect();
            parts.push(self.grow_spike(name, &hot, chains, &rest, &rest_counts));
        }
        parts.push(format!("mixed {:.1} B/row", mixed_bytes as f64 / live_n as f64));
        Some((
            format!(
                "  SPIKE {name}: {} rows, k {k}; {} orders {:?}, {:.2} MB ({:.1} B/row) vs sets and chains {:.2} MB ({:.1} B/row); built in {build_ms:.1} ms fenced, {hashed_ms:.1} ms placed; placed on every set {:.1} B/row, on the whole row {:.1} B/row; {}",
                live.len(),
                orders.len(),
                orders.iter().map(|o| o.order.clone()).collect::<Vec<_>>(),
                bytes as f64 / 1e6,
                bytes as f64 / live.len().max(1) as f64,
                now as f64 / 1e6,
                now as f64 / live.len().max(1) as f64,
                placed_bytes as f64 / live.len().max(1) as f64,
                dedup_placed as f64 / live.len().max(1) as f64,
                parts.join("; ")
            ),
            bytes,
            placed_bytes,
            dedup_placed,
            w_now,
            w_new,
            w_hash,
        ))
    }
    /// For the index-cost spike (`RERAIL_INDEX_REPORT`): its columns, its dedup set's bytes, and each kept chain
    /// index's columns, keys and bytes; then the lazily built key indexes' columns and bytes.
    pub fn index_detail(&self) -> (usize, usize, Vec<(Vec<usize>, usize, usize)>, Vec<(Vec<usize>, usize)>) {
        let set = self.sketches.iter().map(Sketch::bytes).sum::<usize>() + self.dead.capacity() * 8;
        let chains = self.chains.iter().map(|ix| (ix.cols.clone(), ix.runs(), ix.bytes())).collect();
        let csr = self.idx.lock().map(|m| m.values().map(|c| (c.cols.clone(), c.ids.capacity() * 4 + c.table.capacity() * (12 + 1))).collect()).unwrap_or_default();
        (self.k, set, chains, csr)
    }
    pub fn held_bytes(&self) -> (usize, usize, usize) {
        let values = self.rows.capacity() * 8;
        let levels = self.level.capacity() * 4;
        // A hash table's entry and its control byte.
        let set = self.sketches.iter().map(Sketch::bytes).sum::<usize>() + self.dead.capacity() * 8;
        let chains: usize = self.chains.iter().map(Index::bytes).sum();
        (values, levels, set + chains)
    }
    pub fn rows(&self) -> usize {
        self.len() - self.ndead
    }
    /// Whether none of its rows are in its table: a reader of it sees them all only in the engine.
    pub fn none_stored(&self) -> bool {
        self.stored == 0
    }
}

/// A level's device wave (decision 0095 §5, 0098 §6): the count tables its strata read (`wants`, by relation and key
/// columns) that an earlier chunk found worth building and that this run holds, built now as one batch. Returns the
/// launches and the relations they read.
pub fn prefetch_tables(conn: &Connection, wants: &std::collections::BTreeSet<(String, Vec<usize>)>) -> Result<(usize, usize)> {
    let at = conn.at();
    table::prefetch(wants, |name| DERIVED.lock().unwrap().get(&(at, name.to_owned())).cloned()).map_err(err)
}

/// The index-cost spike's growth totals (`InRel::grow_spike`): today's ms, the mixed ms, offers, rows held, today's
/// bytes, the mixed bytes, the mixed ms by part (set, placed, orders); the delta path's offers, today's ms and copy
/// ms, the mixed ms and copy ms; rows read again by doublings; relations with a maintained run's delta.
pub type Grown = (f64, f64, usize, usize, usize, usize, f64, f64, f64, usize, f64, f64, f64, f64, usize, usize);
const GROWN_ZERO: Grown = (0.0, 0.0, 0, 0, 0, 0, 0.0, 0.0, 0.0, 0, 0.0, 0.0, 0.0, 0.0, 0, 0);

thread_local! {
    /// The index-cost spike's mixed shape totals (`InRel::index_spike`).
    static MIXED: std::cell::RefCell<(usize, f64)> = const { std::cell::RefCell::new((0, 0.0)) };
    /// Its growth totals (`InRel::grow_spike`).
    static GROWN: std::cell::RefCell<Grown> = const { std::cell::RefCell::new(GROWN_ZERO) };
    /// The orders grown four ways (`InRel::grow_spike`: eager, tagged, lazy, lazy tagged): grow ms, probe-weighted
    /// ms, merge-once ms, probe ns summed over orders, and rows read by merges (eager, tagged) or orders merged once
    /// (lazy); then orders, and orders no probe read.
    static VARIANTS: std::cell::RefCell<([(f64, f64, f64, f64, usize); 6], usize, usize)> = const { std::cell::RefCell::new(([(0.0, 0.0, 0.0, 0.0, 0); 6], 0, 0)) };
}

/// Tests: this thread's count steps build their tables at their first probe.
#[cfg(any(test, feature = "test-hooks"))]
pub fn count_tables_at_once(on: bool) {
    table::AT_ONCE.with(|c| c.set(on));
}

/// The count tables built so far in this process: histograms, counts per row, and the rows they read.
pub fn tables_built() -> (u64, u64, u64) {
    let b = table::built();
    (b.histograms, b.per_row, b.rows)
}

/// The relations of components run with `nowrite`, by table, until they are materialized.
static DERIVED: LazyLock<Mutex<HashMap<(usize, String), Arc<InRel>>>> = LazyLock::new(Default::default);
const SEED: usize = usize::MAX;
/// Columns with at most this many values report each value's postings (`stats`).

/// A native relation's rows, in place (for a caller that keeps an output in the engine): a source of columns
/// `c0`, `c1`, ….
pub fn derived(conn: impl Scoped, table: &str) -> Option<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>> {
    let conn = conn.db();
    let rel = DERIVED.lock().unwrap().get(&(conn.at(), table.to_owned())).cloned()?;
    Some(Derived::new(rel, None))
}
/// A native relation's rows as a source; `live`, a relation with dead rows' live row ids (dense, as a source's are);
/// `orders`, each column's rows (the source's) sorted by value, built on first probe: SQL that reads a kept relation
/// (a session's, or one a new process read back, t32 4.1) probes it as it probes any held source, not by a scan.
struct Derived(Arc<InRel>, Option<Vec<String>>, Option<Vec<u32>>, Vec<std::sync::OnceLock<Vec<u32>>>);
impl Derived {
    fn new(rel: Arc<InRel>, names: Option<Vec<String>>) -> Arc<dyn crate::code_rule_dsl::facts::vtab::Source> {
        let live = (rel.ndead > 0).then(|| (0..rel.len() as u32).filter(|&id| !is_dead(&rel.dead, id)).collect());
        let orders = (0..rel.k).map(|_| std::sync::OnceLock::new()).collect();
        Arc::new(Derived(rel, names, live, orders))
    }
    /// Column `c`'s rows, sorted by value.
    fn order(&self, c: usize) -> &[u32] {
        use crate::code_rule_dsl::facts::vtab::Source as _;
        self.3[c].get_or_init(|| {
            let mut rows: Vec<u32> = (0..self.len() as u32).collect();
            rows.sort_by_key(|&r| self.value(r as usize, c));
            rows
        })
    }
    /// The rows whose column `c` is `v`, as a slice of its order.
    fn rows_of(&self, c: usize, v: i64) -> &[u32] {
        use crate::code_rule_dsl::facts::vtab::Source as _;
        let order = self.order(c);
        let lo = order.partition_point(|&r| self.value(r as usize, c) < v);
        let hi = order.partition_point(|&r| self.value(r as usize, c) <= v);
        &order[lo..hi.max(lo)]
    }
}
impl crate::code_rule_dsl::facts::vtab::Source for Derived {
    fn columns(&self) -> Vec<String> { self.1.clone().unwrap_or_else(|| (0..self.0.k).map(|i| format!("c{i}")).collect()) }
    fn len(&self) -> usize { self.2.as_ref().map_or_else(|| self.0.len(), Vec::len) }
    fn estimated_matches(&self, c: usize, batch: bool) -> usize {
        let ndv = self.statistics().and_then(|s| s.get(c).map(|x| x.2)).unwrap_or(1).max(1) as usize;
        let one = self.len().div_ceil(ndv);
        if batch { one * 8 } else { one }
    }
    fn indexed(&self, c: usize) -> bool { c < self.0.k }
    // A relation an index holds answers from the index (`hold_source`), not from orders sorted over its rows.
    fn composite(&self) -> bool { self.0.held_source().is_some_and(|s| s.composite()) }
    fn lookup_all(&self, equal: &[(usize, i64)]) -> Option<Vec<usize>> { self.0.held_source()?.lookup_all(equal) }
    fn sorts_rows(&self, c: usize) -> bool { self.0.held_source().is_none() && c < self.0.k }
    fn lookup(&self, c: usize, values: &[i64]) -> Option<Vec<usize>> {
        if c >= self.0.k {
            return None;
        }
        if let Some(src) = self.0.held_source() {
            return src.lookup(c, values);
        }
        let mut out: Vec<usize> = values.iter().flat_map(|&v| self.rows_of(c, v).iter().map(|&r| r as usize)).collect();
        out.sort_unstable();
        Some(out)
    }
    fn sorted_rows(&self, c: usize, v: i64) -> Option<&[u32]> { (c < self.0.k && self.0.held_source().is_none()).then(|| self.rows_of(c, v)) }
    fn value(&self, row: usize, column: usize) -> i64 { self.0.at(self.2.as_ref().map_or(row, |l| l[row] as usize)).at(column) }
    fn value_counts(&self, wanted: &[(usize, i64)]) -> Option<Vec<i64>> {
        if let Some(src) = self.0.held_source() { return src.value_counts(wanted); }
        let mut counted = self.0.counted.lock().unwrap();
        for &(c, _) in wanted {
            if c < self.0.k && !counted.contains_key(&c) {
                let mut counts: HashMap<i64, i64> = HashMap::default();
                for row in 0..self.len() {
                    *counts.entry(self.value(row, c)).or_default() += 1;
                }
                counted.insert(c, counts);
            }
        }
        Some(wanted.iter().map(|(c, v)| counted.get(c).and_then(|m| m.get(v)).copied().unwrap_or(0)).collect())
    }
    fn statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        if let Some(src) = self.0.held_source() && let Some(s) = src.statistics() {
            return Some(self.columns().into_iter().zip(s).map(|(n, (_, a, b, c, d))| (n, a, b, c, d)).collect());
        }
        let m = self.0.measured.get_or_init(|| {
            self.0.posted().unwrap_or_else(|| crate::code_rule_dsl::facts::vtab::syntax::measured(self).into_iter().map(|(_, a, b, c, d)| [a, b, c, d]).collect())
        });
        Some(self.columns().into_iter().zip(m).map(|(n, s)| (n, s[0], s[1], s[2], s[3])).collect())
    }
}

/// `set_fixpoint_materialize(table)`: inserts a native relation's rows (those not already in its table) into the
/// table, for SQL that reads it. Returns the rows written.
pub fn materialize(conn: &Connection, table: &str) -> Result<i64> {
    let Some(rel) = DERIVED.lock().unwrap().get(&(conn.at(), table.to_owned())).cloned() else { return Err(err(format!("no native relation {table}"))) };
    if rel.k > DERIVED_COLUMNS {
        return Err(err(format!("{table}: more than {DERIVED_COLUMNS} columns")));
    }
    // One statement: the rows come from `set_derived` (a scan of the native relation), not one INSERT each.
    let cols: Vec<String> = (0..rel.k).map(|i| format!("c{i}")).collect();
    // OR IGNORE: a relation started from a kept one (`base … seeded`) holds rows its table has too.
    conn.execute(&format!("INSERT OR IGNORE INTO {} SELECT {} FROM set_derived WHERE rel = ?1", crate::code_rule_dsl::logic::sql::table(table), cols.join(", ")), [table])?;
    Ok((rel.len() - rel.stored) as i64)
}

/// Columns `set_derived` serves (a wider relation is refused).
const DERIVED_COLUMNS: usize = 16;

// ---- set_derived(rel): a native relation's rows not yet in its table, for one INSERT … SELECT ----------------------

#[repr(C)]
pub struct DerivedTab { base: ffi::sqlite3_vtab, scope: usize }
unsafe impl<'vtab> VTab<'vtab> for DerivedTab {
    type Aux = ();
    type Cursor = DerivedCursor;
    fn connect(db: &mut VTabConnection, _: Option<&()>, _: &[u8], _: &[u8], _: &[u8], _: &[&[u8]]) -> Result<(Cow<'static, CStr>, Self)> {
        let scope = unsafe { db.handle() } as usize;
        let cols: Vec<String> = (0..DERIVED_COLUMNS).map(|i| format!("c{i}")).collect();
        let schema = CString::new(format!("CREATE TABLE x({}, rel HIDDEN)", cols.join(", "))).map_err(|e| Error::ModuleError(e.to_string()))?;
        Ok((Cow::Owned(schema), DerivedTab { base: ffi::sqlite3_vtab::default(), scope }))
    }
    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let rel = info.constraints().position(|c| c.is_usable() && c.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ && c.column() == DERIVED_COLUMNS as c_int);
        let Some(i) = rel else { return Err(Error::ModuleError("set_derived needs rel = <table>".into())) };
        let mut u = info.constraint_usage(i);
        u.set_argv_index(1);
        u.set_omit(true);
        info.set_estimated_cost(1.0);
        Ok(true)
    }
    fn open(&mut self) -> Result<DerivedCursor> {
        Ok(DerivedCursor { base: ffi::sqlite3_vtab_cursor::default(), scope: self.scope, rel: None, id: 0 })
    }
}

#[repr(C)]
pub struct DerivedCursor { base: ffi::sqlite3_vtab_cursor, scope: usize, rel: Option<Arc<InRel>>, id: usize }
unsafe impl VTabCursor for DerivedCursor {
    fn filter(&mut self, _: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        let table: String = args.get(0)?;
        let rel = DERIVED.lock().unwrap().get(&(self.scope, table.clone())).cloned().ok_or_else(|| Error::ModuleError(format!("no native relation {table}")))?;
        self.id = rel.stored;
        while self.id < rel.len() && is_dead(&rel.dead, self.id as u32) { self.id += 1; }
        self.rel = Some(rel);
        Ok(())
    }
    fn next(&mut self) -> Result<()> {
        self.id += 1;
        if let Some(rel) = &self.rel { while self.id < rel.len() && is_dead(&rel.dead, self.id as u32) { self.id += 1; } }
        Ok(())
    }
    fn eof(&self) -> bool { self.rel.as_ref().is_none_or(|r| self.id >= r.len()) }
    fn column(&self, ctx: &mut VContext, i: c_int) -> Result<()> {
        let rel = self.rel.as_ref().expect("a filtered cursor");
        match rel.values(self.id).get(i as usize) { Some(v) => ctx.set_result(v), None => ctx.set_result(rusqlite::types::Null) }
    }
    fn rowid(&self) -> Result<i64> { Ok(self.id as i64) }
}
/// A relation's statistics in the planner's form (`statkeep::RelStats`, paired: kept by rows that are not whole
/// documents), counted as its rows are indexed and as they die, so a kept relation carries them to the next run and
/// is not counted again from its rows there (decision 0096 step 4). Columns named `c0`, `c1`, … until read.
#[derive(Clone)]
pub struct KeptStats(crate::code_rule_dsl::logic::plan::statkeep::RelStats);
impl KeptStats {
    fn of(k: usize) -> KeptStats {
        KeptStats(crate::code_rule_dsl::logic::plan::statkeep::RelStats::paired((0..k).map(|c| format!("c{c}")).collect()))
    }
}
impl InRel {
    /// Its statistics as kept while its rows came (`Rel::stats`), its columns named `names`: `None` for one not
    /// derived by the native engine, or whose rows were not all counted.
    pub(in crate::code_rule_dsl) fn kept_stats(&self, names: Vec<String>) -> Option<crate::code_rule_dsl::logic::plan::statkeep::RelStats> {
        let s = self.stats.as_ref()?;
        (s.0.rows() == (self.n - self.ndead) as i64 && names.len() == self.k).then(|| s.0.renamed(names))
    }
    /// A component's relation at its fixpoint, its rows and indexes moved.
    fn moved(mut rel: Rel) -> InRel {
        rel.index_rest();
        rel.count_dead();
        InRel { k: rel.k, n: rel.n, rows: rel.data, view: None, base: false, types: rel.types, stored: rel.stored, idx: Mutex::new(HashMap::default()), chains: rel.idx,
                sketches: rel.sketches, serves: rel.serves, dead: rel.dead, ndead: rel.ndead, summaries: rel.summaries, measured: Default::default(), counted: Default::default(), tables: Default::default(), name: std::sync::OnceLock::from(rel.table), from: rel.from, uid: Default::default(), level: rel.level, stats: rel.stats }
    }
    /// Its rows' change from `old`, the kept relation it started as (`Rel::based`), as `(added, removed)` row-major:
    /// the rows it added after `old`'s, and `old`'s live rows it marked dead. A row deleted and derived again is in
    /// both. Costs the change and a pass over the dead bits, not the rows. `None` if it did not start as `old`.
    pub fn delta_since(&self, old: &InRel) -> Option<(Vec<i64>, Vec<i64>)> {
        let at = old as *const InRel as usize;
        if self.k != old.k || self.view.is_some() {
            return None;
        }
        // The same relation, held on unchanged (a quiet stratum's, decision 0101 §1): no change.
        if std::ptr::eq(self, old) {
            return Some((Vec::new(), Vec::new()));
        }
        let k = self.k;
        match &self.from {
            Some(Origin::Based { at: a, n }) if *a == at && *n == old.n => {}
            Some(Origin::Compacted { at: a, old_n, kept, removed }) if *a == at && *old_n == old.n => {
                let added = (*kept..self.n).filter(|&id| !is_dead(&self.dead, id as u32)).flat_map(|id| self.row(id).iter().copied()).collect();
                return Some((added, removed.clone()));
            }
            _ => return None,
        }
        let mut removed = Vec::new();
        for (w, &bits) in self.dead.iter().enumerate() {
            let mut new = bits & !old.dead.get(w).copied().unwrap_or(0);
            while new != 0 {
                let id = w * 64 + new.trailing_zeros() as usize;
                new &= new - 1;
                if id < old.n {
                    removed.extend_from_slice(self.row(id));
                }
            }
        }
        let mut added = Vec::with_capacity((self.n - old.n) * k);
        for id in old.n..self.n {
            if !is_dead(&self.dead, id as u32) {
                added.extend_from_slice(self.row(id));
            }
        }
        Some((added, removed))
    }
    /// Column `c`'s postings, from the index the engine kept on it (`Rel::new`): each value with the first-column
    /// values of its live rows (a row's document, as `syntax::measured` reads it), so nothing is hashed again. A
    /// posting of one live row is handed on as it is; a longer one is walked. `false`, and nothing visited, for a
    /// relation without that index (a view, or rows loaded flat).
    pub fn each_posting(&self, c: usize, visit: &mut dyn FnMut(i64, &[i64])) -> bool {
        if self.view.is_some() || c >= self.k {
            return false;
        }
        probed::note(&format!("{} [postings]", self.name.get().map_or("?", String::as_str)), std::iter::once(c));
        let Some(ix) = self.chains.iter().find(|ix| ix.cols.first() == Some(&c)).filter(|ix| ix.rows() >= self.n) else { return false };
        let (k, data) = (self.k, &self.rows);
        let mut firsts: Vec<i64> = Vec::new();
        ix.each_group(data, k, 1, |ids| {
            let v = data[ids[0] as usize * k + c];
            firsts.clear();
            firsts.extend(ids.iter().filter(|&&id| !is_dead(&self.dead, id)).map(|&id| data[id as usize * k]));
            if !firsts.is_empty() {
                visit(v, &firsts);
            }
        });
        true
    }
    /// Whether every column has its postings (`each_posting`): what its statistics are counted from without reading
    /// its rows (decision 0123: else they are counted from its rows, not begun from postings and then dropped).
    pub fn posts_all(&self) -> bool {
        self.view.is_none() && (0..self.k).all(|c| self.chains.iter().any(|ix| ix.cols.first() == Some(&c) && ix.rows() >= self.n))
    }
    /// Its live rows, row-major.
    pub fn live_rows(&self) -> Option<Vec<i64>> {
        if self.view.is_some() {
            return None;
        }
        if self.ndead == 0 {
            return Some(self.rows[..self.n * self.k].to_vec());
        }
        let mut out = Vec::with_capacity((self.n - self.ndead) * self.k);
        for id in 0..self.n {
            if !is_dead(&self.dead, id as u32) {
                out.extend_from_slice(self.row(id));
            }
        }
        Some(out)
    }
    /// No rows, `k` columns.
    fn empty(k: usize) -> InRel {
        InRel { k, n: 0, rows: Vec::new(), view: None, base: false, types: vec![b'i'; k], stored: 0, idx: Mutex::new(HashMap::default()), chains: Vec::new(), sketches: Vec::new(), serves: Vec::new(), dead: Vec::new(), ndead: 0, summaries: Default::default(), measured: Default::default(), counted: Default::default(), tables: Default::default(), name: Default::default(), from: None, uid: Default::default(), level: Vec::new(), stats: None }
    }
    /// Rows, levels held, and whether it is a view (`RERAIL_LEVELS_WHY`).
    pub fn levels_shape(&self) -> (usize, usize, bool) {
        (self.n, self.level.len(), self.view.is_some())
    }
    /// Whether every row has its level (a relation the native engine derived; decision 0090's levels).
    pub fn has_levels(&self) -> bool {
        self.view.is_none() && self.level.len() == self.n
    }
    /// A whole view's rows are its source's, counted when first asked: a source that makes its rows on lookup
    /// (`syntax::within`) makes them all only for a scan.
    pub fn len(&self) -> usize {
        match &self.view { Some(v) if v.ids.is_none() => v.src.len(), _ => self.n }
    }
    /// A flat relation's row (a derived one: views are base inputs, or an index's relation, `hold_source`).
    pub fn row(&self, id: usize) -> &[i64] { &self.rows[id * self.k..][..self.k] }
    /// Row `id`'s values, flat or read from its view.
    pub fn values(&self, id: usize) -> std::borrow::Cow<'_, [i64]> {
        match &self.view {
            None => std::borrow::Cow::Borrowed(self.row(id)),
            Some(_) => { let r = self.at(id); std::borrow::Cow::Owned((0..self.k).map(|c| r.at(c)).collect()) }
        }
    }
    /// The source a derived relation is read from in place, every row (`hold_source`); `None` for rows held flat.
    pub fn held_source(&self) -> Option<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>> {
        self.view.as_ref().filter(|v| !self.base && v.ids.is_none() && v.eqs.is_empty()).map(|v| Arc::clone(&v.src))
    }
    #[inline]
    pub fn at(&self, id: usize) -> RowRef<'_> {
        match &self.view { Some(v) => RowRef::View(v, id), None => RowRef::Flat(&self.rows[id * self.k..][..self.k]) }
    }
    /// For a key of several columns over a view, the most selective column the source indexes (sorted, or by its
    /// lookup) with at most `LEAD_MATCHES` rows a value: its position in `cols`, and whether sorted.
    /// For a key of several columns of a derived relation (no view), the chain (`chains`) of one of them whose
    /// postings are short, as `lead` takes a source's own index: (the chain, the key column it is on). `None` when
    /// no column's chain holds every row or each is long.
    fn chain_lead(&self, cols: &[usize]) -> Option<(usize, usize)> {
        const LEAD_MATCHES: usize = 8;
        if self.view.is_some() || cols.len() < 2 || self.n == 0 {
            return None;
        }
        cols.iter()
            .enumerate()
            .filter_map(|(j, &c)| {
                let i = self.chains.iter().position(|ix| ix.cols.first() == Some(&c))?;
                let ix = &self.chains[i];
                let ndv = self.sketches.get(c).map_or(0, Sketch::ndv);
                (ix.rows() >= self.n && ndv > 0).then(|| (self.n.div_ceil(ndv), i, j))
            })
            .filter(|&(m, _, _)| m <= LEAD_MATCHES)
            .min()
            .map(|(_, i, j)| (i, j))
    }
    fn lead(&self, cols: &[usize]) -> Option<(usize, bool)> {
        const LEAD_MATCHES: usize = 8;
        let v = self.view.as_ref().filter(|_| cols.len() > 1)?;
        cols.iter()
            .enumerate()
            .filter_map(|(j, &c)| {
                let sc = v.cols[c];
                let sorted_source = v.src.sorts_rows(sc);
                let sorted = sorted_source && v.ids.is_none() && v.eqs.is_empty();
                let source = !sorted_source && v.ids.is_none() && v.src.indexed(sc);
                (sorted || source).then(|| (v.src.estimated_matches(sc, false), j, sorted))
            })
            // A source made on lookup leads with its index whatever the estimate: a hash table of every row would make them all.
            .filter(|&(m, _, _)| m <= LEAD_MATCHES || v.src.made_on_lookup())
            .min()
            .map(|(_, j, sorted)| (j, sorted))
    }
    /// Each column's statistics as `syntax::measured` gives them (live rows, distinct values, the sum of squared
    /// postings, and that sum by document `id >> 32` of the first column), read from the postings the engine kept
    /// while it derived the rows (one index per column, `Rel::new`): a posting's length is its value's count, so
    /// nothing is sorted. A posting of one live row counts once; a longer one, or one holding a dead row, is walked
    /// for its rows' documents. `None` for a relation without those indexes (a view, or rows loaded flat).
    fn posted(&self) -> Option<Vec<[i64; 4]>> {
        if self.view.is_some() {
            return None;
        }
        let k = self.k;
        let data = &self.rows;
        let doc = |id: u32| data[id as usize * k] >> 32;
        let live = (self.n - self.ndead) as i64;
        let mut out = Vec::with_capacity(k);
        let mut docs: Vec<i64> = Vec::new();
        for c in 0..k {
            let ix = self.chains.iter().find(|ix| ix.cols.first() == Some(&c))?;
            if ix.rows() < self.n {
                return None;
            }
            let (mut ndv, mut sq, mut sqd) = (0i64, 0i64, 0i64);
            ix.each_group(data, k, 1, |ids| {
                docs.clear();
                docs.extend(ids.iter().filter(|&&id| !is_dead(&self.dead, id)).map(|&id| doc(id)));
                let l = docs.len() as i64;
                if l == 0 {
                    return;
                }
                ndv += 1;
                sq += l * l;
                if docs.iter().all(|d| *d == docs[0]) {
                    sqd += l * l;
                } else {
                    docs.sort_unstable();
                    let mut at = 0;
                    while at < docs.len() {
                        let end = at + docs[at..].partition_point(|d| *d == docs[at]);
                        sqd += ((end - at) * (end - at)) as i64;
                        at = end;
                    }
                }
            });
            out.push([live, ndv, sq, sqd]);
        }
        Some(out)
    }
    fn index(&self, cols: &[usize]) -> Arc<Csr> {
        let mask = cols.iter().fold(0u64, |m, &c| m | 1 << c);
        let mut idx = self.idx.lock().unwrap();
        idx.entry(mask).or_insert_with(|| {
            let base = self.base;
            let _index = crate::code_rule_dsl::logic::profile::span(move || (if base { "open: index of a base input" } else { "open: index of a derived input" }).into());
            let (t0, c0) = (Instant::now(), thread_cpu());
            // No key columns: the step reads every row, not an index.
            let empty = |source, sorted| Csr { cols: cols.to_vec(), table: HashTable::new(), ids: Vec::new(), source, sorted, chain: None, lead: 0, composite: false };
            let m = if cols.is_empty() {
                empty(false, false)
            } else if let Some(i) = self.chains.iter().position(|ix| ix.answers(cols)) {
                Csr { chain: Some(i), ..empty(false, false) }
            } else if let (Some(v), [c]) = (&self.view, cols) && v.ids.is_none() && v.eqs.is_empty() && v.src.sorts_rows(v.cols[*c]) {
                empty(false, true)
            } else if let (Some(v), [c]) = (&self.view, cols) && v.src.indexed(v.cols[*c]) && !v.src.sorts_rows(v.cols[*c]) {
                // (A source's lookup allocates each probe's rows; over a sorted source a filtered view builds its own.)
                empty(true, false)
            } else if let Some(v) = &self.view && cols.len() > 1 && v.ids.is_none() && v.eqs.is_empty() && v.src.composite() {
                // The source answers every key column at once (the reach index's bitmap test).
                Csr { composite: true, ..empty(false, false) }
            } else if let Some((lead, sorted)) = self.lead(cols) {
                // A key of several columns, one of them selective in the source's own index (a node column of
                // `inside_<set>`, `node`, `name`): its candidates, checked against the other keys by the step, not a
                // hash table of every row.
                Csr { lead, ..empty(!sorted, sorted) }
            } else if let Some((chain, lead)) = self.chain_lead(cols) {
                // The same for a derived relation: one of its key columns' own chains, kept from the waves that made
                // it, is selective, so no composite index is built.
                Csr { chain: Some(chain), lead, ..empty(false, false) }
            } else {
                charge(Unit::IndexEntry, self.len() as u64);
                Csr::build(self, cols)
            };
            if env_set!("RERAIL_SPIKE_INDEX") {
                eprintln!("O125 index base={} took {:.3} ms rows={} cols={cols:?} chain={} source={} sorted={} lead={} view={} of {:?}", self.base, (thread_cpu() - c0) * 1e3, self.len(), m.chain.is_some(), m.source, m.sorted, m.lead, self.view.is_some(), self.view.as_ref().map(|v| (v.src.columns(), v.cols.clone())));
            }
            let l = &mut LOADS.lock().unwrap()[usize::from(!self.base)];
            l.0 += thread_cpu() - c0;
            if env_set!("RDL_PROFILE") {
                eprintln!("      index on {cols:?} of {} rows in {:.1} ms", self.len(), t0.elapsed().as_secs_f64() * 1e3);
            }
            Arc::new(m)
        }).clone()
    }
}
/// Input relations loaded for joins, by database and spec, for the run that loaded them: the base relations are
/// vtabs rebound per batch, so a load is not reused across runs (`forget_inputs`). T cached them for its process,
/// which ran one program over one facts file.
static INPUTS: LazyLock<Mutex<HashMap<String, HashMap<InputKey, Arc<InRel>>>>> = LazyLock::new(Default::default);

/// Drops the input relations loaded on `conn`'s database: a run starts over the relations bound now.
pub fn forget_inputs(conn: &Connection) {
    let path = conn_path(conn).to_string();
    let changing = CHANGING.lock().unwrap().get(&path).cloned();
    let Some(changing) = changing else {
        INPUTS.lock().unwrap().remove(&path);
        STORED.lock().unwrap().retain(|(p, _), _| *p != path);
        return;
    };
    // Kept: base inputs (`b:` specs) that read none of the relations that change between runs; their rows and
    // indexes are the same in the next run. Derived inputs (`d:`, a SQL stratum's table) are made again.
    let reads_changing = |sel: &str| sel.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).any(|w| changing.contains(w));
    if let Some(m) = INPUTS.lock().unwrap().get_mut(&path) { m.retain(|key, _| key.base && !reads_changing(&key.sql)); }
    STORED.lock().unwrap().retain(|(p, table), _| *p != path || (!table.starts_with("main.") && !table.starts_with("temp.") && !reads_changing(table)));
}

/// Relations whose rows change between the native runs on `conn` (`keep_inputs`): while set, `forget_inputs` keeps
/// every other base input loaded, with its indexes.
static CHANGING: LazyLock<Mutex<HashMap<String, std::collections::BTreeSet<String>>>> = LazyLock::new(Default::default);

/// Runs on `conn` that read the same inputs but for `changing` (the bounded rounds: each round rebinds only its
/// `__prev` inputs) keep the others loaded; `None` ends it, and the next `forget_inputs` drops everything.
pub fn keep_inputs(conn: &Connection, changing: Option<std::collections::BTreeSet<String>>) {
    let path = conn_path(conn).to_string();
    match changing {
        Some(c) => {
            CHANGING.lock().unwrap().insert(path, c);
        }
        None => {
            CHANGING.lock().unwrap().remove(&path);
        }
    }
}

/// An input from its spec's text (`<b|d|x>:<hex select>:<hex columns>:<types>`).
fn input(conn: &Connection, spec: &str, syms: &mut Interner) -> Result<Arc<InRel>> {
    input_of(conn, &InputKey::parse(spec).map_err(err)?, syms)
}

fn input_of(conn: impl Scoped, spec: &InputKey, syms: &mut Interner) -> Result<Arc<InRel>> {
    let conn = conn.db();
    let path: &str = &conn_path(conn);
    if let Some(r) = INPUTS.lock().unwrap().get(path).and_then(|m| m.get(spec)) { return Ok(r.clone()); }
    let keep = |rel: &Arc<InRel>| { INPUTS.lock().unwrap().entry(path.to_owned()).or_default().insert(spec.clone(), rel.clone()); };
    let _load = crate::code_rule_dsl::logic::profile::span(|| "open: input load".into());
    let base = spec.base;
    let (table, cols, types) = (&spec.sql, &spec.cols, spec.types.as_bytes());
    let (t0, c0) = (Instant::now(), thread_cpu());
    // A stored table holds symbols as atoms (integers), so any column types read in place there.
    if base && (types.iter().all(|&t| t == b'i') || !table.starts_with("main.")) {
        if let Some(view) = view(conn, &table, &cols) {
            let n = view.ids.as_ref().map_or(0, Vec::len);
            let rel = Arc::new(InRel { k: types.len(), n, rows: Vec::new(), view: Some(view), base, types: types.to_vec(), stored: 0, idx: Mutex::new(HashMap::default()), chains: Vec::new(), sketches: Vec::new(), serves: Vec::new(), dead: Vec::new(), ndead: 0, summaries: Default::default(), measured: Default::default(), counted: Default::default(), tables: Default::default(), name: Default::default(), from: None, uid: Default::default(), level: Vec::new(), stats: None });
            LOADS.lock().unwrap()[0].0 += thread_cpu() - c0;
            if env_set!("RDL_PROFILE") {
                eprintln!("      view of {n} rows in {:.1} ms: SELECT {cols} FROM {table}", t0.elapsed().as_secs_f64() * 1e3);
            }
            keep(&rel);
            return Ok(rel);
        }
    }
    let mut st = conn.sql()?.prepare(&format!("SELECT {cols} FROM {}", crate::code_rule_dsl::logic::sql::table(&table)))?;
    let mut q = st.query([])?;
    let (mut rows, mut n) = (Vec::new(), 0);
    while let Some(r) = q.next()? {
        for (i, &t) in types.iter().enumerate() { rows.push(read_value(r.get_ref(i)?, t, syms).map_err(|e| err(format!("{table}: {e}")))?); }
        n += 1;
    }
    let rel = Arc::new(InRel { k: types.len(), n, rows, view: None, base, types: types.to_vec(), stored: 0, idx: Mutex::new(HashMap::default()), chains: Vec::new(), sketches: Vec::new(), serves: Vec::new(), dead: Vec::new(), ndead: 0, summaries: Default::default(), measured: Default::default(), counted: Default::default(), tables: Default::default(), name: Default::default(), from: None, uid: Default::default(), level: Vec::new(), stats: None });
    {
        let l = &mut LOADS.lock().unwrap()[usize::from(!base)];
        l.0 += thread_cpu() - c0;
        l.1 += rel.len() as u64;
    }
    if env_set!("RDL_PROFILE") {
        eprintln!("      input {} rows in {:.1} ms: SELECT {cols} FROM {table}", rel.len(), t0.elapsed().as_secs_f64() * 1e3);
    }
    keep(&rel);
    Ok(rel)
}

/// The rows of `sql`, `SELECT <cols> FROM main.<input>[ WHERE …]` over a bound input, read from the input where it is
/// held (`view`), each cell as SQL reads it (`read_value`). `None` for any other query.
fn bound_rows(conn: impl Scoped, sql: &str, types: &[u8], syms: &mut Interner) -> Result<Option<Vec<Vec<i64>>>> {
    let conn = conn.db();
    let Some((cols, table)) = sql.strip_prefix("SELECT ").and_then(|s| s.split_once(" FROM ")) else { return Ok(None) };
    let Some(v) = table.starts_with("main.").then(|| view(conn, table, cols)).flatten() else { return Ok(None) };
    let ids: Box<dyn Iterator<Item = usize>> = match &v.ids { Some(ids) => Box::new(ids.iter().map(|&r| r as usize)), None => Box::new(0..v.src.len()) };
    let mut rows = Vec::with_capacity(v.ids.as_ref().map_or(v.src.len(), Vec::len));
    for r in ids {
        let mut row = Vec::with_capacity(types.len());
        for (&c, &t) in v.cols.iter().zip(types) {
            row.push(read_value(ValueRef::from(&v.src.cell(r, c)), t, syms).map_err(|e| err(format!("{table}: {e}")))?);
        }
        rows.push(row);
    }
    Ok(Some(rows))
}

/// A base input over a bound vtab, read in place: `main.<table>` with `<col> = <int>` filters (joined by AND), its
/// columns named. `None` when the table is not a bound source or the select is another shape (it is copied then).
fn view(conn: impl Scoped, table: &str, cols: &str) -> Option<View> {
    let conn = conn.db();
    let (schema, rest) = table.split_once('.')?;
    let (name, filters) = match rest.split_once(" WHERE ") { Some((n, f)) => (n, Some(f)), None => (rest, None) };
    let src = if schema == "main" {
        crate::code_rule_dsl::facts::vtab::bound_source(conn, name)?
    } else {
        // The columns read and those filtered.
        let filtered = filters.into_iter().flat_map(|f| f.split(" AND ")).filter_map(|f| f.split_once(" = ").map(|(c, _)| c));
        let need: Vec<&str> = cols.split(',').filter(|c| c.trim() != "1").chain(filtered).collect();
        stored_table(conn.sql().ok()?, schema, name, &need)?
    };
    let src=src.frozen().unwrap_or(src);
    let names = src.columns();
    let column = |c: &str| names.iter().position(|n| n == c.trim());
    // `SELECT 1`: whether rows are there (a gate's `done_N(0)`), no columns read.
    let cols: Vec<usize> = if cols.trim() == "1" { Vec::new() } else { cols.split(',').map(column).collect::<Option<_>>()? };
    let mut eqs = Vec::new();
    for f in filters.map(|f| f.split(" AND ").collect::<Vec<_>>()).unwrap_or_default() {
        let (c, v) = f.split_once(" = ")?;
        eqs.push((column(c)?, v.trim().parse::<i64>().ok()?));
    }
    let ids = if eqs.is_empty() {
        None
    } else {
        let (c, v) = eqs.iter().copied().find(|&(c, _)| src.indexed(c)).unwrap_or(eqs[0]);
        // The roofline's work: each candidate's filter values read.
        match src.lookup(c, &[v]) {
            Some(cands) => {
                charge(Unit::Value, (cands.len() * eqs.len()) as u64);
                Some(cands.into_iter().filter(|&r| eqs.iter().all(|&(c, v)| src.value(r, c) == v)).map(|r| r as u32).collect())
            }
            None => {
                charge(Unit::Value, (src.len() * eqs.len()) as u64);
                Some(src.filter_eq(&eqs))
            }
        }
    };
    Some(View { src, cols, ids, eqs })
}

/// A table of integer columns in an attached schema (the cross-file pass's `x`), read in place: the columns an input
/// reads are loaded by one statement the first time any input of the run needs them (a table scan, `NOT INDEXED`,
/// so every column is in the table's row order), and every filtered or projected input over it is then a view
/// (`View`). A column's index is built on its first lookup. A table holding a non-integer value is not one.
struct Stored {
    columns: Vec<String>,
    n: std::sync::OnceLock<usize>,
    values: Vec<std::sync::OnceLock<Vec<i64>>>,
    /// Each column's rows ordered by value, with its distinct values and where each one's run starts (`Sorted`).
    index: Vec<std::sync::OnceLock<Sorted>>,
    /// Each column's count of distinct values, from its index (for SQL's estimates).
    distinct: Vec<std::sync::OnceLock<usize>>,
    /// Lexicographic orders over several columns, built on a first `lookup_all` over them (4 bytes a row each;
    /// the table never changes, so an order costs no insert). One serves every search whose columns are its prefix.
    orders: Mutex<Vec<(Vec<usize>, Arc<Vec<u32>>)>>,
    /// For rows held in memory (`columns_source`), the planner's statistics, read from the column indexes when first
    /// asked and kept with the rows: a session's next run reads them for its `r__old` (0094 §4), not measured again.
    /// `None` for a stored table, whose columns load on demand.
    statistics: Option<std::sync::OnceLock<Vec<(String, i64, i64, i64, i64)>>>,
    /// Its rows' runs by document, as `carry_sorted` reads them: made once, then read by the next run's carry.
    runs: std::sync::OnceLock<Vec<((i64, u32), usize, usize)>>,
}
impl Stored {
    /// The runs of its rows of one document: ((document, which of its runs), start, end). A row's document is the one
    /// its first column names, as `memo::document_of` reads it: a node `d << 32 | i`, else the document key itself. A
    /// document's rows are mostly one run; one whose rows come in several (its own node apart) has each counted.
    fn runs(&self) -> &[((i64, u32), usize, usize)] {
        self.runs.get_or_init(|| {
            let first = self.col(0);
            let mut out: Vec<((i64, u32), usize, usize)> = Vec::new();
            let mut seen: rustc_hash::FxHashMap<i64, u32> = rustc_hash::FxHashMap::default();
            let mut at = 0;
            while at < first.len() {
                let v = first[at];
                let d = if v >= 1 << 32 { v >> 32 } else { v };
                // The run's end: the rows of a node of `d` (`d << 32 ..= d << 32 | u32::MAX`), or of the value itself.
                let end = if v >= 1 << 32 {
                    let (lo, hi) = (d << 32, (d << 32) | 0xffff_ffff);
                    at + first[at..].iter().position(|&x| !(lo..=hi).contains(&x) && x != d).unwrap_or(first.len() - at)
                } else {
                    at + first[at..].iter().position(|&x| x != d && !(x >= 1 << 32 && x >> 32 == d)).unwrap_or(first.len() - at)
                };
                let k = seen.entry(d).or_default();
                out.push(((d, *k), at, end));
                *k += 1;
                at = end;
            }
            out
        })
    }
    fn new(columns: Vec<String>, values: Vec<std::sync::OnceLock<Vec<i64>>>) -> Self {
        let k = columns.len();
        Stored { columns, n: std::sync::OnceLock::new(), values, index: (0..k).map(|_| std::sync::OnceLock::new()).collect(), distinct: (0..k).map(|_| std::sync::OnceLock::new()).collect(), orders: Mutex::default(), statistics: None, runs: std::sync::OnceLock::new() }
    }
    /// `syntax::measured`'s statistics, from each column's sorted index (which joins read too): a value's rows are
    /// one run of it, in row order, and their documents (`id >> 32` of the first column) are counted within it. A
    /// column held nowhere (no rule reads it) is stated as a key.
    fn measured(&self) -> Vec<(String, i64, i64, i64, i64)> {
        let n = self.n.get().copied().unwrap_or(0);
        let first = self.col(0);
        let doc = |id: u32| first.get(id as usize).copied().unwrap_or(0) >> 32;
        let mut docs: Vec<i64> = Vec::new();
        (0..self.columns.len())
            .map(|c| {
                let name = self.columns[c].clone();
                if self.values[c].get().is_none() || n == 0 {
                    return (name, n as i64, n as i64, n as i64, n as i64);
                }
                let sorted = self.sorted(c);
                let ids = &sorted.ids;
                let (mut ndv, mut sq, mut sqd) = (0i64, 0i64, 0i64);
                for run in sorted.starts.windows(2) {
                    let (at, end) = (run[0] as usize, run[1] as usize);
                    let l = (end - at) as i64;
                    ndv += 1;
                    sq += l * l;
                    docs.clear();
                    docs.extend(ids[at..end].iter().map(|&r| doc(r)));
                    if !docs.is_sorted() {
                        docs.sort_unstable();
                    }
                    let mut d = 0;
                    while d < docs.len() {
                        let de = d + docs[d..].partition_point(|x| *x == docs[d]);
                        sqd += ((de - d) * (de - d)) as i64;
                        d = de;
                    }
                }
                (name, n as i64, ndv, sq, sqd)
            })
            .collect()
    }
    fn distinct(&self, c: usize) -> usize {
        // Statistics kept with the rows (`columns_source_measured`, or measured once) give it without a sort: a plan
        // asks each key column's count to choose the one it probes by, and sorting all of them to ask was most of an
        // edit's index cost (0101).
        *self.distinct[c].get_or_init(|| {
            let kept = self.statistics.as_ref().and_then(|s| s.get()).and_then(|st| st.iter().find(|m| m.0 == self.columns[c]).map(|m| m.2 as usize));
            kept.unwrap_or_else(|| self.sorted(c).keys.len()).max(1)
        })
    }
    /// An order whose leading columns are exactly `cols` (as a set), built most selective column first.
    fn order(&self, cols: &[usize]) -> (Vec<usize>, Arc<Vec<u32>>) {
        let mut orders = self.orders.lock().unwrap();
        let fits = |o: &[usize]| o.len() >= cols.len() && { let mut p = o[..cols.len()].to_vec(); p.sort_unstable(); p == cols };
        if let Some((o, ids)) = orders.iter().find(|(o, _)| fits(o)) { return (o.clone(), Arc::clone(ids)); }
        let mut by = cols.to_vec();
        by.sort_by_key(|&c| std::cmp::Reverse(self.distinct(c)));
        let cols_v: Vec<&[i64]> = by.iter().map(|&c| self.col(c)).collect();
        let mut ids: Vec<u32> = (0..self.n.get().copied().unwrap_or(0) as u32).collect();
        ids.sort_by(|&a, &b| cols_v.iter().map(|v| v[a as usize].cmp(&v[b as usize])).find(|o| o.is_ne()).unwrap_or(std::cmp::Ordering::Equal));
        let ids = Arc::new(ids);
        orders.push((by.clone(), Arc::clone(&ids)));
        (by, ids)
    }
    fn col(&self, c: usize) -> &[i64] { self.values[c].get().map_or(&[], Vec::as_slice) }
    /// Column `c`'s rows ordered by value (4 bytes a row, no allocation per key); a value's rows are one run.
    fn index(&self, c: usize) -> &[u32] {
        &self.sorted(c).ids
    }
    fn sorted(&self, c: usize) -> &Sorted {
        self.index[c].get_or_init(|| {
            let _s = crate::code_rule_dsl::logic::profile::span(|| format!("sort a stored column ({} rows)", self.col(c).len()));
            Sorted::of(self.col(c))
        })
    }
    /// The rows whose column `c` is `v`: a search of the column's distinct values (contiguous, one per value), not of
    /// its rows through their ids (a cache miss a step).
    fn rows_of(&self, c: usize, v: i64) -> &[u32] {
        let s = self.sorted(c);
        match s.keys.binary_search(&v) {
            Ok(i) => &s.ids[s.starts[i] as usize..s.starts[i + 1] as usize],
            Err(_) => &[],
        }
    }
    /// Loads the columns among `need` not yet loaded, in one statement. False if one holds a non-integer value.
    fn load(&self, conn: &Connection, table: &str, need: &[usize]) -> bool {
        let missing: Vec<usize> = need.iter().copied().filter(|&c| self.values[c].get().is_none()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        if missing.is_empty() { return true; }
        let names: Vec<&str> = missing.iter().map(|&c| self.columns[c].as_str()).collect();
        let read = || -> rusqlite::Result<Option<Vec<Vec<i64>>>> {
            let mut st = conn.prepare(&format!("SELECT {} FROM {} NOT INDEXED", names.join(", "), crate::code_rule_dsl::logic::sql::table(table)))?;
            let mut values = vec![Vec::new(); missing.len()];
            let mut q = st.query([])?;
            while let Some(r) = q.next()? {
                for (i, v) in values.iter_mut().enumerate() {
                    match r.get_ref(i)? { ValueRef::Integer(x) => v.push(x), _ => return Ok(None) }
                }
            }
            drop(q);
            charge(Unit::VmStep, st.get_status(rusqlite::StatementStatus::VmStep) as u64);
            Ok(Some(values))
        };
        let Ok(Some(values)) = read() else { return false };
        for (c, v) in missing.into_iter().zip(values) {
            let _ = self.n.set(v.len());
            let _ = self.values[c].set(v);
        }
        true
    }
}
/// A column's rows ordered by value (`ids`, in row order within a value), its distinct values ascending (`keys`), and
/// where each value's run of `ids` starts (`starts`, one more than `keys`: the last is the row count). 4 bytes a row
/// and 12 a distinct value.
struct Sorted { ids: Vec<u32>, keys: Vec<i64>, starts: Vec<u32> }
impl Sorted {
    fn of(col: &[i64]) -> Sorted {
        charge(Unit::IndexEntry, col.len() as u64);
        // In each case the rows of a value are in row order, as a stable sort by value leaves them.
        // A column already in order (a node column of rows in document order): its runs, nothing sorted.
        if col.is_sorted() {
            let (mut keys, mut starts) = (Vec::new(), Vec::new());
            for (i, &v) in col.iter().enumerate() {
                if keys.last() != Some(&v) {
                    keys.push(v);
                    starts.push(i as u32);
                }
            }
            starts.push(col.len() as u32);
            return Sorted { ids: (0..col.len() as u32).collect(), keys, starts };
        }
        // Few values (a kind, a name): counted, the values alone sorted, and each row placed at its value's next slot.
        let mut counts: rustc_hash::FxHashMap<i64, u32> = rustc_hash::FxHashMap::default();
        let few = col.len() / 8;
        for &v in col {
            *counts.entry(v).or_default() += 1;
            if counts.len() > few {
                break;
            }
        }
        if counts.len() <= few {
            let mut keys: Vec<i64> = counts.keys().copied().collect();
            keys.sort_unstable();
            let mut starts = Vec::with_capacity(keys.len() + 1);
            let mut at = 0u32;
            for k in &keys {
                starts.push(at);
                let n = counts[k];
                // The value's next slot, counted up from its start as its rows are placed.
                counts.insert(*k, at);
                at += n;
            }
            starts.push(at);
            let mut ids = vec![0u32; col.len()];
            for (r, v) in col.iter().enumerate() {
                let slot = counts.get_mut(v).expect("counted");
                ids[*slot as usize] = r as u32;
                *slot += 1;
            }
            return Sorted { ids, keys, starts };
        }
        // Sorted as (value, row) pairs in place: the same order as rows sorted stably by value, without reading a
        // value through its row id at each comparison.
        let mut pairs: Vec<(i64, u32)> = col.iter().enumerate().map(|(r, &v)| (v, r as u32)).collect();
        pairs.sort_unstable();
        let mut keys = Vec::new();
        let mut starts = Vec::new();
        for (i, &(v, _)) in pairs.iter().enumerate() {
            if keys.last() != Some(&v) {
                keys.push(v);
                starts.push(i as u32);
            }
        }
        starts.push(pairs.len() as u32);
        Sorted { ids: pairs.into_iter().map(|(_, r)| r).collect(), keys, starts }
    }
}
impl crate::code_rule_dsl::facts::vtab::Source for Stored {
    fn as_any(&self) -> Option<&dyn std::any::Any> { Some(self) }
    fn columns(&self) -> Vec<String> { self.columns.clone() }
    fn statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        self.statistics.as_ref().map(|s| s.get_or_init(|| self.measured()).clone())
    }
    fn len(&self) -> usize { self.n.get().copied().unwrap_or(0) }
    fn estimated_matches(&self, c: usize, batch: bool) -> usize {
        let one = self.len().div_ceil(self.distinct(c).max(1));
        if batch { one * 8 } else { one }
    }
    fn composite(&self) -> bool { true }
    fn lookup_all(&self, equal: &[(usize, i64)]) -> Option<Vec<usize>> {
        // Omitted columns read as zero, just as `value` does. They have no array to sort or probe.
        // Remove their satisfied predicates before using an order over the columns that are held.
        if equal.iter().any(|&(c, _)| self.values[c].get().is_none()) {
            if equal.iter().any(|&(c, v)| self.values[c].get().is_none() && v != 0) {
                return Some(Vec::new());
            }
            let held: Vec<_> = equal.iter().copied().filter(|&(c, _)| self.values[c].get().is_some()).collect();
            return self.lookup_all(&held);
        }
        if equal.is_empty() {
            return Some((0..self.len()).collect());
        }
        let cols: Vec<usize> = equal.iter().map(|e| e.0).collect();
        let (on, order) = self.order(&cols);
        let by: Vec<(&[i64], i64)> = on[..cols.len()].iter().map(|&c| (self.col(c), equal.iter().find(|e| e.0 == c).map_or(0, |e| e.1))).collect();
        let cmp = |r: u32| by.iter().map(|&(v, x)| v[r as usize].cmp(&x)).find(|o| o.is_ne()).unwrap_or(std::cmp::Ordering::Equal);
        let lo = order.partition_point(|&r| cmp(r).is_lt());
        let hi = order.partition_point(|&r| cmp(r).is_le());
        let mut out: Vec<usize> = order[lo..hi.max(lo)].iter().map(|&r| r as usize).collect();
        out.sort_unstable();
        Some(out)
    }
    /// For SQL over its vtab and a view's constant filters (`lookup`); a join probes the sorted index in place
    /// (`sorted_rows`), or builds its own over a filtered view.
    fn indexed(&self, _: usize) -> bool { true }
    fn lookup(&self, c: usize, values: &[i64]) -> Option<Vec<usize>> {
        if self.values[c].get().is_none() {
            return Some(if values.contains(&0) { (0..self.len()).collect() } else { Vec::new() });
        }
        let mut out: Vec<usize> = values.iter().flat_map(|&v| self.rows_of(c, v).iter().map(|&r| r as usize)).collect();
        out.sort_unstable();
        Some(out)
    }
    /// A column held nowhere (no rule names it, `columns_source`) reads as 0.
    fn value(&self, row: usize, c: usize) -> i64 { self.col(c).get(row).copied().unwrap_or(0) }
    fn sorted_rows(&self, c: usize, v: i64) -> Option<&[u32]> { self.values[c].get().map(|_| self.rows_of(c, v)) }
    fn sorts_rows(&self, c: usize) -> bool { self.values[c].get().is_some() }
}
/// A relation held in memory as columns (`None`: a column no rule reads, which reads as 0), read in place: a
/// probe on a column is a slice of its sorted index, built on first use.
pub fn columns_source(columns: Vec<String>, values: Vec<Option<Vec<i64>>>) -> Arc<dyn crate::code_rule_dsl::facts::vtab::Source> {
    let n = values.iter().flatten().map(Vec::len).max().unwrap_or(0);
    let mut t = Stored::new(columns, values.into_iter().map(|v| { let l = std::sync::OnceLock::new(); if let Some(v) = v { let _ = l.set(v); } l }).collect());
    t.statistics = Some(std::sync::OnceLock::new());
    let _ = t.n.set(n);
    Arc::new(t)
}
/// `columns_source`, its statistics given (kept by delta, decision 0096): read as they are, never measured.
pub fn columns_source_measured(columns: Vec<String>, values: Vec<Option<Vec<i64>>>, statistics: Vec<(String, i64, i64, i64, i64)>) -> Arc<dyn crate::code_rule_dsl::facts::vtab::Source> {
    let n = values.iter().flatten().map(Vec::len).max().unwrap_or(0);
    let mut t = Stored::new(columns, values.into_iter().map(|v| { let l = std::sync::OnceLock::new(); if let Some(v) = v { let _ = l.set(v); } l }).collect());
    let kept = std::sync::OnceLock::new();
    let _ = kept.set(statistics);
    t.statistics = Some(kept);
    let _ = t.n.set(n);
    Arc::new(t)
}
/// Last run's sorted columns and orders of a relation held in memory (`old`), carried into this run's copy of it
/// (`new`, `columns_source`), patched by the rows that changed (decision 0101 §5), not sorted again. Both are read
/// run by run of one document's rows (`Stored::runs`): a run whose rows are the same, in the same
/// order, keeps its rows' places in each index, and the rows of a run that changed or is new are sorted alone and
/// merged in. Each index is then exactly the one `Sorted::of` (or `order`) would build over the rows.
///
/// The rows keep `new`'s order when the runs kept are in their old order. Otherwise (the files parsed this run come
/// first) they are laid out as the runs kept, in their old order, then the rest in `new`'s: a copy, no sort. That
/// copy is returned (`Some`) in place of `new`. Nothing is carried when the two hold other columns, or more than a
/// quarter of the rows changed. Also the number of indexes carried.
pub fn carry_sorted(old: &dyn crate::code_rule_dsl::facts::vtab::Source, new: &dyn crate::code_rule_dsl::facts::vtab::Source) -> (Option<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>>, usize) {
    carry_sorted_at(old, new, CARRY_AT)
}

/// `carry_sorted`, for a source of at least `at` rows.
fn carry_sorted_at(old: &dyn crate::code_rule_dsl::facts::vtab::Source, new: &dyn crate::code_rule_dsl::facts::vtab::Source, at: usize) -> (Option<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>>, usize) {
    let (Some(old), Some(new)) = (old.as_any().and_then(|a| a.downcast_ref::<Stored>()), new.as_any().and_then(|a| a.downcast_ref::<Stored>())) else { return (None, 0) };
    let k = new.columns.len();
    let held = |t: &Stored| (0..k).map(|c| t.values[c].get().is_some()).collect::<Vec<_>>();
    if k == 0 || old.columns != new.columns || held(old) != held(new) || !held(new)[0] {
        return (None, 0);
    }
    // A source this small sorts in about what the carry costs; a column in row order is read in one pass
    // (`Sorted::of`).
    if new.n.get().copied().unwrap_or(0) < at {
        return (None, 0);
    }
    let wanted: Vec<usize> = (0..k).filter(|&c| old.index[c].get().is_some() && new.index[c].get().is_none() && !new.col(c).is_sorted()).collect();
    let orders: Vec<(Vec<usize>, Arc<Vec<u32>>)> = old.orders.lock().unwrap().clone();
    if wanted.is_empty() && orders.is_empty() {
        return (None, 0);
    }
    let (was, now) = (old.runs(), new.runs());
    // Rows whose first column does not keep a document's rows together (an export keyed by a symbol): no runs to
    // keep, and the carry would cost more than a sort.
    if now.len() * 16 > n_new_of(new) || was.len() * 16 > n_new_of(old) {
        return (None, 0);
    }
    let n_old = old.n.get().copied().unwrap_or(0);
    let n_new = new.n.get().copied().unwrap_or(0);
    let at: rustc_hash::FxHashMap<(i64, u32), (usize, usize)> = was.iter().map(|&(d, a, b)| (d, (a, b))).collect();
    let cols: Vec<usize> = (0..k).filter(|&c| new.values[c].get().is_some()).collect();
    // Each run now: the old run it is (its rows the same), if any.
    let same: Vec<Option<usize>> = now
        .iter()
        .map(|&(d, a, b)| at.get(&d).filter(|&&(oa, ob)| ob - oa == b - a && cols.iter().all(|&c| old.col(c)[oa..ob] == new.col(c)[a..b])).map(|&(oa, _)| oa))
        .collect();
    let changed: usize = now.iter().zip(&same).filter(|(_, s)| s.is_none()).map(|(&(_, a, b), _)| b - a).sum();
    if changed * 4 > n_new.max(1) {
        return (None, 0);
    }
    let in_order = same.iter().flatten().is_sorted();
    // The rows as laid out: `new`'s, or the runs kept in their old order and then the rest.
    let laid: Option<Stored> = (!in_order).then(|| {
        let mut kept: Vec<(usize, usize)> = now.iter().zip(&same).filter_map(|(&(_, a, b), s)| s.map(|oa| (oa, b - a))).collect();
        kept.sort_unstable();
        let rest: Vec<(usize, usize)> = now.iter().zip(&same).filter(|(_, s)| s.is_none()).map(|(&(_, a, b), _)| (a, b)).collect();
        let values = (0..k)
            .map(|c| {
                let l = std::sync::OnceLock::new();
                if new.values[c].get().is_some() {
                    let mut v = Vec::with_capacity(n_new);
                    kept.iter().for_each(|&(oa, n)| v.extend_from_slice(&old.col(c)[oa..oa + n]));
                    rest.iter().for_each(|&(a, b)| v.extend_from_slice(&new.col(c)[a..b]));
                    let _ = l.set(v);
                }
                l
            })
            .collect();
        let mut t = Stored::new(new.columns.clone(), values);
        let _ = t.n.set(n_new);
        t.statistics = new.statistics.as_ref().map(|s| {
            let l = std::sync::OnceLock::new();
            if let Some(v) = s.get() {
                let _ = l.set(v.clone());
            }
            l
        });
        t
    });
    let t: &Stored = laid.as_ref().unwrap_or(new);
    // Old row → its row in `t` (u32::MAX: gone), and the rows of `t` that are not old ones (ascending).
    let mut remap = vec![u32::MAX; n_old];
    let mut added: Vec<u32> = Vec::new();
    if in_order {
        for (&(_, a, b), s) in now.iter().zip(&same) {
            match s {
                Some(oa) => {
                    let mut i = 0;
                    while i < b - a {
                        remap[oa + i] = (a + i) as u32;
                        i += 1;
                    }
                }
                None => added.extend(a as u32..b as u32),
            }
        }
    } else {
        let mut kept: Vec<(usize, usize)> = now.iter().zip(&same).filter_map(|(&(_, a, b), s)| s.map(|oa| (oa, b - a))).collect();
        kept.sort_unstable();
        let mut to = 0u32;
        for (oa, n) in kept {
            let mut i = 0;
            while i < n {
                remap[oa + i] = to + i as u32;
                i += 1;
            }
            to += n as u32;
        }
        added.extend(to..n_new as u32);
    }
    charge(Unit::IndexEntry, added.len() as u64);
    let mut carried = 0;
    let remap = &remap;
    for c in wanted {
        let col = t.col(c);
        let s = old.index[c].get().expect("wanted");
        let mut fresh: Vec<(i64, u32)> = added.iter().map(|&r| (col[r as usize], r)).collect();
        fresh.sort_unstable();
        // A merge by (value, row) of the old rows kept, renumbered, and the new ones: plain loops, as a debug build
        // runs them at about a sort's cost otherwise.
        let (mut ids, mut keys, mut starts) = (Vec::with_capacity(n_new), Vec::with_capacity(s.keys.len()), Vec::with_capacity(s.keys.len() + 1));
        let (sk, ss, si) = (s.keys.as_slice(), s.starts.as_slice(), s.ids.as_slice());
        let mut f = 0;
        let mut last: Option<i64> = None;
        let mut key = 0;
        while key < sk.len() {
            let v = sk[key];
            let mut at = ss[key] as usize;
            let end = ss[key + 1] as usize;
            while at < end {
                let to = remap[si[at] as usize];
                at += 1;
                if to == u32::MAX {
                    continue;
                }
                while f < fresh.len() && fresh[f] < (v, to) {
                    if last != Some(fresh[f].0) {
                        last = Some(fresh[f].0);
                        keys.push(fresh[f].0);
                        starts.push(ids.len() as u32);
                    }
                    ids.push(fresh[f].1);
                    f += 1;
                }
                if last != Some(v) {
                    last = Some(v);
                    keys.push(v);
                    starts.push(ids.len() as u32);
                }
                ids.push(to);
            }
            key += 1;
        }
        while f < fresh.len() {
            if last != Some(fresh[f].0) {
                last = Some(fresh[f].0);
                keys.push(fresh[f].0);
                starts.push(ids.len() as u32);
            }
            ids.push(fresh[f].1);
            f += 1;
        }
        starts.push(ids.len() as u32);
        charge(Unit::Value, n_new as u64);
        if t.index[c].set(Sorted { ids, keys, starts }).is_ok() {
            carried += 1;
        }
    }
    // Orders over several columns: rows equal on them stay in row order, as `order`'s stable sort leaves them.
    {
        let mut into = t.orders.lock().unwrap();
        for (by, ids) in orders {
            if into.iter().any(|(o, _)| *o == by) || by.iter().any(|&c| t.values[c].get().is_none()) {
                continue;
            }
            let vs: Vec<&[i64]> = by.iter().map(|&c| t.col(c)).collect();
            let cmp = |a: u32, b: u32| vs.iter().map(|v| v[a as usize].cmp(&v[b as usize])).find(|o| o.is_ne()).unwrap_or_else(|| a.cmp(&b));
            let mut fresh = added.clone();
            fresh.sort_by(|&a, &b| cmp(a, b));
            let mut out = Vec::with_capacity(n_new);
            let mut fresh = fresh.into_iter().peekable();
            for to in ids.iter().map(|&r| remap[r as usize]).filter(|&r| r != u32::MAX) {
                while let Some(&f) = fresh.peek()
                    && cmp(f, to).is_lt()
                {
                    out.push(f);
                    fresh.next();
                }
                out.push(to);
            }
            out.extend(fresh);
            charge(Unit::Value, n_new as u64);
            into.push((by, Arc::new(out)));
            carried += 1;
        }
    }
    (laid.map(|t| Arc::new(t) as Arc<dyn crate::code_rule_dsl::facts::vtab::Source>), carried)
}

fn n_new_of(t: &Stored) -> usize {
    t.n.get().copied().unwrap_or(0)
}

/// The rows at which a source's sorted columns are carried rather than sorted again (`carry_sorted`).
const CARRY_AT: usize = 32_768;

/// The table `schema.name` read once into memory, every column (`None`: not a table of integers).
pub fn table_source(conn: &Connection, schema: &str, name: &str) -> Option<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>> {
    let columns: Vec<String> = conn.prepare(&format!("SELECT * FROM {schema}.{}", crate::code_rule_dsl::logic::sql::ident(name))).ok()?.column_names().map(str::to_owned).collect();
    let need: Vec<&str> = columns.iter().map(String::as_str).collect();
    stored_table(conn, schema, name, &need)
}
/// The stored tables read this run, by database and `schema.table` (dropped with the inputs, `forget_inputs`).
static STORED: LazyLock<Mutex<HashMap<(String, String), Option<Arc<Stored>>>>> = LazyLock::new(Default::default);
/// The stored table `schema.name` with the columns named in `need` loaded (`None`: not one, or not integers).
fn stored_table(conn: &Connection, schema: &str, name: &str, need: &[&str]) -> Option<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>> {
    if schema == "temp" || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') { return None; }
    let key = (conn_path(conn).to_string(), format!("{schema}.{name}"));
    let t = STORED.lock().unwrap().get(&key).cloned();
    let t = match t {
        Some(t) => t?,
        None => {
            // The columns' names, from the statement (prepared, not run).
            let columns: Vec<String> = conn.prepare(&format!("SELECT * FROM {schema}.{}", crate::code_rule_dsl::logic::sql::ident(name))).ok()?.column_names().map(str::to_owned).collect();
            let k = columns.len();
            let t = Arc::new(Stored::new(columns, (0..k).map(|_| std::sync::OnceLock::new()).collect()));
            STORED.lock().unwrap().insert(key.clone(), Some(t.clone()));
            t
        }
    };
    let need: Vec<usize> = need.iter().map(|c| t.columns.iter().position(|n| n == c.trim())).collect::<Option<_>>()?;
    if !t.load(conn, &format!("{schema}.{name}"), &need) {
        STORED.lock().unwrap().insert(key, None);
        return None;
    }
    Some(t as Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)
}

#[derive(Clone, Copy)]
pub enum Key { Slot(usize), Const(i64) }
#[derive(Clone)]
pub enum Src { Delta(usize), Full(usize), FullMulti(usize, Vec<usize>), In(Arc<InRel>, Arc<Csr>) }
#[derive(Clone)]
pub enum Step {
    Unused { proof: crate::code_rule_dsl::logic::analysis::Decision },
    /// Key columns (bound), columns that bind a slot, and column pairs that must be equal (a variable repeated in
    /// the atom, first seen here).
    Atom { src: Src, neg: bool, keys: Vec<(usize, Key)>, binds: Vec<(usize, usize)>, tests: Vec<(usize, usize)> },
    Cat { out: usize, a: Key, b: Key, bind: bool },
    /// `whole` split as `@dot` makes it into `a` and `b` (`""` and the whole, or at each "." with a part before it);
    /// `Some` slots bind, `None` parts (`ka`/`kb`) are tested.
    Split { whole: Key, a: Option<usize>, b: Option<usize>, ka: Key, kb: Key },
    Eq { a: Key, b: Key },
    Lt { a: Key, b: Key, or_eq: bool },
    /// Direct non-null scalar. The shared logical proof selects a bag; the source owns versioned summaries.
    Scalar { src: Src, keys: Vec<(usize, Key)>, tests: Vec<(usize, usize)>, col: Option<usize>,
        kind: ScalarKind, out: usize, bind: bool, bag: Option<Arc<scalars::Bag>> },
    Let { a: usize, b: Key },
    Ne { a: Key, b: Key },
    /// `out = @f(args)` for a `strings::functions` function: text arguments are symbols' names, a text result is
    /// interned; no value (NULL), no binding.
    Func { name: &'static str, kinds: &'static [crate::code_rule_dsl::logic::strings::functions::Kind], args: Vec<Key>, out: usize, bind: bool },
    /// `@doc(a) = @doc(b)` (`eq`) or `!=`: the nodes' documents, `d` of `d << 32 | index`, compared.
    Doc { a: Key, b: Key, eq: bool },
    /// `out = @doc(n)`: node n's document, bound or tested.
    DocOf { out: usize, n: Key, bind: bool },
    /// `@op(s, p) = want`, a text test (`strings::TESTS`) of symbol `s`'s name: by `matcher` when the pattern is a
    /// constant that compiles (once, with the spec), else by `strings::test` of symbol `p`'s name.
    Test { op: &'static str, s: Key, p: Key, matcher: Option<Arc<crate::code_rule_dsl::logic::strings::Matcher>>, want: bool },
    /// `a` is (`want`) or is not one of the constants `set` (`@equals(x, {…})`, `@in(x, {…})` once lowered).
    In { a: Key, set: Vec<i64>, want: bool },
    /// `out = join x by (k…) sep s : { atom }`: the matching rows' distinct (k…, x), ordered by k… then x (numbers by
    /// value, symbols by their names' bytes), their x joined by `sep`'s name, as SQL's `rr_join` makes it; "" for none.
    /// `x` and each `k` are columns, each with whether it holds symbols.
    StrJoin { src: Src, keys: Vec<(usize, Key)>, tests: Vec<(usize, usize)>, x: (usize, bool), by: Vec<(usize, bool)>, sep: i64, out: usize, bind: bool },
}
/// `multi`: the composite indexes (relation, key columns) its steps read, which the component maintains. `counts`:
/// per step, the bindings that entered it and those it passed on, since `set_fixpoint_steps` last read them: the
/// observed fanout of each step, which replanning uses in place of the estimate.
/// `fused`: by step, an atom fused with the filters after it into a row program (`rowcode`).
/// `forms`: per step, its count-table form's state (`table`).
/// `spec`: the digest of the spec it was read from, which a level's batch keeps its pairs by (`level`).
pub struct JoinRule { pub steps: Vec<Step>, pub out: Vec<Term>, pub nslots: usize, multi: Vec<(usize, Vec<usize>)>, pub counts: Vec<[std::cell::Cell<u64>; 2]>,
                      pub fused: Vec<Option<Box<rowcode::Fused>>>, forms: Vec<table::StepForm>, spec: u64 }

fn parse_join(conn: &Connection, spec: &str, syms: &mut Interner) -> Result<JoinRule> {
    join_rule(conn, &JoinSpec::parse(spec).map_err(err)?, syms)
}

/// A native join's steps from its spec: each slot bound by its first step, tested by later ones.
fn join_rule(conn: impl Scoped, spec: &JoinSpec, syms: &mut Interner) -> Result<JoinRule> {
    let conn = conn.db();
    join_rule_with(conn, spec, syms, true)
}

/// `join_rule`, with the CPU's indexes of its input relations (`index`) or without them: a join the device runs
/// reads its relations by its own sorts (`device`), so building the CPU's would be work for nothing. Without them
/// the join must not run on the CPU (`open` parses it again with them when the device refuses it).
fn join_rule_with(conn: impl Scoped, spec: &JoinSpec, syms: &mut Interner, index: bool) -> Result<JoinRule> {
    let conn = conn.db();
    let unindexed = |cols: &[usize]| Arc::new(Csr { cols: cols.to_vec(), table: HashTable::new(), ids: Vec::new(), source: false, sorted: false, chain: None, lead: 0, composite: false });
    let mut bound: Vec<bool> = Vec::new();
    let mut steps = Vec::new();
    let mut out = Vec::new();
    let mut multi = Vec::new();
    fn tok(t: &Tok, syms: &mut Interner) -> Option<Key> {
        match t { Tok::Wild => None, Tok::Int(i) => Some(Key::Const(*i)), Tok::Sym(x) => Some(Key::Const(syms.id(x))), Tok::Slot(s) => Some(Key::Slot(*s)) }
    }
    let grow = |b: &mut Vec<bool>, s: usize| if b.len() <= s { b.resize(s + 1, false) };
    // An aggregate's value goes into its out slot: bound there (true), or tested where an earlier step bound it.
    fn out_slot(o: &Tok, what: &str, bound: &mut Vec<bool>, syms: &mut Interner) -> Result<(usize, bool)> {
        let Some(Key::Slot(o)) = tok(o, syms) else { return Err(err(format!("bad {what}"))) };
        let bind = !bound.get(o).copied().unwrap_or(false);
        if bound.len() <= o { bound.resize(o + 1, false) }
        bound[o] = true;
        Ok((o, bind))
    }
    for item in &spec.items {
        let is_bound = |b: &Vec<bool>, k: Key| match k { Key::Slot(s) => b.get(s).copied().unwrap_or(false), Key::Const(_) => true };
        match item {
            JoinItem::Atom { src, args: atoks, .. } | JoinItem::Count { src, args: atoks, .. } | JoinItem::Extreme { src, args: atoks, .. } | JoinItem::StrJoin { src, args: atoks, .. } => {
                let neg = matches!(item, JoinItem::Atom { neg: true, .. });
                let extreme = matches!(item, JoinItem::Extreme { .. });
                // A strjoin counts its rows as a count does, then joins its value column's.
                let count = matches!(item, JoinItem::Count { .. } | JoinItem::StrJoin { .. });
                let (mut keys, mut binds, mut tests) = (Vec::new(), Vec::new(), Vec::new());
                let mut locals: HashMap<&str, usize> = HashMap::default();
                for (col, t) in atoks.iter().enumerate() {
                    let t = match t {
                        spec::Arg::Local(local) if count || extreme => {
                            if let Some(previous) = locals.insert(local.as_str(), col) { tests.push((col, previous)); }
                            continue;
                        }
                        spec::Arg::Local(local) => return Err(err(format!("bad slot a{local}"))),
                        spec::Arg::Tok(t) => t,
                    };
                    match tok(t, syms) {
                        None => {}
                        Some(k) if is_bound(&bound, k) => keys.push((col, k)),
                        Some(Key::Slot(s)) => {
                            if neg || count || extreme { return Err(err(format!("unbound v{s} in a negated or aggregated atom"))); }
                            match binds.iter().find(|&&(_, b)| b == s) { Some(&(c0, _)) => tests.push((col, c0)), None => binds.push((col, s)) }
                        }
                        Some(Key::Const(_)) => unreachable!(),
                    }
                }
                for &(_, s) in &binds { grow(&mut bound, s); bound[s] = true; }
                // Every key set a component relation is probed by has its order (`Rel::want`).
                if let Source::Delta(r) | Source::Full(r) = src && !keys.is_empty() {
                    multi.push((*r, keys.iter().map(|&(c, _)| c).collect()));
                }
                let src = match src {
                    Source::Delta(r) => Src::Delta(*r),
                    Source::Full(r) if keys.len() >= 2 => {
                        Src::FullMulti(*r, keys.iter().map(|&(c, _)| c).collect())
                    }
                    Source::Full(r) => Src::Full(*r),
                    Source::Native(table) => {
                        let rel = DERIVED.lock().unwrap().get(&(conn.at(), table.clone())).cloned().ok_or_else(|| err(format!("no native relation {table}")))?;
                        let cols: Vec<usize> = keys.iter().map(|&(c, _)| c).collect();
                        let ix = if index { rel.index(&cols) } else { unindexed(&cols) };
                        Src::In(rel, ix)
                    }
                    Source::Input(key) => {
                        let cols: Vec<usize> = keys.iter().map(|&(c, _)| c).collect();
                        let rel = input_of(conn, key, syms)?;
                        let ix = if index { rel.index(&cols) } else { unindexed(&cols) };
                        Src::In(rel, ix)
                    }
                };
                match item {
                    JoinItem::StrJoin { x, by, sep, out: o, .. } => {
                        let Some(Key::Const(sep)) = tok(sep, syms) else { return Err(err("a strjoin's separator is a constant")) };
                        let (o, bind) = out_slot(o, "strjoin", &mut bound, syms)?;
                        steps.push(Step::StrJoin { src, keys, tests, x: *x, by: by.clone(), sep, out: o, bind });
                    }
                    JoinItem::Extreme { op, col, out: o, .. } => {
                        if *col >= atoks.len() { return Err(err("native scalar value column is out of range")); }
                        let (o, bind) = out_slot(o, "min or max", &mut bound, syms)?;
                        let kind = match op { spec::Extreme::Sum => ScalarKind::Sum, spec::Extreme::Min => ScalarKind::Min, spec::Extreme::Max => ScalarKind::Max };
                        steps.push(Step::Scalar { src, keys, tests, col: Some(*col), out: o, bind, kind, bag: None });
                    }
                    JoinItem::Count { out: o, .. } => {
                        let (o, bind) = out_slot(o, "count", &mut bound, syms)?;
                        steps.push(Step::Scalar { src, keys, tests, col: None, out: o, bind, kind: ScalarKind::Count, bag: None });
                    }
                    _ => steps.push(Step::Atom { src, neg, keys, binds, tests }),
                }
            }
            JoinItem::Cat { out: o, a, b } => {
                let (o, a, b) = (tok(o, syms), tok(a, syms), tok(b, syms));
                let (Some(Key::Slot(o)), Some(a), Some(b)) = (o, a, b) else { return Err(err(format!("bad cat: {item}"))) };
                if !is_bound(&bound, a) || !is_bound(&bound, b) {
                    if !is_bound(&bound, Key::Slot(o)) { return Err(err("cat of an unbound slot")); }
                    let part = |b: &mut Vec<bool>, k: Key| match k {
                        Key::Slot(s) if !is_bound(b, k) => { grow(b, s); b[s] = true; Some(s) }
                        _ => None,
                    };
                    let (sa, sb) = (part(&mut bound, a), part(&mut bound, b));
                    steps.push(Step::Split { whole: Key::Slot(o), a: sa, b: sb, ka: a, kb: b });
                    continue;
                }
                let bind = !is_bound(&bound, Key::Slot(o));
                grow(&mut bound, o); bound[o] = true;
                steps.push(Step::Cat { out: o, a, b, bind });
            }
            JoinItem::Lt { a, b, or_eq } => {
                let (Some(a), Some(b)) = (tok(a, syms), tok(b, syms)) else { return Err(err("bad comparison")) };
                if !is_bound(&bound, a) || !is_bound(&bound, b) { return Err(err("order of an unbound slot")); }
                steps.push(Step::Lt { a, b, or_eq: *or_eq });
            }
            JoinItem::Func { name, out: o, args } => {
                let Some((name, kinds, _)) = crate::code_rule_dsl::logic::strings::functions::signature(name) else { return Err(err(format!("no function {name}"))) };
                let args: Vec<Key> = args.iter().map(|t| tok(t, syms)).collect::<Option<Vec<_>>>().ok_or_else(|| err("a function of `_`"))?;
                if args.len() != kinds.len() || args.iter().any(|&a| !is_bound(&bound, a)) { return Err(err(format!("{name} of unbound or missing arguments"))); }
                let Some(Key::Slot(o)) = tok(o, syms) else { return Err(err("a function's value into a constant")) };
                let bind = !is_bound(&bound, Key::Slot(o));
                grow(&mut bound, o); bound[o] = true;
                steps.push(Step::Func { name, kinds, args, out: o, bind });
            }
            JoinItem::Test { op, s, p, want } => {
                use crate::code_rule_dsl::logic::strings;
                let Some(&op) = strings::TESTS.iter().find(|&&t| t == op) else { return Err(err(format!("no text test {op}"))) };
                let (Some(s), Some(p)) = (tok(s, syms), tok(p, syms)) else { return Err(err("a text test of `_`")) };
                if !is_bound(&bound, s) || !is_bound(&bound, p) { return Err(err(format!("{op} of an unbound slot"))); }
                let matcher = match p {
                    Key::Const(c) => {
                        let patterns: Vec<String> = syms.name(c).split(strings::SET_SEP).map(str::to_owned).collect();
                        // A pattern that does not compile matches nothing, as SQL's `str_test`: `strings::test` per row.
                        strings::Matcher::new(op, &patterns).ok().map(Arc::new)
                    }
                    Key::Slot(_) => None,
                };
                steps.push(Step::Test { op, s, p, matcher, want: *want });
            }
            JoinItem::DocOf { out: o, n } => {
                let (Some(Key::Slot(o)), Some(n)) = (tok(o, syms), tok(n, syms)) else { return Err(err("bad docof")) };
                if !is_bound(&bound, n) { return Err(err("the document of an unbound slot")); }
                let bind = !is_bound(&bound, Key::Slot(o));
                grow(&mut bound, o); bound[o] = true;
                steps.push(Step::DocOf { out: o, n, bind });
            }
            JoinItem::TestSet { op, s, want, members } => {
                // A text test against a set of symbols' names.
                use crate::code_rule_dsl::logic::strings;
                let Some(&op) = strings::TESTS.iter().find(|&&t| t == op) else { return Err(err(format!("no text test {op}"))) };
                let Some(s) = tok(s, syms) else { return Err(err("a text test of `_`")) };
                if !is_bound(&bound, s) { return Err(err(format!("{op} of an unbound slot"))); }
                let patterns = members.iter().map(|t| match tok(t, syms) { Some(Key::Const(c)) => Ok(syms.name(c).to_string()), _ => Err(err("a pattern that is not a constant")) }).collect::<Result<Vec<String>>>()?;
                let matcher = strings::Matcher::new(op, &patterns).ok().map(Arc::new);
                // A set that does not compile matches nothing, as SQL's `str_test`.
                let p = Key::Const(syms.id(&patterns.join(&strings::SET_SEP.to_string())));
                steps.push(Step::Test { op, s, p, matcher, want: *want });
            }
            JoinItem::In { a, want, set } => {
                let Some(a) = tok(a, syms) else { return Err(err("a membership test of `_`")) };
                if !is_bound(&bound, a) { return Err(err("a membership test of an unbound slot")); }
                let set = set.iter().map(|t| match tok(t, syms) { Some(Key::Const(c)) => Ok(c), _ => Err(err("a set member that is not a constant")) }).collect::<Result<Vec<_>>>()?;
                steps.push(Step::In { a, set, want: *want });
            }
            JoinItem::Doc { a, b, eq } => {
                let (Some(a), Some(b)) = (tok(a, syms), tok(b, syms)) else { return Err(err("bad comparison")) };
                if !is_bound(&bound, a) || !is_bound(&bound, b) { return Err(err("a document comparison of an unbound slot")); }
                steps.push(Step::Doc { a, b, eq: *eq });
            }
            JoinItem::Eq { a, b, ne } => {
                let (Some(a), Some(b)) = (tok(a, syms), tok(b, syms)) else { return Err(err("bad comparison")) };
                if !is_bound(&bound, b) { return Err(err("comparison with an unbound right side")); }
                match (*ne, a) {
                    (false, Key::Slot(s)) if !is_bound(&bound, a) => { grow(&mut bound, s); bound[s] = true; steps.push(Step::Let { a: s, b }) }
                    _ if !is_bound(&bound, a) => return Err(err("comparison with an unbound left side")),
                    (false, _) => steps.push(Step::Eq { a, b }),
                    _ => steps.push(Step::Ne { a, b }),
                }
            }
            JoinItem::Near { down, .. } => return Err(err(format!("unknown join item {}", if *down { "down" } else { "near" }))),
            JoinItem::Head(terms) => {
                for t in terms {
                    out.push(match t {
                        spec::HeadTok::Cat(a, b) => Term::Cat(*a, *b),
                        spec::HeadTok::Tok(t) => match tok(t, syms) { Some(Key::Const(c)) => Term::Const(c), Some(Key::Slot(s)) => Term::Slot(s), None => return Err(err("_ in a head")) },
                    });
                }
            }
        }
    }
    // Every head term reads a slot the body binds: an unbound one is refused here, not read out of range later.
    let unbound = |s: usize| !bound.get(s).copied().unwrap_or(false);
    if out.iter().any(|t| match t { Term::Slot(s) => unbound(*s), Term::Cat(a, b) => unbound(*a) || unbound(*b), Term::Const(_) => false }) {
        return Err(err(format!("a head reads a slot its body does not bind: {}", spec.to_text())));
    }
    liveness::sums(&mut steps,&out);
    scalars::plan(&mut steps);
    let counts = steps.iter().map(|_| Default::default()).collect();
    let fused = if env_set!("RERAIL_NO_ROW_PROGRAMS") { steps.iter().map(|_| None).collect() } else { rowcode::fuse(&steps) };
    let forms = (0..steps.len()).map(|_| table::StepForm::default()).collect();
    Ok(JoinRule { steps, out, nslots: bound.len(), multi, counts, fused, forms, spec: level::digest(spec) })
}

/// Runs a join probe over the store; its new head tuples go to `fresh`. Returns the tuples it derived.
fn run_join(j: &JoinRule, head: usize, store: &Store, syms: &mut Interner, fresh: &mut [Fresh]) -> Result<u64> {
    run_join_in(j, head, store, syms, fresh, &[], None, None)
}

/// Runs a seed join: from its pairs when a level's batch made them (`level`), else by the host's walk.
fn run_seed(j: &JoinRule, head: usize, store: &Store, syms: &mut Interner, fresh: &mut [Fresh]) -> Result<u64> {
    match level::take(j)? {
        Some(p) => {
            level::USED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            run_join_in(j, head, store, syms, fresh, &[], None, Some(&p))
        }
        None => run_join(j, head, store, syms, fresh),
    }
}

/// Sends a level's seed joins as one batch (decision 0100 §5): the launches sent.
pub fn prefetch_joins(conn: &Connection, specs: &[Arc<JoinSpec>]) -> Result<usize> {
    level::prefetch(conn, specs)
}

/// Drops a chunk's pending pairs and what the lanes keep for it.
pub fn level_clear() {
    level::clear()
}

/// The level batches' launches, and the seeds run from their pairs.
pub fn level_counts() -> (u64, u64) {
    level::counts()
}

/// Runs `j` with `preset` slots bound, and its head rows appended to `out` (not added to any relation).
fn run_join_collect(j: &JoinRule, store: &Store, syms: &mut Interner, preset: &[(usize, i64)], out: &mut Vec<i64>) -> Result<u64> {
    run_join_in(j, 0, store, syms, &mut [], preset, Some(out), None)
}

/// The join's rows for each value of slot `y` in turn, collected: `next` is handed the previous value's rows (empty
/// the first time) and gives the next value, or `None` to stop. One context for every value (a closure's search of
/// its edges, node after node), not one per value.
fn run_join_each(j: &JoinRule, store: &Store, syms: &mut Interner, y: usize, next: &mut dyn FnMut(&mut Vec<i64>) -> Option<i64>) -> Result<u64> {
    let mut out = Vec::new();
    run_join_in_each(j, 0, store, syms, &mut [], &[], Some(&mut out), None, Some((y, next)))
}

#[allow(clippy::too_many_arguments)]
fn run_join_in(j: &JoinRule, head: usize, store: &Store, syms: &mut Interner, fresh: &mut [Fresh], preset: &[(usize, i64)], collect: Option<&mut Vec<i64>>, paired: Option<&level::Paired>) -> Result<u64> {
    run_join_in_each(j, head, store, syms, fresh, preset, collect, paired, None)
}

#[allow(clippy::too_many_arguments)]
fn run_join_in_each(j: &JoinRule, head: usize, store: &Store, syms: &mut Interner, fresh: &mut [Fresh], preset: &[(usize, i64)], collect: Option<&mut Vec<i64>>,
                    paired: Option<&level::Paired>, each: Option<(usize, &mut dyn FnMut(&mut Vec<i64>) -> Option<i64>)>) -> Result<u64> {
    // An empty required relation makes the entire conjunction empty. Check once per
    // invocation, before expanding another input; the next wave can grow this relation.
    // Negative atoms and scalar aggregates do not obey this rule.
    if j.steps.iter().any(|step| match step {
        Step::Atom { src, neg: false, .. } => match src {
            Src::Delta(r) => store.rels[*r].lo == store.rels[*r].hi,
            Src::Full(r) | Src::FullMulti(r, _) => store.rels[*r].n == 0,
            // A lazy whole view has no materialized row count. Asking its length
            // would itself enumerate the producer; absence is not known here.
            Src::In(r, _) => r.n == 0 && r.view.as_ref().is_none_or(|v| v.ids.is_some()),
        },
        _ => false,
    }) {
        // The per-binding driver also records empty answers (closure adjacency,
        // for example). Drain it even though this join cannot emit any rows.
        if let Some((_, next)) = each {
            let out = collect.expect("a join run for each value collects");
            out.clear();
            while next(out).is_some() { out.clear(); }
        }
        return Ok(0);
    }
    struct Ctx<'a, 'c> { j: &'a JoinRule, head: usize, store: &'a Store, syms: &'a mut Interner, fresh: &'a mut [Fresh], n: u64,
                     key: Vec<i64>, row: Vec<i64>, error: Option<Error>, pending: Vec<(usize,i128)>, scratch: Vec<rowcode::Scratch>,
                     /// Each step's place in the runs of the order it probes (`Index::find_at`): the step's keys come
                     /// in a sorted batch where they can (`scan_order`), so each probe starts where the last ended.
                     hints: Vec<[u32; runs::HINTS]>,
                     collect: Option<&'c mut Vec<i64>>,
                     /// The row the first step is at (a count table's counts per row are by it).
                     at0: u32,
                     /// Per atom step, the slots it binds that the steps before `upto` read, and `upto` (`factored`):
                     /// a row whose rest of the body fails before reaching `upto` fails by those values alone.
                     factor: Vec<Option<Factor>>,
                     /// The deepest step entered (`go`), so a factored step knows where its row's descent stopped.
                     deep: usize,
                     /// Per such step, in its current probe, the values of those slots the rest of the body failed for.
                     failed: Vec<rustc_hash::FxHashSet<[i64; FACTOR_MAX]>>,
                     /// Rows passed over by `failed`: read and tested, not joined on.
                     skipped: u64 }
    fn val(slots: &[i64], k: Key) -> i64 { match k { Key::Slot(s) => slots[s], Key::Const(c) => c } }
    /// Where an atom's candidate tuples are: an input relation or a component relation, by id.
    #[derive(Clone, Copy)]
    enum Rows<'a> { In(&'a InRel), Comp(&'a Rel), Source(&'a View) }
    impl<'a> Rows<'a> {
        fn row(self, id: u32) -> RowRef<'a> {
            match self { Rows::In(r) => r.at(id as usize).read(), Rows::Comp(r) => RowRef::Flat(r.row(id as usize)), Rows::Source(v) => RowRef::Source(v, id as usize).read() }
        }
        /// A row deleted in place (`Rel::based`), which no step reads.
        #[inline]
        fn dead(self, id: u32) -> bool {
            match self { Rows::In(r) => is_dead(&r.dead, id), Rows::Comp(r) => r.dead(id), Rows::Source(_) => false }
        }
    }
    /// The rows of step `i`'s scan (`cands`, a range of at least `SCAN_SORTED` rows) in the order of the key step
    /// `i + 1` probes with, when the scan binds all of that key: the next step's probes then come as one sorted batch,
    /// each starting where the last ended in each run (`Index::find_at`), and read each run once, in order.
    fn scan_order(j: &JoinRule, store: &Store, i: usize, rows: Rows<'_>, cands: &Cands<'_>, binds: &[(usize, usize)]) -> Option<Vec<u32>> {
        const SCAN_SORTED: u32 = 256;
        let &Cands::Range(lo, hi) = cands else { return None };
        if hi - lo < SCAN_SORTED || j.fused.get(i).is_some_and(Option::is_some) {
            return None;
        }
        let Some(Step::Atom { src, keys, .. }) = j.steps.get(i + 1) else { return None };
        if keys.is_empty() {
            return None;
        }
        let cols: Vec<usize> = keys.iter().map(|&(c, _)| c).collect();
        let (index, m, k) = match src {
            Src::Delta(r) | Src::Full(r) => { let rel = &store.rels[*r]; let (ix, m) = best_order(&rel.idx, &cols)?; (ix, m, rel.k) }
            Src::FullMulti(r, mc) => { let rel = &store.rels[*r]; let (ix, m) = best_order(&rel.idx, mc)?; (ix, m, rel.k) }
            Src::In(rel, ix) => { let index = &rel.chains[ix.chain?]; let m = index.cols.iter().take_while(|c| ix.cols.contains(c)).count(); (index, m, rel.k) }
        };
        if m == 0 {
            return None;
        }
        // Each of the order's key columns, from this step's row: a column it binds, or a constant.
        enum From { Col(usize), Const(i64) }
        let mut from = Vec::with_capacity(m);
        for c in &index.cols[..m] {
            let &(_, key) = keys.iter().find(|(kc, _)| kc == c)?;
            from.push(match key {
                Key::Const(v) => From::Const(v),
                Key::Slot(s) => From::Col(binds.iter().find(|&&(_, b)| b == s)?.0),
            });
        }
        let n = (hi - lo) as usize;
        let mut vals = Vec::with_capacity(n * m);
        for id in lo..hi {
            let row = rows.row(id);
            vals.extend(from.iter().map(|f| match *f { From::Col(c) => row.at(c), From::Const(v) => v }));
        }
        let tagged = index.is_tagged(k);
        let mut order: Vec<u32> = (0..n as u32).collect();
        let cmp = |a: u32, b: u32| vals[a as usize * m..][..m].cmp(&vals[b as usize * m..][..m]);
        runs::sort_keyed(&mut order, tagged, m, |q, i| vals[q as usize * m + i], cmp);
        Some(order.into_iter().map(|q| lo + q).collect())
    }
    fn go<'a>(cx: &mut Ctx<'a, '_>, i: usize, slots: &mut [i64]) {
        if cx.error.is_some() { return; }
        let (j, store): (&'a JoinRule, &'a Store) = (cx.j, cx.store);
        if i > 0 { let c = &j.counts[i - 1][1]; c.set(c.get() + 1); }
        if let Some(c) = j.counts.get(i) { c[0].set(c[0].get() + 1); }
        if i > cx.deep { cx.deep = i; }
        // Keep exact sums until a predicate, lookup, function, or emitted head
        // observes the binding. A later filter can remove a row before that use.
        if !cx.pending.is_empty() {
            if let Some((_,value))=cx.pending.iter().find(|(slot,_)|j.steps.get(i).map_or_else(
                ||j.out.iter().any(|term|matches!(term,Term::Slot(s) if s==slot)||matches!(term,Term::Cat(a,b) if a==slot||b==slot)),
                |step|step.observes(*slot))) {
                cx.error=Some(super::aggregation::sum_value(*value).unwrap_err());return;
            }
        }
        let Some(step) = j.steps.get(i) else {
            cx.row.clear();
            for o in &j.out {
                let v = match o { Term::Slot(s) => slots[*s], Term::Const(c) => *c, Term::Cat(a, b) => cat(cx.syms, slots[*a], slots[*b]) };
                cx.row.push(v);
            }
            cx.n += 1;
            if let Some(c) = &mut cx.collect { c.extend_from_slice(&cx.row); return; }
            cx.fresh[cx.head].emitted += 1;
            cx.fresh[cx.head].offer(&cx.row, &store.rels[cx.head]);
            return;
        };
        /// The candidates: an input's index on exactly the key columns, a component relation's composite index, or
        /// its shortest posting chain among its key columns (every key is then checked), or every tuple when none is
        /// bound.
        fn candidates<'a>(src: &'a Src, store: &'a Store, key: &[i64], keys: &[(usize, Key)], hints: &mut [u32; runs::HINTS]) -> (Rows<'a>, Cands<'a>) {
            match src {
                Src::In(rel, ix) => {
                    if !keys.is_empty() { probed::note(rel.name.get().map_or("?input", String::as_str), ix.cols.iter().copied()); }
                    if keys.is_empty() { (Rows::In(rel), Cands::Range(0, rel.len() as u32)) }
                    else if let Some(i) = ix.chain {
                        let index = &rel.chains[i];
                        // An order answering the key's columns, or leading with some of them (`chain_lead`: every key
                        // is checked): the key's values in the order's column order.
                        let m = index.cols.iter().take_while(|c| ix.cols.contains(c)).count();
                        let mut buf = [0i64; MAX_KEY];
                        let at = |c: usize| key[ix.cols.iter().position(|&x| x == c).expect("a key column")];
                        (Rows::In(rel), Cands::Runs(index.find_at(&rel.rows, rel.k, permuted(&index.cols[..m], at, &mut buf), None, hints)))
                    }
                    else if ix.composite && let Some(v) = &rel.view {
                        let mut eq = [(0usize, 0i64); MAX_KEY];
                        for (j, &c) in ix.cols.iter().enumerate() { eq[j] = (v.cols[c], key[j]); }
                        let rows = v.src.lookup_all(&eq[..ix.cols.len()]).unwrap_or_default();
                        (Rows::Source(v), Cands::Owned(rows.into_iter().map(|r| r as u32).collect()))
                    }
                    else if ix.sorted && let Some(v) = &rel.view { (Rows::Source(v), Cands::Slice(v.src.sorted_rows(v.cols[ix.cols[ix.lead]], key[ix.lead]).unwrap_or(&[]))) }
                    else if ix.source && let Some(v) = &rel.view { (Rows::Source(v), Cands::Owned(v.find(ix.cols[ix.lead], key[ix.lead]))) }
                    else { (Rows::In(rel), Cands::Slice(ix.get(rel, key))) }
                }
                Src::FullMulti(r, cols) => {
                    let rel = &store.rels[*r];
                    probed::note(&rel.table, cols.iter().copied());
                    (Rows::Comp(rel), rel.probe_at(cols, key, false, hints).0)
                }
                Src::Delta(r) | Src::Full(r) => {
                    let rel = &store.rels[*r];
                    let delta = matches!(src, Src::Delta(_));
                    if keys.is_empty() {
                        (Rows::Comp(rel), if delta { Cands::Range(rel.lo as u32, rel.hi as u32) } else { Cands::Range(0, rel.n as u32) })
                    } else {
                        if probed::on() { probed::note(&if delta { format!("{}+delta", rel.table) } else { rel.table.clone() }, keys.iter().map(|&(c, _)| c)); }
                        let mut cols = [0usize; MAX_KEY];
                        for (j, &(c, _)) in keys.iter().enumerate() { cols[j] = c; }
                        (Rows::Comp(rel), rel.probe_at(&cols[..keys.len()], key, delta, hints).0)
                    }
                }
        }
        }
        match step {
            Step::Atom { src, neg, keys, binds, tests } => {
                cx.key.clear();
                cx.key.extend(keys.iter().map(|&(_, k)| val(slots, k)));
                let (rows, mut cands) = candidates(src, store, &cx.key, keys, &mut cx.hints[i]);
                // A scan's rows in the next step's key order: its probes are then one sorted batch (decision 0112).
                if keys.is_empty() && let Some(order) = scan_order(j, store, i, rows, &cands, binds) {
                    cands = Cands::Owned(order);
                }
                if let Some(f) = j.fused[i].as_deref() {
                    // Fused with the filters after it: the candidates a chunk at a time, the filters one row program
                    // over each chunk, and only its rows go on (`rowcode`).
                    // Its filters run inside the row program, not through `go`: they count as reached (`factored`).
                    if i + f.steps > cx.deep { cx.deep = i + f.steps; }
                    let mut s = std::mem::take(&mut cx.scratch[i]);
                    s.columns.resize_with(f.columns.len(), Vec::new);
                    s.params.clear();
                    s.params.extend(f.params.iter().map(|p| { let v = val(slots, p.key); if p.doc { v >> 32 } else { v } }));
                    fn flush(cx: &mut Ctx<'_, '_>, i: usize, f: &rowcode::Fused, s: &mut rowcode::Scratch, slots: &mut [i64]) {
                        let counts = &cx.j.counts;
                        // Each fused step's entries and its passes, as `go` counts them step by step.
                        let bump = |m: usize, n: u64| {
                            let c = &counts[i + m][1]; c.set(c.get() + n);
                            let c = &counts[i + 1 + m][0]; c.set(c.get() + n);
                        };
                        bump(0, s.rows as u64);
                        let words = f.run(s, |m, n| if m + 1 < f.steps { bump(m + 1, n) }).to_vec();
                        for (w, &word) in words.iter().enumerate() {
                            let mut m = word;
                            while m != 0 {
                                let r = 64 * w + m.trailing_zeros() as usize;
                                m &= m - 1;
                                for &(slot, b) in &f.binds {
                                    slots[slot] = match b { rowcode::Bind::Column(k) => s.columns[k][r], rowcode::Bind::Param(k) => s.params[k] };
                                }
                                if i == 0 { cx.at0 = s.ids[r]; }
                                go(cx, i + 1 + f.steps, slots);
                            }
                        }
                        s.rows = 0;
                        s.ids.clear();
                        s.columns.iter_mut().for_each(Vec::clear);
                    }
                    cands.any(|id| {
                        if rows.dead(id) { return false; }
                        let row = rows.row(id);
                        if !keys.iter().all(|&(c, k)| row.at(c) == val(slots, k)) || !tests.iter().all(|&(c, c0)| row.at(c) == row.at(c0)) { return false; }
                        for &(k, c) in &f.gather { s.columns[k].push(row.at(c)); }
                        if i == 0 { s.ids.push(id); }
                        s.rows += 1;
                        if s.rows == rowcode::CHUNK { flush(cx, i, f, &mut s, slots); }
                        false
                    });
                    if s.rows > 0 { flush(cx, i, f, &mut s, slots); }
                    cx.scratch[i] = s;
                    return;
                }
                // An atom that binds nothing (`ann_root(_)`, a key all bound) is a test: every row it matches leads to
                // the same bindings, so the rest of the body runs once if one does (a semi-join), not once per row.
                let exists = *neg || binds.is_empty();
                // A row whose slots the rest of the body reads are those of a row it already failed for fails too
                // (`factored`): it is passed over, not joined on. `pfn`'s `pcall__ins` by `$o` gave 733,692 rows, each
                // tested against `fv` by `$m` alone and every one rejected, where few distinct `$m` were tested.
                let factor = if exists { None } else { cx.factor[i] };
                if factor.is_some() && !cx.failed[i].is_empty() { cx.failed[i].clear(); }
                let found = cands.any(|id| {
                    if rows.dead(id) { return false; }
                    let row = rows.row(id);
                    if !keys.iter().all(|&(c, k)| row.at(c) == val(slots, k)) || !tests.iter().all(|&(c, c0)| row.at(c) == row.at(c0)) { return false; }
                    if exists { return true; }
                    for &(c, s) in binds { slots[s] = row.at(c); }
                    if i == 0 { cx.at0 = id; }
                    if let Some(Factor { read, n, upto }) = factor {
                        let mut key = [0i64; FACTOR_MAX];
                        for (x, &s) in read[..n].iter().enumerate() { key[x] = slots[s]; }
                        if cx.failed[i].contains(&key) { cx.skipped += 1; return false; }
                        let (made, deep) = (cx.n, cx.deep);
                        cx.deep = i;
                        go(cx, i + 1, slots);
                        // Stopped before `upto`: only `read` of this row's bindings was looked at.
                        if cx.n == made && cx.deep < upto && cx.error.is_none() { cx.failed[i].insert(key); }
                        cx.deep = cx.deep.max(deep);
                        return false;
                    }
                    go(cx, i + 1, slots);
                    false
                });
                if exists && found != *neg { go(cx, i + 1, slots); }
            }
            Step::Scalar { src, keys, tests, col, kind, out, bind, bag } => {
                cx.key.clear();
                cx.key.extend(keys.iter().map(|&(_, k)| val(slots, k)));
                // A count from the step's count table, once it has one (`table`, decision 0098 §1).
                let counting = *kind == ScalarKind::Count && col.is_none();
                if counting && let Some(n) = table::count(&j.forms[i], src, keys, tests, &cx.key, cx.at0) {
                    if *bind { slots[*out] = n } else if slots[*out] != n { return }
                    go(cx, i + 1, slots);
                    return;
                }
                let walked = std::cell::Cell::new(0u64);
                let build = |value: Option<usize>| {
                    let _cpu=crate::code_rule_dsl::logic::profile::span(||"native scalar summary fold".into());
                    let mut inspected=0;
                    let (rows, cands) = candidates(src, store, &cx.key, keys, &mut [0; runs::HINTS]);
                    let mut summary = scalars::Summary::default();
                    cands.any(|id| {
                        inspected+=1;
                        // A row deleted in place (`Rel::based`) is in no group.
                        if rows.dead(id) { return false; }
                        let row = rows.row(id);
                        if keys.iter().all(|&(c,k)| row.at(c)==val(slots,k)) && tests.iter().all(|&(c,c0)| row.at(c)==row.at(c0)) {
                            summary.add(value.map(|c| row.at(c)));
                        }
                        false
                    });
                    charge(Unit::Value,inspected);
                    walked.set(walked.get() + inspected);
                    summary
                };
                let summary = if let Some(bag) = bag {
                    let (pool, epoch) = match src {
                        Src::In(rel, _) => (&rel.summaries, (rel.len(),0,rel.len(),rel.ndead)),
                        Src::Full(r) | Src::FullMulti(r, _) => {let rel=&store.rels[*r]; (&rel.summaries,(rel.n,0,rel.n,rel.ndead))},
                        Src::Delta(r) => {let rel=&store.rels[*r]; (&rel.summaries,(rel.n,rel.lo,rel.hi,rel.ndead))},
                    };
                    pool.evaluate(bag, &cx.key, epoch, build)
                } else { build(*col) };
                if counting { table::spent(&j.forms[i], &j.steps, i, walked.get()); }
                if *kind==ScalarKind::Sum && *bind {
                    cx.pending.retain(|(slot,_)|slot!=out);
                    if let Ok(value)=i64::try_from(summary.sum) {slots[*out]=value;}
                    else {slots[*out]=0;cx.pending.push((*out,summary.sum));}
                    go(cx,i+1,slots);return;
                }
                let v = match summary.result(*kind) {
                    Ok(Some(v)) => v,
                    Ok(None) => return,
                    Err(e) => {cx.error=Some(e); return;}
                };
                if *bind { slots[*out] = v } else if slots[*out] != v { return }
                go(cx, i+1, slots);
            }
            Step::StrJoin { src, keys, tests, x, by, sep, out, bind } => {
                // Numbers before symbols, as SQL's `rr_join` sorts them (a column holds one or the other).
                #[derive(PartialEq, Eq, PartialOrd, Ord)]
                enum Sortable { Number(i64), Text(String) }
                cx.key.clear();
                cx.key.extend(keys.iter().map(|&(_, k)| val(slots, k)));
                let (rows, cands) = candidates(src, store, &cx.key, keys, &mut cx.hints[i]);
                let mut got: Vec<(Vec<Sortable>, String)> = Vec::new();
                let syms = &cx.syms;
                let sortable = |v: i64, sym: bool| if sym { Sortable::Text(syms.name(v).to_string()) } else { Sortable::Number(v) };
                cands.any(|id| {
                    if rows.dead(id) { return false; }
                    let row = rows.row(id);
                    if keys.iter().all(|&(c, k)| row.at(c) == val(slots, k)) && tests.iter().all(|&(c, c0)| row.at(c) == row.at(c0)) {
                        let value = if x.1 { syms.name(row.at(x.0)).to_string() } else { row.at(x.0).to_string() };
                        // Then by x itself: a number by its value, not its digits.
                        got.push((by.iter().chain([x]).map(|&(c, sym)| sortable(row.at(c), sym)).collect(), value));
                    }
                    false
                });
                got.sort();
                got.dedup();
                let sep = cx.syms.name(*sep).to_string();
                let joined = got.into_iter().map(|(_, v)| v).collect::<Vec<_>>().join(&sep);
                let v = cx.syms.id(&joined);
                if *bind { slots[*out] = v } else if slots[*out] != v { return }
                go(cx, i + 1, slots);
            }
            Step::Lt { a, b, or_eq } => {
                let (x, y) = (val(slots, *a), val(slots, *b));
                if x < y || (*or_eq && x == y) { go(cx, i + 1, slots) }
            }
            Step::Cat { out, a, b, bind } => {
                let v = cat(cx.syms, val(slots, *a), val(slots, *b));
                if *bind { slots[*out] = v } else if slots[*out] != v { return }
                go(cx, i + 1, slots);
            }
            Step::Split { whole, a, b, ka, kb } => {
                // As `cat` makes it: `"" . w`, or at each "." with a part before it.
                let w = cx.syms.name(val(slots, *whole)).to_owned();
                for p in std::iter::once(None).chain(w.match_indices('.').filter(|(p, _)| *p > 0).map(|(p, _)| Some(p))) {
                    let (x, y) = p.map_or(("", w.as_str()), |p| (&w[..p], &w[p + 1..]));
                    match a { Some(s) => slots[*s] = cx.syms.id(x), None => if cx.syms.name(val(slots, *ka)) != x { continue } }
                    match b { Some(s) => slots[*s] = cx.syms.id(y), None => if cx.syms.name(val(slots, *kb)) != y { continue } }
                    go(cx, i + 1, slots);
                }
            }
            Step::Eq { a, b } => if val(slots, *a) == val(slots, *b) { go(cx, i + 1, slots) },
            Step::Ne { a, b } => if val(slots, *a) != val(slots, *b) { go(cx, i + 1, slots) },
            Step::Func { name, kinds, args, out, bind } => {
                use crate::code_rule_dsl::logic::strings::functions::{eval, Kind};
                let value = {
                    let mut text = Vec::new();
                    let mut numbers = Vec::new();
                    for (k, a) in kinds.iter().zip(args) {
                        match k { Kind::Text => text.push(cx.syms.name(val(slots, *a))), Kind::Number => numbers.push(val(slots, *a)) }
                    }
                    let text: Vec<&str> = text.iter().map(String::as_str).collect();
                    eval(name, &text, &numbers)
                };
                let v = match value {
                    rusqlite::types::Value::Integer(n) => n,
                    rusqlite::types::Value::Text(s) => cx.syms.id(&s),
                    _ => return,
                };
                if *bind { slots[*out] = v } else if slots[*out] != v { return }
                go(cx, i + 1, slots);
            }
            Step::Doc { a, b, eq } => if ((val(slots, *a) >> 32) == (val(slots, *b) >> 32)) == *eq { go(cx, i + 1, slots) },
            Step::DocOf { out, n, bind } => {
                let d = val(slots, *n) >> 32;
                if *bind { slots[*out] = d } else if slots[*out] != d { return }
                go(cx, i + 1, slots);
            }
            Step::Test { op, s, p, matcher, want } => {
                let s = cx.syms.name(val(slots, *s));
                let hit = match matcher {
                    Some(m) => m.is_match(&s),
                    None => crate::code_rule_dsl::logic::strings::test(op, &s, &cx.syms.name(val(slots, *p))),
                };
                if hit == *want { go(cx, i + 1, slots) }
            }
            Step::Unused {..}=>go(cx,i+1,slots),
            Step::Let { a, b } => {
                let pending=if let Key::Slot(source)=b {cx.pending.iter().find(|(slot,_)|slot==source).map(|(_,v)|*v)} else {None};
                cx.pending.retain(|(slot,_)|slot!=a);
                if let Some(value)=pending {cx.pending.push((*a,value));}
                slots[*a] = val(slots, *b); go(cx, i + 1, slots)
            }
            Step::In { a, set, want } => if set.contains(&val(slots, *a)) == *want { go(cx, i + 1, slots) },
        }
    }
    let scratch = j.steps.iter().map(|_| rowcode::Scratch::default()).collect();
    let factor = factored(j);
    let failed = factor.iter().map(|_| Default::default()).collect();
    let mut cx = Ctx { j, head, store, syms, fresh, n: 0, key: Vec::new(), row: Vec::new(), error: None, pending: Vec::new(), scratch, collect, at0: 0, hints: vec![[0; runs::HINTS]; j.steps.len()],
                       factor, failed, skipped: 0, deep: 0 };
    let mut slots = vec![0i64; j.nslots];
    for &(s, v) in preset { slots[s] = v; }
    match (each, paired, j.steps.first(), j.steps.get(1)) {
        (Some((y, next)), ..) => loop {
            let out = cx.collect.as_deref_mut().expect("a join run for each value collects");
            let Some(v) = next(out) else { break };
            out.clear();
            slots[y] = v;
            go(&mut cx, 0, &mut slots);
            if cx.error.is_some() { break; }
        },
        // The first two steps' pairs from a level's batch: each left row binds the first atom's slots (its rows
        // passed its tests already), each right row is checked as the walk checks it and binds the second's.
        (None, Some(p), Some(Step::Atom { src: Src::In(r0, _), binds: b0, .. }), Some(Step::Atom { src: Src::In(r1, _), keys: k1, binds: b1, tests: t1, .. })) => {
            let (rows0, rows1) = (Rows::In(r0), Rows::In(r1));
            let c0 = &j.counts[0];
            c0[0].set(c0[0].get() + 1);
            let mut last = u32::MAX;
            for (li, rid) in p.pairs.iter() {
                if cx.error.is_some() { break; }
                if li != last {
                    last = li;
                    let id0 = p.lids[li as usize];
                    let row = rows0.row(id0);
                    for &(c, s) in b0 { slots[s] = row.at(c); }
                    cx.at0 = id0;
                    c0[1].set(c0[1].get() + 1);
                    let c1 = &j.counts[1][0];
                    c1.set(c1.get() + 1);
                }
                if rows1.dead(rid) { continue; }
                let row = rows1.row(rid);
                if !k1.iter().all(|&(c, k)| row.at(c) == val(&slots, k)) || !t1.iter().all(|&(c, c0)| row.at(c) == row.at(c0)) { continue; }
                for &(c, s) in b1 { slots[s] = row.at(c); }
                go(&mut cx, 2, &mut slots);
            }
        }
        _ => go(&mut cx, 0, &mut slots),
    }
    scalars::explain(&j.steps,store);
    // Each passed-over row was read and tested against a hash set, as a probe is.
    if cx.skipped > 0 { charge(Unit::JoinRow, cx.skipped); }
    match cx.error {Some(e)=>Err(e),None=>Ok(cx.n)}
}

/// The most slots a factored step's failed set is keyed by.
const FACTOR_MAX: usize = 4;

/// A factored step's key (`factored`): the `n` slots of `read`, which the steps before `upto` read of its bindings.
#[derive(Clone, Copy)]
struct Factor { read: [usize; FACTOR_MAX], n: usize, upto: usize }

/// Per step, for an atom (after the first, not fused) that binds slots: `upto`, the first later step that reads
/// every slot it binds (or the head, past the last step), and `read`, the slots it binds that the steps before
/// `upto` read, at most `FACTOR_MAX`. A row's rest of the body that made nothing and entered no step from `upto` on
/// looked only at `read` of the row's bindings (with the slots bound before): every row with the same values fails
/// alike and is passed over. So the order a rule was planned in costs a probe per distinct value of what the
/// failing steps read, not per row: `pfn`'s `pcall__ins($c, $o, $m)` by `$o`, then `fv` by `$m` (which rejected all
/// 733,692 rows), then `!pfn__old($c, …)` (`upto`, the first step to read `$c`).
fn factored(j: &JoinRule) -> Vec<Option<Factor>> {
    j.steps.iter().enumerate().map(|(i, st)| {
        let Step::Atom { neg: false, binds, .. } = st else { return None };
        if i == 0 || binds.is_empty() || j.fused.get(i).is_some_and(Option::is_some) { return None; }
        let reads = |t: &Step, s: usize| t.observes(s) || matches!(t, Step::Let { b: Key::Slot(x), .. } if *x == s);
        let mut bound: Vec<usize> = binds.iter().map(|&(_, s)| s).collect();
        bound.sort_unstable();
        bound.dedup();
        let mut seen: Vec<usize> = Vec::new();
        let mut upto = j.steps.len();
        for (t, step) in j.steps.iter().enumerate().skip(i + 1) {
            let now: Vec<usize> = bound.iter().copied().filter(|&s| !seen.contains(&s) && reads(step, s)).collect();
            if seen.len() + now.len() == bound.len() { upto = t; break; }
            seen.extend(now);
        }
        // Nothing before `upto` to fail in, or too wide a key.
        if upto <= i + 1 || seen.len() > FACTOR_MAX { return None; }
        let mut read = [0usize; FACTOR_MAX];
        read[..seen.len()].copy_from_slice(&seen);
        Some(Factor { read, n: seen.len(), upto })
    }).collect()
}

#[cfg(test)]
pub mod posted_tests {
    use super::*;

    /// Decision 0112's shape: a relation's indexes are sorted orders, only for the key sets its probes bind, the
    /// first holding every column as its dedup test; a wave's rows are one run (its delta); a maintained relation
    /// started from a kept one shares the kept one's runs rather than copying them; an order holds under 4.5 bytes
    /// a row (an id and its fences; the full-row order 6.5, with its tags).
    #[test]
    fn indexes_are_orders_shared_by_a_maintained_relation() {
        let mut rel = Rel::new("t", b"iii");
        // (0, 1) and (0) chain into the first order; (1) needs its own.
        rel.want(&[1, 0]);
        rel.want(&[0]);
        assert_eq!(rel.idx.len(), 1);
        rel.want(&[1]);
        assert_eq!(rel.idx.len(), 2);
        for w in 0..6i64 {
            let lo = rel.n;
            for i in 0..2000i64 {
                assert!(rel.push(&[w * 2000 + i, i % 97, (i * 31) % 1000]));
                // The dedup test: the same row again is not pushed, before or after its wave's runs are made.
                assert!(!rel.push(&[w * 2000 + i, i % 97, (i * 31) % 1000]));
            }
            let hi = rel.n;
            rel.set_delta(lo, hi);
            assert!(!rel.push(&[w * 2000 + 3, 3, 93]));
            // The delta reads this wave's rows only; all reads every wave's.
            let per = (0..2000).filter(|i| i % 97 == 5).count();
            assert_eq!(rel.probe(&[1], &[5], true).1, per);
            assert_eq!(rel.probe(&[1], &[5], false).1, per * (w as usize + 1));
            assert_eq!(rel.probe(&[0, 1], &[w * 2000 + 5, 5], false).1, 1);
        }
        let old = InRel::moved(rel);
        assert!(old.chains.iter().all(|ix| ix.runs() <= FROZEN_RUNS), "a frozen order holds a few runs");
        for ix in &old.chains {
            // The full-row order keeps a 16-bit tag an id instead of fences (`runs::tagged`).
            let most = if runs::tagged(&ix.cols, old.k) { 6.5 } else { 4.5 };
            assert!((ix.bytes() as f64) < most * old.n as f64, "{:?}: {} bytes a row", ix.cols, ix.bytes() as f64 / old.n as f64);
        }
        let based = Rel::based("t", b"iii", &old, None).expect("a kept relation to start from");
        for (ix, kept) in based.idx.iter().zip(&old.chains) {
            assert!(ix.shares(kept), "the maintained relation's order {:?} is a copy", ix.cols);
        }
        assert!(based.contains(&[3, 3, 93]) && !based.contains(&[3, 3, 94]));
    }

    /// The dedup test a wave at a time (decision 0112): a wave's derived rows, in any order, repeated, and some
    /// held already (one of them dead), become the wave's new rows once each, whether they are checked at the wave's
    /// end or in batches as they come (`FRESH_BATCH`); the relation holds what a set of its rows would.
    #[test]
    fn a_waves_rows_are_checked_once_as_a_sorted_batch() {
        let mut x: u64 = 11;
        let mut next = |m: u64| {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((x >> 33) % m) as i64
        };
        let mut rel = Rel::new("t", b"iii");
        rel.want(&[2]);
        let mut set: std::collections::BTreeSet<Vec<i64>> = Default::default();
        for w in 0..5 {
            let mut f = Fresh::default();
            // Few values a column: many repeats in the wave and many rows held from earlier waves.
            let rows = if w == 3 { 3 * FRESH_BATCH } else { 5000 };
            let mut new = std::collections::BTreeSet::new();
            for _ in 0..rows {
                let row = vec![next(40), next(30), next(20)];
                f.offer(&row, &rel);
                if !set.contains(&row) {
                    new.insert(row);
                }
            }
            assert!(w != 3 || f.n < rows, "a large wave is checked in batches as it comes");
            let lo = rel.n;
            assert_eq!(rel.push_fresh(f, false), !new.is_empty());
            let hi = rel.n;
            assert_eq!(hi - lo, new.len(), "wave {w}: each new row once");
            rel.set_delta(lo, hi);
            set.extend(new);
            if w == 1 {
                // A row deleted in place is new again when derived again.
                let id = (0..rel.n as u32).find(|&id| !rel.dead(id)).expect("a row");
                assert!(kill(&mut rel.dead, id));
                rel.ndead += 1;
                set.remove(rel.row(id as usize));
            }
        }
        let live: std::collections::BTreeSet<Vec<i64>> = (0..rel.n).filter(|&id| !rel.dead(id as u32)).map(|id| rel.row(id).to_vec()).collect();
        assert_eq!(live, set);
        assert_eq!((0..rel.n).filter(|&id| !rel.dead(id as u32)).count(), set.len(), "no live row twice");
    }

    /// A column's sorted index is the same whichever way it is built (in order, few values, many): its rows sorted
    /// stably by value.
    #[test]
    fn a_sorted_index_is_its_rows_sorted_stably_by_value() {
        let mut x: u64 = 7;
        let mut next = |m: u64| {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((x >> 33) % m) as i64
        };
        let columns: Vec<Vec<i64>> = vec![
            vec![],
            vec![5],
            (0..100).map(|i| i / 3).collect(),
            (0..1000).map(|_| next(7) - 3).collect(),
            (0..1000).map(|_| next(1 << 40)).collect(),
            (0..50).map(|_| next(2)).collect(),
        ];
        for col in columns {
            let got = Sorted::of(&col);
            let mut ids: Vec<u32> = (0..col.len() as u32).collect();
            ids.sort_by_key(|&r| col[r as usize]);
            assert_eq!(got.ids, ids, "{col:?}");
            let mut keys = col.clone();
            keys.sort_unstable();
            keys.dedup();
            assert_eq!(got.keys, keys);
            assert_eq!(got.starts.len(), keys.len() + 1);
            for (i, k) in keys.iter().enumerate() {
                assert!(got.ids[got.starts[i] as usize..got.starts[i + 1] as usize].iter().all(|&r| col[r as usize] == *k));
            }
            assert_eq!(*got.starts.last().unwrap() as usize, col.len());
        }
    }

    /// Statistics read from a kept relation's postings are those measured from its rows, with dead rows left out.
    #[test]
    fn statistics_from_postings_are_the_measured_ones() {
        let node = |d: i64, i: i64| (d << 32) | i;
        let mut rel = Rel::new("t", b"iii");
        // Each column's statistics are read from an order leading with it.
        for c in 0..3 { rel.want(&[c]); }
        // Column 0 a node (a key), column 1 a value shared within and across documents, column 2 another document's
        // node (as a reference) or a small number.
        for d in 1..4 {
            for i in 0..20 {
                rel.push(&[node(d, i), i % 3, if i % 2 == 0 { node(4 - d, i % 5) } else { i % 4 }]);
            }
        }
        let old = InRel::moved(rel);
        let mut gone = Rel::new("g", b"iii");
        for (d, i) in [(1, 0), (1, 1), (2, 3), (3, 9), (3, 12)] {
            gone.push(&[node(d, i), i % 3, if i % 2 == 0 { node(4 - d, i % 5) } else { i % 4 }]);
        }
        let gone = InRel::moved(gone);
        for rel in [old.clone_rows(), InRel::moved(Rel::based("t", b"iii", &old, Some(&gone)).expect("few rows dead"))] {
            let rel = Arc::new(rel);
            let source = Derived::new(Arc::clone(&rel), None);
            let measured: Vec<[i64; 4]> = crate::code_rule_dsl::facts::vtab::syntax::measured(source.as_ref()).into_iter().map(|(_, a, b, c, d)| [a, b, c, d]).collect();
            assert_eq!(rel.posted().expect("a relation with its postings"), measured);
        }
    }

    /// Statistics counted from a relation's postings are those counted from its live rows.
    #[test]
    fn statistics_counted_from_postings_are_those_of_the_rows() {
        use crate::code_rule_dsl::logic::plan::statkeep::RelStats;
        let node = |d: i64, i: i64| (d << 32) | i;
        let mut rel = Rel::new("t", b"iiii");
        // Each column's postings are read from an order leading with it.
        for c in 0..4 { rel.want(&[c]); }
        for i in 0..900 {
            // Documents interleaved; a first column that is sometimes a document key; a few-valued column; a value
            // column past the exact map's size that crosses documents; a reference to another document's node.
            let d = 1 + (i * 7) % 5;
            let first = if i % 11 == 0 { d } else { node(d, i) };
            rel.push(&[first, i % 4, (i * 37) % 300, if i % 3 == 0 { node(6 - d, i % 9) } else { node(d, i % 13) }]);
        }
        let old = InRel::moved(rel);
        let mut gone = Rel::new("g", b"iiii");
        for id in [3usize, 40, 41, 300] {
            let r = old.row(id).to_vec();
            gone.push(&r);
        }
        let rel = InRel::moved(Rel::based("t", b"iiii", &old, Some(&InRel::moved(gone))).expect("few dead"));
        let names: Vec<String> = (0..4).map(|c| format!("c{c}")).collect();
        let live = rel.live_rows().expect("flat");
        let mut want = RelStats::paired(names.clone());
        want.documents(&live, 4, 1);
        let got = RelStats::from_postings(names, rel.rows() as i64, &mut |c, visit| rel.each_posting(c, visit)).expect("postings");
        assert_eq!(got, want);
        assert_eq!(got.statistics(&|_| true), want.statistics(&|_| true));
    }

    /// A maintained relation's delta from the relation it started as: the rows it marked dead, and those it added
    /// (a row deleted and derived again in both); none from a relation it did not start as.
    #[test]
    fn a_maintained_relation_gives_its_delta_from_the_one_it_started_as() {
        let mut rel = Rel::new("t", b"ii");
        for i in 0..40 {
            rel.push(&[i, i % 3]);
        }
        let old = Arc::new(InRel::moved(rel));
        let mut gone = Rel::new("g", b"ii");
        for i in [2, 5, 30] {
            gone.push(&[i, i % 3]);
        }
        let gone = InRel::moved(gone);
        let mut new = Rel::based("t", b"ii", &old, Some(&gone)).expect("few rows dead");
        new.push(&[5, 2]);
        new.push(&[100, 1]);
        let new = InRel::moved(new);
        let (mut added, mut removed) = new.delta_since(&old).expect("started as old");
        let rows = |flat: &mut Vec<i64>| {
            let mut r: Vec<Vec<i64>> = flat.chunks_exact(2).map(<[i64]>::to_vec).collect();
            r.sort();
            r
        };
        assert_eq!(rows(&mut added), vec![vec![5, 2], vec![100, 1]]);
        assert_eq!(rows(&mut removed), vec![vec![2, 2], vec![5, 2], vec![30, 0]]);
        assert_eq!(new.live_rows().expect("flat").len(), (40 - 3 + 2) * 2);
        assert!(new.delta_since(&gone).is_none() && old.delta_since(&old) == Some((Vec::new(), Vec::new())));
        // Too many dead to keep in place: made from the survivors, the same delta.
        let mut most = Rel::new("g", b"ii");
        for i in 0..20 {
            most.push(&[i, i % 3]);
        }
        let most = Arc::new(InRel::moved(most));
        assert!(Rel::based("t", b"ii", &old, Some(&most)).is_none());
        let mut new = Rel::compacted("t", b"ii", &old, Some(&most));
        new.push(&[3, 0]);
        new.push(&[100, 1]);
        let new = InRel::moved(new);
        let (mut added, mut removed) = new.delta_since(&old).expect("started as old");
        assert_eq!(rows(&mut added), vec![vec![3, 0], vec![100, 1]]);
        assert_eq!(rows(&mut removed), (0..20).map(|i| vec![i, i % 3]).collect::<Vec<_>>());
    }

    /// Every row a component pushes has its level (decision 0090), which a kept relation counts as 4 bytes a row and
    /// keeps through `based` and compaction.
    #[test]
    fn a_kept_relation_keeps_and_counts_its_levels() {
        let mut rel = Rel::new("t", b"ii");
        for i in 0..40 {
            rel.push(&[i, i % 3]);
        }
        let old = Arc::new(InRel::moved(rel));
        assert!(old.has_levels());
        assert_eq!(old.bytes(), 40 * 2 * 8 + 40 * 4);
        let mut most = Rel::new("g", b"ii");
        for i in 0..20 {
            most.push(&[i, i % 3]);
        }
        let most = InRel::moved(most);
        let new = InRel::moved(Rel::compacted("t", b"ii", &old, Some(&most)));
        assert!(new.has_levels());
        let kept: Vec<u32> = (20..40).map(|i| old.level[i]).collect();
        assert_eq!(new.level, kept);
        let same = InRel::moved(Rel::based("t", b"ii", &old, None).expect("nothing gone"));
        assert_eq!(same.level, old.level);
    }

    /// Rows held as columns measure from their sorted indexes what `syntax::measured` gives, a column held nowhere
    /// as a key; and they keep them.
    #[test]
    fn held_columns_measure_from_their_indexes() {
        let node = |d: i64, i: i64| (d << 32) | i;
        let (mut a, mut b, mut c) = (Vec::new(), Vec::new(), Vec::new());
        for i in 0..60 {
            // Documents out of row order, values shared within and across them.
            let d = [2, 1, 3][i as usize % 3];
            a.push(node(d, i));
            b.push(i % 7);
            c.push(if i % 2 == 0 { node(1, i % 5) } else { 3 });
        }
        let names: Vec<String> = ["a", "b", "c", "x"].iter().map(|s| (*s).to_owned()).collect();
        let held = columns_source(names.clone(), vec![Some(a.clone()), Some(b.clone()), Some(c.clone()), None]);
        let plain = columns_source(names[..3].to_vec(), vec![Some(a), Some(b), Some(c)]);
        let mut want = crate::code_rule_dsl::facts::vtab::syntax::measured(&Unknown(plain));
        want.push(("x".to_owned(), 60, 60, 60, 60));
        assert_eq!(held.statistics().expect("held rows keep statistics"), want);
    }

    #[test]
    fn omitted_columns_obey_zero_semantics_in_indexed_probes() {
        for values in [vec![3], vec![3, 1, 3], vec![]] {
            let source = columns_source(vec!["key".into(), "omitted".into()], vec![Some(values), None]);
            for equal in [vec![], vec![(1, 0)], vec![(1, 7)], vec![(0, 3), (1, 0)], vec![(0, 3), (1, 7)]] {
                let want: Vec<usize> = source.filter_eq(&equal).into_iter().map(|r| r as usize).collect();
                assert_eq!(source.lookup_all(&equal), Some(want), "{equal:?}");
            }
            assert_eq!(source.lookup(1, &[0]), Some((0..source.len()).collect()));
            assert_eq!(source.lookup(1, &[7]), Some(Vec::new()));
            assert!(!source.sorts_rows(1));
            assert_eq!(source.sorted_rows(1, 0), None);
        }
    }

    /// A source with no statistics of its own, so `measured` measures it.
    struct Unknown(Arc<dyn crate::code_rule_dsl::facts::vtab::Source>);
    impl crate::code_rule_dsl::facts::vtab::Source for Unknown {
        fn columns(&self) -> Vec<String> { self.0.columns() }
        fn len(&self) -> usize { self.0.len() }
        fn estimated_matches(&self, c: usize, batch: bool) -> usize { self.0.estimated_matches(c, batch) }
        fn indexed(&self, _: usize) -> bool { false }
        fn lookup(&self, _: usize, _: &[i64]) -> Option<Vec<usize>> { None }
        fn value(&self, row: usize, c: usize) -> i64 { self.0.value(row, c) }
    }

    impl InRel {
        fn clone_rows(&self) -> InRel {
            InRel { k: self.k, n: self.n, rows: self.rows.clone(), view: None, base: false, types: self.types.clone(), stored: self.stored, idx: Mutex::new(HashMap::default()), chains: self.chains.clone(),
                    sketches: self.sketches.clone(), serves: self.serves.clone(), dead: self.dead.clone(), ndead: self.ndead, summaries: Default::default(), measured: Default::default(), counted: Default::default(), tables: Default::default(), name: Default::default(), from: None, uid: Default::default(), level: self.level.clone(), stats: self.stats.clone() }
        }
    }
}

#[cfg(test)]
pub mod sorted_tests {
    use super::Sorted;

    /// What the sort gives: (value, row) pairs in order.
    fn by_sort(col: &[i64]) -> (Vec<u32>, Vec<i64>, Vec<u32>) {
        let mut pairs: Vec<(i64, u32)> = col.iter().enumerate().map(|(r, &v)| (v, r as u32)).collect();
        pairs.sort_unstable();
        let (mut keys, mut starts) = (Vec::new(), Vec::new());
        for (i, &(v, _)) in pairs.iter().enumerate() {
            if keys.last() != Some(&v) {
                keys.push(v);
                starts.push(i as u32);
            }
        }
        starts.push(pairs.len() as u32);
        (pairs.into_iter().map(|(_, r)| r).collect(), keys, starts)
    }

    #[test]
    fn a_column_in_order_is_indexed_as_the_sort_would() {
        for col in [vec![], vec![5], vec![1, 1, 2, 3, 3, 3, 9], vec![-4, 0, 0, 7], vec![3, 1, 2, 2, 0]] {
            let s = Sorted::of(&col);
            assert_eq!((s.ids, s.keys, s.starts), by_sort(&col), "{col:?}");
        }
    }
}

#[cfg(test)]
pub mod carry_sorted_tests {
    use super::*;

    /// A document's rows: node ids `doc << 32 | i`, a kind of few values, a value of many, and a column no rule reads.
    fn rows(docs: &[(i64, usize, i64)]) -> Vec<Option<Vec<i64>>> {
        let (mut id, mut kind, mut val) = (Vec::new(), Vec::new(), Vec::new());
        for &(d, n, salt) in docs {
            for i in 0..n as i64 {
                id.push(d << 32 | i);
                kind.push((i * 7 + salt) % 3);
                val.push(((i * 7919 + salt * 104_729) % 1_000_003) - 500_000);
            }
        }
        let m = id.len();
        vec![Some(id), Some(kind), Some(val), None.filter(|_: &Vec<i64>| m == 0)]
    }

    fn stored(values: Vec<Option<Vec<i64>>>) -> Arc<dyn crate::code_rule_dsl::facts::vtab::Source> {
        columns_source(["id", "kind", "val", "unread"].map(str::to_owned).to_vec(), values)
    }

    fn of(s: &Arc<dyn crate::code_rule_dsl::facts::vtab::Source>) -> &Stored {
        s.as_any().and_then(|a| a.downcast_ref::<Stored>()).expect("stored")
    }

    /// Each index carried is the one built over the new rows: a document gone, one changed, one added.
    #[test]
    fn carried_indexes_are_those_built_again() {
        let old = stored(rows(&[(1, 1000, 0), (2, 300, 1), (3, 300, 2), (4, 1000, 3)]));
        for c in 0..3 {
            let _ = of(&old).sorted(c);
        }
        let _ = of(&old).order(&[1, 2]);
        let docs = [(1, 1000, 0), (3, 290, 5), (4, 1000, 3), (6, 40, 9)];
        let new = stored(rows(&docs));
        let (laid, n) = carry_sorted_at(old.as_ref(), new.as_ref(), 1_000);
        assert!(laid.is_none(), "the runs kept are in order: new's rows as they are");
        assert_eq!(n, 3, "the id column is in row order: read in one pass, not carried");
        let again = stored(rows(&docs));
        same_indexes(of(&new), of(&again));
    }

    /// The runs kept in another order (the files parsed this run first): laid out in their old order, the rest
    /// after, the same rows, and each index the one built over them.
    #[test]
    fn runs_out_of_order_are_laid_out_again() {
        let old = stored(rows(&[(1, 1000, 0), (2, 300, 1), (3, 1000, 2), (4, 1000, 3)]));
        for c in 0..3 {
            let _ = of(&old).sorted(c);
        }
        let _ = of(&old).order(&[1, 2]);
        let new = stored(rows(&[(2, 310, 6), (4, 1000, 3), (1, 1000, 0), (3, 1000, 2)]));
        let (laid, n) = carry_sorted_at(old.as_ref(), new.as_ref(), 1_000);
        let laid = laid.expect("laid out again");
        assert_eq!(n, 4);
        let cols = |t: &Stored| (0..3).map(|c| t.col(c).to_vec()).collect::<Vec<_>>();
        let rows_of = |t: &Stored| {
            let c = cols(t);
            let mut r: Vec<[i64; 3]> = (0..c[0].len()).map(|i| [c[0][i], c[1][i], c[2][i]]).collect();
            r.sort_unstable();
            r
        };
        assert_eq!(rows_of(of(&laid)), rows_of(of(&new)));
        assert_eq!(of(&laid).col(0)[..1000], of(&old).col(0)[..1000], "the first run kept stays first");
        let again = stored(cols(of(&laid)).into_iter().map(Some).chain([None]).collect());
        same_indexes(of(&laid), of(&again));
    }

    fn same_indexes(carried: &Stored, built: &Stored) {
        for c in 0..3 {
            let (a, b) = (carried.sorted(c), built.sorted(c));
            assert_eq!((&a.ids, &a.keys, &a.starts), (&b.ids, &b.keys, &b.starts), "column {c}");
        }
        assert_eq!(*carried.order(&[1, 2]).1, *built.order(&[1, 2]).1);
    }

    /// Nothing is carried when most rows changed.
    #[test]
    fn carries_nothing_when_most_rows_changed() {
        let old = stored(rows(&[(1, 1000, 0), (2, 1000, 1), (3, 1000, 2)]));
        let _ = of(&old).sorted(2);
        let most = stored(rows(&[(1, 1000, 0), (2, 1000, 7), (3, 1000, 8)]));
        let (laid, n) = carry_sorted_at(old.as_ref(), most.as_ref(), 1_000);
        assert!(laid.is_none() && n == 0);
    }
}
