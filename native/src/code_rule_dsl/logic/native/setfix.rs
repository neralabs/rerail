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
//! `v<slot>`, `i:<int>`, `x:<hex>` or `cat:<slot>:<slot>` (the symbols joined by ".").
//! `set_fixpoint_run(name[, max_waves])` returns "new_rows waves visits probe_runs probe_skips solve_ms total_ms
//! per_probe done": per_probe is a comma-separated "ms:rows" for each probe line in spec order, and done is 1 at the
//! fixpoint (new rows are written then), 0 when max_waves stopped it. `set_fixpoint_stats(name)` and
//! `set_fixpoint_reprobe(name, i, hex SQL)` let the caller replan probes between bounded runs.
mod scalars;
mod liveness;
mod rowcode;
use crate::code_rule_dsl::logic::analysis::scalars::Kind as ScalarKind;
use rusqlite::ffi;
use rusqlite::types::{Value, ValueRef};
use rusqlite::vtab::{Context as VContext, CreateVTab, Filters, IndexConstraintOp, IndexInfo, VTab, VTabConnection, VTabCursor, VTabKind};
use rusqlite::{Connection, Error, Result};
use std::borrow::Cow;
use hashbrown::HashTable;
use rustc_hash::{FxHashMap as HashMap, FxHasher};
use std::ffi::{CStr, CString};
use std::hash::Hasher;
use std::os::raw::c_int;
use std::sync::{Arc, LazyLock, Mutex, RwLock};
use std::time::Instant;

/// Strata captured and run again on their own (decision 0095, step 0).
pub(in crate::code_rule_dsl) mod lab;

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
    fn get(&self, s: &str) -> Option<i64> { self.atoms.get(s).map(i64::from) }
    fn name(&self, v: i64) -> String { self.atoms.with_str(v as crate::content_store::atoms::Sym, str::to_owned).unwrap_or_default() }
    fn len(&self) -> usize { self.atoms.len() }
    /// The first interning that failed since the last check.
    fn check(&mut self) -> Result<()> {
        match self.failed.take() { Some(e) => Err(err(e)), None => Ok(()) }
    }
}
static SYMS: LazyLock<Mutex<HashMap<String, Arc<RwLock<Interner>>>>> = LazyLock::new(Default::default);
/// The symbols of `conn`'s database: the process's arena of it.
fn symbols(conn: &Connection) -> Result<Arc<RwLock<Interner>>> {
    let key = conn.path().unwrap_or_default().to_owned();
    let mut syms = SYMS.lock().unwrap();
    let atoms = crate::content_store::atoms::Atoms::for_connection(conn);
    // A database created again at the same path has new atoms (`Atoms::fresh`).
    if let Some(s) = syms.get(&key) && Arc::ptr_eq(&s.read().unwrap().atoms, &atoms) {
        return Ok(Arc::clone(s));
    }
    let s = Arc::new(RwLock::new(Interner { atoms, buf: String::new(), failed: None }));
    syms.insert(key, Arc::clone(&s));
    Ok(s)
}

/// Intern a whole producer result without SQLite callbacks or a lock held across SQL.
pub(in crate::code_rule_dsl) fn intern_batch<'a>(conn: &Connection, texts: impl IntoIterator<Item = &'a str>) -> Result<Vec<i64>> {
    let shared = symbols(conn)?;
    let mut cursor = Interner::for_loading(&shared);
    let ids = texts.into_iter().map(|text| cursor.id(text)).collect();
    cursor.check()?;
    Ok(ids)
}

/// Decode an entire demand batch directly from the shared atom arena. This
/// submits no SQL and holds no lock across a producer or SQLite callback.
pub(in crate::code_rule_dsl) fn text_batch(conn: &Connection, ids: &[i64]) -> Result<Vec<String>> {
    let shared = symbols(conn)?;
    let cursor = shared.read().unwrap();
    ids.iter().map(|&id| {
        if id < 0 || id as usize >= cursor.len() { return Err(err(format!("no symbol {id}"))); }
        Ok(cursor.name(id))
    }).collect()
}

#[cfg(test)]
mod text_batch_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn decoding_a_thousand_request_atoms_submits_no_sql() {
        unsafe extern "C" fn trace(_: u32, state: *mut std::ffi::c_void, _: *mut std::ffi::c_void, _: *mut std::ffi::c_void) -> i32 {
            // SAFETY: the counter outlives the callback, removed before it drops.
            unsafe { &*state.cast::<AtomicUsize>() }.fetch_add(1, Ordering::Relaxed);
            0
        }
        let db = Connection::open_in_memory().unwrap();
        register_symbols(&db).unwrap();
        let expected: Vec<_> = (0..1000).map(|i| format!("request-{i}-α")).collect();
        let ids = intern_batch(&db, expected.iter().map(String::as_str)).unwrap();
        let count = AtomicUsize::new(0);
        // SAFETY: db and count stay alive until this trace is removed below.
        unsafe { ffi::sqlite3_trace_v2(db.handle(), ffi::SQLITE_TRACE_STMT, Some(trace), std::ptr::from_ref(&count).cast_mut().cast()); }
        let decoded = text_batch(&db, &ids);
        unsafe { ffi::sqlite3_trace_v2(db.handle(), 0, None, std::ptr::null_mut()); }
        assert_eq!(decoded.unwrap(), expected);
        assert_eq!(count.load(Ordering::Relaxed), 0);
        assert!(text_batch(&db, &[-1]).is_err());
        assert!(text_batch(&db, &[i64::MAX]).is_err());
    }
}

/// `sym_text(a)`, `sym_atom(text)` and `sym_cat(a, b)` (`a.b`) over `conn`'s symbols: the planner's SQL holds symbols
/// as atoms (decision 0009), and these are the only places it reads or makes their text. NULL gives NULL. Also
/// registers `atoms` and the store's commit hook on `conn` (decision 0049).
pub fn register_symbols(conn: &Connection) -> Result<()> {
    use rusqlite::functions::FunctionFlags;
    crate::content_store::atoms::Atoms::install(conn)?;
    let det = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC;
    let syms = symbols(conn)?;
    let s = Arc::clone(&syms);
    conn.create_scalar_function(c"sym_text", 1, det, move |ctx| {
        let Some(a) = ctx.get::<Option<i64>>(0)? else { return Ok(None) };
        let s = s.read().unwrap();
        if a < 0 || a as usize >= s.len() {
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
        if a.min(b) < 0 || a.max(b) as usize >= s.len() {
            return Err(err(format!("no symbol {}", if a < 0 || a as usize >= s.len() { a } else { b })));
        }
        let id = cat(&mut s, a, b);
        s.check()?;
        Ok(Some(id))
    })?;
    Ok(())
}

/// Appends the symbols pending in `conn`'s database if a flush is due (decision 0049 §4.2): a run's end.
pub fn flush_symbols(conn: &Connection) -> Result<()> {
    crate::content_store::atoms::Atoms::for_connection(conn).flush_if_due(conn).map_err(|e| err(e.to_string()))
}

enum Pat { Any, Slot(usize), Int(i64), Sym(i64) }
enum Term { Slot(usize), Const(i64), Cat(usize, usize) }

fn hash_vals(vals: impl Iterator<Item = i64>) -> u64 {
    let mut h = FxHasher::default();
    for v in vals { h.write_i64(v); }
    h.finish()
}

const NIL: u32 = u32::MAX;

/// Whether row `id` is dead in bitset `dead` (a relation started from a kept one, its deleted rows marked, not
/// removed: decision 0090 step 4). Empty: none is.
#[inline]
fn is_dead(dead: &[u64], id: u32) -> bool {
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
/// A key's postings in an `Index`: its newest row, its row count, and its count among the rows added in wave `wave`
/// (the relation's current delta when `wave` is the relation's).
#[derive(Clone, Copy)]
struct Post { head: u32, len: u32, dlen: u32, wave: u32 }
/// A hash index of a relation on a key-column set. A key's rows are chained newest first through `next` (one slot
/// per row), so a posting costs no allocation, and since a wave's new rows are the newest, a key's delta rows are a
/// prefix of its chain.
#[derive(Clone)]
struct Index { cols: Vec<usize>, table: HashTable<Post>, next: Vec<u32> }
impl Index {
    fn add(&mut self, data: &[i64], k: usize, id: u32, wave: u32) {
        let row = &data[id as usize * k..][..k];
        let cols = &self.cols;
        let h = hash_vals(cols.iter().map(|&c| row[c]));
        let same = |p: &Post| cols.iter().all(|&c| data[p.head as usize * k + c] == row[c]);
        match self.table.find_mut(h, same) {
            Some(p) => {
                self.next.push(p.head);
                p.head = id;
                p.len += 1;
                if p.wave == wave { p.dlen += 1 } else { p.wave = wave; p.dlen = 1 }
            }
            None => {
                self.next.push(NIL);
                self.table.insert_unique(h, Post { head: id, len: 1, dlen: 1, wave }, |p| hash_vals(cols.iter().map(|&c| data[p.head as usize * k + c])));
            }
        }
    }
    /// `key` holds the values of `cols`, in order.
    fn get(&self, data: &[i64], k: usize, key: &[i64]) -> Option<Post> {
        let h = hash_vals(key.iter().copied());
        self.table.find(h, |p| self.cols.iter().zip(key).all(|(&c, &v)| data[p.head as usize * k + c] == v)).copied()
    }
}

/// A relation's tuples in insertion order, flat (`k` columns each); `lo..hi` is the previous wave's delta, the rows
/// added in wave `wave`. `set` deduplicates by row id. `idx`: a hash index per column (the first `k`), then composite
/// indexes, one per key-column set a join probe reads the relation with, so a probe bound on several columns reads
/// exactly its matches rather than the shortest single-column list.
/// `dead`: rows deleted but kept in place (`based`), which every reader skips; `ndead` of them.
struct Rel { table: String, types: Vec<u8>, k: usize, n: usize, data: Vec<i64>, set: HashTable<u32>, stored: usize, lo: usize, hi: usize,
             wave: u32, idx: Vec<Index>, dead: Vec<u64>, ndead: usize, summaries: scalars::Pool }
impl Rel {
    fn new(table: &str, types: &[u8]) -> Rel {
        let k = types.len();
        let idx = (0..k).map(|c| Index { cols: vec![c], table: HashTable::new(), next: Vec::new() }).collect();
        Rel { table: table.to_string(), types: types.to_vec(), k, n: 0, data: Vec::new(), set: HashTable::new(), stored: 0, lo: 0, hi: 0, wave: 0, idx, dead: Vec::new(), ndead: 0, summaries: Default::default() }
    }
    /// A relation that starts as `old` (a kept one: its rows, dedup set and indexes, copied as memory, not hashed
    /// again) less the rows of `gone`, marked dead in place (decision 0090 step 4). Its rows are the first wave's
    /// delta, so each rule's whole plan reads them once. `None` when `old` has no set to start from, or when too
    /// many of its rows would be dead (then it is made from its rows, compacted).
    fn based(table: &str, types: &[u8], old: &InRel, gone: &InRel) -> Option<Rel> {
        if old.view.is_some() || old.k != types.len() || gone.k != old.k || (old.n > 0 && old.set.is_empty()) {
            return None;
        }
        let k = old.k;
        let (data, set) = (&old.rows, &old.set);
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
            let found = set.find(hash_vals(row.iter().copied()), |&id| !is_dead(&dead, id) && &data[id as usize * k..][..k] == &row[..]);
            if let Some(&id) = found && kill(&mut dead, id) {
                ndead += 1;
            }
        }
        if ndead * 8 > old.n.max(1) {
            return None;
        }
        let mut idx: Vec<Index> = old.chains.clone();
        for c in 0..k {
            if !idx.iter().any(|ix| ix.cols == [c]) {
                return None;
            }
        }
        // Every row is the first wave's (`open` makes it the delta): its postings are stamped so.
        for ix in &mut idx {
            for p in ix.table.iter_mut() {
                p.wave = 1;
                p.dlen = p.len;
            }
        }
        Some(Rel { table: table.to_string(), types: types.to_vec(), k, n: old.n, data: data.clone(), set: set.clone(), stored: 0, lo: 0, hi: 0, wave: 0, idx, dead, ndead, summaries: Default::default() })
    }
    #[inline]
    fn dead(&self, id: u32) -> bool {
        is_dead(&self.dead, id)
    }
    fn row(&self, id: usize) -> &[i64] { &self.data[id * self.k..][..self.k] }
    fn contains(&self, row: &[i64]) -> bool {
        let (data, k, dead) = (&self.data, self.k, &self.dead);
        self.set.find(hash_vals(row.iter().copied()), |&id| !is_dead(dead, id) && &data[id as usize * k..][..k] == row).is_some()
    }
    fn add_multi(&mut self, cols: &[usize]) {
        if self.idx.iter().any(|ix| ix.cols == cols) { return; }
        let mut ix = Index { cols: cols.to_vec(), table: HashTable::new(), next: Vec::with_capacity(self.n) };
        charge(Unit::IndexEntry, self.n as u64);
        for id in 0..self.n { ix.add(&self.data, self.k, id as u32, if id >= self.lo { self.wave } else { 0 }); }
        self.idx.push(ix);
    }
    /// Adds a row (of the next wave, its delta) unless it is there.
    fn push(&mut self, row: &[i64]) -> bool {
        let h = hash_vals(row.iter().copied());
        let k = self.k;
        // A dead row derived again is a new row (its old id stays dead).
        if self.set.find(h, |&id| !is_dead(&self.dead, id) && &self.data[id as usize * k..][..k] == row).is_some() { return false; }
        let id = self.n as u32;
        self.data.extend_from_slice(row);
        self.n += 1;
        let data = &self.data;
        self.set.insert_unique(h, id, |&i| hash_vals(data[i as usize * k..][..k].iter().copied()));
        for ix in &mut self.idx { ix.add(data, k, id, self.wave + 1); }
        true
    }
    fn set_delta(&mut self, lo: usize, hi: usize) {
        self.lo = lo;
        self.hi = hi;
        self.wave += 1;
    }
    /// The rows of index `ix` with key `key`, in the delta only or in all: (candidates, count).
    fn postings(&self, ix: usize, key: &[i64], delta: bool) -> (Cands<'_>, usize) {
        let index = &self.idx[ix];
        match index.get(&self.data, self.k, key) {
            None => (Cands::Slice(&[]), 0),
            Some(p) if delta => if p.wave == self.wave { (Cands::Chain { head: p.head, next: &index.next, stop: self.lo as u32 }, p.dlen as usize) }
                                else { (Cands::Slice(&[]), 0) },
            Some(p) => (Cands::Chain { head: p.head, next: &index.next, stop: 0 }, p.len as usize),
        }
    }
}

/// Candidate row ids: a range, a list, or a posting chain (newest first, down to `stop`).
#[derive(Clone)]
enum Cands<'a> { Range(u32, u32), Slice(&'a [u32]), Owned(Vec<u32>), Chain { head: u32, next: &'a [u32], stop: u32 } }
impl Cands<'_> {
    /// Calls `f` on each id until it returns true; returns whether one did.
    fn any(self, mut f: impl FnMut(u32) -> bool) -> bool {
        match self {
            Cands::Range(lo, hi) => (lo..hi).any(f),
            Cands::Slice(ids) => ids.iter().any(|&id| f(id)),
            Cands::Owned(ids) => ids.into_iter().any(f),
            Cands::Chain { head, next, stop } => {
                let mut id = head;
                while id != NIL && id >= stop {
                    if f(id) { return true; }
                    id = next[id as usize];
                }
                false
            }
        }
    }
}

/// New tuples of one relation found in a wave, flat; duplicates are dropped when they are pushed.
#[derive(Clone, Default)]
struct Fresh { data: Vec<i64>, n: usize }
impl Fresh {
    fn add(&mut self, row: &[i64]) { self.data.extend_from_slice(row); self.n += 1; }
}
struct EdgeRule { head: usize, src: usize, key: Vec<usize>, ecols: Vec<usize>, pat: Vec<Pat>, out: Vec<Term>,
                  edges: HashMap<Vec<i64>, Vec<Vec<i64>>>, has_edge: bool, nslots: usize }
/// A probe's body: SQL over the `set_rel` vtabs, or a native join pipeline (`join.rs` spec).
enum Body { Sql(String), Join(JoinRule) }
/// `whole`: a join probe's plan for the wave whose delta is its whole relation (the relation was empty before it), in
/// any literal order, reading that relation in full. `wkey`: its spec, the same for each probe of one rule: in a wave
/// the store does not change, so a whole plan run once derives what each of its rule's probes would (it is not run
/// again).
struct ProbeRule { head: usize, delta: usize, body: Body, whole: Option<JoinRule>, wkey: Option<String> }

pub struct Store { rels: Vec<Rel>, syms: Arc<RwLock<Interner>> }
/// `seeded`: the seed joins have run (in the first wave). `write`: at the fixpoint, insert the new rows into the
/// relations' tables (else they stay native, in `DERIVED`, until `set_fixpoint_materialize`).
/// `requests`: relations whose new rows the caller answers (`pending`, `add`), each with the rows answered so far.
/// `produced`: relations only the caller adds to, which the fixpoint does not write.
struct Component { store: Arc<RwLock<Store>>, edges: Vec<EdgeRule>, probes: Vec<ProbeRule>, gates: Vec<Gate>, seeded: bool, write: bool, done: bool,
                   requests: Vec<(usize, usize)>, produced: Vec<usize> }
/// A gated negation's `done` relation, fed from `src` (its demand) once `quiet` is: `passed` src rows so far.
struct Gate { done: usize, src: Option<usize>, quiet: Vec<usize>, after: Vec<usize>, passed: usize }

/// The connection a component or a derived relation belongs to (its handle): two runs in one process (two corpora,
/// or tests in parallel) name their components and relations alike, and must not see each other's.
///
/// A handle is an address, and a closed connection's address is given to the next one opened: what a connection
/// left here is dropped when it closes (`sqlite3_set_clientdata`'s destructor runs in `sqlite3_close`), so a new
/// connection at the same address starts with none of it. Without this a second run over the same store read the
/// first run's fixpoint as its own (a cone's loaded documents counted as the workspace's).
pub(crate) fn scope(conn: &Connection) -> usize {
    crate::code_rule_dsl::facts::vtab::connection_scope(conn)
}

/// Drops what the connection at `at` left: its components and derived relations (`scope`).
pub(crate) fn forget_scope(at: usize) {
    if let Ok(mut c) = COMPONENTS.lock() {
        c.retain(|((s, _), _)| *s != at);
    }
    if let Ok(mut d) = DERIVED.lock() {
        d.retain(|(s, _), _| *s != at);
    }
    if let Ok(mut a) = ADOPTED.lock() {
        a.retain(|(s, _)| *s != at);
    }
}
static COMPONENTS: Mutex<Vec<((usize, String), Arc<Mutex<Component>>)>> = Mutex::new(Vec::new());
fn component(conn: &Connection, name: &str) -> Result<Arc<Mutex<Component>>> {
    component_in(scope(conn), name)
}
fn component_in(scope: usize, name: &str) -> Result<Arc<Mutex<Component>>> {
    COMPONENTS.lock().unwrap().iter().find(|((s, n), _)| *s == scope && n == name).map(|c| c.1.clone()).ok_or_else(|| err(format!("no set fixpoint named {name:?}")))
}

fn unhex(s: &str) -> Result<String> {
    let b: Result<Vec<u8>> = (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| err(e.to_string()))).collect();
    String::from_utf8(b?).map_err(|e| err(e.to_string()))
}

fn read_value(v: ValueRef<'_>, ty: u8, syms: &mut Interner) -> Result<i64> {
    match (v, ty) {
        (ValueRef::Integer(i), b'i') => Ok(i),
        (ValueRef::Text(t), b's') => Ok(syms.id(std::str::from_utf8(t).map_err(|e| err(e.to_string()))?)),
        (ValueRef::Integer(i), b's') => { let _ = syms; Ok(i) }
        (other, _) => Err(err(format!("unexpected value {other:?} for a column of type {}", ty as char))),
    }
}

use crate::code_rule_dsl::logic::profile::{Unit, work as charge};

fn read_rows(conn: &Connection, table: &str, types: &[u8], syms: &mut Interner) -> Result<Vec<Vec<i64>>> {
    let cols: Vec<String> = (0..types.len()).map(|i| format!("c{i}")).collect();
    read_sql(conn, &format!("SELECT {} FROM {}", cols.join(", "), crate::code_rule_dsl::logic::sql::table(table)), types, syms).map_err(|e| err(format!("{table}: {e}")))
}

fn read_sql(conn: &Connection, sql: &str, types: &[u8], syms: &mut Interner) -> Result<Vec<Vec<i64>>> {
    let table = sql;
    let mut st = conn.prepare(sql)?;
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

fn slot(t: &str) -> Result<usize> { t.strip_prefix('v').and_then(|s| s.parse().ok()).ok_or_else(|| err(format!("bad slot {t}"))) }
fn num<T: std::str::FromStr>(t: &str) -> Result<T> { t.parse().map_err(|_| err(format!("bad number {t}"))) }

pub fn open(conn: &Connection, spec: &str) -> Result<String> {
    let symbols = symbols(conn)?;
    let mut syms = Interner::for_loading(&symbols);
    let (mut name, mut rels, mut edges, mut probes, mut gates) = (String::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut write = true;
    let (mut requests, mut produced) = (Vec::new(), Vec::new());
    for line in spec.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let toks: Vec<&str> = line.split_whitespace().collect();
        match toks[0] {
            "name" => name = toks[1].to_string(),
            "rel" => {
                let mut rel = Rel::new(toks[1], toks[2].as_bytes());
                let mut empty = false;
                let mut from = None;
                let mut based: Option<(String, String)> = None;
                let mut kept_base = false;
                let mut seeded = false;
                let mut t = 3;
                while t < toks.len() {
                    match toks[t] {
                        "empty" => { empty = true; t += 1; }
                        // With `base`: rows a SQL seed or the program's facts wrote to the table, added to it.
                        "seeded" => { seeded = true; t += 1; }
                        "base" => {
                            let (old, gone) = (toks.get(t + 1).ok_or_else(|| err("rel: base needs two tables"))?, toks.get(t + 2).ok_or_else(|| err("rel: base needs two tables"))?);
                            based = Some((unhex(old)?, unhex(gone)?));
                            kept_base = true;
                            empty = true;
                            t += 3;
                        }
                        "from" => { from = Some(unhex(toks.get(t + 1).ok_or_else(|| err("rel: from needs SQL"))?)?); t += 2; }
                        other => return Err(err(format!("rel: unknown token {other}"))),
                    }
                }
                // `empty`: the table was made this run and nothing wrote it (no statement). `from`: its rows are a
                // query's (an input's, read where it is).
                // `base`: the relation starts as a kept native relation less another's rows (the survivors of a
                // maintained stratum, `h__old` less `h__del`), dead in place; else, made from those rows.
                if let Some((old, gone)) = based {
                    let (o, g) = { let d = DERIVED.lock().unwrap(); (d.get(&(scope(conn), old.clone())).cloned(), d.get(&(scope(conn), gone.clone())).cloned()) };
                    let (o, g) = (o.ok_or_else(|| err(format!("rel: no native relation {old}")))?, g.ok_or_else(|| err(format!("rel: no native relation {gone}")))?);
                    match Rel::based(toks[1], toks[2].as_bytes(), &o, &g) {
                        Some(b) => rel = b,
                        None => {
                            let k = rel.k;
                            let mut gone_rows: rustc_hash::FxHashSet<Vec<i64>> = Default::default();
                            for id in 0..g.len() { if !is_dead(&g.dead, id as u32) { let r = g.at(id); gone_rows.insert((0..k).map(|c| r.at(c)).collect()); } }
                            let mut row = vec![0i64; k];
                            for id in 0..o.len() {
                                if is_dead(&o.dead, id as u32) { continue; }
                                let r = o.at(id);
                                for (c, v) in row.iter_mut().enumerate() { *v = r.at(c); }
                                if !gone_rows.contains(&row) { rel.push(&row); }
                            }
                        }
                    }
                    if seeded {
                        for row in read_rows(conn, &rel.table, &rel.types.clone(), &mut syms)? { rel.push(&row); }
                    }
                } else if let Some(sql) = from {
                    for row in read_sql(conn, &sql, &rel.types.clone(), &mut syms)? { rel.push(&row); }
                } else if !empty {
                    for row in read_rows(conn, &rel.table, &rel.types.clone(), &mut syms)? { rel.push(&row); }
                }
                // A relation started from a kept one has none of its rows in its table (made this run).
                rel.stored = if kept_base { 0 } else { rel.n };
                rels.push(rel);
            }
            "probe" => probes.push(ProbeRule { head: num(toks[1])?, delta: num(toks[2])?, body: Body::Sql(unhex(toks[3])?), whole: None, wkey: None }),
            "seed" => probes.push(ProbeRule { head: num(toks[1])?, delta: SEED, body: Body::Join(parse_join(conn, &unhex(toks[2])?, &mut syms)?), whole: None, wkey: None }),
            "nowrite" => write = false,
            "request" => requests.push((num(toks[1])?, 0)),
            "produced" => produced.push(num(toks[1])?),
            "gate" => {
                let list = |t: &str| -> Result<Vec<usize>> { if t == "-" { Ok(Vec::new()) } else { t.split(',').map(num).collect() } };
                let src = if toks[2] == "-" { None } else { Some(num(toks[2])?) };
                gates.push(Gate { done: num(toks[1])?, src, quiet: list(toks[3])?, after: list(toks[4])?, passed: 0 });
            }
            "join" => {
                let (head, delta) = (num(toks[1])?, num(toks[2])?);
                let whole = match toks.get(4) { Some(w) => Some(parse_join(conn, &unhex(w)?, &mut syms)?), None => None };
                let wkey = toks.get(4).map(|w| w.to_string());
                probes.push(ProbeRule { head, delta, body: Body::Join(parse_join(conn, &unhex(toks[3])?, &mut syms)?), whole, wkey });
            }
            "rule" => {
                let (head, src) = (num(toks[1])?, num(toks[2])?);
                let sections: Vec<&[&str]> = toks[5..].split(|t| *t == ";").collect();
                let section = |name: &str| -> Result<&[&str]> {
                    sections.iter().find(|s| s.first() == Some(&name)).map(|s| &s[1..]).ok_or_else(|| err(format!("no {name}")))
                };
                let key: Vec<usize> = section("key")?.iter().map(|t| slot(t)).collect::<Result<_>>()?;
                let ecols: Vec<usize> = section("ecols")?.iter().map(|t| slot(t)).collect::<Result<_>>()?;
                let mut pat = Vec::new();
                for t in section("pat")? {
                    pat.push(if *t == "_" { Pat::Any } else if let Some(i) = t.strip_prefix("i:") { Pat::Int(num(i)?) }
                             else if let Some(x) = t.strip_prefix("x:") { Pat::Sym(syms.id(&unhex(x)?)) } else { Pat::Slot(slot(t)?) });
                }
                let mut out = Vec::new();
                for t in section("head")? {
                    out.push(if let Some(i) = t.strip_prefix("i:") { Term::Const(num(i)?) }
                             else if let Some(x) = t.strip_prefix("x:") { Term::Const(syms.id(&unhex(x)?)) }
                             else if let Some(c) = t.strip_prefix("cat:") {
                                 let (a, b) = c.split_once(':').ok_or_else(|| err("bad cat"))?;
                                 Term::Cat(slot(a)?, slot(b)?)
                             } else { Term::Slot(slot(t)?) });
                }
                let nslots = key.iter().chain(&ecols).copied().chain(pat.iter().filter_map(|p| if let Pat::Slot(s) = p { Some(*s) } else { None }))
                    .max().map_or(0, |m| m + 1);
                let mut map: HashMap<Vec<i64>, Vec<Vec<i64>>> = HashMap::default();
                let has_edge = toks[3] != "-";
                if has_edge {
                    for row in read_rows(conn, toks[3], toks[4].as_bytes(), &mut syms)? {
                        let (k, e) = row.split_at(key.len());
                        map.entry(k.to_vec()).or_default().push(e.to_vec());
                    }
                }
                edges.push(EdgeRule { head, src, key, ecols, pat, out, edges: map, has_edge, nslots });
            }
            other => return Err(err(format!("unknown spec item {other}"))),
        }
    }
    for r in &mut rels { let n = r.n; r.set_delta(0, n); }   // the first wave reads the seeds as its delta
    for p in &probes {
        if let Body::Join(j) = &p.body { for (r, cols) in &j.multi { rels[*r].add_multi(cols); } }
        if let Some(j) = &p.whole { for (r, cols) in &j.multi { rels[*r].add_multi(cols); } }
    }
    syms.check()?;
    drop(syms);
    let store = Arc::new(RwLock::new(Store { rels, syms: symbols }));
    let mut reg = COMPONENTS.lock().unwrap();
    let key = (scope(conn), name.clone());
    reg.retain(|(k, _)| *k != key);
    reg.push((key, Arc::new(Mutex::new(Component { store, edges, probes, gates, seeded: false, write, done: false, requests, produced }))));
    Ok(name)
}

/// At the fixpoint the component's relations move to `DERIVED`, the input of later strata (`r:` sources), with the
/// hash indexes the waves kept (`InRel::chains`): nothing is copied or indexed again.
pub fn close(conn: &Connection, name: &str) -> Result<()> {
    let at_scope = scope(conn);
    let mut reg = COMPONENTS.lock().unwrap();
    let Some(at) = reg.iter().position(|((s, n), _)| *s == at_scope && n == name) else { return Ok(()) };
    let (_, comp) = reg.remove(at);
    drop(reg);
    let comp = comp.lock().unwrap();
    if comp.done {
        let rels = std::mem::take(&mut comp.store.write().unwrap().rels);
        let mut derived = DERIVED.lock().unwrap();
        for rel in rels {
            derived.insert((at_scope, rel.table.clone()), Arc::new(InRel::moved(rel)));
        }
    }
    Ok(())
}

/// Runs waves until the fixpoint, or until `max_waves` more have run (0: no bound). Rows are written to the
/// relations' tables only at the fixpoint, so a bounded run can be resumed after the caller replans probes from
/// `stats`.
pub fn run(conn: &Connection, name: &str, max_waves: u32) -> Result<String> {
    let t0 = Instant::now();
    let comp = component(conn, name)?;
    let mut comp = comp.lock().unwrap();
    let (nrels, symbols) = { let s = comp.store.read().unwrap(); (s.rels.len(), Arc::clone(&s.syms)) };
    {
        let store=comp.store.read().unwrap();
        for probe in &comp.probes {
            if let Body::Join(join)=&probe.body {scalars::register(&join.steps,Some(&store));}
            if let Some(join)=&probe.whole {scalars::register(&join.steps,Some(&store));}
        }
    }
    let mut stmts = Vec::new();
    for p in &comp.probes { stmts.push(match &p.body { Body::Sql(sql) => Some(conn.prepare(sql)?), Body::Join(_) => None }); }
    let (mut waves, mut visits, mut runs, mut skips) = (0u32, 0u64, 0u64, 0u64);
    let mut done = false;
    let mut paused = false;
    let mut seeded = comp.seeded;
    // Per probe: seconds spent in its statement and rows it returned.
    let mut per: Vec<(f64, u64)> = vec![(0.0, 0); comp.probes.len()];
    let mut row: Vec<i64> = Vec::new();
    let wave_trace = std::env::var_os("RDL_WAVE_TRACE").is_some();
    let mut gates = std::mem::take(&mut comp.gates);
    let gates_done: Vec<usize> = gates.iter().map(|g| g.done).collect();
    loop {
        let mut fresh: Vec<Fresh> = vec![Fresh::default(); nrels];
        // Probe rules: SQL over the vtabs, while no lock is held (the vtab cursors take read locks).
        let mut raw: Vec<(usize, Vec<Vec<Value>>)> = Vec::new();
        for (pi, (p, st)) in comp.probes.iter().zip(stmts.iter_mut()).enumerate() {
            let Some(st) = st else { continue };
            let tp = Instant::now();
            let empty = { let s = comp.store.read().unwrap(); s.rels[p.delta].lo == s.rels[p.delta].hi };
            if empty { skips += 1; continue; }
            runs += 1;
            let k = comp.store.read().unwrap().rels[p.head].types.len();
            let mut rows = st.query([])?;
            let mut got = Vec::new();
            while let Some(r) = rows.next()? { got.push((0..k).map(|i| r.get::<_, Value>(i)).collect::<Result<Vec<_>>>()?); }
            per[pi].0 += tp.elapsed().as_secs_f64();
            per[pi].1 += got.len() as u64;
            raw.push((p.head, got));
        }
        let mut guard = comp.store.write().unwrap();
        let store = &mut *guard;
        let mut syms = symbols.write().unwrap();
        for (head, got) in raw {
            let types = store.rels[head].types.clone();
            for vals in got {
                visits += 1;
                row.clear();
                for (v, &ty) in vals.iter().zip(&types) {
                    row.push(read_value(ValueRef::from(v), ty, &mut syms).map_err(|e| err(format!("{}: {e}", store.rels[head].table)))?);
                }
                if !store.rels[head].contains(&row) { fresh[head].add(&row); }
            }
        }
        // Join probes, natively; seed joins in the first wave only.
        let mut wholes_run: Vec<(usize, &str)> = Vec::new();
        for (pi, p) in comp.probes.iter().enumerate() {
            let Body::Join(j) = &p.body else { continue };
            let tp = Instant::now();
            if p.delta == SEED { if seeded { continue } }
            else if store.rels[p.delta].lo == store.rels[p.delta].hi { skips += 1; continue; }
            let j = match (&p.whole, &p.wkey) {
                (Some(w), Some(k)) if p.delta != SEED && store.rels[p.delta].lo == 0 => {
                    // Equal bodies may feed different relations. Sharing the
                    // computation must never discard a destination's rows.
                    let destination = (p.head, k.as_str());
                    if wholes_run.contains(&destination) { skips += 1; continue; }
                    wholes_run.push(destination);
                    w
                }
                _ => j,
            };
            runs += 1;
            if wave_trace {
                let d = if p.delta == SEED { 0 } else { store.rels[p.delta].hi - store.rels[p.delta].lo };
                eprintln!("      wave {waves} probe {pi} of {name} starts, head {}, delta {d}", store.rels[p.head].table);
            }
            let n = run_join(j, p.head, store, &mut syms, &mut fresh)?;
            visits += n;
            per[pi].0 += tp.elapsed().as_secs_f64();
            per[pi].1 += n;
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
                    if !store.rels[rule.head].contains(&row) { fresh[rule.head].add(&row); }
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
        for (r, f) in fresh.into_iter().enumerate() {
            let rel = &mut store.rels[r];
            let (lo, k) = (rel.n, rel.k);
            for i in 0..f.n { any |= rel.push(&f.data[i * k..][..k]); }
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
        if !any { done = true; break; }
        if max_waves > 0 && waves >= max_waves { break; }
    }
    comp.seeded = seeded;
    comp.gates = gates;
    // A symbol a wave could not intern fails the run.
    symbols.write().unwrap().check()?;
    let solve = t0.elapsed();
    let per: Vec<String> = per.iter().map(|(t, n)| format!("{:.1}:{n}", t * 1e3)).collect();
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
                conn.execute(&format!("INSERT INTO {} VALUES {holes}", crate::code_rule_dsl::logic::sql::table(&rel.table)), rusqlite::params_from_iter(rel.data[id * rel.k..end * rel.k].iter()))?;
                new_rows += end - id;
                id = end;
            }
        }
    }
    Ok(format!("{new_rows} {waves} {visits} {runs} {skips} {:.1} {:.1} {} 1", solve.as_secs_f64() * 1e3, t0.elapsed().as_secs_f64() * 1e3, per.join(",")))
}

/// The request rows not yet answered, per request relation (its table), now counted as answered.
pub fn pending(conn: &Connection, name: &str) -> Result<Vec<(String, Vec<Vec<i64>>)>> {
    let comp = component(conn, name)?;
    let mut comp = comp.lock().unwrap();
    let store = Arc::clone(&comp.store);
    let store = store.read().unwrap();
    let mut out = Vec::new();
    for (r, answered) in &mut comp.requests {
        let rel = &store.rels[*r];
        if rel.n > *answered {
            out.push((rel.table.clone(), (*answered..rel.n).filter(|&i| !rel.dead(i as u32)).map(|i| rel.row(i).to_vec()).collect()));
            *answered = rel.n;
        }
    }
    Ok(out)
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
        let cols = &r.idx[..r.k];
        let mut line = std::iter::once(r.n).chain(cols.iter().map(|ix| ix.table.len()))
            .chain(cols.iter().map(|ix| ix.table.iter().map(|p| p.len as usize * p.len as usize).sum::<usize>()))
            .map(|n| n.to_string()).collect::<Vec<_>>().join(" ");
        // Each value's postings in a column of at most FREQ_NDV values (a kind tag), as `|col:hexvalue=n,..`: a
        // constant there is priced exactly, as stat_val prices an input's.
        for (c, ix) in cols.iter().enumerate() {
            if ix.table.len() > FREQ_NDV { continue; }
            let vals: Vec<String> = ix.table.iter().map(|p| {
                let v = r.row(p.head as usize)[c];
                let text = if r.types[c] == b's' { syms.name(v) } else { v.to_string() };
                format!("{}={}", text.bytes().map(|b| format!("{b:02x}")).collect::<String>(), p.len)
            }).collect();
            line.push_str(&format!(" |{c}:{}", vals.join(",")));
        }
        line
    }).collect::<Vec<_>>().join("\n"))
}

/// `set_fixpoint_steps(name)`: per probe line (spec order), its join steps' "in:out" counts since the last call,
/// comma-separated (empty for a SQL probe); the counts restart.
/// Per probe (spec order), its join steps' (in, out) binding counts since the last call (empty for a SQL probe); the
/// counts restart.
pub fn step_counts(conn: &Connection, name: &str) -> Result<Vec<Vec<(i64, i64)>>> {
    let comp = component(conn, name)?;
    let comp = comp.lock().unwrap();
    Ok(comp.probes.iter().map(|p| match &p.body {
        Body::Join(j) => j.counts.iter().map(|[a, b]| (a.replace(0) as i64, b.replace(0) as i64)).collect(),
        Body::Sql(_) => Vec::new(),
    }).collect())
}

/// A relation's live statistics (`stats`): rows, then per column its distinct values and sum of squared postings,
/// and for a column of at most `FREQ_NDV` values each value's postings.
pub struct Live { pub n: i64, pub ndv: Vec<i64>, pub sq: Vec<i64>, pub freq: Vec<(usize, Vec<(i64, i64)>)> }
pub fn live(conn: &Connection, name: &str) -> Result<Vec<Live>> {
    let comp = component(conn, name)?;
    let comp = comp.lock().unwrap();
    let store = comp.store.read().unwrap();
    Ok(store.rels.iter().map(|r| {
        let cols = &r.idx[..r.k];
        let freq = cols.iter().enumerate().filter(|(_, ix)| ix.table.len() <= FREQ_NDV)
            .map(|(c, ix)| (c, ix.table.iter().map(|p| (r.row(p.head as usize)[c], i64::from(p.len))).collect())).collect();
        Live {
            n: r.n as i64,
            ndv: cols.iter().map(|ix| ix.table.len() as i64).collect(),
            sq: cols.iter().map(|ix| ix.table.iter().map(|p| i64::from(p.len) * i64::from(p.len)).sum()).collect(),
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
    let comp = component(conn, name)?;
    let mut comp = comp.lock().unwrap();
    let n = comp.probes.len();
    let body = match kind {
        "sql" => Body::Sql(unhex(body)?),
        "join" => {
            let symbols = Arc::clone(&comp.store.read().unwrap().syms);
            let mut syms = Interner::for_loading(&symbols);
            let j = parse_join(conn, &unhex(body)?, &mut syms)?;
            syms.check()?;
            let mut store = comp.store.write().unwrap();
            for (r, cols) in &j.multi { store.rels[*r].add_multi(cols); }
            Body::Join(j)
        }
        // The probe's whole plan (`ProbeRule::whole`), planned again from live statistics.
        "whole" => {
            let symbols = Arc::clone(&comp.store.read().unwrap().syms);
            let mut syms = Interner::for_loading(&symbols);
            let j = parse_join(conn, &unhex(body)?, &mut syms)?;
            syms.check()?;
            let mut store = comp.store.write().unwrap();
            for (r, cols) in &j.multi { store.rels[*r].add_multi(cols); }
            drop(store);
            let p = comp.probes.get_mut(i).ok_or_else(|| err(format!("no probe {i} of {n}")))?;
            p.whole = Some(j);
            p.wkey = Some(body.to_owned());
            return Ok(());
        }
        other => return Err(err(format!("unknown probe kind {other}"))),
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
        let Some((cands, _)) = keys.iter().map(|&(c, k)| r.postings(c, &[k], self.delta)).min_by_key(|&(_, n)| n) else { return Ok(()) };
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
        ctx.set_result(&v)
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
//   cat <tok> <tok> <tok>  first = second "." third, binding the first or testing it; with the first bound and a part
//                          unbound, splitting the first at each "." (binding the unbound parts, testing a bound one)
//   eq <tok> <tok>, ne <tok> <tok> equal (binding the first if unbound) or not
//   lt <tok> <tok>, le <tok> <tok> integer order, both bound
//   count <src> <tok..> <tok>      the atom's matching tuples (its variables bound), binding or testing the last
//   head <tok..>           the head's terms (`cat:v<a>:v<b>` too)
// Tokens are `_`, `v<slot>`, `i:<int>`, `x:<hex symbol>`. Which columns of an atom are keys (bound) or bind is fixed
// here, in literal order, so each step looks up one index and never re-derives the binding pattern.
// An input relation is read once per process (`INPUTS`) with a hash index per key-column set, built on first use.
// T's `near`/`down` steps (ancestor climbs over a loaded tape) are not kept: tree axes run in the transducer.

/// `@dot(a, b)` (`core.rr`): `a.b`, and `b` when `a` is "".
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
#[cfg(not(target_family = "wasm"))]
fn thread_cpu() -> f64 {
    let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
    t.tv_sec as f64 + t.tv_nsec as f64 / 1e9
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
pub struct InRel { k: usize, n: usize, rows: Vec<i64>, view: Option<View>, base: bool, types: Vec<u8>, stored: usize, idx: Mutex<HashMap<u64, Arc<Csr>>>, chains: Vec<Index>,
                   set: HashTable<u32>, dead: Vec<u64>, ndead: usize, summaries: scalars::Pool,
                   /// Each column's statistics (`syntax::measured`), measured when first asked: a kept relation's rows
                   /// do not change, so a session's later runs read them, not measure them again (0094 §4).
                   measured: std::sync::OnceLock<Vec<[i64; 4]>> }
/// A base input read in place: the bound vtab source's columns `cols`, at its rows `ids` (all rows when `None`, the
/// rows passing the atom's constant filters otherwise). Nothing of the source is copied.
pub struct View { src: Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, cols: Vec<usize>, ids: Option<Vec<u32>>, eqs: Vec<(usize, i64)> }
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
    fn at(self, c: usize) -> i64 {
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
    fn read(self) -> RowRef<'a> {
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
pub struct Csr { cols: Vec<usize>, table: HashTable<(u32, u32, u32)>, ids: Vec<u32>, source: bool, sorted: bool, chain: Option<usize>, lead: usize }
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
        Csr { cols: cols.to_vec(), table, ids, source: false, sorted: false, chain: None, lead: 0 }
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
    let at = scope(conn);
    let mut derived = DERIVED.lock().unwrap();
    for t in tables {
        derived.remove(&(at, t.clone()));
    }
}

/// Drops the indexes of native relations `tables` that stay to the run's end (outputs, kept relations) but that no
/// later stratum reads: their rows are read once more, in order, and their per-column indexes were most of their
/// size. A relation shared with a reader still open keeps them.
pub fn unindex(conn: &Connection, tables: &[String]) {
    let at = scope(conn);
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
    let path = conn.path().unwrap_or_default().to_owned();
    let names = |spec: &str| -> Vec<String> {
        let Some(hex) = spec.split(':').nth(1) else { return Vec::new() };
        let Ok(text) = unhex(hex) else { return Vec::new() };
        text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).map(str::to_owned).collect()
    };
    INPUTS.lock().unwrap().retain(|(p, spec), _| *p != path || !names(spec).iter().any(|w| tables.contains(w)));
    STORED.lock().unwrap().retain(|(p, t), _| *p != path || !t.rsplit('.').next().is_some_and(|n| tables.iter().any(|x| x == n)));
}

/// Drops every native relation of `conn`'s runs: a chunk's, once its exports are written to the cross-file part.
pub fn forget_derived(conn: &Connection) {
    let at = scope(conn);
    DERIVED.lock().unwrap().retain(|(s, _), _| *s != at);
}

/// A native relation of `conn`'s run, for a caller that keeps it past the run with the indexes it has (a session's
/// relations, decision 0090 step 3).
pub fn held(conn: &Connection, table: &str) -> Option<Arc<InRel>> {
    DERIVED.lock().unwrap().get(&(scope(conn), table.to_owned())).cloned()
}

/// Puts a relation kept from an earlier run into `conn`'s next run as the input `table`: its strata read it in place,
/// with the indexes it has (`r:` sources), not copied into a source and indexed again. The run's planner takes the
/// names (`adopted`).
pub fn adopt(conn: &Connection, table: &str, rel: Arc<InRel>) {
    let at = scope(conn);
    DERIVED.lock().unwrap().insert((at, table.to_owned()), rel);
    ADOPTED.lock().unwrap().insert((at, table.to_owned()));
}

/// The relations `adopt`ed into `conn`'s next run, taken by its planner.
pub fn adopted(conn: &Connection) -> std::collections::BTreeSet<String> {
    let at = scope(conn);
    let mut a = ADOPTED.lock().unwrap();
    let mine: std::collections::BTreeSet<String> = a.iter().filter(|(s, _)| *s == at).map(|(_, t)| t.clone()).collect();
    a.retain(|(s, _)| *s != at);
    mine
}
static ADOPTED: Mutex<std::collections::BTreeSet<(usize, String)>> = Mutex::new(std::collections::BTreeSet::new());

impl InRel {
    /// A relation of `rows` (each of `types.len()` values), for one a run left in a table rather than in the engine.
    pub fn flat(rows: &[Vec<i64>], types: Vec<u8>) -> Arc<InRel> {
        let k = types.len();
        let n = rows.len();
        let flat: Vec<i64> = rows.iter().flat_map(|r| r.iter().copied()).collect();
        Arc::new(InRel { k, n, rows: flat, view: None, base: false, types, stored: n, idx: Mutex::new(HashMap::default()), chains: Vec::new(), set: HashTable::new(), dead: Vec::new(), ndead: 0, summaries: Default::default(), measured: Default::default() })
    }
    /// Its rows, as a source of columns `names`.
    pub fn source(self: &Arc<Self>, names: Vec<String>) -> Arc<dyn crate::code_rule_dsl::facts::vtab::Source> {
        Derived::new(Arc::clone(self), Some(names))
    }
    /// Its rows' bytes (its indexes not counted).
    pub fn bytes(&self) -> usize {
        self.rows.len() * 8
    }
    pub fn rows(&self) -> usize {
        self.len() - self.ndead
    }
}

/// The relations of components run with `nowrite`, by table, until they are materialized.
static DERIVED: LazyLock<Mutex<HashMap<(usize, String), Arc<InRel>>>> = LazyLock::new(Default::default);
const SEED: usize = usize::MAX;
/// Columns with at most this many values report each value's postings (`stats`).
const FREQ_NDV: usize = 64;

/// A native relation's rows, in place (for a caller that keeps an output in the engine): a source of columns
/// `c0`, `c1`, ….
pub fn derived(conn: &Connection, table: &str) -> Option<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>> {
    let rel = DERIVED.lock().unwrap().get(&(scope(conn), table.to_owned())).cloned()?;
    Some(Derived::new(rel, None))
}
/// A native relation's rows as a source; `live`, a relation with dead rows' live row ids (dense, as a source's are).
struct Derived(Arc<InRel>, Option<Vec<String>>, Option<Vec<u32>>);
impl Derived {
    fn new(rel: Arc<InRel>, names: Option<Vec<String>>) -> Arc<dyn crate::code_rule_dsl::facts::vtab::Source> {
        let live = (rel.ndead > 0).then(|| (0..rel.len() as u32).filter(|&id| !is_dead(&rel.dead, id)).collect());
        Arc::new(Derived(rel, names, live))
    }
}
impl crate::code_rule_dsl::facts::vtab::Source for Derived {
    fn columns(&self) -> Vec<String> { self.1.clone().unwrap_or_else(|| (0..self.0.k).map(|i| format!("c{i}")).collect()) }
    fn len(&self) -> usize { self.2.as_ref().map_or_else(|| self.0.len(), Vec::len) }
    fn estimated_matches(&self, _: usize, _: bool) -> usize { 1 }
    fn indexed(&self, _: usize) -> bool { false }
    fn lookup(&self, _: usize, _: &[i64]) -> Option<Vec<usize>> { None }
    fn value(&self, row: usize, column: usize) -> i64 { self.0.at(self.2.as_ref().map_or(row, |l| l[row] as usize)).at(column) }
    fn statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        let m = self.0.measured.get_or_init(|| {
            self.0.posted().unwrap_or_else(|| crate::code_rule_dsl::facts::vtab::syntax::measured(self).into_iter().map(|(_, a, b, c, d)| [a, b, c, d]).collect())
        });
        Some(self.columns().into_iter().zip(m).map(|(n, s)| (n, s[0], s[1], s[2], s[3])).collect())
    }
}

/// `set_fixpoint_materialize(table)`: inserts a native relation's rows (those not already in its table) into the
/// table, for SQL that reads it. Returns the rows written.
pub fn materialize(conn: &Connection, table: &str) -> Result<i64> {
    let Some(rel) = DERIVED.lock().unwrap().get(&(scope(conn), table.to_owned())).cloned() else { return Err(err(format!("no native relation {table}"))) };
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
        match rel.row(self.id).get(i as usize) { Some(v) => ctx.set_result(v), None => ctx.set_result(&rusqlite::types::Null) }
    }
    fn rowid(&self) -> Result<i64> { Ok(self.id as i64) }
}
impl InRel {
    /// A component's relation at its fixpoint, its rows and indexes moved.
    fn moved(rel: Rel) -> InRel {
        InRel { k: rel.k, n: rel.n, rows: rel.data, view: None, base: false, types: rel.types, stored: rel.stored, idx: Mutex::new(HashMap::default()), chains: rel.idx,
                set: rel.set, dead: rel.dead, ndead: rel.ndead, summaries: rel.summaries, measured: Default::default() }
    }
    /// A whole view's rows are its source's, counted when first asked: a source that makes its rows on lookup
    /// (`syntax::within`) makes them all only for a scan.
    fn len(&self) -> usize {
        match &self.view { Some(v) if v.ids.is_none() => v.src.len(), _ => self.n }
    }
    /// A flat relation's row (a derived one: views are base inputs only).
    fn row(&self, id: usize) -> &[i64] { &self.rows[id * self.k..][..self.k] }
    #[inline]
    fn at(&self, id: usize) -> RowRef<'_> {
        match &self.view { Some(v) => RowRef::View(v, id), None => RowRef::Flat(&self.rows[id * self.k..][..self.k]) }
    }
    /// For a key of several columns over a view, the most selective column the source indexes (sorted, or by its
    /// lookup) with at most `LEAD_MATCHES` rows a value: its position in `cols`, and whether sorted.
    fn lead(&self, cols: &[usize]) -> Option<(usize, bool)> {
        const LEAD_MATCHES: usize = 8;
        let v = self.view.as_ref().filter(|_| cols.len() > 1)?;
        cols.iter()
            .enumerate()
            .filter_map(|(j, &c)| {
                let sc = v.cols[c];
                let sorted_source = v.src.sorted_rows(sc, 0).is_some();
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
        let dead: Vec<u32> = if self.ndead == 0 { Vec::new() } else { (0..self.n as u32).filter(|&id| is_dead(&self.dead, id)).collect() };
        let mut out = Vec::with_capacity(k);
        let mut docs: Vec<i64> = Vec::new();
        for c in 0..k {
            let ix = self.chains.iter().find(|ix| ix.cols == [c])?;
            if ix.next.len() < self.n {
                return None;
            }
            // The postings holding a dead row: their counts are walked.
            let touched: rustc_hash::FxHashSet<u32> = dead.iter().filter_map(|&id| ix.get(data, k, &[data[id as usize * k + c]]).map(|p| p.head)).collect();
            let (mut ndv, mut sq, mut sqd) = (0i64, 0i64, 0i64);
            for p in ix.table.iter() {
                if p.len == 1 && !touched.contains(&p.head) {
                    ndv += 1;
                    sq += 1;
                    sqd += 1;
                    continue;
                }
                docs.clear();
                let mut id = p.head;
                for _ in 0..p.len {
                    if !is_dead(&self.dead, id) {
                        docs.push(doc(id));
                    }
                    id = ix.next[id as usize];
                }
                let l = docs.len() as i64;
                if l == 0 {
                    continue;
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
            }
            out.push([live, ndv, sq, sqd]);
        }
        Some(out)
    }
    fn index(&self, cols: &[usize]) -> Arc<Csr> {
        let mask = cols.iter().fold(0u64, |m, &c| m | 1 << c);
        let mut idx = self.idx.lock().unwrap();
        idx.entry(mask).or_insert_with(|| {
            let (t0, c0) = (Instant::now(), thread_cpu());
            // No key columns: the step reads every row, not an index.
            let empty = |source, sorted| Csr { cols: cols.to_vec(), table: HashTable::new(), ids: Vec::new(), source, sorted, chain: None, lead: 0 };
            let m = if cols.is_empty() {
                empty(false, false)
            } else if let Some(i) = self.chains.iter().position(|ix| ix.cols == cols) {
                Csr { chain: Some(i), ..empty(false, false) }
            } else if let (Some(v), [c]) = (&self.view, cols) && v.ids.is_none() && v.eqs.is_empty() && v.src.sorted_rows(v.cols[*c], 0).is_some() {
                empty(false, true)
            } else if let (Some(v), [c]) = (&self.view, cols) && v.src.indexed(v.cols[*c]) && v.src.sorted_rows(v.cols[*c], 0).is_none() {
                // (A source's lookup allocates each probe's rows; over a sorted source a filtered view builds its own.)
                empty(true, false)
            } else if let Some((lead, sorted)) = self.lead(cols) {
                // A key of several columns, one of them selective in the source's own index (a node column of
                // `inside_<set>`, `node`, `name`): its candidates, checked against the other keys by the step, not a
                // hash table of every row.
                Csr { lead, ..empty(!sorted, sorted) }
            } else {
                charge(Unit::IndexEntry, self.len() as u64);
                Csr::build(self, cols)
            };
            let l = &mut LOADS.lock().unwrap()[usize::from(!self.base)];
            l.0 += thread_cpu() - c0;
            if std::env::var_os("RDL_PROFILE").is_some() {
                eprintln!("      index on {cols:?} of {} rows in {:.1} ms", self.len(), t0.elapsed().as_secs_f64() * 1e3);
            }
            Arc::new(m)
        }).clone()
    }
}
/// Input relations loaded for joins, by database and spec, for the run that loaded them: the base relations are
/// vtabs rebound per batch, so a load is not reused across runs (`forget_inputs`). T cached them for its process,
/// which ran one program over one facts file.
static INPUTS: LazyLock<Mutex<HashMap<(String, String), Arc<InRel>>>> = LazyLock::new(Default::default);

/// Drops the input relations loaded on `conn`'s database: a run starts over the relations bound now.
pub fn forget_inputs(conn: &Connection) {
    let path = conn.path().unwrap_or_default().to_owned();
    let changing = CHANGING.lock().unwrap().get(&path).cloned();
    let Some(changing) = changing else {
        INPUTS.lock().unwrap().retain(|(p, _), _| *p != path);
        STORED.lock().unwrap().retain(|(p, _), _| *p != path);
        return;
    };
    // Kept: base inputs (`b:` specs) that read none of the relations that change between runs; their rows and
    // indexes are the same in the next run. Derived inputs (`d:`, a SQL stratum's table) are made again.
    let reads_changing = |sel: &str| sel.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).any(|w| changing.contains(w));
    INPUTS.lock().unwrap().retain(|(p, spec), _| {
        if *p != path {
            return true;
        }
        let parts: Vec<&str> = spec.splitn(4, ':').collect();
        matches!(parts.as_slice(), ["b", table, ..] if unhex(table).is_ok_and(|sel| !reads_changing(&sel)))
    });
    STORED.lock().unwrap().retain(|(p, table), _| *p != path || (!table.starts_with("main.") && !table.starts_with("temp.") && !reads_changing(table)));
}

/// Relations whose rows change between the native runs on `conn` (`keep_inputs`): while set, `forget_inputs` keeps
/// every other base input loaded, with its indexes.
static CHANGING: LazyLock<Mutex<HashMap<String, std::collections::BTreeSet<String>>>> = LazyLock::new(Default::default);

/// Runs on `conn` that read the same inputs but for `changing` (the bounded rounds: each round rebinds only its
/// `__prev` inputs) keep the others loaded; `None` ends it, and the next `forget_inputs` drops everything.
pub fn keep_inputs(conn: &Connection, changing: Option<std::collections::BTreeSet<String>>) {
    let path = conn.path().unwrap_or_default().to_owned();
    match changing {
        Some(c) => {
            CHANGING.lock().unwrap().insert(path, c);
        }
        None => {
            CHANGING.lock().unwrap().remove(&path);
        }
    }
}

fn input(conn: &Connection, spec: &str, syms: &mut Interner) -> Result<Arc<InRel>> {
    let key = (conn.path().unwrap_or_default().to_owned(), spec.to_owned());
    if let Some(r) = INPUTS.lock().unwrap().get(&key) { return Ok(r.clone()); }
    let parts: Vec<&str> = spec.splitn(4, ':').collect();
    let [which, table, cols, types] = parts[..] else { return Err(err(format!("bad input {spec}"))) };
    let base = which == "b";
    let (table, cols, types) = (unhex(table)?, unhex(cols)?, types.as_bytes());
    let (t0, c0) = (Instant::now(), thread_cpu());
    // A stored table holds symbols as atoms (integers), so any column types read in place there.
    if base && (types.iter().all(|&t| t == b'i') || !table.starts_with("main.")) {
        if let Some(view) = view(conn, &table, &cols) {
            let n = view.ids.as_ref().map_or(0, Vec::len);
            let rel = Arc::new(InRel { k: types.len(), n, rows: Vec::new(), view: Some(view), base, types: types.to_vec(), stored: 0, idx: Mutex::new(HashMap::default()), chains: Vec::new(), set: HashTable::new(), dead: Vec::new(), ndead: 0, summaries: Default::default(), measured: Default::default() });
            LOADS.lock().unwrap()[0].0 += thread_cpu() - c0;
            if std::env::var_os("RDL_PROFILE").is_some() {
                eprintln!("      view of {n} rows in {:.1} ms: SELECT {cols} FROM {table}", t0.elapsed().as_secs_f64() * 1e3);
            }
            INPUTS.lock().unwrap().insert(key, rel.clone());
            return Ok(rel);
        }
    }
    let mut st = conn.prepare(&format!("SELECT {cols} FROM {}", crate::code_rule_dsl::logic::sql::table(&table)))?;
    let mut q = st.query([])?;
    let (mut rows, mut n) = (Vec::new(), 0);
    while let Some(r) = q.next()? {
        for (i, &t) in types.iter().enumerate() { rows.push(read_value(r.get_ref(i)?, t, syms).map_err(|e| err(format!("{table}: {e}")))?); }
        n += 1;
    }
    let rel = Arc::new(InRel { k: types.len(), n, rows, view: None, base, types: types.to_vec(), stored: 0, idx: Mutex::new(HashMap::default()), chains: Vec::new(), set: HashTable::new(), dead: Vec::new(), ndead: 0, summaries: Default::default(), measured: Default::default() });
    {
        let l = &mut LOADS.lock().unwrap()[usize::from(!base)];
        l.0 += thread_cpu() - c0;
        l.1 += rel.len() as u64;
    }
    if std::env::var_os("RDL_PROFILE").is_some() {
        eprintln!("      input {} rows in {:.1} ms: SELECT {cols} FROM {table}", rel.len(), t0.elapsed().as_secs_f64() * 1e3);
    }
    INPUTS.lock().unwrap().insert(key, rel.clone());
    Ok(rel)
}

/// A base input over a bound vtab, read in place: `main.<table>` with `<col> = <int>` filters (joined by AND), its
/// columns named. `None` when the table is not a bound source or the select is another shape (it is copied then).
fn view(conn: &Connection, table: &str, cols: &str) -> Option<View> {
    let (schema, rest) = table.split_once('.')?;
    let (name, filters) = match rest.split_once(" WHERE ") { Some((n, f)) => (n, Some(f)), None => (rest, None) };
    let src = if schema == "main" {
        crate::code_rule_dsl::facts::vtab::bound_source(conn, name)?
    } else {
        // The columns read and those filtered.
        let filtered = filters.into_iter().flat_map(|f| f.split(" AND ")).filter_map(|f| f.split_once(" = ").map(|(c, _)| c));
        let need: Vec<&str> = cols.split(',').chain(filtered).collect();
        stored_table(conn, schema, name, &need)?
    };
    let src=src.frozen().unwrap_or(src);
    let names = src.columns();
    let column = |c: &str| names.iter().position(|n| n == c.trim());
    let cols: Vec<usize> = cols.split(',').map(column).collect::<Option<_>>()?;
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
    index: Vec<std::sync::OnceLock<Vec<u32>>>,
    /// Each column's count of distinct values, from its index (for SQL's estimates).
    distinct: Vec<std::sync::OnceLock<usize>>,
    /// Lexicographic orders over several columns, built on a first `lookup_all` over them (4 bytes a row each;
    /// the table never changes, so an order costs no insert). One serves every search whose columns are its prefix.
    orders: Mutex<Vec<(Vec<usize>, Arc<Vec<u32>>)>>,
    /// For rows held in memory (`columns_source`), the planner's statistics, read from the column indexes when first
    /// asked and kept with the rows: a session's next run reads them for its `r__old` (0094 §4), not measured again.
    /// `None` for a stored table, whose columns load on demand.
    statistics: Option<std::sync::OnceLock<Vec<(String, i64, i64, i64, i64)>>>,
}
impl Stored {
    fn new(columns: Vec<String>, values: Vec<std::sync::OnceLock<Vec<i64>>>) -> Self {
        let k = columns.len();
        Stored { columns, n: std::sync::OnceLock::new(), values, index: (0..k).map(|_| std::sync::OnceLock::new()).collect(), distinct: (0..k).map(|_| std::sync::OnceLock::new()).collect(), orders: Mutex::default(), statistics: None }
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
                let (ids, col) = (self.index(c), self.col(c));
                let (mut ndv, mut sq, mut sqd) = (0i64, 0i64, 0i64);
                let mut at = 0;
                while at < ids.len() {
                    let v = col[ids[at] as usize];
                    let end = at + ids[at..].partition_point(|&r| col[r as usize] == v);
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
                    at = end;
                }
                (name, n as i64, ndv, sq, sqd)
            })
            .collect()
    }
    fn distinct(&self, c: usize) -> usize {
        *self.distinct[c].get_or_init(|| {
            let (ids, col) = (self.index(c), self.col(c));
            1 + ids.windows(2).filter(|w| col[w[0] as usize] != col[w[1] as usize]).count()
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
        self.index[c].get_or_init(|| {
            let col = self.col(c);
            charge(Unit::IndexEntry, col.len() as u64);
            let mut ids: Vec<u32> = (0..col.len() as u32).collect();
            ids.sort_by_key(|&r| col[r as usize]);
            ids
        })
    }
    fn rows_of(&self, c: usize, v: i64) -> &[u32] {
        let (ids, col) = (self.index(c), self.col(c));
        let lo = ids.partition_point(|&r| col[r as usize] < v);
        let hi = ids.partition_point(|&r| col[r as usize] <= v);
        &ids[lo..hi]
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
impl crate::code_rule_dsl::facts::vtab::Source for Stored {
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
        let mut out: Vec<usize> = values.iter().flat_map(|&v| self.rows_of(c, v).iter().map(|&r| r as usize)).collect();
        out.sort_unstable();
        Some(out)
    }
    /// A column held nowhere (no rule names it, `columns_source`) reads as 0.
    fn value(&self, row: usize, c: usize) -> i64 { self.col(c).get(row).copied().unwrap_or(0) }
    fn sorted_rows(&self, c: usize, v: i64) -> Option<&[u32]> { Some(self.rows_of(c, v)) }
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
/// The table `schema.name` read once into memory, every column (`None`: not a table of integers).
pub fn table_source(conn: &Connection, schema: &str, name: &str) -> Option<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>> {
    let columns: Vec<String> = conn.prepare(&format!("SELECT * FROM {schema}.{}", crate::code_rule_dsl::logic::sql::ident(name))).ok()?.column_names().into_iter().map(str::to_owned).collect();
    let need: Vec<&str> = columns.iter().map(String::as_str).collect();
    stored_table(conn, schema, name, &need)
}
/// The stored tables read this run, by database and `schema.table` (dropped with the inputs, `forget_inputs`).
static STORED: LazyLock<Mutex<HashMap<(String, String), Option<Arc<Stored>>>>> = LazyLock::new(Default::default);
/// The stored table `schema.name` with the columns named in `need` loaded (`None`: not one, or not integers).
fn stored_table(conn: &Connection, schema: &str, name: &str, need: &[&str]) -> Option<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>> {
    if schema == "temp" || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') { return None; }
    let key = (conn.path().unwrap_or_default().to_owned(), format!("{schema}.{name}"));
    let t = STORED.lock().unwrap().get(&key).cloned();
    let t = match t {
        Some(t) => t?,
        None => {
            // The columns' names, from the statement (prepared, not run).
            let columns: Vec<String> = conn.prepare(&format!("SELECT * FROM {schema}.{}", crate::code_rule_dsl::logic::sql::ident(name))).ok()?.column_names().into_iter().map(str::to_owned).collect();
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
enum Key { Slot(usize), Const(i64) }
enum Src { Delta(usize), Full(usize), FullMulti(usize, Vec<usize>), In(Arc<InRel>, Arc<Csr>) }
enum Step {
    Unused { proof: crate::code_rule_dsl::logic::analysis::Decision },
    /// Key columns (bound), columns that bind a slot, and column pairs that must be equal (a variable repeated in
    /// the atom, first seen here).
    Atom { src: Src, neg: bool, keys: Vec<(usize, Key)>, binds: Vec<(usize, usize)>, tests: Vec<(usize, usize)> },
    Cat { out: usize, a: Key, b: Key, bind: bool },
    /// `whole` split at each "." into `a` and `b`; `Some` slots bind, `None` parts (`ka`/`kb`) are tested.
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
pub struct JoinRule { steps: Vec<Step>, out: Vec<Term>, nslots: usize, multi: Vec<(usize, Vec<usize>)>, counts: Vec<[std::cell::Cell<u64>; 2]>,
                      fused: Vec<Option<Box<rowcode::Fused>>> }

fn parse_join(conn: &Connection, spec: &str, syms: &mut Interner) -> Result<JoinRule> {
    let mut bound: Vec<bool> = Vec::new();
    let mut steps = Vec::new();
    let mut out = Vec::new();
    let mut multi = Vec::new();
    fn tok(t: &str, syms: &mut Interner) -> Result<Option<Key>> {
        Ok(if t == "_" { None } else if let Some(i) = t.strip_prefix("i:") { Some(Key::Const(num(i)?)) }
           else if let Some(x) = t.strip_prefix("x:") { Some(Key::Const(syms.id(&unhex(x)?))) } else { Some(Key::Slot(slot(t)?)) })
    }
    let grow = |b: &mut Vec<bool>, s: usize| if b.len() <= s { b.resize(s + 1, false) };
    for line in spec.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let toks: Vec<&str> = line.split_whitespace().collect();
        let is_bound = |b: &Vec<bool>, k: Key| match k { Key::Slot(s) => b.get(s).copied().unwrap_or(false), Key::Const(_) => true };
        match toks[0] {
            "pos" | "neg" | "count" | "sum" | "min" | "max" | "strjoin" => {
                let (neg, count, extreme, sjoin) = (toks[0] == "neg", toks[0] == "count", matches!(toks[0], "sum" | "min" | "max"), toks[0] == "strjoin");
                // Scalar value columns use `_` tokens. A strjoin additionally carries value,
                // ordering, separator, and output tokens after the atom columns.
                if sjoin && toks.len() < 7 { return Err(err("bad strjoin")); }
                let atoks = if count { &toks[2..toks.len() - 1] } else if extreme { &toks[2..toks.len() - 2] } else if sjoin { &toks[2..toks.len() - 4] } else { &toks[2..] };
                let count = count || sjoin;
                let (mut keys, mut binds, mut tests) = (Vec::new(), Vec::new(), Vec::new());
                let mut locals=HashMap::default();
                for (col, t) in atoks.iter().enumerate() {
                    if (count || extreme) && let Some(local)=t.strip_prefix('a') {
                        if let Some(previous)=locals.insert(local,col) {tests.push((col,previous));}
                        continue;
                    }
                    match tok(t, syms)? {
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
                let src = match toks[1] {
                    t if t.starts_with("d") => Src::Delta(num(&t[1..])?),
                    t if t.starts_with("c") && keys.len() >= 2 => {
                        let (r, cols): (usize, Vec<usize>) = (num(&t[1..])?, keys.iter().map(|&(c, _)| c).collect());
                        multi.push((r, cols.clone()));
                        Src::FullMulti(r, cols)
                    }
                    t if t.starts_with("c") => Src::Full(num(&t[1..])?),
                    t if t.starts_with("r:") => {
                        let table = unhex(&t[2..])?;
                        let rel = DERIVED.lock().unwrap().get(&(scope(conn), table.clone())).cloned().ok_or_else(|| err(format!("no native relation {table}")))?;
                        let cols: Vec<usize> = keys.iter().map(|&(c, _)| c).collect();
                        let ix = rel.index(&cols);
                        Src::In(rel, ix)
                    }
                    t if t.starts_with("t:") => {
                        let cols: Vec<usize> = keys.iter().map(|&(c, _)| c).collect();
                        let rel = input(conn, &t[2..], syms)?;
                        let ix = rel.index(&cols);
                        Src::In(rel, ix)
                    }
                    t => return Err(err(format!("bad source {t}"))),
                };
                if sjoin {
                    let n = toks.len();
                    let column = |t: &str| -> Result<(usize, bool)> {
                        let (c, sym) = t.split_once(':').ok_or_else(|| err("bad strjoin column"))?;
                        Ok((num(c)?, sym == "1"))
                    };
                    let x = column(toks[n - 4])?;
                    let by = if toks[n - 3] == "-" { Vec::new() } else { toks[n - 3].split(',').map(column).collect::<Result<_>>()? };
                    let Some(Key::Const(sep)) = tok(toks[n - 2], syms)? else { return Err(err("a strjoin's separator is a constant")) };
                    let Some(Key::Slot(o)) = tok(toks[n - 1], syms)? else { return Err(err("bad strjoin")) };
                    let bind = !is_bound(&bound, Key::Slot(o));
                    grow(&mut bound, o); bound[o] = true;
                    steps.push(Step::StrJoin { src, keys, tests, x, by, sep, out: o, bind });
                } else if extreme {
                    let col = num(toks[toks.len() - 2])?;
                    if col >= atoks.len() { return Err(err("native scalar value column is out of range")); }
                    let Some(Key::Slot(o)) = tok(toks[toks.len() - 1], syms)? else { return Err(err("bad min or max")) };
                    let bind = !is_bound(&bound, Key::Slot(o));
                    grow(&mut bound, o); bound[o] = true;
                    steps.push(Step::Scalar { src, keys, tests, col: Some(col), out: o, bind, kind: match toks[0] {"sum"=>ScalarKind::Sum,"min"=>ScalarKind::Min,_=>ScalarKind::Max}, bag: None });
                } else if count {
                    let Some(Key::Slot(o)) = tok(toks[toks.len() - 1], syms)? else { return Err(err("bad count")) };
                    let bind = !is_bound(&bound, Key::Slot(o));
                    grow(&mut bound, o); bound[o] = true;
                    steps.push(Step::Scalar { src, keys, tests, col: None, out: o, bind, kind: ScalarKind::Count, bag: None });
                } else {
                    steps.push(Step::Atom { src, neg, keys, binds, tests });
                }
            }
            "cat" => {
                let (o, a, b) = (tok(toks[1], syms)?, tok(toks[2], syms)?, tok(toks[3], syms)?);
                let (Some(Key::Slot(o)), Some(a), Some(b)) = (o, a, b) else { return Err(err("bad cat")) };
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
            "lt" | "le" => {
                let (Some(a), Some(b)) = (tok(toks[1], syms)?, tok(toks[2], syms)?) else { return Err(err("bad comparison")) };
                if !is_bound(&bound, a) || !is_bound(&bound, b) { return Err(err("order of an unbound slot")); }
                steps.push(Step::Lt { a, b, or_eq: toks[0] == "le" });
            }
            "func" => {
                // `func <name> <out> <args…>`
                let Some((name, kinds, _)) = crate::code_rule_dsl::logic::strings::functions::signature(toks[1]) else { return Err(err(format!("no function {}", toks[1]))) };
                let args: Vec<Key> = toks[3..].iter().map(|t| tok(t, syms)).collect::<Result<Option<Vec<_>>>>()?.ok_or_else(|| err("a function of `_`"))?;
                if args.len() != kinds.len() || args.iter().any(|&a| !is_bound(&bound, a)) { return Err(err(format!("{name} of unbound or missing arguments"))); }
                let Some(Key::Slot(o)) = tok(toks[2], syms)? else { return Err(err("a function's value into a constant")) };
                let bind = !is_bound(&bound, Key::Slot(o));
                grow(&mut bound, o); bound[o] = true;
                steps.push(Step::Func { name, kinds, args, out: o, bind });
            }
            "test" => {
                // `test <op> <s> <pattern> <0|1>`
                use crate::code_rule_dsl::logic::strings;
                let Some(&op) = strings::TESTS.iter().find(|&&t| t == toks[1]) else { return Err(err(format!("no text test {}", toks[1]))) };
                let (Some(s), Some(p)) = (tok(toks[2], syms)?, tok(toks[3], syms)?) else { return Err(err("a text test of `_`")) };
                if !is_bound(&bound, s) || !is_bound(&bound, p) { return Err(err(format!("{op} of an unbound slot"))); }
                let matcher = match p {
                    Key::Const(c) => {
                        let patterns: Vec<String> = syms.name(c).split(strings::SET_SEP).map(str::to_owned).collect();
                        // A pattern that does not compile matches nothing, as SQL's `str_test`: `strings::test` per row.
                        strings::Matcher::new(op, &patterns).ok().map(Arc::new)
                    }
                    Key::Slot(_) => None,
                };
                steps.push(Step::Test { op, s, p, matcher, want: toks[4] == "1" });
            }
            "docof" => {
                // `docof <out> <n>`
                let (Some(Key::Slot(o)), Some(n)) = (tok(toks[1], syms)?, tok(toks[2], syms)?) else { return Err(err("bad docof")) };
                if !is_bound(&bound, n) { return Err(err("the document of an unbound slot")); }
                let bind = !is_bound(&bound, Key::Slot(o));
                grow(&mut bound, o); bound[o] = true;
                steps.push(Step::DocOf { out: o, n, bind });
            }
            "testset" => {
                // `testset <op> <s> <0|1> <constants…>`: a text test against a set of symbols' names.
                use crate::code_rule_dsl::logic::strings;
                let Some(&op) = strings::TESTS.iter().find(|&&t| t == toks[1]) else { return Err(err(format!("no text test {}", toks[1]))) };
                let Some(s) = tok(toks[2], syms)? else { return Err(err("a text test of `_`")) };
                if !is_bound(&bound, s) { return Err(err(format!("{op} of an unbound slot"))); }
                let patterns = toks[4..].iter().map(|t| match tok(t, syms)? { Some(Key::Const(c)) => Ok(syms.name(c).to_string()), _ => Err(err("a pattern that is not a constant")) }).collect::<Result<Vec<String>>>()?;
                let matcher = strings::Matcher::new(op, &patterns).ok().map(Arc::new);
                // A set that does not compile matches nothing, as SQL's `str_test`.
                let p = Key::Const(syms.id(&patterns.join(&strings::SET_SEP.to_string())));
                steps.push(Step::Test { op, s, p, matcher, want: toks[3] == "1" });
            }
            "in" => {
                // `in <a> <0|1> <constants…>`
                let Some(a) = tok(toks[1], syms)? else { return Err(err("a membership test of `_`")) };
                if !is_bound(&bound, a) { return Err(err("a membership test of an unbound slot")); }
                let set = toks[3..].iter().map(|t| match tok(t, syms)? { Some(Key::Const(c)) => Ok(c), _ => Err(err("a set member that is not a constant")) }).collect::<Result<Vec<_>>>()?;
                steps.push(Step::In { a, set, want: toks[2] == "1" });
            }
            "deq" | "dne" => {
                let (Some(a), Some(b)) = (tok(toks[1], syms)?, tok(toks[2], syms)?) else { return Err(err("bad comparison")) };
                if !is_bound(&bound, a) || !is_bound(&bound, b) { return Err(err("a document comparison of an unbound slot")); }
                steps.push(Step::Doc { a, b, eq: toks[0] == "deq" });
            }
            "eq" | "ne" => {
                let (Some(a), Some(b)) = (tok(toks[1], syms)?, tok(toks[2], syms)?) else { return Err(err("bad comparison")) };
                if !is_bound(&bound, b) { return Err(err("comparison with an unbound right side")); }
                match (toks[0], a) {
                    ("eq", Key::Slot(s)) if !is_bound(&bound, a) => { grow(&mut bound, s); bound[s] = true; steps.push(Step::Let { a: s, b }) }
                    _ if !is_bound(&bound, a) => return Err(err("comparison with an unbound left side")),
                    ("eq", _) => steps.push(Step::Eq { a, b }),
                    _ => steps.push(Step::Ne { a, b }),
                }
            }
            "head" => {
                for t in &toks[1..] {
                    out.push(if let Some(c) = t.strip_prefix("cat:") {
                        let (a, b) = c.split_once(':').ok_or_else(|| err("bad cat"))?;
                        Term::Cat(slot(a)?, slot(b)?)
                    } else {
                        match tok(t, syms)? { Some(Key::Const(c)) => Term::Const(c), Some(Key::Slot(s)) => Term::Slot(s), None => return Err(err("_ in a head")) }
                    });
                }
            }
            other => return Err(err(format!("unknown join item {other}"))),
        }
    }
    // Every head term reads a slot the body binds: an unbound one is refused here, not read out of range later.
    let unbound = |s: usize| !bound.get(s).copied().unwrap_or(false);
    if out.iter().any(|t| match t { Term::Slot(s) => unbound(*s), Term::Cat(a, b) => unbound(*a) || unbound(*b), Term::Const(_) => false }) {
        return Err(err(format!("a head reads a slot its body does not bind: {spec}")));
    }
    liveness::sums(&mut steps,&out);
    scalars::plan(&mut steps);
    let counts = steps.iter().map(|_| Default::default()).collect();
    let fused = if std::env::var_os("RERAIL_NO_ROW_PROGRAMS").is_some() { steps.iter().map(|_| None).collect() } else { rowcode::fuse(&steps) };
    Ok(JoinRule { steps, out, nslots: bound.len(), multi, counts, fused })
}

/// Runs a join probe over the store; its new head tuples go to `fresh`. Returns the tuples it derived.
fn run_join(j: &JoinRule, head: usize, store: &Store, syms: &mut Interner, fresh: &mut [Fresh]) -> Result<u64> {
    struct Ctx<'a> { j: &'a JoinRule, head: usize, store: &'a Store, syms: &'a mut Interner, fresh: &'a mut [Fresh], n: u64,
                     key: Vec<i64>, row: Vec<i64>, error: Option<Error>, pending: Vec<(usize,i128)>, scratch: Vec<rowcode::Scratch> }
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
    fn go<'a>(cx: &mut Ctx<'a>, i: usize, slots: &mut [i64]) {
        if cx.error.is_some() { return; }
        let (j, store): (&'a JoinRule, &'a Store) = (cx.j, cx.store);
        if i > 0 { let c = &j.counts[i - 1][1]; c.set(c.get() + 1); }
        if let Some(c) = j.counts.get(i) { c[0].set(c[0].get() + 1); }
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
            if !store.rels[cx.head].contains(&cx.row) { cx.fresh[cx.head].add(&cx.row); }
            return;
        };
        /// The candidates: an input's index on exactly the key columns, a component relation's composite index, or
        /// its shortest posting chain among its key columns (every key is then checked), or every tuple when none is
        /// bound.
        fn candidates<'a>(src: &'a Src, store: &'a Store, key: &[i64], keys: &[(usize, Key)]) -> (Rows<'a>, Cands<'a>) {
            match src {
                Src::In(rel, ix) => {
                    if keys.is_empty() { (Rows::In(rel), Cands::Range(0, rel.len() as u32)) }
                    else if let Some(i) = ix.chain {
                        let index = &rel.chains[i];
                        (Rows::In(rel), index.get(&rel.rows, rel.k, key).map_or(Cands::Slice(&[]), |p| Cands::Chain { head: p.head, next: &index.next, stop: 0 }))
                    }
                    else if ix.sorted && let Some(v) = &rel.view { (Rows::Source(v), Cands::Slice(v.src.sorted_rows(v.cols[ix.cols[ix.lead]], key[ix.lead]).unwrap_or(&[]))) }
                    else if ix.source && let Some(v) = &rel.view { (Rows::Source(v), Cands::Owned(v.find(ix.cols[ix.lead], key[ix.lead]))) }
                    else { (Rows::In(rel), Cands::Slice(ix.get(rel, key))) }
                }
                Src::FullMulti(r, cols) => {
                    let rel = &store.rels[*r];
                    let ix = rel.idx.iter().position(|ix| &ix.cols == cols).expect("composite index");
                    (Rows::Comp(rel), rel.postings(ix, key, false).0)
                }
                Src::Delta(r) | Src::Full(r) => {
                    let rel = &store.rels[*r];
                    let delta = matches!(src, Src::Delta(_));
                    if keys.is_empty() {
                        (Rows::Comp(rel), if delta { Cands::Range(rel.lo as u32, rel.hi as u32) } else { Cands::Range(0, rel.n as u32) })
                    } else {
                        let best = keys.iter().zip(key).map(|(&(c, _), &v)| rel.postings(c, &[v], delta)).min_by_key(|&(_, n)| n);
                        (Rows::Comp(rel), best.map_or(Cands::Slice(&[]), |b| b.0))
                    }
                }
        }
        }
        match step {
            Step::Atom { src, neg, keys, binds, tests } => {
                cx.key.clear();
                cx.key.extend(keys.iter().map(|&(_, k)| val(slots, k)));
                let (rows, cands) = candidates(src, store, &cx.key, keys);
                if let Some(f) = j.fused[i].as_deref() {
                    // Fused with the filters after it: the candidates a chunk at a time, the filters one row program
                    // over each chunk, and only its rows go on (`rowcode`).
                    let mut s = std::mem::take(&mut cx.scratch[i]);
                    s.columns.resize_with(f.columns.len(), Vec::new);
                    s.params.clear();
                    s.params.extend(f.params.iter().map(|p| { let v = val(slots, p.key); if p.doc { v >> 32 } else { v } }));
                    fn flush(cx: &mut Ctx<'_>, i: usize, f: &rowcode::Fused, s: &mut rowcode::Scratch, slots: &mut [i64]) {
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
                                go(cx, i + 1 + f.steps, slots);
                            }
                        }
                        s.rows = 0;
                        s.columns.iter_mut().for_each(Vec::clear);
                    }
                    cands.any(|id| {
                        if rows.dead(id) { return false; }
                        let row = rows.row(id);
                        if !keys.iter().all(|&(c, k)| row.at(c) == val(slots, k)) || !tests.iter().all(|&(c, c0)| row.at(c) == row.at(c0)) { return false; }
                        for &(k, c) in &f.gather { s.columns[k].push(row.at(c)); }
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
                let found = cands.any(|id| {
                    if rows.dead(id) { return false; }
                    let row = rows.row(id);
                    if !keys.iter().all(|&(c, k)| row.at(c) == val(slots, k)) || !tests.iter().all(|&(c, c0)| row.at(c) == row.at(c0)) { return false; }
                    if exists { return true; }
                    for &(c, s) in binds { slots[s] = row.at(c); }
                    go(cx, i + 1, slots);
                    false
                });
                if exists && found != *neg { go(cx, i + 1, slots); }
            }
            Step::Scalar { src, keys, tests, col, kind, out, bind, bag } => {
                cx.key.clear();
                cx.key.extend(keys.iter().map(|&(_, k)| val(slots, k)));
                let build = |value: Option<usize>| {
                    let _cpu=crate::code_rule_dsl::logic::profile::span(||"native scalar summary fold".into());
                    let mut inspected=0;
                    let (rows, cands) = candidates(src, store, &cx.key, keys);
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
                let (rows, cands) = candidates(src, store, &cx.key, keys);
                let mut got: Vec<(Vec<Sortable>, String)> = Vec::new();
                let syms = &cx.syms;
                let sortable = |v: i64, sym: bool| if sym { Sortable::Text(syms.name(v).to_string()) } else { Sortable::Number(v) };
                cands.any(|id| {
                    if rows.dead(id) { return false; }
                    let row = rows.row(id);
                    if keys.iter().all(|&(c, k)| row.at(c) == val(slots, k)) && tests.iter().all(|&(c, c0)| row.at(c) == row.at(c0)) {
                        let value = if x.1 { syms.name(row.at(x.0)).to_string() } else { row.at(x.0).to_string() };
                        got.push((by.iter().map(|&(c, sym)| sortable(row.at(c), sym)).collect(), value));
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
                let w = cx.syms.name(val(slots, *whole)).to_owned();
                // `@dot("", w)` is `w`: with "" given for the first part, the second is the whole.
                if a.is_none() && cx.syms.name(val(slots, *ka)).is_empty() {
                    match b { Some(s) => slots[*s] = val(slots, *whole), None => if val(slots, *kb) != val(slots, *whole) { return } }
                    go(cx, i + 1, slots);
                    return;
                }
                for (p, _) in w.match_indices('.') {
                    let (x, y) = (&w[..p], &w[p + 1..]);
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
    let mut cx = Ctx { j, head, store, syms, fresh, n: 0, key: Vec::new(), row: Vec::new(), error: None, pending: Vec::new(), scratch };
    let mut slots = vec![0i64; j.nslots];
    go(&mut cx, 0, &mut slots);
    scalars::explain(&j.steps,store);
    match cx.error {Some(e)=>Err(e),None=>Ok(cx.n)}
}

#[cfg(test)]
mod posted_tests {
    use super::*;

    /// Statistics read from a kept relation's postings are those measured from its rows, with dead rows left out.
    #[test]
    fn statistics_from_postings_are_the_measured_ones() {
        let node = |d: i64, i: i64| (d << 32) | i;
        let mut rel = Rel::new("t", b"iii");
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
        for rel in [old.clone_rows(), InRel::moved(Rel::based("t", b"iii", &old, &gone).expect("few rows dead"))] {
            let rel = Arc::new(rel);
            let source = Derived::new(Arc::clone(&rel), None);
            let measured: Vec<[i64; 4]> = crate::code_rule_dsl::facts::vtab::syntax::measured(source.as_ref()).into_iter().map(|(_, a, b, c, d)| [a, b, c, d]).collect();
            assert_eq!(rel.posted().expect("a relation with its postings"), measured);
        }
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
                    set: self.set.clone(), dead: self.dead.clone(), ndead: self.ndead, summaries: Default::default(), measured: Default::default() }
        }
    }
}
