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

use super::{Error, Outcome, Result, lower, run_atoms_native};

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
    let name = |s: &str| -> Result<i64> { Ok(i64::from(store.intern_atoms(&[s]).map_err(|e| Error::Plan(e.to_string()))?[0])) };
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
    let insert = |table: &str, row: Vec<SqlValue>| -> Result<()> {
        let holes = vec!["?"; row.len()].join(", ");
        db.execute(&format!("INSERT INTO s.{table} VALUES ({holes})"), rusqlite::params_from_iter(row))?;
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
    for mut r in rows("SELECT tbl, col, val, n, docs, sqd FROM stat_val")? {
        let v = text(&r[2]);
        r[2] = match domain(&text(&r[0]), &text(&r[1])) {
            Domain::Kind => SqlValue::Integer(kind(&v)),
            Domain::Field => SqlValue::Integer(field(&v)),
            Domain::Name => SqlValue::Integer(name(&v)?),
            _ => r[2].clone(),
        };
        // Two of T's strings can lower to one atom (kinds a dialect does not have are all -1).
        let holes = vec!["?"; r.len()].join(", ");
        db.execute(&format!("INSERT OR IGNORE INTO s.stat_val VALUES ({holes})"), rusqlite::params_from_iter(r))?;
    }
    for r in rows("SELECT d, lo, hi FROM stat_docrange")? {
        let d = int(&r[0]);
        let id = |n: i64| SqlValue::Integer((d << 32) + n - first[&d]);
        insert("stat_docrange", vec![r[0].clone(), id(int(&r[1])), id(int(&r[2]))])?;
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
    /// (phase, user, sys, peak RSS so far).
    pub(crate) phases: Vec<(&'static str, f64, f64, f64)>,
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
    let mut phases: Vec<(&'static str, f64, f64, f64)> = Vec::new();
    let mut last = usage();
    let mut phase = |name: &'static str| {
        let now = usage();
        match phases.iter_mut().find(|p| p.0 == name) {
            Some(p) => {
                p.1 += now.0 - last.0;
                p.2 += now.1 - last.1;
                p.3 = now.2;
            }
            None => phases.push((name, now.0 - last.0, now.1 - last.1, now.2)),
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
    let mut lowered = lower::lower(&prog, dialect, &mut |s| store.intern_atoms(s).map_err(|e| Error::Plan(e.to_string())))?;
    let sets = lowered.transduce_nearest();
    let split = crate::code_rule_dsl::typing::file_local::programs(&lowered.program);
    let part = |program: front::Program| lower::Lowered { program, domains: lowered.domains.clone(), ops: lowered.ops, dialect: lowered.dialect };
    let (local, cross) = (part(split.local), part(split.cross));
    atomized_statistics(&t, &db, &store, &first, dialect)?;
    // T's corpus statistics, kept for the cross-file part's planning: each chunk's overwrite them with its own.
    db.execute_batch("CREATE TABLE s.t_meta AS SELECT * FROM s.stat_meta; CREATE TABLE s.t_col AS SELECT * FROM s.stat_col;")?;
    db.execute("ATTACH ?1 AS x", [cross_path.to_string_lossy()])?;
    for rel in &split.base {
        let cols = super::super::sql::select::input_columns(rel).ok_or_else(|| Error::Plan(format!("no columns for {rel}")))?;
        db.execute_batch(&format!("CREATE TABLE x.{rel}({})", cols.iter().map(|c| format!("{c} INTEGER")).collect::<Vec<_>>().join(", ")))?;
    }
    for rel in &split.exports {
        let width = lowered.program.decls[rel].len();
        let cols: Vec<String> = (0..width).map(|i| format!("c{i}")).collect();
        db.execute_batch(&format!("CREATE TABLE x.{rel}({}, PRIMARY KEY ({})) WITHOUT ROWID", cols.iter().map(|c| format!("{c} INTEGER")).collect::<Vec<_>>().join(", "), cols.join(", ")))?;
    }
    phase("stats");
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
        phase("parse");
        nodes += pending_nodes;
        pending_nodes = 0;
        chunks += 1;
        let sources = syntax::with_nearest(&store, std::mem::take(&mut pending), &sets, binding).map_err(|e| Error::Plan(e.to_string()))?;
        phase("batch");
        for (rel, source) in sources {
            let measured = source_statistics(&db, &rel, source.as_ref())?;
            if split.base.contains(&rel) && rel.starts_with("nearest_") {
                totals.add(&rel, source.len(), &measured);
            }
            modules.bind(&db, &rel, source)?;
            bound.insert(rel);
        }
        let got = run_atoms_native(&local, &db, "s", modes)?;
        let outputs_of = |rel: &str| prog.outputs.iter().any(|o| o == rel);
        native.extend(gather(got, &mut outputs, &mut node_columns, &outputs_of).into_iter().map(|k| format!("local:{k}")));
        db.execute_batch("BEGIN")?;
        for rel in &split.base {
            let cols = super::super::sql::select::input_columns(rel).expect("checked above").join(", ");
            db.execute_batch(&format!("INSERT INTO x.{rel} SELECT {cols} FROM main.{rel}"))?;
        }
        for rel in &split.exports {
            // Rows another chunk wrote too (a file-local relation without a location column) are kept once.
            let inserted = db.execute(&format!("INSERT OR IGNORE INTO x.{rel} SELECT * FROM temp.{rel}"), [])?;
            let rows = Rows::read(&db, &format!("temp.{rel}"))?;
            totals.add(rel, inserted, &syntax::statistics(&rows));
        }
        db.execute_batch("COMMIT")?;
        // The chunk's sources are dropped before the next is built.
        for rel in &bound {
            if let Some(b) = modules.get(rel) {
                b.clear();
            }
        }
        crate::code_rule_dsl::logic::native::forget_inputs(&db);
        phase("local");
    }
    // The cross-file part's statistics: T's for the base relations, measured on `x` for the rest.
    db.execute_batch("DELETE FROM s.stat_meta; INSERT INTO s.stat_meta SELECT * FROM s.t_meta; DELETE FROM s.stat_col; INSERT INTO s.stat_col SELECT * FROM s.t_col;")?;
    totals.write(&db)?;
    // The cross-file pass probes exports on any column: a column's own index (the key serves the first).
    for rel in &split.exports {
        for c in 1..lowered.program.decls[rel].len() {
            db.execute_batch(&format!("CREATE INDEX x.{rel}_c{c} ON {rel}(c{c})"))?;
        }
    }
    for rel in &split.base {
        for (i, cols) in super::vtab_indexes(rel).iter().enumerate() {
            db.execute_batch(&format!("CREATE INDEX x.{rel}_{i} ON {rel}({})", cols.join(", ")))?;
        }
    }
    phase("stats");
    if !cross.program.rules.is_empty() {
        crate::code_rule_dsl::logic::native::read_in_place(&db, "x")?;
        let got = super::run_atoms_native_over(&cross, &db, "x", "s", modes)?;
        native.extend(gather(got, &mut outputs, &mut node_columns, &|_| true).into_iter().map(|k| format!("cross:{k}")));
    }
    phase("cross");
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
    phase("write");
    Ok(Report { phases, files: docs.len(), nodes, native: native.len(), rows, chunks })
}

/// A bound source's statistics (its rows, and `stat_col`'s per column), in schema `s`: a chunk's own.
fn source_statistics(db: &Connection, rel: &str, source: &dyn crate::code_rule_dsl::facts::vtab::Source) -> Result<Vec<(String, i64, i64, i64, i64)>> {
    db.execute("INSERT OR REPLACE INTO s.stat_meta VALUES (?1, ?2)", rusqlite::params![format!("rows.{rel}"), source.len() as i64])?;
    let measured = syntax::statistics(source);
    for (col, rows, ndv, sq, sqd) in &measured {
        db.execute("INSERT OR REPLACE INTO s.stat_col VALUES (?1, ?2, ?3, ?4, ?5, ?6)", rusqlite::params![rel, col, rows, ndv, sq, sqd])?;
    }
    Ok(measured)
}

/// The cross-file pass's statistics of what the chunks wrote, summed over the chunks as they are written (no scan at
/// the end): rows, postings' squares and their per-document sums add exactly (a document is in one chunk). Distinct
/// values add exactly on a column of locations; elsewhere the sum bounds them above, which the planner reads as an
/// estimate.
#[derive(Default)]
struct Totals(BTreeMap<String, (usize, BTreeMap<String, [i64; 4]>)>);
impl Totals {
    fn add(&mut self, rel: &str, rows: usize, measured: &[(String, i64, i64, i64, i64)]) {
        let entry = self.0.entry(rel.to_owned()).or_default();
        entry.0 += rows;
        for (col, rows, ndv, sq, sqd) in measured {
            let t = entry.1.entry(col.clone()).or_default();
            for (a, b) in t.iter_mut().zip([rows, ndv, sq, sqd]) {
                *a += *b;
            }
        }
    }
    fn write(&self, db: &Connection) -> Result<()> {
        for (rel, (rows, cols)) in &self.0 {
            db.execute("INSERT OR REPLACE INTO s.stat_meta VALUES (?1, ?2)", rusqlite::params![format!("rows.{rel}"), *rows as i64])?;
            for (col, [_, ndv, sq, sqd]) in cols {
                db.execute("INSERT OR REPLACE INTO s.stat_col VALUES (?1, ?2, ?3, ?4, ?5, ?6)", rusqlite::params![rel, col, *rows as i64, ndv.max(&1), sq, sqd])?;
            }
        }
        Ok(())
    }
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
