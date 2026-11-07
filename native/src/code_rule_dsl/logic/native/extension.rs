//! Native pieces for rdl's SQL backend, as a SQLite loadable extension:
//!
//! - `tape_load(schema)` reads `<schema>.tape` and `<schema>.node` once; `tape_nearest(n, 'k1,k2,..')` is `n`'s
//!   nearest strict ancestor whose kind is in the set (NULL if none). It replaces the recursive-CTE climb.
//! - `flat_join(v)` is the flat lattice's join as an aggregate; `flat_bin(op, a, b)` its arithmetic transfer.
//! - `lattice_fixpoint(p)` evaluates one flat-lattice component natively. SQL supplies its non-recursive parts as
//!   tables: `<p>_seed(c0 n, c1 v)`, `<p>_copy(c0 n, c1 src)` (n gets src's value) and `<p>_bin(c0 n, c1 op, c2 l,
//!   c3 r)` (n gets flat_bin(op, l, r)). Optionally `<p>_kseed(c0 kind, c1 v)`: every node of that kind (from the
//!   loaded tape) is seeded with v, so per-kind defaults need no row per node. With a second argument it writes the
//!   result straight into that table, or with `@name` keeps it in memory for the `lattice_result` virtual table
//!   (`CREATE VIRTUAL TABLE temp.cv USING lattice_result(name)`: rows (c0, c1), O(1) lookup on `c0 = ?`), otherwise
//!   writes `<p>_out(c0, c1)`. Returns "rows waves visits solve_ms total_ms".
//! - `set_fixpoint_open/run/close` and the `set_rel` vtab evaluate one component of set-valued relations natively
//!   (see `setfix.rs`).
// T's own set engine as copied (`extension/setfix.rs`); `logic::native::setfix` is its adaptation (O19).
mod setfix;
use rusqlite::ffi;
use rusqlite::functions::{Aggregate, Context, FunctionFlags};
use rusqlite::vtab::{Context as VContext, CreateVTab, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConnection, VTabCursor, VTabKind};
use rusqlite::{Connection, Error, Result};
use std::borrow::Cow;
use std::ffi::CStr;
use std::sync::Mutex;
use std::collections::HashMap;
use std::os::raw::{c_char, c_int};
use std::sync::{Arc, RwLock};
use std::time::Instant;

pub const TOP: i64 = 4611686018427387904;
const BOT: i64 = i64::MIN;
const BIG: i64 = 1 << 40;

/// The tree in preorder: each node's parent, kind and descendant count (its subtree is `n+1..=n+size`).
struct Tape { par: Vec<i32>, kind: Vec<u16>, size: Vec<u32>, kinds: HashMap<String, u16> }
static TAPE: RwLock<Option<Arc<Tape>>> = RwLock::new(None);

fn user_err(m: String) -> Error { Error::UserFunctionError(m.into()) }

fn tape_load(conn: &Connection, schema: &str) -> Result<usize> {
    let mut kinds: HashMap<String, u16> = HashMap::new();
    let (mut par, mut kind) = (Vec::new(), Vec::new());
    let mut st = conn.prepare(&format!("SELECT n, kind FROM {schema}.node"))?;
    let mut rows = st.query([])?;
    while let Some(r) = rows.next()? {
        let n: i64 = r.get(0)?;
        let k: String = r.get(1)?;
        let next = kinds.len() as u16;
        let id = *kinds.entry(k).or_insert(next);
        if kind.len() <= n as usize { kind.resize(n as usize + 1, u16::MAX); par.resize(n as usize + 1, -1); }
        kind[n as usize] = id;
    }
    let mut size = vec![0u32; par.len()];
    let mut st = conn.prepare(&format!("SELECT n, par, size FROM {schema}.tape"))?;
    let mut rows = st.query([])?;
    while let Some(r) = rows.next()? {
        let (n, p, z): (i64, i64, i64) = (r.get(0)?, r.get(1)?, r.get(2)?);
        par[n as usize] = p as i32;
        size[n as usize] = z as u32;
    }
    let n = par.len();
    *TAPE.write().unwrap() = Some(Arc::new(Tape { par, kind, size, kinds }));
    Ok(n)
}

fn flat_bin(op: &str, a: i64, b: i64) -> i64 {
    if a == TOP || b == TOP { return TOP; }
    match op {
        "+" => a + b,
        "-" => a - b,
        "*" if a.abs() < BIG && b.abs() < BIG => a * b,
        _ => TOP,
    }
}
fn join(a: i64, b: i64) -> i64 { if a == BOT { b } else if b == BOT || a == b { a } else { TOP } }

struct FlatJoin;
impl Aggregate<i64, Option<i64>> for FlatJoin {
    fn init(&self, _: &mut Context<'_>) -> Result<i64> { Ok(BOT) }
    fn step(&self, ctx: &mut Context<'_>, acc: &mut i64) -> Result<()> { *acc = join(*acc, ctx.get(0)?); Ok(()) }
    fn finalize(&self, _: &mut Context<'_>, acc: Option<i64>) -> Result<Option<i64>> { Ok(acc.filter(|&v| v != BOT)) }
}

/// Semi-naive waves over the component's graph: a wave re-evaluates the dependents of the nodes that changed in the
/// previous one, each from all of its inputs (the seed, copy sources and binary transfers).
fn lattice_fixpoint(conn: &Connection, p: &str, target: Option<&str>) -> Result<String> {
    let t = Instant::now();
    let pairs = |sql: String| -> Result<Vec<(i64, i64)>> {
        let mut st = conn.prepare(&sql)?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    };
    let seeds = pairs(format!("SELECT c0, c1 FROM {p}_seed"))?;
    let copies = pairs(format!("SELECT c0, c1 FROM {p}_copy"))?;
    let mut bins: Vec<(i64, u8, i64, i64)> = Vec::new();
    {
        let mut st = conn.prepare(&format!("SELECT c0, c1, c2, c3 FROM {p}_bin"))?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let o: String = r.get(1)?;
            let op = match o.as_str() { "+" => b'+', "-" => b'-', "*" => b'*', _ => b'?' };   // `**` is not `*`
            bins.push((r.get(0)?, op, r.get(2)?, r.get(3)?));
        }
    }
    let kseed: Vec<(String, i64)> = if conn.prepare(&format!("SELECT 1 FROM {p}_kseed LIMIT 0")).is_ok() {
        let mut st = conn.prepare(&format!("SELECT c0, c1 FROM {p}_kseed"))?;
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_>>()?
    } else { Vec::new() };
    let tape = if kseed.is_empty() { None } else { Some(TAPE.read().unwrap().clone().ok_or_else(|| user_err("tape_load first".into()))?) };
    let mut n = seeds.iter().map(|r| r.0).chain(copies.iter().flat_map(|r| [r.0, r.1]))
        .chain(bins.iter().flat_map(|r| [r.0, r.2, r.3])).max().map_or(0, |m| m as usize + 1);
    if let Some(t) = &tape { n = n.max(t.kind.len()); }
    let mut seed = vec![BOT; n];
    if let Some(t) = &tape {
        let mut by_kind = vec![BOT; t.kinds.len()];
        for (k, v) in &kseed { if let Some(&id) = t.kinds.get(k) { by_kind[id as usize] = join(by_kind[id as usize], *v); } }
        for (v, &k) in t.kind.iter().enumerate() { if k != u16::MAX { seed[v] = by_kind[k as usize]; } }
    }
    for &(k, v) in &seeds { seed[k as usize] = join(seed[k as usize], v); }
    // In-edges per node (to evaluate it) and out-edges (to find dependents), as CSR.
    let csr = |edges: &mut Vec<(u32, u32)>| -> (Vec<u32>, Vec<u32>) {
        edges.sort_unstable();
        let mut start = vec![0u32; n + 1];
        for &(a, _) in edges.iter() { start[a as usize + 1] += 1; }
        for i in 0..n { start[i + 1] += start[i]; }
        (start, edges.iter().map(|e| e.1).collect())
    };
    let (cin_s, cin) = csr(&mut copies.iter().map(|&(a, b)| (a as u32, b as u32)).collect());
    let (bin_s, bin_i) = csr(&mut bins.iter().enumerate().map(|(i, b)| (b.0 as u32, i as u32)).collect());
    let mut outs: Vec<(u32, u32)> = copies.iter().map(|&(a, b)| (b as u32, a as u32)).collect();
    outs.extend(bins.iter().flat_map(|b| [(b.2 as u32, b.0 as u32), (b.3 as u32, b.0 as u32)]));
    let (out_s, out) = csr(&mut outs);
    let absorb = std::env::var("RDL_ABSORB").is_ok();
    let eval = |v: usize, val: &[i64]| -> i64 {
        let mut x = seed[v];
        for &s in &cin[cin_s[v] as usize..cin_s[v + 1] as usize] { x = join(x, val[s as usize]); }
        for &bi in &bin_i[bin_s[v] as usize..bin_s[v + 1] as usize] {
            let (_, o, l, r) = bins[bi as usize];
            let (a, b) = (val[l as usize], val[r as usize]);
            if absorb && (a == TOP || b == TOP) {
                // TOP absorbs: the transfer's result is known before the other input arrives. A Datalog join waits
                // for both; this differs only for keys whose other input never gets a value.
                x = join(x, TOP);
            } else if a != BOT && b != BOT {
                let op = match o { b'+' => "+", b'-' => "-", b'*' => "*", _ => "?" };
                x = join(x, flat_bin(op, a, b));
            }
        }
        x
    };
    let mut val = seed.clone();
    let mut frontier: Vec<u32> = (0..n as u32).filter(|&v| val[v as usize] != BOT).collect();
    let (mut stamp, mut waves, mut visits) = (vec![0u32; n], 0u32, 0u64);
    while !frontier.is_empty() {
        waves += 1;
        let mut cand = Vec::new();
        for &u in &frontier {
            for &v in &out[out_s[u as usize] as usize..out_s[u as usize + 1] as usize] {
                if stamp[v as usize] != waves { stamp[v as usize] = waves; cand.push(v); }
            }
        }
        visits += cand.len() as u64;
        let upd: Vec<(u32, i64)> = cand.iter().filter_map(|&v| { let x = eval(v as usize, &val); (x != val[v as usize]).then_some((v, x)) }).collect();
        frontier = upd.iter().map(|u| u.0).collect();
        for (v, x) in upd { val[v as usize] = x; }
    }
    let solve = t.elapsed();
    if let Some(name) = target.and_then(|t| t.strip_prefix('@')) {
        let rows = val.iter().filter(|&&v| v != BOT).count();
        let mut reg = RESULTS.lock().unwrap();
        reg.retain(|(n, _)| n != name);
        reg.push((name.to_string(), Arc::new(val)));
        return Ok(format!("{rows} {waves} {visits} {:.1} {:.1}", solve.as_secs_f64() * 1e3, t.elapsed().as_secs_f64() * 1e3));
    }
    let out_table = match target {
        Some(t) => t.to_string(),
        None => {
            conn.execute_batch(&format!("DROP TABLE IF EXISTS {p}_out; CREATE TEMP TABLE {p}_out(c0 INTEGER PRIMARY KEY, c1 INTEGER)"))?;
            format!("{p}_out")
        }
    };
    let mut rows = 0usize;
    {
        let mut ins = conn.prepare(&format!("INSERT INTO {out_table} VALUES (?1, ?2)"))?;
        for (k, &v) in val.iter().enumerate() {
            if v != BOT { ins.execute((k as i64, v))?; rows += 1; }
        }
    }
    Ok(format!("{rows} {waves} {visits} {:.1} {:.1}", solve.as_secs_f64() * 1e3, t.elapsed().as_secs_f64() * 1e3))
}

/// Fixpoint results kept in memory, by name, for `lattice_result`.
static RESULTS: Mutex<Vec<(String, Arc<Vec<i64>>)>> = Mutex::new(Vec::new());

#[repr(C)]
struct ResultTab { base: ffi::sqlite3_vtab, vals: Arc<Vec<i64>>, rows: usize }

unsafe impl<'vtab> VTab<'vtab> for ResultTab {
    type Aux = ();
    type Cursor = ResultCursor;
    fn connect(_: &mut VTabConnection, _: Option<&()>, _: &[u8], _: &[u8], _: &[u8], args: &[&[u8]]) -> Result<(Cow<'static, CStr>, Self)> {
        let name = std::str::from_utf8(args.last().copied().unwrap_or(b"")).unwrap_or("").trim().to_string();
        let vals = RESULTS.lock().unwrap().iter().find(|(n, _)| *n == name).map(|r| r.1.clone())
            .ok_or_else(|| Error::ModuleError(format!("no lattice result named {name:?}")))?;
        let rows = vals.iter().filter(|&&v| v != BOT).count();
        Ok((Cow::Borrowed(c"CREATE TABLE x(c0 INTEGER, c1 INTEGER)"), ResultTab { base: ffi::sqlite3_vtab::default(), vals, rows }))
    }
    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let eq = info.constraints().position(|c| c.column() == 0 && c.is_usable() && c.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ);
        if let Some(i) = eq {
            let mut u = info.constraint_usage(i);
            u.set_argv_index(1);
            u.set_omit(true);
            info.set_idx_num(1);
            info.set_estimated_cost(1.0);
            info.set_estimated_rows(1);
        } else {
            info.set_idx_num(0);
            info.set_estimated_cost(self.rows as f64);
            info.set_estimated_rows(self.rows as i64);
        }
        Ok(true)
    }
    fn open(&mut self) -> Result<ResultCursor> {
        Ok(ResultCursor { base: ffi::sqlite3_vtab_cursor::default(), vals: self.vals.clone(), pos: 0, end: 0 })
    }
}
impl CreateVTab<'_> for ResultTab { const KIND: VTabKind = VTabKind::Default; }

#[repr(C)]
struct ResultCursor { base: ffi::sqlite3_vtab_cursor, vals: Arc<Vec<i64>>, pos: usize, end: usize }
impl ResultCursor {
    fn skip(&mut self) { while self.pos < self.end && self.vals[self.pos] == BOT { self.pos += 1; } }
}
unsafe impl VTabCursor for ResultCursor {
    fn filter(&mut self, idx_num: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        if idx_num == 1 {
            match args.get::<Option<i64>>(0)? {
                Some(k) if k >= 0 && (k as usize) < self.vals.len() => { self.pos = k as usize; self.end = k as usize + 1; }
                _ => { self.pos = 0; self.end = 0; }
            }
        } else {
            self.pos = 0; self.end = self.vals.len();
        }
        self.skip();
        Ok(())
    }
    fn next(&mut self) -> Result<()> { self.pos += 1; self.skip(); Ok(()) }
    fn eof(&self) -> bool { self.pos >= self.end }
    fn column(&self, ctx: &mut VContext, i: c_int) -> Result<()> {
        if i == 0 { ctx.set_result(&(self.pos as i64)) } else { ctx.set_result(&self.vals[self.pos]) }
    }
    fn rowid(&self) -> Result<i64> { Ok(self.pos as i64) }
}

/// # Safety
/// Called by SQLite's extension loader.
// W links SQLite bundled, so rusqlite's `loadable_extension` (and `extension_init2`) is not
// available, and the symbol is not exported from the addon: the entry point registers on the
// handle of a connection of the same SQLite (O19).
pub unsafe extern "C" fn sqlite3_extension_init(db: *mut ffi::sqlite3, err: *mut *mut c_char, api: *mut ffi::sqlite3_api_routines) -> c_int {
    let _ = (err, api);
    match unsafe { Connection::from_handle(db) }.and_then(init) {
        Ok(_) => ffi::SQLITE_OK,
        Err(_) => ffi::SQLITE_ERROR,
    }
}

fn init(db: Connection) -> Result<bool> {
    let det = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC;
    db.create_scalar_function(c"tape_load", 1, FunctionFlags::SQLITE_UTF8, |ctx| {
        let schema: String = ctx.get(0)?;
        let conn = unsafe { ctx.get_connection()? };
        tape_load(&conn, &schema).map(|n| n as i64)
    })?;
    db.create_scalar_function(c"tape_nearest", 2, det, |ctx| {
        let tape = TAPE.read().unwrap().clone().ok_or_else(|| user_err("tape_load first".into()))?;
        let t2 = tape.clone();
        let set = ctx.get_or_create_aux(1, move |v| -> Result<Vec<bool>, Error> {
            let mut m = vec![false; t2.kinds.len()];
            for k in v.as_str()?.split(',') { if let Some(&id) = t2.kinds.get(k) { m[id as usize] = true; } }
            Ok(m)
        })?;
        let n: i64 = ctx.get(0)?;
        let mut cur = tape.par[n as usize];
        while cur >= 0 {
            if set[tape.kind[cur as usize] as usize] { return Ok(Some(cur as i64)); }
            cur = tape.par[cur as usize];
        }
        Ok(None)
    })?;
    db.create_scalar_function(c"flat_bin", 3, det, |ctx| {
        let o: String = ctx.get(0)?;
        Ok(flat_bin(&o, ctx.get(1)?, ctx.get(2)?))
    })?;
    db.create_aggregate_function(c"flat_join", 1, det, FlatJoin)?;
    for nargs in [1, 2] {
        db.create_scalar_function(c"lattice_fixpoint", nargs, FunctionFlags::SQLITE_UTF8, |ctx| {
            let p: String = ctx.get(0)?;
            let target: Option<String> = if ctx.len() > 1 { Some(ctx.get(1)?) } else { None };
            let conn = unsafe { ctx.get_connection()? };
            lattice_fixpoint(&conn, &p, target.as_deref())
        })?;
    }
    db.create_scalar_function(c"set_fixpoint_open", 1, FunctionFlags::SQLITE_UTF8, |ctx| {
        let spec: String = ctx.get(0)?;
        let conn = unsafe { ctx.get_connection()? };
        setfix::open(&conn, &spec)
    })?;
    for n in [1, 2] {
        db.create_scalar_function(c"set_fixpoint_run", n, FunctionFlags::SQLITE_UTF8, |ctx| {
            let name: String = ctx.get(0)?;
            let max_waves: i64 = if ctx.len() > 1 { ctx.get(1)? } else { 0 };
            let conn = unsafe { ctx.get_connection()? };
            setfix::run(&conn, &name, max_waves as u32)
        })?;
    }
    db.create_scalar_function(c"set_fixpoint_materialize", 1, FunctionFlags::SQLITE_UTF8, |ctx| {
        let table: String = ctx.get(0)?;
        let conn = unsafe { ctx.get_connection()? };
        setfix::materialize(&conn, &table)
    })?;
    db.create_scalar_function(c"set_fixpoint_steps", 1, FunctionFlags::SQLITE_UTF8, |ctx| {
        let name: String = ctx.get(0)?;
        setfix::steps(&name)
    })?;
    db.create_scalar_function(c"set_fixpoint_loads", 0, FunctionFlags::SQLITE_UTF8, |_| Ok(setfix::loads()))?;
    db.create_scalar_function(c"set_fixpoint_stats", 1, FunctionFlags::SQLITE_UTF8, |ctx| {
        let name: String = ctx.get(0)?;
        setfix::stats(&name)
    })?;
    db.create_scalar_function(c"set_fixpoint_reprobe", 4, FunctionFlags::SQLITE_UTF8, |ctx| {
        let (name, i, kind, body): (String, i64, String, String) = (ctx.get(0)?, ctx.get(1)?, ctx.get(2)?, ctx.get(3)?);
        let conn = unsafe { ctx.get_connection()? };
        setfix::reprobe(&conn, &name, i as usize, &kind, &body).map(|_| 1i64)
    })?;
    db.create_scalar_function(c"set_fixpoint_close", 1, FunctionFlags::SQLITE_UTF8, |ctx| {
        let name: String = ctx.get(0)?;
        setfix::close(&name).map(|_| 1i64)
    })?;
    const SETREL: Module<setfix::RelTab> = Module::read_only_module();
    db.create_module(c"set_rel", &SETREL, None)?;
    const RESULT: Module<ResultTab> = Module::read_only_module();
    db.create_module(c"lattice_result", &RESULT, None)?;
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The entry point registers T's functions; `tape_nearest` climbs the loaded tape.
    #[test]
    fn extension_entry_registers_tape_nearest_over_a_loaded_tape() {
        let conn = Connection::open_in_memory().unwrap();
        assert_eq!(
            unsafe { sqlite3_extension_init(conn.handle(), std::ptr::null_mut(), std::ptr::null_mut()) },
            ffi::SQLITE_OK
        );
        conn.execute_batch(
            "CREATE TABLE node(n INTEGER, kind TEXT); CREATE TABLE tape(n INTEGER, par INTEGER, size INTEGER);
             INSERT INTO node VALUES (0,'module'),(1,'function'),(2,'call'),(3,'name');
             INSERT INTO tape VALUES (0,-1,3),(1,0,2),(2,1,1),(3,2,0);",
        )
        .unwrap();
        let n: i64 = conn.query_row("SELECT tape_load('main')", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 4);
        let near = |n: i64, kinds: &str| -> Option<i64> {
            conn.query_row("SELECT tape_nearest(?1, ?2)", rusqlite::params![n, kinds], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(near(3, "function"), Some(1));
        assert_eq!(near(3, "call,module"), Some(2));
        assert_eq!(near(0, "module"), None);
    }
}
