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
pub mod keys;
mod held;
use runs::{Index, RowData};
mod rows;
pub use rows::{Facts, InBlock, Row, in_chunk};
use rows::{Rows, Shared};
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
use std::sync::atomic::{AtomicU64 as OrderCount, Ordering as AtomicOrdering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock, RwLock};
use std::time::Instant;

/// Strata captured and run again on their own (decision 0095, step 0).
pub mod lab;
pub mod spec;
#[cfg(test)]
mod survivor_tests;
#[cfg(test)]
mod maintain_mode_tests;
#[cfg(test)]
mod cut_tests;
#[cfg(test)]
mod view_tests;
#[cfg(test)]
mod base_probe;
mod inputs_tests;
#[cfg(all(test, target_os = "macos"))]
mod blocks_compare_tests;
use spec::{Arg, ComponentSpec, InputKey, JoinItem, JoinSpec, Line, Reprobe, Source, Start, Tok};
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
pub struct Interner { pub atoms: Arc<crate::content_store::atoms::Atoms>, buf: String, failed: Option<String>,
                       /// A morsel's texts not yet atoms (`Deferred`): none is interned while it walks.
                       deferred: Option<Box<Deferred>> }

/// The texts a morsel of a walk made that are not atoms yet, each under a value of its own from `DEFERRED` on, in the
/// order the morsel first made them. The walk's merge interns every morsel's in the morsels' order, which is the
/// order one walk would have made them in, so each gets the atom one walk would have given it, and the morsels'
/// rows are read with those atoms in place of these values (`morsels`). The morsels then intern nothing as they
/// walk: no atom's id follows the threads' timing, and no morsel takes the arena's every shard to write.
#[derive(Default)]
struct Deferred { texts: Vec<String>, at: HashMap<String, u32> }


/// The first value of a text a morsel deferred (`Deferred`): past every atom (`Sym` is 32 bits) and below every node
/// (whose document is in the high 32 bits, from 0x6000… on).
const DEFERRED: i64 = 1 << 40;

/// A database's shared symbols: its interner, behind the lock that `sym_atom`, `sym_cat` and `cat_symbols`
/// write under, with its atom arena beside the lock. The arena is the interner's own and never changes, so a
/// loader's cursor (`Interner::for_loading`, at every component's open) reads it without the lock: it waited there
/// behind the SQL functions' writes, 2,004 of 2,204 samples parked (Q203 item 3, ab71).
pub struct Symbols { atoms: Arc<crate::content_store::atoms::Atoms>, cursor: RwLock<Interner> }

impl Symbols {
    fn new(atoms: Arc<crate::content_store::atoms::Atoms>) -> Symbols {
        Symbols { atoms: Arc::clone(&atoms), cursor: RwLock::new(Interner { atoms, buf: String::new(), failed: None, deferred: None }) }
    }
    /// The atom arena the symbols are in, read without the lock.
    pub fn atoms(&self) -> &Arc<crate::content_store::atoms::Atoms> { &self.atoms }
}

impl std::ops::Deref for Symbols {
    type Target = RwLock<Interner>;
    fn deref(&self) -> &RwLock<Interner> { &self.cursor }
}

impl Interner {
    /// A loader's scratch cursor over the same atom arena. Loading SQL may call sym_text/sym_atom,
    /// so it must not hold the registered cursor's lock across SQLite execution; nor does it take it.
    fn for_loading(shared: &Symbols) -> Self {
        Self { atoms: Arc::clone(shared.atoms()), buf: String::new(), failed: None, deferred: None }
    }
    /// The text `v` stands for if it is a text this morsel deferred.
    fn deferred_text(&self, v: i64) -> Option<&str> {
        let d = self.deferred.as_ref()?;
        let at = usize::try_from(v.checked_sub(DEFERRED)?).ok()?;
        d.texts.get(at).map(String::as_str)
    }
    /// The atom `id` would give `s`, if it would not make one (`Atoms::existing`): found as interning finds it, with
    /// no read interning would not make. In a morsel, a text it deferred is its deferred value.
    fn held(&self, s: &str) -> Option<i64> {
        self.atoms.existing(s).map(i64::from).or_else(|| Some(DEFERRED + i64::from(*self.deferred.as_ref()?.at.get(s)?)))
    }
    fn id(&mut self, s: &str) -> i64 {
        if self.deferred.is_some() {
            if let Some(v) = self.held(s) {
                return v;
            }
            let d = self.deferred.as_mut().expect("deferring");
            let at = d.texts.len() as u32;
            d.texts.push(s.to_owned());
            d.at.insert(s.to_owned(), at);
            return DEFERRED + i64::from(at);
        }
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
    fn get(&self, s: &str) -> Option<i64> { self.atoms.get(s).map(i64::from).or_else(|| Some(DEFERRED + i64::from(*self.deferred.as_ref()?.at.get(s)?))) }
    /// Atom `v`'s text, reading the database's atoms written since `symbols` when `v` is past the arena; "" if there
    /// is no such atom or they could not be read (a join step has no error to return: it matches as it did before).
    pub fn name(&self, v: i64) -> String {
        if let Some(t) = self.deferred_text(v) {
            return t.to_owned();
        }
        let Ok(atom) = crate::content_store::atoms::Sym::try_from(v) else { return String::new() };
        self.atoms.with_str(atom, str::to_owned).or_else(|| self.atoms.value(atom).ok().flatten()).unwrap_or_default()
    }
    /// Whether `v` is an atom of the database, read as `name` reads it.
    fn known(&self, v: i64) -> Result<bool> {
        if self.deferred_text(v).is_some() {
            return Ok(true);
        }
        let Ok(atom) = crate::content_store::atoms::Sym::try_from(v) else { return Ok(false) };
        Ok(self.atoms.with_str(atom, |_| ()).is_some() || self.atoms.value(atom).map_err(|e| err(e.to_string()))?.is_some())
    }
    /// The first interning that failed since the last check.
    fn check(&mut self) -> Result<()> {
        match self.failed.take() { Some(e) => Err(err(e)), None => Ok(()) }
    }
}

/// The shared object's symbols of each database (decision 0442), kept from run to run with their arena: a run need not read a large
/// store's atoms again. They hold no file open: the store's last handle closes them (`ContentStore`'s drop).
/// `RERAIL_LEVELS_ON_CYCLES`: a leveled component keeps levels only on its relations on a cycle (`open`).
#[derive(Default)]
struct DatabaseSymbols(Mutex<HashMap<String, Arc<Symbols>>>);
/// The symbols of `conn`'s database: the shared object's arena of it.
pub fn symbols(conn: impl Scoped) -> Result<Arc<Symbols>> {
    let conn = conn.db();
    // A connection's symbols are found once (by its path) and kept for it until it closes (`forget_scope`).
    let on = on(&conn);
    if let Some(s) = on.syms.lock().as_ref() {
        return Ok(Arc::clone(s));
    }
    let s = symbols_by_path(conn)?;
    forgotten_on_close();
    *on.syms.lock() = Some(Arc::clone(&s));
    Ok(s)
}
/// The path of `conn`'s main database (`""` in memory), found once per connection and kept until it closes
/// (`forget_scope`): SQLite's `sqlite3_db_filename` takes the connection's mutex at every call.
fn conn_path(conn: impl Scoped) -> Arc<str> {
    let conn = conn.db();
    let on = on(&conn);
    if let Some(p) = on.path.lock().as_ref() {
        return Arc::clone(p);
    }
    forgotten_on_close();
    // Off the run's thread (a strand, decision 0113) the path is not known unless the run's thread found it
    // (`prime`): not kept, so the run's own lookup finds it.
    let Ok(sql) = conn.sql() else { return Arc::from("") };
    let p: Arc<str> = Arc::from(sql.path().unwrap_or_default());
    *on.path.lock() = Some(Arc::clone(&p));
    p
}

/// What a stratum off the run's thread reads of its connection, found here first (decision 0113): its path and its
/// symbols, kept by its scope.
pub fn prime(conn: &Connection) -> Result<Arc<crate::content_store::atoms::Atoms>> {
    conn_path(conn);
    let syms = symbols(conn)?;
    Ok(Arc::clone(syms.atoms()))
}
/// What the evaluator keeps for one connection (decision 0442): on the connection itself (`vtab::ConnData`), dropped
/// as it closes, or (its relations and components) as a run reuses it (`forget_engine`). Two connections, on any
/// threads, never reach each other's.
#[derive(Default)]
pub struct OnConn {
    /// Its main database's path (`conn_path`): SQLite's `sqlite3_db_filename` takes the connection's mutex.
    path: parking_lot::Mutex<Option<Arc<str>>>,
    /// Its symbols (`symbols`).
    syms: parking_lot::Mutex<Option<Arc<Symbols>>>,
    /// Its open components, by name.
    components: parking_lot::Mutex<HashMap<String, Arc<Mutex<Component>>>>,
    /// The relations of components run with `nowrite`, by table, until they are materialized.
    derived: parking_lot::Mutex<HashMap<String, Arc<InRel>>>,
    /// The relations `adopt`ed into its next run.
    adopted: parking_lot::Mutex<std::collections::BTreeSet<String>>,
    /// Input relations loaded for joins, by spec, for the run that loaded them (`INPUTS` before: see `forget_inputs`).
    inputs: parking_lot::Mutex<HashMap<InputKey, Arc<InRel>>>,
    /// Relations whose rows change between its native runs (`keep_inputs`).
    changing: parking_lot::Mutex<Option<std::collections::BTreeSet<String>>>,
    /// The given versions of a run's changed reads (`give_versions`), by table: `Old`, `Minus`, `Plus`, `New`.
    given: parking_lot::Mutex<HashMap<String, [Arc<InRel>; 4]>>,
}

/// What the evaluator keeps for `conn` (`OnConn`).
fn on(conn: impl Scoped) -> Arc<OnConn> {
    conn.db().data().get::<OnConn>()
}
fn symbols_by_path(conn: impl Scoped) -> Result<Arc<Symbols>> {
    let conn = conn.db();
    let key = conn_path(conn).to_string();
    let held_ = crate::code_rule_dsl::logic::profile::shared::<DatabaseSymbols>();
    let mut syms = held_.0.lock().unwrap();
    let atoms = crate::content_store::atoms::Atoms::for_connection(conn.sql()?);
    // A store created again at the same path has new atoms (`Atoms::for_database`, checked by `Atoms::attach`).
    if let Some(s) = syms.get(&key) && Arc::ptr_eq(s.atoms(), &atoms) {
        return Ok(Arc::clone(s));
    }
    // An arena made after the store was written holds none of its atoms yet: the run's SQL and joins read them.
    atoms.read_stored().map_err(|e| err(e.to_string()))?;
    let s = Arc::new(Symbols::new(atoms));
    syms.insert(key, Arc::clone(&s));
    Ok(s)
}

/// The symbol of `text` in `conn`'s symbols, where it has one (a rule's text constant as its column holds it).
pub fn symbol_id(conn: impl Scoped, text: &str) -> Result<Option<i64>> {
    Ok(symbols(conn)?.read().unwrap().get(text))
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
/// The texts of `atoms`, read under one lock of `conn`'s symbols.
pub fn symbol_texts(conn: &Connection, atoms: &[i64]) -> Result<Vec<String>> {
    let symbols = symbols(conn)?;
    let s = symbols.read().unwrap();
    atoms.iter().map(|&a| if s.known(a)? { Ok(s.name(a).to_owned()) } else { Err(err(format!("no symbol {a}"))) }).collect()
}

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
    /// The run's probes (decision 0442).
    #[derive(Default)]
    struct Probes(std::sync::Mutex<HashMap<(String, Vec<usize>), u64>>);
    #[inline]
    pub(crate) fn on() -> bool {
        rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var_os("RERAIL_INDEX_REPORT").is_some())
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
        crate::code_rule_dsl::logic::run::with_local(|p: &Probes| *p.0.lock().unwrap_or_else(|e| e.into_inner()).entry((rel.to_owned(), cols)).or_insert(0) += 1);
    }
    /// The probes since the last `take`.
    pub(crate) fn take() -> HashMap<(String, Vec<usize>), u64> {
        crate::code_rule_dsl::logic::run::local_if_any::<Probes>().map(|p| std::mem::take(&mut *p.0.lock().unwrap_or_else(|e| e.into_inner()))).unwrap_or_default()
    }
}

/// The rows a reader of a relation's live rows takes at a time (`InRel::each_live_batch`).
pub const LIVE_BATCH: usize = 4096;

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
pub struct Rel { table: String, types: Vec<u8>, pub k: usize, pub n: usize, data: Rows, stored: usize, lo: usize, hi: usize,
             wave: u32, idx: Vec<Index>, serves: Vec<Vec<Vec<usize>>>, indexed: usize, pending: keys::RowSet, sketches: Vec<Sketch>,
             dead: Vec<u64>, ndead: usize, summaries: scalars::Pool,
             /// Its row ranges known to be in an order already (decision 0180), recorded as its waves write them.
             sorted: runs::Sorted,
             /// The kept relation it started as (`InRel::delta_since`).
             from: Option<Origin>,
             /// Each row's level (decision 0090: `lbase` plus the wave that pushed it), while every row has one.
             level: Vec<u32>, lbase: u32,
             /// Whether its rows get levels: only a relation of a recursive stratum reads them (`open`), and at 4
             /// bytes a row they are not held for the others.
             leveled: bool,
             /// Its statistics kept as its rows come (`kept_stats`), the planner's form of them kept by delta
             /// (decision 0096): rows `..indexed` counted in, and those `stats_dead` marks counted out.
             stats: Option<KeptStats>, stats_dead: Vec<u64>,
             /// A lattice relation's key and join (`lattice` lines): one live row a key, its value the join of all the
             /// key's values derived (`Lat::fold`). `None`: a set.
             lat: Option<Box<Lat>>,
             /// The maintain mode's level cap (decision 0364): while a recursive candidate is checked, a row of this
             /// level or above is no row (`dead`), as a row present only below the candidate's level supports it.
             /// `u32::MAX`: none.
             cap: u32,
             /// Declared sorted (`.decl … sorted`, decision 0456 K7): its live rows laid out in canonical order (its
             /// columns in turn) when its component is done (`InRel::moved`).
             ordered: bool }

/// A lattice relation's state (decision 0124): its last column is the value, the columns before it the key. A key's
/// row is replaced when a wave's values for it join to a new value: the old row dead, the new one the next wave's
/// delta, so readers and later probes see the folded value only. As the SQL kernel folds (`schedule::lattice::fold`):
/// absence is bottom, the top absorbs, and after `widen` changes of a key its next value is the lattice's widening.
pub struct Lat {
    /// The lattice's name (`front::LatticeDecl::lattice`).
    name: String,
    keys: usize,
    lattice: Arc<dyn super::Lattice>,
    widen: Option<u32>,
    /// Each key's live row, its changes so far (its first value counts one) and its round: an index of the relation in
    /// its block, the one the device folds through too (decision 0384 step 5).
    map: keys::KeyMap,
}

impl Lat {
    /// The rows `rows` (row-major, `k` columns) folded by key, in first-seen key order: (key, the join of its values).
    fn group(&self, rows: &[i64], k: usize) -> Vec<(Box<[i64]>, i64)> {
        let mut order: Vec<(Box<[i64]>, i64)> = Vec::new();
        let mut slot: HashMap<&[i64], usize> = HashMap::default();
        for row in rows.chunks_exact(k) {
            let (key, v) = (&row[..self.keys], row[self.keys]);
            match slot.get(key) {
                Some(&i) => order[i].1 = self.lattice.join(order[i].1, v),
                None => {
                    slot.insert(key, order.len());
                    order.push((key.into(), v));
                }
            }
        }
        order
    }
}

/// The id of a live row equal to `row` in the relation of `orders` (its first holds every column), not `skip`.
fn find_row<D: RowData + ?Sized>(orders: &[Index], data: &D, k: usize, row: &[i64], skip: impl Fn(u32) -> bool) -> Option<u32> {
    let o = orders.first()?;
    let mut key = [0i64; MAX_KEY];
    let key = permuted(&o.cols, |c| row[c], &mut key);
    o.find(data, k, key, None).ids().find(|&id| !skip(id) && data.row(id, k) == row)
}

/// The widest key an order is probed with inline; a relation of more columns is not indexed (`Rel::new`).
const MAX_KEY: usize = 64;

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
    /// `dead`: the old relation's dead rows, as it started (its version at the run's start, decision 0364 stage 2:
    /// what `old`, `minus` and `plus` are read from, with no other relation kept).
    Based { at: usize, n: usize, dead: Arc<Vec<u64>> },
    /// Made from the old relation's survivors (too many dead to keep in place): its first `kept` rows, then those
    /// it added; `removed`, the old rows it left out, row-major.
    Compacted { at: usize, old_n: usize, kept: usize, removed: Vec<i64> },
}
impl Rel {
    fn new(table: &str, types: &[u8]) -> Rel {
        let k = types.len();
        assert!(k <= MAX_KEY, "{table}: {k} columns, past the {MAX_KEY} an order is probed with");
        Rel { table: table.to_string(), types: types.to_vec(), k, n: 0, data: Rows::new(k), stored: 0, lo: 0, hi: 0, wave: 0,
              idx: vec![Index::new((0..k).collect())], serves: vec![Vec::new()], indexed: 0, pending: keys::RowSet::default(),
              sketches: vec![Sketch::default(); k], dead: Vec::new(), ndead: 0, summaries: Default::default(), sorted: Default::default(), from: None,
              level: Vec::new(), lbase: 0, leveled: true, stats: None, stats_dead: Vec::new(), lat: None, cap: u32::MAX, ordered: false }
    }
    /// Makes this relation a lattice's (its `lattice` line): the rows it holds so far fold by key, as a wave's do.
    fn set_lattice(&mut self, name: &str, keys: usize, lattice: Arc<dyn super::Lattice>, widen: Option<u32>) -> Result<()> {
        if keys + 1 != self.k {
            return Err(err(format!("{}: a lattice of {keys} key columns over {} columns", self.table, self.k)));
        }
        let rows = self.data.to_vec(0, self.n, self.k);
        let mut fresh = Rel::new(&self.table, &self.types.clone());
        fresh.stored = 0;
        fresh.lat = Some(Box::new(Lat { name: name.to_owned(), keys, lattice, widen, map: keys::KeyMap::new(None, 0) }));
        fresh.fold(&rows);
        *self = fresh;
        Ok(())
    }
    /// Whether its rows keep levels (decision 0090; the device writes them, 0311).
    /// Its rows as one relation in a block, its dead rows marked: what the device reads in place (decision 0325).
    /// `None` with no rows, or when they are not all in it.
    pub fn in_block(&self) -> Option<InBlock> {
        let b = if self.ndead > 0 { self.data.in_block_dead(&self.dead) } else { self.data.in_block() };
        b.filter(|b| b.rows as usize == self.n)
    }
    /// Room for `n` rows past its rows in its tail's block, for the device to write them there (decision 0384 step
    /// 3): the tail, and the place in it of the first. `push_fresh` then claims them (`Placed::Tail`).
    pub fn tail_room(&self, n: usize) -> Option<crate::code_rule_dsl::facts::rows::Lent> {
        self.data.tail_room(n)
    }
    /// `tail_room`, when its tail's block has no room: the tail moved into a relation with room (`Rows::tail_moved`).
    pub fn tail_moved(&self, n: usize) -> Option<crate::code_rule_dsl::facts::rows::Lent> {
        self.data.tail_moved(n)
    }
    /// Whether it folds rows into a lattice (decision 0124) rather than adding them.
    pub fn folds(&self) -> bool {
        self.lat.is_some()
    }
    pub fn keeps_levels(&self) -> bool {
        self.leveled
    }
    /// Whether this is a lattice relation, and how the device folds it (`metal_join::Batch::lattice_fold`): `(code,
    /// top, widen)`, code 0 min, 1 max, 2 flat, 3 shortlex_min; or why it cannot (another lattice's join is the CPU's).
    pub fn device_lattice(&self) -> Option<std::result::Result<(u32, i64, Option<u32>), String>> {
        let lat = self.lat.as_ref()?;
        Some(match lat.name.as_str() {
            "min" => Ok((0, lat.lattice.top(), lat.widen)),
            "max" => Ok((1, lat.lattice.top(), lat.widen)),
            "flat" => Ok((2, lat.lattice.top(), lat.widen)),
            // Its values are texts the component's rules make into one arena, compared there (decision 0307).
            "shortlex_min" => Ok((3, lat.lattice.top(), lat.widen)),
            other => Err(format!("the lattice {other} (the device folds min, max, flat and shortlex_min)")),
        })
    }
    /// A lattice relation's new rows `rows` (row-major): each key's values joined with its row's, and a key whose
    /// value changed replaced (`Lat`). Whether any key changed.
    fn fold(&mut self, rows: &[i64]) -> bool {
        let k = self.k;
        let mut lat = self.lat.take().expect("a lattice relation");
        let top = lat.lattice.top();
        let mut any = false;
        let mut row = vec![0i64; k];
        for (key, agg) in lat.group(rows, k) {
            lat.map.reserve(&self.data, k, lat.keys, self.data.home().as_ref());
            let slot = lat.map.find(&self.data, k, &key);
            let held = slot.ok().map(|s| lat.map.row(s));
            let changes = slot.map_or(0, |s| lat.map.changes(s));
            let old = held.map(|id| self.data.value(id, k, lat.keys));
            let mut v = match old {
                None => agg,
                Some(o) if o == top => top,
                Some(o) => lat.lattice.lub(o, agg),
            };
            if old == Some(v) {
                continue;
            }
            if let (Some(after), Some(o)) = (lat.widen, old) {
                if changes >= after {
                    v = lat.lattice.widen(o, v);
                }
                if v == o {
                    continue;
                }
            }
            if let Some(id) = held
                && kill(&mut self.dead, id)
            {
                self.ndead += 1;
            }
            row[..lat.keys].copy_from_slice(&key);
            row[lat.keys] = v;
            let id = self.n as u32;
            self.data.extend_from_slice(&row);
            if self.leveled && self.level.len() == self.n {
                self.level.push(self.lbase + self.wave + 1);
            }
            self.n += 1;
            let (Ok(s) | Err(s)) = slot;
            lat.map.set(s, id, changes + 1, self.wave);
            any = true;
        }
        self.lat = Some(lat);
        any
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
        if old.view.is_some() || k != types.len() || gone.k != k {
            return None;
        }
        // A kept relation read from the frames (a new process's) has no orders or sketches: the dedup order is made
        // by one sort of its ids and its sketches counted, its rows still shared (smoking gun 8: it was made from
        // its rows, each pushed again through the dedup set, `compacted`).
        // Only the orders that hold every row are started from: one a reader built over fewer (a lazily filled
        // order) is left out, not a reason to copy every row (0364 R2).
        let whole: Vec<bool> = old.chains.iter().map(|o| o.rows() == old.n).collect();
        let held = old.chains.first().is_some_and(|o| o.cols.len() == k && o.rows() == old.n) && old.sketches.len() == k;
        let made: Option<(Index, Vec<Sketch>)> = (!held).then(|| {
            let mut ix = Index::new((0..k).collect());
            ix.add_run_in(&old.rows, k, 0, old.n, 0, &old.sorted.within(&ix.cols, k, 0, old.n as u32));
            let mut sketches = vec![Sketch::default(); k];
            for id in (0..old.n as u32).filter(|&id| !is_dead(&old.dead, id)) {
                for (c, sk) in sketches.iter_mut().enumerate() {
                    sk.add(old.rows.value(id, k, c));
                }
            }
            (ix, sketches)
        });
        let orders: Vec<Index> = match &made { Some((ix, _)) => vec![ix.clone()], None => old.chains.iter().zip(&whole).filter(|(_, w)| **w).map(|(o, _)| o.clone()).collect() };
        let data = &old.rows;
        let mut dead = old.dead.to_vec();
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
            if let Some(id) = find_row(&orders, data, k, &row, |id| is_dead(&dead, id)) && kill(&mut dead, id) {
                ndead += 1;
            }
        }
        if ndead * 8 > old.n.max(1) {
            return None;
        }
        let mut idx: Vec<Index> = orders;
        if idx.iter().any(|ix| ix.rows() != old.n) {
            return None;
        }
        // Every row is the first wave's (`open` makes it the delta): its runs are stamped so.
        for ix in &mut idx {
            ix.restamp(1);
        }
        let serves = if made.is_none() && old.serves.len() == whole.len() { old.serves.iter().zip(&whole).filter(|(_, w)| **w).map(|(s, _)| s.clone()).collect() } else { idx.iter().map(|_| Vec::new()).collect() };
        let sketches = match made { Some((_, sketches)) => sketches, None => old.sketches.clone() };
        let level = if old.has_levels() { old.level.to_vec() } else { Vec::new() };
        let from = Some(Origin::Based { at: old as *const InRel as usize, n: old.n, dead: Arc::new(old.dead.to_vec()) });
        Some(Rel { table: table.to_string(), types: types.to_vec(), k, n: old.n, data: old.rows.shared(old.n), stored: 0, lo: 0, hi: 0, wave: 0, idx, serves,
                   indexed: old.n, pending: keys::RowSet::default(), sketches, dead, ndead, summaries: Default::default(), sorted: old.sorted.clone(), from, level,
                   lbase: 0, leveled: true, stats: old.stats.clone(), stats_dead: old.dead.to_vec(), lat: None, cap: u32::MAX, ordered: false })
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
        // A kept relation read in place (decision 0456 step 6, a view of its entry) is counted by its view.
        if old.k == k {
            rel.from = Some(Origin::Compacted { at: old as *const InRel as usize, old_n: old.len(), kept: rel.n, removed });
        }
        rel
    }
    #[inline]
    fn dead(&self, id: u32) -> bool {
        is_dead(&self.dead, id) || (self.cap != u32::MAX && self.level.get(id as usize).is_some_and(|&l| l >= self.cap))
    }
    fn row(&self, id: usize) -> Row { self.data.row(id as u32, self.k) }
    /// Row `id`'s value in column `c`, read where it lies.
    fn value(&self, id: usize, c: usize) -> i64 { self.data.value(id as u32, self.k, c) }
    /// A live row equal to `row`, not one of `skip`: in the first order, or pushed since the last wave (`pending`).
    fn find(&self, row: &[i64], skip: impl Fn(u32) -> bool) -> Option<u32> {
        let (data, k) = (&self.data, self.k);
        if let Some(id) = self.pending.find(data, k, row, &skip) {
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
            self.idx[i].reorder(data, k, cols, &self.sorted);
        } else {
            let cols = chained(std::slice::from_ref(&set), k);
            let mut ix = Index::new(cols);
            charge(Unit::IndexEntry, self.indexed as u64);
            // The rows already in runs: those before the delta as of wave 0, the delta's as of its wave; the rest are
            // indexed with the others' at the next `set_delta`.
            let lo = self.lo.min(self.indexed);
            let known = |a: usize, b: usize| self.sorted.within(&ix.cols, k, a as u32, b as u32);
            let (first, second) = (known(0, lo), known(lo, self.indexed));
            ix.add_run_in(data, k, 0, lo, 0, &first);
            ix.add_run_in(data, k, lo, self.indexed, self.wave, &second);
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
        self.pending.reserve(&self.data, k, n);
    }
    /// A row of the device's fixpoint (decision 0311): pushed, or folded into its lattice, a new row's level
    /// `lbase + round + 1`, as the CPU's `lbase + wave + 1` (its seeds' wave is 1, the device's first round).
    fn push_leveled(&mut self, row: &[i64], round: u32) -> bool {
        let before = self.n;
        let any = if self.lat.is_some() { self.fold(row) } else { self.push(row) };
        if self.n > before && self.leveled && self.level.len() == self.n {
            *self.level.last_mut().expect("a level pushed") = self.lbase + round + 1;
        }
        any
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
        self.pending.insert(&self.data, k, id);
        true
    }
    /// A wave's derived rows added as the next wave's, each once, unless held: checked as one sorted batch
    /// (`Fresh::settle`), so the dedup test reads each of the first order's runs once, in order, not row by row
    /// (decision 0112). `pend`: the rows stay in no order past this wave's end (a gate's relation, its delta set
    /// later), so they are also put in `pending` for the dedup tests until then. Whether any was new.
    fn push_fresh(&mut self, mut f: Fresh, pend: bool) -> bool {
        // Rows placed by the device (decision 0325 step 4) are taken as they lie: claimed in this relation's tail,
        // where the device wrote them past its rows (decision 0384 step 3), or their relation made its tail. Beside
        // rows derived here (which may repeat them) or into a lattice (which folds each), they are read with those
        // instead.
        let placed = std::mem::take(&mut f.placed);
        // A lattice relation's rows the device folded, with its key map (decision 0384 step 5): taken as they lie,
        // the map its own, into a relation with none yet (a device fixpoint's heads start empty).
        if let Some(map) = f.map.take()
            && f.n == 0 && self.n == 0 && placed.len() == 1
            && let Some(lat) = self.lat.as_mut()
        {
            let (p, rounds) = placed.into_iter().next().expect("one lot");
            let m = p.len() as usize;
            debug_assert_eq!(map.keys(), m, "a key a row");
            match p {
                Placed::Part(p) => self.data.append_part(p),
                // SAFETY: the writer finished, and wrote every value of the rows `n` counts.
                Placed::Tail { lent, n } => unsafe { self.data.claim_tail(&lent, n) },
            }
            lat.map = map;
            for x in 0..m {
                if self.leveled && self.level.len() == self.n {
                    let round = rounds.as_ref().map_or(self.wave, |r| r[x]);
                    self.level.push(self.lbase + round + 1);
                }
                self.n += 1;
            }
            return m > 0;
        }
        if !placed.is_empty() && (f.n > 0 || self.lat.is_some()) {
            let k = self.k;
            for (p, rounds) in placed {
                // Read where the writer put them; a tail's room is reused by the pushes that settle them.
                for id in 0..p.len() as usize {
                    let row = p.row(id as u32, k);
                    // A fixpoint's row at its round (decision 0311), as its rows were added before step 4.
                    match &rounds {
                        Some(r) if f.levels.len() == f.n => f.add_leveled(&row, r[id]),
                        _ => {
                            // copy (0353 step 6): a placed row read back beside rows derived here (row 8).
                            const PLACED_BACK: rerail_prof::copies::Site = rerail_prof::copies::Site::new(8, "6", "rows the device placed, read back into Fresh (push_fresh)");
                            PLACED_BACK.copied(8 * row.len());
                            f.data.extend_from_slice(&row);
                            f.n += 1;
                        }
                    }
                    f.lost = true;
                }
            }
        } else if !placed.is_empty() {
            let k = self.k;
            let from = self.n;
            for (p, rounds) in placed {
                let m = p.len() as usize;
                match p {
                    Placed::Part(p) => self.data.append_part(p),
                    // SAFETY: the writer finished, and wrote every value of the rows `n` counts.
                    Placed::Tail { lent, n } => unsafe { self.data.claim_tail(&lent, n) },
                }
                for x in 0..m {
                    if self.leveled && self.level.len() == self.n {
                        let round = rounds.as_ref().map_or(self.wave, |r| r[x]);
                        self.level.push(self.lbase + round + 1);
                    }
                    self.n += 1;
                }
            }
            if pend {
                self.pending.reserve(&self.data, k, self.n - from);
                for id in from..self.n {
                    self.pending.insert(&self.data, k, id as u32);
                }
            }
            return self.n > from;
        }
        f.settle(self);
        if self.lat.is_some() {
            debug_assert!(!pend, "a lattice relation in a gate's place");
            return self.fold(&f.data[..f.n * self.k]);
        }
        let (k, m) = (self.k, f.n);
        if m == 0 {
            return false;
        }
        let from = self.n;
        const INTO_RELATION: rerail_prof::copies::Site = rerail_prof::copies::Site::new(8, "6", "head rows copied from Fresh into their relation (push_fresh)");
        // copy (0353 step 6): head rows staged in `Fresh`, then copied into their relation (row 8).
        INTO_RELATION.time(|| (self.data.extend_from_slice(&f.data[..m * k]), 8 * m * k));
        // The rows come sorted by the first order (`Fresh::settle`): known so, their runs take them in place.
        self.sorted.note(&self.data, k, &self.idx[0].cols, from as u32, (from + m) as u32);
        for _ in 0..m {
            if self.leveled && self.level.len() == self.n {
                self.level.push(self.lbase + self.wave + 1);
            }
            self.n += 1;
        }
        if pend {
            self.pending.reserve(&self.data, k, self.n - from);
            for id in from..self.n {
                self.pending.insert(&self.data, k, id as u32);
            }
        }
        true
    }
    /// Rows `indexed..n` counted in the sketches; `pending` emptied (they are in the orders now).
    fn indexed_to_n(&mut self) {
        let _s = (self.n - self.indexed >= SPANNED_ROWS).then(|| crate::code_rule_dsl::logic::profile::span(|| "rows sketched and counted (indexed_to_n)".into()));
        let (data, k, lo, hi) = (&self.data, self.k, self.indexed, self.n);
        let sketch = |c: usize, sk: &mut Sketch| for id in lo..hi { sk.add(data.value(id as u32, k, c)); };
        let count = |stats: &mut Option<KeptStats>| if let Some(stats) = stats { stats.0.delta(&data.to_vec(lo, hi, k), &[], k); };
        // Many rows: each column's sketch, and the statistics, a job of its own (decision 0294). A sketch takes its
        // column's values in the rows' order, as one thread gives them (its exact counts give way in that order).
        if hi - lo >= SPANNED_ROWS && crate::threads::max() > 1 {
            let (sketches, stats) = (&mut self.sketches, &mut self.stats);
            crate::threads::scope(|jobs| {
                for (c, sk) in sketches.iter_mut().enumerate() {
                    jobs.spawn("a column sketched", move |_| sketch(c, sk));
                }
                jobs.spawn("rows counted", move |_| count(stats));
            });
        } else {
            for (c, sk) in self.sketches.iter_mut().enumerate() {
                sketch(c, sk);
            }
            count(&mut self.stats);
        }
        self.indexed = self.n;
        self.pending = keys::RowSet::default();
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
                    let row = self.data.row(id as u32, k);
                    if now >> (id % 64) & 1 == 1 { minus.extend_from_slice(&row) } else { plus.extend_from_slice(&row) }
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
            let known = |a: usize, b: usize| self.sorted.within(&ix.cols, k, a as u32, b as u32);
            let (first, second) = (known(from, split), known(split, n));
            ix.add_run_in(&self.data, k, from, split, self.wave, &first);
            ix.add_run_in(&self.data, k, split, n, w, &second);
        }
        self.indexed_to_n();
        self.lo = lo;
        self.hi = hi;
        self.wave = w;
    }
    /// The rows pushed before the first wave (the kept rows the store was given) in every order, as rows of the wave
    /// before it: the maintain mode probes before any wave (decision 0364), and a probe reads every row in no order
    /// yet (`Cands::Pending`). Its rechecks then scanned the kept relation, a candidate at a time (t43, step 1).
    fn index_kept(&mut self) {
        let (from, n, k) = (self.indexed, self.n, self.k);
        if from >= n {
            return;
        }
        for ix in &mut self.idx {
            let known = self.sorted.within(&ix.cols, k, from as u32, n as u32);
            ix.add_run_in(&self.data, k, from, n, self.wave, &known);
        }
        self.indexed_to_n();
    }
    /// The rows pushed since the last wave as runs of no wave's delta (a relation frozen, `InRel::moved`), then at
    /// most `FROZEN_RUNS` runs an order: its probes cannot merge them later.
    fn index_rest(&mut self) {
        let (from, n, k) = (self.indexed, self.n, self.k);
        for ix in &mut self.idx {
            ix.settle(&self.data, k, u32::MAX);
            let known = self.sorted.within(&ix.cols, k, from as u32, n as u32);
            ix.add_run_in(&self.data, k, from, n, 0, &known);
            ix.tier(&self.data, k, FROZEN_RUNS);
        }
        self.indexed_to_n();
    }
    /// The rows whose columns `keys` hold the values `key` (in that order), in the delta only or in all:
    /// (candidates, count). The order answering the most of them leads (each is checked by the caller); with none,
    /// every row of the delta or of all. Rows pushed since the last wave (in no order yet; none on the engine's own
    /// path, which probes after `set_delta`) are candidates of all, as they are of no delta.
    /// `probe`, each run of the order searched from where this sequence of probes last was in it (`hints`).
    /// `within`: a range of a column (`Ranged`), read as such where the order's column after the key's is that column.
    fn probe_at(&self, keys: &[usize], key: &[i64], delta: bool, hints: &mut [u32; runs::HINTS], within: Within) -> (Cands<'_>, usize) {
        let Some((ix, m)) = best_order(&self.idx, keys) else { return self.probe(keys, key, delta) };
        let mut buf = [0i64; MAX_KEY];
        let at = |c: usize| key[keys.iter().position(|&x| x == c).expect("a key column")];
        let mut found = ix.find_at(&self.data, self.k, permuted(&ix.cols[..m], at, &mut buf), delta.then_some(self.wave), hints);
        if let Some((c, lo, hi)) = within
            && m == keys.len()
            && ix.cols.get(m) == Some(&c)
        {
            found = found.within(&self.data, self.k, c, lo, hi);
            if probed::on() { probed::note(&format!("{} range", self.table), std::iter::once(c)); }
        }
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
/// A join step's last probe of a source's lookup: its key and the candidates it gave.
type LastProbe = Option<(Vec<i64>, Probed)>;
/// A probe's candidates: their ids, or their ids with their rows' values read as they were found (`View::find_read`).
#[derive(Clone)]
enum Probed { Ids(Vec<u32>), Read(Arc<Vec<(u32, [i64; READ])>>, u64) }
#[derive(Clone)]
enum Cands<'a> { Range(u32, u32), Slice(&'a [u32]), Owned(Vec<u32>), Runs(runs::Found<'a>), Pending(runs::Found<'a>, u32, u32),
    /// Rows with the values a step reads, read as the probe found them (`View::find_read`).
    Read(Arc<Vec<(u32, [i64; READ])>>) }
impl Cands<'_> {
    /// Calls `f` on each id until it returns true; returns whether one did.
    fn any(self, mut f: impl FnMut(u32) -> bool) -> bool {
        match self {
            Cands::Range(lo, hi) => (lo..hi).any(f),
            Cands::Slice(ids) => ids.iter().any(|&id| f(id)),
            Cands::Owned(ids) => ids.into_iter().any(f),
            Cands::Runs(found) => found.ids().any(f),
            Cands::Pending(found, lo, hi) => found.ids().any(&mut f) || (lo..hi).any(f),
            Cands::Read(rows) => rows.iter().any(|&(id, _)| f(id)),
        }
    }
}

/// The tuples of one relation derived in a wave, flat, not yet checked against it (decision 0112: the dedup test a
/// batch at a time). Rows `..settled` are distinct, not held by the relation, and sorted by its first order; the rest
/// are as derived. `emitted`: every row a rule derived this wave, new or not (`RERAIL_SPIKE_DERIVATIONS`: the
/// derivations a counting scheme would count).
/// `breaks`: the rows past `settled` that come before the row derived just before them in the first order (decision
/// 0187: the writer records what it knows), so `settle` sorts only the stretches out of order and merges the rest.
/// `lost`: rows came that were not checked so (`add`), or so many breaks that the whole batch is sorted.
#[derive(Clone, Default)]
/// `levels`: per row, the device fixpoint's round that made it (decision 0311), where its relation keeps levels; else
/// empty.
pub struct Fresh { data: Vec<i64>, n: usize, emitted: u64, settled: usize, breaks: Vec<u32>, lost: bool, levels: Vec<u32>,
                   /// Rows written in place by another writer (the device, decision 0325 step 4), each lot distinct,
                   /// not held by the relation, and in no order: the relation takes them as they lie (`push_fresh`).
                   /// With each lot, its rows' rounds when the device's fixpoint made them (decision 0311's levels).
                   placed: Vec<(Placed, Option<Vec<u32>>)>,
                   /// A lattice relation's key map the device folded its placed rows through (decision 0384 step 5):
                   /// the relation takes it as its own with them.
                   map: Option<keys::KeyMap> }

/// Where another writer (the device) wrote rows of a relation (decision 0325 step 4).
#[derive(Clone)]
pub enum Placed {
    /// Into a relation of their own, which the relation takes as its tail (`Rows::append_part`): where its tail
    /// had no room for them.
    Part(crate::code_rule_dsl::facts::rows::Part),
    /// Into the relation's own tail, rows `at..at + n` of the room it gave (`Rows::tail_room`, decision 0384 step 3):
    /// the relation claims them (`Rows::claim_tail`), and stays one relation.
    Tail { lent: Arc<crate::code_rule_dsl::facts::rows::Lent>, n: u32 },
}

impl Placed {
    /// Its rows.
    pub fn len(&self) -> u32 {
        match self {
            Placed::Part(p) => p.len(),
            Placed::Tail { n, .. } => *n,
        }
    }

    /// Its row `i`'s `k` values, where the writer put them.
    fn row(&self, i: u32, k: usize) -> crate::code_rule_dsl::facts::rows::Row {
        match self {
            Placed::Part(p) => crate::code_rule_dsl::facts::rows::Row::from_fn(k, |c| p.get(i, c)),
            // SAFETY: the writer finished (the lot is placed after its batch), and wrote every row of its room.
            Placed::Tail { lent, .. } => unsafe { Rows::unclaimed(lent, i, k) },
        }
    }
}

/// The shortest stretch of a wave's batch taken as in order (`Fresh::settle`): shorter ones are sorted with their
/// neighbours, as merging them costs about what sorting them does.
const IN_ORDER: usize = 64;

/// Rows a wave's derived tuples hold before they are checked against their relation (`Fresh::offer`): a wave that
/// derives many rows already held keeps at most about this many more.
const FRESH_BATCH: usize = 1 << 16;

/// `RERAIL_SPIKE_DERIVATIONS`: rows derived (new or already held) and rows held new, by relation, over the run
/// (`run::local`, decision 0442).
#[derive(Default)]
pub struct Derivations(pub std::sync::Mutex<std::collections::BTreeMap<String, (u64, u64)>>);
impl Fresh {
    /// Rows another writer wrote in place (`placed`): distinct, none held by the relation; with `rounds`, each row's
    /// round of the device's fixpoint (its level, as `add_leveled`'s).
    pub fn place(&mut self, placed: Placed, rounds: Option<Vec<u32>>) {
        debug_assert!(rounds.as_ref().is_none_or(|r| r.len() == placed.len() as usize), "a round per row");
        if placed.len() > 0 {
            self.placed.push((placed, rounds));
        }
    }
    /// A lattice relation's rows another writer folded in place (the device, decision 0384 step 5): one live row a
    /// key, each with its round where the relation keeps levels, and the key map it folded them through.
    pub fn place_folded(&mut self, placed: Placed, rounds: Option<Vec<u32>>, map: keys::KeyMap) {
        debug_assert!(self.placed.is_empty() && self.n == 0, "a fold's rows beside others");
        self.place(placed, rounds);
        self.map = Some(map);
    }
    /// The last lot another writer wrote (`place`): where a next one for the same relation goes on (decision 0384 step 3).
    pub fn last_placed(&self) -> Option<&Placed> {
        self.placed.last().map(|(p, _)| p)
    }
    /// A row in no known order (its batch is sorted whole).
    pub fn add(&mut self, row: &[i64]) { self.data.extend_from_slice(row); self.n += 1; self.lost = true; }
    /// A row of the device's fixpoint and the round that made it (decision 0311: its level, above the run's base).
    pub fn add_leveled(&mut self, row: &[i64], round: u32) {
        debug_assert_eq!(self.levels.len(), self.n, "a leveled row among unleveled ones");
        self.add(row);
        self.levels.push(round);
    }
    /// A row a rule derived for `rel`: kept unchecked, the rows past `settled` checked a batch at a time. Whether it
    /// comes before the last one in `rel`'s first order is noted (`breaks`): one comparison of two rows at hand.
    #[inline]
    fn offer(&mut self, row: &[i64], rel: &Rel) {
        let k = rel.k;
        if !self.lost && self.n > self.settled && runs::known_beyond_runs() {
            let cols = &rel.idx[0].cols;
            if runs::cmp_vals(cols, runs::tagged(cols, k), &self.data[(self.n - 1) * k..self.n * k], row).is_gt() {
                self.breaks.push(self.n as u32);
                // Out of order throughout: sorted whole, the breaks no longer kept.
                if self.breaks.len() * IN_ORDER > self.n - self.settled + IN_ORDER * IN_ORDER {
                    self.lost = true;
                    self.breaks = Vec::new();
                }
            }
        }
        self.data.extend_from_slice(row);
        self.n += 1;
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
            runs::cmp_ids(data, k, cols, tagged, a, b)
        };
        let mut order: Vec<u32> = (settled as u32..n as u32).collect();
        runs::presorted::site(runs::presorted::FRESH, n - settled, |i| cmp((settled + i - 1) as u32, (settled + i) as u32).is_gt());
        let sort = |v: &mut [u32]| {
            runs::sort_keyed(v, tagged, cols.len(), |q, i| data.value(q, k, cols[i]), &cmp, &mut runs::NoTags);
        };
        if self.lost || !runs::known_beyond_runs() {
            sort(&mut order);
        } else {
            // The stretches between breaks, as derived, are in order: the long ones merged, not sorted again.
            let mut known: Vec<(usize, usize)> = Vec::new();
            let mut start = 0;
            for &b in self.breaks.iter().chain(std::iter::once(&(n as u32))) {
                let b = b as usize - settled;
                if b - start >= IN_ORDER {
                    known.push((start, b));
                }
                start = b;
            }
            let taken = runs::sort_pieces(&mut order, &known, sort, |a, b| cmp(a, b).is_le());
            runs::presorted::site_known(runs::presorted::FRESH, taken as u64);
        }
        self.breaks.clear();
        self.lost = false;
        order.dedup_by(|a, b| cmp(*a, *b).is_eq());
        // The new rows' check: rows pushed since the last wave (in no order yet), then the first order.
        let mut held = vec![false; order.len()];
        if rel.indexed < rel.n {
            for (h, &q) in held.iter_mut().zip(&order) {
                *h = rel.contains(&data.row(q, k));
            }
        }
        rel.idx[0].held_sorted(&rel.data, k, data, &order, |id| !rel.dead(id), &mut held);
        // Merged with the settled rows (sorted, distinct, not held), less those the new rows repeat.
        let mut out: Vec<i64> = Vec::with_capacity((settled + order.len()) * k);
        let (mut s, mut kept) = (0, 0);
        for (&q, h) in order.iter().zip(held) {
            while s < settled && cmp(s as u32, q).is_lt() {
                out.extend_from_slice(&data.row(s as u32, k));
                (s, kept) = (s + 1, kept + 1);
            }
            if !h && !(s < settled && cmp(s as u32, q).is_eq()) {
                out.extend_from_slice(&data.row(q, k));
                kept += 1;
            }
        }
        out.extend_from_slice(&data[s * k..settled * k]);
        self.n = kept + settled - s;
        self.settled = self.n;
        self.data = out;
    }
}
impl Fresh {
    /// The rows a morsel of the same walk settled (`settle`: sorted by the first order, distinct, none held), after
    /// this one's: one stretch in order, so the next `settle` merges it rather than sorting it.
    fn absorb(&mut self, o: Fresh) {
        self.emitted += o.emitted;
        if o.n == 0 {
            return;
        }
        debug_assert_eq!(o.settled, o.n, "a morsel's rows are settled");
        if !self.lost && self.n > self.settled {
            self.breaks.push(self.n as u32);
            if self.breaks.len() * IN_ORDER > self.n - self.settled + o.n + IN_ORDER * IN_ORDER {
                self.lost = true;
                self.breaks = Vec::new();
            }
        }
        if self.data.is_empty() {
            self.data = o.data;
        } else {
// copy (0353 step 6): head rows staged and merged before their relation takes them. They go: a CPU stratum
// claims and writes its rows in the relation's block, as the device does (decision 0353).
            const MERGED: rerail_prof::copies::Site = rerail_prof::copies::Site::new(8, "6", "head rows staged, merged across workers (Fresh::absorb)");
            MERGED.copied(8 * o.data.len());
            self.data.extend_from_slice(&o.data);
        }
        self.n += o.n;
    }
}

/// A wave of at least this many rows has its settling, indexing and sketching profiled apart (spans cost a clock read).
const SPANNED_ROWS: usize = 1 << 16;

struct EdgeRule { head: usize, src: usize, key: Vec<usize>, ecols: Vec<usize>, pat: Vec<Pat>, out: Vec<Term>,
                  edges: HashMap<Vec<i64>, Vec<Vec<i64>>>, has_edge: bool, nslots: usize }
/// A probe's body: SQL over the `set_rel` vtabs, or a native join pipeline (`join.rs` spec).
/// `Unplanned`: a join probe whose plan is made when its delta first has rows (decision 0123 §9, `Line::Unplanned`).
enum Body { Sql(String), Join(JoinRule), Unplanned }
/// `whole`: a join probe's plan for the wave whose delta is its whole relation (the relation was empty before it), in
/// any literal order, reading that relation in full. `wkey`: its spec, the same for each probe of one rule: in a wave
/// the store does not change, so a whole plan run once derives what each of its rule's probes would (it is not run
/// again).
/// `budget`: the work (steps entered) the probe may still do before it is cut (decision 0206), set by the stratum's
/// driver; none, no limit.
/// `full`: its head reached the width cap under `RERAIL_WIDEST=cut` (decision 0462): it derives no more this run.
struct ProbeRule { head: usize, delta: usize, body: Body, whole: Option<JoinRule>, wkey: Option<Arc<JoinSpec>>, budget: std::cell::Cell<Option<u64>>, full: std::cell::Cell<bool> }

pub struct Store { pub rels: Vec<Rel>, syms: Arc<Symbols> }
/// `seeded`: the seed joins have run (in the first wave). `write`: at the fixpoint, insert the new rows into the
/// relations' tables (else they stay native, in `DERIVED`, until `set_fixpoint_materialize`).
/// `requests`: relations whose new rows the caller answers (`pending`, `add`), each with the rows answered so far.
/// `produced`: relations only the caller adds to, which the fixpoint does not write.
/// `device`: the seed joins' device plans, by probe, when the spec asked for the device and it takes every one of them
/// (`device::compile`); `seed_specs`, their specs, to parse them again with the CPU's indexes when the device refuses
/// one at run time.
struct Component { store: Arc<RwLock<Store>>, edges: Vec<EdgeRule>, probes: Vec<ProbeRule>, gates: Vec<Gate>, seeded: bool, write: bool, done: bool,
                   requests: Vec<(usize, usize)>, produced: Vec<usize>, closure: Option<closure::Closure>,
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
                   seed_specs: Vec<(usize, Arc<JoinSpec>)>,
                   /// A wave stopped at a probe that spent its budget (decision 0206): resumed by the next `run`.
                   cut: Option<Cut>,
                   /// The last cut, for the driver (`take_cut`).
                   last_cut: Option<CutReport>,
                   /// The rows of the cut probe's first trial to finish (O169): its probe, its sample, the rows it
                   /// made (flat, its head's width). Every finished trial over that sample made the same rows.
                   trial_rows: Option<(usize, Rows0, Vec<i64>)>,
                   /// The maintain mode's joins (decision 0364 stage 3), run once before the seeds.
                   maint: MaintMode }

/// The maintain mode of a non-recursive component (decision 0364 stage 3). Its relations start as their kept ones,
/// in place (`Start::Base`). Each removal join reads the versions of what changed below (a changed atom's rows gone
/// over the others' rows at the start) and gives candidates; a candidate that no rule derives over the relations now
/// (`rechecks`, its head bound) is killed in place. The add joins are the component's seeds (exact: a changed atom's
/// rows added, over the relations now), and `adds` those checked first. Candidates are a buffer of the run.
#[derive(Default)]
struct MaintMode {
    removes: Vec<(usize, JoinRule)>,
    /// Per removal line, its spec's first two items, as the profile names the line's span.
    remove_names: Vec<String>,
    adds: Vec<(usize, JoinRule)>,
    rechecks: Vec<(usize, JoinRule)>,
    /// A recursive component's (by levels, decision 0364 §2): per delta probe, the relation whose row drives it, its
    /// head, the probe's join without that atom (its slots bound), and that atom's arguments: the rows that read a
    /// row gone, found from the row; last, the probe it is made from (its index in `probes`), whose kept plan its
    /// work is reported under (`driver_step_counts`).
    drivers: Vec<(usize, usize, JoinRule, Vec<Arg>, usize)>,
    leveled: bool,
    /// A component the static check proved (`Line::Measured`): candidates in its measure's order, rechecked once
    /// each, with no levels.
    measured: Option<Measured>,
    done: bool,
    /// Candidates, those killed, and checked adds added: the run's report.
    counts: [u64; 3],
}

/// A proved component's measure (`Line::Measured`).
struct Measured {
    desc: bool,
    /// The relation of each node's key (`ast_depth`, `node_order`), read through its bound source in place: no SQL
    /// inside the waves (decision 0104).
    key: Option<String>,
    /// Whether the order is the measure's: else a candidate that stays is checked again when a row it may read goes.
    keyed: bool,
    cols: Vec<usize>,
    ranks: Vec<usize>,
}

impl Measured {
    /// Row `row` of relation `r`'s place in the order, as the heap pops it (least first): its key, then its rank.
    /// `None`: its node has no key now (a removal's candidate at a node the edit took out of the tree).
    fn key(&self, conn: Option<&Connection>, r: usize, row: &[i64]) -> Result<Option<(i64, u32)>> {
        let v = *row.get(self.cols[r]).ok_or_else(|| err(format!("measured: relation {r} has no column {}", self.cols[r])))?;
        let k = match &self.key {
            None => v,
            Some(rel) => {
                let conn = conn.ok_or_else(|| err("measured: a key read with no connection".to_owned()))?;
                let source = crate::code_rule_dsl::facts::vtab::bound_source(conn, rel).ok_or_else(|| err(format!("measured: no source bound to {rel}")))?;
                let cols = source.columns();
                let at = |c: &str| cols.iter().position(|x| x == c).ok_or_else(|| err(format!("measured: {rel} has no column {c}")));
                let (n, kc) = (at("n")?, at("k")?);
                let rows = match source.lookup(n, &[v]) {
                    Some(rows) => rows,
                    None => {
                        source.prepare_index(&[n]);
                        source.lookup(n, &[v]).ok_or_else(|| err(format!("measured: {rel} has no lookup by its node")))?
                    }
                };
                match rows.first() {
                    Some(&row) => source.value(row, kc),
                    None => return Ok(None),
                }
            }
        };
        Ok(Some((if self.desc { !k } else { k }, self.ranks[r] as u32)))
    }
}

impl MaintMode {
    /// Whether one of `head`'s rules derives `row` over the relations now.
    fn derives(&self, head: usize, row: &[i64], store: &Store, syms: &mut Interner, out: &mut Vec<i64>) -> Result<bool> {
        'rules: for (_, j) in self.rechecks.iter().filter(|(h, _)| *h == head) {
            // The rule's head must be able to name the row: its constants equal the row's values, a slot repeated
            // the same value. Else it derives no such row (`v("CYC", i)` is not derived by `v("SEESE", i) :- …`).
            let mut preset: Vec<(usize, i64)> = Vec::with_capacity(j.out.len());
            for (t, &v) in j.out.iter().zip(row) {
                match t {
                    Term::Const(c) if *c != v => continue 'rules,
                    Term::Const(_) => {}
                    Term::Slot(s) => match preset.iter().find(|(x, _)| x == s) {
                        Some(&(_, w)) if w != v => continue 'rules,
                        Some(_) => {}
                        None => preset.push((*s, v)),
                    },
                    // A computed term is not bound by the candidate: the rows the rule derives are compared below.
                    Term::Cat(..) => {}
                }
            }
            out.clear();
            run_join_collect(j, store, syms, &preset, out)?;
            // The row itself, not only a row of its bound columns: a computed head term, or a slot a later step binds
            // again (an aggregate's value), makes rows the candidate must equal.
            if out.chunks_exact(row.len().max(1)).any(|got| got == row) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// A profile's view of `head`'s rechecks: per rule, its steps as source:keys (`F` full, `M` several keys, `D`
    /// delta, `I` input), `!` negated.
    fn recheck_shape(&self, head: Option<usize>) -> String {
        let Some(head) = head else { return String::new() };
        self.rechecks.iter().filter(|(h, _)| *h == head).map(|(_, j)| {
            j.steps.iter().filter_map(|s| match s {
                Step::Atom { src, neg, keys, .. } => Some(format!("{}{}{}:{}", if *neg { "!" } else { "" }, match src { Src::Full(_) => "F", Src::FullMulti(..) => "M", Src::Delta(_) => "D", Src::In(r, _) => if r.view.is_some() { "V" } else { "I" } },
                    match src { Src::Full(r) | Src::FullMulti(r, _) | Src::Delta(r) => r.to_string(), Src::In(r, _) => r.n.to_string() }, keys.len())),
                _ => None,
            }).collect::<Vec<_>>().join(" ")
        }).collect::<Vec<_>>().join(" | ")
    }

    /// The rows of `driver`'s head that read `row` of its relation (`drivers`): its atom's slots preset from the row,
    /// where the row matches the atom's constants and repeated slots.
    fn readers(driver: &(usize, usize, JoinRule, Vec<Arg>, usize), row: &[i64], store: &Store, syms: &mut Interner, out: &mut Vec<i64>) -> Result<()> {
        let (_, _, j, args, _) = driver;
        let mut preset: Vec<(usize, i64)> = Vec::with_capacity(args.len());
        for (a, &v) in args.iter().zip(row) {
            match a {
                Arg::Tok(Tok::Slot(s)) => match preset.iter().find(|(x, _)| x == s) {
                    Some(&(_, w)) if w != v => return Ok(()),
                    Some(_) => {}
                    None => preset.push((*s, v)),
                },
                Arg::Tok(Tok::Int(i)) if *i != v => return Ok(()),
                Arg::Tok(Tok::Sym(x)) if syms.id(x) != v => return Ok(()),
                _ => {}
            }
        }
        out.clear();
        run_join_collect(j, store, syms, &preset, out)?;
        Ok(())
    }

    /// A recursive component by levels (decision 0364 §2): candidates in level order, lowest first. One stays when a
    /// rule derives it from rows of its component present and below its level (`Rel::cap`); else it goes for now,
    /// and the rows that read it are candidates at their own levels. Then each row gone that a rule derives now is
    /// added again (a new row, above every kept level: a rewrite is a kill and an add), with the checked adds, as the
    /// first wave's delta: the waves carry them on.
    ///
    /// A component the static check proved (`measured`, decision 0364 §5) keeps no levels: its candidates go in its
    /// measure's order, and a row's derivations read only rows before it there, all decided. So each is rechecked
    /// once, over the relations as they are (no `cap`), and a row gone is never derived again. Where the run cannot
    /// read the measure's key (`!keyed`), a candidate that stays is checked again when a row it may read goes: a
    /// kill is sound whatever the order (rows only go, so a row no rule derives now none derives at the end), and
    /// with no cycle every stay is checked against rows that stay. A candidate whose node has no key now (`loose`: the
    /// edit took the node out of the tree) is treated so alone: it goes first, and when it stays it is checked again
    /// as a row it may read goes; a loose row that goes queues its readers again, decided or not.
    fn run_leveled(&mut self, store: &mut Store, syms: &mut Interner, conn: Option<&Connection>) -> Result<()> {
        use std::cmp::Reverse;
        type Heap = std::collections::BinaryHeap<Reverse<(i64, u32, usize, u32)>>;
        let mut out = Vec::new();
        let mut heap: Heap = Default::default();
        let mut seen: rustc_hash::FxHashSet<(usize, u32)> = Default::default();
        let measured = self.measured.take();
        let what = if measured.is_some() { "measure" } else { "levels" };
        // `seen`: the rows queued once, ever; unkeyed, the rows queued now (a decided row may be queued again).
        let again = measured.as_ref().is_some_and(|m| !m.keyed);
        let mut loose: rustc_hash::FxHashSet<(usize, u32)> = Default::default();
        let push = |heap: &mut Heap, store: &Store, r: usize, rows: &[i64], seen: &mut rustc_hash::FxHashSet<(usize, u32)>, counts: &mut [u64; 3], loose: &mut rustc_hash::FxHashSet<(usize, u32)>, force: bool| -> Result<()> {
            let rel = &store.rels[r];
            for row in rows.chunks_exact(rel.k.max(1)) {
                if let Some(id) = rel.find(row, |id| is_dead(&rel.dead, id)) && (seen.insert((r, id)) || force) {
                    counts[0] += 1;
                    let (k, rank) = match &measured {
                        Some(m) => match m.key(conn, r, row)? {
                            Some(key) => key,
                            None => {
                                loose.insert((r, id));
                                (i64::MIN, 0)
                            }
                        },
                        None => (i64::from(rel.level.get(id as usize).copied().unwrap_or(0)), 0),
                    };
                    heap.push(Reverse((k, rank, r, id)));
                }
            }
            Ok(())
        };
        for (head, j) in &self.removes {
            let _s = crate::code_rule_dsl::logic::profile::span(|| format!("mode: a removal join ({what})"));
            let mut rows = Vec::new();
            run_join_collect(j, store, syms, &[], &mut rows)?;
            push(&mut heap, store, *head, &rows, &mut seen, &mut self.counts, &mut loose, false)?;
        }
        let mut gone: Vec<(usize, Vec<i64>)> = Vec::new();
        let _s = crate::code_rule_dsl::logic::profile::span(|| format!("mode: candidates by {what} ({} first)", heap.len()));
        while let Some(Reverse((l, _, r, id))) = heap.pop() {
            let is_loose = loose.contains(&(r, id));
            if again || is_loose {
                seen.remove(&(r, id));
            }
            let row: Vec<i64> = store.rels[r].row(id as usize).to_vec();
            let stays = if measured.is_some() {
                self.derives(r, &row, store, syms, &mut out)
            } else {
                for rel in &mut store.rels {
                    rel.cap = l as u32;
                }
                let stays = self.derives(r, &row, store, syms, &mut out);
                for rel in &mut store.rels {
                    rel.cap = u32::MAX;
                }
                stays
            };
            if stays? {
                continue;
            }
            let rel = &mut store.rels[r];
            if kill(&mut rel.dead, id) {
                rel.ndead += 1;
                self.counts[1] += 1;
            }
            for d in self.drivers.iter().filter(|d| d.0 == r) {
                Self::readers(d, &row, store, syms, &mut out)?;
                let rows = std::mem::take(&mut out);
                push(&mut heap, store, d.1, &rows, &mut seen, &mut self.counts, &mut loose, is_loose)?;
                out = rows;
            }
            if measured.is_none() {
                gone.push((r, row));
            }
        }
        drop(_s);
        // Added again where a rule derives it now, and the checked adds; then the first wave's delta.
        let before: Vec<usize> = store.rels.iter().map(|r| r.n).collect();
        let mut again: Vec<(usize, Vec<i64>)> = Vec::new();
        let _s = crate::code_rule_dsl::logic::profile::span(|| format!("mode: the rows gone derived again? ({}) {}", gone.len(), self.recheck_shape(gone.first().map(|g| g.0))));
        for (r, row) in gone {
            if self.derives(r, &row, store, syms, &mut out)? {
                again.push((r, row));
            }
        }
        drop(_s);
        for (head, j) in &self.adds {
            let _s = crate::code_rule_dsl::logic::profile::span(|| format!("mode: a checked add ({what})"));
            let mut rows = Vec::new();
            run_join_collect(j, store, syms, &[], &mut rows)?;
            let k = store.rels[*head].k;
            for row in rows.chunks_exact(k.max(1)) {
                let rel = &store.rels[*head];
                if rel.find(row, |id| rel.dead(id)).is_none() && self.derives(*head, row, store, syms, &mut out)? {
                    again.push((*head, row.to_vec()));
                }
            }
        }
        for (r, row) in again {
            if store.rels[r].push(&row) {
                self.counts[2] += 1;
            }
        }
        for (r, rel) in store.rels.iter_mut().enumerate() {
            if rel.n > before[r] {
                rel.set_delta(before[r], rel.n);
            }
        }
        Ok(())
    }

    /// The removals, then the checked adds (decision 0364 stage 3). A non-recursive component's rules read no
    /// relation of it, so a recheck reads only lower strata, whose rows are final: candidates are decided in any order.
    fn run(&mut self, store: &mut Store, syms: &mut Interner, conn: Option<&Connection>) -> Result<()> {
        if self.done {
            return Ok(());
        }
        self.done = true;
        {
            let _s = crate::code_rule_dsl::logic::profile::span(|| "mode: the kept rows indexed".into());
            for rel in &mut store.rels {
                rel.index_kept();
            }
        }
        if self.leveled {
            return self.run_leveled(store, syms, conn);
        }
        let mut out = Vec::new();
        let mut kills: Vec<(usize, u32)> = Vec::new();
        let mut seen: rustc_hash::FxHashSet<(usize, u32)> = Default::default();
        for (i, (head, j)) in self.removes.iter().enumerate() {
            let mut rows = Vec::new();
            {
                let _s = crate::code_rule_dsl::logic::profile::span(|| format!("mode: a removal join: {}", self.remove_names.get(i).map_or("", String::as_str)));
                run_join_collect(j, store, syms, &[], &mut rows)?;
            }
            let _s = crate::code_rule_dsl::logic::profile::span(|| format!("mode: rechecks ({} candidate rows)", rows.len() / store.rels[*head].k.max(1)));
            let k = store.rels[*head].k;
            // A derived relation has a column at least (`simplify::unit_columns`).
            for row in rows.chunks_exact(k.max(1)) {
                let rel = &store.rels[*head];
                let Some(id) = rel.find(row, |id| rel.dead(id)) else { continue };
                if !seen.insert((*head, id)) {
                    continue;
                }
                self.counts[0] += 1;
                if !self.derives(*head, row, store, syms, &mut out)? {
                    kills.push((*head, id));
                }
            }
        }
        for &(head, id) in &kills {
            let rel = &mut store.rels[head];
            if kill(&mut rel.dead, id) {
                rel.ndead += 1;
                self.counts[1] += 1;
            }
        }
        let mut added: Vec<(usize, Vec<i64>)> = Vec::new();
        for (head, j) in &self.adds {
            let _s = crate::code_rule_dsl::logic::profile::span(|| "mode: a checked add".into());
            let mut rows = Vec::new();
            run_join_collect(j, store, syms, &[], &mut rows)?;
            let k = store.rels[*head].k;
            for row in rows.chunks_exact(k.max(1)) {
                let rel = &store.rels[*head];
                if rel.find(row, |id| rel.dead(id)).is_none() && self.derives(*head, row, store, syms, &mut out)? {
                    added.push((*head, row.to_vec()));
                }
            }
        }
        for (head, row) in added {
            if store.rels[head].push(&row) {
                self.counts[2] += 1;
            }
        }
        Ok(())
    }
}

/// A relation's change since a relation it started as (`InRel::delta_ids_since`): the ids of its rows added, and of
/// those gone, or the rows gone that a compaction recorded (no row of it holds them). Rows are read where they lie.
pub struct DeltaIds<'a> {
    rel: &'a InRel,
    gone: Gone<'a>,
    added: Vec<u32>,
}

enum Gone<'a> {
    Ids(Vec<u32>),
    Rows(&'a [i64]),
}

impl DeltaIds<'_> {
    /// Its rows gone and added, counted.
    pub fn counts(&self) -> (usize, usize) {
        let gone = match &self.gone { Gone::Ids(ids) => ids.len(), Gone::Rows(rows) => rows.len() / self.rel.k.max(1) };
        (gone, self.added.len())
    }
    /// Each row gone (`sign` −1) or added (+1), read where it lies.
    pub fn each(&self, sign: i64, f: &mut dyn FnMut(&[i64])) {
        let k = self.rel.k.max(1);
        if sign > 0 {
            self.added.iter().for_each(|&id| f(&self.rel.row(id as usize)));
            return;
        }
        match &self.gone {
            Gone::Ids(ids) => ids.iter().for_each(|&id| f(&self.rel.row(id as usize))),
            Gone::Rows(rows) => rows.chunks_exact(k).for_each(|r| f(r)),
        }
    }
}

/// A wave stopped at probe `next` (decision 0206): its rows so far, and the rows of its first step's atom it has
/// left (`None`: all of them). The next `run` goes on with the probe's body as it is then (the replacement the
/// driver planned), reading only those rows of that atom (`restricted`).
/// `whole`: the probe ran its whole plan (O168), which goes on there.
struct Cut { fresh: Vec<Fresh>, next: usize, left: Option<Rows0>, step: Option<usize>, on_device: bool, whole: bool }

/// Where a step's rows are, so that rows left by one order are read as the same rows by another: an input's rows, or
/// a component relation's (its delta's ids are the relation's).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RowsOf { In(usize), Delta(usize), Comp(usize) }
impl RowsOf {
    fn of(src: &Src) -> RowsOf {
        match src {
            Src::In(r, _) => RowsOf::In(Arc::as_ptr(r) as usize),
            Src::Delta(r) => RowsOf::Delta(*r),
            Src::Full(r) | Src::FullMulti(r, _) => RowsOf::Comp(*r),
        }
    }
}

/// The step of `j` that reads the rows a cut left (`of`): the step the driver named (`cut_step`: its replacement
/// leads with the atom the old order led with) where it reads them, else the one positive atom that reads them (a
/// delta probe's delta). None where that is not known (two atoms read them, a self-join): the step then reads all its
/// rows, as many twice as the cut order had finished; they make the same rows.
fn restricted(j: &JoinRule, of: RowsOf, named: Option<usize>) -> Option<usize> {
    let reads = |s: &Step| matches!(s, Step::Atom { src, neg: false, .. } if RowsOf::of(src) == of);
    if let Some(i) = named && j.steps.get(i).is_some_and(reads) {
        return Some(i);
    }
    let mut at = j.steps.iter().enumerate().filter(|(_, s)| reads(s)).map(|(i, _)| i);
    match (at.next(), at.next()) {
        (Some(i), None) => Some(i),
        _ => None,
    }
}

/// An atom's rows still to read: a bitset over `lo..`.
#[derive(Clone)]
/// `order`: the rows in the order a cut walk was to read them (`Rows0::left`), which a walk going on from the cut
/// reads them in where its scan's key orders them so (`scan_order`).
struct Rows0 { of: RowsOf, lo: u32, bits: Vec<u64>, n: u64, order: Option<Vec<u32>> }
impl Rows0 {
    /// A cut walk's rows left, in the order it was to read them.
    fn left(of: RowsOf, ids: &[u32]) -> Rows0 {
        Rows0 { order: Some(ids.to_vec()), ..Rows0::of(of, ids) }
    }
    #[inline]
    fn has(&self, id: u32) -> bool {
        let at = id.wrapping_sub(self.lo) as usize;
        self.bits.get(at / 64).is_some_and(|w| w >> (at % 64) & 1 == 1)
    }
    fn of(of: RowsOf, ids: &[u32]) -> Rows0 {
        let lo = ids.iter().copied().min().unwrap_or(0);
        let hi = ids.iter().copied().max().map_or(lo, |h| h + 1);
        let mut bits = vec![0u64; (hi - lo) as usize / 64 + 1];
        let mut n = 0;
        for &id in ids {
            let at = (id - lo) as usize;
            if bits[at / 64] >> (at % 64) & 1 == 0 { n += 1; }
            bits[at / 64] |= 1 << (at % 64);
        }
        Rows0 { of, lo, bits, n, order: None }
    }
}

/// What a cut probe did (decision 0206): its probe, the work it did in this run, the rows of its first step's atom
/// it finished and those it left. `led`: that step read an atom's rows, so the rows finished are known; else none
/// are, and all are left (its partial rows are kept: they are a set's).
#[derive(Clone, Debug, PartialEq)]
pub struct CutReport { pub probe: usize, pub work: u64, pub done: u64, pub left: u64, pub led: bool, pub whole: bool }


/// Decision 0206's counts: cuts, then waves gone on over the rows left only, and over all rows; then trials run.
pub fn cut_counts() -> [u64; 4] {
    run_counts().cuts.each_ref().map(|c| c.load(std::sync::atomic::Ordering::Relaxed))
}

/// The limit a join's walk runs under (`run_join_in_each`): set by `run` for the probe it is about to run. `only`:
/// the rows a step (by index) may read.
struct Limit { budget: u64, only: Option<(usize, Rows0)> }
/// What a limited walk did: its work, and at a cut the first step's rows left (`None`: not cut; `Some(None)`: cut,
/// with every row left).
struct Spent { work: u64, left: Option<Option<Rows0>>, done: u64, led: bool }
/// A walk's limit, handed to it (`run_join_in_each` takes it, so no walk nested in it sees it), and what the walk
/// did under it, handed back.
struct Limited { limit: Option<Limit>, spent: Option<Spent> }

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

/// What the evaluator keeps for a connection is dropped when the connection closes (`forget_scope`, through
/// `vtab::install_forget`), in every binary that runs it: installed here, before the first state is kept, as well as
/// at the engine's load (`code_rule_dsl`'s installs). Without it the logic crate's own tests, which never load the
/// engine, kept a closed connection's relations for the next connection at its address (a join over a test's
/// `events` read the rows of another test's `events`).
pub(crate) fn forgotten_on_close() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| crate::code_rule_dsl::facts::vtab::install_forget(crate::code_rule_dsl::facts::vtab::Forget { scope: forget_scope, engine: forget_engine }));
}

/// Drops what the connection at `at` left: its components and derived relations (`scope`).
pub fn forget_scope(_at: usize, data: &crate::code_rule_dsl::facts::vtab::ConnData) {
    // Its symbols, path, loaded inputs and what its runs left in SQL (`plan::Left`) go with the connection's own
    // values (`OnConn`).
    forget_engine(data);
}

/// Drops the engine's relations and components a run left on the connection at `at` (when it closes, or when the
/// next run reuses it, `vtab::reuse`). They are freed on a thread of their own: no run's work.
pub fn forget_engine(data: &crate::code_rule_dsl::facts::vtab::ConnData) {
    let Some(on) = data.get_if_any::<OnConn>() else { return };
    let gone: (Vec<Arc<Mutex<Component>>>, Vec<Arc<InRel>>) =
        (std::mem::take(&mut *on.components.lock()).into_values().collect(), std::mem::take(&mut *on.derived.lock()).into_values().collect());
    on.adopted.lock().clear();
    on.given.lock().clear();
    if !gone.0.is_empty() || !gone.1.is_empty() {
        rerail_accel::exec::drop_beside(gone);
    }
}
fn component(conn: impl Scoped, name: &str) -> Result<Arc<Mutex<Component>>> {
    component_in(&on(conn), name)
}
fn component_in(on: &OnConn, name: &str) -> Result<Arc<Mutex<Component>>> {
    on.components.lock().get(name).cloned().ok_or_else(|| err(format!("no set fixpoint named {name:?}")))
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
        rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var_os($name).is_some())
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
    let mut maint = MaintMode::default();
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
    // The relations declared sorted (`Line::Sorted`, decision 0456 K7).
    let ordered: Vec<usize> = spec.lines.iter().find_map(|l| if let Line::Sorted(r) = l { Some(r.clone()) } else { None }).unwrap_or_default();
    let mut cyclic: Vec<usize> = Vec::new();
    let mut kept_old = false;
    let mut seed_specs: Vec<(usize, Arc<JoinSpec>)> = Vec::new();
    // Each relation's kept rows (`Start::Base`), by index: under `wave0_changes` they are no wave's delta.
    let mut kept_n: Vec<usize> = Vec::new();
    // The relations whose rows outlive the run: their own blocks (`Life::Kept`).
    let kept_rows: Vec<usize> = spec.lines.iter().find_map(|l| if let Line::KeptRows(r) = l { Some(r.clone()) } else { None }).unwrap_or_default();
    for line in &spec.lines {
        match line {
            Line::Rel { name: table, types, start } => {
                let mut rel = Rel::new(table, types.as_bytes());
                if kept_rows.contains(&rels.len()) {
                    rel.data = Rows::living(rel.k, rows::Life::Kept);
                }
                // `Empty`: the table was made this run and nothing wrote it (no statement). `From`: its rows are a
                // query's (an input's, read where it is).
                // `Base`: the relation starts as a kept native relation less another's rows (the survivors of a
                // maintained stratum, `h__old` less `h__del`), dead in place; else, made from those rows. `seeded`:
                // rows a SQL seed or the program's facts wrote to the table, added to it.
                let what: &'static str = match start { Start::Base { .. } => "open: survivors", Start::From(_) => "open: rows from SQL", Start::Rows => "open: rows from its table", Start::Empty => "open: empty" };
                let _rows = crate::code_rule_dsl::logic::profile::span(move || what.into());
                match start {
                    Start::Base { old, gone, seeded } => {
                        let (o, g) = { let on = on(&conn); let d = on.derived.lock(); (d.get(old.as_str()).cloned(), if gone == "-" { None } else { d.get(gone.as_str()).cloned() }) };
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
            Line::Probe { head, delta, sql } => probes.push(ProbeRule { head: *head, delta: *delta, body: Body::Sql(sql.clone()), whole: None, wkey: None, budget: Default::default(), full: Default::default() }),
            Line::Unplanned { head, delta } => probes.push(ProbeRule { head: *head, delta: *delta, body: Body::Unplanned, whole: None, wkey: None, budget: Default::default(), full: Default::default() }),
            Line::Device => device_wanted = solver().is_some(),
            Line::Leveled(rels) => { leveled = true; cyclic.extend(rels.iter().copied()); }
            Line::Measured { desc, key, keyed, cols, ranks } => maint.measured = Some(Measured { desc: *desc, key: key.clone(), keyed: *keyed, cols: cols.clone(), ranks: ranks.clone() }),
            Line::KeptOld => kept_old = true,
            Line::KeptRows(_) | Line::Sorted(_) => {}
            Line::Lattice { rel, keys, name, widen } => {
                let lattice = super::lattice(name).map_err(|e| err(format!("lattice {name}: {e}")))?;
                rels.get_mut(*rel).ok_or_else(|| err(format!("lattice: no relation {rel}")))?.set_lattice(name, *keys, lattice, *widen)?;
            }
            Line::Seed { head, join } => {
                let j = { let _j = crate::code_rule_dsl::logic::profile::span(|| "open: joins".into()); join_rule_with(conn, join, &mut syms, !device_wanted)? };
                if device_wanted { seed_specs.push((probes.len(), join.clone())); }
                probes.push(ProbeRule { head: *head, delta: SEED, body: Body::Join(j), whole: None, wkey: None, budget: Default::default(), full: Default::default() });
            }
            Line::Remove { head, join } => {
                if crate::code_rule_dsl::logic::profile::on() {
                    let d: Vec<String> = join.items.iter().take(2).map(|i| format!("{i:?}")).collect();
                    maint.remove_names.push(d.join(" , ").chars().take(200).collect());
                }
                maint.removes.push((*head, joins(join, &mut syms)?))
            }
            Line::AddChecked { head, join } => maint.adds.push((*head, joins(join, &mut syms)?)),
            Line::Recheck { head, join } => {
                // Its head's slots are bound by the candidate before its first step.
                let pre: Vec<usize> = join.items.last().and_then(|i| match i { JoinItem::Head(t) => Some(t), _ => None }).into_iter().flatten()
                    .filter_map(|t| match t { spec::HeadTok::Tok(Tok::Slot(s)) => Some(*s), _ => None }).collect();
                let _j = crate::code_rule_dsl::logic::profile::span(|| "open: joins".into());
                maint.rechecks.push((*head, join_rule_bound(conn, join, &mut syms, true, &pre)?));
            }
            Line::NoWrite => write = false,
            Line::Request(r) => requests.push((*r, 0)),
            Line::Produced(r) => produced.push(*r),
            Line::Stats(r) => counted.push(*r),
            Line::Gate { done, src, quiet, after } => gates.push(Gate { done: *done, src: *src, quiet: quiet.clone(), after: after.clone(), passed: 0 }),
            Line::Join { head, delta, join, whole } => {
                let w = match whole { Some(w) => Some(joins(w, &mut syms)?), None => None };
                if device_wanted { delta_specs.push((probes.len(), join.to_text())); }
                probes.push(ProbeRule { head: *head, delta: *delta, body: Body::Join(joins(join, &mut syms)?), whole: w, wkey: whole.clone(), budget: Default::default(), full: Default::default() });
            }
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
    // A recursive component in the maintain mode (decision 0364 §2): each delta probe is also a driver, its delta atom
    // taken out and its slots bound, to find the rows that read a row gone.
    if !maint.removes.is_empty() || !maint.adds.is_empty() {
        // A probe's index: the lines that make probes, in spec order, as the loop above pushed them.
        let mut probe = 0;
        for line in &spec.lines {
            let at_probe = probe;
            if matches!(line, Line::Probe { .. } | Line::Unplanned { .. } | Line::Seed { .. } | Line::Join { .. }) {
                probe += 1;
            }
            let Line::Join { head, delta, join, .. } = line else { continue };
            let Some(at) = join.items.iter().position(|i| matches!(i, JoinItem::Atom { neg: false, src: spec::Source::Delta(d), .. } if d == delta)) else { continue };
            let JoinItem::Atom { args, .. } = &join.items[at] else { unreachable!() };
            let pre: Vec<usize> = args.iter().filter_map(|a| match a { Arg::Tok(Tok::Slot(s)) => Some(*s), _ => None }).collect();
            let mut rest = (**join).clone();
            rest.items.remove(at);
            if at < rest.from.len() {
                rest.from.remove(at);
            }
            let j = join_rule_bound(conn, &rest, &mut syms, true, &pre)?;
            maint.drivers.push((*delta, *head, j, args.clone(), at_probe));
        }
        maint.leveled = spec.lines.iter().any(|l| matches!(l, Line::Join { .. }));
    }
    // The device's plans of the seeds, or why it takes none of them: then they are parsed again with the CPU's indexes.
    let device_plans = if let Some(dev) = solver().filter(|_| device_wanted) {
        let _plan = crate::code_rule_dsl::logic::profile::span(|| "open: device plans".into());
        let recursive = probes.iter().any(|p| p.delta != SEED);
        let refused = if !edges.is_empty() || !gates.is_empty() || !requests.is_empty() || !produced.is_empty() {
            Some("a component with gates, requests or edge rules".to_owned())
        } else if let Some(why) = rels.iter().filter_map(Rel::device_lattice).find_map(std::result::Result::err).filter(|_| recursive) {
            // A recursive one folds each round on the device (`device::fixpoint`); a lattice it has no fold for
            // stays on the CPU. Seeds alone fold as the store takes the rows.
            Some(format!("a recursive component over {why}"))
        } else if recursive && probes.iter().any(|p| p.delta != SEED && matches!(p.body, Body::Sql(_))) {
            Some("a recursive component with a SQL probe".to_owned())
        } else {
            None
        };
        // A shortlex_min relation's value column holds the texts its rules make (decision 0307): read as texts.
        let own_texts: Vec<(usize, usize)> = rels.iter().enumerate().filter(|(_, r)| matches!(r.device_lattice(), Some(Ok((3, ..))))).map(|(i, r)| (i, r.k - 1)).collect();
        let plans = match refused {
            Some(r) => Err(r),
            None => seed_specs.iter().map(|(pi, js)| (*pi, js.to_text())).chain(delta_specs.iter().cloned()).map(|(pi, text)| match &probes[pi].body {
                Body::Join(j) => dev.compile_join(j, &text, &own_texts).map(|p| (pi, p)),
                Body::Sql(_) => Err("a SQL seed".to_owned()),
                Body::Unplanned => Err("a probe not planned yet".to_owned()),
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
    // cycle of the component's rules (a recursive stratum's), or one started from a kept
    // relation with levels. The others hold none.
    {
        let n = rels.len();
        let mut next: Vec<Vec<usize>> = vec![Vec::new(); n];
        // A seed probe (`SEED`) reads no relation of the component.
        for p in probes.iter().filter(|p| p.delta < n) { next[p.delta].push(p.head); }
        for e in edges.iter().filter(|e| e.src < n) { next[e.src].push(e.head); }
        let mut keep = vec![false; n];
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
            if !rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var_os("RERAIL_LEVELS_ON_CYCLES").is_some()) { keep[r] = true; continue; }
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
    for &r in &ordered {
        if let Some(rel) = rels.get_mut(r) {
            rel.ordered = true;
        }
    }
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
    for p in &probes {
        if let Body::Join(j) = &p.body { for (r, cols) in &j.multi { rels[*r].want(cols); } }
        if let Some(j) = &p.whole { for (r, cols) in &j.multi { rels[*r].want(cols); } }
    }
    // The maintain mode's joins probe the component's relations too (decision 0364): a recheck by its head's
    // values, a level's readers by the row gone. Without their orders, each probe read every row (t43, step 1).
    let mode_joins = maint.removes.iter().chain(&maint.adds).chain(&maint.rechecks).map(|(_, j)| j).chain(maint.drivers.iter().map(|d| &d.2));
    for j in mode_joins {
        for (r, cols) in &j.multi { rels[*r].want(cols); }
    }
    syms.check()?;
    drop(syms);
    let store = Arc::new(RwLock::new(Store { rels, syms: symbols }));
    forgotten_on_close();
    let on = on(&conn);
    let mut reg = on.components.lock();
    let key = name.clone();
    let mut comp = Component { store, edges, probes, gates, seeded: false, write, done: false, requests, produced, closure: None, waiting: None, device_wait: None,
                               device: device_plans, seed_specs, deferred: Vec::new(), presenting: false, cut: None, last_cut: None, trial_rows: None, maint };
    // A stratum placed on the device runs there whole: a recursive one as the device's rounds (`fixpoint`), not as
    // a closure (decision 0098 §1's form, for a stratum on the CPU).
    let on_device_rounds = match (solver(), comp.device.as_ref()) {
        (Some(dev), Some(d)) => d.iter().any(|(_, p)| dev.reads_own(p)),
        _ => false,
    };
    // The maintain mode's component neither: its first wave's delta is the rows its removals put back.
    if !on_device_rounds && comp.maint.drivers.is_empty() {
        comp.closure = {
            let _h = crate::code_rule_dsl::logic::lanes::prof::host("closure", "plan: recognize the form");
            closure::recognize(&comp)
        };
    }
    reg.insert(key, Arc::new(Mutex::new(comp)));
    Ok(name)
}

/// At the fixpoint the component's relations move to `DERIVED`, the input of later strata (`r:` sources), with the
/// hash indexes the waves kept (`InRel::chains`): nothing is copied or indexed again.
pub fn close(conn: impl Scoped, name: &str) -> Result<()> {
    let conn = conn.db();
    let on = on(&conn);
    forgotten_on_close();
    let mut reg = on.components.lock();
    let Some(comp) = reg.remove(name) else { return Ok(()) };
    drop(reg);
    let comp = comp.lock().unwrap();
    if comp.done {
        let rels = std::mem::take(&mut comp.store.write().unwrap().rels);
        forgotten_on_close();
        let mut derived = on.derived.lock();
        for rel in rels {
            let moved = InRel::moved(rel);
            let table = moved.name.get().cloned().unwrap_or_default();
            derived.insert(table, Arc::new(moved));
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
    // The run's limits (decision 0462): its cap, whether it cuts, its memory stop, and its record.
    let limits = super::width::Limits::here();
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
    for p in &comp.probes { stmts.push(match &p.body { Body::Sql(sql) => Some(conn.sql()?.prepare(sql)?), Body::Join(_) | Body::Unplanned => None }); }
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
    let (held_back, mut present) = (!comp.deferred.is_empty(), false);
    let gates_done: Vec<usize> = gates.iter().map(|g| g.done).collect();
    // The closure form (decision 0098 §1): seeds once, then the walk's reach over distinct nodes, in two halves around
    // its launches. When the caller can wait (`lanes::wave::can_wait`: the stratum driver, which runs a level's
    // strata to their launches first), the launches join this thread's device wave and the run returns `3`; the run
    // after the wave ran finishes it. Otherwise they run here, as a batch of their own.
    let mut closure_ran: Option<Option<closure::Ran>> = None;
    // The maintain mode's removals and checked adds (decision 0364 stage 3), before the seeds add the rest.
    if !seeded && comp.cut.is_none() && !comp.maint.done && !(comp.maint.removes.is_empty() && comp.maint.adds.is_empty()) {
        let _m = crate::code_rule_dsl::logic::profile::span(|| "maintain mode: removals and rechecks".into());
        let mut maint = std::mem::take(&mut comp.maint);
        let store = Arc::clone(&comp.store);
        let mut guard = store.write().unwrap();
        let ran = maint.run(&mut guard, &mut cursor, conn.sql().ok());
        drop(guard);
        crate::code_rule_dsl::logic::profile::decision("the maintain mode (decision 0364)", || format!("{name}: {} candidates, {} killed, {} checked adds added", maint.counts[0], maint.counts[1], maint.counts[2]));
        comp.maint = maint;
        ran?;
    }
    let tp = Instant::now();
    if let Some((cl, prep, tickets)) = comp.waiting.take() {
        let outs = crate::code_rule_dsl::logic::lanes::wave::take(&tickets).map_err(|e| err(format!("{name}: {e}")))?;
        let mut guard = comp.store.write().unwrap();
        closure_ran = Some(closure::finish(&cl, prep, outs, &mut guard));
        drop(guard);
        comp.closure = Some(cl);
    } else if !seeded && comp.cut.is_none() && let Some(cl) = comp.closure.take() {
        let mut guard = comp.store.write().unwrap();
        let store = &mut *guard;
        let mut syms = &mut cursor;
        let mut fresh: Vec<Fresh> = vec![Fresh::default(); nrels];
        for (pi, p) in comp.probes.iter().enumerate() {
            let (Body::Join(j), SEED) = (&p.body, p.delta) else { continue };
            runs += 1;
            let n = run_seed(j, p.head, store, &mut syms, &mut fresh, None)?;
            visits += n;
            per[pi].1 += n;
        }
        let k = store.rels[0].k;
        store.rels[0].reserve(fresh[0].n);
        for i in 0..fresh[0].n { store.rels[0].push(&fresh[0].data.row(i as u32, k)); }
        let prep = closure::prepare(&cl, store, &mut syms);
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
    if !seeded && comp.cut.is_none() && comp.device_wait.is_none() && crate::code_rule_dsl::logic::lanes::wave::can_wait()
        && let Some(dev) = solver()
        && let Some(plans) = comp.device.as_ref()
        && !plans.iter().any(|(_, p)| dev.reads_own(p))
        && std::env::var("RERAIL_DEVICE_WAVES").map_or(true, |v| v != "off")
    {
        let rules: Vec<(&JoinPlan, &JoinRule, usize)> = plans.iter().map(|(pi, plan)| match &comp.probes[*pi].body {
            Body::Join(j) => (plan, j, comp.probes[*pi].head),
            Body::Sql(_) | Body::Unplanned => unreachable!("a device plan is a join's"),
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
    // A probe not planned yet whose delta has rows (decision 0123 §9): the run returns for its plan before the wave
    // that would read them (`done 4`), and goes on from there when run again.
    let mut unplanned = false;
    // A wave cut at a probe that spent its budget (decision 0206): the run returns `5`, and the next goes on there.
    let mut was_cut = false;
    while !done {
        if comp.probes.iter().any(|p| matches!(p.body, Body::Unplanned)) {
            let s = comp.store.read().unwrap();
            if comp.probes.iter().any(|p| matches!(p.body, Body::Unplanned) && s.rels[p.delta].lo < s.rels[p.delta].hi) {
                unplanned = true;
                break;
            }
        }
        // A cut wave goes on (decision 0206): its rows so far, from the probe it was cut at, which reads only the
        // delta rows it had left (with its body as it is now, the driver's replacement). The probes before it, the
        // SQL ones and the device's, ran already.
        let resume = comp.cut.take();
        let resuming = resume.is_some();
        let start = resume.as_ref().map_or(0, |c| c.next);
        let resumed_on_device = resume.as_ref().is_some_and(|c| c.on_device);
        let mut resume_left = resume.as_ref().and_then(|c| c.left.clone().map(|l| (c.step, l)));
        let mut fresh: Vec<Fresh> = match resume { Some(c) => c.fresh, None => vec![Fresh::default(); nrels] };
        // Probe rules: SQL over the vtabs, while no lock is held (the vtab cursors take read locks).
        let mut raw: Vec<(usize, Vec<Vec<Value>>)> = Vec::new();
        for (pi, (p, st)) in comp.probes.iter().zip(stmts.iter_mut()).enumerate() {
            let Some(st) = st else { continue };
            if resuming { continue; }
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
        let tried = !resuming && !seeded && comp.device.is_some() && dev.is_some();
        // The component's fixpoint ran on the device (a recursive one): its rows are all of them, and it is done.
        let mut device_fixed = false;
        let on_device = resumed_on_device || tried && {
            let dev = dev.expect("a device");
            let plans = comp.device.as_ref().expect("device plans");
            let tp = Instant::now();
            let c0 = cpu_now();
            let rules: Vec<(&JoinPlan, &JoinRule, usize)> = plans.iter().map(|(pi, plan)| match &comp.probes[*pi].body {
                Body::Join(j) => (plan, j, comp.probes[*pi].head),
                Body::Sql(_) | Body::Unplanned => unreachable!("a device plan is a join's"),
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
                    for (r, (f, d)) in fresh.iter_mut().zip(dev_fresh).enumerate() {
                        // Rows with their rounds (a relation that keeps levels, decision 0311): each at its level.
                        // copy (0353 step 6): the device's rows staged (decision 0353).
                        const DEVICE_STAGED: rerail_prof::copies::Site = rerail_prof::copies::Site::new(8, "6", "the device's head rows staged in Fresh (setfix)");
                        DEVICE_STAGED.copied(8 * d.data.len());
                        if !d.levels.is_empty() {
                            let rel = &mut store.rels[r];
                            let k = rel.k;
                            for (i, &round) in d.levels.iter().enumerate() {
                                rel.push_leveled(&d.data[i * k..(i + 1) * k], round);
                            }
                            continue;
                        }
// copy (0353 step 6): the device's rows staged (decision 0353), counted above.
                        f.data.extend_from_slice(&d.data);
                        f.n += d.n;
                        // The device's rows, in no order known here.
                        f.lost |= d.n > 0;
                        // Its rows written in place (decision 0325 step 4), and a lattice's key map they were folded
                        // through (decision 0384 step 5).
                        f.placed.extend(d.placed);
                        if d.map.is_some() {
                            debug_assert!(f.map.is_none(), "two folds' maps for one relation");
                            f.map = d.map;
                        }
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
        // body and different heads (a magic relation and its reader's, decision 0145's demand) are two runs.
        let mut wholes_run: Vec<(usize, &Arc<JoinSpec>)> = Vec::new();
        let mut cut_at: Option<(usize, Option<Rows0>, CutReport)> = None;
        for (pi, p) in comp.probes.iter().enumerate() {
            if cut_at.is_some() { break; }
            let Body::Join(j) = &p.body else { continue };
            if p.full.get() { continue; }
            let tp = Instant::now();
            if device_fixed { break; }
            if p.delta == SEED { if seeded || on_device { continue } }
            else if store.rels[p.delta].lo == store.rels[p.delta].hi { skips += 1; continue; }
            let (j, whole) = match (&p.whole, &p.wkey) {
                (Some(w), Some(k)) if p.delta != SEED && store.rels[p.delta].lo == 0 => {
                    if wholes_run.iter().any(|&(h, w)| h == p.head && (Arc::ptr_eq(w, k) || **w == **k)) { skips += 1; continue; }
                    wholes_run.push((p.head, k));
                    (w, true)
                }
                _ => (j, false),
            };
            if pi < start { continue; }
            runs += 1;
            if wave_trace {
                let d = if p.delta == SEED { 0 } else { store.rels[p.delta].hi - store.rels[p.delta].lo };
                eprintln!("      wave {waves} probe {pi} of {name} starts, head {}, delta {} {d} (lo {}){}", store.rels[p.head].table, store.rels[p.delta.min(store.rels.len() - 1)].table, if p.delta == SEED { 0 } else { store.rels[p.delta].lo }, if whole { ", its whole plan" } else if p.whole.is_some() { ", has a whole plan" } else { "" });
            }
            let c0 = cpu_now();
            // Decision 0206: a probe's budget (a seed's, a delta probe's or its whole plan's, O168), and the rows it
            // has left after a cut.
            let budget = p.budget.get();
            let only = if pi == start { resume_left.take().and_then(|(named, l)| match restricted(j, l.of, named) {
                Some(i) => { run_counts().cuts[1].fetch_add(1, std::sync::atomic::Ordering::Relaxed); Some((i, l)) }
                // At a step no longer known: the run reads all its rows.
                None => { run_counts().cuts[2].fetch_add(1, std::sync::atomic::Ordering::Relaxed); None }
            }) } else { None };
            let mut limited = (budget.is_some() || only.is_some()).then(|| Limited { limit: Some(Limit { budget: budget.unwrap_or(u64::MAX), only }), spent: None });
            let ran = if p.delta == SEED { run_seed(j, p.head, store, &mut syms, &mut fresh, limited.as_mut()) } else { run_join_limited(j, p.head, store, &mut syms, &mut fresh, limited.as_mut()) };
            // Decision 0462: a head at the width cap. Cut, the probe keeps what it made and derives no more; else the
            // run stops, its error naming the relation and the probe (the driver adds its rule).
            let n = match ran {
                Ok(n) => n,
                Err(e) if super::width::as_too_wide(&e).is_some() => {
                    if !limits.cutting() { return Err(super::width::with_rules(e, |t| vec![format!("probe {pi} of {name} (head `{t}`)")])); }
                    p.full.set(true);
                    limits.note_cut(&store.rels[p.head].table, &name, pi);
                    0
                }
                Err(e) => return Err(e),
            };
            if let Some(b) = budget && let Some(spent) = limited.and_then(|l| l.spent) {
                p.budget.set(Some(b.saturating_sub(spent.work)));
                if let Some(left) = spent.left {
                    p.budget.set(None);
                    run_counts().cuts[0].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    cut_at = Some((pi, left, CutReport { probe: pi, work: spent.work, done: spent.done, left: 0, led: spent.led, whole }));
                }
            }
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
        if let Some((next, left, mut report)) = cut_at {
            report.left = left.as_ref().map_or(0, |l| l.n);
            drop(guard);
            // Its body as it is, the rows left are its first step's; a reprobe forgets that (`cut_step`).
            let whole = report.whole;
            comp.cut = Some(Cut { fresh, next, left, step: Some(0), on_device, whole });
            comp.last_cut = Some(report);
            comp.trial_rows = None;
            was_cut = true;
            break;
        }
        // Edge rules, natively.
        let (mut slots, mut bound, mut key): (Vec<i64>, Vec<bool>, Vec<i64>) = (Vec::new(), Vec::new(), Vec::new());
        // Decision 0462: an edge rule's head at the width cap takes no more rows; the run stops after the edges, or
        // goes on with the relation cut (`RERAIL_WIDEST=cut`).
        let mut edge_full: Option<usize> = None;
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
                    if fresh[rule.head].n + store.rels[rule.head].n >= limits.cap { edge_full = Some(rule.head); return; }
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
        if let Some(h) = edge_full {
            let table = &store.rels[h].table;
            if !limits.cutting() { return Err(super::width::with_rules(super::width::too_wide(table, limits.cap), |t| vec![format!("an edge rule of {name} (head `{t}`)")])); }
            limits.note_cut_by(table, format!("an edge rule of {name}"));
        }
        // The run's memory watch said stop (decision 0462): the wave's rows are dropped and the run stops.
        if let Some(e) = limits.stop_error() {
            return Err(e);
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
            let big = f.n >= SPANNED_ROWS;
            any |= { let _s = big.then(|| crate::code_rule_dsl::logic::profile::span(|| "a wave's rows settled (Fresh::settle)".into())); rel.push_fresh(f, gates_done.contains(&r)) };
            limits.note_rows(&rel.table, rel.n);
            if counting && emitted > 0 {
                crate::code_rule_dsl::logic::run::with_local(|all: &Derivations| {
                    let mut all = all.0.lock().unwrap();
                    let e = all.entry(rel.table.clone()).or_default();
                    e.0 += emitted;
                    e.1 += (rel.n - lo) as u64;
                });
            }
            lo_of[r] = lo;
            if !gates_done.contains(&r) { let hi = rel.n; let _s = (big && hi > lo).then(|| crate::code_rule_dsl::logic::profile::span(|| "a wave's rows indexed (set_delta)".into())); rel.set_delta(lo, hi); }
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
    // A symbol a wave could not intern fails the run.
    cursor.check()?;
    symbols.write().unwrap().check()?;
    let solve = t0.elapsed();
    let per: Vec<String> = per.iter().map(|(t, n, c)| format!("{:.1}:{n}:{:.3}", t * 1e3, c * 1e3)).collect();
    if was_cut {
        return Ok(format!("0 {waves} {visits} {runs} {skips} {:.1} {:.1} {} 5", solve.as_secs_f64() * 1e3, t0.elapsed().as_secs_f64() * 1e3, per.join(",")));
    }
    if paused {
        return Ok(format!("0 {waves} {visits} {runs} {skips} {:.1} {:.1} {} 2", solve.as_secs_f64() * 1e3, t0.elapsed().as_secs_f64() * 1e3, per.join(",")));
    }
    if unplanned {
        return Ok(format!("0 {waves} {visits} {runs} {skips} {:.1} {:.1} {} 4", solve.as_secs_f64() * 1e3, t0.elapsed().as_secs_f64() * 1e3, per.join(",")));
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
                conn.sql()?.execute(&format!("INSERT INTO {} VALUES {holes}", crate::code_rule_dsl::logic::sql::table(&rel.table)), rusqlite::params_from_iter(rel.data.to_vec(id, end, rel.k).iter()))?;
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

/// Holds back request rows the caller does not answer yet (a load past the budget's current level, decision 0145):
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
        if probed::on() { probed::note(&format!("{} [stats]", r.table), std::iter::empty()); }
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
        let body = match &probe.body { Body::Join(j) => Some((format!("probe{p}"), j)), Body::Sql(_) | Body::Unplanned => None };
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
    pub right_ids: Vec<u32>,
    /// The right side's identity: its relation, key columns and column tests (one sort serves every case of it).
    pub right_key: String,
    /// Whether each key holds symbols.
    pub symbols: Vec<bool>,
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
        let body = match &probe.body { Body::Join(j) => Some((format!("probe{p}"), j)), Body::Sql(_) | Body::Unplanned => None };
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
        Body::Join(j) => {
            let c: Vec<(u64, u64)> = j.counts.iter().map(|[a, b]| (a.replace(0), b.replace(0))).collect();
            j.plan_counts(&c).into_iter().map(|(a, b)| (a as i64, b as i64)).collect()
        }
        Body::Sql(_) | Body::Unplanned => Vec::new(),
    }).collect())
}

/// The maintain mode's lines of component `name` (decision 0364 stage 3): its removal joins, its checked adds and its
/// rechecks, each in spec order, with each one's steps' (in, out) binding counts by plan step since the last call (as
/// `step_counts`'s); the counts restart. Their work, for the next run's planning budget (Q208).
pub fn maint_step_counts(conn: impl Scoped, name: &str) -> Result<[Vec<Vec<(i64, i64)>>; 3]> {
    let conn = conn.db();
    let comp = component(conn, name)?;
    let comp = comp.lock().unwrap();
    let of = |js: &[(usize, JoinRule)]| -> Vec<Vec<(i64, i64)>> {
        js.iter().map(|(_, j)| {
            let c: Vec<(u64, u64)> = j.counts.iter().map(|[a, b]| (a.replace(0), b.replace(0))).collect();
            j.plan_counts(&c).into_iter().map(|(a, b)| (a as i64, b as i64)).collect()
        }).collect()
    };
    Ok([of(&comp.maint.removes), of(&comp.maint.adds), of(&comp.maint.rechecks)])
}

/// The drivers of component `name`'s maintain mode (a recursive component's, decision 0364 §2): per driver, the probe
/// it is made from (its index in `step_counts`'s list) and its steps' (in, out) binding counts by its own plan steps
/// since the last call; the counts restart. A driver runs its probe's order less the delta atom, so its work is
/// that probe's kept plan's, for the next run's planning budget (Q208).
pub fn driver_step_counts(conn: impl Scoped, name: &str) -> Result<Vec<(usize, Vec<(i64, i64)>)>> {
    let conn = conn.db();
    let comp = component(conn, name)?;
    let comp = comp.lock().unwrap();
    Ok(comp.maint.drivers.iter().map(|(_, _, j, _, probe)| {
        let c: Vec<(u64, u64)> = j.counts.iter().map(|[a, b]| (a.replace(0), b.replace(0))).collect();
        (*probe, j.plan_counts(&c).into_iter().map(|(a, b)| (a as i64, b as i64)).collect())
    }).collect())
}

/// Profiled: per probe of component `name`, its plan steps' CPU seconds since the last call, and last its head's
/// (its computed terms and rows made: `JoinRule::step_ns`, `plan_cpu`); the clocks restart. Taken when `step_counts` is, so the two cover the same walks.
pub fn step_cpu(conn: impl Scoped, name: &str) -> Result<Vec<Vec<f64>>> {
    let conn = conn.db();
    let comp = component(conn, name)?;
    let comp = comp.lock().unwrap();
    Ok(comp.probes.iter().map(|p| match &p.body {
        Body::Join(j) => {
            let c: Vec<f64> = j.step_ns.iter().map(|c| c.replace(0) as f64 / 1e9).collect();
            j.plan_cpu(&c)
        }
        Body::Sql(_) | Body::Unplanned => Vec::new(),
    }).collect())
}

/// Per probe of component `name`, by plan step (as `step_counts`'s), whether the CPU's probe there reads only the
/// range of its index that later comparisons bound (`ranges`, Q245 item 5): its rows out are then fewer than a
/// probe's that reads every row of its key (the device's), from the same bindings. A probe that is not a join: none.
pub fn step_ranged(conn: impl Scoped, name: &str) -> Result<Vec<Vec<bool>>> {
    let conn = conn.db();
    let comp = component(conn, name)?;
    let comp = comp.lock().unwrap();
    Ok(comp.probes.iter().map(|p| match &p.body {
        Body::Join(j) => {
            let mut by = vec![false; j.plan_steps];
            for (i, r) in j.ranges.iter().enumerate() {
                if let (Some(_), Some(&at)) = (r, j.from.get(i)) && at < by.len() {
                    by[at] = true;
                }
            }
            by
        }
        Body::Sql(_) | Body::Unplanned => Vec::new(),
    }).collect())
}

/// Per probe of component `name`, the first step past the one that binds its head's slots (`tail`): from it on, the
/// CPU's walk counts one witness (decision 0229) and the device's stages every row (decision 0277), so the two
/// counts agree only before it. A probe that is not a join: 0.
pub fn step_tails(conn: impl Scoped, name: &str) -> Result<Vec<usize>> {
    let conn = conn.db();
    let comp = component(conn, name)?;
    let comp = comp.lock().unwrap();
    Ok(comp.probes.iter().map(|p| match &p.body {
        Body::Join(j) => tail(j),
        Body::Sql(_) | Body::Unplanned => 0,
    }).collect())
}

/// One step's histogram (`step_histograms`): its rows past its constants, and their counts by key (`histogram_key`).
pub type Histogram = (u64, rustc_hash::FxHashMap<u64, u64>);

/// A histogram's key for one row's key columns (`values`): the value itself for one column; for several, a 64-bit mix
/// of them (two keys of a million rows meet once in 10^7 histograms), so a key is 8 bytes in place, not a vector of
/// its own (Q74: 0.24 GB of `Vec` keys at a cold run's peak, 300 files). Both sides of a pair key alike.
fn histogram_key(values: impl Iterator<Item = Option<i64>>) -> Option<u64> {
    let mut h: u64 = 0;
    let mut n = 0;
    for v in values {
        let v = v? as u64;
        h = if n == 0 { v } else { (h.rotate_left(5) ^ v).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ (h >> 29) };
        n += 1;
    }
    Some(if n > 1 { h ^ (h >> 32).wrapping_mul(0xbf58_476d_1ce4_e5b9) } else { h })
}

/// Decision 0203 §3: per wanted (step, key columns) of probe `line`'s join, the rows of the step's relation (every
/// live row, not its delta alone) that pass the step's constants, counted by the values of those columns. A relation
/// of more than `cap` rows is read at a stride, its counts scaled. `None` for a step that is not a positive atom.
/// `whole`: the steps are of the line's whole plan (O168), whose order is not its delta join's.
pub fn step_histograms(conn: impl Scoped, name: &str, line: usize, whole: bool, wanted: &[(usize, Vec<usize>)], cap: usize) -> Result<Vec<Option<Histogram>>> {
    let conn = conn.db();
    let comp = component(conn, name)?;
    let comp = comp.lock().unwrap();
    let store = comp.store.read().unwrap();
    let j = comp.probes.get(line).and_then(|p| match (whole, &p.body) {
        (true, _) => p.whole.as_ref(),
        (false, Body::Join(j)) => Some(j),
        (false, _) => None,
    });
    let Some(j) = j else { return Ok(vec![None; wanted.len()]) };
    Ok(wanted.iter().map(|(at, cols)| {
        // `at` is the plan's step: the atom is the step made first for it.
        let Some(Step::Atom { src, neg: false, keys, .. }) = j.step_of(*at).and_then(|s| j.steps.get(s)) else { return None };
        let consts: Vec<(usize, i64)> = keys.iter().filter_map(|&(c, k)| match k { Key::Const(v) => Some((c, v)), Key::Slot(_) => None }).collect();
        let mut counts: rustc_hash::FxHashMap<u64, u64> = rustc_hash::FxHashMap::default();
        let mut passed = 0u64;
        let mut count = |n: usize, value: &dyn Fn(u32, usize) -> Option<i64>| -> u64 {
            let stride = n.div_ceil(cap.max(1)).max(1);
            let mut id = 0usize;
            while id < n {
                let i = id as u32;
                if consts.iter().all(|&(c, v)| value(i, c) == Some(v)) && let Some(key) = histogram_key(cols.iter().map(|&c| value(i, c))) {
                    passed += 1;
                    *counts.entry(key).or_insert(0) += 1;
                }
                id += stride;
            }
            stride as u64
        };
        let stride = match src {
            Src::Delta(r) | Src::Full(r) | Src::FullMulti(r, _) => {
                let rel = &store.rels[*r];
                count(rel.n, &|i, c| (!rel.dead(i)).then(|| rel.value(i as usize, c)))
            }
            Src::In(rel, _) => count(rel.len(), &|i, c| (!is_dead(&rel.dead, i)).then(|| rel.at(i as usize).read().at(c))),
        };
        if stride > 1 {
            counts.values_mut().for_each(|n| *n *= stride);
        }
        Some((passed * stride, counts))
    }).collect())
}

/// A trial of an order (decision 0207): its work over the sample, the sample's rows, the rows it made, and whether it
/// passed its budget (its work is then a lower bound). `done`: the sample's rows finished before a cut (all of them
/// where it was not cut).
#[derive(Clone, Debug, PartialEq)]
pub struct TrialRun {
    pub work: u64,
    pub rows: u64,
    pub done: u64,
    pub made: u64,
    pub cut: bool,
    /// Each step's rows in and out over the trial (decision 0354: an order the planner did not keep, as samples).
    pub steps: Vec<(u64, u64)>,
}

/// Decision 0207: runs probe `probe`'s body, or `spec` in its place, over a sample of at most `sample` rows of its
/// first atom (the rows its cut left, where it was cut; else every row its first step reads), under `budget` steps.
/// Nothing is added to any relation: the rows made are counted and dropped. The probe's own counts are as they were.
/// `step`: where `spec` reads those rows (it leads with the same atom); else where known (`restricted`). `None` when
/// the sample cannot be placed in `spec`, or the probe is no join.
pub fn trial(conn: impl Scoped, name: &str, probe: usize, spec: Option<&JoinSpec>, step: Option<usize>, sample: usize, budget: u64) -> Result<Option<TrialRun>> {
    trial_over(conn, name, probe, spec, step, Err(sample), budget)
}

/// The rows `trial` draws its sample from, for probe `probe`: the rows its cut left, else every row its first step
/// reads, as row ids of the relation they live in (decision 0209's split spike picks among them).
pub fn trial_ids(conn: impl Scoped, name: &str, probe: usize) -> Result<Option<Vec<u32>>> {
    let conn = conn.db();
    let comp = component(conn, name)?;
    let comp = comp.lock().unwrap();
    let cut = comp.cut.as_ref().filter(|c| c.next == probe);
    if let Some(left) = cut.and_then(|c| c.left.as_ref()) {
        return Ok(Some((0..left.bits.len() * 64).filter(|&b| left.bits[b / 64] >> (b % 64) & 1 == 1).map(|b| left.lo + b as u32).collect()));
    }
    let Some(p) = comp.probes.get(probe) else { return Ok(None) };
    let Body::Join(own) = &p.body else { return Ok(None) };
    let store = comp.store.read().unwrap();
    let Some(Step::Atom { src, .. }) = own.steps.first() else { return Ok(None) };
    Ok(Some(match src {
        Src::Delta(r) => (store.rels[*r].lo as u32..store.rels[*r].hi as u32).collect(),
        Src::Full(r) | Src::FullMulti(r, _) => (0..store.rels[*r].n as u32).collect(),
        Src::In(r, _) => (0..r.len() as u32).collect(),
    }))
}

/// `trial` over the rows `pick` names (`Ok`, among those `trial_ids` gives), or a sample of at most that many at a
/// stride (`Err`).
pub fn trial_over(conn: impl Scoped, name: &str, probe: usize, spec: Option<&JoinSpec>, step: Option<usize>, pick: std::result::Result<&[u32], usize>, budget: u64) -> Result<Option<TrialRun>> {
    let conn = conn.db();
    let comp = component(conn, name)?;
    let mut comp = comp.lock().unwrap();
    let symbols = Arc::clone(&comp.store.read().unwrap().syms);
    let mut syms = Interner::for_loading(&symbols);
    let made = match spec {
        Some(spec) => {
            let j = join_rule(conn, spec, &mut syms)?;
            let mut store = comp.store.write().unwrap();
            for (r, cols) in &j.multi { store.rels[*r].want(cols); }
            Some(j)
        }
        None => None,
    };
    let cut = comp.cut.as_ref().filter(|c| c.next == probe);
    let from_cut = cut.and_then(|c| c.left.as_ref()).is_some();
    let Some(p) = comp.probes.get(probe) else { return Ok(None) };
    // The probe's own body: its whole plan where the cut stopped that (O168).
    let own = match (&p.body, &p.whole, cut.is_some_and(|c| c.whole)) {
        (_, Some(w), true) => w,
        (Body::Join(j), _, _) => j,
        _ => return Ok(None),
    };
    let head = p.head;
    let store = comp.store.read().unwrap();
    // The rows: those the cut left, else all the first step reads; a sample at a stride.
    let (of, ids): (RowsOf, Vec<u32>) = match cut.and_then(|c| c.left.as_ref()) {
        Some(left) => (left.of, (0..left.bits.len() * 64).filter(|&b| left.bits[b / 64] >> (b % 64) & 1 == 1).map(|b| left.lo + b as u32).collect()),
        None => {
            let Some(Step::Atom { src, .. }) = own.steps.first() else { return Ok(None) };
            let range = match src {
                Src::Delta(r) => store.rels[*r].lo as u32..store.rels[*r].hi as u32,
                Src::Full(r) | Src::FullMulti(r, _) => 0..store.rels[*r].n as u32,
                Src::In(r, _) => 0..r.len() as u32,
            };
            (RowsOf::of(src), range.collect())
        }
    };
    if ids.is_empty() { return Ok(None) }
    let picked: Vec<u32> = match pick {
        Ok(only) => { let only: std::collections::HashSet<u32> = only.iter().copied().collect(); ids.iter().copied().filter(|i| only.contains(i)).collect() }
        Err(sample) => { let stride = ids.len().div_ceil(sample.max(1)).max(1); ids.iter().step_by(stride).copied().collect() }
    };
    if picked.is_empty() { return Ok(None) }
    let rows = Rows0::of(of, &picked);
    let j = made.as_ref().unwrap_or(own);
    let named = if made.is_some() { step } else { Some(0) };
    let Some(at) = restricted(j, of, named) else { return Ok(None) };
    scalars::register(&j.steps, Some(&store));
    let counts: Vec<[u64; 2]> = j.counts.iter().map(|[a, b]| [a.get(), b.get()]).collect();
    let step_ns: Vec<u64> = j.step_ns.iter().map(Count::get).collect();
    let mut out = Vec::new();
    let n = rows.n;
    let kept = rows.clone();
    let mut limited = Limited { limit: Some(Limit { budget, only: Some((at, rows)) }), spent: None };
    let got = run_join_in_each(j, 0, &store, &mut syms, &mut [], &[], Some(&mut out), None, None, Some(&mut limited));
    let spent = limited.spent;
    let steps: Vec<(u64, u64)> = j.counts.iter().zip(&counts).map(|(c, [a, b])| (c[0].get().saturating_sub(*a), c[1].get().saturating_sub(*b))).collect();
    for (c, [a, b]) in j.counts.iter().zip(counts) { c[0].set(a); c[1].set(b); }
    for (c, t) in j.step_ns.iter().zip(step_ns) { c.set(t); }
    let made_rows = got?;
    syms.check()?;
    drop(store);
    run_counts().cuts[3].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let run = spent.map(|s| TrialRun { work: s.work, rows: n, done: if s.left.is_some() { s.done.min(n) } else { n }, made: made_rows, cut: s.left.is_some(), steps });
    // O169: the first trial to finish over the rows a cut left keeps its rows, to be the wave's (`keep_trial_rows`).
    if from_cut && run.as_ref().is_some_and(|r| !r.cut) && comp.trial_rows.as_ref().is_none_or(|t| t.0 != probe) {
        comp.trial_rows = Some((probe, kept, out));
        let _ = head;
    }
    Ok(run)
}

/// Decision 0245: runs `spec` over a sample of its own first step's rows, at a stride, under `budget` steps: an
/// order that leads with another atom than the cut's cannot read the rows the cut left (`restricted`), and its work
/// over the line is its work over the sample scaled by the lead's rows. `size` gives the sample's rows for the
/// lead's. Nothing is added to any relation, and its rows are not kept (O169 keeps a trial's over the cut's rows).
/// With the lead's rows, and the ids of those sampled (a walk over them scales the trial, decision 0233). `None`
/// where its first step reads no atom's rows.
pub fn trial_own(conn: impl Scoped, name: &str, spec: &JoinSpec, size: &dyn Fn(usize) -> usize, budget: u64) -> Result<Option<(TrialRun, usize, Vec<u32>)>> {
    let conn = conn.db();
    let comp = component(conn, name)?;
    let comp = comp.lock().unwrap();
    let symbols = Arc::clone(&comp.store.read().unwrap().syms);
    let mut syms = Interner::for_loading(&symbols);
    let j = join_rule(conn, spec, &mut syms)?;
    {
        let mut store = comp.store.write().unwrap();
        for (r, cols) in &j.multi { store.rels[*r].want(cols); }
    }
    let store = comp.store.read().unwrap();
    let Some(Step::Atom { src, neg: false, .. }) = j.steps.first() else { return Ok(None) };
    let range = match src {
        Src::Delta(r) => store.rels[*r].lo as u32..store.rels[*r].hi as u32,
        Src::Full(r) | Src::FullMulti(r, _) => 0..store.rels[*r].n as u32,
        Src::In(r, _) => 0..r.len() as u32,
    };
    let all = range.len();
    if all == 0 { return Ok(None) }
    let stride = all.div_ceil(size(all).max(1)).max(1);
    let picked: Vec<u32> = range.step_by(stride).collect();
    let rows = Rows0::of(RowsOf::of(src), &picked);
    let n = rows.n;
    scalars::register(&j.steps, Some(&store));
    // The rows made are collected and dropped: a join emits to its head's fresh rows otherwise.
    let mut out = Vec::new();
    let mut limited = Limited { limit: Some(Limit { budget, only: Some((0, rows)) }), spent: None };
    let got = run_join_in_each(&j, 0, &store, &mut syms, &mut [], &[], Some(&mut out), None, None, Some(&mut limited));
    drop(out);
    let spent = limited.spent;
    let steps: Vec<(u64, u64)> = j.counts.iter().map(|[a, b]| (a.get(), b.get())).collect();
    let made = got?;
    syms.check()?;
    run_counts().cuts[3].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Ok(spent.map(|s| (TrialRun { work: s.work, rows: n, done: if s.left.is_some() { s.done.min(n) } else { n }, made, cut: s.left.is_some(), steps }, all, picked)))
}

/// O169: the rows the cut probe's first finished trial made join its wave, and its sample's rows leave the rows the
/// probe has left: the order that goes on does not make them again. The rows kept.
pub fn keep_trial_rows(conn: impl Scoped, name: &str) -> Result<u64> {
    let comp = component(conn.db(), name)?;
    let mut comp = comp.lock().unwrap();
    let Some((probe, sample, rows)) = comp.trial_rows.take() else { return Ok(0) };
    let head = comp.probes[probe].head;
    let store = Arc::clone(&comp.store);
    let store = store.read().unwrap();
    let Some(cut) = comp.cut.as_mut().filter(|c| c.next == probe) else { return Ok(0) };
    let Some(left) = cut.left.as_mut().filter(|l| l.of == sample.of) else { return Ok(0) };
    let k = store.rels[head].k.max(1);
    for row in rows.chunks_exact(k) {
        cut.fresh[head].emitted += 1;
        cut.fresh[head].offer(row, &store.rels[head]);
    }
    for (w, b) in sample.bits.iter().enumerate() {
        let mut m = *b;
        while m != 0 {
            let id = sample.lo + (64 * w) as u32 + m.trailing_zeros();
            m &= m - 1;
            let at = id.wrapping_sub(left.lo) as usize;
            if let Some(word) = left.bits.get_mut(at / 64) && *word >> (at % 64) & 1 == 1 {
                *word &= !(1u64 << (at % 64));
                left.n -= 1;
            }
        }
    }
    Ok((rows.len() / k) as u64)
}

/// A relation's live statistics (`stats`): rows, then per column its distinct values and sum of squared postings,
/// and each value's postings when a bounded exact histogram is available.
pub struct Live { pub n: i64, pub delta: i64, pub ndv: Vec<i64>, pub sq: Vec<i64>, pub freq: Vec<(usize, Vec<(i64, i64)>)> }
pub fn live(conn: impl Scoped, name: &str) -> Result<Vec<Live>> {
    let conn = conn.db();
    let comp = component(conn, name)?;
    let comp = comp.lock().unwrap();
    let store = comp.store.read().unwrap();
    Ok(store.rels.iter().map(live_of).collect())
}

/// `live`, of only the relations whose rows moved since the caller last read them (`seen`, its last reading of
/// each: rows, delta's start and end); `None` for the others. A relation's statistics are its rows' count, its delta
/// and its sketches, and the sketches change only as rows are pushed: equal readings, equal statistics. A wave of a
/// component of 220 relations moves a few of them.
pub fn live_changed(conn: impl Scoped, name: &str, seen: &mut Vec<[usize; 3]>) -> Result<Vec<Option<Live>>> {
    let conn = conn.db();
    let comp = component(conn, name)?;
    let comp = comp.lock().unwrap();
    let store = comp.store.read().unwrap();
    seen.resize(store.rels.len(), [usize::MAX; 3]);
    Ok(store.rels.iter().zip(seen.iter_mut()).map(|(r, last)| {
        let now = [r.n, r.lo, r.hi];
        (*last != now).then(|| { *last = now; live_of(r) })
    }).collect())
}

fn live_of(r: &Rel) -> Live {
    if probed::on() { probed::note(&format!("{} [stats]", r.table), std::iter::empty()); }
    live_from(r.n, r.hi - r.lo, &r.sketches)
}

/// `n` rows' live statistics, `delta` of them the last wave's, from their columns' sketches.
fn live_from(n: usize, delta: usize, cols: &[Sketch]) -> Live {
    let freq = cols.iter().enumerate().filter_map(|(c, sk)| sk.exact_counts().map(|s| (c, s.iter().map(|&(v, n)| (v, i64::from(n))).collect())))
        .collect();
    Live {
        n: n as i64,
        delta: delta as i64,
        ndv: cols.iter().map(|sk| sk.ndv() as i64).collect(),
        sq: cols.iter().map(|sk| sk.sq() as i64).collect(),
        freq,
    }
}

/// Per probe, the rows its factored steps passed over since it was last asked (`JoinRule::passed_over`).
pub fn passed_over(conn: &Connection, name: &str) -> Result<Vec<u64>> {
    let comp = component(conn, name)?;
    let comp = comp.lock().unwrap();
    Ok(comp.probes.iter().map(|p| match &p.body {
        Body::Join(j) => j.passed_over.replace(0),
        Body::Sql(_) | Body::Unplanned => 0,
    }).collect())
}

pub fn steps(conn: &Connection, name: &str) -> Result<String> {
    let comp = component(conn, name)?;
    let comp = comp.lock().unwrap();
    Ok(comp.probes.iter().map(|p| match &p.body {
        Body::Join(j) => {
            let c: Vec<(u64, u64)> = j.counts.iter().map(|[a, b]| (a.replace(0), b.replace(0))).collect();
            j.plan_counts(&c).iter().map(|(a, b)| format!("{a}:{b}")).collect::<Vec<_>>().join(",")
        }
        Body::Sql(_) | Body::Unplanned => String::new(),
    }).collect::<Vec<_>>().join("\n"))
}

/// Decision 0206: each probe's work budget, by probe index (`None`, no limit). A probe that spends it is cut at the
/// delta row it is at: the run returns `5`, and `take_cut` says where.
pub fn set_budgets(conn: impl Scoped, name: &str, budgets: &[(usize, Option<u64>)]) -> Result<()> {
    let comp = component(conn.db(), name)?;
    let comp = comp.lock().unwrap();
    for &(i, b) in budgets {
        if let Some(p) = comp.probes.get(i) { p.budget.set(b); }
    }
    Ok(())
}

/// Where the rows a cut probe left are read in its body now (decision 0206): the step of its replacement that reads
/// the atom its old first step read.
pub fn cut_step(conn: impl Scoped, name: &str, step: usize) -> Result<()> {
    let comp = component(conn.db(), name)?;
    let mut comp = comp.lock().unwrap();
    if let Some(c) = &mut comp.cut { c.step = Some(step); }
    Ok(())
}

/// The last cut of the run (decision 0206), taken.
pub fn take_cut(conn: impl Scoped, name: &str) -> Result<Option<CutReport>> {
    let comp = component(conn.db(), name)?;
    let mut comp = comp.lock().unwrap();
    Ok(comp.last_cut.take())
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
            let p = comp.probes.get_mut(i).ok_or_else(|| err(format!("no probe {i} of {n}")))?;
            p.whole = Some(j);
            p.wkey = Some(Arc::clone(spec));
            if let Some(c) = &mut comp.cut && c.next == i { c.step = None; }
            return Ok(());
        }
    };
    comp.probes.get_mut(i).ok_or_else(|| err(format!("no probe {i} of {n}")))?.body = body;
    // A cut probe's new body reads the rows it left where the driver says (`cut_step`), else where known.
    if let Some(c) = &mut comp.cut && c.next == i { c.step = None; }
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
        // SAFETY: the connecting connection's own handle, not closed by the borrowed `Connection` made of it.
        let conn = unsafe { Connection::from_handle(db.handle()) }?;
        let comp = component_in(&on(&conn), &arg(0)).map_err(|e| Error::ModuleError(e.to_string()))?;
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
        cands.any(|id| { if !r.dead(id) && keys.iter().all(|&(c, k)| r.value(id as usize, c) == k) { ids.push(id) } false });
        Ok(())
    }
    fn next(&mut self) -> Result<()> { self.pos += 1; Ok(()) }
    fn eof(&self) -> bool { self.pos >= self.ids.len() }
    fn column(&self, ctx: &mut VContext, i: c_int) -> Result<()> {
        let s = self.store.read().unwrap();
        let r = &s.rels[self.rel];
        let v = r.value(self.ids[self.pos] as usize, i as usize);
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

/// `@dot(a, b)` (`core.rerail`): `a.b`, and `b` when `a` is "".
/// `cat(a, b)` as a rule's head computes it, with `conn`'s symbols (the recursive-maintenance spike's evaluator).
pub fn cat_symbols(conn: &Connection, a: i64, b: i64) -> i64 {
    let syms = symbols(conn).expect("the connection's symbols");
    let mut syms = syms.write().unwrap();
    cat(&mut syms, a, b)
}

/// `cat`'s text if it is an atom (`Interner::held`): none made.
fn cat_held(syms: &mut Interner, a: i64, b: i64) -> Option<i64> {
    if let (Ok(x), Ok(y)) = (crate::content_store::atoms::Sym::try_from(a), crate::content_store::atoms::Sym::try_from(b)) {
        let mut s = std::mem::take(&mut syms.buf);
        let got = syms.atoms.cat_held(x, y, &mut s);
        let id = match got {
            Some(Ok(Some(atom))) => Some(i64::from(atom)),
            Some(Ok(None)) if syms.atoms.misses_are_final() => None,
            Some(Ok(None)) => syms.held(&s),
            Some(Err(())) => Some(b),
            None => { syms.buf = s; return cat_held_read(syms, a, b) }
        };
        syms.buf = s;
        return id;
    }
    cat_held_read(syms, a, b)
}

/// Value `v`'s text pushed on `s`: a text the morsel deferred, or an atom's in memory.
fn push_text(syms: &Interner, s: &mut String, v: i64) {
    match syms.deferred_text(v) {
        Some(t) => s.push_str(t),
        None => { syms.atoms.with_str(v as crate::content_store::atoms::Sym, |t| s.push_str(t)); }
    }
}

/// `cat_held` for texts not all in memory: each read where it is.
fn cat_held_read(syms: &mut Interner, a: i64, b: i64) -> Option<i64> {
    if syms.name(a).is_empty() {
        return Some(b);
    }
    let mut s = std::mem::take(&mut syms.buf);
    s.clear();
    push_text(syms, &mut s, a);
    s.push('.');
    push_text(syms, &mut s, b);
    let id = syms.held(&s);
    syms.buf = s;
    id
}

fn cat(syms: &mut Interner, a: i64, b: i64) -> i64 {
    if let (Ok(x), Ok(y)) = (crate::content_store::atoms::Sym::try_from(a), crate::content_store::atoms::Sym::try_from(b)) {
        let mut s = std::mem::take(&mut syms.buf);
        let got = syms.atoms.cat_held(x, y, &mut s);
        let id = match got {
            Some(Ok(Some(atom))) => i64::from(atom),
            Some(Ok(None)) => syms.id(&s),
            Some(Err(())) => b,
            None => { syms.buf = s; return cat_read(syms, a, b) }
        };
        syms.buf = s;
        return id;
    }
    cat_read(syms, a, b)
}

/// `cat` for texts not all in memory: each read where it is.
fn cat_read(syms: &mut Interner, a: i64, b: i64) -> i64 {
    if syms.name(a).is_empty() {
        return b;
    }
    let mut s = std::mem::take(&mut syms.buf);
    s.clear();
    push_text(syms, &mut s, a);
    s.push('.');
    push_text(syms, &mut s, b);
    let id = syms.id(&s);
    syms.buf = s;
    id
}

/// This thread's CPU seconds (wasm32: `prof::single_thread_cpu`).
#[cfg(target_family = "wasm")]
pub fn thread_cpu() -> f64 {
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
pub fn thread_cpu() -> f64 {
    let (user, kernel) = crate::code_rule_dsl::prof::windows_cpu(true);
    (user + kernel).as_secs_f64()
}

/// CPU seconds and rows of input loads and index builds: [base facts (construction), lower strata's tables], over
/// the run (decision 0442).
#[derive(Default)]
struct Loads(Mutex<[(f64, u64); 2]>);

/// Adds `cpu` seconds and `rows` to the run's loads of part `i` (`Loads`).
fn loaded(i: usize, cpu: f64, rows: u64) {
    crate::code_rule_dsl::logic::run::with_local(|l: &Loads| {
        let l = &mut l.0.lock().unwrap()[i];
        l.0 += cpu;
        l.1 += rows;
    });
}

pub fn orders_counted() -> (u64, u64) {
    (run_counts().orders_opened.load(AtomicOrdering::Relaxed), run_counts().orders_made.load(AtomicOrdering::Relaxed))
}

/// `set_fixpoint_loads()`: "base_cpu_ms base_rows derived_cpu_ms derived_rows". Base-fact relations and their
/// indexes are construction (the product's tape and anchor index hold them), reported apart from execution.
pub fn loads() -> String {
    let l = crate::code_rule_dsl::logic::run::local_if_any::<Loads>().map(|l| *l.0.lock().unwrap()).unwrap_or_default();
    format!("{:.1} {} {:.1} {}", l[0].0 * 1e3, l[0].1, l[1].0 * 1e3, l[1].1)
}

/// An input relation: `n` rows flat (`k` columns each), and hash indexes by key-column bitmask. `base`: read from
/// the facts (or a view of them), not from a derived relation.
/// `set`, `dead`, `ndead`: a moved relation's dedup set and dead rows (`Rel`), kept so a later run's relation can
/// start from it (`Rel::based`).
pub struct InRel { pub k: usize, n: usize, rows: Rows, pub view: Option<View>, base: bool, types: Vec<u8>, stored: usize, idx: Mutex<HashMap<u64, Arc<Csr>>>, chains: Vec<Index>,
                   /// Its row ranges known to be in an order already (decision 0180): its relation's, when moved.
                   sorted: runs::Sorted,
                   /// Its columns' statistics and its orders' key sets, as its relation had them (`Rel`).
                   sketches: Vec<Sketch>, serves: Vec<Vec<Vec<usize>>>, pub dead: Shared<u64>, pub ndead: usize, summaries: scalars::Pool,
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
                   level: Shared<u32>,
                   /// Its statistics, kept as its relation's rows came and went (`Rel::stats`).
                   stats: Option<KeptStats>,
                   /// A view of an input's columns read into flat rows (`hold_view`, Q37): made again in less than
                   /// keeping it costs, so never kept.
                   pub viewed: bool,
                   /// A column each of whose values is at most one row, found without an order of its own (`Keyed`): a
                   /// view's key column (`hold_view`). Its probes make no order (`keyed_rows`).
                   keyed: Option<Keyed>,
                   /// Its versions as read (`version_view`, decision 0364 stage 2): `Old`, `Minus`, `Plus`, each made
                   /// once, so every join of the run that reads one reads the same relation and its indexes.
                   vers: [std::sync::OnceLock<Option<Arc<InRel>>>; 3] }
/// A key column of a relation (`InRel::keyed`) and how a value's row is found: by the source the relation's rows
/// are, row for row (`Source::key_row`: a node's own column, through the batch's own index of its documents), or by
/// the rows themselves where they are runs ascending in the column that do not overlap (`runs`: each run's first
/// value and its rows, sorted by that value; a probe finds its run, then its row, by two binary searches).
#[derive(Clone)]
pub struct Keyed { col: usize, runs: Vec<(i64, u32, u32)>, src: Option<(Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, usize)> }
impl Keyed {
    /// Column `c` of `n` rows of `k` values (`flat`) as such runs, if its values are a key in them: each maximal run
    /// that ascends strictly, sorted by first value, and each run's last value under the next one's first. `None`
    /// where a value repeats, or the runs are more than one in 64 rows (a column in no order).
    fn of(flat: &[i64], k: usize, c: usize) -> Option<Keyed> {
        if c >= k {
            return None;
        }
        let n = flat.len() / k;
        let at = |r: usize| flat[r * k + c];
        let mut runs: Vec<(i64, u32, u32)> = Vec::new();
        let mut start = 0;
        for r in 1..=n {
            if r == n || at(r) <= at(r - 1) {
                runs.push((at(start), start as u32, r as u32));
                if runs.len() > n / 64 + 1 {
                    return None;
                }
                start = r;
            }
        }
        runs.sort_unstable_by_key(|r| r.0);
        runs.windows(2).all(|w| at(w[0].2 as usize - 1) < w[1].0).then_some(Keyed { col: c, runs, src: None })
    }
}
/// A base input read in place: the bound vtab source's columns `cols`, at its rows `ids` (all rows when `None`, the
/// rows passing the atom's constant filters otherwise). Nothing of the source is copied.
/// A version of a relation that started from a kept one (decision 0364 stage 2, `InRel::version_view`): its rows
/// at the run's start (`Old`), those of them dead now (`Minus`), and those added since and live (`Plus`). The
/// relation itself is its rows now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ver {
    Old,
    Minus,
    Plus,
}

pub struct View { pub src: Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, cols: Vec<usize>, ids: Option<Vec<u32>>, eqs: Vec<(usize, i64)> }
impl View {
    /// Whether the view reads every row of its source (no constant filters chose its rows).
    pub fn whole(&self) -> bool {
        self.ids.is_none()
    }
    /// Whether constants chose its rows (a statement's `WHERE c = v`), not a version's or a pick's ids.
    pub fn filtered(&self) -> bool {
        !self.eqs.is_empty()
    }
    /// The constants that chose its rows: each a source's column and its value.
    pub fn eqs(&self) -> &[(usize, i64)] {
        &self.eqs
    }
    /// The source's column that view column `c` reads.
    pub fn column(&self, c: usize) -> usize {
        self.cols[c]
    }
    /// The source's rows whose view column `c` is `v` and which pass the filters, through the source's own index.
    /// The candidates of a probe of key columns `cols` (view columns) with values `key`, found through column
    /// `cols[lead]`. Where the key binds the source's document column and a key column whose lookup can be kept to
    /// one document (a held `node` view's kind, decision 0329), that document's alone: a probe by kind otherwise
    /// reads every document of the language, and by document every node of it. Every key is checked by the step.
    fn find_keyed(&self, cols: &[usize], key: &[i64], lead: usize) -> Vec<u32> {
        if self.eqs.is_empty()
            && let Some(dc) = self.src.doc_column()
            && let Some(j) = cols.iter().position(|&k| self.cols[k] == dc)
            && let Some(i) = cols.iter().position(|&k| self.src.kept_to_doc(self.cols[k]))
            && let Some(rows) = self.src.lookup_in(self.cols[cols[i]], &[key[i]], key[j])
        {
            return rows.into_iter().map(|r| r as u32).collect();
        }
        self.find(cols[lead], key[lead])
    }
    /// `find_keyed`'s rows, each with its values of view columns `need` (bit `c`: column `c`), read as the source's
    /// probe found them (`Source::lookup_read`); `None` where it has no such read, or the probe is not `find`'s.
    fn find_read(&self, cols: &[usize], key: &[i64], lead: usize, need: u64) -> Option<Vec<(u32, [i64; READ])>> {
        if !self.eqs.is_empty() || self.ids.is_some() || self.cols.len() > READ {
            return None;
        }
        // `find_keyed`'s document-kept lookup goes first where it applies.
        if let Some(dc) = self.src.doc_column()
            && cols.iter().any(|&k| self.cols[k] == dc)
            && cols.iter().any(|&k| self.src.kept_to_doc(self.cols[k]))
        {
            return None;
        }
        let (mut src, mut at, mut m) = ([0usize; READ], [0usize; READ], 0);
        for c in 0..self.cols.len() {
            if need >> c & 1 != 0 {
                src[m] = self.cols[c];
                at[m] = c;
                m += 1;
            }
        }
        let mut rows = Vec::new();
        let read = self.src.lookup_read(self.cols[cols[lead]], key[lead], &src[..m], &mut |row, got| {
            let mut vals = [0i64; READ];
            for j in 0..m {
                vals[at[j]] = got[j];
            }
            rows.push((row as u32, vals));
        });
        read.then_some(rows)
    }
    fn find(&self, c: usize, v: i64) -> Vec<u32> {
        if !self.eqs.is_empty() && let Some(rows) = self.src.lookup_eq(self.cols[c], v, &self.eqs) {
            return rows.into_iter().map(|r| r as u32).collect();
        }
        let rows = self.src.lookup(self.cols[c], &[v]).unwrap_or_default();
        rows.into_iter().filter(|&r| self.eqs.iter().all(|&(c, v)| self.src.value(r, c) == v)).map(|r| r as u32).collect()
    }
}
/// One row of an input relation: a slice of its flat rows, or a row of its view.
#[derive(Clone, Copy)]
pub enum RowRef<'a> { Flat(&'a [i64]), Rows(&'a rows::Rows, u32, usize), View(&'a View, usize), Source(&'a View, usize), Read([i64; READ], usize) }
/// The widest view row read whole (`RowRef::read`); a wider one is read a value at a time.
const READ: usize = 8;
/// The fewest rows of a view's scan read in one walk (`Rows::each`), not a row at a time: a walk reads each document
/// the range holds whole.
const WALK_SCAN: u32 = 64;
/// The most rows of a view's scan read in one walk and held with their values to be put in the next step's key order
/// (`walked_in_order`): 68 bytes a row, 17 MB at most; a longer scan's rows are read a row at a time as they are
/// ordered.
const WALKED_ROWS: usize = 1 << 18;
impl<'a> RowRef<'a> {
    #[inline]
    pub fn at(self, c: usize) -> i64 {
        match self {
            RowRef::Flat(r) => r[c],
            RowRef::Rows(r, id, k) => r.value(id, k, c),
            RowRef::View(v, id) => v.src.value(v.ids.as_ref().map_or(id, |ids| ids[id] as usize), v.cols[c]),
            RowRef::Source(v, row) => v.src.value(row, v.cols[c]),
            RowRef::Read(vals, _) => vals[c],
        }
    }
    /// `read`, the view's columns in `need` alone (bit `c`: column `c`), the others left 0 and never read: a step
    /// reads only the columns it binds, tests or checks. A view's column may be a search on the tape (a node's
    /// parent on format 15), which a key the probe already matched need not pay (decision 0353 step 2).
    #[inline]
    pub fn read_need(self, need: u64) -> RowRef<'a> {
        let (v, row) = match self {
            RowRef::View(v, id) => (v, v.ids.as_ref().map_or(id, |ids| ids[id] as usize)),
            RowRef::Source(v, row) => (v, row),
            other => return other,
        };
        let n = v.cols.len();
        if n > READ {
            return self;
        }
        if need & ((1u64 << n) - 1) == (1u64 << n) - 1 {
            return self.read();
        }
        let (mut cols, mut at, mut m) = ([0usize; READ], [0usize; READ], 0);
        for c in 0..n {
            if need >> c & 1 != 0 {
                cols[m] = v.cols[c];
                at[m] = c;
                m += 1;
            }
        }
        let (mut got, mut vals) = ([0i64; READ], [0i64; READ]);
        v.src.row_at(row, &cols[..m], &mut got[..m]);
        for j in 0..m {
            vals[at[j]] = got[j];
        }
        RowRef::Read(vals, row)
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
/// Over a derived relation (no view) with no kept chain answering the key, the index is a sorted order of its own on
/// the key columns (`order`, decision 0112: 4 bytes a row, not a hash table's 18 and its build's 46). It is made
/// when first probed (`ordered`: printed or not, decision 0186), so an order no probe reaches costs nothing.
/// Over a relation whose key column `cols[lead]` ascends over its rows (`InRel::keyed`), a probe is that column's
/// binary search, at most one row, every other key checked by the step (`keyed`): nothing is built.
pub struct Csr { cols: Vec<usize>, table: HashTable<(u32, u32, u32)>, ids: Vec<u32>, source: bool, sorted: bool, chain: Option<usize>, lead: usize, composite: bool, keyed: bool, ordered: Option<bool>, order: OnceLock<Index>,
    /// A derived input's order (`ordered`) for a ranged probe (`Ranged`): the key's columns, then this one, so a key's
    /// rows are in its order and a range of it is found by searching (`Found::within`).
    then: Option<usize>,
                 /// A whole source's rows in one block relation, probed by that relation's own index there (its number;
                 /// decision 0325 step 5), shared with the device: no host table of its rows. Its buckets' rows are
                 /// candidates, checked against the key by the reader.
                 block: Option<(crate::code_rule_dsl::facts::rows::InBlock, usize)> }
impl Csr {
    /// Whether `get` gives a key's rows alone (not a bucket's candidates).
    fn exact(&self) -> bool {
        self.block.is_none()
    }
    /// The sorted order a probe walks: the relation's kept chain (`chain`) or this index's own order, made now if
    /// this is its first probe.
    fn runs<'a>(&'a self, rel: &'a InRel) -> Option<&'a Index> {
        match self.ordered {
            // Made under its lock, on this thread alone (`runs::inline`, Q136): a sort on the pool let this thread's
            // wait run another job, a probe of this index, which waited on the lock its own thread held. Made outside
            // the lock instead (ba519e73a), each probe racing the first made its own order, one a thread, and a probe
            // run inside the sort's wait made another: the same sort several times over. Under the lock, the others
            // wait for the one build.
            Some(printed) => Some(self.order.get_or_init(|| runs::inline(|| {
                let _index = crate::code_rule_dsl::logic::profile::span(|| "probe: order of a derived input".into());
                let c0 = thread_cpu();
                charge(Unit::IndexEntry, rel.len() as u64);
                // A ranged probe's order holds the range's column after the key's; its prints would be of both, and a
                // probe gives the key alone, so it is searched by its rows.
                let cols: Vec<usize> = self.cols.iter().copied().chain(self.then).collect();
                let mut order = if printed && self.then.is_none() { Index::printed(cols.clone()) } else { Index::new(cols.clone()) };
                order.add_run_in(&rel.rows, rel.k, 0, rel.n, 0, &rel.sorted.within(&cols, rel.k, 0, rel.n as u32));
                run_counts().orders_made.fetch_add(1, AtomicOrdering::Relaxed);
                loaded(1, thread_cpu() - c0, 0);
                if env_set!("RERAIL_SPIKE_INDEX") {
                    eprintln!("O125 order made took {:.3} ms rows={} cols={:?} of {} viewed={}", (thread_cpu() - c0) * 1e3, rel.len(), self.cols, rel.name.get().map_or("?", String::as_str), rel.viewed);
                }
                order
            }))),
            None => self.chain.map(|i| &rel.chains[i]),
        }
    }
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
        Csr { cols: cols.to_vec(), table, ids, source: false, sorted: false, chain: None, lead: 0, composite: false, keyed: false, ordered: None, order: OnceLock::new(), block: None, then: None }
    }
    /// `key` holds the values of `cols`, in order. Its rows of the key; for an index in the block (`block`), its
    /// bucket's rows, a key's candidates, which the reader checks (`exact`).
    fn get<'a>(&'a self, rel: &InRel, key: &[i64]) -> &'a [u32] {
        if let Some((ib, i)) = &self.block {
            return ib.relation().group(*i, key.iter().fold(rerail_blocks::SEED, |h, &v| rerail_blocks::mix(h, v)));
        }
        self.table.find(hash_vals(key.iter().copied()), |&(rep, _, _)| self.cols.iter().zip(key).all(|(&c, &v)| rel.at(rep as usize).at(c) == v))
            .map_or(&[], |&(_, start, len)| &self.ids[start as usize..][..len as usize])
    }
}
/// Drops native relations `tables` of `conn`'s run: no later stratum reads them (`plan::last_reads`), so their rows
/// and the indexes the waves kept are freed now, not when the run ends.
pub fn release(conn: &Connection, tables: &[String]) {
    forgotten_on_close();
    let on = on(conn);
    let mut derived = on.derived.lock();
    for t in tables {
        derived.remove(t);
    }
}

/// Drops the indexes of native relations `tables` that stay to the run's end (outputs, kept relations) but that no
/// later stratum reads: their rows are read once more, in order, and their per-column indexes were most of their
/// size. A relation shared with a reader still open keeps them.
pub fn unindex(conn: &Connection, tables: &[String]) {
    forgotten_on_close();
    let on = on(conn);
    let mut derived = on.derived.lock();
    for t in tables {
        if let Some(rel) = derived.get_mut(t).and_then(Arc::get_mut) {
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
    on(conn).inputs.lock().retain(|key, _| !reads(key));
    crate::code_rule_dsl::logic::profile::shared::<StoredTables>().0.lock().unwrap().retain(|(p, t), _| *p != path || !t.rsplit('.').next().is_some_and(|n| tables.iter().any(|x| x == n)));
}

/// Drops every native relation of `conn`'s runs: a chunk's, once its exports are written to the cross-file part.
pub fn forget_derived(conn: &Connection) {
    on(conn).derived.lock().clear();
}

/// A native relation of `conn`'s run, for a caller that keeps it past the run with the indexes it has (a session's
/// relations, decision 0090 step 3).
pub fn held(conn: impl Scoped, table: &str) -> Option<Arc<InRel>> {
    let conn = conn.db();
    on(&conn).derived.lock().get(table).cloned()
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
    crate::code_rule_dsl::logic::plan::corpus::maintain::shape::held(table);
    let _ = rel.name.set(table.to_owned());
    forgotten_on_close();
    let on = on(conn);
    on.derived.lock().insert(table.to_owned(), rel);
    on.adopted.lock().insert(table.to_owned());
}

/// Holds `table` in `conn`'s run as `rel`, a relation it already holds under another name: a quiet stratum's
/// relation is its kept one, unchanged (decision 0101 §1), shared with its indexes.
pub fn hold_as(conn: impl Scoped, table: &str, rel: Arc<InRel>) {
    crate::code_rule_dsl::logic::plan::corpus::maintain::shape::held(table);
    let conn = conn.db();
    forgotten_on_close();
    on(&conn).derived.lock().insert(table.to_owned(), rel);
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
    forgotten_on_close();
    on(&conn).derived.lock().insert(table.to_owned(), rel);
    Ok(())
}

/// `hold_view`'s relation read in place (decision 0329): `src`'s rows in its columns `cols`, no row copied, for a source
/// that answers a view's probes from its own index (a batch's `node`: `Source::sorted_rows`, a field by child off the
/// tape) and counts its columns without reading its rows (`syntax::node_column_counts`), which the caller gives as the
/// view's statistics. Read in place through a source without them, each reader's row went through it (Q37: 8.4 s
/// against the copy's 6.0 on fixtures/python), and `hold_view` reads such a source once into flat rows.
pub fn hold_view_in_place(conn: impl Scoped, table: &str, src: Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, cols: Vec<usize>, types: Vec<u8>) {
    crate::code_rule_dsl::logic::plan::corpus::maintain::shape::held(table);
    let conn = conn.db();
    let k = types.len();
    debug_assert_eq!(cols.len(), k);
    let view = View { src, cols, ids: None, eqs: Vec::new() };
    let rel = Arc::new(InRel { vers: Default::default(), viewed: true, keyed: None, k, n: 0, rows: Rows::new(k), view: Some(view), base: false, types, stored: 0, idx: Mutex::new(HashMap::default()), chains: Vec::new(), sorted: Default::default(), sketches: Vec::new(), serves: Vec::new(), dead: Shared::default(), ndead: 0, summaries: Default::default(), measured: Default::default(), counted: Default::default(), tables: Default::default(), name: Default::default(), from: None, uid: Default::default(), level: Shared::default(), stats: None });
    let _ = rel.name.set(table.to_owned());
    forgotten_on_close();
    on(&conn).derived.lock().insert(table.to_owned(), rel);
}

/// A changed read's versions given, not marked (0364 R1, R2): an input's old rows, its rows gone, its rows added and
/// its rows now,
/// each read in place over its source (its columns `cols` of it, or all), as `Version(table, Old | Minus | Plus)`
/// reads them while the maintain mode runs a stratum that reads it. Beside the relation, not in its place: an input
/// the engine holds is read as it is, with its indexes, and none is held under another name. `take_versions` drops
/// them.
pub fn give_versions(conn: impl Scoped, table: &str, types: Vec<u8>, given: [(Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, Option<Vec<usize>>); 4]) {
    let conn = conn.db();
    let k = types.len();
    let view = |(src, cols): (Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, Option<Vec<usize>>)| -> Arc<InRel> {
        let viewed = cols.is_some();
        let view = View { src, cols: cols.unwrap_or_else(|| (0..k).collect()), ids: None, eqs: Vec::new() };
        Arc::new(InRel { vers: Default::default(), viewed, keyed: None, k, n: 0, rows: Rows::new(k), view: Some(view), base: false, types: types.clone(), stored: 0, idx: Mutex::new(HashMap::default()), chains: Vec::new(), sorted: Default::default(), sketches: Vec::new(), serves: Vec::new(), dead: Shared::default(), ndead: 0, summaries: Default::default(), measured: Default::default(), counted: Default::default(), tables: Default::default(), name: Default::default(), from: None, uid: Default::default(), level: Shared::default(), stats: None })
    };
    forgotten_on_close();
    on(&conn).given.lock().insert(table.to_owned(), given.map(view));
}

/// Drops `tables`' given versions (`give_versions`).
pub fn take_versions(conn: impl Scoped, tables: &[String]) {
    let conn = conn.db();
    let on = on(&conn);
    let mut given = on.given.lock();
    for t in tables {
        given.remove(t.as_str());
    }
}

/// Holds `table` in `conn`'s run as `src`'s rows, read in place (a view of every row): a relation an index holds
/// (the reach index, decision 0119) is probed through the source's own lookups, never copied into rows.
pub fn hold_source(conn: impl Scoped, table: &str, src: Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, types: Vec<u8>) {
    let conn = conn.db();
    let rel = InRel::over_source(src, types);
    let _ = rel.name.set(table.to_owned());
    forgotten_on_close();
    on(&conn).derived.lock().insert(table.to_owned(), rel);
}

/// Holds `table` in `conn`'s run as `src`'s columns `cols`, one row of each source row: a rule that copies a bound
/// input's columns (`kind(n, k) :- node(_, n, k, _, _)`, Q37), no stratum. Its rows are read once, document by
/// document (`Source::rows_of`), into flat rows its readers read as a derived relation's: no component, join, set
/// or index is made for it. Read in place instead, each reader's row went through the source (its row's document,
/// a kind's dialect), and the readers read more rows than it holds: 8.4 s against the copy's 6.0 on
/// fixtures/python.
/// Its live statistics are a copy's: each column's sketch, fed its rows as a stratum's relation is as they arrive
/// (`live_of`), so its readers plan as they did with the copy. Given the input's column statistics instead, it had
/// no histogram of its kinds (`kind(x, "call")` priced at the average kind), and the 300-file corpus's plans
/// moved: 49.1 s of strata CPU against the copies' 46.8.
pub fn hold_view(conn: impl Scoped, table: &str, src: Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, cols: Vec<usize>, types: Vec<u8>) -> Live {
    let conn = conn.db();
    let k = cols.len();
    debug_assert_eq!(k, types.len());
    let n = src.len();
    let (flat, sketches) = view_rows(src.as_ref(), &cols);
    let keyed = view_key(&src, &cols, &flat);
    let live = live_from(n, n, &sketches);
    let mut rel = Arc::into_inner(InRel::from_flat(flat, n, types)).expect("a new relation");
    rel.sketches = sketches;
    rel.viewed = true;
    if env_set!("RERAIL_SPIKE_INDEX") {
        eprintln!("O125 view held {table} rows={n} keyed={:?}", keyed.as_ref().map(|ky| (ky.col, ky.runs.len(), ky.src.is_some())));
    }
    rel.keyed = keyed;
    let rel = Arc::new(rel);
    let _ = rel.name.set(table.to_owned());
    forgotten_on_close();
    on(&conn).derived.lock().insert(table.to_owned(), rel);
    live
}

/// The key of a view of `src`'s columns `cols` (its rows `flat`, the source's row for row), by which its probes find
/// a row without an order (`InRel::keyed`): the source's own key where it has one (`Source::keys_rows`); else a
/// column checked on the rows to be a key in ascending runs (`Keyed::of`), not assumed (a column that ascends but
/// repeats a value, a document's, is none).
fn view_key(src: &Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, cols: &[usize], flat: &[i64]) -> Option<Keyed> {
    match (0..cols.len()).find(|&c| src.keys_rows(cols[c])) {
        Some(c) => Some(Keyed { col: c, runs: Vec::new(), src: Some((Arc::clone(src), cols[c])) }),
        None => (0..cols.len()).find_map(|c| Keyed::of(flat, cols.len(), c)),
    }
}

/// `src`'s columns `cols`, row after row, and each column's sketch of them (`hold_view`).
fn view_rows(src: &dyn crate::code_rule_dsl::facts::vtab::Source, cols: &[usize]) -> (Vec<i64>, Vec<Sketch>) {
    let k = cols.len();
    let mut flat = Vec::new();
    src.rows_of(cols, &mut flat);
    debug_assert_eq!(flat.len(), src.len() * k);
    // Each column's sketch, its values in row order, as a stratum's relation sketches its rows (`indexed_to_n`):
    // many rows, a job a column.
    let mut sketches = vec![Sketch::default(); k];
    let sketch = |c: usize, sk: &mut Sketch| for row in flat.chunks_exact(k) { sk.add(row[c]); };
    if src.len() >= SPANNED_ROWS && k > 1 && crate::threads::max() > 1 {
        crate::threads::scope(|jobs| {
            for (c, sk) in sketches.iter_mut().enumerate() {
                jobs.spawn("a view's column sketched", move |_| sketch(c, sk));
            }
        });
    } else {
        for (c, sk) in sketches.iter_mut().enumerate() {
            sketch(c, sk);
        }
    }
    (flat, sketches)
}

/// The source a relation of `conn`'s run is held as (`hold_source`), for a caller that reads the index itself.
pub fn held_source(conn: impl Scoped, table: &str) -> Option<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>> {
    let conn = conn.db();
    let rel = on(&conn).derived.lock().get(table).cloned()?;
    rel.held_source()
}

/// Holds `table` in `conn`'s run as a relation with no rows, as a component that derived none would leave it: for a
/// stratum with nothing to run, which opens no component.
pub fn hold_empty(conn: impl Scoped, table: &str, types: Vec<u8>) {
    let conn = conn.db();
    forgotten_on_close();
    on(&conn).derived.lock().insert(table.to_owned(), InRel::flat(&[], types));
}

/// Holds `table` in `conn`'s run as a relation of `rows` (flat, `types.len()` values each): one a run kept from an
/// earlier run of the same stratum over the same inputs (decision 0077 §4), read as if this run had derived it.
pub fn hold_rows(conn: &Connection, table: &str, types: Vec<u8>, rows: Vec<i64>, n: usize) {
    crate::code_rule_dsl::logic::plan::corpus::maintain::shape::held(table);
    let rel = InRel::kept(types, rows, n);
    let _ = rel.name.set(table.to_owned());
    forgotten_on_close();
    on(&conn).derived.lock().insert(table.to_owned(), rel);
}

/// `rel`, an earlier run's kept relation bound where it lies (decision 0401 F3), held as `conn`'s run's `table`.
pub fn hold_bound(conn: &Connection, table: &str, rel: Arc<InRel>) {
    crate::code_rule_dsl::logic::plan::corpus::maintain::shape::held(table);
    let _ = rel.name.set(table.to_owned());
    forgotten_on_close();
    on(&conn).derived.lock().insert(table.to_owned(), rel);
}

/// A native relation of `conn`'s run as it can be kept and held again (`hold_rows`): its columns' types and its live
/// rows, flat.
pub fn kept_rows(conn: &Connection, table: &str) -> Option<(Vec<u8>, Vec<i64>, usize)> {
    let rel = on(&conn).derived.lock().get(table).cloned()?;
    if rel.view.is_some() {
        return None;
    }
    let k = rel.k;
    let mut out = Vec::with_capacity(rel.rows() * k);
    for id in 0..rel.len() as u32 {
        if rel.ndead > 0 && is_dead(&rel.dead, id) {
            continue;
        }
        out.extend_from_slice(&rel.rows.row(id, k));
    }
    Some((rel.types.clone(), out, rel.rows()))
}

/// The relations `adopt`ed into `conn`'s next run, taken by its planner.
pub fn adopted(conn: &Connection) -> std::collections::BTreeSet<String> {
    std::mem::take(&mut *on(conn).adopted.lock())
}

impl InRel {
    /// A relation of `rows` (flat, `types.len()` values each) an earlier run kept (decision 0077 §4), in no table.
    pub fn kept(types: Vec<u8>, rows: Vec<i64>, n: usize) -> Arc<InRel> {
        let k = types.len();
        Arc::new(InRel { vers: Default::default(), viewed: false, keyed: None, k, n, rows: Rows::frozen(k, rows, n), view: None, base: false, types, stored: 0, idx: Mutex::new(HashMap::default()), chains: Vec::new(), sorted: Default::default(), sketches: Vec::new(), serves: Vec::new(), dead: Shared::default(), ndead: 0, summaries: Default::default(), measured: Default::default(), counted: Default::default(), tables: Default::default(), name: Default::default(), from: None, uid: Default::default(), level: Shared::default(), stats: None })
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
        Arc::new(InRel { vers: Default::default(), viewed: false, keyed: None, k, n, rows: Rows::frozen(k, flat, n), view: None, base: false, types, stored: n, idx: Mutex::new(HashMap::default()), chains: Vec::new(), sorted: Default::default(), sketches: Vec::new(), serves: Vec::new(), dead: Shared::default(), ndead: 0, summaries: Default::default(), measured: Default::default(), counted: Default::default(), tables: Default::default(), name: Default::default(), from: None, uid: Default::default(), level: Shared::default(), stats: None })
    }
    /// A relation of `n` rows, flat (`types.len()` values each), as a stored frame gives them (t33 K4): read as a
    /// relation the run derived.
    pub fn from_flat(rows: Vec<i64>, n: usize, types: Vec<u8>) -> Arc<InRel> {
        let k = types.len();
        Arc::new(InRel { vers: Default::default(), viewed: false, keyed: None, k, n, rows: Rows::frozen(k, rows, n), view: None, base: false, types, stored: 0, idx: Mutex::new(HashMap::default()), chains: Vec::new(), sorted: Default::default(), sketches: Vec::new(), serves: Vec::new(), dead: Shared::default(), ndead: 0, summaries: Default::default(), measured: Default::default(), counted: Default::default(), tables: Default::default(), name: Default::default(), from: None, uid: Default::default(), level: Shared::default(), stats: None })
    }
    /// `from_flat`'s relation with each row's level (decision 0090), as a frame of its levels gives them: `None` when
    /// they are not one per row.
    pub fn from_flat_levels(rows: Vec<i64>, n: usize, types: Vec<u8>, level: Vec<u32>) -> Option<Arc<InRel>> {
        if level.len() != n {
            return None;
        }
        let mut rel = Arc::into_inner(Self::from_flat(rows, n, types)).expect("a new relation");
        rel.level = level.into();
        Some(Arc::new(rel))
    }
    /// Its live rows' levels, in `live_rows`' order: `None` unless every row has one (`has_levels`).
    pub fn live_levels(&self) -> Option<Vec<u32>> {
        if !self.has_levels() {
            return None;
        }
        Some((0..self.n).filter(|&id| self.ndead == 0 || !is_dead(&self.dead, id as u32)).map(|id| self.level[id]).collect())
    }
    /// A relation of `rows` (`n` of them, `types.len()` values each) gathered where they lie (`Rows::append_part`),
    /// rows `dead` marks not among them: an export the chunks handed on, read in place.
    pub fn held(rows: Rows, n: usize, types: Vec<u8>, dead: Vec<u64>) -> Arc<InRel> {
        Self::held_shared(rows, n, types, dead.into(), Shared::default())
    }
    /// `held`, its dead bits and levels a kept block's, read where they lie (decision 0401 F3).
    pub fn held_shared(rows: Rows, n: usize, types: Vec<u8>, dead: Shared<u64>, level: Shared<u32>) -> Arc<InRel> {
        let k = types.len();
        debug_assert!(level.is_empty() || level.len() == n, "{} levels of {n} rows", level.len());
        let ndead = dead.iter().map(|w| w.count_ones() as usize).sum();
        Arc::new(InRel { vers: Default::default(), viewed: false, keyed: None, k, n, rows, view: None, base: false, types, stored: 0, idx: Mutex::new(HashMap::default()), chains: Vec::new(), sorted: Default::default(), sketches: Vec::new(), serves: Vec::new(), dead, ndead, summaries: Default::default(), measured: Default::default(), counted: Default::default(), tables: Default::default(), name: Default::default(), from: None, uid: Default::default(), level, stats: None })
    }
    /// Its rows, as a source of columns `names`.
    pub fn source(self: &Arc<Self>, names: Vec<String>) -> Arc<dyn crate::code_rule_dsl::facts::vtab::Source> {
        Derived::new(Arc::clone(self), Some(names))
    }
    /// Its rows' bytes and their levels' (decision 0090, 4 bytes a row); its indexes not counted.
    pub fn bytes(&self) -> usize {
        self.rows.bytes() + self.level.len() * 4
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
        let flat = self.rows.to_vec(0, self.n, self.k);
        let (k, n, rows) = (self.k, self.n, &flat[..]);
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
        let dup = crate::code_rule_dsl::logic::run::with_local(|all: &Derivations| all.0.lock().unwrap().get(name).copied()).map_or(0.0, |(d, nw)| if nw > 0 { d.saturating_sub(nw) as f64 / nw as f64 } else { 0.0 });
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
        let flat = self.rows.to_vec(0, self.n, self.k);
        let (k, n, rows) = (self.k, self.n, &flat[..]);
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
                    got_now += usize::from(find_row(&self.chains, rows, k, &r, |x| is_dead(&self.dead, x)).is_some());
                    continue;
                }
                let exact = self.chains.iter().find(|ix| ix.cols == *shape);
                let pick = exact.map(|ix| ix.find(rows, k, &shape.iter().map(|&c| r[c]).collect::<Vec<_>>(), None)).or_else(|| {
                    shape.iter().filter_map(|&c| self.chains.iter().find(|ix| ix.cols == [c])).map(|ix| ix.find(rows, k, &[r[ix.cols[0]]], None)).min_by_key(runs::Found::len)
                });
                let Some(found) = pick else { continue };
                for at in found.ids() {
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
        let csr = self.idx.lock().map(|m| m.values().map(|c| (c.cols.clone(), c.ids.capacity() * 4 + c.table.capacity() * (12 + 1) + c.order.get().map_or(0, Index::bytes))).collect()).unwrap_or_default();
        (self.k, set, chains, csr)
    }
    pub fn held_bytes(&self) -> (usize, usize, usize) {
        let values = self.rows.capacity_bytes();
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
    let on = on(conn);
    table::prefetch(wants, |name| on.derived.lock().get(name).cloned()).map_err(err)
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
const SEED: usize = usize::MAX;
/// Columns with at most this many values report each value's postings (`stats`).

/// A native relation's rows, in place (for a caller that keeps an output in the engine): a source of columns
/// `c0`, `c1`, ….
pub fn derived(conn: impl Scoped, table: &str) -> Option<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>> {
    let conn = conn.db();
    let rel = on(&conn).derived.lock().get(table).cloned()?;
    Some(Derived::new(rel, None))
}
/// A derived relation's rows where they lie (a `Derived` source, not a view): its rows, how many, its dead bits
/// (empty when none) and its columns' types. For a reader that keeps them rather than copying them (an export).
pub fn held_rows(src: &dyn crate::code_rule_dsl::facts::vtab::Source) -> Option<(&Rows, usize, &[u64], &[u8])> {
    let d = src.as_any()?.downcast_ref::<Derived>()?;
    let r = &d.0;
    r.view.is_none().then(|| (&r.rows, r.n, if r.ndead > 0 { &r.dead[..] } else { &[][..] }, &r.types[..]))
}
/// A held relation's source (`Derived`, not a view, as wide as `width`): its relation, for readers that walk its rows
/// where they lie, part by part, not a value at a time through the source.
fn held_rel(src: &dyn crate::code_rule_dsl::facts::vtab::Source, width: usize) -> Option<&InRel> {
    let d = src.as_any()?.downcast_ref::<Derived>()?;
    (d.0.view.is_none() && d.0.k == width).then_some(&*d.0)
}
/// `src`'s rows in its order, row-major, a batch of at most `LIVE_BATCH` rows at a time, read where they lie: `None`
/// unless it is a held relation's source of `width` columns.
pub fn each_held_batch(src: &dyn crate::code_rule_dsl::facts::vtab::Source, width: usize, f: impl FnMut(&[i64])) -> Option<()> {
    let rel = held_rel(src, width)?;
    rel.each_live_batch(0, rel.n, f)
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
    /// The held index's answer (row ids of the whole relation) as this source's rows: with dead rows, a row is its
    /// position among the live ones (`len`, `value`), and a dead row is no answer.
    fn live_rows(&self, ids: Vec<usize>) -> Vec<usize> {
        match &self.2 {
            None => ids,
            Some(live) => ids.into_iter().filter_map(|id| live.binary_search(&(id as u32)).ok()).collect(),
        }
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
    fn as_any(&self) -> Option<&dyn std::any::Any> { Some(self) }
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
    fn lookup_all(&self, equal: &[(usize, i64)]) -> Option<Vec<usize>> { self.0.held_source()?.lookup_all(equal).map(|ids| self.live_rows(ids)) }
    fn sorts_rows(&self, c: usize) -> bool { self.0.held_source().is_none() && c < self.0.k }
    fn lookup(&self, c: usize, values: &[i64]) -> Option<Vec<usize>> {
        if c >= self.0.k {
            return None;
        }
        if let Some(src) = self.0.held_source() {
            return src.lookup(c, values).map(|ids| self.live_rows(ids));
        }
        let mut out: Vec<usize> = values.iter().flat_map(|&v| self.rows_of(c, v).iter().map(|&r| r as usize)).collect();
        out.sort_unstable();
        Some(out)
    }
    fn sorted_rows(&self, c: usize, v: i64) -> Option<&[u32]> { (c < self.0.k && self.0.held_source().is_none()).then(|| self.rows_of(c, v)) }
    fn value(&self, row: usize, column: usize) -> i64 { self.0.at(self.2.as_ref().map_or(row, |l| l[row] as usize)).at(column) }
    fn value_counts(&self, wanted: &[(usize, i64)]) -> Option<Vec<i64>> {
        // The held index counts dead rows too: with dead rows, the live ones are counted here.
        if self.2.is_none() && let Some(src) = self.0.held_source() { return src.value_counts(wanted); }
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
    let Some(rel) = on(&conn).derived.lock().get(table).cloned() else { return Err(err(format!("no native relation {table}"))) };
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
pub struct DerivedTab { base: ffi::sqlite3_vtab, on: Arc<OnConn> }
unsafe impl<'vtab> VTab<'vtab> for DerivedTab {
    type Aux = ();
    type Cursor = DerivedCursor;
    fn connect(db: &mut VTabConnection, _: Option<&()>, _: &[u8], _: &[u8], _: &[u8], _: &[&[u8]]) -> Result<(Cow<'static, CStr>, Self)> {
        // SAFETY: the connecting connection's own handle, not closed by the borrowed `Connection` made of it.
        let on = on(&unsafe { Connection::from_handle(db.handle()) }?);
        let cols: Vec<String> = (0..DERIVED_COLUMNS).map(|i| format!("c{i}")).collect();
        let schema = CString::new(format!("CREATE TABLE x({}, rel HIDDEN)", cols.join(", "))).map_err(|e| Error::ModuleError(e.to_string()))?;
        Ok((Cow::Owned(schema), DerivedTab { base: ffi::sqlite3_vtab::default(), on }))
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
        Ok(DerivedCursor { base: ffi::sqlite3_vtab_cursor::default(), on: Arc::clone(&self.on), rel: None, id: 0 })
    }
}

#[repr(C)]
pub struct DerivedCursor { base: ffi::sqlite3_vtab_cursor, on: Arc<OnConn>, rel: Option<Arc<InRel>>, id: usize }
unsafe impl VTabCursor for DerivedCursor {
    fn filter(&mut self, _: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        let table: String = args.get(0)?;
        let rel = self.on.derived.lock().get(table.as_str()).cloned().ok_or_else(|| Error::ModuleError(format!("no native relation {table}")))?;
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
/// A relation as `InRel::mark` saw it: enough to find its change later (`delta_since_mark`).
#[derive(Clone, Debug)]
pub struct RelMark { at: usize, n: usize, k: usize, dead: Vec<u64> }

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
        // Declared sorted (decision 0456 K7): laid out in canonical order, unless its rows are numbered as the kept
        // relation it started from numbers them (its versions are read by id, decision 0364).
        if rel.ordered && rel.from.is_none() && rel.lat.is_none() && rel.k > 0 {
            return InRel::in_canonical_order(rel);
        }
        InRel { vers: Default::default(), viewed: false, keyed: None, k: rel.k, n: rel.n, rows: rel.data.freeze(rel.n), view: None, base: false, types: rel.types, stored: rel.stored, idx: Mutex::new(HashMap::default()), chains: rel.idx, sorted: rel.sorted,
                sketches: rel.sketches, serves: rel.serves, dead: rel.dead.into(), ndead: rel.ndead, summaries: rel.summaries, measured: Default::default(), counted: Default::default(), tables: Default::default(), name: std::sync::OnceLock::from(rel.table), from: rel.from, uid: Default::default(), level: rel.level.into(), stats: rel.stats }
    }
    /// `rel`'s live rows in canonical order (its columns in turn, as the keep orders a kept relation, decision 0456
    /// K3): one sort here, so the keep sorts nothing and its readers probe its leading columns as a range of rows in
    /// place (`runs::Sorted` over every row, on its columns but the last: on every column an order is the dedup
    /// order, which leads with a hash). Its levels follow their rows; its orders are made again as probed.
    fn in_canonical_order(rel: Rel) -> InRel {
        let k = rel.k;
        let mut ids: Vec<u32> = (0..rel.n as u32).filter(|&id| !is_dead(&rel.dead, id)).collect();
        ids.sort_unstable_by(|&a, &b| (0..k).map(|c| rel.data.value(a, k, c).cmp(&rel.data.value(b, k, c))).find(|o| o.is_ne()).unwrap_or(std::cmp::Ordering::Equal));
        let n = ids.len();
        let mut flat = Vec::with_capacity(n * k);
        for &id in &ids {
            flat.extend((0..k).map(|c| rel.data.value(id, k, c)));
        }
        let level = if rel.level.len() == rel.n { ids.iter().map(|&id| rel.level[id as usize]).collect() } else { Vec::new() };
        let mut sorted = runs::Sorted::default();
        let lead: Vec<usize> = (0..k.saturating_sub(1).max(1)).collect();
        sorted.note(&flat[..], k, &lead, 0, n as u32);
        InRel { vers: Default::default(), viewed: false, keyed: None, k, n, rows: Rows::frozen(k, flat, n), view: None, base: false, types: rel.types, stored: rel.stored, idx: Mutex::new(HashMap::default()), chains: Vec::new(), sorted,
                sketches: rel.sketches, serves: rel.serves, dead: Shared::default(), ndead: 0, summaries: rel.summaries, measured: Default::default(), counted: Default::default(), tables: Default::default(), name: std::sync::OnceLock::from(rel.table), from: None, uid: Default::default(), level: level.into(), stats: rel.stats }
    }
    /// What `delta_since_mark` needs of this relation later, without holding it: its address, rows and dead rows.
    /// The rows whose columns `cols` hold `key`, each given to `f` (a row: its value of a column), read as a join
    /// step reads this relation (`run_join`'s candidates of `Src::In`): through its own index for `cols`, made once
    /// and kept with the joins' (`index`). A probe neither scans nor sorts the relation; every key column is checked.
    /// The per-key sampling of decision 0232 probes with it.
    pub fn probe_each(&self, cols: &[usize], key: &[i64], f: &mut dyn FnMut(&dyn Fn(usize) -> i64)) {
        let ix = self.index(cols);
        // The index is kept by its set of columns: the key in the index's own column order.
        let mut buf = [0i64; MAX_KEY];
        for (j, c) in ix.cols.iter().enumerate() { buf[j] = key[cols.iter().position(|x| x == c).expect("a key column")]; }
        let (cols, key) = (&ix.cols[..], &buf[..ix.cols.len()]);
        let mut give = |row: RowRef<'_>| if cols.iter().zip(key).all(|(&c, &v)| row.at(c) == v) { f(&|c| row.at(c)) };
        if cols.is_empty() {
            for id in 0..self.len() { if !is_dead(&self.dead, id as u32) { give(self.at(id).read()) } }
        } else if let Some(index) = ix.runs(self) {
            let m = index.cols.iter().take_while(|c| ix.cols.contains(c)).count();
            let mut buf = [0i64; MAX_KEY];
            let at = |c: usize| key[ix.cols.iter().position(|&x| x == c).expect("a key column")];
            let mut hints = [0u32; runs::HINTS];
            Cands::Runs(index.find_at(&self.rows, self.k, permuted(&index.cols[..m], at, &mut buf), None, &mut hints))
                .any(|id| { if !is_dead(&self.dead, id) { give(self.at(id as usize).read()) } false });
        } else if ix.keyed {
            for id in self.keyed_rows(key[ix.lead]) { if !is_dead(&self.dead, id) { give(self.at(id as usize).read()) } }
        } else if ix.composite && let Some(v) = &self.view {
            let eq: Vec<(usize, i64)> = ix.cols.iter().zip(key).map(|(&c, &k)| (v.cols[c], k)).collect();
            for r in v.src.lookup_all(&eq).unwrap_or_default() { give(RowRef::Source(v, r).read()) }
        } else if ix.sorted && let Some(v) = &self.view {
            let (c, k) = (v.cols[ix.cols[ix.lead]], key[ix.lead]);
            match v.src.sorted_range(c, k) {
                Some(range) => for r in range { give(RowRef::Source(v, r as usize).read()) },
                None => for &r in v.src.sorted_rows(c, k).unwrap_or(&[]) { give(RowRef::Source(v, r as usize).read()) },
            }
        } else if ix.source && let Some(v) = &self.view {
            for r in v.find_keyed(&ix.cols, key, ix.lead) { give(RowRef::Source(v, r as usize).read()) }
        } else {
            let exact = ix.exact();
            for &id in ix.get(self, key) {
                if !is_dead(&self.dead, id) && (exact || ix.cols.iter().zip(key).all(|(&c, &v)| self.at(id as usize).at(c) == v)) { give(self.at(id as usize).read()) }
            }
        }
    }

    /// Decision 0236: `probe_each`, sampled where the index gives more than `limit` candidates: `limit` of them are
    /// drawn by position (with replacement, from `draw`), and `f` sees those that match. The matches, estimated as the
    /// candidates times the share drawn that match; exact, with `f` on every match, where there are `limit` or fewer
    /// candidates, or the index cannot be read by position (a source's lookups: those are read whole).
    pub fn probe_sample<'s>(&'s self, cols: &[usize], key: &[i64], limit: usize, draw: &mut dyn FnMut() -> u64, f: &mut dyn FnMut(&dyn Fn(usize) -> i64)) -> f64 {
        let ix = self.index(cols);
        let mut buf = [0i64; MAX_KEY];
        for (j, c) in ix.cols.iter().enumerate() { buf[j] = key[cols.iter().position(|x| x == c).expect("a key column")]; }
        let (icols, ikey) = (&ix.cols[..], &buf[..ix.cols.len()]);
        // The candidates by position: their count, and the `i`th's row where it is live.
        let mut run = |n: usize, row: &dyn Fn(usize) -> Option<RowRef<'s>>| -> f64 {
            let mut m = 0u64;
            let mut one = |i: usize| if let Some(r) = row(i) && icols.iter().zip(ikey).all(|(&c, &v)| r.at(c) == v) { m += 1; f(&|c| r.at(c)) };
            if n <= limit {
                (0..n).for_each(&mut one);
                return m as f64;
            }
            for _ in 0..limit { one((draw() % n as u64) as usize) }
            n as f64 * m as f64 / limit as f64
        };
        let live = |id: u32| (!is_dead(&self.dead, id)).then(|| self.at(id as usize).read());
        if icols.is_empty() {
            run(self.len(), &|i| live(i as u32))
        } else if let Some(index) = ix.runs(self) {
            let m = index.cols.iter().take_while(|c| ix.cols.contains(c)).count();
            let mut pbuf = [0i64; MAX_KEY];
            let at = |c: usize| ikey[ix.cols.iter().position(|&x| x == c).expect("a key column")];
            let mut hints = [0u32; runs::HINTS];
            let found = index.find_at(&self.rows, self.k, permuted(&index.cols[..m], at, &mut pbuf), None, &mut hints);
            run(found.len(), &|i| live(found.nth(i)))
        } else if ix.keyed {
            let rows = self.keyed_rows(ikey[ix.lead]);
            run(rows.len(), &|i| live(rows.start + i as u32))
        } else if !ix.composite && ix.sorted && let Some(v) = &self.view {
            let (c, k) = (v.cols[ix.cols[ix.lead]], ikey[ix.lead]);
            if let Some(range) = v.src.sorted_range(c, k) {
                run(range.len(), &|i| Some(RowRef::Source(v, range.start as usize + i).read()))
            } else {
                let rows = v.src.sorted_rows(c, k).unwrap_or(&[]);
                run(rows.len(), &|i| Some(RowRef::Source(v, rows[i] as usize).read()))
            }
        } else if !ix.composite && !ix.source || self.view.is_none() {
            let ids = ix.get(self, ikey);
            run(ids.len(), &|i| live(ids[i]))
        } else {
            let mut m = 0u64;
            self.probe_each(cols, key, &mut |r| { m += 1; f(r) });
            m as f64
        }
    }

    /// Decision 0247: how many candidates a probe by `key` at `cols` reads before testing them, without reading
    /// them: at least its matches, and exact where the index holds the key's columns alone. A walk weighs a
    /// binding's next step by it.
    pub fn probe_size(&self, cols: &[usize], key: &[i64]) -> f64 {
        let ix = self.index(cols);
        let mut buf = [0i64; MAX_KEY];
        for (j, c) in ix.cols.iter().enumerate() { buf[j] = key[cols.iter().position(|x| x == c).expect("a key column")]; }
        let (icols, ikey) = (&ix.cols[..], &buf[..ix.cols.len()]);
        if icols.is_empty() {
            self.len() as f64
        } else if let Some(index) = ix.runs(self) {
            let m = index.cols.iter().take_while(|c| ix.cols.contains(c)).count();
            let mut pbuf = [0i64; MAX_KEY];
            let at = |c: usize| ikey[ix.cols.iter().position(|&x| x == c).expect("a key column")];
            let mut hints = [0u32; runs::HINTS];
            index.find_at(&self.rows, self.k, permuted(&index.cols[..m], at, &mut pbuf), None, &mut hints).len() as f64
        } else if ix.keyed {
            self.keyed_rows(ikey[ix.lead]).len() as f64
        } else if !ix.composite && ix.sorted && let Some(v) = &self.view {
            let (c, k) = (v.cols[ix.cols[ix.lead]], ikey[ix.lead]);
            v.src.sorted_range(c, k).map_or_else(|| v.src.sorted_rows(c, k).map_or(0, |r| r.len()), |r| r.len()) as f64
        } else if !ix.composite && !ix.source || self.view.is_none() {
            ix.get(self, ikey).len() as f64
        } else {
            let mut m = 0.0;
            self.probe_each(cols, key, &mut |_| m += 1.0);
            m
        }
    }

    pub fn mark(&self) -> RelMark {
        RelMark { at: self as *const InRel as usize, n: self.len(), k: self.k, dead: self.dead.to_vec() }
    }
    /// Its rows' change from `old`, the kept relation it started as (`Rel::based`), as `(added, removed)` row-major:
    /// the rows it added after `old`'s, and `old`'s live rows it marked dead. A row deleted and derived again is in
    /// both. Costs the change and a pass over the dead bits, not the rows. `None` if it did not start as `old`.
    /// A test's view of it: the engine reads a change where it lies (`delta_ids_since`).
    #[cfg(test)]
    pub fn delta_since(&self, old: &InRel) -> Option<(Vec<i64>, Vec<i64>)> {
        if self.k != old.k || self.view.is_some() {
            return None;
        }
        // The same relation, held on unchanged (a quiet stratum's, decision 0101 §1): no change.
        if std::ptr::eq(self, old) {
            return Some((Vec::new(), Vec::new()));
        }
        self.delta_from(old as *const InRel as usize, old.len())
    }
    /// Its rows gone and added since its version at the run's start, counted, none copied: `None` without a version.
    pub fn version_counts(&self) -> Option<(usize, usize)> {
        Some((self.version_ids(Ver::Minus)?.len(), self.version_ids(Ver::Plus)?.len()))
    }
    /// Its change from its version at the run's start (decision 0364), `(added, removed)` row-major: `None` without
    /// a version. Only a frame's delta is made from it (`plan::Delta::rows`): the rows are that frame's bytes, written
    /// to the store, not a change kept in memory, so they are not counted as copied (0364 R2).
    pub fn version_delta(&self) -> Option<(Vec<i64>, Vec<i64>)> {
        let rows_of = |ids: Vec<u32>| ids.into_iter().fold(Vec::new(), |mut v, id| { v.extend_from_slice(&self.row(id as usize)); v });
        Some((rows_of(self.version_ids(Ver::Plus)?), rows_of(self.version_ids(Ver::Minus)?)))
    }
    /// `old` made `new` in place (0364 R2, R4): a stratum derived again past θ keeps its kept relation. The rows it no
    /// longer derives are marked dead and those it newly derives added, so its change is its version (its marks),
    /// which its readers read in place, as any maintained relation's, not rows diffed into delta relations. Every row
    /// takes `new`'s level (R4: a derive again resets levels): where `new` has none, the kept levels are dropped;
    /// where `old` had none, every live row is then one `new` derives, found or added, and takes its level. `None`
    /// where `old` cannot be started from (`Rel::based`), or the two differ in width.
    pub fn rebased(table: &str, old: &InRel, new: &InRel) -> Option<InRel> {
        let levels = new.has_levels();
        if old.k != new.k || new.view.is_some() {
            return None;
        }
        let types = old.types().to_vec();
        // Too many of its rows dead to start from in place: compacted first (its live rows, collected), so the
        // version starts from those, and the change is still marks against them.
        let mut rel = match Rel::based(table, &types, old, None) {
            Some(rel) => rel,
            None if old.view.is_none() => Rel::compacted(table, &types, old, None),
            None => return None,
        };
        if !levels {
            rel.level.clear();
        } else if rel.level.len() != rel.n {
            rel.level = vec![0; rel.n];
        }
        let start = rel.n;
        let k = new.k;
        let mut found = vec![false; start];
        let mut row = vec![0i64; k];
        for j in 0..new.n {
            if is_dead(&new.dead, j as u32) {
                continue;
            }
            let r = new.at(j);
            for (c, v) in row.iter_mut().enumerate() {
                *v = r.at(c);
            }
            let level = if levels { new.level[j] } else { 0 };
            match rel.find(&row, |id| rel.dead(id)) {
                Some(id) if (id as usize) < start => {
                    found[id as usize] = true;
                    if levels {
                        rel.level[id as usize] = level;
                    }
                }
                Some(_) => {}
                None => {
                    if rel.push(&row) && levels {
                        *rel.level.last_mut().expect("a level pushed") = level;
                    }
                }
            }
        }
        for (id, &kept) in found.iter().enumerate() {
            if !kept && kill(&mut rel.dead, id as u32) {
                rel.ndead += 1;
            }
        }
        Some(InRel::moved(rel))
    }
    /// `delta_since_mark`'s change, as the ids of the rows it names (`DeltaIds`): read where they lie, none copied
    /// (0364 R2). `None` where it did not start as the marked relation.
    pub fn delta_ids_since_mark(&self, old: &RelMark) -> Option<DeltaIds<'_>> {
        if self.k != old.k || self.view.is_some() {
            return None;
        }
        if self as *const InRel as usize == old.at && self.n == old.n && *self.dead == *old.dead {
            return Some(DeltaIds { rel: self, gone: Gone::Ids(Vec::new()), added: Vec::new() });
        }
        self.ids_from(old.at, old.n)
    }
    /// `delta_since`'s change, as the ids of the rows it names (`DeltaIds`): none copied (0364 R2).
    pub fn delta_ids_since(&self, old: &InRel) -> Option<DeltaIds<'_>> {
        if self.k != old.k || self.view.is_some() {
            return None;
        }
        if std::ptr::eq(self, old) {
            return Some(DeltaIds { rel: self, gone: Gone::Ids(Vec::new()), added: Vec::new() });
        }
        self.ids_from(old as *const InRel as usize, old.n)
    }
    fn ids_from(&self, at: usize, n: usize) -> Option<DeltaIds<'_>> {
        match &self.from {
            Some(Origin::Based { at: a, n: n0, .. }) if *a == at && *n0 == n => {
                Some(DeltaIds { rel: self, gone: Gone::Ids(self.version_ids(Ver::Minus)?), added: self.version_ids(Ver::Plus)? })
            }
            // Compacted with nothing removed (a version started from the kept relation's live rows, `rebased`): its
            // marks are its change, as a based relation's.
            Some(Origin::Compacted { at: a, old_n, removed, .. }) if *a == at && *old_n == n && removed.is_empty() => {
                Some(DeltaIds { rel: self, gone: Gone::Ids(self.version_ids(Ver::Minus)?), added: self.version_ids(Ver::Plus)? })
            }
            Some(Origin::Compacted { at: a, old_n, kept, removed }) if *a == at && *old_n == n => {
                let added = (*kept..self.n).filter(|&id| !is_dead(&self.dead, id as u32)).map(|id| id as u32).collect();
                Some(DeltaIds { rel: self, gone: Gone::Rows(removed), added })
            }
            _ => None,
        }
    }
    /// `delta_since`'s change from the relation at `at` of `n` rows it started as: read from its version (decision
    /// 0364 stage 2), the rows live at the start and dead now and those added since; from its survivors' record
    /// when it was compacted.
    #[cfg(test)]
    fn delta_from(&self, at: usize, n: usize) -> Option<(Vec<i64>, Vec<i64>)> {
        match &self.from {
            Some(Origin::Based { at: a, n: n0, .. }) if *a == at && *n0 == n => {
                let rows_of = |ids: Vec<u32>| { crate::code_rule_dsl::logic::plan::corpus::maintain::shape::rows_copied(ids.len()); ids.into_iter().fold(Vec::new(), |mut v, id| { v.extend_from_slice(&self.row(id as usize)); v }) };
                Some((rows_of(self.version_ids(Ver::Plus)?), rows_of(self.version_ids(Ver::Minus)?)))
            }
            Some(Origin::Compacted { at: a, old_n, kept, removed }) if *a == at && *old_n == n => {
                let added = (*kept..self.n).filter(|&id| !is_dead(&self.dead, id as u32)).fold(Vec::new(), |mut v, id| { v.extend_from_slice(&self.row(id)); v });
                crate::code_rule_dsl::logic::plan::corpus::maintain::shape::rows_copied(added.len() / self.k.max(1) + removed.len() / self.k.max(1));
                Some((added, removed.clone()))
            }
            _ => None,
        }
    }
    /// Column `c`'s postings, from the index the engine kept on it (`Rel::new`): each value with the first-column
    /// values of its live rows (a row's document, as `syntax::measured` reads it), so nothing is hashed again. A
    /// posting of one live row is handed on as it is; a longer one is walked. `false`, and nothing visited, for a
    /// relation without that index (a view, or rows loaded flat).
    pub fn each_posting(&self, c: usize, visit: &mut dyn FnMut(i64, &[i64])) -> bool {
        if self.view.is_some() || c >= self.k {
            return false;
        }
        if probed::on() { probed::note(&format!("{} [postings]", self.name.get().map_or("?", String::as_str)), std::iter::once(c)); }
        let Some(ix) = self.chains.iter().find(|ix| ix.cols.first() == Some(&c)).filter(|ix| ix.rows() >= self.n) else { return false };
        let (k, data) = (self.k, &self.rows);
        let mut firsts: Vec<i64> = Vec::new();
        ix.each_group(data, k, 1, |ids| {
            let v = data.row(ids.first(), k)[c];
            firsts.clear();
            firsts.extend(ids.iter().filter(|&id| !is_dead(&self.dead, id)).map(|id| data.value(id, k, 0)));
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
    /// Its live rows of ids `lo..hi`, row-major, read where they lie a batch at a time into one scratch of at most
    /// `LIVE_BATCH` rows (`f` per batch): a frame's digest and bytes are made from these, with no copy of the relation
    /// (`retain::held_frame`). A view's rows are read through it, where they lie (a kept relation bound in place,
    /// decision 0456 step 6b, has no rows of its own and no dead ones).
    pub fn each_live_batch(&self, lo: usize, hi: usize, mut f: impl FnMut(&[i64])) -> Option<()> {
        let k = self.k;
        // A view's rows read through it where they lie (a kept relation bound in place, decision 0456 step 6).
        if self.view.is_some() {
            let hi = hi.min(self.len());
            // copy: not block data: a batch of a view's values handed to the reader, never kept.
            let mut buf: Vec<i64> = Vec::with_capacity(LIVE_BATCH.min(hi.saturating_sub(lo)) * k);
            let mut id = lo;
            while id < hi {
                let end = (id + LIVE_BATCH).min(hi);
                buf.clear();
                for i in id..end {
                    let r = self.at(i);
                    buf.extend((0..k).map(|c| r.at(c)));
                }
                f(&buf);
                id = end;
            }
            return Some(());
        }
        let hi = hi.min(self.n);
        let mut buf: Vec<i64> = Vec::with_capacity(LIVE_BATCH.min(hi.saturating_sub(lo)) * k);
        let mut id = lo;
        while id < hi {
            let end = (id + LIVE_BATCH).min(hi);
            buf.clear();
            // Part by part: each part found once, its rows read in order.
            self.rows.each_run(id as u32, end as u32, |p, from, to, first| {
                for i in from..to {
                    if self.ndead == 0 || !is_dead(&self.dead, first + (i - from)) {
                        buf.extend((0..k).map(|c| p.get(i, c)));
                    }
                }
            });
            f(&buf);
            id = end;
        }
        Some(())
    }
    /// Its live rows, row-major (a view's read through it, `each_live_batch`).
    pub fn live_rows(&self) -> Option<Vec<i64>> {
        if self.view.is_some() {
            let mut out = Vec::with_capacity(self.len() * self.k);
            // copy: load: a view's rows for a caller that takes them flat.
            self.each_live_batch(0, self.len(), |b| out.extend_from_slice(b))?;
            return Some(out);
        }
        if self.ndead == 0 {
            return Some(self.rows.to_vec(0, self.n, self.k));
        }
        let mut out = Vec::with_capacity((self.n - self.ndead) * self.k);
        for id in 0..self.n {
            if !is_dead(&self.dead, id as u32) {
                out.extend_from_slice(&self.row(id));
            }
        }
        Some(out)
    }
    /// No rows, `k` columns.
    fn empty(k: usize) -> InRel {
        InRel { vers: Default::default(), viewed: false, keyed: None, k, n: 0, rows: Rows::new(k), view: None, base: false, types: vec![b'i'; k], stored: 0, idx: Mutex::new(HashMap::default()), chains: Vec::new(), sorted: Default::default(), sketches: Vec::new(), serves: Vec::new(), dead: Shared::default(), ndead: 0, summaries: Default::default(), measured: Default::default(), counted: Default::default(), tables: Default::default(), name: Default::default(), from: None, uid: Default::default(), level: Shared::default(), stats: None }
    }
    /// Rows, levels held, and whether it is a view (`RERAIL_LEVELS_WHY`).
    pub fn levels_shape(&self) -> (usize, usize, bool) {
        (self.n, self.level.len(), self.view.is_some())
    }
    /// Each row's level, every row's (dead ones too), when it has them (`has_levels`); none otherwise.
    pub fn levels(&self) -> &[u32] {
        if self.has_levels() { &self.level } else { &[] }
    }
    /// Its dead rows' bits (bit `id`); none when no row is dead.
    pub fn dead_bits(&self) -> &[u64] {
        if self.ndead > 0 { &self.dead } else { &[] }
    }
    /// Whether every row has its level (a relation the native engine derived; decision 0090's levels).
    /// Whether it is read through a view (`Some`), not rows of its own.
    pub fn view_kind(&self) -> Option<()> {
        self.view.as_ref().map(|_| ())
    }
    pub fn has_levels(&self) -> bool {
        self.view.is_none() && self.level.len() == self.n
    }
    /// A whole view's rows are its source's, counted when first asked: a source that makes its rows on lookup
    /// (`syntax::within`) makes them all only for a scan.
    pub fn len(&self) -> usize {
        match &self.view { Some(v) if v.ids.is_none() => v.src.len(), _ => self.n }
    }
    /// A flat relation's row (a derived one: views are base inputs, or an index's relation, `hold_source`).
    pub fn row(&self, id: usize) -> Row { self.rows.row(id as u32, self.k) }
    /// A flat relation's rows in one block, for a reader that binds its block (the device, decision 0325): none for
    /// a view, a relation with dead rows (a reader skips no rows yet) or no columns.
    /// Its version at the run's start (decision 0364 stage 2): its row count then and its dead rows then, when it
    /// started from a kept relation (`Rel::based`, or `Rel::compacted` with no rows gone). `None` for one made from
    /// rows, or compacted with rows gone (those are in no row of it).
    pub fn start(&self) -> Option<(usize, &[u64])> {
        match &self.from {
            Some(Origin::Based { n, dead, .. }) if self.view.is_none() => Some((*n, dead.as_slice())),
            // Compacted with no rows gone (the maintain mode's start, decision 0364): its first `kept` rows are the
            // kept relation's live ones, every one live at the start.
            Some(Origin::Compacted { kept, removed, .. }) if self.view.is_none() && removed.is_empty() => Some((*kept, &[])),
            _ => None,
        }
    }
    /// The ids of its version `ver` that are not the relation's own live rows (`Ver::Minus`: rows live at the
    /// start and dead now; `Ver::Plus`: rows added since, live), ascending. `None` without a version, or for `Old`.
    pub fn version_ids(&self, ver: Ver) -> Option<Vec<u32>> {
        let (n0, dead0) = self.start()?;
        match ver {
            Ver::Old => None,
            Ver::Minus => {
                let mut out = Vec::new();
                for (w, &now) in self.dead.iter().enumerate().take(n0.div_ceil(64)) {
                    let mut gone = now & !dead0.get(w).copied().unwrap_or(0);
                    while gone != 0 {
                        let id = w * 64 + gone.trailing_zeros() as usize;
                        gone &= gone - 1;
                        if id < n0 {
                            out.push(id as u32);
                        }
                    }
                }
                Some(out)
            }
            Ver::Plus => Some((n0..self.n).filter(|&id| !is_dead(&self.dead, id as u32)).map(|id| id as u32).collect()),
        }
    }
    /// Its version `ver` as a relation read in place, no row copied (decision 0364 stage 2): `Old`, its first rows
    /// with the start's dead rows (its rows shared); `Minus` and `Plus`, a view of their ids over it. `None` without
    /// a version (a relation made from rows, or a view).
    pub fn version_view(self: &Arc<Self>, ver: Ver) -> Option<Arc<InRel>> {
        self.vers[ver as usize].get_or_init(|| self.version_made(ver)).clone()
    }
    fn version_made(self: &Arc<Self>, ver: Ver) -> Option<Arc<InRel>> {
        let (n0, dead0) = self.start()?;
        match ver {
            Ver::Old => {
                let mut old = InRel::held(self.rows.shared(n0), n0, self.types.clone(), dead0.to_vec());
                // Its rows' levels as they were (a recursive stratum's, decision 0364): a level is never lowered in
                // place, so the first `n0` are the start's.
                if self.has_levels() {
                    Arc::get_mut(&mut old).expect("just made").level = self.level[..n0].to_vec().into();
                }
                Some(old)
            }
            Ver::Minus | Ver::Plus => {
                let ids = self.version_ids(ver)?;
                let k = self.k;
                let names: Vec<String> = (0..k).map(|c| format!("c{c}")).collect();
                // Over its rows with no dead marks (shared, not copied), so a view's ids are the relation's own: a
                // source of it would number its live rows, and a row gone is not among them.
                let every = InRel::held(self.rows.shared(self.n), self.n, self.types.clone(), Vec::new());
                let view = View { src: every.source(names), cols: (0..k).collect(), ids: Some(ids), eqs: Vec::new() };
                let n = view.ids.as_ref().map_or(0, Vec::len);
                Some(Arc::new(InRel { vers: Default::default(), viewed: false, keyed: None, k, n, rows: Rows::new(k), view: Some(view), base: false, types: self.types.clone(), stored: 0, idx: Mutex::new(HashMap::default()), chains: Vec::new(), sorted: Default::default(), sketches: Vec::new(), serves: Vec::new(), dead: Shared::default(), ndead: 0, summaries: Default::default(), measured: Default::default(), counted: Default::default(), tables: Default::default(), name: Default::default(), from: None, uid: Default::default(), level: Shared::default(), stats: None }))
            }
        }
    }
    /// Rows `ids` of `src`, read where they lie, as a source of columns `names` (0364 R2): an input's change named
    /// by its rows' places in its kept frames and its appended rows, not copied out.
    pub fn picked(src: Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, ids: Vec<u32>, names: Vec<String>) -> Arc<dyn crate::code_rule_dsl::facts::vtab::Source> {
        let k = names.len();
        let n = ids.len();
        let view = View { src, cols: (0..k).collect(), ids: Some(ids), eqs: Vec::new() };
        let rel = Arc::new(InRel { vers: Default::default(), viewed: false, keyed: None, k, n, rows: Rows::new(k), view: Some(view), base: false, types: vec![b'i'; k], stored: 0, idx: Mutex::new(HashMap::default()), chains: Vec::new(), sorted: Default::default(), sketches: Vec::new(), serves: Vec::new(), dead: Shared::default(), ndead: 0, summaries: Default::default(), measured: Default::default(), counted: Default::default(), tables: Default::default(), name: Default::default(), from: None, uid: Default::default(), level: Shared::default(), stats: None });
        rel.source(names)
    }
    /// A relation read in place from every row of `src`, its columns in order (`hold_source`).
    pub fn over_source(src: Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, types: Vec<u8>) -> Arc<InRel> {
        let k = types.len();
        // Rows that lie in an order (a kept relation's canonical one) are known sorted: a scan of them for the next
        // step's probes is not sorted again (decision 0187).
        let sorted = src.order().map_or_else(runs::Sorted::default, |cols| runs::Sorted::whole(src.len() as u32, k, &cols));
        let view = View { src, cols: (0..k).collect(), ids: None, eqs: Vec::new() };
        Arc::new(InRel { vers: Default::default(), viewed: false, keyed: None, k, n: 0, rows: Rows::new(k), view: Some(view), base: false, types, stored: 0, idx: Mutex::new(HashMap::default()), chains: Vec::new(), sorted, sketches: Vec::new(), serves: Vec::new(), dead: Shared::default(), ndead: 0, summaries: Default::default(), measured: Default::default(), counted: Default::default(), tables: Default::default(), name: Default::default(), from: None, uid: Default::default(), level: Shared::default(), stats: None })
    }
    pub fn in_block(&self) -> Option<InBlock> {
        if self.view.is_some() || self.k == 0 {
            return None;
        }
        let b = if self.ndead > 0 { self.rows.in_block_dead(&self.dead) } else { self.rows.in_block() };
        b.filter(|b| b.rows as usize == self.n)
    }
    /// Its rows, dead ones too, as one relation in one block (`Rows::in_block`): for a reader that takes its dead bits
    /// apart (`dead_bits`), as a kept entry's writer does. `in_block` writes the dead rows' delta into that block,
    /// which a kept relation bound over its read-only mapping (decision 0401 F3) cannot take.
    pub fn rows_in_block(&self) -> Option<InBlock> {
        if self.view.is_some() || self.k == 0 {
            return None;
        }
        self.rows.in_block().filter(|b| b.rows as usize == self.n)
    }
    /// Row `id`'s value in column `c`, read where it lies: its own rows', or a view's through it (a kept relation
    /// bound in place, decision 0456 step 6).
    pub fn value(&self, id: usize, c: usize) -> i64 {
        match &self.view { None => self.rows.value(id as u32, self.k, c), Some(_) => self.at(id).at(c) }
    }
    /// The row whose `keyed` column is `v`: the source's, or its run (the last starting at or under `v`), then its
    /// row in the run, by binary searches of that column read where it lies. None or one.
    fn keyed_rows(&self, v: i64) -> std::ops::Range<u32> {
        let ky = self.keyed.as_ref().expect("a keyed relation");
        if let Some((src, sc)) = &ky.src {
            return match src.key_row(*sc, v) { Some(r) => r as u32..r as u32 + 1, None => 0..0 };
        }
        let (k, c) = (self.k, ky.col);
        let i = ky.runs.partition_point(|r| r.0 <= v);
        let Some(&(_, start, end)) = i.checked_sub(1).map(|i| &ky.runs[i]) else { return 0..0 };
        let (mut lo, mut hi) = (start, end);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.rows.value(mid, k, c) < v { lo = mid + 1 } else { hi = mid }
        }
        if lo < end && self.rows.value(lo, k, c) == v { lo..lo + 1 } else { lo..lo }
    }
    /// Row `id`'s values, flat or read from its view.
    pub fn values(&self, id: usize) -> Row {
        match &self.view {
            None => self.row(id),
            Some(_) => { let r = self.at(id).read(); Row::from_fn(self.k, |c| r.at(c)) }
        }
    }
    /// Whether it is a kept relation read where it lies (a view of a `Source::kept` source): its rows are a relation's,
    /// kept as a relation the run holds is, not made again as a view of an input's columns is.
    pub fn kept_in_place(&self) -> bool {
        self.view.as_ref().is_some_and(|v| v.src.kept())
    }
    /// The source a derived relation is read from in place, every row and column in order (`hold_source`); `None`
    /// for rows held flat, and for a view of some of its source's columns (`hold_view`), whose column numbers are
    /// not the source's.
    pub fn held_source(&self) -> Option<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>> {
        self.view
            .as_ref()
            .filter(|v| !self.base && v.ids.is_none() && v.eqs.is_empty() && v.cols.iter().enumerate().all(|(i, &c)| i == c) && v.src.columns().len() == v.cols.len())
            .map(|v| Arc::clone(&v.src))
    }
    #[inline]
    pub fn at(&self, id: usize) -> RowRef<'_> {
        match &self.view { Some(v) => RowRef::View(v, id), None => RowRef::Rows(&self.rows, id as u32, self.k) }
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
        let doc = |id: u32| data.value(id, k, 0) >> 32;
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
                docs.extend(ids.iter().filter(|&id| !is_dead(&self.dead, id)).map(&doc));
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
    /// `index(cols)` for a probe that reads a range of column `c` (`Ranged`): where that index is a derived input's
    /// sorted order made on its first probe, one on `cols` then `c`, so a key's rows are in `c`'s order. Otherwise the
    /// plain index (a range is read where its order happens to hold `c` next, else the comparisons filter).
    fn index_then(&self, cols: &[usize], c: usize) -> Arc<Csr> {
        let plain = self.index(cols);
        if plain.ordered.is_none() || cols.contains(&c) || c >= 56 {
            return plain;
        }
        let mask = cols.iter().fold(0u64, |m, &x| m | 1 << x) | (c as u64 + 1) << 56;
        let mut idx = self.idx.lock().unwrap();
        idx.entry(mask).or_insert_with(|| {
            run_counts().orders_opened.fetch_add(1, AtomicOrdering::Relaxed);
            Arc::new(Csr { cols: cols.to_vec(), table: HashTable::new(), ids: Vec::new(), source: false, sorted: false, chain: None, lead: 0, composite: false, keyed: false, ordered: Some(false), order: OnceLock::new(), block: None, then: Some(c) })
        }).clone()
    }
    fn index(&self, cols: &[usize]) -> Arc<Csr> {
        let mask = cols.iter().fold(0u64, |m, &c| m | 1 << c);
        let mut idx = self.idx.lock().unwrap();
        idx.entry(mask).or_insert_with(|| {
            let base = self.base;
            let _index = crate::code_rule_dsl::logic::profile::span(move || (if base { "open: index of a base input" } else { "open: index of a derived input" }).into());
            let (t0, c0) = (Instant::now(), thread_cpu());
            // No key columns: the step reads every row, not an index.
            let empty = |source, sorted| Csr { cols: cols.to_vec(), table: HashTable::new(), ids: Vec::new(), source, sorted, chain: None, lead: 0, composite: false, keyed: false, ordered: None, order: OnceLock::new(), block: None, then: None };
            let m = if cols.is_empty() {
                empty(false, false)
            } else if let Some(i) = self.chains.iter().position(|ix| ix.answers(cols)) {
                Csr { chain: Some(i), ..empty(false, false) }
            } else if let Some(lead) = self.keyed.as_ref().and_then(|ky| cols.iter().position(|&x| x == ky.col)) {
                Csr { keyed: true, lead, ..empty(false, false) }
            } else if let (Some(v), [c]) = (&self.view, cols) && v.ids.is_none() && v.eqs.is_empty() && v.src.sorts_rows(v.cols[*c]) {
                empty(false, true)
            } else if let Some(v) = self.view.as_ref().filter(|v| v.ids.is_none() && v.eqs.is_empty())
                && let Some((ib, i)) = v.src.own_index(&cols.iter().map(|&c| v.cols[c]).collect::<Vec<_>>())
            {
                // A whole source held as one block relation to be probed so (`held`): its own index, found or
                // built there and shared with the device (decision 0325 step 5), not a host table or a lookup a probe.
                Csr { block: Some((ib, i)), ..empty(false, false) }
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
                // `RERAIL_KEY_PRINTS` (measurement aids): `off` probes the sorted order by its rows, `hash` builds the hash
                // table of before decision 0112's derived-input order.
                // 0: prints (the default), 1: `off`, 2: `hash`.
                let mode = rerail_prof::code_rule_dsl::logic::profile::setting(|| match std::env::var("RERAIL_KEY_PRINTS").as_deref() { Ok("off") => 1, Ok("hash") => 2, _ => 0 });
                if self.view.is_none() && mode != 2 {
                    // A derived relation's rows are its own: a sorted order on the key columns (dead rows are in
                    // it, as in a kept chain; readers skip them). Its probes give whole keys, so it keeps a print a
                    // row and a probe reads prints, not rows (`runs::print_span`).
                    // It is made on its first probe (`Csr::runs`), unless `RERAIL_LAZY_ORDERS=off` (a measurement aid).
                    let csr = Csr { ordered: Some(mode != 1), ..empty(false, false) };
                    run_counts().orders_opened.fetch_add(1, AtomicOrdering::Relaxed);
                    if rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_LAZY_ORDERS").is_ok_and(|v| v == "off")) { csr.runs(self); }
                    csr
                } else if let Some(v) = self.view.as_ref().filter(|v| v.ids.is_none() && v.eqs.is_empty())
                    && let Some(ib) = v.src.in_block().filter(|ib| ib.rows as usize == v.src.len())
                    && let Some(i) = ib.own_index(&cols.iter().map(|&c| v.cols[c] as u16).collect::<Vec<_>>())
                {
                    // A whole source in one block relation: that relation's own index, found or built there.
                    Csr { block: Some((ib, i)), ..empty(false, false) }
                } else {
                    charge(Unit::IndexEntry, self.len() as u64);
                    Csr::build(self, cols)
                }
            };
            if env_set!("RERAIL_SPIKE_INDEX") {
                eprintln!("O125 index base={} took {:.3} ms rows={} cols={cols:?} chain={} source={} sorted={} lead={} view={} of {:?}", self.base, (thread_cpu() - c0) * 1e3, self.len(), m.chain.is_some(), m.source, m.sorted, m.lead, self.view.is_some(), self.view.as_ref().map(|v| (v.src.columns(), v.cols.clone())));
            }
            loaded(usize::from(!self.base), thread_cpu() - c0, 0);
            if env_set!("RDL_PROFILE") {
                eprintln!("      index on {cols:?} of {} rows in {:.1} ms", self.len(), t0.elapsed().as_secs_f64() * 1e3);
            }
            Arc::new(m)
        }).clone()
    }
}
// Input relations loaded for joins (`OnConn::inputs`), by spec, for the run that loaded them: the base relations are
// vtabs rebound per batch, so a load is not reused across runs (`forget_inputs`). A connection's, not its database's:
// connections on one database bind sources of their own (a run's chunks evaluating at once, decision 0254), and one's
// run must neither read nor drop another's. T cached them for its process, which ran one program over one facts file.

/// Drops the input relations loaded on `conn`'s database: a run starts over the relations bound now.
pub fn forget_inputs(conn: &Connection) {
    let path = conn_path(conn).to_string();
    let on = on(conn);
    let changing = on.changing.lock().clone();
    let Some(changing) = changing else {
        on.inputs.lock().clear();
        crate::code_rule_dsl::logic::profile::shared::<StoredTables>().0.lock().unwrap().retain(|(p, _), _| *p != path);
        return;
    };
    // Kept: base inputs (`b:` specs) that read none of the relations that change between runs; their rows and
    // indexes are the same in the next run. Derived inputs (`d:`, a SQL stratum's table) are made again.
    let reads_changing = |sel: &str| sel.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).any(|w| changing.contains(w));
    on.inputs.lock().retain(|key, _| key.base && !reads_changing(&key.sql));
    crate::code_rule_dsl::logic::profile::shared::<StoredTables>().0.lock().unwrap().retain(|(p, table), _| *p != path || (!table.starts_with("main.") && !table.starts_with("temp.") && !reads_changing(table)));
}

// Relations whose rows change between the native runs on a connection (`OnConn::changing`, `keep_inputs`): while
// set, `forget_inputs` keeps every other base input loaded, with its indexes.

/// Runs on `conn` that read the same inputs but for `changing` (the bounded rounds: each round rebinds only its
/// `__prev` inputs) keep the others loaded; `None` ends it, and the next `forget_inputs` drops everything.
pub fn keep_inputs(conn: &Connection, changing: Option<std::collections::BTreeSet<String>>) {
    *on(conn).changing.lock() = changing;
}

/// An input from its spec's text (`<b|d|x>:<hex select>:<hex columns>:<types>`).
fn input(conn: &Connection, spec: &str, syms: &mut Interner) -> Result<Arc<InRel>> {
    input_of(conn, &InputKey::parse(spec).map_err(err)?, syms)
}

fn input_of(conn: impl Scoped, spec: &InputKey, syms: &mut Interner) -> Result<Arc<InRel>> {
    let conn = conn.db();
    let on = on(&conn);
    if let Some(r) = on.inputs.lock().get(spec) { return Ok(r.clone()); }
    let keep = |rel: &Arc<InRel>| { on.inputs.lock().insert(spec.clone(), rel.clone()); };
    let _load = crate::code_rule_dsl::logic::profile::span(|| "open: input load".into());
    let base = spec.base;
    let (table, cols, types) = (&spec.sql, &spec.cols, spec.types.as_bytes());
    let (t0, c0) = (Instant::now(), thread_cpu());
    // A stored table holds symbols as atoms (integers), so any column types read in place there.
    if base && (types.iter().all(|&t| t == b'i') || !table.starts_with("main.")) {
        if let Some(view) = view(conn, &table, &cols) {
            // A held relation bound whole (an export the chunks handed on): its rows bound where they lie, as the
            // relation's own, not read a value at a time through its source.
            if view.ids.is_none() && view.eqs.is_empty() && view.cols.iter().enumerate().all(|(i, &c)| i == c)
                && let Some(held) = held_rel(view.src.as_ref(), types.len())
            {
                let rel = Arc::new(InRel { vers: Default::default(), viewed: false, keyed: None, k: held.k, n: held.n, rows: held.rows.clone(), view: None, base, types: types.to_vec(), stored: 0, idx: Mutex::new(HashMap::default()), chains: Vec::new(), sorted: Default::default(), sketches: Vec::new(), serves: Vec::new(), dead: held.dead.clone(), ndead: held.ndead, summaries: Default::default(), measured: Default::default(), counted: Default::default(), tables: Default::default(), name: Default::default(), from: None, uid: Default::default(), level: Shared::default(), stats: None });
                loaded(0, thread_cpu() - c0, 0);
                keep(&rel);
                return Ok(rel);
            }
            let n = view.ids.as_ref().map_or(0, Vec::len);
            let rel = Arc::new(InRel { vers: Default::default(), viewed: false, keyed: None, k: types.len(), n, rows: Rows::new(types.len()), view: Some(view), base, types: types.to_vec(), stored: 0, idx: Mutex::new(HashMap::default()), chains: Vec::new(), sorted: Default::default(), sketches: Vec::new(), serves: Vec::new(), dead: Shared::default(), ndead: 0, summaries: Default::default(), measured: Default::default(), counted: Default::default(), tables: Default::default(), name: Default::default(), from: None, uid: Default::default(), level: Shared::default(), stats: None });
            loaded(0, thread_cpu() - c0, 0);
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
    let rel = Arc::new(InRel { vers: Default::default(), viewed: false, keyed: None, k: types.len(), n, rows: Rows::frozen(types.len(), rows, n), view: None, base, types: types.to_vec(), stored: 0, idx: Mutex::new(HashMap::default()), chains: Vec::new(), sorted: Default::default(), sketches: Vec::new(), serves: Vec::new(), dead: Shared::default(), ndead: 0, summaries: Default::default(), measured: Default::default(), counted: Default::default(), tables: Default::default(), name: Default::default(), from: None, uid: Default::default(), level: Shared::default(), stats: None });
    loaded(usize::from(!base), thread_cpu() - c0, rel.len() as u64);
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
    // The name as the statement writes it: a relation named like a SQLite keyword is quoted (`main."plan"`,
    // `sql::ident`), and is bound by its own name (O-pse-alone-strict).
    let name = &*crate::code_rule_dsl::logic::sql::unquote(name);
    let src = if schema == "main" {
        crate::code_rule_dsl::facts::vtab::bound_source(conn, name)?
    } else {
        // The columns read and those filtered.
        let filtered = filters.into_iter().flat_map(|f| f.split(" AND ")).filter_map(|f| f.split_once(" = ").map(|(c, _)| c));
        let need: Vec<&str> = cols.split(',').filter(|c| c.trim() != "1").chain(filtered).collect();
        stored_table(conn.sql().ok()?, schema, name, &need)?
    };
    let src=src.frozen().unwrap_or(src);
    // Its columns as its table names them (`vtab::named`): a relation handed on as `c0`, `c1` under a base one's name
    // (`value`) is read by the base names, as its statements write them.
    let names = if schema == "main" { crate::code_rule_dsl::facts::vtab::named(name, src.as_ref()) } else { src.columns() };
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
    /// Whether each column is in row order (a table's column read in its order): its probes are ranges found by a
    /// search of the column (`sorted_range`), its distinct values counted along it; no `Sorted` is built for it.
    in_order: Vec<std::sync::OnceLock<bool>>,
}
impl Stored {
    fn new(columns: Vec<String>, values: Vec<std::sync::OnceLock<Vec<i64>>>) -> Self {
        let k = columns.len();
        Stored { columns, n: std::sync::OnceLock::new(), values, index: (0..k).map(|_| std::sync::OnceLock::new()).collect(), distinct: (0..k).map(|_| std::sync::OnceLock::new()).collect(), orders: Mutex::default(), in_order: (0..k).map(|_| std::sync::OnceLock::new()).collect() }
    }
    fn distinct(&self, c: usize) -> usize {
        *self.distinct[c].get_or_init(|| {
            if self.in_order(c) {
                let col = self.col(c);
                (1 + (1..col.len()).filter(|&i| col[i] != col[i - 1]).count()).min(col.len()).max(1)
            } else {
                self.sorted(c).keys.len().max(1)
            }
        })
    }
    /// Whether column `c` is held and in row order (`in_order`).
    fn in_order(&self, c: usize) -> bool {
        self.values[c].get().is_some() && *self.in_order[c].get_or_init(|| self.col(c).is_sorted())
    }
    /// Column `c`'s rows whose value is `v`, the column in row order: two searches of the column.
    fn range_of(&self, c: usize, v: i64) -> std::ops::Range<u32> {
        let col = self.col(c);
        col.partition_point(|&x| x < v) as u32..col.partition_point(|&x| x <= v) as u32
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
    fn columns(&self) -> Vec<String> { self.columns.clone() }
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
        let mut out: Vec<usize> = if self.in_order(c) {
            values.iter().flat_map(|&v| self.range_of(c, v).map(|r| r as usize)).collect()
        } else {
            values.iter().flat_map(|&v| self.rows_of(c, v).iter().map(|&r| r as usize)).collect()
        };
        out.sort_unstable();
        Some(out)
    }
    /// A column held nowhere (no rule names it, `columns_source`) reads as 0.
    fn value(&self, row: usize, c: usize) -> i64 { self.col(c).get(row).copied().unwrap_or(0) }
    fn sorted_rows(&self, c: usize, v: i64) -> Option<&[u32]> { self.values[c].get().map(|_| self.rows_of(c, v)) }
    fn sorted_range(&self, c: usize, v: i64) -> Option<std::ops::Range<u32>> { self.in_order(c).then(|| self.range_of(c, v)) }
    fn sorts_rows(&self, c: usize) -> bool { self.values[c].get().is_some() }
}
/// A relation held in memory as columns (`None`: a column no rule reads, which reads as 0), read in place: a
/// probe on a column is a slice of its sorted index, built on first use.
pub fn columns_source(columns: Vec<String>, values: Vec<Option<Vec<i64>>>) -> Arc<dyn crate::code_rule_dsl::facts::vtab::Source> {
    Arc::new(held::Held::new(columns, values, None))
}
/// `columns_source`, its statistics given (kept by delta, decision 0096): read as they are, never measured.
pub fn columns_source_measured(columns: Vec<String>, values: Vec<Option<Vec<i64>>>, statistics: Vec<(String, i64, i64, i64, i64)>) -> Arc<dyn crate::code_rule_dsl::facts::vtab::Source> {
    Arc::new(held::Held::new(columns, values, Some(statistics)))
}
/// The table `schema.name` read once into memory, every column (`None`: not a table of integers).
pub fn table_source(conn: &Connection, schema: &str, name: &str) -> Option<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>> {
    let columns: Vec<String> = conn.prepare(&format!("SELECT * FROM {schema}.{}", crate::code_rule_dsl::logic::sql::ident(name))).ok()?.column_names().map(str::to_owned).collect();
    let need: Vec<&str> = columns.iter().map(String::as_str).collect();
    stored_table(conn, schema, name, &need)
}
/// The stored tables read this run, by database and `schema.table` (dropped with the inputs, `forget_inputs`).
#[derive(Default)]
struct StoredTables(Mutex<HashMap<(String, String), Option<Arc<Stored>>>>);
/// The stored table `schema.name` with the columns named in `need` loaded (`None`: not one, or not integers).
fn stored_table(conn: &Connection, schema: &str, name: &str, need: &[&str]) -> Option<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>> {
    if schema == "temp" || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') { return None; }
    let key = (conn_path(conn).to_string(), format!("{schema}.{name}"));
    let held_ = crate::code_rule_dsl::logic::profile::shared::<StoredTables>();
    let t = held_.0.lock().unwrap().get(&key).cloned();
    let t = match t {
        Some(t) => t?,
        None => {
            // The columns' names, from the statement (prepared, not run).
            let columns: Vec<String> = conn.prepare(&format!("SELECT * FROM {schema}.{}", crate::code_rule_dsl::logic::sql::ident(name))).ok()?.column_names().map(str::to_owned).collect();
            let k = columns.len();
            let t = Arc::new(Stored::new(columns, (0..k).map(|_| std::sync::OnceLock::new()).collect()));
            crate::code_rule_dsl::logic::profile::shared::<StoredTables>().0.lock().unwrap().insert(key.clone(), Some(t.clone()));
            t
        }
    };
    let need: Vec<usize> = need.iter().map(|c| t.columns.iter().position(|n| n == c.trim())).collect::<Option<_>>()?;
    if !t.load(conn, &format!("{schema}.{name}"), &need) {
        crate::code_rule_dsl::logic::profile::shared::<StoredTables>().0.lock().unwrap().insert(key, None);
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
    /// `texts`: for a node read (`syntax::NODE_READS`), the run's binding of the long-text nodes it reads through
    /// (`functions::node_read`).
    Func { name: &'static str, kinds: &'static [crate::code_rule_dsl::logic::strings::functions::Kind], args: Vec<Key>, out: usize, bind: bool,
        texts: Option<NodeTexts> },
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
pub struct JoinRule { pub steps: Vec<Step>, pub out: Vec<Term>, pub nslots: usize, multi: Vec<(usize, Vec<usize>)>, pub counts: Vec<[Count; 2]>,
                      /// Per step, the texts it makes that are only looked up or not read (`probes`): `PROBE_*` and
                      /// `UNREAD_*` of a `@dot`'s, a function's or a split's parts. None is interned.
                      pub probes: Vec<u8>,
                      pub fused: Vec<Option<Box<rowcode::Fused>>>, forms: Vec<table::StepForm>, spec: u64,
                      /// Rows passed over by a factored step (`factored`): counted in `counts` as joined, not walked.
                      pub passed_over: Count,
                      /// Profiled: per step, and last the head's rows made, the CPU nanoseconds its walks spent there,
                      /// the source's lookups included (`StepClock`); added as `counts` are, and taken with them.
                      pub step_ns: Vec<Count>,
                      /// Per step, the plan step it was made for (`JoinSpec::from`; `plan_steps` for a step of the
                      /// head's computed terms), and the plan's steps: what the engine reports by step is by plan step.
                      pub from: Vec<usize>, pub plan_steps: usize,
                      /// The rows the planner expects after each plan step, every row walked (`JoinSpec::est`); empty
                      /// where it did not price the rule.
                      pub est: Vec<f64>,
                      /// Per step, a range of the column it binds that later comparisons bound (`ranges`).
                      ranges: Vec<Option<Ranged>> }

impl JoinRule {
    /// Why a source it reads could not make the rows it read (`Source::failed`), if one could not.
    fn failed(&self) -> Option<String> {
        self.steps.iter().find_map(|s| match s {
            Step::Atom { src: Src::In(rel, _), .. } => rel.view.as_ref().and_then(|v| v.src.failed()),
            _ => None,
        })
    }
}
impl JoinRule {
    /// Its steps' (in, out) counts by plan step (`plan_counts`).
    pub fn plan_counts(&self, c: &[(u64, u64)]) -> Vec<(u64, u64)> {
        plan_counts(&self.from, self.plan_steps, c)
    }
    /// Its steps' CPU by plan step, and last the head's (`plan_cpu`).
    pub fn plan_cpu(&self, c: &[f64]) -> Vec<f64> {
        plan_cpu(&self.from, self.plan_steps, c)
    }
    /// The step made first for plan step `p`.
    pub fn step_of(&self, p: usize) -> Option<usize> {
        self.from.iter().position(|&f| f == p)
    }
}

/// Where a node read (`Step::Func`'s `texts`) finds the run's long-text nodes: the binding, when the join was made
/// after it, or the connection's data (weakly), looked up as the step runs when it was made before (a phase's plan
/// made before its run binds its nodes). Without a binding a read is NULL, as the SQL function's
/// (`functions::register`).
#[derive(Clone)]
pub enum NodeTexts {
    Held(Arc<dyn crate::code_rule_dsl::facts::vtab::Source>),
    At(std::sync::Weak<crate::code_rule_dsl::facts::vtab::ConnData>),
}

impl std::fmt::Debug for NodeTexts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NodeTexts::Held(_) => f.write_str("Held"),
            NodeTexts::At(_) => f.write_str("At"),
        }
    }
}

/// Whether a scan's walk may run in morsels (`run_join_in_each`'s `morsels`): `RERAIL_MORSELS=off` walks every join
/// on its caller's thread, as before (to compare).
/// Not under `walks_whole`.
fn morsels_on() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_MORSELS").map_or(true, |v| v != "off")) && !rerail_prof::code_rule_dsl::logic::profile::bound::<WalksWhole>().0
}

thread_local! {
    /// The walks this thread ran in morsels (`morsels_walked`).
    static IN_MORSELS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Runs `f` with each join walked whole on its thread, not in morsels: for work that already keeps the pool busy
/// beside others like it (a run's chunks evaluating at once, decision 0254), where morsels would only add jobs and
/// the memory of their rows. The jobs `f` starts walk so too (`carried`).
pub fn walks_whole<R>(f: impl FnOnce() -> R) -> R {
    let _whole = rerail_prof::code_rule_dsl::logic::profile::bind(WalksWhole(true));
    f()
}

/// Bound under `walks_whole`: this run's joins each walk on one thread, in its jobs too (the run's context).
#[derive(Clone, Copy, Default)]
struct WalksWhole(bool);

/// A job's account of the chunk under way (`rows::enter_chunk`, Q203 item 3), entered on its thread (`run_state`):
/// the chunk itself is the run's context's, carried with it; what the job's thread allocates and does not free is
/// counted there (`GIVEN`, `TOP`) and given to the chunk as the job ends.
pub(in crate::code_rule_dsl) fn carried(c: &mut crate::code_rule_dsl::logic::run_state::Capture) {
    c.part(&["GIVEN", "TOP"], crate::code_rule_dsl::logic::run_state::Shared(crate::code_rule_dsl::facts::rows::current_chunk(), crate::code_rule_dsl::facts::rows::enter_chunk));
}


/// Tests: the rows a cut left that walks going on from it read in their order (`run_counts().left_in_order`).
#[cfg(any(test, feature = "test-hooks"))]
pub fn left_read_in_order() -> u64 {
    run_counts().left_in_order.load(std::sync::atomic::Ordering::Relaxed)
}

/// Tests: the walks this thread has run in morsels.
#[cfg(any(test, feature = "test-hooks"))]
pub fn morsels_walked() -> u64 {
    IN_MORSELS.with(std::cell::Cell::get)
}

/// A count of a join's (`JoinRule::counts`, `passed_over`): a walk counts into cells of its own and adds them here
/// once, as it ends, so a scan walked in morsels on several threads (`morsels`) counts as one walk does.
#[derive(Default, Debug)]
pub struct Count(std::sync::atomic::AtomicU64);
impl Count {
    pub fn get(&self) -> u64 { self.0.load(std::sync::atomic::Ordering::Relaxed) }
    pub fn set(&self, v: u64) { self.0.store(v, std::sync::atomic::Ordering::Relaxed) }
    pub fn replace(&self, v: u64) -> u64 { self.0.swap(v, std::sync::atomic::Ordering::Relaxed) }
    fn add(&self, v: u64) { if v > 0 { self.0.fetch_add(v, std::sync::atomic::Ordering::Relaxed); } }
}

/// Steps' (in, out) counts by the plan step each was made for (`from`, of `plan_steps`): a plan step's bindings into
/// its first step and out of its last. A plan step that made no step passes its rows on; the head's computed terms'
/// steps are no plan step's.
pub fn plan_counts(from: &[usize], plan_steps: usize, c: &[(u64, u64)]) -> Vec<(u64, u64)> {
    let mut out: Vec<Option<(u64, u64)>> = vec![None; plan_steps];
    for (&p, &(a, b)) in from.iter().zip(c) {
        if let Some(o) = out.get_mut(p) {
            *o = Some(o.map_or((a, b), |(a0, _)| (a0, b)));
        }
    }
    let mut last = 0;
    out.into_iter().map(|o| { let o = o.unwrap_or((last, last)); last = o.1; o }).collect()
}

/// Steps' CPU, and last the head's rows made (`c`'s last), by plan step, and last the head's: its computed terms'
/// steps and its rows made.
pub fn plan_cpu(from: &[usize], plan_steps: usize, c: &[f64]) -> Vec<f64> {
    let mut out = vec![0.0; plan_steps + 1];
    for (i, &t) in c.iter().enumerate() {
        out[from.get(i).copied().unwrap_or(plan_steps).min(plan_steps)] += t;
    }
    out
}

/// The processor's tick counter: the virtual counter on ARM (24 MHz on Apple's), the time-stamp counter on x86.
/// Read at every step a profiled walk enters and leaves, where a thread-CPU clock read (a few hundred ns) would
/// cost more than the steps it times. Its unit cancels: a walk's ticks are scaled to its thread's CPU (`StepClock`).
#[inline(always)]
fn ticks() -> u64 {
    #[cfg(target_arch = "aarch64")]
    {
        let t: u64;
        // SAFETY: reads the virtual counter, which user code may read on every platform the engine runs on.
        unsafe { std::arch::asm!("mrs {}, cntvct_el0", out(reg) t, options(nomem, nostack, preserves_flags)) };
        t
    }
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: the time-stamp counter is readable from user code.
        unsafe { core::arch::x86_64::_rdtsc() }
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        0
    }
}

/// A profiled walk's CPU per step (Q36: a view's row can cost far more than a unit of work, and the stratum's CPU
/// alone does not say which step's source it is). Each step entered and left charges the ticks since the last
/// charge to the step the walk was in, so a step's time is its own, its source's lookups and tests included, the
/// steps after it not. At the end the ticks are scaled to the thread's CPU over the walk: a thread descheduled
/// mid-walk spreads that time over the steps in proportion, not onto the one it stopped in.
struct StepClock { ticks: Vec<u64>, at: usize, last: u64, t0: u64, cpu0: f64 }
impl StepClock {
    /// Off unless profiling. `steps + 2` places: each step, the head's rows made, and the walk outside any step.
    fn start(steps: usize) -> Option<Box<StepClock>> {
        crate::code_rule_dsl::logic::profile::on().then(|| {
            let t = ticks();
            Box::new(StepClock { ticks: vec![0; steps + 2], at: steps + 1, last: t, t0: t, cpu0: crate::code_rule_dsl::logic::profile::now() })
        })
    }
    #[inline(always)]
    fn to(&mut self, at: usize) -> usize {
        let t = ticks();
        self.ticks[self.at] += t.wrapping_sub(self.last);
        self.last = t;
        std::mem::replace(&mut self.at, at)
    }
    /// The walk's CPU, by step, added to the join's (`JoinRule::step_ns`).
    fn end(mut self, j: &JoinRule) {
        let outside = self.ticks.len() - 1;
        self.to(outside);
        let (span, cpu) = (self.last.wrapping_sub(self.t0), crate::code_rule_dsl::logic::profile::now() - self.cpu0);
        if span == 0 || cpu <= 0.0 { return; }
        let ns = cpu * 1e9 / span as f64;
        for (c, &t) in j.step_ns.iter().zip(&self.ticks) { c.add((t as f64 * ns) as u64); }
    }
}

fn parse_join(conn: &Connection, spec: &str, syms: &mut Interner) -> Result<JoinRule> {
    join_rule(conn, &JoinSpec::parse(spec).map_err(err)?, syms)
}

impl Step {
    /// Whether the step reads slot `x`: what it observes (`observes`, a step that tests its result against a bound
    /// slot reading that slot), or a `Let`'s source. One definition for every reader of the question (the probes,
    /// the factoring, the device). A second copy here missed a `@cat`'s and a function's tested result: a split's
    /// part only a later `$r = @substr(…)` tested was taken as read by nothing and held 0, so the test failed on
    /// every row (`hexpr`'s dotted annotations, once a plan split `res`'s names).
    pub fn mentions(&self, x: usize) -> bool {
        self.observes(x) || matches!(self, Step::Let { b: Key::Slot(s), .. } if *s == x)
    }
}

pub(crate) const PROBE_OUT: u8 = 1;
pub(crate) const PROBE_A: u8 = 2;
pub(crate) const PROBE_B: u8 = 4;
/// Read by nothing (`UNREAD_OUT`, `_A`, `_B`): the text must exist (a function's NULL ends the branch) but needs no
/// atom, nor a lookup: the slot holds 0. `match("(?<m>…): *(?<rest>.*)", l)` binds `rest` for no reader.
pub(crate) const UNREAD_OUT: u8 = 8;
pub(crate) const UNREAD_A: u8 = 16;
pub(crate) const UNREAD_B: u8 = 32;
/// Read only as text (decision 0228): every later reader is a `cat`'s part or a text function's text argument, so
/// the text is held in the join's scratch for its branch (`Ctx::texts`, a value `SCRATCH + i`), as the device holds
/// one in its arena, and needs no atom. The findings' JSON is one text of thirty pieces: each piece was an atom.
pub(crate) const TEXT_OUT: u8 = 64;

/// The first value of a text held in a join's scratch (`TEXT_OUT`): below every atom and every integer a rule reads.
const SCRATCH: i64 = i64::MIN / 2;

/// Which texts the steps make only to look them up: a `@dot`, a text function's result or a split's part whose slot no
/// head reads, and every later step reads only as a key of a positive atom. Such a text needs no atom of its own: one
/// with no atom (`Interner::held`, found as interning finds one, none made) is in no row, and the join's branch ends
/// there. Interned instead, every split of every qualified name and every `@dot` probe was a new atom (12.7 M
/// over PostHog, 789 MB in the store). A step that tests its result against a bound slot (`bind: false`) probes
/// whatever reads the slot: a text that is no atom is no bound slot's value.
fn probes(steps: &[Step], out: &[Term]) -> Vec<u8> {
    let in_head = |x: usize| out.iter().any(|t| match t { Term::Slot(s) => *s == x, Term::Cat(a, b) => *a == x || *b == x, Term::Const(_) => false });
    // `unread`, `probe` or 0 by what reads slot `x` after step `from`. A positive atom reads a slot only as a key
    // (`mentions`); any other reader (a test of its name, a negative atom, which must see a miss as "no row") needs it.
    let use_of = |from: usize, x: usize, unread: u8, probe: u8| {
        if in_head(x) {
            return 0;
        }
        let mut readers = steps[from + 1..].iter().filter(|step| step.mentions(x)).peekable();
        if readers.peek().is_none() {
            return unread;
        }
        if readers.all(|step| matches!(step, Step::Atom { neg: false, .. })) { probe } else { 0 }
    };
    // Whether every reader of slot `x` after step `from` reads it as text alone (`TEXT_OUT`).
    let as_text = |from: usize, x: usize| {
        let me = |k: &Key| matches!(k, Key::Slot(s) if *s == x);
        let mut readers = steps[from + 1..].iter().filter(|step| step.mentions(x)).peekable();
        !in_head(x) && readers.peek().is_some() && readers.all(|step| match step {
            Step::Cat { out, a, b, .. } => *out != x && (me(a) || me(b)),
            Step::Func { kinds, args, out, .. } => {
                *out != x && args.iter().zip(kinds.iter()).all(|(a, k)| !me(a) || matches!(k, crate::code_rule_dsl::logic::strings::functions::Kind::Text))
            }
            _ => false,
        })
    };
    steps
        .iter()
        .enumerate()
        .map(|(i, step)| match step {
            Step::Cat { bind: false, .. } | Step::Func { bind: false, .. } => PROBE_OUT,
            Step::Cat { out, bind: true, .. } | Step::Func { out, bind: true, .. } if as_text(i, *out) => TEXT_OUT,
            Step::Cat { out, bind: true, .. } | Step::Func { out, bind: true, .. } => use_of(i, *out, UNREAD_OUT, PROBE_OUT),
            Step::Split { a, b, .. } => a.map_or(0, |x| use_of(i, x, UNREAD_A, PROBE_A)) | b.map_or(0, |x| use_of(i, x, UNREAD_B, PROBE_B)),
            _ => 0,
        })
        .collect()
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
    join_rule_bound(conn, spec, syms, index, &[])
}

/// `join_rule_with`, its slots `pre` bound before its first step (a recheck's head, decision 0364 stage 3): a step
/// that names one probes by it, and the caller presets them (`run_join_collect`).
fn join_rule_bound(conn: impl Scoped, spec: &JoinSpec, syms: &mut Interner, index: bool, pre: &[usize]) -> Result<JoinRule> {
    let conn = conn.db();
    let unindexed = |cols: &[usize]| Arc::new(Csr { cols: cols.to_vec(), table: HashTable::new(), ids: Vec::new(), source: false, sorted: false, chain: None, lead: 0, composite: false, keyed: false, ordered: None, order: OnceLock::new(), block: None, then: None });
    let mut bound: Vec<bool> = Vec::new();
    for &s in pre {
        if bound.len() <= s { bound.resize(s + 1, false) }
        bound[s] = true;
    }
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
                        let rel = on(&conn).derived.lock().get(table.as_str()).cloned().ok_or_else(|| err(format!("no native relation {table}")))?;
                        let cols: Vec<usize> = keys.iter().map(|&(c, _)| c).collect();
                        let ix = if index { rel.index(&cols) } else { unindexed(&cols) };
                        Src::In(rel, ix)
                    }
                    // A held relation's version (decision 0364): a maintained stratum's removal and add joins read
                    // its rows at the run's start, those gone and those new, in place.
                    Source::Version(table, ver) => {
                        // A changed input's versions are given beside it (`give_versions`); a held relation's are its marks.
                        let on = on(&conn);
                        let given = on.given.lock().get(table.as_str()).cloned();
                        let rel = if let Some([old, minus, plus, now]) = given {
                            Some(match ver { spec::Ver::Old => old, spec::Ver::Minus => minus, spec::Ver::Plus => plus, spec::Ver::New => now })
                        } else {
                        let held = on.derived.lock().get(table.as_str()).cloned().ok_or_else(|| err(format!("no native relation {table}")))?;
                        match ver {
                            spec::Ver::New => Some(held),
                            spec::Ver::Old => held.version_view(Ver::Old),
                            spec::Ver::Minus => held.version_view(Ver::Minus),
                            spec::Ver::Plus => held.version_view(Ver::Plus),
                        }
                        };
                        let rel = rel.ok_or_else(|| err(format!("{table} has no version (it did not start from a kept relation)")))?;
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
                // A node's text or value is read through the run's binding of the long-text nodes, which holds its
                // batch (`syntax::LongTexts`).
                let texts = if crate::code_rule_dsl::facts::vtab::syntax::NODE_READS.contains(&name) {
                    let long = crate::code_rule_dsl::facts::vtab::syntax::LONG_TEXT_NODES;
                    Some(match crate::code_rule_dsl::facts::vtab::bound_source(&conn, long) {
                        Some(nodes) => NodeTexts::Held(nodes),
                        None => NodeTexts::At(conn.db().data_weak()),
                    })
                } else {
                    None
                };
                steps.push(Step::Func { name, kinds, args, out: o, bind, texts });
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
    let probes = probes(&steps, &out);
    let step_ns = (0..=steps.len()).map(|_| Default::default()).collect();
    // Each step's plan step, where the planner said (a spec read from text did not: each step is its own).
    let (from, plan_steps) = if spec.from.len() == steps.len() {
        (spec.from.iter().map(|&p| p as usize).collect(), spec.plan_steps as usize)
    } else {
        ((0..steps.len()).collect(), steps.len())
    };
    let ranges = ranges(&steps);
    // A ranged probe of a derived input reads the order on its key then the range's column (`index_then`).
    for (st, r) in steps.iter_mut().zip(&ranges) {
        if let (Step::Atom { src: Src::In(rel, ix), .. }, Some(r)) = (st, r)
            && ix.ordered.is_some()
            && ix.then.is_none()
        {
            *ix = rel.index_then(&ix.cols, r.col);
        }
    }
    Ok(JoinRule { steps, out, nslots: bound.len(), multi, counts, fused, forms, probes, spec: level::digest(spec), passed_over: Default::default(), step_ns, from, plan_steps, est: spec.est.0.clone(), ranges })
}

/// Runs a join probe over the store; its new head tuples go to `fresh`. Returns the tuples it derived.
fn run_join(j: &JoinRule, head: usize, store: &Store, syms: &mut Interner, fresh: &mut [Fresh]) -> Result<u64> {
    run_join_limited(j, head, store, syms, fresh, None)
}

/// `run_join` under a limit (decision 0206), what it did handed back in `limited`.
fn run_join_limited(j: &JoinRule, head: usize, store: &Store, syms: &mut Interner, fresh: &mut [Fresh], limited: Option<&mut Limited>) -> Result<u64> {
    run_join_in_each(j, head, store, syms, fresh, &[], None, None, None, limited)
}

/// Runs a seed join: from its pairs when a level's batch made them (`level`), else by the host's walk.
fn run_seed(j: &JoinRule, head: usize, store: &Store, syms: &mut Interner, fresh: &mut [Fresh], limited: Option<&mut Limited>) -> Result<u64> {
    match level::take(j)? {
        Some(p) => {
            level::run_counts().used.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            // From a level's pairs: unlimited.
            run_join_in(j, head, store, syms, fresh, &[], None, Some(&p))
        }
        None => run_join_limited(j, head, store, syms, fresh, limited),
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
    run_join_in_each(j, 0, store, syms, &mut [], &[], Some(&mut out), None, Some((y, next)), None)
}

#[allow(clippy::too_many_arguments)]
fn run_join_in(j: &JoinRule, head: usize, store: &Store, syms: &mut Interner, fresh: &mut [Fresh], preset: &[(usize, i64)], collect: Option<&mut Vec<i64>>, paired: Option<&level::Paired>) -> Result<u64> {
    run_join_in_each(j, head, store, syms, fresh, preset, collect, paired, None, None)
}

/// Whether a positive atom of `j` reads an input relation known to be empty, so the whole conjunction is: the CPU's
/// walk then returns at once (`run_join_in_each`), and the device encodes nothing for it (`device::run`, `submit`).
/// A lazy view's length is not known without enumerating it, so a view is not known empty.
pub fn reads_empty_input(j: &JoinRule) -> bool {
    j.steps.iter().any(|s| matches!(s, Step::Atom { src: Src::In(r, _), neg: false, .. } if r.n == 0 && r.view.as_ref().is_none_or(|v| v.ids.is_some())))
}

#[allow(clippy::too_many_arguments)]
fn run_join_in_each(j: &JoinRule, head: usize, store: &Store, syms: &mut Interner, fresh: &mut [Fresh], preset: &[(usize, i64)], collect: Option<&mut Vec<i64>>,
                    paired: Option<&level::Paired>, each: Option<(usize, &mut dyn FnMut(&mut Vec<i64>) -> Option<i64>)>, mut limited: Option<&mut Limited>) -> Result<u64> {
    // The limit `run` handed this walk (decision 0206). A walk for each value, or from a level's pairs, runs
    // unlimited.
    let limit = limited.as_mut().and_then(|l| l.limit.take()).filter(|_| each.is_none() && paired.is_none());
    // An empty required relation makes the entire conjunction empty. Check once per
    // invocation, before expanding another input; the next wave can grow this relation.
    // Negative atoms and scalar aggregates do not obey this rule.
    // An empty input is the device's test too (`reads_empty_input`); the component's own relations are the store's.
    if reads_empty_input(j) || j.steps.iter().any(|step| match step {
        Step::Atom { src, neg: false, .. } => match src {
            Src::Delta(r) => store.rels[*r].lo == store.rels[*r].hi,
            Src::Full(r) | Src::FullMulti(r, _) => store.rels[*r].n == 0,
            Src::In(..) => false,
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
                     /// The run's cap on a derived relation's rows (decision 0462, `width::Limits`).
                     cap: usize,
                     /// The texts the branch being walked holds as text alone (`TEXT_OUT`), by `value - SCRATCH`: each
                     /// dropped as its step's branch returns.
                     texts: Vec<String>,
                     /// Each step's place in the runs of the order it probes (`Index::find_at`): the step's keys come
                     /// in a sorted batch where they can (`scan_order`), so each probe starts where the last ended.
                     hints: Vec<[u32; runs::HINTS]>,
                     /// Each step's last probe of a source's lookup and the candidates it gave: the step's keys come
                     /// sorted where they can, so a key the step probed just before is answered by its candidates
                     /// again, not by the source (a held `kind` probed by kind reads every document of the language).
                     last: Vec<LastProbe>,
                     collect: Option<&'c mut Vec<i64>>,
                     /// The row the first step is at (a count table's counts per row are by it).
                     at0: u32,
                     /// Whether the first step has reached a row (`at0` is one): a cut before it left every row.
                     seen0: bool,
                     /// Per atom step, the slots it binds that the steps before `upto` read, and `upto` (`factored`):
                     /// a row whose rest of the body fails before reaching `upto` fails by those values alone.
                     factor: Vec<Option<Factor>>,
                     /// The deepest step entered (`go`), so a factored step knows where its row's descent stopped.
                     deep: usize,
                     /// Per such step, in its current probe, the values of those slots the rest of the body failed for,
                     /// each with what that failure added to the steps' counts from this step on (`counts[i..]`, in
                     /// and out per step): a row passed over adds them again, so the counts are the join's rows, as
                     /// the walk with no factoring (and the device) counts them; only the work is saved.
                     failed: Vec<rustc_hash::FxHashMap<[i64; FACTOR_MAX], Box<[u64]>>>,
                     /// The counts from a factored step on, before its row's descent (a stack: descents nest).
                     snap: Vec<u64>,
                     /// Rows passed over by `failed`: read and tested, not joined on.
                     skipped: u64,
                     /// Decision 0206: the steps entered, the most the walk may enter (`u64::MAX`, no limit), and
                     /// whether it passed that and stops.
                     work: u64, budget: u64, stop: bool,
                     /// The step whose rows are restricted, the rows it may still read (others are passed over), and
                     /// the first step's candidates in the order read, under a budget.
                     only_step: usize, only: Option<Rows0>, list0: Option<(RowsOf, Vec<u32>)>,
                     /// The first step from which the body binds nothing the head reads (`tail`), or `usize::MAX` for
                     /// a walk that collects: from it on, a row made ends its step's loop (one witness is enough).
                     cut_from: usize,
                     /// Per step, the bindings that entered it and those it passed on in this walk: added to the
                     /// join's (`JoinRule::counts`) as the walk ends.
                     counts: Vec<[std::cell::Cell<u64>; 2]>,
                     /// A morsel's rows of the first step (`morsels`): read in place of its candidates.
                     first: Option<(Rows<'a>, Cands<'a>)>,
                     /// At a cut of a walk in morsels, the first step's rows its morsels left (each one's from the row
                     /// it stopped in), in place of those from one row of `list0` on.
                     left0: Option<Vec<u32>>,
                     /// Profiled: the CPU per step (`StepClock`).
                     clock: Option<Box<StepClock>> }
    fn val(slots: &[i64], k: Key) -> i64 { match k { Key::Slot(s) => slots[s], Key::Const(c) => c } }
    /// Where an atom's candidate tuples are: an input relation or a component relation, by id.
    #[derive(Clone, Copy)]
    enum Rows<'a> { In(&'a InRel), Comp(&'a Rel), Source(&'a View) }
    impl<'a> Rows<'a> {
        fn row(self, id: u32) -> RowRef<'a> {
            match self { Rows::In(r) => r.at(id as usize).read(), Rows::Comp(r) => RowRef::Rows(&r.data, id, r.k), Rows::Source(v) => RowRef::Source(v, id as usize).read() }
        }
        /// `row`, a view's columns in `need` alone (`RowRef::read_need`).
        fn row_need(self, id: u32, need: u64) -> RowRef<'a> {
            match self { Rows::In(r) => r.at(id as usize).read_need(need), Rows::Comp(r) => RowRef::Rows(&r.data, id, r.k), Rows::Source(v) => RowRef::Source(v, id as usize).read_need(need) }
        }
        /// Calls `f` on each candidate and its row (`row_need`) until it returns true; returns whether one did. A
        /// scan of a whole view (a range of its rows) reads them in one walk (`Source::for_rows_in`): a view of the
        /// tape then finds each row's node, id and parent as it walks, where a row at a time is a select, a granule
        /// search and a parent search (decision 0353 step 2).
        fn each(self, cands: Cands<'_>, need: u64, mut f: impl FnMut(u32, RowRef<'a>) -> bool) -> bool {
            if let (Rows::In(r), &Cands::Range(lo, hi)) = (self, &cands)
                && hi - lo >= WALK_SCAN
                && let Some(v) = &r.view
                && v.ids.is_none()
                && v.cols.len() <= READ
            {
                let (mut cols, mut at, mut m) = ([0usize; READ], [0usize; READ], 0);
                for c in 0..v.cols.len() {
                    if need >> c & 1 != 0 {
                        cols[m] = v.cols[c];
                        at[m] = c;
                        m += 1;
                    }
                }
                let mut hit = false;
                v.src.for_rows_in(lo as usize..hi as usize, &cols[..m], &mut |row, got| {
                    let mut vals = [0i64; READ];
                    for j in 0..m {
                        vals[at[j]] = got[j];
                    }
                    hit = f(row as u32, RowRef::Read(vals, row));
                    !hit
                });
                return hit;
            }
            if let Cands::Read(rows) = cands {
                return rows.iter().any(|&(id, vals)| f(id, RowRef::Read(vals, id as usize)));
            }
            cands.any(|id| f(id, self.row_need(id, need)))
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
    /// `left`: the rows to read are these (a cut's rows left, in the order its walk read them, `Rows0::order`) of
    /// the range: in the key's order when they are already (`None`), else sorted. A walk going on from a cut sorted
    /// all of the range again, then filtered it (decision 0294: a seed cut at each check, 7 sorts of 2.2 M rows).
    /// `pre`: the range's rows already read, each with its values by column (`walked_in_order`), read from there.
    fn scan_order(j: &JoinRule, store: &Store, i: usize, rows: Rows<'_>, cands: &Cands<'_>, binds: &[(usize, usize)], left: Option<&[u32]>, pre: Option<&[(u32, [i64; READ])]>) -> Option<Vec<u32>> {
        const SCAN_SORTED: u32 = 256;
        let &Cands::Range(lo, hi) = cands else { return None };
        let n = left.map_or((hi - lo) as usize, <[u32]>::len);
        if n < SCAN_SORTED as usize || j.fused.get(i).is_some_and(Option::is_some) {
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
            Src::In(rel, ix) => { let index = ix.runs(rel)?; let m = index.cols.iter().take_while(|c| ix.cols.contains(c)).count(); (index, m, rel.k) }
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
        let tagged = index.is_tagged(k);
        // The scan's rows known sorted by the probes' key (decision 0187): its columns in turn (constants hold no
        // order), tagged by the first when the next order is.
        let mut by: Vec<usize> = Vec::new();
        for f in &from {
            if let From::Col(c) = *f && !by.contains(&c) {
                by.push(c);
            }
        }
        let tag = if tagged && let From::Col(c) = from[0] { Some(c) } else { None };
        let known: Vec<(usize, usize)> = if left.is_some() || !runs::known_beyond_runs() { Vec::new() } else { match rows {
            Rows::In(r) => r.sorted.within_key(r.k, tag, &by, lo, hi),
            Rows::Comp(r) => r.sorted.within_key(r.k, tag, &by, lo, hi),
            Rows::Source(_) => Vec::new(),
        }.into_iter().map(|(a, b)| ((a - lo) as usize, (b - lo) as usize)).collect() };
        if known == [(0, n)] {
            // In the key's order already: the scan's rows as they are.
            runs::presorted::site_known(runs::presorted::SCAN, n as u64);
            return None;
        }
        // The row at each place, and each place's key values; many rows are read in parts at once (decision 0294), as
        // the morsels read them.
        let id_at = |p: usize| left.map_or(lo + p as u32, |l| l[p]);
        let mut vals = vec![0i64; n * m];
        let read = |part: &mut [i64], first: usize| for (r, out) in part.chunks_exact_mut(m).enumerate() {
            if let Some(pre) = pre {
                let v = &pre[first + r].1;
                for (o, f) in out.iter_mut().zip(&from) { *o = match *f { From::Col(c) => v[c], From::Const(x) => x }; }
                continue;
            }
            let row = rows.row(id_at(first + r));
            for (o, f) in out.iter_mut().zip(&from) { *o = match *f { From::Col(c) => row.at(c), From::Const(v) => v }; }
        };
        if n >= runs::PAR_ROWS && crate::threads::max() > 1 {
            let size = n.div_ceil(crate::threads::max() * 4);
            crate::threads::scope(|jobs| for (p, part) in vals.chunks_mut(size * m).enumerate() {
                jobs.spawn("a scan's keys read", move |_| read(part, p * size));
            });
        } else {
            read(&mut vals, 0);
        }
        let cmp = |a: u32, b: u32| vals[a as usize * m..][..m].cmp(&vals[b as usize * m..][..m]);
        let key_cmp = |a: u32, b: u32| (if tagged { runs::tag(vals[a as usize * m]).cmp(&runs::tag(vals[b as usize * m])) } else { std::cmp::Ordering::Equal }).then_with(|| cmp(a, b));
        // Rows a cut left come in the order their walk read them: in this key's order where the walk's order was
        // the same (one pass tells), and then they are read as they are.
        let known: Vec<(usize, usize)> = match left {
            Some(_) if (1..n).all(|p| key_cmp(p as u32 - 1, p as u32).is_le()) => {
                runs::presorted::site_known(runs::presorted::SCAN, n as u64);
                run_counts().left_in_order.fetch_add(n as u64, std::sync::atomic::Ordering::Relaxed);
                return None;
            }
            _ => known,
        };
        let mut order: Vec<u32> = (0..n as u32).collect();
        runs::presorted::site(runs::presorted::SCAN, n, |i| key_cmp(i as u32 - 1, i as u32).is_gt());
        // The stretches not known sorted are sorted, then every piece merged (several in-order ranges are merged,
        // not sorted again).
        let taken = runs::sort_pieces(&mut order, &known, |v| { runs::sort_keyed(v, tagged, m, |q, i| vals[q as usize * m + i], cmp, &mut runs::NoTags); }, |a, b| key_cmp(a, b).is_le());
        runs::presorted::site_known(runs::presorted::SCAN, taken as u64);
        Some(order.into_iter().map(|q| id_at(q as usize)).collect())
    }
    /// A scan of a whole view put in the next step's key order (`scan_order`, decision 0112), its rows read in one walk
    /// (`Source::for_rows_in`, decision 0353 step 2) with the values the step reads (`need`), then ordered by them: not
    /// each row read alone twice, its keys to sort by and then its values (Q220: `kind`, a view of `node`, scanned at
    /// 61 ns a row against 13 when it was a relation of its own). `None` where the walk does not serve: a range a
    /// `Rows::each` walk reads as it is, or one past `WALKED_ROWS`.
    fn walked_in_order<'b>(j: &JoinRule, store: &Store, i: usize, rows: Rows<'_>, cands: &Cands<'_>, binds: &[(usize, usize)], need: u64) -> Option<Cands<'b>> {
        let (Rows::In(r), &Cands::Range(lo, hi)) = (rows, cands) else { return None };
        let v = r.view.as_ref().filter(|v| v.ids.is_none() && v.cols.len() <= READ)?;
        let n = (hi - lo) as usize;
        if n < 256 || n > WALKED_ROWS || j.fused.get(i).is_some_and(Option::is_some) {
            return None;
        }
        if !matches!(j.steps.get(i + 1), Some(Step::Atom { keys, .. }) if !keys.is_empty()) {
            return None;
        }
        let (mut cols, mut at, mut m) = ([0usize; READ], [0usize; READ], 0);
        for c in 0..v.cols.len() {
            if need >> c & 1 != 0 {
                cols[m] = v.cols[c];
                at[m] = c;
                m += 1;
            }
        }
        let mut read: Vec<(u32, [i64; READ])> = Vec::with_capacity(n);
        v.src.for_rows_in(lo as usize..hi as usize, &cols[..m], &mut |row, got| {
            let mut vals = [0i64; READ];
            for k in 0..m {
                vals[at[k]] = got[k];
            }
            read.push((row as u32, vals));
            true
        });
        debug_assert!(read.len() == n && read.iter().enumerate().all(|(p, &(id, _))| id == lo + p as u32), "the walk reads the range's rows in order");
        let read = match scan_order(j, store, i, rows, cands, binds, None, Some(&read)) {
            Some(order) => order.into_iter().map(|id| read[(id - lo) as usize]).collect(),
            None => read,
        };
        Some(Cands::Read(Arc::new(read)))
    }
    /// The text a scratch value names (`TEXT_OUT`), if `v` is one.
    fn scratch<'s>(cx: &'s Ctx<'_, '_>, v: i64) -> Option<&'s str> {
        let at = v.checked_sub(SCRATCH).filter(|&at| at >= 0 && v < 0)?;
        cx.texts.get(at as usize).map(String::as_str)
    }
    /// Value `v`'s text: a scratch text's, or an atom's.
    fn text(cx: &Ctx<'_, '_>, v: i64) -> String {
        scratch(cx, v).map_or_else(|| cx.syms.name(v), str::to_owned)
    }
    /// The value a step's made text `t` gets, by how its slot is read (`probes`): held as text, none, looked up
    /// (-1: no atom, so no row holds it), or interned.
    fn text_value(cx: &mut Ctx<'_, '_>, probe: u8, t: String) -> i64 {
        if probe & TEXT_OUT != 0 {
            cx.texts.push(t);
            SCRATCH + cx.texts.len() as i64 - 1
        } else if probe & UNREAD_OUT != 0 {
            0
        } else if probe & PROBE_OUT != 0 {
            cx.syms.held(&t).unwrap_or(-1)
        } else {
            cx.syms.id(&t)
        }
    }
    /// Enters step `i` (past the last, the head's row): profiled, its time is charged to it until it returns.
    #[inline(always)]
    fn go<'a>(cx: &mut Ctx<'a, '_>, i: usize, slots: &mut [i64]) {
        let Some(clock) = cx.clock.as_mut() else { return enter(cx, i, slots) };
        let was = clock.to(i);
        enter(cx, i, slots);
        if let Some(clock) = cx.clock.as_mut() { clock.to(was); }
    }
    fn enter<'a>(cx: &mut Ctx<'a, '_>, i: usize, slots: &mut [i64]) {
        if cx.error.is_some() || cx.stop { return; }
        // The walk's work is the steps it enters, as the planner counts a line's (`profile::plan_work`).
        cx.work += 1;
        if cx.work > cx.budget { cx.stop = true; return; }
        let (j, store): (&'a JoinRule, &'a Store) = (cx.j, cx.store);
        if i > 0 { let c = &cx.counts[i - 1][1]; c.set(c.get() + 1); }
        if let Some(c) = cx.counts.get(i) { c[0].set(c[0].get() + 1); }
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
            // Decision 0462: no relation past the width cap. Its rows and those this walk made (an upper bound: some
            // may repeat) stop at it; the run stops or cuts the probe (`width`), never fills memory.
            let rel = &store.rels[cx.head];
            if cx.fresh[cx.head].n + rel.n >= cx.cap { cx.error = Some(super::width::too_wide(&rel.table, cx.cap)); return; }
            cx.fresh[cx.head].emitted += 1;
            cx.fresh[cx.head].offer(&cx.row, rel);
            return;
        };
        /// The candidates: an input's index on exactly the key columns, a component relation's composite index, or
        /// its shortest posting chain among its key columns (every key is then checked), or every tuple when none is
        /// bound.
        fn candidates<'a>(src: &'a Src, store: &'a Store, key: &[i64], keys: &[(usize, Key)], hints: &mut [u32; runs::HINTS], last: &mut LastProbe, need: u64, within: Within) -> (Rows<'a>, Cands<'a>) {
            match src {
                Src::In(rel, ix) => {
                    if !keys.is_empty() { probed::note(rel.name.get().map_or("?input", String::as_str), ix.cols.iter().copied()); }
                    if keys.is_empty() { (Rows::In(rel), Cands::Range(0, rel.len() as u32)) }
                    else if let Some(index) = ix.runs(rel) {
                        // An order answering the key's columns, or leading with some of them (`chain_lead`: every key
                        // is checked): the key's values in the order's column order.
                        let m = index.cols.iter().take_while(|c| ix.cols.contains(c)).count();
                        let mut buf = [0i64; MAX_KEY];
                        let at = |c: usize| key[ix.cols.iter().position(|&x| x == c).expect("a key column")];
                        let found = index.find_at(&rel.rows, rel.k, permuted(&index.cols[..m], at, &mut buf), None, hints);
                        let found = match within {
                            Some((c, lo, hi)) if m == keys.len() && index.cols.get(m) == Some(&c) => {
                                if probed::on() { probed::note(&format!("{} range", rel.name.get().map_or("?input", String::as_str)), std::iter::once(c)); }
                                found.within(&rel.rows, rel.k, c, lo, hi)
                            }
                            _ => found,
                        };
                        (Rows::In(rel), Cands::Runs(found))
                    }
                    else if ix.keyed { let r = rel.keyed_rows(key[ix.lead]); (Rows::In(rel), Cands::Range(r.start, r.end)) }
                    else if ix.composite && let Some(v) = &rel.view {
                        let mut eq = [(0usize, 0i64); MAX_KEY];
                        for (j, &c) in ix.cols.iter().enumerate() { eq[j] = (v.cols[c], key[j]); }
                        let rows = v.src.lookup_all(&eq[..ix.cols.len()]).unwrap_or_default();
                        (Rows::Source(v), Cands::Owned(rows.into_iter().map(|r| r as u32).collect()))
                    }
                    else if ix.sorted && let Some(v) = &rel.view {
                        // From where the last probe of this step was (`hints`, unused by a source otherwise): probes
                        // in the column's order are a merge over it (decision 0456 K7).
                        let (c, k) = (v.cols[ix.cols[ix.lead]], key[ix.lead]);
                        (Rows::Source(v), match v.src.sorted_range_at(c, k, &mut hints[0]) { Some(r) => Cands::Range(r.start, r.end), None => Cands::Slice(v.src.sorted_rows(c, k).unwrap_or(&[])) })
                    }
                    else if ix.source && let Some(v) = &rel.view {
                        let probed = match last {
                            Some((k, p)) if k.as_slice() == key && !matches!(&*p, Probed::Read(_, n) if *n != need) => p.clone(),
                            _ => {
                                // A step that reads columns has them read as the probe finds its rows; one that reads
                                // none (`need` 0, a test) takes the ids.
                                let read = if need != 0 { v.find_read(&ix.cols, key, ix.lead, need) } else { None };
                                let p = match read {
                                    Some(rows) => Probed::Read(Arc::new(rows), need),
                                    None => Probed::Ids(v.find_keyed(&ix.cols, key, ix.lead)),
                                };
                                match last {
                                    Some((k, q)) => {
                                        k.clear();
                                        // copy: not block data (a probe key's values, kept to compare the next probe with)
                                        k.extend_from_slice(key);
                                        *q = p.clone();
                                    }
                                    None => *last = Some((key.to_vec(), p.clone())),
                                }
                                p
                            }
                        };
                        match probed {
                            Probed::Ids(ids) => (Rows::Source(v), Cands::Owned(ids)),
                            Probed::Read(rows, _) => (Rows::Source(v), Cands::Read(rows)),
                        }
                    }
                    else { (Rows::In(rel), Cands::Slice(ix.get(rel, key))) }
                }
                Src::FullMulti(r, cols) => {
                    let rel = &store.rels[*r];
                    probed::note(&rel.table, cols.iter().copied());
                    (Rows::Comp(rel), rel.probe_at(cols, key, false, hints, within).0)
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
                        (Rows::Comp(rel), rel.probe_at(&cols[..keys.len()], key, delta, hints, within).0)
                    }
                }
        }
        }
        match step {
            Step::Atom { src, neg, keys, binds, tests } => {
                cx.key.clear();
                cx.key.extend(keys.iter().map(|&(_, k)| val(slots, k)));
                let given = if i == 0 { cx.first.take() } else { None };
                let handed = given.is_some();
                // The key columns a view's probe matched exactly, which its rows need not be read or checked for: the
                // sorted index's column, a lookup on one key column, every column of a composite lookup. A view's
                // column may be a search on the tape (decision 0353 step 2: a node's parent on format 15).
                let exact: u64 = match src {
                    _ if handed || keys.is_empty() => 0,
                    Src::In(rel, ix) if rel.view.is_some() && ix.runs(rel).is_none() => {
                        if ix.composite { ix.cols.iter().fold(0, |m, &c| m | 1 << c) }
                        else if ix.sorted || (ix.source && ix.cols.len() == 1) { 1 << ix.cols[ix.lead] }
                        else { 0 }
                    }
                    _ => 0,
                };
                let need: u64 = keys.iter().filter(|&&(c, _)| exact >> c & 1 == 0).fold(0u64, |m, &(c, _)| m | 1 << c)
                    | tests.iter().fold(0u64, |m, &(c, c0)| m | 1 << c | 1 << c0)
                    | binds.iter().fold(0u64, |m, &(c, _)| m | 1 << c)
                    | j.fused[i].as_deref().map_or(0, |f| f.gather.iter().fold(0u64, |m, &(_, c)| m | 1 << c));
                let within: Within = j.ranges[i].map(|r| (r.col, r.lo.map(|(k, eq)| (val(slots, k), eq)), r.hi.map(|(k, eq)| (val(slots, k), eq))));
                let (rows, mut cands) = given.unwrap_or_else(|| candidates(src, store, &cx.key, keys, &mut cx.hints[i], &mut cx.last[i], need, within));
                // A scan's rows in the next step's key order: its probes are then one sorted batch (decision 0112).
                // A morsel's rows are in that order already (`morsels`).
                // Decision 0206: the rows still to read, at the step restricted to them (rows of another place, an
                // order that reads the atom elsewhere, are not restricted: read again, they make the same rows); and a
                // first step's candidates as read, so that a cut knows the rows it left. A scan going on from a cut
                // reads the rows it left, in their order where that is the key's (`scan_order`).
                let reads_delta = i == cx.only_step;
                let only = if reads_delta { cx.only.take().filter(|o| o.of == RowsOf::of(src)) } else { None };
                if keys.is_empty() && !handed {
                    let left = only.as_ref().and_then(|o| o.order.as_deref()).zip(match cands { Cands::Range(lo, hi) => Some((lo, hi)), _ => None });
                    if let Some((ids, (lo, hi))) = left {
                        let ids: Vec<u32> = ids.iter().copied().filter(|id| (lo..hi).contains(id)).collect();
                        cands = Cands::Owned(scan_order(j, store, i, rows, &cands, binds, Some(&ids), None).unwrap_or(ids));
                    } else if let Some(read) = walked_in_order(j, store, i, rows, &cands, binds, need) {
                        cands = read;
                    } else if let Some(order) = scan_order(j, store, i, rows, &cands, binds, None, None) {
                        cands = Cands::Owned(order);
                    }
                }
                if i == 0 && cx.budget != u64::MAX {
                    let mut list = Vec::new();
                    cands.any(|id| { if only.as_ref().is_none_or(|o| o.has(id)) { list.push(id) } false });
                    cx.list0 = Some((RowsOf::of(src), list.clone()));
                    cands = Cands::Owned(list);
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
                    // Past the head (`tail`), the walk needs one witness: the first row that makes the head's row ends the
                    // step, and its rows are gathered 8 at first, then 8 times as many each chunk, so a binding whose
                    // first rows hold a witness reads those alone. A zero count's helper over the lines between a
                    // decorator and each later def (`rr_agg15`) walked 25.4 M lines for 25,426 pairs, each pair's every
                    // line to the head. As the row-by-row path ends it: without the cut a fused step read every candidate
                    // past the head's last binding, and a planner pricing that step as one witness took a cross product
                    // there (work queue Q240).
                    let made = cx.n;
                    let tail = i >= cx.cut_from;
                    let mut limit = if tail { 8 } else { rowcode::CHUNK };
                    let mut cut = false;
                    /// Runs a chunk; whether the head's row was made past the head (`made`, the rows before it).
                    fn flush(cx: &mut Ctx<'_, '_>, i: usize, f: &rowcode::Fused, s: &mut rowcode::Scratch, slots: &mut [i64], made: Option<u64>) -> bool {
                        let mut cut = false;
                        let counts = &cx.counts;
                        // Each fused step's entries and its passes, as `go` counts them step by step.
                        let bump = |m: usize, n: u64| {
                            let c = &counts[i + m][1]; c.set(c.get() + n);
                            let c = &counts[i + 1 + m][0]; c.set(c.get() + n);
                        };
                        bump(0, s.rows as u64);
                        let added = std::cell::Cell::new(s.rows as u64);
                        let words = f.run(s, |m, n| if m + 1 < f.steps { bump(m + 1, n); added.set(added.get() + n) }).to_vec();
                        // The chunk's filter work is charged after its rows run (below): charged first, a chunk
                        // whose filters alone spend the budget cut before its first row, every run (no progress).
                        'rows: for (w, &word) in words.iter().enumerate() {
                            let mut m = word;
                            while m != 0 {
                                // Cut (decision 0206): the rows from here on are left, so the row the cut stopped at
                                // (`at0`) is the last one run, not one of these never run.
                                if cx.stop { break 'rows; }
                                let r = 64 * w + m.trailing_zeros() as usize;
                                m &= m - 1;
                                for &(slot, b) in &f.binds {
                                    slots[slot] = match b { rowcode::Bind::Column(k) => s.columns[k][r], rowcode::Bind::Param(k) => s.params[k] };
                                }
                                if i == 0 { cx.at0 = s.ids[r]; cx.seen0 = true; }
                                go(cx, i + 1 + f.steps, slots);
                                if made.is_some_and(|m| cx.n > m) { cut = true; break 'rows; }
                            }
                        }
                        cx.work += added.get();
                        if cx.work > cx.budget { cx.stop = true; }
                        s.rows = 0;
                        s.ids.clear();
                        s.columns.iter_mut().for_each(Vec::clear);
                        cut
                    }
                    rows.each(cands, need, |id, row| {
                        if cx.stop { return true; }
                        if rows.dead(id) || only.as_ref().is_some_and(|o| !o.has(id)) { return false; }
                        if !keys.iter().all(|&(c, k)| exact >> c & 1 != 0 || row.at(c) == val(slots, k)) || !tests.iter().all(|&(c, c0)| row.at(c) == row.at(c0)) { return false; }
                        for &(k, c) in &f.gather { s.columns[k].push(row.at(c)); }
                        if i == 0 { s.ids.push(id); }
                        s.rows += 1;
                        if s.rows == limit {
                            cut = flush(cx, i, f, &mut s, slots, tail.then_some(made));
                            limit = (limit * 8).min(rowcode::CHUNK);
                            return cut;
                        }
                        false
                    });
                    if s.rows > 0 && !cx.stop && !cut { flush(cx, i, f, &mut s, slots, tail.then_some(made)); }
                    s.rows = 0;
                    s.ids.clear();
                    s.columns.iter_mut().for_each(Vec::clear);
                    cx.scratch[i] = s;
                    if reads_delta { cx.only = only; }
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
                let found = rows.each(cands, need, |id, row| {
                    if cx.stop { return true; }
                    if rows.dead(id) || only.as_ref().is_some_and(|o| !o.has(id)) { return false; }
                    if !keys.iter().all(|&(c, k)| exact >> c & 1 != 0 || row.at(c) == val(slots, k)) || !tests.iter().all(|&(c, c0)| row.at(c) == row.at(c0)) { return false; }
                    if exists { return true; }
                    for &(c, s) in binds { slots[s] = row.at(c); }
                    if i == 0 { cx.at0 = id; cx.seen0 = true; }
                    if let Some(Factor { read, n, upto }) = factor {
                        let mut key = [0i64; FACTOR_MAX];
                        for (x, &s) in read[..n].iter().enumerate() { key[x] = slots[s]; }
                        if let Some(added) = cx.failed[i].get(&key) {
                            for (c, d) in cx.counts[i..].iter().flatten().zip(added.iter()) { c.set(c.get() + d); }
                            cx.skipped += 1;
                            return false;
                        }
                        let (made, deep) = (cx.n, cx.deep);
                        cx.deep = i;
                        let at = cx.snap.len();
                        cx.snap.extend(cx.counts[i..].iter().flatten().map(std::cell::Cell::get));
                        go(cx, i + 1, slots);
                        // Stopped before `upto`: only `read` of this row's bindings was looked at.
                        if cx.n == made && cx.deep < upto && cx.error.is_none() {
                            let added: Box<[u64]> = cx.counts[i..].iter().flatten().zip(&cx.snap[at..]).map(|(c, b)| c.get() - b).collect();
                            cx.failed[i].insert(key, added);
                        }
                        cx.snap.truncate(at);
                        cx.deep = cx.deep.max(deep);
                        return i >= cx.cut_from && cx.n > made;
                    }
                    let made = cx.n;
                    go(cx, i + 1, slots);
                    // The head's row is made, and nothing from here on binds what it reads: every other row of this
                    // step makes the same one again (`tail`).
                    i >= cx.cut_from && cx.n > made
                });
                if reads_delta { cx.only = only; }
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
                    let (rows, cands) = candidates(src, store, &cx.key, keys, &mut [0; runs::HINTS], &mut None, 0, None);
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
                let (rows, cands) = candidates(src, store, &cx.key, keys, &mut cx.hints[i], &mut cx.last[i], 0, None);
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
            Step::Cat { out, a, b, bind } if j.probes[i] & TEXT_OUT != 0 || [val(slots, *a), val(slots, *b)].iter().any(|v| scratch(cx, *v).is_some()) => {
                // A part or the whole held as text (`TEXT_OUT`): `@dot` of the texts, as `cat` makes it.
                let (x, y) = (val(slots, *a), val(slots, *b));
                let first = text(cx, x);
                let joined = if first.is_empty() { None } else { Some(format!("{first}.{}", text(cx, y))) };
                let mark = cx.texts.len();
                let v = match joined {
                    None if j.probes[i] & TEXT_OUT != 0 || scratch(cx, y).is_none() => y,
                    None => text_value(cx, j.probes[i], text(cx, y)),
                    Some(t) => text_value(cx, j.probes[i], t),
                };
                if v == -1 && j.probes[i] & PROBE_OUT != 0 { return }
                if *bind { slots[*out] = v } else if slots[*out] != v { return }
                go(cx, i + 1, slots);
                cx.texts.truncate(mark);
            }
            Step::Cat { out, a, b, bind } => {
                let v = if j.probes[i] & UNREAD_OUT != 0 {
                    0
                } else if j.probes[i] & PROBE_OUT != 0 {
                    match cat_held(cx.syms, val(slots, *a), val(slots, *b)) { Some(v) => v, None => return }
                } else {
                    cat(cx.syms, val(slots, *a), val(slots, *b))
                };
                if *bind { slots[*out] = v } else if slots[*out] != v { return }
                go(cx, i + 1, slots);
            }
            Step::Split { whole, a, b, ka, kb } => {
                // As `cat` makes it: `"" . w`, or at each "." with a part before it.
                let probe = j.probes[i];
                // The whole's text, and each part's atom where the part is only looked up (`PROBE_A`, `PROBE_B`), read
                // under one read of the arena. `Err`: not known in memory, looked up after (`held`). A part not in
                // memory where a miss is final has no atom (`Atoms::misses_are_final`).
                type Part = Result<Option<i64>, ()>;
                let at = |w: &str| std::iter::once(None).chain(w.match_indices('.').filter(|(p, _)| *p > 0).map(|(p, _)| Some(p))).collect::<Vec<_>>();
                let (pa, pb) = (a.is_some() && probe & PROBE_A != 0 && probe & UNREAD_A == 0, b.is_some() && probe & PROBE_B != 0 && probe & UNREAD_B == 0);
                let fin = cx.syms.atoms.misses_are_final();
                let read = crate::content_store::atoms::Sym::try_from(val(slots, *whole)).ok().and_then(|id| cx.syms.atoms.in_memory(|m| {
                    let w = m.str(id)?;
                    let look = |t: &str| -> Part { match m.get(t) { Some(v) => Ok(Some(i64::from(v))), None if fin => Ok(None), None => Err(()) } };
                    let parts: Vec<(Option<usize>, Part, Part)> = at(w).into_iter().map(|p| {
                        let (x, y) = p.map_or(("", w), |p| (&w[..p], &w[p + 1..]));
                        (p, if pa { look(x) } else { Err(()) }, if pb { look(y) } else { Err(()) })
                    }).collect();
                    Some((w.to_owned(), parts))
                }));
                let (w, parts) = read.unwrap_or_else(|| {
                    let w = cx.syms.name(val(slots, *whole));
                    let parts = at(&w).into_iter().map(|p| (p, Err(()), Err(()))).collect();
                    (w, parts)
                });
                for (p, ax, by) in parts {
                    let (x, y) = p.map_or(("", w.as_str()), |p| (&w[..p], &w[p + 1..]));
                    let held = |cx: &mut Ctx<'_, '_>, part: Part, t: &str| match part { Ok(v) => v, Err(()) => cx.syms.held(t) };
                    match a {
                        Some(s) if probe & UNREAD_A != 0 => slots[*s] = 0,
                        Some(s) if probe & PROBE_A != 0 => match held(cx, ax, x) { Some(v) => slots[*s] = v, None => continue },
                        Some(s) => slots[*s] = cx.syms.id(x),
                        None => if cx.syms.name(val(slots, *ka)) != x { continue },
                    }
                    match b {
                        Some(s) if probe & UNREAD_B != 0 => slots[*s] = 0,
                        Some(s) if probe & PROBE_B != 0 => match held(cx, by, y) { Some(v) => slots[*s] = v, None => continue },
                        Some(s) => slots[*s] = cx.syms.id(y),
                        None => if cx.syms.name(val(slots, *kb)) != y { continue },
                    }
                    go(cx, i + 1, slots);
                }
            }
            Step::Eq { a, b } => if val(slots, *a) == val(slots, *b) { go(cx, i + 1, slots) },
            Step::Ne { a, b } => if val(slots, *a) != val(slots, *b) { go(cx, i + 1, slots) },
            Step::Func { name, kinds, args, out, bind, texts } => {
                use crate::code_rule_dsl::logic::strings::functions::{eval, node_read, Kind};
                let value = if let Some(nodes) = texts {
                    let read = match nodes {
                        NodeTexts::Held(nodes) => node_read(name, Some(&**nodes), val(slots, args[0])),
                        NodeTexts::At(at) => {
                            let nodes = at.upgrade().and_then(|d| crate::code_rule_dsl::facts::vtab::bound_source_in(&d, crate::code_rule_dsl::facts::vtab::syntax::LONG_TEXT_NODES));
                            node_read(name, nodes.as_deref(), val(slots, args[0]))
                        }
                    };
                    read.map_or(rusqlite::types::Value::Null, rusqlite::types::Value::Text)
                } else {
                    let mut strs = Vec::new();
                    let mut numbers = Vec::new();
                    for (k, a) in kinds.iter().zip(args) {
                        match k { Kind::Text => strs.push(text(cx, val(slots, *a))), Kind::Number => numbers.push(val(slots, *a)) }
                    }
                    let strs: Vec<&str> = strs.iter().map(String::as_str).collect();
                    eval(name, &strs, &numbers)
                };
                let mark = cx.texts.len();
                let v = match value {
                    rusqlite::types::Value::Integer(n) => n,
                    // Looked up and no atom: no row holds it.
                    rusqlite::types::Value::Text(s) => match text_value(cx, j.probes[i], s) {
                        -1 if j.probes[i] & PROBE_OUT != 0 => return,
                        v => v,
                    },
                    _ => return,
                };
                if *bind { slots[*out] = v } else if slots[*out] != v { return }
                go(cx, i + 1, slots);
                cx.texts.truncate(mark);
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
    /// A first step that scans many rows, walked in morsels on the pool (MLDB's `parallelMapChunked` over a range,
    /// each chunk with its own accumulator, merged in order): its rows in the next step's key order (`scan_order`),
    /// cut into about `MORSELS_PER_THREAD` ranges a thread, of at least `MORSEL_ROWS` rows each. Each morsel walks
    /// with its own counts, interning cursor and new rows, which it settles (`Fresh::settle`: sorted by the first
    /// order, distinct, not held) before it ends; its rows are taken into the walk's in the morsels' order
    /// (`Fresh::absorb`), and a collecting walk's rows are its morsels' in order, as one walk reads them. Whether it
    /// did: not for a limited walk (its cut is a row of one walk's order), a walk under a test's count tables made at
    /// once (`count_tables_at_once`, this thread's), a step that is not a plain scan, nor on one thread.
    fn morsels(cx: &mut Ctx<'_, '_>, slots: &[i64]) -> bool {
        const MORSEL_ROWS: usize = 4096;
        const MORSELS_PER_THREAD: usize = 4;
        let threads = crate::threads::max();
        // Rows restricted at a later step than the first are read there, by one walk.
        if threads < 2 || (cx.only.is_some() && cx.only_step != 0) || table::at_once() {
            return false;
        }
        let (j, store) = (cx.j, cx.store);
        let Some(Step::Atom { src, neg: false, keys, binds, .. }) = j.steps.first() else { return false };
        if !keys.is_empty() || !morsels_on() {
            return false;
        }
        // A walk that may make atoms defers its new texts in each morsel (`Deferred`), interned at the merge.
        let defer = makes_atoms(j);
        let mut hints = [0; runs::HINTS];
        let (rows, cands) = candidates_of(src, store, &mut hints);
        let Cands::Range(lo, hi) = cands else { return false };
        if ((hi - lo) as usize) < 2 * MORSEL_ROWS * MORSELS_PER_THREAD {
            return false;
        }
        // The rows in the order one walk reads them, less those a cut's rerun does not read again (decision 0206).
        let only = cx.only.take().filter(|o| o.of == RowsOf::of(src));
        let limited = cx.budget != u64::MAX;
        // The rows a cut left, in their order where that is the key's; else the range's in the key's order (then
        // restricted to the rows left).
        let left: Option<Vec<u32>> = only.as_ref().and_then(|o| o.order.as_ref()).map(|ids| ids.iter().copied().filter(|id| (lo..hi).contains(id)).collect());
        let order = { let _s = crate::code_rule_dsl::logic::profile::span(|| "the morsels' scan order (scan_order)".into()); scan_order(j, store, 0, rows, &Cands::Range(lo, hi), binds, left.as_deref(), None) };
        let list: Option<Vec<u32>> = match (order, &only, left) {
            (o, _, Some(left)) => Some(o.unwrap_or(left)),
            (o, Some(only), None) => Some(o.map_or_else(|| (lo..hi).filter(|&id| only.has(id)).collect(), |o| o.into_iter().filter(|&id| only.has(id)).collect())),
            (Some(o), None, None) => Some(o),
            (None, None, None) if limited => Some((lo..hi).collect()),
            (None, None, None) => None,
        };
        let n = list.as_ref().map_or((hi - lo) as usize, Vec::len);
        // A limited walk's budget is shared out evenly, rounded up: each morsel stops past its share, so a cut walk's
        // work reaches its budget, as one walk's does (the planner checks the line there: short of it by a share's
        // rounding, the line went on with a budget of a few rows, cut again), and where it stops does not depend on
        // the threads' timing.
        let left = cx.budget.saturating_sub(cx.work);
        let parts = (threads * MORSELS_PER_THREAD).min(n / MORSEL_ROWS).min(if limited { (left / MORSEL_ROWS as u64) as usize } else { usize::MAX });
        if parts < 2 {
            if let Some(o) = only { cx.only = Some(o); }
            return false;
        }
        // Each morsel's counts hold its own entry into the first step, the walk's one: a share more each.
        let share = if limited { (left + parts as u64).div_ceil(parts as u64) } else { u64::MAX };
        let cut = |p: usize| p * n / parts;
        let firsts: Vec<Cands<'_>> = (0..parts).map(|p| match &list {
            Some(o) => Cands::Slice(&o[cut(p)..cut(p + 1)]),
            None => Cands::Range(lo + cut(p) as u32, lo + cut(p + 1) as u32),
        }).collect();
        struct Out { n: u64, fresh: Fresh, rows: Vec<i64>, counts: Vec<[u64; 2]>, skipped: u64, work: u64, failed: Option<String>, error: Option<Error>,
                     left: Option<Vec<u32>>, texts: Vec<String> }
        let (head, collecting, cut_from, cap) = (cx.head, cx.collect.is_some(), cx.cut_from, cx.cap);
        let atoms = Arc::clone(&cx.syms.atoms);
        let mut outs: Vec<Option<Out>> = (0..parts).map(|_| None).collect();
        crate::threads::scope(|jobs| {
            for (first, out) in firsts.into_iter().zip(outs.iter_mut()) {
                let atoms = Arc::clone(&atoms);
                jobs.spawn("a join's morsel", move |_| {
                    let mut syms = Interner { atoms, buf: String::new(), failed: None, deferred: defer.then(Default::default) };
                    let mut fresh: Vec<Fresh> = if collecting { Vec::new() } else { vec![Fresh::default(); head + 1] };
                    let mut rows = Vec::new();
                    let factor = factored(j);
                    let failed = factor.iter().map(|_| Default::default()).collect();
                    let mut w = Ctx { j, head, store, syms: &mut syms, fresh: &mut fresh, n: 0, key: Vec::new(), row: Vec::new(), error: None, cap, pending: Vec::new(),
                                      scratch: j.steps.iter().map(|_| rowcode::Scratch::default()).collect(), texts: Vec::new(),
                                      collect: if collecting { Some(&mut rows) } else { None }, at0: 0, seen0: false, hints: vec![[0; runs::HINTS]; j.steps.len()], last: vec![None; j.steps.len()],
                                      factor, failed, snap: Vec::new(), skipped: 0, deep: 0, work: 0, budget: share, stop: false,
                                      only_step: usize::MAX, only: None, list0: None, cut_from,
                                      counts: j.counts.iter().map(|_| Default::default()).collect(), first: Some((rows_of_first(src, store), first)),
                                      left0: None, clock: StepClock::start(j.steps.len()) };
                    let mut slots = slots.to_vec();
                    go(&mut w, 0, &mut slots);
                    let (n, skipped, work, error) = (w.n, w.skipped, w.work, w.error.take());
                    // Cut at its share: its rows from the one it stopped in (that row included), as one walk's cut.
                    let left = w.stop.then(|| w.list0.as_ref().map_or_else(Vec::new, |(_, list)| {
                        let at = if w.seen0 { list.iter().rposition(|&x| x == w.at0).unwrap_or(0) } else { 0 };
                        list[at..].to_vec()
                    }));
                    let counts = w.counts.iter().map(|[a, b]| [a.get(), b.get()]).collect();
                    // Its own thread's CPU, by step: added to the join's as it ends (the counts go by the walk's).
                    if let Some(clock) = w.clock.take() { clock.end(j); }
                    drop(w);
                    let mut mine = if collecting { Fresh::default() } else { std::mem::take(&mut fresh[head]) };
                    let texts = syms.deferred.take().map_or_else(Vec::new, |d| d.texts);
                    // Rows holding a deferred text are settled once they hold its atom (`settle_deferred`).
                    if !collecting && texts.is_empty() { mine.settle(&store.rels[head]); }
                    *out = Some(Out { n, fresh: mine, rows, counts, skipped, work, failed: syms.failed.take(), error, left, texts });
                });
            }
        });
        if limited {
            cx.list0 = Some((RowsOf::of(src), list.unwrap_or_default()));
        }
        if outs.iter().flatten().any(|o| !o.texts.is_empty()) {
            settle_deferred(cx, &mut outs.iter_mut().flatten().map(|o| (&mut o.texts, &mut o.fresh, &mut o.rows)).collect::<Vec<_>>());
        }
        let _taken = crate::code_rule_dsl::logic::profile::span(|| "the morsels' rows taken (absorb)".into());
        for out in outs.into_iter().flatten() {
            // Each morsel entered the first step once, the walk once (as its counts, below); one that stopped also
            // counted the step it was refused, a walk that stops once.
            cx.work += out.work.saturating_sub(1 + u64::from(out.left.is_some()));
            if let Some(l) = out.left {
                cx.stop = true;
                cx.left0.get_or_insert_with(Vec::new).extend(l);
            }
            cx.n += out.n;
            cx.skipped += out.skipped;
            for (c, [a, b]) in cx.counts.iter().zip(out.counts) { c[0].set(c[0].get() + a); c[1].set(c[1].get() + b); }
            // Each morsel entered the first step once; the walk did, once.
            let c = &cx.counts[0][0];
            c.set(c.get() - 1);
            if let Some(f) = out.failed { cx.syms.failed.get_or_insert(f); }
            if cx.error.is_none() { cx.error = out.error; }
            match cx.collect.as_deref_mut() {
                Some(c) => c.extend_from_slice(&out.rows),
                None => {
                    cx.fresh[head].absorb(out.fresh);
                    // Each morsel checked the cap on its own rows (decision 0462): together they are checked again.
                    let rel = &cx.store.rels[head];
                    if cx.error.is_none() && cx.fresh[head].n + rel.n > cx.cap { cx.error = Some(super::width::too_wide(&rel.table, cx.cap)); }
                }
            }
        }
        cx.counts[0][0].set(cx.counts[0][0].get() + 1);
        cx.work += 1 + u64::from(cx.stop);
        IN_MORSELS.with(|m| m.set(m.get() + 1));
        true
    }
    /// A walk in one piece. One that may make atoms defers its new texts, as a morsel does (`Deferred`), and interns
    /// them at its end with one `intern_all` (`settle_deferred`). Interned row by row, each new text took the arena's
    /// write lock, and every thread reading a text (`cat_held`, `with_str`) waited behind it: over PostHog, the run's
    /// most-waited lock once the planner's were gone (decision 0315). The walk's head rows go to a `Fresh` of its own
    /// (its rows hold deferred values until settled), absorbed into the stratum's as a morsel's are.
    fn whole(cx: &mut Ctx<'_, '_>, slots: &mut [i64]) {
        if !makes_atoms(cx.j) || cx.syms.deferred.is_some() {
            return go(cx, 0, slots);
        }
        cx.syms.deferred = Some(Default::default());
        let (head, collecting) = (cx.head, cx.collect.is_some());
        let from = cx.collect.as_ref().map_or(0, |c| c.len());
        let before = if collecting { None } else { Some(std::mem::take(&mut cx.fresh[head])) };
        go(cx, 0, slots);
        let mut texts = cx.syms.deferred.take().map_or_else(Vec::new, |d| d.texts);
        let mut mine = if collecting { Fresh::default() } else { std::mem::take(&mut cx.fresh[head]) };
        let mut rows = cx.collect.as_deref_mut().map_or_else(Vec::new, |c| c.split_off(from));
        if texts.is_empty() {
            if !collecting { mine.settle(&cx.store.rels[head]); }
        } else {
            run_counts().deferred_walks[0].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            run_counts().deferred_walks[1].fetch_add(texts.len() as u64, std::sync::atomic::Ordering::Relaxed);
            settle_deferred(cx, &mut [(&mut texts, &mut mine, &mut rows)]);
        }
        match (cx.collect.as_deref_mut(), before) {
            (Some(c), _) => c.extend_from_slice(&rows),
            (None, Some(mut f)) => {
                f.absorb(mine);
                cx.fresh[head] = f;
            }
            (None, None) => unreachable!("a walk not collecting kept its head's rows"),
        }
    }
    /// The morsels' deferred texts interned, in the morsels' order (one `intern_all`), and each morsel's rows read
    /// with their atoms in place of its deferred values, in the head's columns that may hold a made text
    /// (`made_columns`); then each morsel's rows settled, on the pool, as a morsel that made no text settles its own.
    fn settle_deferred(cx: &mut Ctx<'_, '_>, outs: &mut [(&mut Vec<String>, &mut Fresh, &mut Vec<i64>)]) {
        let mut order: Vec<&str> = Vec::new();
        let mut seen: HashMap<&str, usize> = HashMap::default();
        let at: Vec<Vec<usize>> = outs.iter().map(|(texts, _, _)| texts.iter().map(|t| *seen.entry(t.as_str()).or_insert_with(|| { order.push(t.as_str()); order.len() - 1 })).collect()).collect();
        let atoms = match cx.syms.atoms.intern_all(&order) {
            Ok(a) => a,
            Err(e) => {
                cx.syms.failed.get_or_insert_with(|| e.to_string());
                vec![0; order.len()]
            }
        };
        drop(seen);
        let (j, store, head, collecting) = (cx.j, cx.store, cx.head, cx.collect.is_some());
        let made = made_columns(j);
        let width = j.out.len();
        crate::threads::scope(|jobs| {
            for ((texts, fresh, rows), at) in outs.iter_mut().zip(&at) {
                if texts.is_empty() {
                    continue;
                }
                let map: Vec<i64> = at.iter().map(|&i| i64::from(atoms[i])).collect();
                let made = &made;
                jobs.spawn("a morsel's made texts", move |_| {
                    let swap = |data: &mut [i64]| for row in data.chunks_exact_mut(width) {
                        for (v, _) in row.iter_mut().zip(made).filter(|(_, m)| **m) {
                            if let Some(&a) = v.checked_sub(DEFERRED).and_then(|i| map.get(usize::try_from(i).ok()?)) {
                                *v = a;
                            }
                        }
                    };
                    if collecting {
                        swap(rows);
                    } else {
                        assert_eq!(store.rels[head].k, width, "a head row is its terms");
                        swap(&mut fresh.data[..fresh.n * width]);
                        // Sorted whole: the atoms' order is not the deferred values'.
                        fresh.settled = 0;
                        fresh.lost = true;
                        fresh.breaks.clear();
                        fresh.settle(&store.rels[head]);
                    }
                    texts.clear();
                });
            }
        });
    }
    /// Per column of the head, whether it may hold a text the walk makes: a `@cat` of the head, or a slot a `@cat`,
    /// a split, a text function or a joined text binds, or one a `Let` copies from such a slot.
    fn made_columns(j: &JoinRule) -> Vec<bool> {
        use crate::code_rule_dsl::logic::strings::functions::{signature, Kind};
        let mut made = vec![false; j.nslots];
        let mark = |s: usize, made: &mut Vec<bool>| if let Some(m) = made.get_mut(s) { *m = true };
        for step in &j.steps {
            match step {
                Step::Cat { out, .. } | Step::StrJoin { out, .. } => mark(*out, &mut made),
                Step::Func { name, out, .. } if signature(name).is_none_or(|(_, _, k)| matches!(k, Kind::Text)) => mark(*out, &mut made),
                Step::Split { a, b, .. } => {
                    for s in a.iter().chain(b) {
                        mark(*s, &mut made);
                    }
                }
                Step::Let { a, b: Key::Slot(src) } if made.get(*src).copied().unwrap_or(false) => mark(*a, &mut made),
                _ => {}
            }
        }
        j.out.iter().map(|t| match t { Term::Cat(..) => true, Term::Slot(s) => made.get(*s).copied().unwrap_or(false), Term::Const(_) => false }).collect()
    }
    /// Whether the walk may make atoms row by row: a joined head (`@cat` in it), a joined text, or a `@cat`, text
    /// function or split part whose text is interned (neither looked up, unread, nor held as the walk's scratch:
    /// `probes`). In morsels, such a walk defers its new texts (`Deferred`), interned at the merge in the morsels'
    /// order, so their ids are one walk's (decision 0292 §1; 0279 §7 kept such a walk whole). A walk that only
    /// reads texts walks in morsels as it did (0279 §7): the arena's reads take a shard of their own.
    fn makes_atoms(j: &JoinRule) -> bool {
        const KEPT: u8 = PROBE_OUT | UNREAD_OUT | TEXT_OUT;
        j.out.iter().any(|t| matches!(t, Term::Cat(..)))
            || j.steps.iter().zip(&j.probes).any(|(s, &p)| match s {
                Step::Cat { .. } | Step::Func { .. } => p & KEPT == 0,
                Step::Split { a, b, .. } => (a.is_some() && p & (PROBE_A | UNREAD_A) == 0) || (b.is_some() && p & (PROBE_B | UNREAD_B) == 0),
                Step::StrJoin { .. } => true,
                _ => false,
            })
    }
    /// A plain scan's rows and candidates, as `candidates` gives them for a step with no key.
    fn candidates_of<'a>(src: &'a Src, store: &'a Store, _hints: &mut [u32; runs::HINTS]) -> (Rows<'a>, Cands<'a>) {
        match src {
            Src::In(rel, _) => (Rows::In(rel), Cands::Range(0, rel.len() as u32)),
            Src::FullMulti(r, _) => (Rows::Comp(&store.rels[*r]), Cands::Owned(Vec::new())),
            Src::Delta(r) => { let rel = &store.rels[*r]; (Rows::Comp(rel), Cands::Range(rel.lo as u32, rel.hi as u32)) }
            Src::Full(r) => { let rel = &store.rels[*r]; (Rows::Comp(rel), Cands::Range(0, rel.n as u32)) }
        }
    }
    fn rows_of_first<'a>(src: &'a Src, store: &'a Store) -> Rows<'a> {
        let mut hints = [0; runs::HINTS];
        candidates_of(src, store, &mut hints).0
    }
    let scratch = j.steps.iter().map(|_| rowcode::Scratch::default()).collect();
    let factor = factored(j);
    let failed = factor.iter().map(|_| Default::default()).collect();
    let mut cx = Ctx { j, head, store, syms, fresh, n: 0, key: Vec::new(), row: Vec::new(), error: None, cap: super::width::Limits::here().cap, pending: Vec::new(), scratch, texts: Vec::new(), collect, at0: 0, seen0: false, hints: vec![[0; runs::HINTS]; j.steps.len()], last: vec![None; j.steps.len()],
                       factor, failed, snap: Vec::new(), skipped: 0, deep: 0,
                       work: 0, budget: limit.as_ref().map_or(u64::MAX, |l| l.budget), stop: false,
                       only_step: limit.as_ref().and_then(|l| l.only.as_ref()).map_or(usize::MAX, |o| o.0), only: limit.and_then(|l| l.only).map(|o| o.1), list0: None,
                       cut_from: usize::MAX, counts: j.counts.iter().map(|_| Default::default()).collect(), first: None, left0: None,
                       clock: StepClock::start(j.steps.len()) };
    if cx.collect.is_none() { cx.cut_from = tail(j); }
    let mut slots = vec![0i64; j.nslots];
    for &(s, v) in preset { slots[s] = v; }
    match (each, paired, j.steps.first(), j.steps.get(1)) {
        (Some((y, next)), ..) => loop {
            let out = cx.collect.as_deref_mut().expect("a join run for each value collects");
            let Some(v) = next(out) else { break };
            out.clear();
            slots[y] = v;
            go(&mut cx, 0, &mut slots);
            if cx.error.is_some() || cx.stop { break; }
        },
        // The first two steps' pairs from a level's batch: each left row binds the first atom's slots (its rows
        // passed its tests already), each right row is checked as the walk checks it and binds the second's.
        (None, Some(p), Some(Step::Atom { src: Src::In(r0, _), binds: b0, .. }), Some(Step::Atom { src: Src::In(r1, _), keys: k1, binds: b1, tests: t1, .. })) => {
            let (rows0, rows1) = (Rows::In(r0), Rows::In(r1));
            let bump = |cx: &Ctx<'_, '_>, i: usize, o: usize| { let c = &cx.counts[i][o]; c.set(c.get() + 1); };
            bump(&cx, 0, 0);
            let mut last = u32::MAX;
            for (li, rid) in p.pairs.iter() {
                // At a cut the pairs after it are left: `at0` stays at the row it stopped at.
                if cx.error.is_some() || cx.stop { break; }
                if li != last {
                    last = li;
                    let id0 = p.lids[li as usize];
                    let row = rows0.row(id0);
                    for &(c, s) in b0 { slots[s] = row.at(c); }
                    cx.at0 = id0;
                    cx.seen0 = true;
                    bump(&cx, 0, 1);
                    bump(&cx, 1, 0);
                }
                if rows1.dead(rid) { continue; }
                let row = rows1.row(rid);
                if !k1.iter().all(|&(c, k)| row.at(c) == val(&slots, k)) || !t1.iter().all(|&(c, c0)| row.at(c) == row.at(c0)) { continue; }
                for &(c, s) in b1 { slots[s] = row.at(c); }
                go(&mut cx, 2, &mut slots);
            }
        }
        _ => if !morsels(&mut cx, &slots) { whole(&mut cx, &mut slots) },
    }
    // The walk's counts, into the join's, and its CPU by step.
    for (c, l) in j.counts.iter().zip(&cx.counts) { c[0].add(l[0].get()); c[1].add(l[1].get()); }
    if let Some(clock) = cx.clock.take() { clock.end(j); }
    scalars::explain(&j.steps,store);
    // Each passed-over row was read and tested against a hash set, as a probe is.
    if cx.skipped > 0 { charge(Unit::JoinRow, cx.skipped); j.passed_over.add(cx.skipped); }
    // A limited walk's work, and at a cut its delta rows left: from the first step's row it stopped at (that row
    // included: its rows so far are kept, a set's), in the order read; or, where the first step did not read the
    // delta, every delta row it was to read.
    if cx.budget != u64::MAX {
        let led = cx.list0.is_some();
        let total = cx.list0.as_ref().map_or(0, |l| l.1.len() as u64);
        // Cut: the first step's rows from the one it stopped at; with no rows known, all (`None`).
        let left = cx.stop.then(|| cx.list0.as_ref().map(|(of, list)| {
            if let Some(l) = &cx.left0 { return Rows0::left(*of, l); }
            let at = if cx.seen0 { list.iter().rposition(|&x| x == cx.at0).unwrap_or(0) } else { 0 };
            Rows0::left(*of, &list[at..])
        }));
        let done = match &left { Some(Some(l)) => total - l.n.min(total), Some(None) => 0, None => total };
        if let Some(l) = limited {
            l.spent = Some(Spent { work: cx.work, left, done, led });
        }
    }
    match cx.error {Some(e)=>Err(e),None=>Ok(cx.n)}
}

/// The slots step `st` binds.
/// A probed atom's range on a column it binds: `col`, after the values `lo` and before `hi` (each with whether it is
/// in the range), which comparisons later in the body test against what is bound before the atom.
/// A probe's range of one column, its ends' values (`Ranged` with its ends read).
type Within = Option<(usize, runs::End, runs::End)>;

#[derive(Clone, Copy)]
struct Ranged { col: usize, lo: Option<(Key, bool)>, hi: Option<(Key, bool)> }

/// Per step, a positive probed atom's range (`Ranged`): `line($p, $l, _)` probed by `$p`, then `$l > $a, $l < $b`
/// with `$a` and `$b` bound before it, reads only the rows of `$p` between them where an order of `line` holds `$l`
/// after `$p` (`Found::within`). The comparisons stay as filters: a range read only narrows the candidates. A zero
/// count's helper over the lines between a decorator and a later def read each pair's file's every line.
fn ranges(steps: &[Step]) -> Vec<Option<Ranged>> {
    let mut before: Vec<usize> = Vec::new();
    steps.iter().enumerate().map(|(i, st)| {
        let r = match st {
            Step::Atom { neg: false, keys, binds, .. } if !keys.is_empty() => {
                let known = |k: &Key| match *k { Key::Const(_) => true, Key::Slot(s) => before.contains(&s) };
                let mut found: Option<Ranged> = None;
                for later in &steps[i + 1..] {
                    let Step::Lt { a, b, or_eq } = later else { continue };
                    // `a < b`: a bound slot below the column is its lower end, above it its upper end.
                    let (col, end, upper) = match (a, b) {
                        (Key::Slot(x), y) if known(y) => match binds.iter().find(|&&(_, s)| s == *x) { Some(&(c, _)) => (c, *y, true), None => continue },
                        (y, Key::Slot(x)) if known(y) => match binds.iter().find(|&&(_, s)| s == *x) { Some(&(c, _)) => (c, *y, false), None => continue },
                        _ => continue,
                    };
                    let r = found.get_or_insert(Ranged { col, lo: None, hi: None });
                    if r.col != col { continue; }
                    let slot = if upper { &mut r.hi } else { &mut r.lo };
                    if slot.is_none() { *slot = Some((end, *or_eq)); }
                }
                found
            }
            _ => None,
        };
        before.extend(bound_by(st));
        r
    }).collect()
}

fn bound_by(st: &Step) -> Vec<usize> {
    match st {
        Step::Atom { neg: false, binds, .. } => binds.iter().map(|&(_, s)| s).collect(),
        Step::Cat { out, bind: true, .. } | Step::Scalar { out, bind: true, .. } | Step::Func { out, bind: true, .. }
        | Step::DocOf { out, bind: true, .. } | Step::StrJoin { out, bind: true, .. } => vec![*out],
        Step::Split { a, b, .. } => a.iter().chain(b).copied().collect(),
        Step::Let { a, .. } => vec![*a],
        _ => Vec::new(),
    }
}

/// The first step from which the body binds no slot the head reads: past it, every row a step reads leads to the
/// same head row, so the walk needs one witness, not all of them (a semi-join on the rest of the body). A factorized
/// part's key, `fzk($d) :- demand($d), node($d, $v, …), line($v, $l)`, read every node of each document for one row
/// a document (PostHog: 10.6 M rows for 7,086 keys at one step).
pub fn tail(j: &JoinRule) -> usize {
    let head: Vec<usize> = j.out.iter().flat_map(|t| match *t { Term::Slot(s) => vec![s], Term::Cat(a, b) => vec![a, b], Term::Const(_) => Vec::new() }).collect();
    j.steps.iter().enumerate().filter(|(_, st)| bound_by(st).iter().any(|s| head.contains(s))).map(|(i, _)| i + 1).max().unwrap_or(0)
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
        let reads = |t: &Step, s: usize| t.mentions(s);
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

    /// A wave's derived rows partly in the first order already (decision 0187: the batch notes where its rows leave
    /// it): stretches in order, rows out of order between them, repeats and rows held from earlier waves, checked in
    /// batches as they come. The relation then holds the same rows in the same places as when every row was sorted
    /// (`add`: in no known order), and an order re-sorted on other columns (`reorder`, its runs' ranges known
    /// sorted) answers as the other's does.
    #[test]
    fn a_waves_rows_in_order_are_merged_not_sorted() {
        let mut x: u64 = 5;
        let mut next = |m: u64| {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((x >> 33) % m) as i64
        };
        let (mut known, mut plain) = (Rel::new("t", b"iii"), Rel::new("t", b"iii"));
        for rel in [&mut known, &mut plain] {
            rel.want(&[2]);
        }
        for w in 0..4 {
            let (mut f, mut g) = (Fresh::default(), Fresh::default());
            let rows = if w == 2 { 2 * FRESH_BATCH + 999 } else { 6000 };
            let mut made = 0;
            while made < rows {
                // A stretch in the first order (tagged: the leading value's tag, then the columns), then a few rows
                // as they come.
                let mut stretch: Vec<Vec<i64>> = (0..100 + next(400)).map(|_| vec![next(60), next(30), next(25)]).collect();
                let first = known.idx[0].cols.clone();
                stretch.sort_by(|a, b| runs::cmp_vals(&first, true, a, b));
                stretch.extend((0..next(4)).map(|_| vec![next(60), next(30), next(25)]));
                for row in stretch.into_iter().take(rows - made) {
                    f.offer(&row, &known);
                    g.offer(&row, &plain);
                    g.lost = true;
                    made += 1;
                }
            }
            assert!(!f.lost, "wave {w}: the batch kept its breaks");
            assert!(f.breaks.len() * 20 < f.n - f.settled + 20, "wave {w}: few breaks");
            known.push_fresh(f, false);
            plain.push_fresh(g, false);
            assert_eq!((known.n, known.data.to_vec(0, known.n, 3)), (plain.n, plain.data.to_vec(0, plain.n, 3)), "wave {w}: the same rows in the same places");
            for rel in [&mut known, &mut plain] {
                let (lo, hi) = (rel.indexed, rel.n);
                rel.set_delta(lo, hi);
            }
        }
        // An order re-sorted on other columns (a chain of key sets), its runs over ranges known sorted or not.
        for rel in [&mut known, &mut plain] {
            rel.want(&[2, 1]);
        }
        let ids = |rel: &Rel, key: &[i64]| {
            let ix = rel.idx.iter().find(|ix| ix.cols[..2] == [2, 1]).expect("an order on (2, 1)");
            let mut v: Vec<u32> = ix.find(&rel.data, 3, key, None).ids().collect();
            v.sort_unstable();
            v
        };
        assert!(known.idx.iter().zip(&plain.idx).all(|(a, b)| a.bytes() <= b.bytes()));
        for a in 0..25 {
            for b in 0..30 {
                assert_eq!(ids(&known, &[a, b]), ids(&plain, &[a, b]), "key {a} {b}");
            }
        }
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
                set.remove(&*rel.row(id as usize));
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

    /// A view of a source's columns (`hold_view`, Q37) has the live statistics a stratum's copy of them has: each
    /// column's distinct values, squared postings and exact counts, so its readers plan as they did with the copy.
    #[test]
    fn a_views_statistics_are_its_copys() {
        let node = |d: i64, i: i64| (d << 32) | i;
        // Few rows, and enough for a job a column (`SPANNED_ROWS`).
        for per in [300, SPANNED_ROWS as i64 / 3 + 1] {
            let mut input = Rel::new("node", b"iiii");
            // Documents, nodes (a key), kinds (few, skewed), and starts (many).
            for d in 1..4 {
                for i in 0..per {
                    input.push(&[d, node(d, i), if i % 5 == 0 { 7 } else { i % 90 }, i * 3]);
                }
            }
            let source = Derived::new(Arc::new(InRel::moved(input)), None);
            for cols in [vec![1, 2], vec![2, 1, 3], vec![0, 1]] {
                let (flat, sketches) = view_rows(source.as_ref(), &cols);
                let n = flat.len() / cols.len();
                let viewed = live_from(n, n, &sketches);
                let mut copy = Rel::new("kind", &vec![b'i'; cols.len()]);
                for row in flat.chunks_exact(cols.len()) {
                    copy.push(row);
                }
                copy.indexed_to_n();
                let copied = live_of(&copy);
                let sorted = |l: &Live| l.freq.iter().map(|(c, v)| { let mut v = v.clone(); v.sort_unstable(); (*c, v) }).collect::<Vec<_>>();
                assert_eq!((viewed.n, &viewed.ndv, &viewed.sq), (copied.n, &copied.ndv, &copied.sq), "{cols:?}");
                assert_eq!(sorted(&viewed), sorted(&copied), "{cols:?}");
                assert_eq!(viewed.freq.iter().any(|(c, _)| cols[*c] == 2), cols.contains(&2), "the kinds' exact counts: {cols:?}");
            }
        }
    }

    /// A source keyed on its node column (`Source::keys_rows`), as a batch's views are: its rows another source's,
    /// a row found by a map of the node column.
    struct NodeKeyed(Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, HashMap<i64, usize>);
    impl crate::code_rule_dsl::facts::vtab::Source for NodeKeyed {
        fn columns(&self) -> Vec<String> { self.0.columns() }
        fn len(&self) -> usize { self.0.len() }
        fn estimated_matches(&self, column: usize, batch: bool) -> usize { self.0.estimated_matches(column, batch) }
        fn indexed(&self, column: usize) -> bool { self.0.indexed(column) }
        fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>> { self.0.lookup(column, values) }
        fn value(&self, row: usize, column: usize) -> i64 { self.0.value(row, column) }
        fn keys_rows(&self, column: usize) -> bool { column == 1 }
        fn key_row(&self, column: usize, value: i64) -> Option<usize> { assert_eq!(column, 1); self.1.get(&value).copied() }
    }

    /// A view's probes by its key column (`InRel::keyed`) find no order and find what a copy's orders find: by the
    /// key alone, the key with another column, keys absent, between documents and past them; a probe without the key
    /// is the copy's order. The key is the source's where it keys a column (a node's ids out of row order, as with
    /// definition ids); else one checked on the rows (a node ascending in each document's rows, the documents not in
    /// key order): not a column that ascends with repeats (the document), nor runs that overlap.
    #[test]
    fn a_views_probes_by_its_key_are_its_copys() {
        let node = |d: i64, i: i64| (d << 32) | i;
        for by_source in [false, true] {
            let mut input = Rel::new("node", b"iiii");
            for d in [3, 2, 1] {
                for i in 0..200 {
                    // By the source: ids out of row order within a document.
                    let id = if by_source { (i * 7) % 200 } else { i };
                    input.push(&[d, node(d, id), id % 7, i * 3]);
                }
            }
            let inner = Derived::new(Arc::new(InRel::moved(input)), None);
            let source: Arc<dyn crate::code_rule_dsl::facts::vtab::Source> = if by_source {
                let rows = (0..inner.len()).map(|r| (inner.value(r, 1), r)).collect();
                Arc::new(NodeKeyed(Arc::clone(&inner), rows))
            } else {
                inner
            };
            for (cols, key) in [(vec![1, 2], Some(0)), (vec![2, 1, 3], Some(1)), (vec![0, 2], None)] {
                let (flat, _) = view_rows(source.as_ref(), &cols);
                let keyed = view_key(&source, &cols, &flat);
                let how = |ky: &Keyed| (ky.col, ky.runs.len(), ky.src.as_ref().map(|s| s.1));
                assert_eq!(keyed.as_ref().map(how), key.map(|c| if by_source { (c, 0, Some(1)) } else { (c, 3, None) }), "{cols:?}");
                let n = flat.len() / cols.len();
                let copy = InRel::from_flat(flat.clone(), n, vec![b'i'; cols.len()]);
                let mut view = Arc::into_inner(InRel::from_flat(flat.clone(), n, vec![b'i'; cols.len()])).expect("a new relation");
                view.keyed = keyed;
                let view = Arc::new(view);
                let Some(kc) = key else { continue };
                // Checked on the rows: out of order within a document, or the rows twice over (each run ascends,
                // the runs overlap), the column is no key.
                let twice: Vec<i64> = (0..2 * n).flat_map(|r| copy.row(r % n).to_vec()).collect();
                assert!(Keyed::of(&twice, cols.len(), kc).is_none(), "{cols:?}");
                assert_eq!(Keyed::of(&flat, cols.len(), kc).is_some(), !by_source, "{cols:?}");
                let other = (kc + 1) % cols.len();
                for probe in [vec![kc], vec![kc, other], vec![other, kc]] {
                    for nd in [node(1, 0), node(1, 199), node(2, 0), node(2, 77), node(3, 199), node(3, 200), node(4, 0), 0, i64::MAX, i64::MIN] {
                        let o = if nd == node(2, 77) { 77 % 7 } else { 0 };
                        let key: Vec<i64> = probe.iter().map(|&c| if c == kc { nd } else if cols[c] == 2 { o } else { 0 }).collect();
                        let rows = |r: &InRel| { let mut out = Vec::new(); r.probe_each(&probe, &key, &mut |row| out.push((0..cols.len()).map(|c| row(c)).collect::<Vec<_>>())); out };
                        assert!(probe != [kc] || nd != node(2, 77) || rows(&copy).len() == 1, "a probe that finds a row");
                        assert_eq!(rows(&view), rows(&copy), "{cols:?} {probe:?} {key:?}");
                        // Its candidates, before the other keys are checked: the key's row.
                        let held = rows(&copy).len().max(usize::from(copy.probe_size(&[kc], &[nd]) > 0.0));
                        assert_eq!(view.probe_size(&probe, &key), held as f64, "{cols:?} {probe:?} {key:?}");
                        let mut seen = 0;
                        let m = view.probe_sample(&probe, &key, 4, &mut || 0, &mut |_| seen += 1);
                        assert_eq!((m, seen), (rows(&copy).len() as f64, rows(&copy).len()), "{cols:?} {probe:?} {key:?}");
                    }
                }
                // The view's index for a probe with its key is the key's search; without it, an order as the copy's.
                for probe in [vec![kc], vec![kc, other]] {
                    assert!(view.index(&probe).keyed, "{cols:?} {probe:?}");
                }
                assert!(!view.index(&[other]).keyed, "{cols:?}");
            }
        }
    }

    /// K-R11 (decision 0456 K7): a relation declared sorted is laid out in canonical order when its component is
    /// done: its live rows (a dead one left out) in its columns' order, known sorted on its leading column for the
    /// whole of them, so an order on it is its rows in place.
    #[test]
    fn a_relation_declared_sorted_is_laid_out_in_order() {
        let mut rel = Rel::new("t", b"ii");
        rel.ordered = true;
        for row in [[5, 1], [2, 9], [5, 0], [1, 1], [2, 3]] {
            assert!(rel.push(&row));
        }
        assert!(kill(&mut rel.dead, 1));
        rel.ndead += 1;
        let r = InRel::moved(rel);
        let rows: Vec<[i64; 2]> = (0..r.n).map(|i| [r.at(i).at(0), r.at(i).at(1)]).collect();
        assert_eq!(rows, [[1, 1], [2, 3], [5, 0], [5, 1]]);
        assert_eq!((r.ndead, r.sorted.within(&[0], 2, 0, r.n as u32)), (0, vec![(0, 4)]));
    }

    /// Decision 0456 K7: a source whose rows lie in an order (`Source::order`, a kept relation read in place) is known
    /// sorted over every row on its order's columns but the last (as a relation laid out in order is), so a scan of
    /// it for the next step's probes is not sorted again; one with no order is known in none.
    #[test]
    fn a_source_in_order_is_known_sorted() {
        struct InOrder(Arc<dyn crate::code_rule_dsl::facts::vtab::Source>);
        impl crate::code_rule_dsl::facts::vtab::Source for InOrder {
            fn columns(&self) -> Vec<String> { self.0.columns() }
            fn len(&self) -> usize { self.0.len() }
            fn estimated_matches(&self, c: usize, batch: bool) -> usize { self.0.estimated_matches(c, batch) }
            fn indexed(&self, _: usize) -> bool { false }
            fn lookup(&self, _: usize, _: &[i64]) -> Option<Vec<usize>> { None }
            fn value(&self, row: usize, c: usize) -> i64 { self.0.value(row, c) }
            fn order(&self) -> Option<Vec<usize>> { Some(vec![0, 1]) }
        }
        let cols = vec![Some(vec![1, 1, 2, 5]), Some(vec![3, 4, 0, 9])];
        let src = super::columns_source(vec!["a".into(), "b".into()], cols);
        let r = InRel::over_source(Arc::new(InOrder(src.clone())), b"ii".to_vec());
        assert_eq!(r.sorted.within_key(2, None, &[0], 0, 4), vec![(0, 4)], "a scan by its first column");
        // Not in the dedup order on both columns, which leads with a hash (`runs::tagged`).
        assert!(r.sorted.within_key(2, Some(0), &[0, 1], 0, 4).is_empty(), "not in the dedup order");
        assert!(r.sorted.within_key(2, None, &[1], 0, 4).is_empty(), "not by its second alone");
        assert!(InRel::over_source(src, b"ii".to_vec()).sorted.within_key(2, None, &[0], 0, 4).is_empty(), "a source with no order");
    }

    /// A relation over a source (a kept relation bound in place, decision 0456 step 6b) has no rows of its own: its
    /// values, its live rows and their batches are read through it, as a held relation's are (a value of it once
    /// panicked, "row 0 of a relation of 0 rows", and its live rows were none, so a kept stratum's put held no rows).
    #[test]
    fn a_relation_over_a_source_reads_its_rows_through_it() {
        let src = super::columns_source(vec!["a".into(), "b".into()], vec![Some(vec![1, 2, 5]), Some(vec![3, 4, 0])]);
        let r = InRel::over_source(src, b"ii".to_vec());
        assert_eq!((r.len(), r.rows()), (3, 3));
        assert_eq!((0..3).map(|i| [r.value(i, 0), r.value(i, 1)]).collect::<Vec<_>>(), [[1, 3], [2, 4], [5, 0]], "its values");
        assert_eq!(r.live_rows().as_deref(), Some(&[1, 3, 2, 4, 5, 0][..]), "its live rows");
        let mut batches = Vec::new();
        assert!(r.each_live_batch(1, 9, |b| batches.push(b.to_vec())).is_some());
        assert_eq!(batches, [vec![2, 4, 5, 0]], "rows 1.. in one batch, bounded by its rows");
        let cells = crate::code_rule_dsl::logic::plan::Cells::plain(2);
        // Its digest is its rows', as the same rows held flat have (a sum, blind to their order): a kept relation
        // bound in place and passed on unchanged is its entry, not re-kept. Re-kept (its rows changed), every row is
        // put, read through it: rr-align's 4e3d12466 re-kept views by a frame and `kept_rows`, which take a relation
        // in a block, and lost their rows (7d8103eed's warm-run losses, which it met by naming a view by its reads).
        use crate::code_rule_dsl::logic::plan::corpus::retain;
        let flat = InRel::from_flat(vec![5, 0, 1, 3, 2, 4], 3, b"ii".to_vec());
        let d = retain::held_digest(&cells, &r);
        assert!(d.is_some() && d == retain::held_digest(&cells, &flat), "a view's digest is its rows'");
        let put = retain::kept_put(&crate::content_store::KeptRel { name: "r".into(), ident: [1; 32] }, 0, Vec::new(), b"ii".to_vec(), &r);
        match put.what {
            crate::content_store::kept_facts::KeptWhat::Flat { rows, n, .. } => assert_eq!((rows, n), (vec![1, 3, 2, 4, 5, 0], 3), "every row of the view kept"),
            _ => panic!("a view is kept as its rows"),
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
    /// A relation's digest taken from its rows where they lie (`retain::held_digest`) is the digest of a copy of its
    /// live rows (`rows_digest`), dead rows and batch edges included.
    #[test]
    fn a_digest_read_in_place_is_the_digest_of_the_copied_rows() {
        use crate::code_rule_dsl::logic::plan::{Cells, rows_digest};
        use crate::code_rule_dsl::logic::plan::corpus::retain::held_digest;
        let n = 3 * LIVE_BATCH + 17;
        let mut rel = Rel::new("t", b"iii");
        for i in 0..n as i64 {
            rel.push(&[i * 3 - 7, (i * 7919) % 1001 - 500, -i]);
        }
        let old = InRel::moved(rel);
        let mut gone = Rel::new("g", b"iii");
        for id in [0usize, 5, LIVE_BATCH - 1, LIVE_BATCH, 2 * LIVE_BATCH + 3, n - 1] {
            let r = old.row(id).to_vec();
            gone.push(&r);
        }
        let dead = InRel::moved(Rel::based("t", b"iii", &old, Some(&InRel::moved(gone))).expect("few dead"));
        for held in [&old, &dead] {
            let live = held.live_rows().expect("flat");
            let cells = Cells::plain(3);
            assert_eq!(held_digest(&cells, held), Some(rows_digest(&cells, &live, held.rows())), "{} dead", held.ndead);
        }
    }

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

    /// A maintained relation's versions (decision 0364 stage 2): `Old` is its rows at the start, dead rows then
    /// skipped; `Minus` the rows live then and dead now; `Plus` the rows added since and live. Each is read in place:
    /// `Old` shares the relation's rows, `Minus` and `Plus` are views of its ids. A relation made from rows has none.
    #[test]
    fn a_maintained_relations_versions_are_read_where_its_rows_lie() {
        let rows_of = |r: &InRel| {
            let mut out: Vec<Vec<i64>> = (0..r.len()).filter(|&i| r.view.is_some() || !is_dead(&r.dead, i as u32)).map(|i| { let row = r.at(i); (0..r.k).map(|c| row.at(c)).collect() }).collect();
            out.sort();
            out
        };
        let rel_of = |rows: &[[i64; 2]]| {
            let mut r = Rel::new("g", b"ii");
            for row in rows {
                r.push(row);
            }
            InRel::moved(r)
        };
        let mut rel = Rel::new("t", b"ii");
        for i in 0..40 {
            rel.push(&[i, i % 3]);
        }
        // The kept relation has a dead row already (an earlier run's): it is no row of any version.
        let first = Arc::new(InRel::moved(rel));
        let kept = Arc::new(InRel::moved(Rel::based("t", b"ii", &first, Some(&rel_of(&[[7, 1]]))).expect("few dead")));
        assert!(first.version_view(Ver::Old).is_none() && first.version_ids(Ver::Minus).is_none(), "a relation made from rows has no version");
        let mut new = Rel::based("t", b"ii", &kept, Some(&rel_of(&[[2, 2], [5, 2], [30, 0]]))).expect("few dead");
        new.push(&[5, 2]);
        new.push(&[100, 1]);
        let new = Arc::new(InRel::moved(new));
        let old = new.version_view(Ver::Old).expect("a version");
        assert_eq!(rows_of(&old), rows_of(&kept), "old is the kept rows");
        assert!(old.rows.shares(&new.rows), "old shares the relation's rows");
        let minus = new.version_view(Ver::Minus).expect("a version");
        assert_eq!(rows_of(&minus), vec![vec![2, 2], vec![5, 2], vec![30, 0]]);
        let plus = new.version_view(Ver::Plus).expect("a version");
        assert_eq!(rows_of(&plus), vec![vec![5, 2], vec![100, 1]]);
        assert_eq!(new.version_ids(Ver::Plus), Some(vec![40, 41]));
        // The delta is the versions' rows.
        let (mut added, mut removed) = new.delta_since(&kept).expect("started as kept");
        let flat = |f: &mut Vec<i64>| { let mut r: Vec<Vec<i64>> = f.chunks_exact(2).map(<[i64]>::to_vec).collect(); r.sort(); r };
        assert_eq!(flat(&mut added), rows_of(&plus));
        assert_eq!(flat(&mut removed), rows_of(&minus));
    }

    /// A relation the maintain mode starts from a kept one read in place (decision 0456 step 6, a view of its
    /// entry: `n` 0, its rows its source's) has its version: its rows then are the view's (ep2's edit runs had none,
    /// "no version after its maintain mode").
    #[test]
    fn a_relation_started_from_a_view_has_its_version() {
        let src = crate::code_rule_dsl::logic::native::component::columns_source(vec!["a".into(), "b".into()], vec![Some(vec![1, 2, 3]), Some(vec![10, 20, 30])]);
        let old = InRel::over_source(src, b"ii".to_vec());
        let mut new = Rel::compacted("t", b"ii", &old, None);
        new.push(&[4, 40]);
        let new = Arc::new(InRel::moved(new));
        assert_eq!(new.start().map(|(n, dead)| (n, dead.len())), Some((3, 0)), "its start: the view's rows");
        assert_eq!(new.version_delta(), Some((vec![4, 40], Vec::new())));
        assert_eq!(new.delta_since(&old), Some((vec![4, 40], Vec::new())));
        let mark = old.mark();
        let change = new.delta_ids_since_mark(&mark).expect("started from the view");
        assert_eq!(change.counts(), (0, 1), "since its mark: one row added, none gone");
        let mut added = Vec::new();
        change.each(1, &mut |r| added.extend_from_slice(r)); // copy: not block data (the test's expected rows)
        assert_eq!(added, vec![4, 40]);
    }

    /// A relation derived again past θ is made a version of its kept rows (0364 R2, `InRel::rebased`): the rows it no
    /// longer derives marked dead, those it newly derives added, so its change is its marks, read in place, for its
    /// readers and for the frames alike. A kept relation with too many rows dead is compacted first, and its change
    /// is still marks against its live rows.
    #[test]
    fn a_relation_derived_again_is_a_version_of_its_kept_rows() {
        let rows_at = |rel: &InRel, ids: Vec<u32>| -> Vec<Vec<i64>> {
            let mut out: Vec<Vec<i64>> = ids.into_iter().map(|id| { let r = rel.at(id as usize); (0..2).map(|c| r.at(c)).collect() }).collect();
            out.sort();
            out
        };
        let derived = |rows: &mut dyn Iterator<Item = [i64; 2]>| {
            let mut rel = Rel::new("t", b"ii");
            for r in rows {
                rel.push(&r);
            }
            InRel::moved(rel)
        };
        // In place (no row dead), and compacted first (half its rows dead: past `based`'s eighth).
        for dead in [0usize, 20] {
            let mut kept = Rel::new("t", b"ii");
            for i in 0..40 {
                kept.push(&[i, i % 3]);
            }
            for id in 0..dead {
                if kill(&mut kept.dead, id as u32) {
                    kept.ndead += 1;
                }
            }
            let old = Arc::new(InRel::moved(kept));
            // Now: the live rows less 25 and 30, with 100 and 101 new.
            let now = derived(&mut (dead as i64..40).filter(|&i| i != 25 && i != 30).map(|i| [i, i % 3]).chain([[100, 1], [101, 2]]));
            let new = InRel::rebased("t", &old, &now).expect("made a version of its kept rows");
            assert_eq!(rows_at(&new, new.version_ids(Ver::Minus).expect("a version")), vec![vec![25, 1], vec![30, 0]], "dead {dead}: its rows gone");
            assert_eq!(rows_at(&new, new.version_ids(Ver::Plus).expect("a version")), vec![vec![100, 1], vec![101, 2]], "dead {dead}: its rows added");
            assert_eq!(new.delta_ids_since(&old).expect("started from old").counts(), (2, 2), "dead {dead}: the frames read the same change");
            assert_eq!(new.rows(), now.rows());
        }
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
        assert_eq!(*new.level, *kept);
        let same = InRel::moved(Rel::based("t", b"ii", &old, None).expect("nothing gone"));
        assert_eq!(*same.level, *old.level);
    }

    /// Rows held (`held`) measure what `syntax::measured` gives, a column held nowhere as a key; and they keep it.
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

    /// A stored table's column in row order is its own index (from django-bugs' 63c36e0c0): a probe of it is a range
    /// of row ids (`sorted_range`), the rows a sorted index gives, and its distinct values are counted along it; no
    /// index is built for it.
    #[test]
    fn a_column_in_row_order_is_probed_as_a_range() {
        use crate::code_rule_dsl::facts::vtab::Source;
        let n = 12_299;
        let mut rows: Vec<[i64; 2]> = (0..n as i64).map(|i| [(7 + i / 500) << 32 | (i % 50), (i * 7919) % 97]).collect();
        rows.sort_unstable();
        let cols: Vec<Vec<i64>> = (0..2).map(|c| rows.iter().map(|r| r[c]).collect()).collect();
        let stored = || {
            let t = Stored::new(vec!["n".into(), "a".into()], cols.iter().map(|c| { let l = std::sync::OnceLock::new(); let _ = l.set(c.clone()); l }).collect());
            let _ = t.n.set(cols[0].len());
            t
        };
        let (fresh, plain) = (stored(), stored());
        let _ = plain.sorted(0);
        for v in [cols[0][0], cols[0][2000], cols[0][cols[0].len() / 2], -5] {
            let range: Vec<u32> = fresh.sorted_range(0, v).expect("in order").collect();
            assert_eq!(range, plain.rows_of(0, v), "range of {v}");
            assert_eq!(fresh.lookup(0, &[v]), plain.lookup(0, &[v]));
        }
        assert_eq!(fresh.distinct(0), plain.sorted(0).keys.len());
        assert!(fresh.sorted_range(1, 40).is_none(), "a column out of order has no range");
        assert!(fresh.index[0].get().is_none(), "no index of the column in order");
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
            InRel { vers: Default::default(), viewed: false, keyed: self.keyed.clone(), k: self.k, n: self.n, rows: self.rows.clone(), view: None, base: false, types: self.types.clone(), stored: self.stored, idx: Mutex::new(HashMap::default()), chains: self.chains.clone(), sorted: self.sorted.clone(),
                    sketches: self.sketches.clone(), serves: self.serves.clone(), dead: self.dead.clone(), ndead: self.ndead, summaries: Default::default(), measured: Default::default(), counted: Default::default(), tables: Default::default(), name: Default::default(), from: None, uid: Default::default(), level: self.level.clone(), stats: self.stats.clone() }
        }
    }
}

#[cfg(test)]
pub mod probe_tests {
    use super::*;
    use crate::code_rule_dsl::logic::strings::functions::Kind;

    /// A slot a later step tests its result against (`bind: false`) is read by that step: a split's part so tested
    /// is no unread part (held as 0, the test would fail on every row), nor only a probe's key. `types.rerail`'s `hexpr`,
    /// planned from `res($t, $d)`: `$t = @cat($d0, $r)` split `"pkg.m.B"`, then `$r = @substr($h, …)` tested the
    /// part, and the dotted annotation resolved to nothing.
    #[test]
    fn a_tested_result_reads_its_slot() {
        // Slots: 0 the whole, 1 and 2 its parts, 3 the text the function reads.
        let split = Step::Split { whole: Key::Slot(0), a: Some(1), b: Some(2), ka: Key::Slot(1), kb: Key::Slot(2) };
        let tested = |out| Step::Func { name: "substr", kinds: &[Kind::Text, Kind::Number, Kind::Number], args: vec![Key::Slot(3), Key::Const(0), Key::Const(1)], out, bind: false, texts: None };
        let made = Step::Func { name: "substr", kinds: &[Kind::Text, Kind::Number, Kind::Number], args: vec![Key::Slot(3), Key::Const(0), Key::Const(1)], out: 2, bind: true, texts: None };
        assert!(tested(2).mentions(2) && !made.mentions(2));
        let cat = |bind| Step::Cat { out: 2, a: Key::Slot(3), b: Key::Slot(3), bind };
        assert!(cat(false).mentions(2) && !cat(true).mentions(2));
        let p = probes(&[split.clone(), tested(2)], &[]);
        assert_eq!(p[0] & (UNREAD_B | PROBE_B), 0, "the part a function tests is held as its text's atom");
        assert_eq!(p[0] & UNREAD_A, UNREAD_A, "the part nothing reads is unread");
        let p = probes(&[split, cat(false)], &[]);
        assert_eq!(p[0] & (UNREAD_B | PROBE_B), 0, "the part a `@cat` tests");
    }
}

#[cfg(test)]
pub mod scope_tests {
    use super::*;

    /// What a connection left by its address (its path, its symbols, the inputs loaded from its database) is dropped
    /// when it closes, whoever loaded the evaluator: a closed connection's address goes to the next one opened. Only
    /// the engine's set-up installed the drop (`install_languages`), so in this crate's own tests a second test's
    /// connection at the first's address read the first's path, and with it the first's `events` (4 rows for 3,000:
    /// `repeated_native_consumers_fold_each_demanded_bag_once` after `native_join_shares_summaries_…`).
    #[test]
    fn a_closed_connection_leaves_nothing_by_its_address() {
        let path = crate::code_rule_dsl::fixtures::temp();
        drop(crate::content_store::ContentStore::open(&path).unwrap());
        let conn = Connection::open(&path).unwrap();
        assert!(conn_path(&conn).ends_with(path.file_name().unwrap().to_str().unwrap()));
        symbols(&conn).unwrap();
        let kept = std::sync::Arc::downgrade(&on(&conn));
        assert!(kept.upgrade().is_some_and(|o| o.path.lock().is_some() && o.syms.lock().is_some()));
        drop(conn);
        assert!(kept.upgrade().is_none(), "the closed connection's path and symbols are dropped with it");
        // A connection opened after it, at its address or not, starts with none of it.
        let next = Connection::open(&path).unwrap();
        assert!(on(&next).path.lock().is_none() && on(&next).syms.lock().is_none());
    }

    /// A loader's cursor (`Interner::for_loading`, at every component's open) takes no lock: while a writer
    /// (`sym_atom`, `sym_cat`) holds the symbols, one is made at once, over the same arena, and interns into it.
    /// It read the arena under the lock, and waited there (Q203 item 3: 2,004 samples of 2,204 parked).
    #[test]
    fn a_loaders_cursor_is_made_while_the_symbols_are_written() {
        let path = crate::code_rule_dsl::fixtures::temp();
        drop(crate::content_store::ContentStore::open(&path).unwrap());
        let conn = Connection::open(&path).unwrap();
        let shared = symbols(&conn).unwrap();
        let mut writing = shared.write().unwrap();
        let mut cursor = Interner::for_loading(&shared);
        assert!(Arc::ptr_eq(cursor.atoms(), writing.atoms()));
        let id = cursor.id("a loader's text");
        assert_eq!(writing.get("a loader's text"), Some(id));
        writing.check().unwrap();
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

/// The run's counts of this module's work (decision 0442: a run's own, `run::local`).
#[derive(Default)]
pub struct SetfixCounts {
    /// Walks in one piece that deferred their new texts and interned them in one batch (`whole`), and those texts: for
    /// the profile.
    pub deferred_walks: [std::sync::atomic::AtomicU64; 2],
    /// Cuts made, cut waves gone on reading only the rows left or all (`restricted`), and trials run (decision 0207),
    /// in this process.
    pub cuts: [std::sync::atomic::AtomicU64; 4],
    /// Derived inputs' orders opened (an index asked for at a join's compile) and made (first probed), decision 0186.
    pub orders_opened: OrderCount,
    pub orders_made: OrderCount,
    /// Rows a cut left that a walk going on from it read in their order, unsorted (`scan_order`, decision 0294).
    pub left_in_order: std::sync::atomic::AtomicU64,
}

/// The run's `SetfixCounts`.
#[inline]
pub fn run_counts() -> std::sync::Arc<SetfixCounts> {
    crate::code_rule_dsl::logic::run::local::<SetfixCounts>()
}

/// A `min` lattice relation for another crate's tests (the device's, decision 0384 step 5): folded on the CPU, or
/// given rows another writer folded with their key map, and read back as each live row's key, value and changes.
#[cfg(any(test, feature = "test-hooks"))]
pub struct TestLattice(Rel);
#[cfg(any(test, feature = "test-hooks"))]
impl TestLattice {
    /// A relation of two columns, `min` by its first.
    pub fn min() -> TestLattice {
        let mut rel = Rel::new("t", b"ii");
        rel.set_lattice("min", 1, Arc::new(super::Min), None).expect("a lattice");
        TestLattice(rel)
    }
    /// Its fold's code, top and widen count, as the device folds by it.
    pub fn device_lattice(&self) -> (u32, i64, Option<u32>) {
        self.0.device_lattice().expect("a lattice").expect("one the device folds")
    }
    /// `rows` (row-major) folded on the CPU.
    pub fn fold(&mut self, rows: &[i64]) -> bool {
        self.0.fold(rows)
    }
    /// The rows `part` another writer folded through `map`, taken as they lie (`Fresh::place_folded`).
    pub fn take_folded(&mut self, part: crate::code_rule_dsl::facts::rows::Part, map: keys::KeyMap) -> bool {
        let mut fresh = Fresh::default();
        fresh.place_folded(Placed::Part(part), None, map);
        self.0.push_fresh(fresh, false)
    }
    /// Its rows, dead ones too.
    pub fn len(&self) -> usize {
        self.0.n
    }
    /// Each live row's key, value and its key's changes, sorted; each checked to be its key's slot's row.
    pub fn live(&self) -> Vec<(i64, i64, u32)> {
        let (rel, lat) = (&self.0, self.0.lat.as_ref().expect("a lattice"));
        let mut out: Vec<(i64, i64, u32)> = (0..rel.n as u32).filter(|&i| !is_dead(&rel.dead, i)).map(|i| {
            let key = rel.data.value(i, 2, 0);
            let s = lat.map.find(&rel.data, 2, &[key]).expect("each live row's key in the map");
            assert_eq!(lat.map.row(s), i, "the key's slot holds its live row");
            (key, rel.data.value(i, 2, 1), lat.map.changes(s))
        }).collect();
        out.sort_unstable();
        out
    }
}
