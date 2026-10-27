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
//!   rel <table> <types>                        types: one char per column, `i` (int or node) or `s` (symbol)
//!   rule <head rel> <src rel> <edge table|-> <edge types|-> key <slot..> ; ecols <slot..> ; pat <tok..> ; head <tok..>
//!   probe <head rel> <delta rel> <hex SQL>     the SQL's columns are the head's
//!   seed <head rel> <hex join>                 a native join run once, in the first wave
//!   join <head rel> <delta rel> <hex join> [<hex join>]   a native probe; the second, if given, runs instead in the
//!                                              wave whose delta is the whole relation (any order, read in full)
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

fn err(m: impl Into<String>) -> Error { Error::UserFunctionError(m.into().into()) }

/// Symbols are the store's atoms (`content_store::atoms`, C12), so a symbol column holds atom ids and joins
/// directly with fact families' atom columns. T numbered symbols per process and stored them as text.
/// Each database has one mirror of its `atoms` table here (`symbols`): an entry point catches it up with the table
/// (`sync`), a run interns the symbols it creates (`cat`) after the table's tail, and `flush` appends those to the
/// table before any of them is written, in the caller's (write) transaction. `buf`: scratch for `cat`, so joining
/// two symbols allocates only when the result is new.
#[derive(Default)]
pub struct Interner { atoms: crate::content_store::atoms::Interner, stored: usize, buf: String }
impl Interner {
    fn id(&mut self, s: &str) -> i64 { i64::from(self.atoms.intern(s)) }
    fn get(&self, s: &str) -> Option<i64> { self.atoms.get(s).map(i64::from) }
    fn name(&self, v: i64) -> &str { self.atoms.str(v as crate::content_store::atoms::Sym) }
    /// Drops symbols never flushed (a failed run's) and reads the table's new tail.
    fn sync(&mut self, conn: &Connection) -> Result<()> {
        self.atoms.truncate(self.stored);
        self.atoms.catch_up(conn).map_err(|e| err(e.to_string()))?;
        self.stored = self.atoms.len();
        Ok(())
    }
    /// Appends the symbols interned since `sync` to the table.
    fn flush(&mut self, conn: &Connection) -> Result<()> {
        let mut insert = conn.prepare_cached("INSERT INTO atoms(atom, value) VALUES (?1, ?2)")?;
        for atom in self.stored..self.atoms.len() {
            insert.execute(rusqlite::params![atom as i64, self.atoms.str(atom as crate::content_store::atoms::Sym)])?;
        }
        self.stored = self.atoms.len();
        Ok(())
    }
}
static SYMS: LazyLock<Mutex<HashMap<String, Arc<RwLock<Interner>>>>> = LazyLock::new(Default::default);
/// The symbols of `conn`'s database, caught up with its `atoms` table.
fn symbols(conn: &Connection) -> Result<Arc<RwLock<Interner>>> {
    let key = conn.path().unwrap_or_default().to_owned();
    let syms = Arc::clone(SYMS.lock().unwrap().entry(key).or_default());
    syms.write().unwrap().sync(conn)?;
    Ok(syms)
}

/// `sym_text(a)`, `sym_atom(text)` and `sym_cat(a, b)` (`a.b`) over `conn`'s symbols: the planner's SQL holds symbols
/// as atoms (decision 0009), and these are the only places it reads or makes their text. NULL gives NULL. New symbols
/// reach the table at `flush_symbols`.
pub fn register_symbols(conn: &Connection) -> Result<()> {
    use rusqlite::functions::FunctionFlags;
    let det = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC;
    let syms = symbols(conn)?;
    let s = Arc::clone(&syms);
    conn.create_scalar_function(c"sym_text", 1, det, move |ctx| {
        let Some(a) = ctx.get::<Option<i64>>(0)? else { return Ok(None) };
        let s = s.read().unwrap();
        if a < 0 || a as usize >= s.atoms.len() {
            return Err(err(format!("no symbol {a}")));
        }
        Ok(Some(s.name(a).to_owned()))
    })?;
    let s = Arc::clone(&syms);
    conn.create_scalar_function(c"sym_atom", 1, det, move |ctx| {
        let Some(t) = ctx.get::<Option<String>>(0)? else { return Ok(None) };
        Ok(Some(s.write().unwrap().id(&t)))
    })?;
    conn.create_scalar_function(c"sym_cat", 2, det, move |ctx| {
        let (Some(a), Some(b)) = (ctx.get::<Option<i64>>(0)?, ctx.get::<Option<i64>>(1)?) else { return Ok(None) };
        let mut s = syms.write().unwrap();
        if a.min(b) < 0 || a.max(b) as usize >= s.atoms.len() {
            return Err(err(format!("no symbol {}", if a < 0 || a as usize >= s.atoms.len() { a } else { b })));
        }
        Ok(Some(cat(&mut s, a, b)))
    })?;
    Ok(())
}

/// Appends the symbols `conn`'s SQL interned since the last flush to its `atoms` table.
pub fn flush_symbols(conn: &Connection) -> Result<()> {
    let key = conn.path().unwrap_or_default().to_owned();
    let syms = SYMS.lock().unwrap().get(&key).cloned();
    if let Some(syms) = syms {
        syms.write().unwrap().flush(conn)?;
    }
    Ok(())
}

enum Pat { Any, Slot(usize), Int(i64), Sym(i64) }
enum Term { Slot(usize), Const(i64), Cat(usize, usize) }

fn hash_vals(vals: impl Iterator<Item = i64>) -> u64 {
    let mut h = FxHasher::default();
    for v in vals { h.write_i64(v); }
    h.finish()
}

const NIL: u32 = u32::MAX;
/// A key's postings in an `Index`: its newest row, its row count, and its count among the rows added in wave `wave`
/// (the relation's current delta when `wave` is the relation's).
#[derive(Clone, Copy)]
struct Post { head: u32, len: u32, dlen: u32, wave: u32 }
/// A hash index of a relation on a key-column set. A key's rows are chained newest first through `next` (one slot
/// per row), so a posting costs no allocation, and since a wave's new rows are the newest, a key's delta rows are a
/// prefix of its chain.
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
struct Rel { table: String, types: Vec<u8>, k: usize, n: usize, data: Vec<i64>, set: HashTable<u32>, stored: usize, lo: usize, hi: usize,
             wave: u32, idx: Vec<Index> }
impl Rel {
    fn new(table: &str, types: &[u8]) -> Rel {
        let k = types.len();
        let idx = (0..k).map(|c| Index { cols: vec![c], table: HashTable::new(), next: Vec::new() }).collect();
        Rel { table: table.to_string(), types: types.to_vec(), k, n: 0, data: Vec::new(), set: HashTable::new(), stored: 0, lo: 0, hi: 0, wave: 0, idx }
    }
    fn row(&self, id: usize) -> &[i64] { &self.data[id * self.k..][..self.k] }
    fn contains(&self, row: &[i64]) -> bool {
        let (data, k) = (&self.data, self.k);
        self.set.find(hash_vals(row.iter().copied()), |&id| &data[id as usize * k..][..k] == row).is_some()
    }
    fn add_multi(&mut self, cols: &[usize]) {
        if self.idx.iter().any(|ix| ix.cols == cols) { return; }
        let mut ix = Index { cols: cols.to_vec(), table: HashTable::new(), next: Vec::with_capacity(self.n) };
        for id in 0..self.n { ix.add(&self.data, self.k, id as u32, if id >= self.lo { self.wave } else { 0 }); }
        self.idx.push(ix);
    }
    /// Adds a row (of the next wave, its delta) unless it is there.
    fn push(&mut self, row: &[i64]) -> bool {
        let h = hash_vals(row.iter().copied());
        let k = self.k;
        if self.set.find(h, |&id| &self.data[id as usize * k..][..k] == row).is_some() { return false; }
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
#[derive(Clone, Copy)]
enum Cands<'a> { Range(u32, u32), Slice(&'a [u32]), Chain { head: u32, next: &'a [u32], stop: u32 } }
impl Cands<'_> {
    /// Calls `f` on each id until it returns true; returns whether one did.
    fn any(self, mut f: impl FnMut(u32) -> bool) -> bool {
        match self {
            Cands::Range(lo, hi) => (lo..hi).any(f),
            Cands::Slice(ids) => ids.iter().any(|&id| f(id)),
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
/// any literal order, reading that relation in full.
struct ProbeRule { head: usize, delta: usize, body: Body, whole: Option<JoinRule> }

pub struct Store { rels: Vec<Rel>, syms: Arc<RwLock<Interner>> }
/// `seeded`: the seed joins have run (in the first wave). `write`: at the fixpoint, insert the new rows into the
/// relations' tables (else they stay native, in `DERIVED`, until `set_fixpoint_materialize`).
struct Component { store: Arc<RwLock<Store>>, edges: Vec<EdgeRule>, probes: Vec<ProbeRule>, gates: Vec<Gate>, seeded: bool, write: bool }
/// A gated negation's `done` relation, fed from `src` (its demand) once `quiet` is: `passed` src rows so far.
struct Gate { done: usize, src: Option<usize>, quiet: Vec<usize>, after: Vec<usize>, passed: usize }

static COMPONENTS: Mutex<Vec<(String, Arc<Mutex<Component>>)>> = Mutex::new(Vec::new());
fn component(name: &str) -> Result<Arc<Mutex<Component>>> {
    COMPONENTS.lock().unwrap().iter().find(|(n, _)| n == name).map(|c| c.1.clone()).ok_or_else(|| err(format!("no set fixpoint named {name:?}")))
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

fn read_rows(conn: &Connection, table: &str, types: &[u8], syms: &mut Interner) -> Result<Vec<Vec<i64>>> {
    let cols: Vec<String> = (0..types.len()).map(|i| format!("c{i}")).collect();
    let mut st = conn.prepare(&format!("SELECT {} FROM {table}", cols.join(", ")))?;
    let mut rows = st.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let mut row = Vec::with_capacity(types.len());
        for (i, &t) in types.iter().enumerate() { row.push(read_value(r.get_ref(i)?, t, syms)?); }
        out.push(row);
    }
    Ok(out)
}

fn slot(t: &str) -> Result<usize> { t.strip_prefix('v').and_then(|s| s.parse().ok()).ok_or_else(|| err(format!("bad slot {t}"))) }
fn num<T: std::str::FromStr>(t: &str) -> Result<T> { t.parse().map_err(|_| err(format!("bad number {t}"))) }

pub fn open(conn: &Connection, spec: &str) -> Result<String> {
    let symbols = symbols(conn)?;
    let mut syms = symbols.write().unwrap();
    let (mut name, mut rels, mut edges, mut probes, mut gates) = (String::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut write = true;
    for line in spec.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let toks: Vec<&str> = line.split_whitespace().collect();
        match toks[0] {
            "name" => name = toks[1].to_string(),
            "rel" => {
                let mut rel = Rel::new(toks[1], toks[2].as_bytes());
                for row in read_rows(conn, &rel.table, &rel.types.clone(), &mut syms)? { rel.push(&row); }
                rel.stored = rel.n;
                rels.push(rel);
            }
            "probe" => probes.push(ProbeRule { head: num(toks[1])?, delta: num(toks[2])?, body: Body::Sql(unhex(toks[3])?), whole: None }),
            "seed" => probes.push(ProbeRule { head: num(toks[1])?, delta: SEED, body: Body::Join(parse_join(conn, &unhex(toks[2])?, &mut syms)?), whole: None }),
            "nowrite" => write = false,
            "gate" => {
                let list = |t: &str| -> Result<Vec<usize>> { if t == "-" { Ok(Vec::new()) } else { t.split(',').map(num).collect() } };
                let src = if toks[2] == "-" { None } else { Some(num(toks[2])?) };
                gates.push(Gate { done: num(toks[1])?, src, quiet: list(toks[3])?, after: list(toks[4])?, passed: 0 });
            }
            "join" => {
                let (head, delta) = (num(toks[1])?, num(toks[2])?);
                let whole = match toks.get(4) { Some(w) => Some(parse_join(conn, &unhex(w)?, &mut syms)?), None => None };
                probes.push(ProbeRule { head, delta, body: Body::Join(parse_join(conn, &unhex(toks[3])?, &mut syms)?), whole });
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
    syms.flush(conn)?;
    drop(syms);
    let store = Arc::new(RwLock::new(Store { rels, syms: symbols }));
    let mut reg = COMPONENTS.lock().unwrap();
    reg.retain(|(n, _)| *n != name);
    reg.push((name.clone(), Arc::new(Mutex::new(Component { store, edges, probes, gates, seeded: false, write }))));
    Ok(name)
}

pub fn close(name: &str) -> Result<()> {
    COMPONENTS.lock().unwrap().retain(|(n, _)| n != name);
    Ok(())
}

/// Runs waves until the fixpoint, or until `max_waves` more have run (0: no bound). Rows are written to the
/// relations' tables only at the fixpoint, so a bounded run can be resumed after the caller replans probes from
/// `stats`.
pub fn run(conn: &Connection, name: &str, max_waves: u32) -> Result<String> {
    let t0 = Instant::now();
    let comp = component(name)?;
    let mut comp = comp.lock().unwrap();
    let (nrels, symbols) = { let s = comp.store.read().unwrap(); (s.rels.len(), Arc::clone(&s.syms)) };
    symbols.write().unwrap().sync(conn)?;
    let mut stmts = Vec::new();
    for p in &comp.probes { stmts.push(match &p.body { Body::Sql(sql) => Some(conn.prepare(sql)?), Body::Join(_) => None }); }
    let (mut waves, mut visits, mut runs, mut skips) = (0u32, 0u64, 0u64, 0u64);
    let mut done = false;
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
                for (v, &ty) in vals.iter().zip(&types) { row.push(read_value(ValueRef::from(v), ty, &mut syms)?); }
                if !store.rels[head].contains(&row) { fresh[head].add(&row); }
            }
        }
        // Join probes, natively; seed joins in the first wave only.
        for (pi, p) in comp.probes.iter().enumerate() {
            let Body::Join(j) = &p.body else { continue };
            let tp = Instant::now();
            if p.delta == SEED { if seeded { continue } }
            else if store.rels[p.delta].lo == store.rels[p.delta].hi { skips += 1; continue; }
            let j = match &p.whole { Some(w) if p.delta != SEED && store.rels[p.delta].lo == 0 => w, _ => j };
            runs += 1;
            if wave_trace {
                let d = if p.delta == SEED { 0 } else { store.rels[p.delta].hi - store.rels[p.delta].lo };
                eprintln!("      wave {waves} probe {pi} starts, delta {d}");
            }
            let n = run_join(j, p.head, store, &mut syms, &mut fresh);
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
    // The component keeps the symbols its waves created, so they reach the table now, done or not.
    symbols.write().unwrap().flush(conn)?;
    let solve = t0.elapsed();
    let per: Vec<String> = per.iter().map(|(t, n)| format!("{:.1}:{n}", t * 1e3)).collect();
    if !done {
        return Ok(format!("0 {waves} {visits} {runs} {skips} {:.1} {:.1} {} 0", solve.as_secs_f64() * 1e3, t0.elapsed().as_secs_f64() * 1e3, per.join(",")));
    }
    let store = comp.store.read().unwrap();
    let mut new_rows = 0usize;
    // Every relation of the component, whole, is an input of later strata (`r:` sources).
    for rel in &store.rels {
        DERIVED.lock().unwrap().insert(rel.table.clone(), Arc::new(InRel { k: rel.k, n: rel.n, rows: rel.data.clone(), base: false, types: rel.types.clone(),
                                                                         stored: rel.stored, idx: Mutex::new(HashMap::default()) }));
    }
    for rel in store.rels.iter().filter(|_| comp.write) {
        let marks: Vec<String> = (1..=rel.types.len()).map(|i| format!("?{i}")).collect();
        let mut ins = conn.prepare(&format!("INSERT INTO {} VALUES ({})", rel.table, marks.join(", ")))?;
        for id in rel.stored..rel.n {
            ins.execute(rusqlite::params_from_iter(rel.row(id)))?;
            new_rows += 1;
        }
    }
    Ok(format!("{new_rows} {waves} {visits} {runs} {skips} {:.1} {:.1} {} 1", solve.as_secs_f64() * 1e3, t0.elapsed().as_secs_f64() * 1e3, per.join(",")))
}

/// Live statistics of a component's relations, one line per relation in spec order: its row count, then each
/// column's distinct count (its hash index's key count), then each column's sum of squared posting lengths (the
/// skew: sum/rows is the expected postings of a row's value, the planner's `fan`). What the planner lacks for a relation still being derived.
pub fn stats(name: &str) -> Result<String> {
    let comp = component(name)?;
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
                let text = if r.types[c] == b's' { syms.name(v).to_owned() } else { v.to_string() };
                format!("{}={}", text.bytes().map(|b| format!("{b:02x}")).collect::<String>(), p.len)
            }).collect();
            line.push_str(&format!(" |{c}:{}", vals.join(",")));
        }
        line
    }).collect::<Vec<_>>().join("\n"))
}

/// `set_fixpoint_steps(name)`: per probe line (spec order), its join steps' "in:out" counts since the last call,
/// comma-separated (empty for a SQL probe); the counts restart.
pub fn steps(name: &str) -> Result<String> {
    let comp = component(name)?;
    let comp = comp.lock().unwrap();
    Ok(comp.probes.iter().map(|p| match &p.body {
        Body::Join(j) => j.counts.iter().map(|[a, b]| format!("{}:{}", a.replace(0), b.replace(0))).collect::<Vec<_>>().join(","),
        Body::Sql(_) => String::new(),
    }).collect::<Vec<_>>().join("\n"))
}

/// Replaces probe `i`'s body, replanned from `stats`: `kind` is `sql` (hex SQL, prepared by the next `run`) or `join`
/// (a hex join spec).
pub fn reprobe(conn: &Connection, name: &str, i: usize, kind: &str, body: &str) -> Result<()> {
    let comp = component(name)?;
    let mut comp = comp.lock().unwrap();
    let n = comp.probes.len();
    let body = match kind {
        "sql" => Body::Sql(unhex(body)?),
        "join" => {
            let symbols = Arc::clone(&comp.store.read().unwrap().syms);
            let mut syms = symbols.write().unwrap();
            syms.sync(conn)?;
            let j = parse_join(conn, &unhex(body)?, &mut syms)?;
            syms.flush(conn)?;
            let mut store = comp.store.write().unwrap();
            for (r, cols) in &j.multi { store.rels[*r].add_multi(cols); }
            Body::Join(j)
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
    fn connect(_: &mut VTabConnection, _: Option<&()>, _: &[u8], _: &[u8], _: &[u8], args: &[&[u8]]) -> Result<(Cow<'static, CStr>, Self)> {
        let arg = |i: usize| std::str::from_utf8(args.get(i).copied().unwrap_or(b"")).unwrap_or("").trim().trim_matches('\'').to_string();
        let comp = component(&arg(0)).map_err(|e| Error::ModuleError(e.to_string()))?;
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
            if self.delta { self.ids.extend(r.lo as u32..r.hi as u32) } else { self.ids.extend(0..r.n as u32) }
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
        cands.any(|id| { if keys.iter().all(|&(c, k)| r.row(id as usize)[c] == k) { ids.push(id) } false });
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

fn cat(syms: &mut Interner, a: i64, b: i64) -> i64 {
    let mut s = std::mem::take(&mut syms.buf);
    s.clear();
    s.push_str(syms.name(a));
    s.push('.');
    s.push_str(syms.name(b));
    let id = syms.id(&s);
    syms.buf = s;
    id
}

/// This thread's CPU seconds.
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
pub struct InRel { k: usize, n: usize, rows: Vec<i64>, base: bool, types: Vec<u8>, stored: usize, idx: Mutex<HashMap<u64, Arc<Csr>>> }
/// A hash index of an immutable relation on `cols`: each key's row ids are one run of `ids`, found from the key's
/// first row (`rep`) and its run (`start`, `len`). Two passes over the rows, no allocation per key.
pub struct Csr { cols: Vec<usize>, table: HashTable<(u32, u32, u32)>, ids: Vec<u32> }
impl Csr {
    fn build(rel: &InRel, cols: &[usize]) -> Csr {
        let key = |id: usize| cols.iter().map(move |&c| rel.rows[id * rel.k + c]);
        let same = |a: usize, b: usize| cols.iter().all(|&c| rel.rows[a * rel.k + c] == rel.rows[b * rel.k + c]);
        let hashes: Vec<u64> = (0..rel.n).map(|id| hash_vals(key(id))).collect();
        let mut table: HashTable<(u32, u32, u32)> = HashTable::new();
        for id in 0..rel.n {
            match table.find_mut(hashes[id], |&(rep, _, _)| same(rep as usize, id)) {
                Some(e) => e.2 += 1,
                None => { table.insert_unique(hashes[id], (id as u32, 0, 1), |&(rep, _, _)| hashes[rep as usize]); }
            }
        }
        let mut at = 0;
        for e in table.iter_mut() { e.1 = at; at += e.2; e.2 = 0; }
        let mut ids = vec![0u32; rel.n];
        for id in 0..rel.n {
            let e = table.find_mut(hashes[id], |&(rep, _, _)| same(rep as usize, id)).expect("a key seen in the first pass");
            ids[(e.1 + e.2) as usize] = id as u32;
            e.2 += 1;
        }
        Csr { cols: cols.to_vec(), table, ids }
    }
    /// `key` holds the values of `cols`, in order.
    fn get<'a>(&'a self, rel: &InRel, key: &[i64]) -> &'a [u32] {
        self.table.find(hash_vals(key.iter().copied()), |&(rep, _, _)| self.cols.iter().zip(key).all(|(&c, &v)| rel.rows[rep as usize * rel.k + c] == v))
            .map_or(&[], |&(_, start, len)| &self.ids[start as usize..][..len as usize])
    }
}
/// The relations of components run with `nowrite`, by table, until they are materialized.
static DERIVED: LazyLock<Mutex<HashMap<String, Arc<InRel>>>> = LazyLock::new(Default::default);
const SEED: usize = usize::MAX;
/// Columns with at most this many values report each value's postings (`stats`).
const FREQ_NDV: usize = 64;

/// `set_fixpoint_materialize(table)`: inserts a native relation's rows (those not already in its table) into the
/// table, for SQL that reads it. Returns the rows written.
pub fn materialize(conn: &Connection, table: &str) -> Result<i64> {
    let Some(rel) = DERIVED.lock().unwrap().get(table).cloned() else { return Err(err(format!("no native relation {table}"))) };
    let marks: Vec<String> = (1..=rel.k).map(|i| format!("?{i}")).collect();
    let mut ins = conn.prepare(&format!("INSERT INTO {table} VALUES ({})", marks.join(", ")))?;
    for id in rel.stored..rel.len() {
        ins.execute(rusqlite::params_from_iter(rel.row(id)))?;
    }
    Ok((rel.len() - rel.stored) as i64)
}
impl InRel {
    fn len(&self) -> usize { self.n }
    fn row(&self, id: usize) -> &[i64] { &self.rows[id * self.k..][..self.k] }
    fn index(&self, cols: &[usize]) -> Arc<Csr> {
        let mask = cols.iter().fold(0u64, |m, &c| m | 1 << c);
        let mut idx = self.idx.lock().unwrap();
        idx.entry(mask).or_insert_with(|| {
            let (t0, c0) = (Instant::now(), thread_cpu());
            // No key columns: the step reads every row, not an index.
            let m = if cols.is_empty() { Csr { cols: Vec::new(), table: HashTable::new(), ids: Vec::new() } } else { Csr::build(self, cols) };
            let l = &mut LOADS.lock().unwrap()[usize::from(!self.base)];
            l.0 += thread_cpu() - c0;
            if std::env::var_os("RDL_PROFILE").is_some() {
                eprintln!("      index on {cols:?} of {} rows in {:.1} ms", self.len(), t0.elapsed().as_secs_f64() * 1e3);
            }
            Arc::new(m)
        }).clone()
    }
}
static INPUTS: LazyLock<Mutex<HashMap<String, Arc<InRel>>>> = LazyLock::new(Default::default);

fn input(conn: &Connection, spec: &str, syms: &mut Interner) -> Result<Arc<InRel>> {
    if let Some(r) = INPUTS.lock().unwrap().get(spec) { return Ok(r.clone()); }
    let parts: Vec<&str> = spec.splitn(4, ':').collect();
    let [which, table, cols, types] = parts[..] else { return Err(err(format!("bad input {spec}"))) };
    let base = which == "b";
    let (table, cols, types) = (unhex(table)?, unhex(cols)?, types.as_bytes());
    let (t0, c0) = (Instant::now(), thread_cpu());
    let mut st = conn.prepare(&format!("SELECT {cols} FROM {table}"))?;
    let mut q = st.query([])?;
    let (mut rows, mut n) = (Vec::new(), 0);
    while let Some(r) = q.next()? {
        for (i, &t) in types.iter().enumerate() { rows.push(read_value(r.get_ref(i)?, t, syms)?); }
        n += 1;
    }
    let rel = Arc::new(InRel { k: types.len(), n, rows, base, types: types.to_vec(), stored: 0, idx: Mutex::new(HashMap::default()) });
    {
        let l = &mut LOADS.lock().unwrap()[usize::from(!base)];
        l.0 += thread_cpu() - c0;
        l.1 += rel.len() as u64;
    }
    if std::env::var_os("RDL_PROFILE").is_some() {
        eprintln!("      input {} rows in {:.1} ms: SELECT {cols} FROM {table}", rel.len(), t0.elapsed().as_secs_f64() * 1e3);
    }
    INPUTS.lock().unwrap().insert(spec.to_string(), rel.clone());
    Ok(rel)
}

#[derive(Clone, Copy)]
enum Key { Slot(usize), Const(i64) }
enum Src { Delta(usize), Full(usize), FullMulti(usize, Vec<usize>), In(Arc<InRel>, Arc<Csr>) }
enum Step {
    /// Key columns (bound), columns that bind a slot, and column pairs that must be equal (a variable repeated in
    /// the atom, first seen here).
    Atom { src: Src, neg: bool, keys: Vec<(usize, Key)>, binds: Vec<(usize, usize)>, tests: Vec<(usize, usize)> },
    Cat { out: usize, a: Key, b: Key, bind: bool },
    /// `whole` split at each "." into `a` and `b`; `Some` slots bind, `None` parts (`ka`/`kb`) are tested.
    Split { whole: Key, a: Option<usize>, b: Option<usize>, ka: Key, kb: Key },
    Eq { a: Key, b: Key },
    Lt { a: Key, b: Key, or_eq: bool },
    Count { src: Src, keys: Vec<(usize, Key)>, tests: Vec<(usize, usize)>, out: usize, bind: bool },
    Let { a: usize, b: Key },
    Ne { a: Key, b: Key },
}
/// `multi`: the composite indexes (relation, key columns) its steps read, which the component maintains. `counts`:
/// per step, the bindings that entered it and those it passed on, since `set_fixpoint_steps` last read them: the
/// observed fanout of each step, which replanning uses in place of the estimate.
pub struct JoinRule { steps: Vec<Step>, out: Vec<Term>, nslots: usize, multi: Vec<(usize, Vec<usize>)>, counts: Vec<[std::cell::Cell<u64>; 2]> }

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
            "pos" | "neg" | "count" => {
                let (neg, count) = (toks[0] == "neg", toks[0] == "count");
                let atoks = if count { &toks[2..toks.len() - 1] } else { &toks[2..] };
                let (mut keys, mut binds, mut tests) = (Vec::new(), Vec::new(), Vec::new());
                for (col, t) in atoks.iter().enumerate() {
                    match tok(t, syms)? {
                        None => {}
                        Some(k) if is_bound(&bound, k) => keys.push((col, k)),
                        Some(Key::Slot(s)) => {
                            if neg || count { return Err(err(format!("unbound v{s} in a negated or counted atom"))); }
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
                        let rel = DERIVED.lock().unwrap().get(&table).cloned().ok_or_else(|| err(format!("no native relation {table}")))?;
                        let cols: Vec<usize> = keys.iter().map(|&(c, _)| c).collect();
                        let ix = rel.index(&cols);
                        Src::In(rel, ix)
                    }
                    t if t.starts_with("t:") => {
                        let rel = input(conn, &t[2..], syms)?;
                        let cols: Vec<usize> = keys.iter().map(|&(c, _)| c).collect();
                        let ix = rel.index(&cols);
                        Src::In(rel, ix)
                    }
                    t => return Err(err(format!("bad source {t}"))),
                };
                if count {
                    let Some(Key::Slot(o)) = tok(toks[toks.len() - 1], syms)? else { return Err(err("bad count")) };
                    let bind = !is_bound(&bound, Key::Slot(o));
                    grow(&mut bound, o); bound[o] = true;
                    steps.push(Step::Count { src, keys, tests, out: o, bind });
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
    let counts = steps.iter().map(|_| Default::default()).collect();
    Ok(JoinRule { steps, out, nslots: bound.len(), multi, counts })
}

/// Runs a join probe over the store; its new head tuples go to `fresh`. Returns the tuples it derived.
fn run_join(j: &JoinRule, head: usize, store: &Store, syms: &mut Interner, fresh: &mut [Fresh]) -> u64 {
    struct Ctx<'a> { j: &'a JoinRule, head: usize, store: &'a Store, syms: &'a mut Interner, fresh: &'a mut [Fresh], n: u64,
                     key: Vec<i64>, row: Vec<i64> }
    fn val(slots: &[i64], k: Key) -> i64 { match k { Key::Slot(s) => slots[s], Key::Const(c) => c } }
    /// Where an atom's candidate tuples are: an input relation or a component relation, by id.
    #[derive(Clone, Copy)]
    enum Rows<'a> { In(&'a InRel), Comp(&'a Rel) }
    impl<'a> Rows<'a> {
        fn row(self, id: u32) -> &'a [i64] { match self { Rows::In(r) => r.row(id as usize), Rows::Comp(r) => r.row(id as usize) } }
    }
    fn go<'a>(cx: &mut Ctx<'a>, i: usize, slots: &mut [i64]) {
        let (j, store): (&'a JoinRule, &'a Store) = (cx.j, cx.store);
        if i > 0 { let c = &j.counts[i - 1][1]; c.set(c.get() + 1); }
        if let Some(c) = j.counts.get(i) { c[0].set(c[0].get() + 1); }
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
                let found = cands.any(|id| {
                    let row = rows.row(id);
                    if !keys.iter().all(|&(c, k)| row[c] == val(slots, k)) || !tests.iter().all(|&(c, c0)| row[c] == row[c0]) { return false; }
                    if *neg { return true; }
                    for &(c, s) in binds { slots[s] = row[c]; }
                    go(cx, i + 1, slots);
                    false
                });
                if *neg && !found { go(cx, i + 1, slots); }
            }
            Step::Count { src, keys, tests, out, bind } => {
                cx.key.clear();
                cx.key.extend(keys.iter().map(|&(_, k)| val(slots, k)));
                let (rows, cands) = candidates(src, store, &cx.key, keys);
                let mut n = 0i64;
                cands.any(|id| {
                    let row = rows.row(id);
                    if keys.iter().all(|&(c, k)| row[c] == val(slots, k)) && tests.iter().all(|&(c, c0)| row[c] == row[c0]) { n += 1 }
                    false
                });
                if *bind { slots[*out] = n } else if slots[*out] != n { return }
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
                for (p, _) in w.match_indices('.') {
                    let (x, y) = (&w[..p], &w[p + 1..]);
                    match a { Some(s) => slots[*s] = cx.syms.id(x), None => if cx.syms.name(val(slots, *ka)) != x { continue } }
                    match b { Some(s) => slots[*s] = cx.syms.id(y), None => if cx.syms.name(val(slots, *kb)) != y { continue } }
                    go(cx, i + 1, slots);
                }
            }
            Step::Eq { a, b } => if val(slots, *a) == val(slots, *b) { go(cx, i + 1, slots) },
            Step::Ne { a, b } => if val(slots, *a) != val(slots, *b) { go(cx, i + 1, slots) },
            Step::Let { a, b } => { slots[*a] = val(slots, *b); go(cx, i + 1, slots) }
        }
    }
    let mut cx = Ctx { j, head, store, syms, fresh, n: 0, key: Vec::new(), row: Vec::new() };
    let mut slots = vec![0i64; j.nslots];
    go(&mut cx, 0, &mut slots);
    cx.n
}
