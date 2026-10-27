//! The logic's native fixpoints (05 Phase 6 row 1), registered on the store's connection.
//!
//! From Typing-support's `rdl/sqlite-ext` (T), where they were a loadable extension that rdl's SQL
//! backend called:
//! - `flat_join(v)` is the flat lattice's join as an aggregate; `flat_bin(op, a, b)` its arithmetic transfer.
//! - `lattice_fixpoint(p)` evaluates one flat-lattice component natively. SQL supplies its non-recursive parts as
//!   tables: `<p>_seed(c0 n, c1 v)`, `<p>_copy(c0 n, c1 src)` (n gets src's value) and `<p>_bin(c0 n, c1 op, c2 l,
//!   c3 r)` (n gets flat_bin(op, l, r)). With a second argument it writes the result straight into that table, or
//!   with `@name` keeps it in memory for the `lattice_result` virtual table (`CREATE VIRTUAL TABLE temp.cv USING
//!   lattice_result(name)`: rows (c0, c1), O(1) lookup on `c0 = ?`), otherwise writes `<p>_out(c0, c1)`. Returns
//!   "rows waves visits solve_ms total_ms". T's lattice was flat only; here a third argument names a declared
//!   `Lattice` (`flat`, `min`, `max`, or one the logic declares) and a fourth the changes after which a node widens,
//!   which a lattice of unbounded height requires. `flat` without `widen` is T's semantics, kept as the CPU
//!   binding and the reference for the transducer's file-local lattices.
//! - `set_fixpoint_open/run/close` and the `set_rel` vtab evaluate one component of set-valued relations natively
//!   (see `setfix.rs`).
//!
//! Not kept: T's `tape_load`/`tape_nearest` and per-kind seeds (`<p>_kseed`), which read a per-run copy of one
//! tree. Ancestor climbs and other file-local work run in the transducer (Phase 4 item f), whose facts arrive here
//! as seed rows.
mod setfix;
pub(in crate::code_rule_dsl) use setfix::{flush_symbols, register_symbols};
use rusqlite::ffi;
use rusqlite::functions::{Aggregate, Context, FunctionFlags};
use rusqlite::vtab::{Context as VContext, CreateVTab, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConnection, VTabCursor, VTabKind};
use rusqlite::{Connection, Error, Result};
use std::borrow::Cow;
use std::ffi::CStr;
use std::sync::Mutex;
use std::os::raw::c_int;
use std::sync::{Arc, LazyLock};
use std::time::Instant;

pub const TOP: i64 = 4611686018427387904;
const BOT: i64 = i64::MIN;
const BIG: i64 = 1 << 40;

fn user_err(m: String) -> Error { Error::UserFunctionError(m.into()) }

fn flat_bin(op: &str, a: i64, b: i64) -> i64 {
    if a == TOP || b == TOP { return TOP; }
    match op {
        "+" => a + b,
        "-" => a - b,
        "*" if a.abs() < BIG && b.abs() < BIG => a * b,
        _ => TOP,
    }
}
fn join(a: i64, b: i64) -> i64 { Flat.join(a, b) }

/// A lattice over `i64` values (integers or atoms) under which `lattice_fixpoint` runs (§12 `.decl … merge @join
/// widen k`). `BOT` is the absent value: `join` and `transfer` see it only through the provided methods.
pub(in crate::code_rule_dsl) trait Lattice: Send + Sync {
    /// The least upper bound of two present values.
    fn lub(&self, a: i64, b: i64) -> i64;
    /// The greatest value, which absorbs every transfer.
    fn top(&self) -> i64;
    /// Whether every ascending chain is finite; one that is not runs only with `widen`.
    fn finite(&self) -> bool;
    /// Replaces `new`, the next value of a node that has changed `k` times already, by one on a finite chain above
    /// `old`. Finite lattices keep `new`.
    fn widen(&self, old: i64, new: i64) -> i64 {
        let _ = old;
        new
    }
    fn join(&self, a: i64, b: i64) -> i64 {
        if a == BOT { b } else if b == BOT { a } else { self.lub(a, b) }
    }
    /// The arithmetic transfer `flat_bin` names by `op`, with `top` absorbing.
    fn transfer(&self, op: u8, a: i64, b: i64) -> i64 {
        let top = self.top();
        if a == top || b == top { return top; }
        match op {
            b'+' => a + b,
            b'-' => a - b,
            b'*' if a.abs() < BIG && b.abs() < BIG => a * b,
            _ => top,
        }
    }
}

/// T's flat lattice: one value, or TOP when two differ. Height 3.
pub(in crate::code_rule_dsl) struct Flat;
impl Lattice for Flat {
    fn lub(&self, a: i64, b: i64) -> i64 { if a == b { a } else { TOP } }
    fn top(&self) -> i64 { TOP }
    fn finite(&self) -> bool { true }
}

/// The greatest value seen; unbounded, so it widens to TOP.
pub(in crate::code_rule_dsl) struct Max;
impl Lattice for Max {
    fn lub(&self, a: i64, b: i64) -> i64 { a.max(b) }
    fn top(&self) -> i64 { TOP }
    fn finite(&self) -> bool { false }
    fn widen(&self, old: i64, new: i64) -> i64 { if new > old { TOP } else { new } }
}

/// The least value seen (the order is reversed: smaller is higher); unbounded, so it widens to its top, -TOP.
pub(in crate::code_rule_dsl) struct Min;
impl Lattice for Min {
    fn lub(&self, a: i64, b: i64) -> i64 { a.min(b) }
    fn top(&self) -> i64 { -TOP }
    fn finite(&self) -> bool { false }
    fn widen(&self, old: i64, new: i64) -> i64 { if new < old { -TOP } else { new } }
}

/// Declared lattices by name: the built-ins, and those the logic declares (`declare`), such as the typing's
/// approximate type terms (Phase 10).
static LATTICES: LazyLock<Mutex<Vec<(String, Arc<dyn Lattice>)>>> = LazyLock::new(|| {
    Mutex::new(vec![("flat".to_owned(), Arc::new(Flat) as Arc<dyn Lattice>), ("max".to_owned(), Arc::new(Max)), ("min".to_owned(), Arc::new(Min))])
});

/// Declares `lattice` under `name`, replacing an earlier declaration.
pub(in crate::code_rule_dsl) fn declare(name: &str, lattice: Arc<dyn Lattice>) {
    let mut lattices = LATTICES.lock().unwrap();
    lattices.retain(|(n, _)| n != name);
    lattices.push((name.to_owned(), lattice));
}

fn lattice(name: &str) -> Result<Arc<dyn Lattice>> {
    LATTICES.lock().unwrap().iter().find(|(n, _)| n == name).map(|(_, l)| Arc::clone(l))
        .ok_or_else(|| user_err(format!("no lattice named {name:?}")))
}

struct FlatJoin;
impl Aggregate<i64, Option<i64>> for FlatJoin {
    fn init(&self, _: &mut Context<'_>) -> Result<i64> { Ok(BOT) }
    fn step(&self, ctx: &mut Context<'_>, acc: &mut i64) -> Result<()> { *acc = join(*acc, ctx.get(0)?); Ok(()) }
    fn finalize(&self, _: &mut Context<'_>, acc: Option<i64>) -> Result<Option<i64>> { Ok(acc.filter(|&v| v != BOT)) }
}

/// Semi-naive waves over the component's graph: a wave re-evaluates the dependents of the nodes that changed in the
/// previous one, each from all of its inputs (the seed, copy sources and binary transfers), in `lat`. A node that
/// has changed `widen` times takes `lat.widen` of its next value; a lattice of unbounded height needs one.
fn lattice_fixpoint(conn: &Connection, p: &str, target: Option<&str>, lat: &dyn Lattice, widen: Option<u32>) -> Result<String> {
    if !lat.finite() && widen.is_none() {
        return Err(user_err(format!("unbounded lattice for {p}: declare `widen k`")));
    }
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
    let n = seeds.iter().map(|r| r.0).chain(copies.iter().flat_map(|r| [r.0, r.1]))
        .chain(bins.iter().flat_map(|r| [r.0, r.2, r.3])).max().map_or(0, |m| m as usize + 1);
    let mut seed = vec![BOT; n];
    for &(k, v) in &seeds { seed[k as usize] = lat.join(seed[k as usize], v); }
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
        for &s in &cin[cin_s[v] as usize..cin_s[v + 1] as usize] { x = lat.join(x, val[s as usize]); }
        for &bi in &bin_i[bin_s[v] as usize..bin_s[v + 1] as usize] {
            let (_, o, l, r) = bins[bi as usize];
            let (a, b) = (val[l as usize], val[r as usize]);
            if absorb && (a == lat.top() || b == lat.top()) {
                // TOP absorbs: the transfer's result is known before the other input arrives. A Datalog join waits
                // for both; this differs only for keys whose other input never gets a value.
                x = lat.join(x, lat.top());
            } else if a != BOT && b != BOT {
                x = lat.join(x, lat.transfer(o, a, b));
            }
        }
        x
    };
    let mut val = seed.clone();
    let mut frontier: Vec<u32> = (0..n as u32).filter(|&v| val[v as usize] != BOT).collect();
    let (mut stamp, mut waves, mut visits) = (vec![0u32; n], 0u32, 0u64);
    let mut changes = vec![0u32; if widen.is_some() { n } else { 0 }];
    while !frontier.is_empty() {
        waves += 1;
        let mut cand = Vec::new();
        for &u in &frontier {
            for &v in &out[out_s[u as usize] as usize..out_s[u as usize + 1] as usize] {
                if stamp[v as usize] != waves { stamp[v as usize] = waves; cand.push(v); }
            }
        }
        visits += cand.len() as u64;
        let upd: Vec<(u32, i64)> = cand.iter().filter_map(|&v| {
            let (v, old) = (v as usize, val[v as usize]);
            let mut x = eval(v, &val);
            if let Some(k) = widen && x != old {
                if changes[v] >= k && old != BOT { x = lat.widen(old, x) }
                changes[v] += 1;
            }
            (x != old).then_some((v as u32, x))
        }).collect();
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

/// Registers the functions and virtual tables on `db`.
pub(in crate::code_rule_dsl) fn register(db: &Connection) -> Result<()> {
    let det = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC;
    db.create_scalar_function(c"flat_bin", 3, det, |ctx| {
        let o: String = ctx.get(0)?;
        Ok(flat_bin(&o, ctx.get(1)?, ctx.get(2)?))
    })?;
    db.create_aggregate_function(c"flat_join", 1, det, FlatJoin)?;
    // lattice_fixpoint(p[, target[, lattice[, widen]]]): the lattice by declared name (`flat` by default), and the
    // changes after which a node widens. A NULL target is the default `<p>_out`.
    for nargs in [1, 2, 3, 4] {
        db.create_scalar_function(c"lattice_fixpoint", nargs, FunctionFlags::SQLITE_UTF8, |ctx| {
            let p: String = ctx.get(0)?;
            let target: Option<String> = if ctx.len() > 1 { ctx.get(1)? } else { None };
            let lat = lattice(&if ctx.len() > 2 { ctx.get::<String>(2)? } else { "flat".to_owned() })?;
            let widen: Option<i64> = if ctx.len() > 3 { ctx.get(3)? } else { None };
            let conn = unsafe { ctx.get_connection()? };
            lattice_fixpoint(&conn, &p, target.as_deref(), &*lat, widen.map(|k| k.max(0) as u32))
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
    Ok(())
}

#[cfg(test)]
mod tests;
