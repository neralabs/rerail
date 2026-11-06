//! A logic program over a corpus on disk, as T ran its programs (list step 23, the Phase 6 check; the
//! `rerail-engine logic` tool).
//!
//! The corpus is parsed into core facts (`facts::core`), bound as T's base relations (`facts::vtab::syntax`), with
//! `nearest` from the transducer's file-local FIX (`typing::file_local`, O12), and the lowered program runs in a native
//! mode over atoms (`run_atoms_native`, O13). Its outputs are written as T's `rdl.py sql` writes them: one sorted,
//! tab-separated `<relation>.csv` per output, symbols as text and node ids as T's.
//!
//! With T's own `facts.sqlite` for the same corpus (to reproduce T), two things are read from it: the planner's
//! statistics (`stats.py build`'s tables, atomized as the program is: `atomized_statistics`), and each document's
//! first node id, which maps a node `d << 32 | index` to T's global id. T's documents (`doc`) name the files, in
//! T's order.
//!
//! Without it, the typing programs' own ingestion (list step 29): the files are the corpus's `.py` (or `.rs`)
//! files in path order, and every chunk measures its base relations' statistics as it binds them, the tree's
//! included (per kind its nodes and sizes, per parent kind its children, the postings of the program's constants:
//! `Own`). The cross-file part plans with the chunks' sums. Nodes are written as `path:index`.

use super::{Error, Outcome, Result, lower};

use crate::code_rule_dsl::facts::core;
use crate::code_rule_dsl::facts::vtab::{Modules, syntax};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::logic::front;
use crate::code_rule_dsl::model::FileInput;
use crate::code_rule_dsl::typing::cross_rounds;
mod references;
mod typing;
#[cfg(test)]
mod base_tests;
pub(crate) use typing::{Typing, TypingReport};
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
/// The calling thread's CPU (user, system) seconds.
pub(crate) fn thread_usage() -> (f64, f64) {
    #[cfg(target_os = "macos")]
    {
        let mut info: libc::thread_basic_info = unsafe { std::mem::zeroed() };
        let mut count = (std::mem::size_of::<libc::thread_basic_info>() / std::mem::size_of::<libc::integer_t>()) as libc::mach_msg_type_number_t;
        // SAFETY: THREAD_BASIC_INFO fills a `thread_basic_info` of `count` integers for this thread's port (the
        // pthread's own, so no port right is taken).
        let port = unsafe { libc::pthread_mach_thread_np(libc::pthread_self()) };
        unsafe { libc::thread_info(port, libc::THREAD_BASIC_INFO as libc::thread_flavor_t, (&raw mut info).cast(), &mut count) };
        let t = |v: libc::time_value_t| f64::from(v.seconds) + f64::from(v.microseconds) / 1e6;
        (t(info.user_time), t(info.system_time))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let mut u: libc::rusage = unsafe { std::mem::zeroed() };
        unsafe { libc::getrusage(libc::RUSAGE_THREAD, &mut u) };
        let s = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
        (s(u.ru_utime), s(u.ru_stime))
    }
}

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
    /// The cross-file part's bounded rounds (`typing::cross_rounds`).
    pub(crate) rounds: cross_rounds::Rounds,
    /// Declarations, summaries and terms (`corpus::typing`).
    pub(crate) typing: TypingReport,
    /// Text tests (decision 0037): relations, distinct strings tested, and the CPU testing them (seconds).
    pub(crate) text: (usize, usize, f64),
}

use std::collections::{BTreeMap, BTreeSet};

/// Runs `program` (`.rr` text, `logic::rr`) over the files T's `t_facts` names under `root`, in native `modes`, with `nearest`
/// bound as `binding`, and writes its outputs into `out` as T does. The store and its derived tables live in `out`.
///
/// The program is split at its file-local relations (`typing::file_local::programs`, list step 29). Documents are
/// taken in chunks of about `chunk_nodes` nodes, each built on a worker thread while the previous one runs (list
/// step 24): its file-local part is run, the exported relations are appended to `cross.sqlite` in `out` (schema `x`),
/// the base columns the cross-file part names are appended in memory, and the chunk is dropped. The cross-file part
/// runs once, over the base relations held in memory and the exports read once from `x`, all bound in `main`. Memory
/// is two chunks', the held base columns, and the cross-file part's own.
pub(crate) fn run(program: &str, root: &Path, t_facts: Option<&Path>, out: &Path, modes: &str, binding: Binding, chunk_nodes: usize) -> Result<Report> {
    run_typed(program, root, t_facts, out, modes, binding, chunk_nodes, &Typing::default())
}

/// The functors only CPython gives (core.rr): its printer and its dump of a tree.
const CPYTHON_FUNCTORS: [&str; 2] = ["unparse", "ast_dump"];

/// Fails, naming it, where `prog` reads one of core.rr's relations that nothing derives and the runner does not bind
/// (`value`, `str_part`, …), or calls a functor only CPython gives: the planner would otherwise fail on its missing
/// statistics (`no statistics for value.c0`). A program's own inputs are its business (facts it is given later).
fn bound(prog: &front::Program) -> Result<()> {
    let derived: BTreeSet<&str> = prog.rules.iter().map(|r| r.head.rel.as_str()).chain(prog.facts.keys().map(String::as_str)).collect();
    let cpython = "CPython (core.rr: the isolated `ast.parse` worker), which the corpus runner does not run";
    for rule in &prog.rules {
        for a in rule.body.iter().filter_map(front::Literal::atom) {
            let rel = a.rel.as_str();
            if derived.contains(rel) || super::super::sql::select::input_columns(rel).is_some() || !crate::code_rule_dsl::logic::rr::is_core(rel) {
                continue;
            }
            let why = match rel {
                "str_part" => format!("it needs {cpython}"),
                "list_len" => "it counts a pattern's list variable ($$$X), and patterns do not lower".to_owned(),
                _ => "the corpus runner has no producer of it (the relations it binds: candidates/datalog/authored/README.md)".to_owned(),
            };
            return Err(Error::Plan(format!("{rel} is read, and nothing derives it or binds it: {why}")));
        }
        let mut calls = Vec::new();
        for l in &rule.body {
            match l {
                front::Literal::Atom(a) | front::Literal::Agg { atom: a, .. } => calls.extend(&a.args),
                front::Literal::Cmp { l, r, .. } => calls.extend([l, r]),
                _ => {}
            }
        }
        calls.extend(&rule.head.args);
        while let Some(t) = calls.pop() {
            if let front::Term::Call(f, args) = t {
                if CPYTHON_FUNCTORS.contains(&f.as_str()) {
                    return Err(Error::Plan(format!("@{f} is called, and it needs {cpython}")));
                }
                calls.extend(args);
            }
        }
    }
    Ok(())
}

/// `run` with typing inputs and outputs (`typing`: declarations, dependency summaries, summarizing).
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_typed(program: &str, root: &Path, t_facts: Option<&Path>, out: &Path, modes: &str, binding: Binding, chunk_nodes: usize, typing: &Typing) -> Result<Report> {
    let mut phases: Vec<(&'static str, f64, f64, f64, usize)> = Vec::new();
    // The main thread's CPU (the chunk builder's is its own, below) and the process's peak RSS.
    let main_usage = || { let (u, s) = thread_usage(); (u, s, usage().2) };
    let mut last = main_usage();
    let mut counted = 0;
    let mut phase = |name: &'static str, statements: usize| {
        let now = main_usage();
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
    let profiling = crate::code_rule_dsl::logic::profile::on();
    if profiling {
        crate::code_rule_dsl::logic::profile::reset();
    }
    // Plans are kept across this run's chunks only (decision 0038).
    let _plans = super::KeepPlans::start();
    let mut prog = crate::code_rule_dsl::logic::rr::parse(program)?;
    let t = t_facts.map(|t_facts| Connection::open_with_flags(t_facts, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)).transpose()?;
    let (docs, first, rust): (Vec<(i64, String)>, HashMap<i64, i64>, bool) = match &t {
        Some(t) => {
            let docs: Vec<(i64, String)> = t.prepare("SELECT d, path FROM doc ORDER BY d")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
            let first = t.prepare("SELECT d, min(n) FROM node GROUP BY d")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
            let rust = docs.first().is_some_and(|(_, p)| p.ends_with(".rs"));
            (docs, first, rust)
        }
        None => {
            let (python, rs, text) = corpus_files(root)?;
            let rust = rs.len() > python.len();
            // The run's grammar files and the text-only documents, in path order.
            let mut files = if rust { rs } else { python };
            files.extend(text);
            files.sort();
            (files.into_iter().enumerate().map(|(i, path)| (i as i64 + 1, path)).collect(), HashMap::new(), rust)
        }
    };
    let paths: HashMap<i64, String> = docs.iter().cloned().collect();
    // Declarations and dependency summaries, as `declared_ret` facts (`corpus::typing`).
    let mut typed = TypingReport::default();
    let mut declared = BTreeMap::new();
    if prog.decls.contains_key("declared_ret") {
        let files: Vec<String> = docs.iter().map(|(_, p)| p.clone()).filter(|p| !text_document(p)).collect();
        let (facts, d) = typing::facts(typing, root, &files, &mut typed)?;
        if !facts.is_empty() {
            prog = crate::code_rule_dsl::logic::rr::parse(&format!("{program}\n{facts}"))?;
        }
        declared = d;
    }
    let (label, dialect) = if rust { ("rust", "rust") } else { ("python", "python") };
    let dialect = crate::code_rule_dsl::language::dialect_by_label(dialect).ok_or_else(|| Error::Plan(format!("no dialect {label}")))?;
    std::fs::create_dir_all(out).map_err(|e| Error::Plan(e.to_string()))?;
    let path = out.join("store.sqlite");
    let cross_path = out.join("cross.sqlite");
    // A failed run leaves its write-ahead log beside the store; reopening a new store over it reads stale pages.
    for p in [&path, &cross_path].into_iter().flat_map(|p| ["", "-wal", "-shm"].map(|s| std::path::PathBuf::from(format!("{}{s}", p.display())))) {
        if p.exists() {
            std::fs::remove_file(&p).map_err(|e| Error::Plan(e.to_string()))?;
        }
    }
    let store = ContentStore::open(&path).map_err(|e| Error::Plan(e.to_string()))?;
    let db = Connection::open(&path)?;
    let statements = Statements::on(&db);
    statements.enter("stats");
    // The base relations it reads and does not derive are inputs, `.input` or not, as an authored file's are
    // (`convert::overlay`): the batches bind them, where a relation of its own would be an empty table.
    base_inputs(&mut prog);
    // Bodies simplified first (decision 0038): implied atoms and set-member kind tests drop, identical relations merge.
    crate::code_rule_dsl::logic::simplify::simplify(&mut prog);
    // Text tests over text relations become relations the batches fill from their vocabulary (decision 0037).
    let vocabulary = crate::code_rule_dsl::logic::strings::as_relations(&mut prog).map_err(Error::Plan)?;
    let mut lowered = lower::lower(&prog, dialect, &mut |s| store.intern_atoms(s).map_err(|e| Error::Plan(e.to_string())))?;
    let sets = lowered.transduce_nearest();
    bound(&lowered.program)?;
    let read: BTreeSet<String> = lowered.program.rules.iter().flat_map(|r| r.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.clone())).collect();
    // An input no rule reads is never bound or measured: it is not an input of what runs.
    let outputs = lowered.program.outputs.clone();
    lowered.program.inputs.retain(|rel| read.contains(rel) || outputs.contains(rel));
    let split = crate::code_rule_dsl::typing::file_local::programs(&lowered.program);
    // The reference graph's relations, for a program that reads them: its files are summarized as the chunks are
    // built, and linked once they all are (`references`).
    let references = if rust { None } else { references::Files::new(&prog, &read, &docs.iter().map(|(_, p)| p.clone()).filter(|p| !text_document(p)).collect::<Vec<_>>())? };
    let part = |program: front::Program| lower::Lowered { program, domains: lowered.domains.clone(), ops: lowered.ops, dialect: lowered.dialect };
    crate::code_rule_dsl::logic::profile::decision("file-local split", || {
        format!(
            "{} rules run per chunk over its files (file-local); {} rules run once over all files (cross-file); {} relations exported to the cross part, {} base relations kept for it",
            split.local.rules.len(),
            split.cross.rules.len(),
            split.exports.len(),
            split.base.len()
        )
    });
    crate::code_rule_dsl::logic::profile::decision("nearest ancestors", || {
        let names: Vec<&str> = sets.iter().map(|(n, _)| n.as_str()).collect();
        format!("{} sets made by the transducer's file-local FIX, bound {binding:?}: {}", sets.len(), names.join(", "))
    });
    let (local, cross) = (part(split.local), part(split.cross));
    // T's corpus statistics, read once: each chunk plans with them and its sources' own, measured in memory, and the
    // cross-file part with them and the chunks' totals. Without T, every chunk measures its own (`Own`).
    let t_stats = match &t {
        Some(t) => {
            atomized_statistics(t, &db, &store, &first, dialect)?;
            super::Stats::of_schema(&db, &lowered.program, "s")?
        }
        None => {
            // A document's nodes are `d << 32 | index`: its interval, for the SQL strata's range reads.
            db.execute_batch("ATTACH ':memory:' AS s; CREATE TABLE s.stat_docrange(d INTEGER PRIMARY KEY, lo INTEGER, hi INTEGER);")?;
            let ranges = docs.iter().map(|(d, _)| vec![SqlValue::Integer(*d), SqlValue::Integer(d << 32), SqlValue::Integer((d << 32) | 0xffff_ffff)]).collect();
            super::insert_many(&db, "INSERT INTO s.stat_docrange", ranges)?;
            super::Stats::empty(&lowered.program)
        }
    };
    let own = t.is_none();
    crate::code_rule_dsl::logic::profile::decision("statistics", || {
        if own { "each chunk measures its own sources' statistics; the planner prices that chunk's plans from them".into() } else { "T's corpus statistics, read once, with each chunk's sources' own".into() }
    });
    if let Some(n) = std::env::var("RERAIL_SAMPLE_EVERY").ok().filter(|n| n != "1") {
        crate::code_rule_dsl::logic::profile::decision("sampling", || format!("every {n}th file only (RERAIL_SAMPLE_EVERY): findings are a sample's"));
    }
    let constants = Own::constants(&lowered.program);
    let mut own_totals = Own::default();
    db.execute("ATTACH ?1 AS x", [cross_path.to_string_lossy()])?;
    // Each base relation the cross-file part reads is kept in memory, across the chunks, as the columns its rules
    // name (and those decoding a finding's location reads); the others are never read. (Was: written to `x` and
    // read back, 0.75 s on Python.)
    let decodes = cross.program.outputs.iter().any(|o| o == "finding");
    let mut stored: Vec<(String, Vec<String>)> = Vec::new();
    for rel in split.base.iter().filter(|rel| !references::RELATIONS.contains(&rel.as_str())) {
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
        stored.push((rel.clone(), named.iter().map(|c| (*c).to_owned()).collect()));
    }
    for rel in &split.exports {
        let width = lowered.program.decls[rel].len();
        let cols: Vec<String> = (0..width).map(|i| format!("c{i}")).collect();
        db.execute_batch(&format!("CREATE TABLE x.{rel}({}, PRIMARY KEY ({})) WITHOUT ROWID", cols.iter().map(|c| format!("{c} INTEGER")).collect::<Vec<_>>().join(", "), cols.join(", ")))?;
    }
    phase("stats", statements.count());
    statements.enter("parse");
    let modules = Modules::default();
    let mut base_rows: BTreeMap<String, (Vec<String>, Vec<Option<Vec<i64>>>)> = BTreeMap::new();
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
    // Chunk k+1 is built (read, parsed, `nearest` bound) on a worker thread while chunk k's file-local part runs
    // (E12, list step 24): at most one chunk waits built, so memory is two chunks'. The worker's CPU is measured on its
    // own thread and reported as `parse` and `batch`; the main thread's as the other phases.
    type Chunk = (usize, Vec<(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)>);
    let (send, chunks_in) = std::sync::mpsc::sync_channel::<Result<Chunk>>(1);
    let (mut parse_cpu, mut batch_cpu) = ((0.0, 0.0), (0.0, 0.0));
    let worker_cpu = std::thread::scope(|scope| -> Result<((f64, f64), (f64, f64))> {
    let worker = scope.spawn(|| {
        let (mut parse, mut batch) = ((0.0, 0.0), (0.0, 0.0));
        let add = |acc: &mut (f64, f64), from: (f64, f64)| {
            let now = thread_usage();
            acc.0 += now.0 - from.0;
            acc.1 += now.1 - from.1;
        };
        let mut pending = Vec::new();
        let mut pending_nodes = 0;
        let mut at = docs.iter().peekable();
        let mut t = thread_usage();
        let keep_tapes = syntax::TAPED.iter().any(|rel| read.contains(*rel));
        // A document's facts: a grammar file's tree, or a text file's text (`core::text`). A text file that is not
        // UTF-8 (an image, an archive) is no document.
        let build = |d: &i64, path: &String| -> Result<Option<syntax::Document>> {
            let _parse = crate::code_rule_dsl::logic::profile::span(|| "worker parse".into());
            let read = crate::code_rule_dsl::logic::profile::span(|| "read".into());
            let source = match std::fs::read_to_string(root.join(path)) {
                Ok(source) => source,
                Err(e) if e.kind() == std::io::ErrorKind::InvalidData && text_document(path) => return Ok(None),
                Err(e) => return Err(Error::Plan(format!("{path}: {e}"))),
            };
            drop(read);
            if text_document(path) {
                let facts = Arc::new(core::text(Arc::new(source)));
                return Ok(Some(syntax::Document { key: *d as u32, path: path.clone(), core: format!("/c#core:{path}"), digest: core::digest(&facts), facts, tape: None }));
            }
            let _build = crate::code_rule_dsl::logic::profile::span(|| "tree-sitter and facts".into());
            let file = FileInput::new(path.clone(), label.to_owned(), Arc::new(source), None, None, "core").map_err(|e| Error::Plan(format!("{path}: {e:?}")))?;
            let Some((facts, doc)) = core::build_with_tape(&file).map_err(|e| Error::Plan(format!("{path}: {e:?}")))? else {
                return Err(Error::Plan(format!("{path} did not parse")));
            };
            let facts = Arc::new(facts);
            if let Some(references) = &references {
                references.add(*d as u32, path, &file.source, &facts)?;
            }
            // W's values and f-string trees are read off the tape, in the file's context: kept only for a program that
            // reads them.
            let tape = keep_tapes.then(|| (doc, file));
            Ok(Some(syntax::Document { key: *d as u32, path: path.clone(), core: format!("/c#core:{path}"), digest: core::digest(&facts), facts, tape }))
        };
        let mut sent = false;
        while let Some((d, path)) = at.next() {
            match build(d, path) {
                Ok(Some(doc)) => {
                    pending_nodes += doc.facts.len();
                    pending.push(doc);
                }
                Ok(None) => {}
                Err(e) => {
                    let _ = send.send(Err(e));
                    sent = true;
                    break;
                }
            }
            if pending_nodes < chunk_nodes && at.peek().is_some() {
                continue;
            }
            add(&mut parse, t);
            t = thread_usage();
            let batch_span = crate::code_rule_dsl::logic::profile::span(|| "worker batch".into());
            let built = syntax::with_nearest(&store, std::mem::take(&mut pending), &sets, &vocabulary, &read, binding).map_err(|e| Error::Plan(e.to_string()));
            drop(batch_span);
            add(&mut batch, t);
            let n = std::mem::take(&mut pending_nodes);
            sent = true;
            if send.send(built.map(|sources| (n, sources))).is_err() {
                break;
            }
            t = thread_usage();
        }
        // A corpus with no document still runs, over empty base relations: its outputs are what the program derives
        // from none (no rows, for a rule that reads a document), not an error.
        if !sent {
            let built = syntax::with_nearest(&store, Vec::new(), &sets, &vocabulary, &read, binding).map_err(|e| Error::Plan(e.to_string()));
            let _ = send.send(built.map(|sources| (0, sources)));
        }
        drop(send);
        (parse, batch)
    });
    for built in chunks_in {
        let (chunk_nodes_built, sources) = built?;
        phase("wait", statements.count());
        statements.enter("local");
        nodes += chunk_nodes_built;
        chunks += 1;
        let mut stats = t_stats.clone();
        if own {
            let _measure = crate::code_rule_dsl::logic::profile::span(|| "measure constants".into());
            let chunk = Own::measure(&sources, &constants);
            chunk.apply(&mut stats);
            own_totals.add(&chunk);
        }
        for (rel, source) in sources {
            // A relation no rule reads is bound unmeasured: measuring reads every row, and a `py.*`
            // family's first read makes them (decision 0029).
            if !read.contains(&rel) {
                modules.bind(&db, &rel, source)?;
                bound.insert(rel);
                continue;
            }
            if let Some((_, names)) = stored.iter().find(|(r, _)| *r == rel) {
                let have = source.columns();
                let held = base_rows.entry(rel.clone()).or_insert_with(|| (have.clone(), vec![None; have.len()]));
                for name in names {
                    let c = have.iter().position(|h| h == name).ok_or_else(|| Error::Plan(format!("{rel} has no column {name}")))?;
                    let col = held.1[c].get_or_insert_with(Vec::new);
                    col.extend((0..source.len()).map(|r| source.value(r, c)));
                }
            }
            let measure = crate::code_rule_dsl::logic::profile::span(|| format!("measure {rel}"));
            let measured = syntax::statistics(source.as_ref());
            drop(measure);
            let made_here = |rel: &str| {
                rel.starts_with("nearest_")
                    || [crate::code_rule_dsl::logic::front::AXIS, crate::code_rule_dsl::logic::strings::RELATION, crate::code_rule_dsl::logic::strings::MATCHES, crate::code_rule_dsl::logic::strings::GROUPS]
                        .iter()
                        .any(|p| rel.starts_with(p))
            };
            if split.base.contains(&rel) && (own || made_here(&rel)) {
                totals.add(&rel, source.len(), &measured);
            }
            stats.measured(&rel, measured.into_iter().map(|(c, rows, ndv, sq, sqd)| (c, [rows, ndv, sq, sqd])), []);
            modules.bind(&db, &rel, source)?;
            bound.insert(rel);
        }
        let running = crate::code_rule_dsl::logic::profile::span(|| "run".into());
        let got = super::run_atoms_native_over(&local, &db, "main", stats, split.exports.iter().cloned().collect(), modes)?;
        drop(running);
        let kept = got.kept.clone();
        let outputs_of = |rel: &str| prog.outputs.iter().any(|o| o == rel);
        native.extend(gather(got, &mut outputs, &mut node_columns, &outputs_of).into_iter().map(|k| format!("local:{k}")));
        db.execute_batch("BEGIN")?;
        for rel in &split.exports {
            // Rows another chunk wrote too (a file-local relation without a location column) are kept once. A kept
            // export goes from the engine to `x` in one statement and is measured in place; one a SQL stratum
            // derived is in its table.
            let types: Vec<&str> = lowered.program.decls[rel].iter().map(|(_, t)| t.as_str()).collect();
            if kept.contains(rel) {
                let cols = (0..types.len()).map(|i| format!("c{i}")).collect::<Vec<_>>().join(", ");
                // `set_derived` gives the rows the engine derived beyond its table's; a relation of facts and rules
                // has its facts there (the engine read them as stored rows), so they go first.
                let mut inserted = 0;
                if lowered.program.facts.get(rel).is_some_and(|f| !f.is_empty()) {
                    inserted += db.execute(&format!("INSERT OR IGNORE INTO x.{rel} SELECT {cols} FROM temp.{rel}"), [])?;
                }
                inserted += db.execute(&format!("INSERT OR IGNORE INTO x.{rel} SELECT {cols} FROM set_derived WHERE rel = ?1"), [rel])?;
                let rows = crate::code_rule_dsl::logic::native::component::derived(&db, rel).ok_or_else(|| Error::Plan(format!("{rel} is not held")))?;
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
        statements.enter("wait");
    }
    worker.join().map_err(|_| Error::Plan("the chunk builder panicked".to_owned()))
    })?;
    (parse_cpu.0, parse_cpu.1, batch_cpu.0, batch_cpu.1) = (worker_cpu.0.0, worker_cpu.0.1, worker_cpu.1.0, worker_cpu.1.1);
    // The reference graph over every chunk's files, bound for the cross-file part with its statistics.
    if let Some(references) = references {
        for (rel, columns, values) in references.relations(&store, &read)? {
            let source = crate::code_rule_dsl::logic::native::component::columns_source(columns, values);
            totals.add(rel, source.len(), &syntax::statistics(source.as_ref()));
            modules.bind(&db, rel, source)?;
        }
    }
    statements.enter("stats");
    // The cross-file part's statistics: T's for the base relations, the chunks' totals for the rest.
    let mut cross_stats = t_stats;
    if own {
        own_totals.apply(&mut cross_stats);
    }
    totals.apply(&mut cross_stats);

    // The cross-file part reads every input in place from `main`: the base relations held in memory, and the
    // exports, read from `x` once each.
    for (rel, (columns, values)) in std::mem::take(&mut base_rows) {
        modules.bind(&db, &rel, crate::code_rule_dsl::logic::native::component::columns_source(columns, values))?;
    }
    for rel in &split.exports {
        let source = crate::code_rule_dsl::logic::native::component::table_source(&db, "x", rel).ok_or_else(|| Error::Plan(format!("x.{rel} is not a table of integers")))?;
        modules.bind(&db, rel, source)?;
    }
    phase("stats", statements.count());
    statements.enter("cross");
    let mut rounds = cross_rounds::Rounds::default();
    // `returns`, held for the typing outputs (`corpus::typing`).
    let wanted: BTreeSet<String> = cross.program.decls.contains_key("returns").then(|| "returns".to_owned()).into_iter().filter(|r| !prog.outputs.contains(r)).collect();
    let mut returns = Vec::new();
    if !cross.program.rules.is_empty() {
        let bounded = cross_rounds::bound(&cross.program)?;
        let got = if bounded.hops.is_empty() {
            rounds = cross_rounds::Rounds { run: 1, converged: true, rows: vec![] };
            super::run_atoms_native_over(&cross, &db, "main", cross_stats, wanted.clone(), modes)?
        } else {
            run_rounds(&db, &modules, part(bounded.program), &bounded.hops, &wanted, &cross_stats, modes, &mut rounds)?
        };
        if wanted.contains("returns") {
            returns = held_rows(&db, &got, "returns", 2)?;
        }
        let outputs_of = |rel: &str| prog.outputs.iter().any(|o| o == rel);
        native.extend(gather(got, &mut outputs, &mut node_columns, &outputs_of).into_iter().map(|k| format!("cross:{k}")));
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
                    // A node column holding no document's node (`unset`'s -1) is written as it is.
                    .map(|(i, v)| match v.parse::<i64>() {
                        Ok(v) if node_columns.contains(&i) && own && paths.contains_key(&(v >> 32)) => format!("{}:{}", paths[&(v >> 32)], v & 0xffff_ffff),
                        Ok(v) if node_columns.contains(&i) && !own && first.contains_key(&(v >> 32)) => (first[&(v >> 32)] + (v & 0xffff_ffff)).to_string(),
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
    if prog.decls.contains_key("declared_ret") {
        typing::outputs(typing, &db, &store, root, outputs.get("type_of"), &returns, &declared, &mut typed)?;
    }
    phase("write", statements.count());
    let rss = usage().2;
    phases.insert(1, ("parse (worker)", parse_cpu.0, parse_cpu.1, rss, 0));
    phases.insert(2, ("batch (worker)", batch_cpu.0, batch_cpu.1, rss, 0));
    let (tested, text_cpu) = *vocabulary.tested.lock().unwrap();
    drop(statements);
    crate::code_rule_dsl::logic::profile::decision("chunks", || format!("{chunks} chunks of about {chunk_nodes} nodes; the next chunk is parsed and batched on a worker while this one runs"));
    if profiling && let Some(dir) = crate::code_rule_dsl::logic::profile::directory() {
        let mut header = format!(
            "Profile of a corpus run: {} files, {nodes} nodes, {chunks} chunks; text tests {} relations, {tested} strings, {text_cpu:.3} s CPU\n\n== Phases (CPU = user + system; worker phases run beside the main thread)\n",
            docs.len(),
            vocabulary.tests.len()
        );
        for (name, user, sys, rss, n) in &phases {
            header.push_str(&format!("{:>9.2} s  {name:<16} rss {rss:.0} MB  {n} statements\n", user + sys));
        }
        header.push_str(&format!("\nProgram: {} lines; rules are numbered as in the `rule #k` notes at the end.\n", program.lines().filter(|l| !l.trim().is_empty()).count()));
        crate::code_rule_dsl::logic::profile::write(&dir, &header).map_err(|e| Error::Plan(format!("profile: {e}")))?;
    }
    Ok(Report { phases, files: docs.len(), nodes, native: native.len(), rows, chunks, rounds, typing: typed, text: (vocabulary.tests.len(), tested, text_cpu) })
}


/// The cross-file part's bounded rounds (`typing::cross_rounds`): `round` is the part with its hops reading
/// `<rel>__prev`, a table in `x` that holds the previous round's rows (none in round 0). Each round's rows of the
/// hop relations are digested; an unchanged digest ends the rounds (the fixpoint), as does round `K`. The last
/// round's outcome is the part's.
/// A relation's rows after a run: held by the engine (`kept`), or in its table.
fn held_rows(db: &Connection, got: &Outcome, rel: &str, width: usize) -> Result<Vec<Vec<i64>>> {
    use crate::code_rule_dsl::logic::native::component;
    let source: Arc<dyn crate::code_rule_dsl::facts::vtab::Source> = if got.kept.contains(rel) {
        component::derived(db, rel).ok_or_else(|| Error::Plan(format!("{rel} is not held")))?
    } else {
        Arc::new(Rows::read(db, rel)?)
    };
    Ok((0..source.len()).map(|i| (0..width).map(|c| source.value(i, c)).collect()).collect())
}

#[allow(clippy::too_many_arguments)]
fn run_rounds(db: &Connection, modules: &Modules, round: lower::Lowered, hops: &BTreeSet<String>, wanted: &BTreeSet<String>, stats: &super::Stats, modes: &str, report: &mut cross_rounds::Rounds) -> Result<Outcome> {
    use crate::code_rule_dsl::logic::native::component;
    let types = |rel: &str| -> Vec<String> { round.program.decls[rel].iter().map(|(_, t)| t.clone()).collect() };
    for rel in hops {
        let cols: Vec<String> = (0..types(rel).len()).map(|i| format!("c{i}")).collect();
        db.execute_batch(&format!(
            "DROP TABLE IF EXISTS x.{rel}{p}; CREATE TABLE x.{rel}{p}({}, PRIMARY KEY ({})) WITHOUT ROWID",
            cols.iter().map(|c| format!("{c} INTEGER")).collect::<Vec<_>>().join(", "),
            cols.join(", "),
            p = cross_rounds::PREV
        ))?;
    }
    // Round 0's previous rows: none, measured as such.
    let mut prev_totals = Totals::default();
    for rel in hops {
        let (t, width) = (types(rel), types(rel).len());
        let empty = Rows { columns: (0..width).map(|i| format!("c{i}")).collect(), values: vec![Vec::new(); width] };
        prev_totals.add_rows(&format!("{rel}{}", cross_rounds::PREV), 0, &empty, &t.iter().map(String::as_str).collect::<Vec<_>>());
    }
    let mut last = None;
    let k = cross_rounds::k();
    for r in 0..=k {
        for rel in hops {
            let prev = format!("{rel}{}", cross_rounds::PREV);
            let source = component::table_source(db, "x", &prev).ok_or_else(|| Error::Plan(format!("x.{prev} is not a table of integers")))?;
            modules.bind(db, &prev, source)?;
        }
        let mut st = stats.clone();
        prev_totals.apply(&mut st);
        // A hop relation the program outputs is written to its table, to be decoded; the others are held.
        let held: BTreeSet<String> = hops.iter().filter(|r| !round.program.outputs.contains(r)).chain(wanted).cloned().collect();
        let got = super::run_atoms_native_over(&round, db, "main", st, held, modes)?;
        // This round's rows of the hop relations: from the engine, or their table.
        let mut rows: BTreeMap<String, Vec<Vec<i64>>> = BTreeMap::new();
        prev_totals = Totals::default();
        for rel in hops {
            let width = types(rel).len();
            let source: Arc<dyn crate::code_rule_dsl::facts::vtab::Source> = if got.kept.contains(rel) {
                component::derived(db, rel).ok_or_else(|| Error::Plan(format!("{rel} is not held")))?
            } else {
                Arc::new(Rows::read(db, rel)?)
            };
            let t = types(rel);
            let t: Vec<&str> = t.iter().map(String::as_str).collect();
            prev_totals.add_rows(&format!("{rel}{}", cross_rounds::PREV), source.len(), source.as_ref(), &t);
            rows.insert(rel.clone(), (0..source.len()).map(|i| (0..width).map(|c| source.value(i, c)).collect()).collect());
        }
        let digest = cross_rounds::digest(&rows);
        report.run = r + 1;
        report.rows.push(rows.values().map(Vec::len).sum());
        if last == Some(digest) || r == k {
            report.converged = last == Some(digest);
            return Ok(got);
        }
        last = Some(digest);
        db.execute_batch("BEGIN")?;
        for (rel, rows) in &rows {
            let prev = format!("{rel}{}", cross_rounds::PREV);
            db.execute(&format!("DELETE FROM x.{prev}"), [])?;
            let width = types(rel).len();
            let mut ins = db.prepare(&format!("INSERT OR IGNORE INTO x.{prev} VALUES ({})", vec!["?"; width].join(", ")))?;
            for row in rows {
                ins.execute(rusqlite::params_from_iter(row.iter()))?;
            }
        }
        db.execute_batch("COMMIT")?;
        crate::code_rule_dsl::logic::native::forget_inputs(db);
    }
    unreachable!("round K returns")
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
    /// The profile's span of the phase entered last (`logic::profile`).
    span: std::cell::RefCell<Option<crate::code_rule_dsl::logic::profile::Span>>,
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
            let profiled = crate::code_rule_dsl::logic::profile::on();
            if state.costs.is_none() && !profiled {
                return 0;
            }
            let stmt = p.cast::<ffi::sqlite3_stmt>();
            // SAFETY: for SQLITE_TRACE_PROFILE, P is the statement and X its time in nanoseconds.
            let (ns, text, vm, scan, sort, auto) = unsafe {
                let ns = *x.cast::<i64>() as u64;
                let text = std::ffi::CStr::from_ptr(ffi::sqlite3_sql(stmt)).to_string_lossy().split_whitespace().collect::<Vec<_>>().join(" ");
                let st = |op| ffi::sqlite3_stmt_status(stmt, op, 1) as u64;
                (ns, text, st(ffi::SQLITE_STMTSTATUS_VM_STEP), st(ffi::SQLITE_STMTSTATUS_FULLSCAN_STEP), st(ffi::SQLITE_STMTSTATUS_SORT), st(ffi::SQLITE_STMTSTATUS_AUTOINDEX))
            };
            if profiled {
                crate::code_rule_dsl::logic::profile::sql(text.clone(), ns, vm, scan, sort, auto);
            }
            let Some(costs) = &state.costs else { return 0 };
            let phase = *state.phase.lock();
            let mut costs = costs.lock();
            let c = costs.entry((phase, text)).or_default();
            for (a, b) in c.iter_mut().zip([1, ns, vm, scan, sort, auto]) {
                *a += b;
            }
            0
        }
        let state = Box::new(StatementState { costs: std::env::var_os("RDL_STMTS").map(|_| Default::default()), ..Default::default() });
        let profile = state.costs.is_some() || crate::code_rule_dsl::logic::profile::on();
        let mask = ffi::SQLITE_TRACE_STMT as u32 | if profile { ffi::SQLITE_TRACE_PROFILE as u32 } else { 0 };
        // SAFETY: the connection is live for the run; the state is boxed (a stable address) and outlives the callback.
        let db = unsafe { db.handle() };
        unsafe { ffi::sqlite3_trace_v2(db, mask, Some(trace), std::ptr::from_ref(&*state).cast_mut().cast()) };
        Statements { db, state, span: Default::default() }
    }
    fn count(&self) -> usize {
        self.state.count.load(std::sync::atomic::Ordering::Relaxed)
    }
    /// The phase the statements from now on are charged to.
    fn enter(&self, phase: &'static str) {
        *self.state.phase.lock() = phase;
        // The last phase's span closes before this one opens (spans nest by a stack).
        drop(self.span.borrow_mut().take());
        *self.span.borrow_mut() = Some(crate::code_rule_dsl::logic::profile::span(|| phase.to_owned()));
    }
}
impl Drop for Statements {
    fn drop(&mut self) {
        drop(self.span.borrow_mut().take());
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


/// The suffixes of the files the corpus runner reads as text only (core.rr's `.scope text`): documentation and
/// configuration, never a run's own outputs (`.csv`, `.sqlite`, which may lie under the root) or binaries.
const TEXT_SUFFIXES: [&str; 9] = ["md", "markdown", "rst", "txt", "toml", "cfg", "ini", "yaml", "yml"];

/// Whether the corpus runner reads the file at `path` as text only: no dialect has its suffix and it is one of
/// `TEXT_SUFFIXES`, so it has `doc` and the source text's relations and no tree.
fn text_document(path: &str) -> bool {
    crate::code_rule_dsl::language::label_of_path(path).is_none()
        && Path::new(path).extension().and_then(|e| e.to_str()).is_some_and(|e| TEXT_SUFFIXES.contains(&e.to_ascii_lowercase().as_str()))
}

/// The corpus's files under `root`, relative and in path order: Python's (`.py`), Rust's (`.rs`) and the text-only
/// documents (`text_document`, hidden files left out), skipping hidden directories and `target`. A file of another
/// grammar is none of them.
fn corpus_files(root: &Path) -> Result<(Vec<String>, Vec<String>, Vec<String>)> {
    let (mut python, mut rust, mut text) = (Vec::new(), Vec::new(), Vec::new());
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).map_err(|e| Error::Plan(format!("{}: {e}", dir.display())))? {
            let path = entry.map_err(|e| Error::Plan(e.to_string()))?.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if path.is_dir() {
                if !name.starts_with('.') && name != "target" {
                    pending.push(path);
                }
                continue;
            }
            if !path.is_file() {
                continue;
            }
            let relative = path.strip_prefix(root).map_err(|e| Error::Plan(e.to_string()))?.to_string_lossy().into_owned();
            match path.extension().and_then(|e| e.to_str()) {
                Some("py") => python.push(relative),
                Some("rs") => rust.push(relative),
                _ if !name.starts_with('.') && text_document(&relative) => text.push(relative),
                _ => {}
            }
        }
    }
    python.sort();
    rust.sort();
    text.sort();
    // `RERAIL_SAMPLE_EVERY=n`: every n-th file only, to iterate on a rule's cost quickly (its findings are a sample's).
    if let Some(n) = std::env::var("RERAIL_SAMPLE_EVERY").ok().and_then(|n| n.parse::<usize>().ok()).filter(|n| *n > 1) {
        for files in [&mut python, &mut rust, &mut text] {
            *files = files.iter().step_by(n).cloned().collect();
        }
    }
    Ok((python, rust, text))
}

/// Makes the base relations `prog` reads and does not derive inputs (`select::input_columns` names a base relation).
fn base_inputs(prog: &mut front::Program) {
    let read: Vec<String> = {
        let derived: BTreeSet<&str> = prog.rules.iter().map(|r| r.head.rel.as_str()).chain(prog.facts.keys().map(String::as_str)).collect();
        prog.rules
            .iter()
            .flat_map(|r| r.body.iter().filter_map(front::Literal::atom))
            .map(|a| a.rel.as_str())
            .filter(|rel| !derived.contains(rel) && prog.decls.contains_key(*rel) && super::super::sql::select::input_columns(rel).is_some())
            .map(str::to_owned)
            .collect()
    };
    for rel in read {
        if !prog.inputs.contains(&rel) {
            prog.inputs.push(rel);
        }
    }
}

/// The base relations' statistics a chunk measures as it binds them, beyond each column's moments
/// (`syntax::statistics`), which T's `stats.py` made at ingestion: per kind atom its nodes and summed subtree sizes
/// (`stat_kind`), per (parent kind, child kind) its edges (`stat_edge`), the documents, and per (relation, column,
/// constant) of the program's constants its rows (`stat_val`'s `n`). Summed over the chunks for the cross-file part:
/// a document is in one chunk, so every count adds exactly.
#[derive(Default)]
struct Own {
    kinds: BTreeMap<i64, (i64, i64)>,
    edges: BTreeMap<(i64, i64), i64>,
    docs: i64,
    postings: BTreeMap<(String, String, i64), i64>,
}

impl Own {
    /// The program's integer constants by (input relation, column name) they are compared at.
    fn constants(prog: &front::Program) -> BTreeMap<(String, String), BTreeSet<i64>> {
        let mut out: BTreeMap<(String, String), BTreeSet<i64>> = BTreeMap::new();
        for literal in prog.rules.iter().flat_map(|r| &r.body) {
            let Some(atom) = literal.atom() else { continue };
            if !prog.inputs.contains(&atom.rel) {
                continue;
            }
            for (i, arg) in atom.args.iter().enumerate() {
                if let front::Term::Int(v) = arg {
                    out.entry((atom.rel.clone(), super::sql_col(&atom.rel, i))).or_default().insert(*v);
                }
            }
        }
        out
    }

    fn measure(sources: &[(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)], constants: &BTreeMap<(String, String), BTreeSet<i64>>) -> Self {
        let mut own = Own::default();
        let of = |rel: &str| sources.iter().find(|(r, _)| r == rel).map(|(_, s)| s.as_ref());
        let column = |source: &dyn crate::code_rule_dsl::facts::vtab::Source, name: &str| source.columns().iter().position(|c| c == name);
        // Kinds by node id, then the tape's sizes and parents against them.
        let mut kind_of = HashMap::new();
        if let Some(node) = of("node")
            && let (Some(n), Some(k), Some(d)) = (column(node, "n"), column(node, "kind"), column(node, "d"))
        {
            let mut docs = BTreeSet::new();
            for r in 0..node.len() {
                let kind = node.value(r, k);
                kind_of.insert(node.value(r, n), kind);
                own.kinds.entry(kind).or_default().0 += 1;
                docs.insert(node.value(r, d));
            }
            own.docs = docs.len() as i64;
        }
        if let Some(tape) = of("tape")
            && let (Some(n), Some(par), Some(size)) = (column(tape, "n"), column(tape, "par"), column(tape, "size"))
        {
            for r in 0..tape.len() {
                let Some(&kind) = kind_of.get(&tape.value(r, n)) else { continue };
                own.kinds.entry(kind).or_default().1 += tape.value(r, size);
                if let Some(&parent) = kind_of.get(&tape.value(r, par)) {
                    *own.edges.entry((parent, kind)).or_default() += 1;
                }
            }
        }
        for ((rel, col), values) in constants {
            let Some(source) = of(rel) else { continue };
            let Some(c) = column(source, col) else { continue };
            for r in 0..source.len() {
                let v = source.value(r, c);
                if values.contains(&v) {
                    *own.postings.entry((rel.clone(), col.clone(), v)).or_default() += 1;
                }
            }
        }
        own
    }

    fn add(&mut self, other: &Own) {
        for (k, (n, size)) in &other.kinds {
            let e = self.kinds.entry(*k).or_default();
            e.0 += n;
            e.1 += size;
        }
        for (e, n) in &other.edges {
            *self.edges.entry(*e).or_default() += n;
        }
        self.docs += other.docs;
        for (key, n) in &other.postings {
            *self.postings.entry(key.clone()).or_default() += n;
        }
    }

    fn apply(&self, stats: &mut super::Stats) {
        stats.tree(&self.kinds, &self.edges, self.docs);
        let mut by: BTreeMap<&str, Vec<(String, i64, i64)>> = BTreeMap::new();
        for ((rel, col, v), n) in &self.postings {
            by.entry(rel.as_str()).or_default().push((col.clone(), *v, *n));
        }
        for (rel, vals) in by {
            stats.measured(rel, [], vals);
        }
    }
}
