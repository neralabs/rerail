//! A logic program over a corpus on disk, as T ran its programs (list step 23, the Phase 6 check; the
//! `rerail-engine logic` tool).
//!
//! The corpus is parsed into core facts (`facts::core`), bound as T's base relations (`facts::vtab::syntax`), with
//! `nearest` from the transducer's file-local FIX (`typing::file_local`, O12), and the lowered program runs in a native
//! mode over atoms (`run_atoms_native`, O13). Its outputs are written as T's `rdl.py sql` writes them: one sorted,
//! tab-separated `<relation>.csv` per output, symbols as text and node ids as T's.
//!
//! Two things are read from T's own `facts.sqlite` for the same corpus, until the typing programs have their own
//! ingestion (list step 29): the planner's statistics (`stats.py build`'s tables, atomized as the program is:
//! `atomized_statistics`), and each document's first node id, which maps a node `d << 32 | index` to T's global id.
//! T's documents (`doc`) name the files, in T's order.

use super::{Error, Outcome, Result, lower};

use crate::code_rule_dsl::facts::core::{self, Built};
use crate::code_rule_dsl::facts::vtab::{Modules, syntax};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::logic::front;
use crate::code_rule_dsl::model::FileInput;
use crate::code_rule_dsl::typing::file_local::Binding;
use crate::content_store::ContentStore;
use rusqlite::Connection;
use rusqlite::types::Value as SqlValue;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

fn text(v: &SqlValue) -> String {
    super::text(v)
}

fn int(v: &SqlValue) -> i64 {
    super::int(v)
}

/// T's statistics (`stats.py build`'s tables in `t`) with T's strings lowered as the program's are (decision 0009)
/// and T's node ids as the vtabs' (`first`: each document's first id in T's facts), in schema `s` of `db`:
/// `stat_kind`/`stat_edge` by kind atoms, `stat_val` by its column's domain, `stat_docrange` as id intervals.
pub(in crate::code_rule_dsl) fn atomized_statistics(t: &Connection, db: &Connection, store: &ContentStore, first: &HashMap<i64, i64>, dialect: GrammarDialect) -> Result<()> {
    use lower::Domain;
    let tag = crate::code_rule_dsl::language::tape_tag(dialect).ok_or_else(|| Error::Plan("the dialect has no tape tag".to_owned()))?;
    let kinds = dialect.kinds();
    let kind = |s: &str| i64::from(crate::code_rule_dsl::facts::core::kind_atom(tag, kinds.kind_id(s)));
    let field = |s: &str| kinds.field_id(s).map_or(-1, |f| i64::from(syntax::field_atom(tag, f)));
    let domain = |tbl: &str, col: &str| match (tbl, col) {
        ("node", "kind") => Domain::Kind,
        ("field", "f") => Domain::Field,
        ("text" | "op", "s") | ("doc", "path" | "module") | ("pkg", "p") => Domain::Name,
        _ => Domain::Int,
    };
    db.execute_batch(
        "ATTACH ':memory:' AS s;
         CREATE TABLE s.stat_meta(k TEXT PRIMARY KEY, v INTEGER);
         CREATE TABLE s.stat_col(tbl TEXT, col TEXT, rows INTEGER, ndv INTEGER, sq INTEGER, sqd INTEGER, PRIMARY KEY (tbl, col));
         CREATE TABLE s.stat_kind(kind INTEGER PRIMARY KEY, n INTEGER, size INTEGER);
         CREATE TABLE s.stat_edge(pkind INTEGER, ckind INTEGER, field, n INTEGER);
         CREATE TABLE s.stat_val(tbl TEXT, col TEXT, val, n INTEGER, docs INTEGER, sqd INTEGER, PRIMARY KEY (tbl, col, val));
         CREATE TABLE s.stat_docrange(d INTEGER PRIMARY KEY, lo INTEGER, hi INTEGER);",
    )?;
    let rows = |sql: &str| -> Result<Vec<Vec<SqlValue>>> {
        let mut st = t.prepare(sql)?;
        let width = st.column_count();
        let rows = st.query_map([], |r| (0..width).map(|i| r.get::<_, SqlValue>(i)).collect())?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    };
    // Rows per table, inserted many per statement at the end.
    let pending: std::cell::RefCell<BTreeMap<String, Vec<Vec<SqlValue>>>> = Default::default();
    let insert = |table: &str, row: Vec<SqlValue>| -> Result<()> {
        pending.borrow_mut().entry(format!("INSERT INTO s.{table}")).or_default().push(row);
        Ok(())
    };
    db.execute_batch("BEGIN")?;
    for table in ["stat_meta", "stat_col"] {
        for r in rows(&format!("SELECT * FROM {table}"))? {
            insert(table, r)?;
        }
    }
    for r in rows("SELECT kind, n, size FROM stat_kind")? {
        insert("stat_kind", vec![SqlValue::Integer(kind(&text(&r[0]))), r[1].clone(), r[2].clone()])?;
    }
    for r in rows("SELECT pkind, ckind, field, n FROM stat_edge")? {
        insert("stat_edge", vec![SqlValue::Integer(kind(&text(&r[0]))), SqlValue::Integer(kind(&text(&r[1]))), r[2].clone(), r[3].clone()])?;
    }
    let vals = rows("SELECT tbl, col, val, n, docs, sqd FROM stat_val")?;
    // The names among the values, interned in one transaction (not one each).
    let names: Vec<String> = vals.iter().filter(|r| domain(&text(&r[0]), &text(&r[1])) == Domain::Name).map(|r| text(&r[2])).collect();
    let atoms = store.intern_atoms(&names.iter().map(String::as_str).collect::<Vec<_>>()).map_err(|e| Error::Plan(e.to_string()))?;
    let named: HashMap<&str, i64> = names.iter().map(String::as_str).zip(atoms.into_iter().map(i64::from)).collect();
    let name = |s: &str| -> Result<i64> { Ok(named[s]) };
    for mut r in vals {
        let v = text(&r[2]);
        r[2] = match domain(&text(&r[0]), &text(&r[1])) {
            Domain::Kind => SqlValue::Integer(kind(&v)),
            Domain::Field => SqlValue::Integer(field(&v)),
            Domain::Name => SqlValue::Integer(name(&v)?),
            _ => r[2].clone(),
        };
        // Two of T's strings can lower to one atom (kinds a dialect does not have are all -1).
        pending.borrow_mut().entry("INSERT OR IGNORE INTO s.stat_val".to_owned()).or_default().push(r);
    }
    for r in rows("SELECT d, lo, hi FROM stat_docrange")? {
        let d = int(&r[0]);
        let id = |n: i64| SqlValue::Integer((d << 32) + n - first[&d]);
        insert("stat_docrange", vec![r[0].clone(), id(int(&r[1])), id(int(&r[2]))])?;
    }
    for (statement, rows) in pending.into_inner() {
        super::insert_many(db, &statement, rows)?;
    }
    db.execute_batch("COMMIT")?;
    Ok(())
}

/// CPU seconds (user + system) and peak RSS in MB of this process so far.
pub(crate) fn usage() -> (f64, f64, f64) {
    let mut u: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut u) };
    let s = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
    // macOS reports ru_maxrss in bytes, Linux in kilobytes.
    let rss = if cfg!(target_os = "macos") { u.ru_maxrss as f64 / 1e6 } else { u.ru_maxrss as f64 / 1e3 };
    (s(u.ru_utime), s(u.ru_stime), rss)
}

/// What a corpus run did: its phases' CPU (user, system) and the peak RSS after each, and the outcome.
pub(crate) struct Report {
    /// (phase, user, sys, peak RSS so far, SQLite statements run).
    pub(crate) phases: Vec<(&'static str, f64, f64, f64, usize)>,
    pub(crate) files: usize,
    pub(crate) nodes: usize,
    pub(crate) native: usize,
    pub(crate) rows: BTreeMap<String, usize>,
    pub(crate) chunks: usize,
}

use std::collections::{BTreeMap, BTreeSet};

/// Runs `program` (`.rdl` text) over the files T's `t_facts` names under `root`, in native `modes`, with `nearest`
/// bound as `binding`, and writes its outputs into `out` as T does. The store and its derived tables live in `out`.
///
/// The program is split at its file-local relations (`typing::file_local::programs`, list step 29). Documents are
/// taken in chunks of about `chunk_nodes` nodes: each chunk is parsed, bound and its file-local part run, and what the
/// cross-file part reads (the exported relations, the base relations it reads) is appended to `cross.sqlite` in
/// `out` (schema `x`); the chunk is then dropped. The cross-file part runs once, over `x`. Memory is one chunk's, and
/// the cross-file part's own.
pub(crate) fn run(program: &str, root: &Path, t_facts: &Path, out: &Path, modes: &str, binding: Binding, chunk_nodes: usize) -> Result<Report> {
    let mut phases: Vec<(&'static str, f64, f64, f64, usize)> = Vec::new();
    let mut last = usage();
    let mut counted = 0;
    let mut phase = |name: &'static str, statements: usize| {
        let now = usage();
        let n = statements - counted;
        counted = statements;
        match phases.iter_mut().find(|p| p.0 == name) {
            Some(p) => {
                p.1 += now.0 - last.0;
                p.2 += now.1 - last.1;
                p.3 = now.2;
                p.4 += n;
            }
            None => phases.push((name, now.0 - last.0, now.1 - last.1, now.2, n)),
        }
        last = now;
    };
    let prog = front::parse(program)?;
    let t = Connection::open_with_flags(t_facts, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let docs: Vec<(i64, String)> = t.prepare("SELECT d, path FROM doc ORDER BY d")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    let first: HashMap<i64, i64> = t.prepare("SELECT d, min(n) FROM node GROUP BY d")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    let rust = docs.first().is_some_and(|(_, p)| p.ends_with(".rs"));
    let (label, dialect) = if rust { ("rust", "rust") } else { ("python", "python") };
    let dialect = crate::code_rule_dsl::language::dialect_by_label(dialect).ok_or_else(|| Error::Plan(format!("no dialect {label}")))?;
    std::fs::create_dir_all(out).map_err(|e| Error::Plan(e.to_string()))?;
    let path = out.join("store.sqlite");
    let cross_path = out.join("cross.sqlite");
    for p in [&path, &cross_path] {
        if p.exists() {
            std::fs::remove_file(p).map_err(|e| Error::Plan(e.to_string()))?;
        }
    }
    let store = ContentStore::open(&path).map_err(|e| Error::Plan(e.to_string()))?;
    let db = Connection::open(&path)?;
    let statements = Statements::on(&db);
    statements.enter("stats");
    let mut lowered = lower::lower(&prog, dialect, &mut |s| store.intern_atoms(s).map_err(|e| Error::Plan(e.to_string())))?;
    let sets = lowered.transduce_nearest();
    let split = crate::code_rule_dsl::typing::file_local::programs(&lowered.program);
    let part = |program: front::Program| lower::Lowered { program, domains: lowered.domains.clone(), ops: lowered.ops, dialect: lowered.dialect };
    let (local, cross) = (part(split.local), part(split.cross));
    atomized_statistics(&t, &db, &store, &first, dialect)?;
    // T's corpus statistics, read once: each chunk plans with them and its sources' own, measured in memory, and the
    // cross-file part with them and the chunks' totals.
    let t_stats = super::Stats::of_schema(&db, &lowered.program, "s")?;
    db.execute("ATTACH ?1 AS x", [cross_path.to_string_lossy()])?;
    // Each base relation the cross-file part reads is stored with the columns its rules name (and those decoding a
    // finding's location reads): the others are never read.
    let decodes = cross.program.outputs.iter().any(|o| o == "finding");
    let mut stored: Vec<(String, String)> = Vec::new();
    for rel in &split.base {
        let cols = super::super::sql::select::input_columns(rel).ok_or_else(|| Error::Plan(format!("no columns for {rel}")))?;
        let decode: &[&str] = match rel.as_str() {
            "node" if decodes => &["d", "n", "start"],
            "doc" if decodes => &["d", "path"],
            _ => &[],
        };
        let named: Vec<&str> = cols
            .iter()
            .enumerate()
            .filter(|(i, c)| {
                decode.contains(c)
                    || cross.program.rules.iter().flat_map(|r| &r.body).any(|l| match l {
                        front::Literal::Atom(a) | front::Literal::Agg { atom: a, .. } => &a.rel == rel && a.args.get(*i).is_some_and(|t| !matches!(t, front::Term::Wild)),
                        _ => false,
                    })
            })
            .map(|(_, c)| *c)
            .collect();
        if named.is_empty() {
            continue;
        }
        db.execute_batch(&format!("CREATE TABLE x.{rel}({})", named.iter().map(|c| format!("{c} INTEGER")).collect::<Vec<_>>().join(", ")))?;
        stored.push((rel.clone(), named.join(", ")));
    }
    for rel in &split.exports {
        let width = lowered.program.decls[rel].len();
        let cols: Vec<String> = (0..width).map(|i| format!("c{i}")).collect();
        db.execute_batch(&format!("CREATE TABLE x.{rel}({}, PRIMARY KEY ({})) WITHOUT ROWID", cols.iter().map(|c| format!("{c} INTEGER")).collect::<Vec<_>>().join(", "), cols.join(", ")))?;
    }
    phase("stats", statements.count());
    statements.enter("parse");
    let modules = Modules::default();
    let mut bound = BTreeSet::new();
    let mut totals = Totals::default();
    let (mut nodes, mut native, mut chunks) = (0, BTreeSet::new(), 0);
    let mut outputs: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut node_columns: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let gather = |got: Outcome, outputs: &mut BTreeMap<String, Vec<String>>, node_columns: &mut BTreeMap<String, Vec<usize>>, keep: &dyn Fn(&str) -> bool| {
        for (rel, lines) in got.outputs {
            if keep(&rel) {
                outputs.entry(rel.clone()).or_default().extend(lines);
                node_columns.insert(rel.clone(), got.nodes[&rel].clone());
            }
        }
        got.native
    };
    let mut pending = Vec::new();
    let mut pending_nodes = 0;
    let mut at = docs.iter().peekable();
    while at.peek().is_some() || !pending.is_empty() {
        if let Some((d, path)) = at.next() {
            let source = std::fs::read_to_string(root.join(path)).map_err(|e| Error::Plan(format!("{path}: {e}")))?;
            let file = FileInput::new(path.clone(), label.to_owned(), Arc::new(source), None, None, "core").map_err(|e| Error::Plan(format!("{path}: {e:?}")))?;
            let facts = match core::build(&file).map_err(|e| Error::Plan(format!("{path}: {e:?}")))? {
                Built::Facts(f) => Arc::new(f),
                Built::Unparsed => return Err(Error::Plan(format!("{path} did not parse"))),
            };
            pending_nodes += facts.len();
            pending.push(syntax::Document { key: *d as u32, path: path.clone(), core: format!("/c#core:{path}"), digest: core::digest(&facts), facts });
            if pending_nodes < chunk_nodes && at.peek().is_some() {
                continue;
            }
        }
        phase("parse", statements.count());
        statements.enter("batch");
        nodes += pending_nodes;
        pending_nodes = 0;
        chunks += 1;
        let sources = syntax::with_nearest(&store, std::mem::take(&mut pending), &sets, binding).map_err(|e| Error::Plan(e.to_string()))?;
        phase("batch", statements.count());
        statements.enter("local");
        let mut stats = t_stats.clone();
        for (rel, source) in sources {
            let measured = syntax::statistics(source.as_ref());
            if split.base.contains(&rel) && rel.starts_with("nearest_") {
                totals.add(&rel, source.len(), &measured);
            }
            stats.measured(&rel, measured.into_iter().map(|(c, rows, ndv, sq, sqd)| (c, [rows, ndv, sq, sqd])), []);
            modules.bind(&db, &rel, source)?;
            bound.insert(rel);
        }
        let got = super::run_atoms_native_over(&local, &db, "main", stats, split.exports.iter().cloned().collect(), modes)?;
        let kept = got.kept.clone();
        let outputs_of = |rel: &str| prog.outputs.iter().any(|o| o == rel);
        native.extend(gather(got, &mut outputs, &mut node_columns, &outputs_of).into_iter().map(|k| format!("local:{k}")));
        db.execute_batch("BEGIN")?;
        for (rel, cols) in &stored {
            db.execute_batch(&format!("INSERT INTO x.{rel} SELECT {cols} FROM main.{rel}"))?;
        }
        for rel in &split.exports {
            // Rows another chunk wrote too (a file-local relation without a location column) are kept once. A kept
            // export goes from the engine to `x` in one statement and is measured in place; one a SQL stratum
            // derived is in its table.
            let types: Vec<&str> = lowered.program.decls[rel].iter().map(|(_, t)| t.as_str()).collect();
            if kept.contains(rel) {
                let cols = (0..types.len()).map(|i| format!("c{i}")).collect::<Vec<_>>().join(", ");
                let inserted = db.execute(&format!("INSERT OR IGNORE INTO x.{rel} SELECT {cols} FROM set_derived WHERE rel = ?1"), [rel])?;
                let rows = crate::code_rule_dsl::logic::native::component::derived(rel).ok_or_else(|| Error::Plan(format!("{rel} is not held")))?;
                totals.add_rows(rel, inserted, rows.as_ref(), &types);
            } else {
                let inserted = db.execute(&format!("INSERT OR IGNORE INTO x.{rel} SELECT * FROM temp.{rel}"), [])?;
                let rows = Rows::read(&db, &format!("temp.{rel}"))?;
                totals.add_rows(rel, inserted, &rows, &types);
            }
        }
        db.execute_batch("COMMIT")?;
        // The chunk's sources are dropped before the next is built.
        for rel in &bound {
            if let Some(b) = modules.get(rel) {
                b.clear();
            }
        }
        crate::code_rule_dsl::logic::native::forget_inputs(&db);
        phase("local", statements.count());
        statements.enter("parse");
    }
    statements.enter("stats");
    // The cross-file part's statistics: T's for the base relations, the chunks' totals for the rest.
    let mut cross_stats = t_stats;
    totals.apply(&mut cross_stats);

    // Each input is read by one statement (a scan), so only decoding needs an index: `node` and `doc` by key.
    for rel in stored.iter().map(|(r, _)| r).filter(|r| decodes && matches!(r.as_str(), "node" | "doc")) {
        for (i, cols) in super::vtab_indexes(rel).iter().take(1).enumerate() {
            db.execute_batch(&format!("CREATE INDEX x.{rel}_{i} ON {rel}({})", cols.join(", ")))?;
        }
    }
    phase("stats", statements.count());
    statements.enter("cross");
    if !cross.program.rules.is_empty() {
        let got = super::run_atoms_native_over(&cross, &db, "x", cross_stats, BTreeSet::new(), modes)?;
        native.extend(gather(got, &mut outputs, &mut node_columns, &|_| true).into_iter().map(|k| format!("cross:{k}")));
    }
    phase("cross", statements.count());
    statements.enter("write");
    let mut rows = BTreeMap::new();
    for (rel, lines) in &outputs {
        let node_columns = &node_columns[rel];
        let mut decoded: Vec<String> = lines
            .iter()
            .map(|row| {
                row.split('\t')
                    .enumerate()
                    .map(|(i, v)| match v.parse::<i64>() {
                        Ok(v) if node_columns.contains(&i) => (first[&(v >> 32)] + (v & 0xffff_ffff)).to_string(),
                        _ => v.to_owned(),
                    })
                    .collect::<Vec<_>>()
                    .join("\t")
            })
            .collect();
        decoded.sort();
        decoded.dedup();
        let mut text = decoded.join("\n");
        if !text.is_empty() {
            text.push('\n');
        }
        std::fs::write(out.join(format!("{rel}.csv")), text).map_err(|e| Error::Plan(e.to_string()))?;
        rows.insert(rel.clone(), decoded.len());
    }
    phase("write", statements.count());
    Ok(Report { phases, files: docs.len(), nodes, native: native.len(), rows, chunks })
}


/// The statements SQLite runs on a connection (`SQLITE_TRACE_STMT`: each statement's first step, those inside
/// `execute_batch` and those the native engine's SQL functions run on it included), counted until dropped.
///
/// `RDL_STMTS=<file>`: also each statement's cost at its end (`SQLITE_TRACE_PROFILE`): its time and SQLite's own
/// counters (VM steps, full-scan steps, sorts, automatic-index rows), summed by statement text and written to `<file>`
/// as tab-separated rows (phase, runs, ms, vm steps, full-scan steps, sorts, autoindex rows, text).
struct Statements {
    db: *mut rusqlite::ffi::sqlite3,
    state: Box<StatementState>,
}
#[derive(Default)]
struct StatementState {
    count: std::sync::atomic::AtomicUsize,
    phase: parking_lot::Mutex<&'static str>,
    costs: Option<parking_lot::Mutex<HashMap<(&'static str, String), [u64; 6]>>>,
}
impl Statements {
    fn on(db: &Connection) -> Self {
        use rusqlite::ffi;
        unsafe extern "C" fn trace(event: u32, state: *mut std::ffi::c_void, p: *mut std::ffi::c_void, x: *mut std::ffi::c_void) -> std::ffi::c_int {
            // SAFETY: the state outlives the callback (removed on drop).
            let state = unsafe { &*state.cast::<StatementState>() };
            if event == ffi::SQLITE_TRACE_STMT as u32 {
                state.count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return 0;
            }
            let Some(costs) = &state.costs else { return 0 };
            let stmt = p.cast::<ffi::sqlite3_stmt>();
            // SAFETY: for SQLITE_TRACE_PROFILE, P is the statement and X its time in nanoseconds.
            let (ns, text, vm, scan, sort, auto) = unsafe {
                let ns = *x.cast::<i64>() as u64;
                let text = std::ffi::CStr::from_ptr(ffi::sqlite3_sql(stmt)).to_string_lossy().split_whitespace().collect::<Vec<_>>().join(" ");
                let st = |op| ffi::sqlite3_stmt_status(stmt, op, 1) as u64;
                (ns, text, st(ffi::SQLITE_STMTSTATUS_VM_STEP), st(ffi::SQLITE_STMTSTATUS_FULLSCAN_STEP), st(ffi::SQLITE_STMTSTATUS_SORT), st(ffi::SQLITE_STMTSTATUS_AUTOINDEX))
            };
            let phase = *state.phase.lock();
            let mut costs = costs.lock();
            let c = costs.entry((phase, text)).or_default();
            for (a, b) in c.iter_mut().zip([1, ns, vm, scan, sort, auto]) {
                *a += b;
            }
            0
        }
        let state = Box::new(StatementState { costs: std::env::var_os("RDL_STMTS").map(|_| Default::default()), ..Default::default() });
        let mask = ffi::SQLITE_TRACE_STMT as u32 | if state.costs.is_some() { ffi::SQLITE_TRACE_PROFILE as u32 } else { 0 };
        // SAFETY: the connection is live for the run; the state is boxed (a stable address) and outlives the callback.
        let db = unsafe { db.handle() };
        unsafe { ffi::sqlite3_trace_v2(db, mask, Some(trace), std::ptr::from_ref(&*state).cast_mut().cast()) };
        Statements { db, state }
    }
    fn count(&self) -> usize {
        self.state.count.load(std::sync::atomic::Ordering::Relaxed)
    }
    /// The phase the statements from now on are charged to.
    fn enter(&self, phase: &'static str) {
        *self.state.phase.lock() = phase;
    }
}
impl Drop for Statements {
    fn drop(&mut self) {
        // SAFETY: as in `on`; the connection is still open (dropped after this).
        unsafe { rusqlite::ffi::sqlite3_trace_v2(self.db, 0, None, std::ptr::null_mut()) };
        if let (Some(costs), Some(file)) = (&self.state.costs, std::env::var_os("RDL_STMTS")) {
            let mut all: Vec<((&str, String), [u64; 6])> = costs.lock().drain().collect();
            all.sort_by_key(|(_, c)| std::cmp::Reverse(c[1]));
            let text: String = all
                .iter()
                .map(|((phase, sql), [n, ns, vm, scan, sort, auto])| format!("{phase}\t{n}\t{:.3}\t{vm}\t{scan}\t{sort}\t{auto}\t{sql}\n", *ns as f64 / 1e6))
                .collect();
            let _ = std::fs::write(file, text);
        }
    }
}

/// The cross-file pass's statistics of what the chunks wrote, as T's `stats.py` defines them, summed over the chunks
/// as each chunk's rows are written (from the rows in memory; no scan at the end). A document is in one chunk, so
/// per-document sums (`sqd`, `docs`) add exactly. Value columns (not node ids) keep each value's postings (`stat_val`:
/// `n`, `docs`, `sqd`), merged across chunks, so their `ndv` and `sq` are exact too; the planner reads a constant
/// missing from `stat_val` as matching no row (`const_freq`), so a value column without them would price every
/// constant at zero rows. Node columns' values never cross documents, so their `ndv` adds exactly.
#[derive(Default)]
struct Totals {
    rels: BTreeMap<String, (usize, BTreeMap<String, [i64; 4]>)>,
    /// Per (relation, value column): per value, (n, docs, sqd).
    vals: BTreeMap<(String, String), HashMap<i64, [i64; 3]>>,
}
impl Totals {
    fn add(&mut self, rel: &str, rows: usize, measured: &[(String, i64, i64, i64, i64)]) {
        let entry = self.rels.entry(rel.to_owned()).or_default();
        entry.0 += rows;
        for (col, rows, ndv, sq, sqd) in measured {
            let t = entry.1.entry(col.clone()).or_default();
            for (a, b) in t.iter_mut().zip([rows, ndv, sq, sqd]) {
                *a += *b;
            }
        }
    }
    /// A chunk's rows of `rel` (`inserted` of them new to `x`), with its columns' declared types.
    fn add_rows(&mut self, rel: &str, inserted: usize, rows: &dyn crate::code_rule_dsl::facts::vtab::Source, types: &[&str]) {
        let n = rows.len();
        // A row's document: its first node column's (`d << 32 | index`), or one document for a relation without one.
        let doc_col = types.iter().position(|t| *t == "node");
        let doc = |r: usize| doc_col.map_or(-1, |c| rows.value(r, c) >> 32);
        let entry = self.rels.entry(rel.to_owned()).or_default();
        entry.0 += inserted;
        for (c, name) in rows.columns().iter().enumerate() {
            if types.get(c) == Some(&"node") {
                let measured = column_moments(rows, c, &doc);
                let t = entry.1.entry(name.clone()).or_default();
                for (a, b) in t.iter_mut().zip([n as i64, measured.0, measured.1, measured.2]) {
                    *a += b;
                }
                continue;
            }
            // Per (document, value) counts of this chunk, then per value: n, documents, and the sum of squares.
            let mut per: HashMap<(i64, i64), i64> = HashMap::new();
            for r in 0..n {
                *per.entry((doc(r), rows.value(r, c))).or_default() += 1;
            }
            let vals = self.vals.entry((rel.to_owned(), name.clone())).or_default();
            for ((_, v), k) in per {
                let e = vals.entry(v).or_default();
                e[0] += k;
                e[1] += 1;
                e[2] += k * k;
            }
        }
    }
    /// The totals as the planner's statistics (T's `stats.py` definitions), over `stats`.
    fn apply(&self, stats: &mut super::Stats) {
        for (rel, (rows, measured)) in &self.rels {
            stats.measured(rel, measured.iter().map(|(col, [_, ndv, sq, sqd])| (col.clone(), [*rows as i64, (*ndv).max(1), *sq, *sqd])), []);
        }
        for ((rel, col), per) in &self.vals {
            let rows = self.rels.get(rel).map_or(0, |r| r.0 as i64);
            let (ndv, sq, sqd) = per.values().fold((0, 0, 0), |(a, b, c), e| (a + 1, b + e[0] * e[0], c + e[2]));
            stats.measured(rel, [(col.clone(), [rows, ndv.max(1), sq, sqd])], per.iter().map(|(v, [n, _, _])| (col.clone(), *v, *n)));
        }
    }
}

/// A node column's (ndv, sq, sqd) in one chunk: its values never cross documents.
fn column_moments(rows: &dyn crate::code_rule_dsl::facts::vtab::Source, c: usize, doc: &dyn Fn(usize) -> i64) -> (i64, i64, i64) {
    let mut per: HashMap<i64, i64> = HashMap::new();
    let mut per_doc: HashMap<(i64, i64), i64> = HashMap::new();
    for r in 0..rows.len() {
        *per.entry(rows.value(r, c)).or_default() += 1;
        *per_doc.entry((doc(r), rows.value(r, c))).or_default() += 1;
    }
    (per.len() as i64, per.values().map(|k| k * k).sum(), per_doc.values().map(|k| k * k).sum())
}

/// A chunk's rows of a relation (columns `c0`, `c1`, …), read once to measure them.
struct Rows {
    columns: Vec<String>,
    values: Vec<Vec<i64>>,
}
impl Rows {
    fn read(db: &Connection, table: &str) -> Result<Self> {
        let mut st = db.prepare(&format!("SELECT * FROM {table}"))?;
        let columns: Vec<String> = (0..st.column_count()).map(|i| format!("c{i}")).collect();
        let mut values = vec![Vec::new(); columns.len()];
        let mut q = st.query([])?;
        while let Some(r) = q.next()? {
            for (c, v) in values.iter_mut().enumerate() {
                v.push(r.get::<_, i64>(c)?);
            }
        }
        Ok(Rows { columns, values })
    }
}
impl crate::code_rule_dsl::facts::vtab::Source for Rows {
    fn columns(&self) -> Vec<String> {
        self.columns.clone()
    }
    fn len(&self) -> usize {
        self.values.first().map_or(0, Vec::len)
    }
    fn estimated_matches(&self, _: usize, _: bool) -> usize {
        1
    }
    fn indexed(&self, _: usize) -> bool {
        false
    }
    fn lookup(&self, _: usize, _: &[i64]) -> Option<Vec<usize>> {
        None
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        self.values[column][row]
    }
}
