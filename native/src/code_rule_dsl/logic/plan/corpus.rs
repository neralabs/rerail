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

/// `nearest_<set>`'s statistics, made at bind time, beside T's in schema `s`.
fn nearest_statistics(db: &Connection, rel: &str, source: &dyn crate::code_rule_dsl::facts::vtab::Source) -> Result<()> {
    db.execute("INSERT OR REPLACE INTO s.stat_meta VALUES (?1, ?2)", rusqlite::params![format!("rows.{rel}"), source.len() as i64])?;
    for (col, rows, ndv, sq, sqd) in syntax::statistics(source) {
        db.execute("INSERT OR REPLACE INTO s.stat_col VALUES (?1, ?2, ?3, ?4, ?5, ?6)", rusqlite::params![rel, col, rows, ndv, sq, sqd])?;
    }
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
}

use std::collections::BTreeMap;

/// Runs `program` (`.rdl` text) over the files T's `t_facts` names under `root`, in native `modes`, with `nearest`
/// bound as `binding`, and writes its outputs into `out` as T does. The store and its derived tables live in `out`.
pub(crate) fn run(program: &str, root: &Path, t_facts: &Path, out: &Path, modes: &str, binding: Binding) -> Result<Report> {
    let mut phases = Vec::new();
    let mut last = usage();
    let mut phase = |name: &'static str| {
        let now = usage();
        phases.push((name, now.0 - last.0, now.1 - last.1, now.2));
        last = now;
    };
    let prog = front::parse(program)?;
    let t = Connection::open_with_flags(t_facts, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let docs: Vec<(i64, String)> = t.prepare("SELECT d, path FROM doc ORDER BY d")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    let first: HashMap<i64, i64> = t.prepare("SELECT d, min(n) FROM node GROUP BY d")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    let rust = docs.first().is_some_and(|(_, p)| p.ends_with(".rs"));
    let (label, dialect) = if rust { ("rust", "rust") } else { ("python", "python") };
    let dialect = crate::code_rule_dsl::language::dialect_by_label(dialect).ok_or_else(|| Error::Plan(format!("no dialect {label}")))?;
    let mut documents = Vec::with_capacity(docs.len());
    let mut nodes = 0;
    for (d, path) in &docs {
        let source = std::fs::read_to_string(root.join(path)).map_err(|e| Error::Plan(format!("{path}: {e}")))?;
        let file = FileInput::new(path.clone(), label.to_owned(), Arc::new(source), None, None, "core").map_err(|e| Error::Plan(format!("{path}: {e:?}")))?;
        let facts = match core::build(&file).map_err(|e| Error::Plan(format!("{path}: {e:?}")))? {
            Built::Facts(f) => Arc::new(f),
            Built::Unparsed => return Err(Error::Plan(format!("{path} did not parse"))),
        };
        nodes += facts.len();
        documents.push(syntax::Document { key: *d as u32, path: path.clone(), core: format!("/c#core:{path}"), digest: core::digest(&facts), facts });
    }
    phase("parse");
    std::fs::create_dir_all(out).map_err(|e| Error::Plan(e.to_string()))?;
    let path = out.join("store.sqlite");
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| Error::Plan(e.to_string()))?;
    }
    let store = ContentStore::open(&path).map_err(|e| Error::Plan(e.to_string()))?;
    let db = Connection::open(&path)?;
    let mut lowered = lower::lower(&prog, dialect, &mut |s| store.intern_atoms(s).map_err(|e| Error::Plan(e.to_string())))?;
    let sets = lowered.transduce_nearest();
    atomized_statistics(&t, &db, &store, &first, dialect)?;
    phase("stats");
    let modules = Modules::default();
    let sources = syntax::with_nearest(&store, documents, &sets, binding).map_err(|e| Error::Plan(e.to_string()))?;
    phase("batch");
    for (rel, source) in sources {
        if rel.starts_with("nearest_") {
            nearest_statistics(&db, &rel, source.as_ref())?;
        }
        modules.bind(&db, &rel, source)?;
    }
    phase("bind");
    let got: Outcome = run_atoms_native(&lowered, &db, "s", modes)?;
    phase("run");
    let mut rows = BTreeMap::new();
    for (rel, lines) in &got.outputs {
        let node_columns = &got.nodes[rel];
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
        let mut text = decoded.join("\n");
        if !text.is_empty() {
            text.push('\n');
        }
        std::fs::write(out.join(format!("{rel}.csv")), text).map_err(|e| Error::Plan(e.to_string()))?;
        rows.insert(rel.clone(), decoded.len());
    }
    phase("write");
    Ok(Report { phases, files: docs.len(), nodes, native: got.native.len(), rows })
}
