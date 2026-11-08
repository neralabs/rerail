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

use crate::code_rule_dsl::facts::core::{self, Built};
use crate::code_rule_dsl::facts::vtab::{Modules, syntax};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::logic::front;
use crate::code_rule_dsl::logic::rr::WORLD;
use crate::code_rule_dsl::model::FileInput;
use crate::code_rule_dsl::typing::cross_rounds;
pub(in crate::code_rule_dsl) mod guests;
mod hiding;
mod scoping;
pub(in crate::code_rule_dsl) use hiding::qualified;
pub(in crate::code_rule_dsl) mod memo;
mod prepared;
mod maintain;
mod retain;
mod bridge;
pub(crate) mod phases;
mod inputs;
pub(in crate::code_rule_dsl) use inputs::parameters as input_parameters;
mod world;
mod cross_memo;
mod typing;
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
        ("text" | "op" | "op_at" | "path_part" | "file_name" | "value", "s") | ("doc", "path" | "module") | ("pkg", "p") => Domain::Name,
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
    // The browser build's one thread (`prof::single_thread_cpu`), all of it user time.
    #[cfg(target_family = "wasm")]
    {
        (crate::code_rule_dsl::prof::single_thread_cpu().as_secs_f64(), 0.0)
    }
    #[cfg(not(any(target_os = "macos", target_family = "wasm")))]
    {
        let mut u: libc::rusage = unsafe { std::mem::zeroed() };
        unsafe { libc::getrusage(libc::RUSAGE_THREAD, &mut u) };
        let s = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
        (s(u.ru_utime), s(u.ru_stime))
    }
}

pub(crate) fn usage() -> (f64, f64, f64) {
    // wasm32: the one thread's time as user CPU, and no resident-set counter.
    #[cfg(target_family = "wasm")]
    {
        (crate::code_rule_dsl::prof::single_thread_cpu().as_secs_f64(), 0.0, 0.0)
    }
    #[cfg(not(target_family = "wasm"))]
    {
        let mut u: libc::rusage = unsafe { std::mem::zeroed() };
        unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut u) };
        let s = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
        // macOS reports ru_maxrss in bytes, Linux in kilobytes.
        let rss = if cfg!(target_os = "macos") { u.ru_maxrss as f64 / 1e6 } else { u.ru_maxrss as f64 / 1e3 };
        (s(u.ru_utime), s(u.ru_stime), rss)
    }
}

/// What a corpus run did: its phases' CPU (user, system) and the peak RSS after each, and the outcome.
#[derive(Default)]
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
    /// The typing programs run for the rule's bridged relations (`bridge`), with their reports.
    pub(crate) typing_runs: bridge::Runs,
    /// The earlier phases this run read (decision 0083 §1), each its own run.
    pub(crate) phase_runs: Vec<phases::PhaseRun>,
    /// The per-file memo: files read from it, and chunks whose rows it could not keep (a row naming no document).
    pub(crate) memo: (usize, usize),
    /// The cross-file memo (decision 0076): import components found and components derived.
    pub(crate) cross: (usize, usize),
    /// Where the prepared program came from: `prepared` by this run, kept in this process's `memory`, or read from the
    /// `store` (decision 0073).
    pub(crate) prepared: &'static str,
    /// A session's run: the cross-file strata it evaluated, of all (decision 0077; `None` outside a session).
    pub(crate) cross_strata: Option<(usize, usize)>,
    /// Of those, strata maintained from deltas, of them recursive, and strata derived again (decision 0077 §3).
    pub(crate) cross_maintained: (usize, usize, usize),
    /// A session's run: the cross part's inputs that changed, and why its strata derived again were (each count).
    pub(crate) cross_changed: Vec<String>,
    pub(crate) cross_again: BTreeMap<&'static str, usize>,
    /// With `RERAIL_CROSS_DELTA_COUNTS`: rows over-deleted, inserted and held by the maintained relations, the rows of
    /// the deltas of their inputs, and the rows truly deleted and inserted.
    pub(crate) cross_delta_counts: [usize; 6],
    /// What the run consulted, as (what, path, digest): each workspace document's bytes, and what its requests read
    /// (`world::World::consulted`). A run whose consulted things are as they were derives the same (`unchanged`).
    pub(crate) consulted: Vec<(String, String, String)>,
    /// What phase 0's observer did during the run: stats, answers reused, files hashed, directories listed.
    pub(crate) files_observed: crate::content_store::observe::Counts,
}

/// Whether a run of `programs` over `root` that consulted `consulted` (`Report::consulted`) would consult the same
/// now: the same workspace documents with the same bytes, and every request answered alike (decision 0083 §2). It
/// reads what the run read, and derives nothing. A program whose inputs come from elsewhere too (declarations,
/// earlier phases, an explicit batch) is never judged unchanged.
pub(crate) fn unchanged(programs: &[&str], root: &Path, typing: &Typing, options: &serde_json::Value, consulted: &[(String, String, String)]) -> Result<bool> {
    if options.get("inputFiles").is_some() || consulted.is_empty() {
        return Ok(false);
    }
    let flags: Vec<String> = options.get("env").and_then(|v| v.as_array()).into_iter().flatten().filter_map(|v| v.as_str().map(str::to_owned)).collect();
    let settings = crate::code_rule_dsl::tools::settings(root, &flags).map_err(Error::Plan)?;
    let with_settings: Vec<String> = programs.iter().map(|p| crate::code_rule_dsl::tools::with_settings(p, &settings)).collect();
    let programs: Vec<&str> = with_settings.iter().map(String::as_str).collect();
    let prog = combine(&programs, "")?;
    if prog.decls.contains_key("declared_ret") || !phases::wanted(&prog, None)?.is_empty() {
        return Ok(false);
    }
    let language = prog.settings.iter().find(|(key, _)| key == "language").map(|(_, value)| value.trim().trim_matches('"').to_owned());
    let found = scoped(corpus_files(root)?, &prog, options);
    let label = corpus_language(language.as_deref(), typing, &found);
    let files = found.of(label);
    let docs: Vec<&(String, String, String)> = consulted.iter().filter(|c| c.0 == "doc").collect();
    if files.len() != docs.len() || files.iter().zip(&docs).any(|(f, d)| *f != d.1 || world::bytes_digest(&root.join(f)) != d.2) {
        return Ok(false);
    }
    let requests: Vec<(String, String, String)> = consulted.iter().filter(|c| c.0 != "doc").cloned().collect();
    Ok(world::World::new(root, &prog).still(&requests))
}

use std::collections::{BTreeMap, BTreeSet};

pub(in crate::code_rule_dsl) mod session;

/// Runs `program` (`.rr` text, `logic::rr`) over the files T's `t_facts` names under `root`, in native `modes`, with `nearest`
/// bound as `binding`, and writes its outputs into `out` as T does. The store and its derived tables live in `out`.
///
/// The program is split at its file-local relations (`typing::file_local::programs`, list step 29). Documents are
/// taken in chunks of about `chunk_nodes` nodes, each built on a worker thread while the previous one runs (list
/// step 24): its file-local part is run, the exported relations are appended to `cross.sqlite` in `out` (schema `x`),
/// the base columns the cross-file part names are appended in memory, and the chunk is dropped. The cross-file part
/// runs once, over the base relations held in memory and the exports read once from `x`, all bound in `main`. Memory
/// is two chunks', the held base columns, and the cross-file part's own.
/// Repository readers publish typed rows once, before file-local partitioning.
fn external_inputs(prog: &mut front::Program, root: &Path) -> Result<()> {
    inputs::populate(prog, root)?;
    // `core.rr` relations the engine does not make yet, read as empty, each named in the profile (open item O76).
    for (rel, what) in NOT_MADE {
        if prog.inputs.iter().any(|i| i == rel) {
            crate::code_rule_dsl::logic::profile::decision("relations not made yet (O76)", || format!("{rel} is empty: {what}"));
            prog.inputs.retain(|i| i != rel);
        }
    }
    Ok(())
}

/// A chunk's mix, for which statistics measured on one chunk stand for another, scaled by nodes: the file types
/// (extensions) holding a tenth of its nodes or more, in order, and its files' size, as the power of 4 of its mean
/// nodes per file. Per-document relations (`doc`, `path_part`) grow with files, not nodes, so a chunk of many small
/// files is not a chunk of a few large ones scaled. Documents are `(path, nodes)`.
fn chunk_mix<'a>(docs: impl Iterator<Item = (&'a str, usize)>) -> String {
    let mut by: BTreeMap<&str, usize> = BTreeMap::new();
    let mut files = 0;
    for (path, n) in docs {
        let name = path.rsplit('/').next().unwrap_or(path);
        *by.entry(name.rsplit_once('.').map_or("", |(_, ext)| ext)).or_default() += n.max(1);
        files += 1;
    }
    let total: usize = by.values().sum();
    let size = (total / files.max(1)).max(1).ilog2() / 2;
    let types = by.into_iter().filter(|(_, n)| n * 10 >= total).map(|(ext, _)| ext).collect::<Vec<_>>().join(",");
    format!("{types}/{size}")
}

/// The `core.rr` relations a rule may read that the engine does not make yet (O76): each is empty.
const NOT_MADE: [(&str, &str); 1] = [("string_annotation", "Ruff's table of the call positions that take a type as a string is not built")];

/// The embed slice of a lowered program (decisions 0055, 0058): the rules its requests need (`embed`, `embed_value`
/// and the demands), with those as its outputs; `None` when no rule makes a request. It is cut from the whole
/// program, not its file-local part, and runs over each chunk: a guest or a value is made from its chunk's
/// documents, so a demand whose context the split would run cross-file (`q(_, x)`, a value's nodes) is still asked
/// in the chunk.
fn embed_slice(local: &front::Program) -> Option<front::Program> {
    let wanted: Vec<&str> = guests::LIBRARY.iter().copied().chain(guests::DEMANDS).filter(|r| local.rules.iter().any(|x| x.head.rel == *r)).collect();
    if wanted.is_empty() {
        return None;
    }
    let mut need: BTreeSet<String> = wanted.iter().map(|r| (*r).to_owned()).collect();
    loop {
        let more: BTreeSet<String> = local
            .rules
            .iter()
            .filter(|r| need.contains(&r.head.rel))
            .flat_map(|r| r.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.clone()))
            .filter(|rel| !need.contains(rel))
            .collect();
        if more.is_empty() {
            break;
        }
        need.extend(more);
    }
    let mut slice = local.clone();
    slice.rules.retain(|r| need.contains(&r.head.rel));
    slice.outputs = wanted.iter().map(|r| (*r).to_owned()).collect();
    Some(slice)
}

pub(crate) fn run(program: &str, root: &Path, t_facts: Option<&Path>, out: &Path, modes: &str, binding: Binding, chunk_nodes: usize) -> Result<Report> {
    run_typed(&[program], root, t_facts, out, modes, binding, chunk_nodes, &Typing::default())
}

/// Every program of `programs` in one run: each file is read, parsed and batched once, and every rule runs over each
/// chunk (`combine`).
pub(crate) fn run_all(programs: &[&str], root: &Path, out: &Path, modes: &str, binding: Binding, chunk_nodes: usize) -> Result<Report> {
    run_typed(programs, root, None, out, modes, binding, chunk_nodes, &Typing::default())
}

/// The programs `programs` (`.rr` text, each with `extra` after it) as one program. One program is parsed as it is.
/// Of several, each one's own relations (the ones it derives or declares beyond its inputs) take a prefix, `p<i>_`,
/// so programs that name relations alike (a converter's `m650`, `exists_0`) do not meet. The base inputs and the
/// `violation` output are shared, and kind sets are named by their kinds (`ks<j>`), so programs asking for one set
/// share its tape pass. The simplifier then merges relations that are equal across programs (decision 0038).
pub(crate) fn combine(programs: &[&str], extra: &str) -> Result<front::Program> {
    use std::sync::{Mutex, OnceLock};
    // A function of the texts alone (`rr::parse` reads no file), so kept for the process by their digest: a session or
    // a test that runs one program over many roots parses and hides it once. A few programs at a time are kept.
    static COMBINED: OnceLock<Mutex<HashMap<[u8; 32], front::Program>>> = OnceLock::new();
    const KEPT: usize = 32;
    let key = texts_digest(programs, extra);
    let cache = COMBINED.get_or_init(Default::default);
    if let Some(program) = cache.lock().expect("the combined programs").get(&key) {
        crate::code_rule_dsl::prof::count("corpus.combined_reused", 1);
        return Ok(program.clone());
    }
    let program = combine_texts(programs, extra)?;
    let mut kept = cache.lock().expect("the combined programs");
    if kept.len() >= KEPT {
        kept.clear();
    }
    kept.insert(key, program.clone());
    Ok(program)
}

/// The digest of `combine`'s texts: what a combined program is a function of.
fn texts_digest(programs: &[&str], extra: &str) -> [u8; 32] {
    use sha2::Digest as _;
    let mut hash = sha2::Sha256::new();
    // Without decision 0091's guards (`scoping`, a measurement's switch) the same texts combine to another program.
    if programs.len() > 1 && std::env::var("RERAIL_SCOPE_GUARDS").is_ok_and(|v| v == "off") {
        hash.update(b"scope guards off");
    }
    for text in programs.iter().copied().chain([extra]) {
        hash.update((text.len() as u64).to_le_bytes());
        hash.update(text.as_bytes());
    }
    hash.finalize().into()
}

/// A program prepared for a run (`run_with_options`): pruned, simplified, demanded, lowered, its joins pushed into the
/// chunks, located and split. A function of the combined program's texts, what the run added to it (facts, inputs,
/// outputs), the stage and the dialect; and of the store's atoms, so `interned` is every intern call lowering made,
/// asked again of the store on reuse.
#[derive(Clone)]
struct Prepared {
    prog: front::Program,
    vocabulary: Arc<crate::code_rule_dsl::logic::strings::Vocabulary>,
    lowered: lower::Lowered,
    sets: Vec<(String, Vec<i64>)>,
    demanded: Vec<String>,
    pushed: Vec<String>,
    located: std::result::Result<Vec<String>, Vec<String>>,
    merged: (usize, usize),
    split: crate::code_rule_dsl::typing::file_local::Programs,
    interned: Vec<(Vec<String>, Vec<u32>)>,
}

fn prepared_cache() -> &'static std::sync::Mutex<HashMap<[u8; 32], Prepared>> {
    static PREPARED: std::sync::OnceLock<std::sync::Mutex<HashMap<[u8; 32], Prepared>>> = std::sync::OnceLock::new();
    PREPARED.get_or_init(Default::default)
}

/// A few prepared programs at a time are kept.
const PREPARED_KEPT: usize = 16;

fn combine_texts(programs: &[&str], extra: &str) -> Result<front::Program> {
    use front::{Literal, Program};
    // A program reads only relations it declares: an error here, not the planner's panic later (`front::declared`).
    let parse = |text: &str| -> Result<Program> {
        let prog = crate::code_rule_dsl::logic::rr::parse(&if extra.is_empty() { text.to_owned() } else { format!("{text}\n{extra}") })?;
        front::declared(&prog)?;
        Ok(prog)
    };
    let parsing = crate::code_rule_dsl::logic::profile::span(|| format!("parse each program ({})", programs.len()));
    let mut parsed = programs.iter().map(|text| parse(text)).collect::<std::result::Result<Vec<_>, _>>()?;
    // `RERAIL_DUMP_PROGRAMS=<file>`: each program's prefix (`p<i>_`) and its first policy or line (a debugging aid).
    if let Ok(path) = std::env::var("RERAIL_DUMP_PROGRAMS") {
        let lines: String = programs
            .iter()
            .enumerate()
            .map(|(i, text)| {
                let first = text.lines().find(|l| l.trim_start().starts_with(".policy")).or_else(|| text.lines().find(|l| !l.trim().is_empty())).unwrap_or("");
                format!("p{i}_\t{}\n", first.trim())
            })
            .collect();
        let _ = std::fs::write(path, lines);
    }
    // Each program's library modules (decision 0089), when it names them by `#include`.
    let modular: Vec<Option<crate::code_rule_dsl::logic::rr::Modules>> = if programs.len() > 1 {
        programs.iter().map(|text| crate::code_rule_dsl::logic::rr::modules(text)).collect::<std::result::Result<_, _>>()?
    } else {
        vec![None]
    };
    drop(parsing);
    let makes_guests = parsed.iter().any(|p| p.rules.iter().any(|r| guests::REQUESTS.contains(&r.head.rel.as_str())));
    let bounds: Vec<u32> = parsed.iter().map(hiding::bound).collect();
    // The libraries once (decision 0089): two or more programs name theirs as modules, and all are read under one
    // bound of the documents they show, so the library's relations are the same for each.
    // `RERAIL_LIBRARIES=copied` copies them into each program, as before, to compare.
    let copied = std::env::var("RERAIL_LIBRARIES").is_ok_and(|v| v == "copied");
    let shared = !copied && modular.iter().flatten().count() > 1 && bounds.windows(2).all(|w| w[0] == w[1]);
    crate::code_rule_dsl::logic::profile::decision("libraries (decision 0089)", || {
        if shared {
            format!("held once: {} of {} programs name their libraries as modules", modular.iter().flatten().count(), programs.len())
        } else if copied {
            "copied into each program (RERAIL_LIBRARIES=copied)".to_owned()
        } else if modular.iter().flatten().count() > 1 {
            "copied into each program: the programs show different documents (`.guests`)".to_owned()
        } else {
            "copied into each program: fewer than two programs name their libraries as modules".to_owned()
        }
    });
    let mut library: Option<Program> = None;
    if shared {
        let _library = crate::code_rule_dsl::logic::profile::span(|| "the libraries, once (decision 0089)".into());
        let core = crate::code_rule_dsl::logic::rr::core_names();
        let mut paths: Vec<std::path::PathBuf> = Vec::new();
        let mut declared_in: BTreeMap<&str, &std::path::Path> = BTreeMap::new();
        for modules in modular.iter().flatten() {
            for (path, _, names) in &modules.files {
                if paths.contains(path) {
                    continue;
                }
                for name in names.iter().filter(|n| !core.contains(*n)) {
                    if let Some(other) = declared_in.insert(name, path) {
                        return Err(Error::Plan(format!("{name} is declared by two library files of the run, {} and {}", other.display(), path.display())));
                    }
                }
                paths.push(path.clone());
            }
        }
        for (i, modules) in modular.iter().enumerate() {
            let Some(modules) = modules else { continue };
            for name in &modules.declared {
                if let Some(file) = modules.files.iter().find(|(_, _, names)| names.contains(name) && !core.contains(name)) {
                    return Err(Error::Plan(format!("program #{i} declares {name}, which {} it includes declares", file.0.display())));
                }
            }
        }
        library = Some(crate::code_rule_dsl::logic::rr::lower_library(&paths)?);
        let mut cones = 0;
        let mut kept = 0;
        for (prog, modules) in parsed.iter_mut().zip(&modular) {
            if let Some(modules) = modules {
                let (dropped, cone) = own_part(prog, modules, core);
                kept += dropped;
                cones += cone;
            }
        }
        let rules = library.as_ref().map_or(0, |l| l.rules.len());
        crate::code_rule_dsl::logic::profile::decision("library relations", || {
            format!("{rules} library rules once; {kept} relations taken from the library instead of a program's copy; {cones} relations each copied for the program whose clauses add to them (a request's cone, §3)")
        });
    }
    // A run that makes guests bounds each program's rules to the documents it shows (decision 0058 §1); a run that
    // makes none is left as written. With the libraries once, the combined program is bounded once (below).
    if makes_guests && !shared {
        let _hiding = crate::code_rule_dsl::logic::profile::span(|| {
            format!("bound each program to the documents it shows ({} programs, {} rules; decision 0058)", parsed.len(), parsed.iter().map(|p| p.rules.len()).sum::<usize>())
        });
        for (i, p) in parsed.iter_mut().enumerate() {
            let _one = crate::code_rule_dsl::logic::profile::span(|| format!("program #{i} ({} rules)", p.rules.len()));
            let bound = hiding::bound(p);
            hiding::hide(p, bound);
        }
    }
    let _merging = crate::code_rule_dsl::logic::profile::span(|| "merge the programs".into());
    if parsed.len() == 1 {
        return Ok(parsed.pop().expect("one program"));
    }
    let axis = crate::code_rule_dsl::logic::front::AXIS;
    let mut all = Program::default();
    let mut sets: BTreeMap<Vec<String>, String> = BTreeMap::new();
    let core = crate::code_rule_dsl::logic::rr::core_names();
    let mut policies: Vec<scoping::Policy> = Vec::new();
    // The library, when held once, is the first part: its names are the run's (decision 0089 §2).
    let units = library.into_iter().map(|l| (None, l)).chain(parsed.into_iter().enumerate().map(|(i, p)| (Some(i), p)));
    for (unit, mut prog) in units {
        let i = unit.unwrap_or(usize::MAX);
        let scope = prog.doc_scopes.clone();
        let (named, library) = front::globals(&prog.settings);
        prog.settings.retain(|(k, _)| k != front::GLOBAL);
        // The documents read: those some policy's scope selects; every one when a policy has none.
        all.doc_scopes = match (i, all.doc_scopes.take(), prog.doc_scopes.take()) {
            (0, _, mine) => mine,
            (_, Some(mut a), Some(b)) => {
                a.extend(b);
                Some(a)
            }
            _ => None,
        };
        for g in std::mem::take(&mut prog.text_globs) {
            if !all.text_globs.contains(&g) {
                all.text_globs.push(g);
            }
        }
        // Kind sets by their kinds: this program's name for each, and the shared one.
        let mut set_of: BTreeMap<String, String> = BTreeMap::new();
        for (name, kinds) in std::mem::take(&mut prog.kindsets) {
            let n = sets.len();
            let shared = sets.entry(kinds.clone()).or_insert_with(|| format!("ks{n}")).clone();
            all.kindsets.insert(shared.clone(), kinds);
            set_of.insert(name, shared);
        }
        let shared_outputs = ["violation", "rr_diagnostic"];
        // Named repository readers belong to their policy. Equal bytes may share a reader later;
        // equal relation names alone do not mean equal input files or selectors.
        let file_inputs: BTreeSet<_> = prog.settings.iter().filter(|(key, _)| key == "input")
            .filter_map(|(_, text)| text.split_once('(').map(|(rel, _)| rel.to_owned())).collect();
        let own: BTreeSet<String> = prog
            .decls
            .keys()
            .chain(prog.facts.keys())
            .chain(prog.rules.iter().map(|r| &r.head.rel))
            // The library's relations keep their names (decision 0089 §2).
            .filter(|_| unit.is_some())
            // `core.rr`'s relations are the engine's, one per run however many programs include it (decision 0047);
            // a repository file a policy reads (`.input r(IO=…)`) is its own.
            .filter(|r| (!prog.inputs.contains(r) || file_inputs.contains(*r)) && !shared_outputs.contains(&r.as_str()) && !core.contains(*r) && !bridge::is_bridged(r) && !phases::is_phased(r) && !guests::SHARED.contains(&r.as_str()))
            .cloned()
            .collect();
        // The library held once is no policy's: canonical by construction, made over every document read (0091 §3).
        if unit.is_some() {
            policies.push(scoping::Policy { prefix: format!("p{i}_"), own: own.iter().map(|r| format!("p{i}_{r}")).collect(), named, library, scope });
        }
        let rename = |rel: &str| -> String {
            if own.contains(rel) {
                return format!("p{i}_{rel}");
            }
            match rel.strip_prefix(axis).and_then(|set| set_of.get(set)) {
                Some(shared) => format!("{axis}{shared}"),
                None => rel.to_owned(),
            }
        };
        let atom = |a: &mut front::Atom| a.rel = rename(&a.rel);
        for rule in &mut prog.rules {
            atom(&mut rule.head);
            for l in &mut rule.body {
                match l {
                    Literal::Atom(a) => atom(a),
                    Literal::Agg { atom: a, .. } => atom(a),
                    Literal::Nearest { kindset, .. } => {
                        if let Some(shared) = set_of.get(kindset) {
                            *kindset = shared.clone();
                        }
                    }
                    Literal::Cmp { .. } | Literal::Ast { .. } => {}
                }
            }
        }
        all.nullary.extend(prog.nullary.iter().map(|rel|rename(rel)));
        for (rel, cols) in prog.decls {
            all.decls.entry(rename(&rel)).or_insert(cols);
        }
        for (rel, decl) in prog.lattice {
            all.lattice.insert(rename(&rel), decl);
        }
        for (rel, rows) in prog.facts {
            all.facts.insert(rename(&rel), rows);
        }
        for (rel, mut native) in prog.natives {
            native.rename_reads(rename);
            all.natives.insert(rename(&rel), native);
        }
        // A rule `core.rr` gives (its head is the engine's) is taken once.
        for rule in prog.rules {
            if !(core.contains(&rule.head.rel) && all.rules.contains(&rule)) {
                all.rules.push(rule);
            }
        }
        for list in [(&mut all.inputs, prog.inputs), (&mut all.outputs, prog.outputs), (&mut all.demand, prog.demand)] {
            for rel in list.1 {
                let rel = rename(&rel);
                if !list.0.contains(&rel) {
                    list.0.push(rel);
                }
            }
        }
        all.policies.extend(prog.policies);
        // What the batch makes for the program's rules: its regexes over spans, its AST patterns.
        all.rx_spans.extend(prog.rx_spans);
        all.ast_patterns.extend(prog.ast_patterns);
        for mut setting in prog.settings {
            if matches!(setting.0.as_str(), "input" | "output") && let Some((rel, params)) = setting.1.split_once('(') {
                setting.1 = format!("{}({params}", rename(rel));
            }
            if !all.settings.contains(&setting) {
                all.settings.push(setting);
            }
        }
    }
    // The combined program bounded once to the documents it shows (decision 0058 §1, 0089 §4): a library's rules are
    // the same for every program, so one analysis gives what each program's would.
    if shared && makes_guests {
        let _hiding = crate::code_rule_dsl::logic::profile::span(|| format!("bound the combined program to the documents it shows ({} rules; decision 0058)", all.rules.len()));
        hiding::hide(&mut all, bounds[0]);
    }
    // The library's relations once for the run, each policy's rules over its own scope's documents (decision 0091).
    let spacing = crate::code_rule_dsl::logic::profile::span(|| "fact spaces".into());
    let report = scoping::guard(&mut all, &policies).map_err(Error::Plan)?;
    drop(spacing);
    crate::code_rule_dsl::logic::profile::decision("fact spaces", || {
        let guarded: Vec<String> = report.guarded.iter().map(|(p, n)| format!("{p}: {n}")).collect();
        format!(
            "{} global relations one copy for the run ({} policies' copies), and {} copies of engine relations; {} library relations a policy changed, its own{}; guards on the reads of the policies narrower than the run: {} (decision 0091)",
            report.canonical.len(),
            report.canonical.values().sum::<usize>(),
            report.engine,
            report.demoted.len(),
            if report.demoted.is_empty() { String::new() } else { format!(" ({})", report.demoted.iter().take(6).cloned().collect::<Vec<_>>().join(", ")) },
            if guarded.is_empty() { "none".to_owned() } else { guarded.join(", ") },
        )
    });
    Ok(all)
}

/// Of a program whose text names its libraries as modules (decision 0089 §5), the part the run's library does not
/// hold, taken out of `prog`: what is left is the program's own. A library relation goes to the library unless the
/// program's own clauses add to it or it reads one they add to, through any relation (the request's cone, §3): the
/// program keeps its own copy of those, as before. A relation lowering made for library rules alone (every reader
/// of it the library's) goes with them. Returns the relations taken out, and the cone's.
fn own_part(prog: &mut front::Program, modules: &crate::code_rule_dsl::logic::rr::Modules, core: &BTreeSet<String>) -> (usize, usize) {
    use front::Literal;
    let library: BTreeSet<&str> = modules.files.iter().flat_map(|(_, _, names)| names).map(String::as_str).filter(|r| !core.contains(*r)).collect();
    let reads = |r: &front::Rule| -> Vec<String> {
        r.body
            .iter()
            .filter_map(|l| match l {
                Literal::Atom(a) | Literal::Agg { atom: a, .. } => Some(a.rel.clone()),
                _ => None,
            })
            .collect()
    };
    let edges: Vec<(String, Vec<String>)> = prog
        .rules
        .iter()
        .map(|r| (r.head.rel.clone(), reads(r)))
        .chain(prog.natives.iter().map(|(rel, n)| (rel.clone(), n.reads().into_iter().map(str::to_owned).collect())))
        .collect();
    let mut cone: BTreeSet<String> = modules.heads.iter().filter(|h| library.contains(h.as_str())).cloned().collect();
    loop {
        let before = cone.len();
        for (head, body) in &edges {
            if !cone.contains(head) && body.iter().any(|b| cone.contains(b)) {
                cone.insert(head.clone());
            }
        }
        if cone.len() == before {
            break;
        }
    }
    let mut readers: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (head, body) in &edges {
        for b in body {
            readers.entry(b.as_str()).or_default().insert(head.as_str());
        }
    }
    let mut part: BTreeSet<String> = library.iter().filter(|r| !cone.contains(**r)).map(|r| (*r).to_owned()).collect();
    let candidates: BTreeSet<String> = prog
        .decls
        .keys()
        .chain(prog.rules.iter().map(|r| &r.head.rel))
        .chain(prog.natives.keys())
        .filter(|r| !library.contains(r.as_str()) && !core.contains(*r) && !cone.contains(*r))
        .cloned()
        .collect();
    loop {
        let before = part.len();
        for c in &candidates {
            if !part.contains(c) && readers.get(c.as_str()).is_some_and(|rs| !rs.is_empty() && rs.iter().all(|h| part.contains(*h))) {
                part.insert(c.clone());
            }
        }
        if part.len() == before {
            break;
        }
    }
    let taken = part.iter().filter(|r| prog.decls.contains_key(*r) || prog.rules.iter().any(|x| &x.head.rel == *r)).count();
    prog.rules.retain(|r| !part.contains(&r.head.rel));
    prog.decls.retain(|r, _| !part.contains(r));
    prog.facts.retain(|r, _| !part.contains(r));
    prog.natives.retain(|r, _| !part.contains(r));
    prog.lattice.retain(|r, _| !part.contains(r));
    prog.nullary.retain(|r| !part.contains(r));
    let cones = cone.iter().filter(|r| library.contains(r.as_str())).count();
    (taken, cones)
}

/// Records the run's manifest in the persistent store, and, when it is new, starts collection on a thread of its own:
/// passes until one finds nothing twice or 200 ms have gone (`RERAIL_CORPUS_GC=off`: none). The history's high-water
/// mark is `RERAIL_CORPUS_HISTORY_MB` (default 512), the low-water mark three quarters of it.
/// The chunk builder's thread, in `scope`.
#[cfg(not(target_family = "wasm"))]
fn spawn_chunks<'scope, T: Send + 'scope>(
    scope: &'scope std::thread::Scope<'scope, '_>,
    build: impl FnOnce() -> T + Send + 'scope,
) -> std::thread::ScopedJoinHandle<'scope, T> {
    scope.spawn(build)
}

/// wasm32 has no threads: the chunk builder runs to its end here, before the chunks are evaluated.
#[cfg(target_family = "wasm")]
fn spawn_chunks<'scope, T>(_: &'scope std::thread::Scope<'scope, '_>, build: impl FnOnce() -> T) -> Ran<T> {
    Ran(build())
}

/// A chunk builder that already ran (wasm32), joined as a thread would be.
#[cfg(target_family = "wasm")]
struct Ran<T>(T);

#[cfg(target_family = "wasm")]
impl<T> Ran<T> {
    fn join(self) -> std::thread::Result<T> {
        Ok(self.0)
    }
}

/// A run's hold on its memo plan (`ContentStore::hold_corpus_plan`) and on each source it stores or finds
/// (`ContentStore::hold_sources`), released when the run ends; a run that dies leaves them to lapse with their lease.
struct HeldPlan<'a> {
    store: &'a ContentStore,
    owner: String,
}

impl<'a> HeldPlan<'a> {
    /// The lease: longer than a run, short enough that a dead run's hold does not keep a plan for long.
    const LEASE_SECS: i64 = 6 * 3600;

    fn new(store: &'a ContentStore, plans: &[[u8; 32]]) -> std::result::Result<Self, crate::content_store::StoreError> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let owner = format!("corpus-run:{}:{}:{}", std::process::id(), now.as_nanos(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        let lease = now.as_secs() as i64 + Self::LEASE_SECS;
        for plan in plans {
            store.hold_corpus_plan(*plan, &owner, lease)?;
        }
        store.hold_sources(&owner, lease)?;
        Ok(HeldPlan { store, owner })
    }
}

impl Drop for HeldPlan<'_> {
    fn drop(&mut self) {
        // The manifest names the plan now, or the run failed: either way the hold is done. A failed release lapses.
        let _ = self.store.release_pins(&self.owner);
    }
}

fn record_run(store: &ContentStore, root: &Path, sources: &[crate::content_store::SourceId], program: &[u8; 32], also: &[[u8; 32]], relations: &[(String, String)], reading: Option<crate::content_store::CollectionLease>) -> Result<Option<std::thread::JoinHandle<(usize, std::result::Result<String, String>)>>> {
    memo::off();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    store
        .record_corpus_run_naming(&root.to_string_lossy(), sources, crate::code_rule_dsl::workspace::grammar_version(), i64::from(crate::code_rule_dsl::tape::TAPE_FORMAT_VERSION), memo::plan_digest(program), also, now)
        .map_err(|e| Error::Plan(e.to_string()))?;
    store.record_corpus_relations(memo::plan_digest(program), relations).map_err(|e| Error::Plan(e.to_string()))?;
    // The manifest now protects the sources. Release before collection, including warm runs
    // that must resume a pass deferred by another active reader.
    drop(reading);
    if std::env::var("RERAIL_CORPUS_GC").is_ok_and(|v| v == "off") {
        return Ok(None);
    }
    let mut policy = crate::content_store::CorpusPolicy::default();
    if let Some(mb) = std::env::var("RERAIL_CORPUS_HISTORY_MB").ok().and_then(|v| v.parse::<u64>().ok()) {
        (policy.high_water, policy.low_water) = (mb << 20, (mb << 20) / 4 * 3);
    }
    // No thread, no second connection, unless a pass has work (a run of fresh roots decides rarely).
    if !store.corpus_pass_wanted(&policy, now).map_err(|e| Error::Plan(e.to_string()))? {
        return Ok(None);
    }
    let path = store.database_path().to_path_buf();
    Ok(Some(std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let mut passes = 0;
        let outcome = (|| -> std::result::Result<String, crate::content_store::StoreError> {
            let store = ContentStore::open(&path)?;
            let (mut idle, mut kept, mut plan_rows, mut tapes, mut sources) = (0, None, 0, 0, 0);
            while idle < 2 && started.elapsed() < std::time::Duration::from_millis(200) {
                let pass = store.collect_corpus_pass(&policy, now)?;
                passes += 1;
                idle = if pass.did_work() { 0 } else { idle + 1 };
                kept = pass.kept.or(kept);
                (plan_rows, tapes, sources) = (plan_rows + pass.plan_rows, tapes + pass.collection.tapes, sources + pass.collection.sources);
            }
            Ok(match kept {
                Some(k) => format!(
                    "{} manifests kept, {} evicted; history {:.1} MB (high water {} MB), protected {:.1} MB; {tapes} tapes, {sources} sources and {plan_rows} memo rows of dead plans deleted{}",
                    k.manifests,
                    k.evicted,
                    k.history_bytes as f64 / 1048576.0,
                    policy.high_water >> 20,
                    k.protected_bytes as f64 / 1048576.0,
                    if idle < 2 { "; more left for later passes" } else { "" }
                ),
                None => format!("no decision due; {tapes} tapes, {sources} sources and {plan_rows} memo rows deleted"),
            })
        })()
        .map_err(|e| e.to_string());
        (passes, outcome)
    })))
}

#[cfg(test)]
thread_local! {
    /// A test's own store, so that another test's collection in the shared default store does not evict its rows.
    pub(in crate::code_rule_dsl) static TEST_TAPE_CACHE: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Held by a test that starts the process's session (`session::start`) and by one whose runs must not see it: every
/// run uses a live session's store.
#[cfg(test)]
pub(in crate::code_rule_dsl) static SESSION_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The store the corpus runner keeps tapes in: `RERAIL_TAPE_CACHE`, else one in the temporary
/// directory; `RERAIL_TAPE_CACHE=off` keeps none.
pub(in crate::code_rule_dsl) fn tape_cache() -> std::result::Result<Option<ContentStore>, crate::content_store::StoreError> {
    tape_cache_path().map(|path| ContentStore::open(&path)).transpose()
}

/// Runs `f` with phase 0's observer (`content_store::observe`), kept beside the tape cache (`<tape cache>.files`), or
/// in memory for the process when there is none. Every path the engine reads goes through it: rule files, corpus
/// files, what resolution rules list, read and load.
pub(in crate::code_rule_dsl) fn observe<T>(f: impl FnOnce(&mut crate::content_store::observe::Observer) -> T) -> T {
    let store = tape_cache_path().map(|p| crate::content_store::observe::store_beside(&p));
    crate::content_store::observe::with(store.as_deref(), f)
}

/// The path `tape_cache` opens (`None`: `RERAIL_TAPE_CACHE=off`).
pub(in crate::code_rule_dsl) fn tape_cache_path() -> Option<std::path::PathBuf> {
    #[cfg(test)]
    if let Some(path) = TEST_TAPE_CACHE.with(|p| p.borrow().clone()) {
        return Some(path);
    }
    match std::env::var("RERAIL_TAPE_CACHE") {
        Ok(v) if v == "off" => None,
        Ok(v) => Some(std::path::PathBuf::from(v)),
        Err(_) => Some(std::env::temp_dir().join("rerail-tape-cache.sqlite")),
    }
}

/// `run` with typing inputs and outputs (`typing`: declarations, dependency summaries, summarizing).
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_typed(programs: &[&str], root: &Path, t_facts: Option<&Path>, out: &Path, modes: &str, binding: Binding, chunk_nodes: usize, typing: &Typing) -> Result<Report> {
    run_with_options(programs, root, t_facts, out, modes, binding, chunk_nodes, typing, &serde_json::Value::Null)
}

#[allow(clippy::too_many_arguments)]
/// The first chunk's nodes, at most: what it adds per node sizes the rest.
const PROBE_NODES: usize = 65_536;
/// What a chunk's file-local run may add to the footprint (`RERAIL_CHUNK_MB`).
const CHUNK_MB: f64 = 384.0;
/// The run's whole footprint budget (`RERAIL_RUN_MB`): a chunk adds at most three quarters of what is left of it
/// above the footprint the chunk starts at (the program, its plans, the cross part's rows), so the run stays within
/// it however large the program is.
const RUN_MB: f64 = 1024.0;
/// The fewest nodes a chunk is sized to: a smaller one costs more in each chunk's fixed work than it saves.
const MIN_CHUNK_NODES: usize = 8_192;

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_with_options(
    programs: &[&str],
    root: &Path,
    t_facts: Option<&Path>,
    out: &Path,
    modes: &str,
    binding: Binding,
    chunk_nodes: usize,
    typing: &Typing,
    options: &serde_json::Value,
) -> Result<Report> {
    let before = observe(|o| {
        o.begin();
        o.counts
    });
    let report = run_observed(programs, root, t_facts, out, modes, binding, chunk_nodes, typing, options);
    // Phase 0's observations, written once the run has read everything (they may lag the run, never lead it).
    let did = observe(|o| {
        o.flush();
        o.counts.since(&before)
    });
    report.map(|mut r| {
        r.files_observed = did;
        r
    })
}

#[allow(clippy::too_many_arguments)]
fn run_observed(
    programs: &[&str],
    root: &Path,
    t_facts: Option<&Path>,
    out: &Path,
    modes: &str,
    binding: Binding,
    chunk_nodes: usize,
    typing: &Typing,
    options: &serde_json::Value,
) -> Result<Report> {
    // The run's settings, as `env` facts (t21 E4): `<root>/rerail.env`, then the `env` option's `key=value`s (the
    // command line's `--env`).
    let flags: Vec<String> = options.get("env").and_then(|v| v.as_array()).into_iter().flatten().filter_map(|v| v.as_str().map(str::to_owned)).collect();
    let settings = crate::code_rule_dsl::tools::settings(root, &flags).map_err(Error::Plan)?;
    let with_settings: Vec<String> = programs.iter().map(|p| crate::code_rule_dsl::tools::with_settings(p, &settings)).collect();
    let programs: &[&str] = &with_settings.iter().map(String::as_str).collect::<Vec<_>>();
    // The main thread's CPU and the process's peak RSS when the run began: the first phase counts combining.
    let began = { let (u, s) = thread_usage(); (u, s, usage().2) };
    let mut phases: Vec<(&'static str, f64, f64, f64, usize)> = Vec::new();
    // The main thread's CPU (the chunk builder's is its own, below) and the process's peak RSS.
    let main_usage = || { let (u, s) = thread_usage(); (u, s, usage().2) };
    let mut last = began;
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
    // A profiled run holds the profile (`profile::one_run`), and asks again once it does: a test that turned
    // profiling on may have turned it off meanwhile.
    let _profile = crate::code_rule_dsl::logic::profile::on().then(crate::code_rule_dsl::logic::profile::one_run);
    let profiling = _profile.is_some() && crate::code_rule_dsl::logic::profile::on();
    if profiling {
        crate::code_rule_dsl::logic::profile::reset();
    }
    let combining = crate::code_rule_dsl::logic::profile::span(|| "parse the program".into());
    let mut prog = combine(programs, "")?;
    let mut texts = texts_digest(programs, "");
    drop(combining);
    // Typing programs' answers the rule reads (`bridge`): each program runs first, and its rows are the rule's facts.
    let (bridged, typing_runs) = bridge::facts(&prog, root, out, modes, binding, chunk_nodes, typing)?;
    // Earlier phases' exports it reads (`phases`, decision 0083 §1): each phase runs first, in its own fact space.
    let (phased, phase_runs) = phases::facts(&prog, root, out, modes, binding, chunk_nodes, typing, options)?;
    let add_bridged = |prog: &mut front::Program| {
        for (rel, rows) in bridged.iter().chain(&phased) {
            prog.facts.entry(rel.clone()).or_default().extend(rows.iter().cloned());
        }
    };
    add_bridged(&mut prog);
    // Plans are kept across this run's chunks only (decision 0038).
    let _plans = super::KeepPlans::start();
    // And the values derived from them (`logic::derived`): each chunk's join specs.
    let _derived = crate::code_rule_dsl::logic::derived::Keep::start();
    // A session's run (`session`, decision 0063): its store and its memo, held for the run.
    let mut held = session::current();
    let t = t_facts.map(|t_facts| Connection::open_with_flags(t_facts, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)).transpose()?;
    // The program's `.language`, which picks the corpus's files: `.rr` files only when it names `rr` (decision 0061).
    let language = prog.settings.iter().find(|(key, _)| key == "language").map(|(_, value)| value.trim().trim_matches('"').to_owned());
    let (docs, first, label): (Vec<(i64, String)>, HashMap<i64, i64>, &'static str) = match &t {
        Some(t) => {
            let docs: Vec<(i64, String)> = t.prepare("SELECT d, path FROM doc ORDER BY d")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
            let first = t.prepare("SELECT d, min(n) FROM node GROUP BY d")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
            let label = if docs.first().is_some_and(|(_, p)| p.ends_with(".rs")) { "rust" } else { "python" };
            (docs, first, label)
        }
        None => {
            let (files, label) = if let Some(supplied) = options.get("inputFiles") {
                let supplied = supplied.as_array().ok_or_else(|| Error::Plan("inputFiles must be a list of relative document paths".to_owned()))?;
                let files = supplied.iter().map(|value| {
                    let path = value.as_str().ok_or_else(|| Error::Plan("inputFiles must contain strings".to_owned()))?;
                    if path.is_empty() || Path::new(path).components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
                        return Err(Error::Plan(format!("inputFiles: invalid relative path {path:?}")));
                    }
                    Ok(path.to_owned())
                }).collect::<Result<Vec<_>>>()?;
                let label = match language.as_deref() {
                    Some("python") => "python",
                    Some("rust") => "rust",
                    Some("rr") => "rr",
                    Some(other) => return Err(Error::Plan(format!("explicit input batch: unsupported corpus language {other:?}"))),
                    None if files.iter().filter(|p| p.ends_with(".rs")).count() > files.len() / 2 => "rust",
                    None => "python",
                };
                (files, label)
            } else {
                let found = scoped(corpus_files(root)?, &prog, options);
                let label = corpus_language(language.as_deref(), typing, &found);
                (found.of(label), label)
            };
            // A document's key is a function of its path (decision 0052), so a memoised row's node ids stay valid.
            let keys = memo::doc_keys(&files);
            (keys.into_iter().zip(files).collect(), HashMap::new(), label)
        }
    };
    let mut docs = docs;
    // What the run consults (`Report::consulted`): first the workspace documents, when they are files under `root`.
    let mut consulted: Vec<(String, String, String)> =
        if t.is_none() { docs.iter().map(|(_, p)| ("doc".to_owned(), p.clone(), world::bytes_digest(&root.join(p)))).collect() } else { Vec::new() };
    let mut paths: HashMap<i64, String> = docs.iter().cloned().collect();
    // The documents requests loaded (decision 0065 §2a), whose rows are not the run's findings: their keys.
    let mut loaded_keys: std::collections::HashSet<i64> = std::collections::HashSet::new();
    // Declarations and dependency summaries, as `declared_ret` facts (`corpus::typing`).
    let mut typed = TypingReport::default();
    let mut declared = BTreeMap::new();
    if prog.decls.contains_key("declared_ret") {
        let files: Vec<String> = docs.iter().map(|(_, p)| p.clone()).collect();
        let (facts, d) = typing::facts(typing, root, &files, &mut typed)?;
        if !facts.is_empty() {
            prog = combine(programs, &facts)?;
            texts = texts_digest(programs, &facts);
            add_bridged(&mut prog);
        }
        declared = d;
    }
    // Typing may rebuild the combined program. Publish external facts after that rebuild.
    external_inputs(&mut prog, root)?;
    // Guest requests (decision 0055) and the file system's (decision 0065 §2a) are outputs, so the program keeps the
    // rules that make them.
    let world_requests: Vec<&str> = WORLD.iter().map(|(_, req)| *req).chain([world::LOAD]).collect();
    // Only when a rule the outputs need reads a guest: a library's `parsed(…)` in rules nothing reads asks for no
    // parse (lazy-architecture's included name rules asked for every string annotation of the corpus).
    let roots: Vec<String> = prog.outputs.iter().filter(|o| !guests::REQUESTS.contains(&o.as_str())).cloned().collect();
    let reads_guests = front::needed(&prog, roots).iter().any(|rel| {
        prog.rules.iter().filter(|r| &r.head.rel == rel).any(|r| r.body.iter().filter_map(front::Literal::atom).any(|a| guests::READS.contains(&a.rel.as_str())))
    });
    crate::code_rule_dsl::logic::profile::decision("guest requests", || {
        if reads_guests { "made: a rule the outputs need reads guests".into() } else { "none: no rule the outputs need reads a guest".into() }
    });
    for rel in guests::LIBRARY.iter().copied().chain(guests::DEMANDS).filter(|_| reads_guests).chain(world_requests) {
        if prog.rules.iter().any(|r| r.head.rel == rel) && !prog.outputs.iter().any(|o| o == rel) {
            prog.outputs.push(rel.to_owned());
        }
    }
    let dialect = crate::code_rule_dsl::language::dialect_by_label(label).ok_or_else(|| Error::Plan(format!("no dialect {label}")))?;
    if !dialect.built() {
        return Err(Error::Plan(format!("{label} is not in this build (its `lang-{label}` feature is off)")));
    }
    std::fs::create_dir_all(out).map_err(|e| Error::Plan(e.to_string()))?;
    // A long-lived process (the browser's engine, `session`) keeps its store, its last program compiled and each
    // file's file-local rows from run to run: an edit evaluates the files it changed and the cross-file part.
    let session = held.as_mut().filter(|_| t.is_none() && !prog.decls.contains_key("declared_ret"));
    let (path, cross_path) = match &session {
        Some(s) => (s.path.clone(), out.join("cross.sqlite")),
        None => (out.join("store.sqlite"), out.join("cross.sqlite")),
    };
    // A session's store is kept from run to run; the cross-file scratch never is.
    for p in [&path, &cross_path] {
        if (session.is_none() || *p == cross_path) && p.exists() {
            std::fs::remove_file(p).map_err(|e| Error::Plan(e.to_string()))?;
        }
    }
    // The persistent store: tapes kept across runs, keyed by content (`RERAIL_TAPE_CACHE` names it, `off` parses
    // every file), and the run's atoms and each file's memoised rows (decision 0052): atoms are never given out again,
    // so a memoised row's symbols mean the same in every run. The run's SQL works in `store.sqlite`, on the same
    // atoms. Without the store, or reproducing T, the atoms are the run's own; in a session, the session's store's.
    // Without the `tape-cache` feature (the browser build) there is no store to keep: `tapes` is `None` where the
    // compiler can see it, so the memo, the run record and their zstd frames are not linked.
    let tapes = if cfg!(feature = "tape-cache") { tape_cache().map_err(|e| Error::Plan(e.to_string()))? } else { None };
    // Hold before any source/stat lookup, through parsing, memo publication and the run manifest. This shared lock is
    // the run's one hold on collection (O108): every collector takes it exclusively first (`try_collect`), and a run
    // that dies releases it with its process.
    let reading = tapes.as_ref().map(|cache| cache.hold_collection()).transpose().map_err(|e| Error::Plan(e.to_string()))?;
    let own_store;
    let store: &ContentStore = match (&session, &tapes) {
        (Some(s), _) => &s.store,
        (None, Some(cache)) if t.is_none() => {
            // The file first: the arena is found by the database's canonical path, which a missing file has not.
            std::fs::File::create(&path).map_err(|e| Error::Plan(format!("{}: {e}", path.display())))?;
            cache.share_atoms_with(&path);
            cache
        }
        _ => {
            own_store = ContentStore::open(&path).map_err(|e| Error::Plan(e.to_string()))?;
            &own_store
        }
    };
    let db = Connection::open(&path)?;
    // The run's databases are scratch, made again by every run (removed above): a commit need not reach the disk.
    db.execute_batch("PRAGMA synchronous=OFF")?;
    let statements = Statements::on(&db);
    statements.enter("stats");
    // The program as this run prepares it, kept for the process (`Prepared`): a session or a test running one program
    // over many roots prepares it once. The key is what preparing reads besides the store's atoms, which are asked
    // again below.
    // Cross-file rows memoised per import component (decision 0076): the program asks, and its documents' language
    // has import rules in `components.rr`. Not for a discovery round of `world`, which runs part of the program; the
    // final round's world facts are program facts, so they are in the plan's digest.
    let cross_mode = cross_memo::mode();
    // With requests answered inside the fixpoint too: the cone runs first, so every loaded document's exports, the
    // component rules' only reads, are in before the components are (decision 0065 §2a).
    let loading_run = world::wanted(&prog);
    let cross_declared = cross_memo::declared(&prog).filter(|_| cross_mode != cross_memo::Mode::Off && label == "python");
    // A program that reads the components' relations itself (`rr_cx_reach`: a closure-local lookup) has them with or
    // without the memo.
    let reads_components = prog.rules.iter().any(|r| r.body.iter().filter_map(|l| l.atom()).any(|a| a.rel.starts_with("rr_cx_")));
    if reads_components && label != "python" {
        return Err(Error::Plan(format!("the import components' relations (`rr_cx_*`) are Python's; this run's documents are {label}")));
    }
    if cross_declared.is_some() || reads_components {
        cross_memo::add(&mut prog)?;
    }
    let prepared_key: [u8; 32] = {
        use sha2::Digest as _;
        let mut hash = sha2::Sha256::new();
        hash.update(texts);
        hash.update(format!("{label}\0{:?}\0{:?}\0{:?}\0{}", prog.facts, prog.inputs, prog.outputs, cross_declared.is_some()).as_bytes());
        hash.finalize().into()
    };
    // The lowered program's atoms are the store's: the same strings must still be the same atoms.
    let holds = |p: &Prepared| {
        p.interned.iter().all(|(strings, atoms)| {
            let strings: Vec<&str> = strings.iter().map(String::as_str).collect();
            store.intern_atoms(&strings).is_ok_and(|got| got.iter().map(|a| u32::from(*a)).eq(atoms.iter().copied()))
        })
    };
    let kept = prepared_cache().lock().expect("the prepared programs").get(&prepared_key).cloned().filter(holds);
    // Kept across processes in the persistent store, under the programs' text (decision 0073); a session's or T's run
    // keeps it in memory only.
    let durable = if tapes.is_some() && t.is_none() && session.is_none() {
        Some(memo::source(store, &programs.join("\0")).map_err(|e| Error::Plan(e.to_string()))?)
    } else {
        None
    };
    // The sources the stored program lives under: the program text's, its facts document's and its groups of automata
    // (decisions 0073 to 0075). No file roots them, so the run's manifest names them, and collection keeps them while
    // it keeps the run.
    let mut program_sources: Vec<crate::content_store::SourceId> = durable.into_iter().collect();
    // And the plans it is stored under, which the manifest names too: an earlier build's go once no kept run names them.
    let program_plans: Vec<[u8; 32]> = if durable.is_some() { prepared::plans(&prepared_key) } else { Vec::new() };
    let (kept, from) = match (kept, durable) {
        (Some(p), _) => (Some(p), "memory"),
        (None, Some(source)) => {
            let found = prepared::lookup(store, source, dialect, &prepared_key).map_err(|e| Error::Plan(e.to_string()))?.filter(|(p, _)| holds(p));
            (found.map(|(p, documents)| {
                program_sources.extend(documents);
                p
            }), "store")
        }
        (None, None) => (None, ""),
    };
    let reused = kept.is_some();
    let prepared = match kept {
        Some(p) => {
            crate::code_rule_dsl::prof::count("corpus.prepared_reused", 1);
            if from == "store" {
                let mut kept = prepared_cache().lock().expect("the prepared programs");
                if kept.len() >= PREPARED_KEPT {
                    kept.clear();
                }
                kept.insert(prepared_key, p.clone());
            }
            p
        }
        None => {
            let mut interned: Vec<(Vec<String>, Vec<u32>)> = Vec::new();
            // Only the rules an output reads, directly or through other rules: a rule that includes a library runs the part
            // of it the rule uses, not all of it (T's front prunes the same way).
            let preparing = crate::code_rule_dsl::logic::profile::span(|| "prune, simplify, demand".into());
            prog = front::prune(prog);
            // Bodies simplified first (decision 0038): implied atoms and set-member kind tests drop, identical relations merge.
            crate::code_rule_dsl::logic::simplify::simplify(&mut prog);
            // Tests a rule negates of the nodes it has bound, computed for those nodes only, not every node (magic sets).
            let (mut prog, demanded) = crate::code_rule_dsl::logic::magic::demand_negated_tests(prog);
            drop(preparing);
            // Text tests over text relations become relations the batches fill from their vocabulary (decision 0037).
            let vocabulary = crate::code_rule_dsl::logic::strings::as_relations(&mut prog).map_err(Error::Plan)?;
            let lowering = crate::code_rule_dsl::logic::profile::span(|| "lower the program".into());
            let mut lowered = lower::lower(&prog, dialect, &mut |s| {
                let atoms: Vec<u32> = store.intern_atoms(s).map_err(|e| Error::Plan(e.to_string()))?.into_iter().map(u32::from).collect();
                interned.push((s.iter().map(|x| (*x).to_owned()).collect(), atoms.clone()));
                Ok(atoms)
            })?;
            drop(lowering);
            let _step = crate::code_rule_dsl::logic::profile::span(|| "nearest sets made by the transducer".into());
            let sets = lowered.transduce_nearest();
            drop(_step);
            // The file-local part of each cross-file rule runs in the chunks, which export its rows, not their base relations.
            let _step = crate::code_rule_dsl::logic::profile::span(|| "push the file-local joins into the chunks".into());
            let pushed = crate::code_rule_dsl::typing::file_local::push_local_joins(&mut lowered.program);
            drop(_step);
            // Every relation the chunks hand on names its document per row, so a file's rows can be memoised (decision 0052).
            let _step = crate::code_rule_dsl::logic::profile::span(|| "locate the chunks' rows".into());
            let located = crate::code_rule_dsl::typing::file_local::locate(&mut lowered.program);
            drop(_step);
            // The joins pushed are made per rule, so two programs' copies of one library rule made two: merged again, and so
            // are the relations over them, before the program splits.
            let _step = crate::code_rule_dsl::logic::profile::span(|| "merge identical relations again".into());
            let merged = crate::code_rule_dsl::logic::simplify::merge_identical(&mut lowered.program);
            drop(_step);
            let _step = crate::code_rule_dsl::logic::profile::span(|| "split the file-local and cross-file parts".into());
            let split = crate::code_rule_dsl::typing::file_local::programs(&lowered.program);
            drop(_step);
            let prepared = Prepared { prog, vocabulary: Arc::new(vocabulary), lowered, sets, demanded, pushed, located, merged, split, interned };
            let mut kept = prepared_cache().lock().expect("the prepared programs");
            if kept.len() >= PREPARED_KEPT {
                kept.clear();
            }
            kept.insert(prepared_key, prepared.clone());
            drop(kept);
            if let Some(source) = durable {
                // A document whose tape does not give back its facts is a bug of `program_facts`: said, not stored.
                match prepared::remember(store, source, &prepared_key, &prepared).map_err(|e| Error::Plan(e.to_string()))? {
                    Some(documents) => program_sources.extend(documents),
                    None => crate::code_rule_dsl::logic::profile::decision("program prepared", || "not stored: a part's tape does not give back its facts (decision 0074)".to_owned()),
                }
            }
            prepared
        }
    };
    let origin = if reused { from } else { "prepared" };
    crate::code_rule_dsl::logic::profile::decision("program prepared", || match (reused, from) {
        (true, "store") => "read from the store: an earlier process prepared the same program and additions (decision 0073)".to_owned(),
        (true, _) => "reused: this process prepared the same program and additions before".to_owned(),
        (false, _) if durable.is_some() => "prepared, and stored for later processes (decision 0073)".to_owned(),
        (false, _) => "prepared".to_owned(),
    });
    let Prepared { prog, vocabulary, lowered, sets, demanded, pushed, located, merged: (remerged, reshared), mut split, interned: _ } = prepared;
    // The text tests' plans compiled once for the process (decision 0075), their groups of automata read from the store
    // when an earlier process determinized them.
    let compiling = crate::code_rule_dsl::logic::profile::span(|| "compile the text tests".into());
    let compiled = match durable {
        Some(_) => {
            let groups = prepared::StoredDfas(store, Default::default());
            let compiled = vocabulary.compile(Some(&groups));
            program_sources.extend(groups.1.into_inner().unwrap_or_default());
            compiled
        }
        None => vocabulary.compile(None),
    };
    drop(compiling);
    if let Some(c) = compiled.filter(|c| c.read + c.built + c.lazy > 0) {
        crate::code_rule_dsl::logic::profile::decision("text tests compiled (decision 0075)", || format!("groups of automata: {} read from the store, {} determinized, {} on the lazy DFA", c.read, c.built, c.lazy));
    }
    let vocabulary = vocabulary.again();
    if !demanded.is_empty() {
        crate::code_rule_dsl::logic::profile::decision("demand (magic sets)", || format!("{} negated tests computed only for the nodes their readers bind: {}", demanded.len(), demanded.join(", ")));
    }
    if !pushed.is_empty() {
        crate::code_rule_dsl::logic::profile::decision("file-local joins pushed into the chunks", || format!("{} cross-file rules read a join made per chunk instead of its base relations (first: {})", pushed.len(), pushed.iter().take(6).cloned().collect::<Vec<_>>().join(", ")));
    }
    if remerged > 0 {
        crate::code_rule_dsl::logic::profile::decision("merged after the joins were pushed", || format!("{remerged} relations merged into an identical one ({reshared} classes shared across programs)"));
    }
    let unlocated: Vec<&String> = split.exports.iter().chain(split.local.outputs.iter()).filter(|r| split.locations.get(*r).is_none_or(Vec::is_empty)).collect();
    // A file-local part reading the run's options (`file_local::option_input`) is memoised under them too.
    let memo_program = {
        let digest = memo::program_digest(&split.local);
        let reads_options = split.local.rules.iter().flat_map(|r| &r.body).filter_map(|l| l.atom()).any(|a| crate::code_rule_dsl::typing::file_local::option_input(&a.rel));
        if reads_options { memo::with_options(digest, options) } else { digest }
    };
    let cross_plan = match &cross_declared {
        Some(declared) => cross_memo::split(&mut split.cross, declared, &memo_program, &split.same_doc)?,
        None => None,
    };
    crate::code_rule_dsl::logic::profile::decision("cross-file memo (decision 0076)", || match (&cross_declared, &cross_plan) {
        (None, _) if cross_memo::declared(&prog).is_none() => "off: the program declares no `.pragma \"cross-memo\"`".to_owned(),
        (None, _) => format!("off ({})", if cross_mode == cross_memo::Mode::Off { "RERAIL_CROSS_MEMO=off" } else { "no import rules for the run's language" }),
        (Some(_), None) => "off: the component rules did not reach the cross-file part".to_owned(),
        (Some(_), Some(plan)) => format!("{} relations memoised per import component ({}){}", plan.memo.len(), plan.memo.keys().cloned().collect::<Vec<_>>().join(", "), if cross_mode == cross_memo::Mode::Verify { "; verifying: derived afresh and compared" } else { "" }),
    });
    // The file-local outputs, whose memo entries hold lines (recorded for a SQL session below).
    let local_outputs = split.local.outputs.clone();
    // `declared_ret` rows come from outside the memo's key (`corpus::typing`): a program whose rules still read them
    // after pruning is not memoised (a round of `world` does not read them).
    let reads_declared = prog.rules.iter().flat_map(|r| &r.body).filter_map(|l| l.atom()).any(|a| a.rel == "declared_ret");
    // The memo: the session's, in memory, or the tape cache's (decision 0052).
    let memo = match (&session, &tapes) {
        (Some(s), _) => Some(memo::Memo::Held(&s.memo)),
        (None, Some(cache)) if t.is_none() => Some(memo::Memo::Stored(cache)),
        _ => None,
    };
    let memoised = memo.is_some() && t.is_none() && unlocated.is_empty() && !reads_declared && std::env::var("RERAIL_CORPUS_MEMO").map_or(true, |v| v != "off") && options.get("memo").and_then(|v| v.as_str()) != Some("off");
    crate::code_rule_dsl::logic::profile::decision("per-file memo", || match (&located, memoised) {
        (_, true) => format!("each file's file-local rows are memoised under its content, path and the run's plan{}; {} relations given a document column", if session.is_some() { ", in this session's memory" } else { "" }, located.as_ref().map_or(0, Vec::len)),
        (Err(blocked), _) => format!("off: rows of {blocked:?} name no document"),
        _ => format!("off ({})", if memo.is_none() { "no persistent store" } else if t.is_some() { "T's facts" } else if unlocated.is_empty() { "typing, RERAIL_CORPUS_MEMO=off or the run's `memo: off`" } else { "a relation without a location column" }),
    });
    let read: BTreeSet<String> = lowered.program.rules.iter().flat_map(|r| r.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.clone())).collect();
    let part = |program: front::Program| lower::Lowered { program, domains: lowered.domains.clone(), ops: lowered.ops, dialect: lowered.dialect };
    crate::code_rule_dsl::logic::profile::decision("file-local split", || {
        format!(
            "{} rules run per chunk over its files (file-local); {} rules run once over all files (cross-file); {} relations exported to the cross part, {} base relations kept for it; exports and file-local outputs without a location column: {:?}",
            split.local.rules.len(),
            split.cross.rules.len(),
            split.exports.len(),
            split.base.len(),
            split.exports.iter().chain(split.local.outputs.iter().filter(|o| !split.exports.contains(o))).filter(|e| split.locations.get(*e).is_none_or(Vec::is_empty)).collect::<BTreeSet<_>>()
        )
    });
    crate::code_rule_dsl::logic::profile::decision("cross-file causes", || {
        // Root causes only: a relation cross-file because it reads another is listed under that one.
        let causes = crate::code_rule_dsl::typing::file_local::cross_causes(&lowered.program);
        let mut roots: Vec<String> = causes.iter().filter(|(_, why)| !why.starts_with("reads cross-file ")).map(|(rel, why)| format!("{rel}: {why}")).collect();
        let total = roots.len();
        roots.truncate(60);
        format!("{total} relations cross-file of their own ({} by reading them): {}", causes.len() - total, roots.join("; "))
    });
    crate::code_rule_dsl::logic::profile::decision("nearest ancestors", || {
        let names: Vec<&str> = sets.iter().map(|(n, _)| n.as_str()).collect();
        format!("{} sets made by the transducer's file-local FIX, bound {binding:?}: {}", sets.len(), names.join(", "))
    });
    // Requests answered inside the fixpoint (decision 0065 §2a): the cross-file part's cone of requests runs first,
    // pausing for answers. A load's answer is the loaded document's rows of every per-file relation the cone reads,
    // made by the chunk path as any file's are; the rest of the cross-file part reads the cone's rows as inputs.
    let loading: Option<(front::Program, BTreeSet<String>)> = if loading_run {
        let mut whole = split.cross.clone();
        whole.answers = world::answers(&whole);
        let cone = crate::code_rule_dsl::logic::answers::split(&whole).ok_or_else(|| Error::Plan("a program that reads the file system makes no request in its cross-file part".to_owned()))?;
        let per_file: BTreeSet<String> = cone.load.rules.iter().flat_map(|r| &r.body).filter_map(|l| l.atom()).map(|a| a.rel.clone()).filter(|r| split.exports.contains(r) || split.base.contains(r)).collect();
        if let Some(base) = per_file.iter().find(|r| split.base.contains(*r)) {
            return Err(Error::Plan(format!("the cone of requests reads base relation {base}: a load answers exports only (decision 0065 §2a)")));
        }
        if let Some(produced) = whole.answers.get_mut(world::LOAD) {
            produced.extend(per_file);
        }
        crate::code_rule_dsl::logic::answers::gate(&mut whole).map_err(|e| Error::Plan(e.to_string()))?;
        let parts = crate::code_rule_dsl::logic::answers::split(&whole).expect("the program has answers");
        crate::code_rule_dsl::logic::profile::decision("requests (decision 0065 §2a)", || {
            format!(
                "{} rules make and answer requests inside one fixpoint ({} gated negations), before the rest of the cross-file part; answers: {}",
                parts.load.rules.len(),
                parts.load.gates.len(),
                parts.load.answers.iter().map(|(r, p)| format!("{r} → {}", p.join(", "))).collect::<Vec<_>>().join("; ")
            )
        });
        // The rest reads the answers as inputs: the runner binds what it gave.
        let mut rest = parts.rest;
        for rel in parts.load.answers.values().flatten().filter(|r| !split.exports.contains(r)) {
            if !rest.inputs.contains(rel) {
                rest.inputs.push(rel.clone());
            }
        }
        split.cross = rest;
        Some((parts.load, parts.reads))
    } else {
        None
    };
    let (local, mut cross) = (part(split.local), part(split.cross));
    let slice = embed_slice(&lowered.program).map(part);
    let slice_reads: BTreeSet<String> = slice.iter().flat_map(|s| &s.program.rules).flat_map(|r| &r.body).filter_map(|l| l.atom()).map(|a| a.rel.clone()).collect();
    crate::code_rule_dsl::logic::profile::decision("guests", || match &slice {
        Some(s) => format!("an embed slice of {} rules runs per chunk, up to {} rounds, before the file-local part (decision 0055)", s.program.rules.len(), guests::DEPTH),
        None => "no rule asks for a guest".into(),
    });
    // The tree's projection: `core.rr`'s for a program that includes it, T's for one that declares its base relations
    // itself (decision 0046); relations no rule reads are not made.
    let projection = syntax::Projection::of(&prog, read.clone());
    crate::code_rule_dsl::logic::profile::decision("base relations", || format!("{:?} shape (decision 0046)", projection.shape));
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
        if own { "the first chunk measures its sources' statistics; later chunks scale them by their size (a relation first seen later is measured then); the planner prices each chunk's plans from them".into() } else { "T's corpus statistics, read once, with the first chunk's sources' own, scaled to each chunk".into() }
    });
    if let Some(n) = std::env::var("RERAIL_SAMPLE_EVERY").ok().filter(|n| n != "1") {
        crate::code_rule_dsl::logic::profile::decision("sampling", || format!("every {n}th file only (RERAIL_SAMPLE_EVERY): findings are a sample's"));
    }
    let constants = Own::constants(&lowered.program);
    let mut own_totals = Own::default();
    // Measured once per mix (`Own::measure` and `syntax::statistics` read every row, sorting every column): the
    // first chunk of a mix, with its nodes or rows, scaled to each later chunk of that mix. A chunk's mix is the file
    // types holding a tenth of its nodes or more (`chunk_mix`), so a chunk of TypeScript after chunks of Python is
    // measured again, and a stray file is not a new mix.
    let mut own_once: HashMap<String, (usize, Own)> = HashMap::new();
    // The stored tree counts this run read or wrote, for its manifest.
    let mut own_sources: Vec<crate::content_store::SourceId> = Vec::new();
    let mut measured_once: HashMap<(String, String), (usize, Vec<(String, i64, i64, i64, i64)>)> = HashMap::new();
    // `x` is the run's scratch (deleted at the start of the next): no journal and no syncs, since a run that stops
    // leaves nothing to recover. The browser build keeps it in memory, where its in-memory filesystem is anyway, so
    // its pages skip the filesystem's calls; so are the temp tables the strata make.
    #[cfg(target_family = "wasm")]
    db.execute_batch("ATTACH ':memory:' AS x")?;
    #[cfg(not(target_family = "wasm"))]
    db.execute("ATTACH ?1 AS x", [cross_path.to_string_lossy()])?;
    // `x` is this run's: tables an earlier run over the same store read (`table_source`, cached by the store's path
    // and the table's name) are another run's rows. Without this a second run read the first's cross-file tables:
    // a cone's loaded documents, there at the first run's end, counted as the workspace's.
    crate::code_rule_dsl::logic::native::forget_inputs(&db);
    let _: String = db.query_row("PRAGMA x.journal_mode = OFF", [], |r| r.get(0))?;
    db.execute_batch("PRAGMA x.synchronous = OFF; PRAGMA temp_store = MEMORY;")?;
    // Each base relation the cross-file part reads is kept in memory, across the chunks, as the columns its rules
    // name (and those decoding a finding's location reads); the others are never read. (Was: written to `x` and
    // read back, 0.75 s on Python.)
    let decodes = cross.program.outputs.iter().any(|o| o == "finding");
    let mut stored: Vec<(String, Vec<String>)> = Vec::new();
    // The repository inputs (`facts::repository`) the program reads: made once, bound once beside every chunk's.
    let repository: Vec<(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, Vec<(String, i64, i64, i64, i64)>)> = {
        let files: Vec<String> = docs.iter().map(|(_, p)| p.clone()).collect();
        let reads = |r: &str| read.contains(r) && prog.inputs.iter().any(|i| i == r);
        crate::code_rule_dsl::facts::repository::relations(&store, root, &files, options, &reads)
            .map_err(|e| Error::Plan(e.to_string()))?
            .into_iter()
            .filter(|(name, _)| reads(name))
            .map(|(name, r)| {
                let source: Arc<dyn crate::code_rule_dsl::facts::vtab::Source> = Arc::new(r);
                let measured = syntax::statistics(source.as_ref());
                (name.to_owned(), source, measured)
            })
            .collect()
    };
    for rel in split.base.iter().filter(|rel| !repository.iter().any(|(r, _, _)| r == *rel)) {
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
    // The run holds its memo plan and the sources it reads until its manifest names them (O95): a collector running
    // meanwhile would otherwise take the rows this run writes, or the sources they and its tapes are of.
    let _held = match memo {
        Some(memo::Memo::Stored(_)) if memoised => Some(HeldPlan::new(store, &[&[memo::plan_digest(&memo_program)][..], &program_plans].concat()).map_err(|e| Error::Plan(e.to_string()))?),
        _ => None,
    };
    // The base relations a chunk keeps only for the nodes its exports hold (`file_local::semijoin_bases`): read by
    // the cross part only as lookups of those nodes.
    let semijoin: BTreeSet<String> = crate::code_rule_dsl::typing::file_local::semijoin_bases(&cross.program, &split.base)
        .unwrap_or_default()
        .into_iter()
        .filter(|r| stored.iter().any(|(s, _)| s == r))
        .collect();
    if !stored.is_empty() {
        crate::code_rule_dsl::logic::profile::decision("semi-join of the held base relations", || {
            if semijoin.is_empty() {
                "none: a cross-file rule reads a base relation other than as a lookup of a node a cross-file relation holds".to_owned()
            } else {
                format!("{:?}: each chunk keeps the rows of the nodes its exports hold, not every node's", semijoin)
            }
        });
    }
    // A cross-file part that reads base relations takes their rows from the chunks: a memoised file's entry holds its
    // rows of them too (`memo::base_key`), every column, so it hands on what an evaluated file would.
    if !stored.is_empty() && memoised {
        crate::code_rule_dsl::logic::profile::decision("per-file memo holds base rows", || format!("the cross-file part reads base relations {:?}: each file's entry holds its rows of them", stored.iter().map(|(r, _)| r).collect::<Vec<_>>()));
    }
    // One transaction for every export's table, not a commit each.
    let mut tables = String::from("BEGIN;");
    for rel in &split.exports {
        let width = lowered.program.decls[rel].len();
        let cols: Vec<String> = (0..width).map(|i| format!("c{i}")).collect();
        tables.push_str(&format!("CREATE TABLE x.{}({}, PRIMARY KEY ({})) WITHOUT ROWID;", crate::code_rule_dsl::logic::sql::ident(rel), cols.iter().map(|c| format!("{c} INTEGER")).collect::<Vec<_>>().join(", "), cols.join(", ")));
    }
    tables.push_str("COMMIT;");
    db.execute_batch(&tables)?;
    phase("stats", statements.count());
    statements.enter("parse");
    let modules = Modules::default();
    // Every input the cross-file part reads, as bound: a session keeps their digests (decision 0077).
    let mut cross_inputs: Vec<(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)> = Vec::new();
    for (rel, source, _) in &repository {
        modules.bind(&db, rel, Arc::clone(source))?;
        cross_inputs.push((rel.clone(), Arc::clone(source)));
    }
    let repository_stats = |stats: &mut super::Stats| {
        for (rel, _, measured) in &repository {
            stats.measured(rel, measured.iter().map(|(c, rows, ndv, sq, sqd)| (c.clone(), [*rows, *ndv, *sq, *sqd])), []);
        }
    };
    let mut base_rows: BTreeMap<String, (Vec<String>, Vec<Option<Vec<i64>>>)> = BTreeMap::new();
    // The base rows memo hits replayed, every column: measured once, after the chunks.
    let mut replayed: BTreeMap<String, (Vec<String>, Vec<Vec<i64>>)> = BTreeMap::new();
    let mut bound = BTreeSet::new();
    let mut totals = Totals::default();
    let (mut nodes, mut native, mut chunks) = (0, BTreeSet::new(), 0);
    let (mut memo_hits, mut memo_unattributed) = (0usize, 0usize);
    // Why a chunk's rows were not memoised: the relation, and what its row named.
    let mut memo_why: BTreeSet<String> = BTreeSet::new();
    let mut outputs: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut node_columns: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let gather = |got: Outcome, outputs: &mut BTreeMap<String, Vec<String>>, node_columns: &mut BTreeMap<String, Vec<usize>>, keep: &dyn Fn(&str) -> bool| {
        let _gather = crate::code_rule_dsl::logic::profile::span(|| "gather outputs".into());
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
    // A chunk: its nodes and sources (none when it is only memo hits), its documents (key, path, source, nodes) and
    // the memo hits read since the last chunk.
    type Chunk = (usize, Vec<(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)>, Vec<(i64, String, Option<memo::Content>, u64)>, Vec<(String, memo::Entry)>, Vec<syntax::Document>);
    #[cfg(not(target_family = "wasm"))]
    let (send, chunks_in) = std::sync::mpsc::sync_channel::<Result<Option<Chunk>>>(1);
    // wasm32 has no threads: the builder runs to its end first (`spawn_chunks`), so every chunk waits here.
    #[cfg(target_family = "wasm")]
    let (send, chunks_in) = std::sync::mpsc::channel::<Result<Option<Chunk>>>();
    // Dropped when this thread stops on an error, so the builder's next send fails and it ends: else the scope waits
    // for a builder blocked on a full channel, and the error is never reported.
    let chunks_in = std::cell::RefCell::new(Some(chunks_in));
    // What each side of the channel is doing, for the other's waits (`profile::wait`).
    let (worker_doing, main_doing) = (crate::code_rule_dsl::logic::profile::Activity::default(), crate::code_rule_dsl::logic::profile::Activity::default());
    worker_doing.set("parse");
    main_doing.set("setup");
    let (mut parse_cpu, mut batch_cpu) = ((0.0, 0.0), (0.0, 0.0));
    // A run over the persistent store records which sources it read, its manifest (decision 0053).
    let recording = matches!(memo, Some(memo::Memo::Stored(_)));
    // The stats the run records are as of its start: an entry is trusted only for a file last changed before then.
    let started_ns = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as i64);
    let canonical_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf()).to_string_lossy().into_owned();
    let stat_index = if recording {
        store.corpus_stats(&canonical_root).map_err(|e| Error::Plan(e.to_string()))?
    } else {
        crate::content_store::StatIndex::default()
    };
    let stat_index = &stat_index;
    // The chunks' size: the caller's, at most, but sized to what a chunk's run holds (`CHUNK_MB`): the first chunk is
    // `PROBE_NODES` nodes, and each run measures the footprint it adds per node. 64 data-layer rules hold about
    // 5 KB a node at their peak, so the caller's 262,144 nodes took 1.3 GB; the converted rules hold much less.
    let chunk_target = Arc::new(std::sync::atomic::AtomicUsize::new(chunk_nodes.min(PROBE_NODES)));
    let worker_target = Arc::clone(&chunk_target);
    // The documents the builder reads, in batches, each ended by `None` on the chunks' channel: the corpus's, then
    // each load's (decision 0065 §2a). A run that loads nothing closes the channel after the corpus's, so the
    // builder ends without waiting (as it must without threads).
    let (docs_out, docs_in) = std::sync::mpsc::channel::<Vec<(i64, String, Option<Arc<str>>)>>();
    docs_out.send(docs.iter().map(|(d, p)| (*d, p.clone(), None)).collect()).map_err(|_| Error::Plan("the chunk builder's channel".to_owned()))?;
    let mut docs_out = loading.is_some().then_some(docs_out);
    // What loading made: each loaded document (key, path), and the rows the rest of the cross-file part reads of the
    // cone and of the answers.
    let mut loaded_docs: Vec<(i64, String)> = Vec::new();
    let mut loading_rows: Vec<(String, Vec<Vec<i64>>)> = Vec::new();
    #[cfg(target_family = "wasm")]
    if docs_out.is_some() {
        return Err(Error::Plan("requests answered inside the fixpoint need threads (decision 0065 §2a)".to_owned()));
    }
    let worker_cpu = std::thread::scope(|scope| -> Result<((f64, f64), (f64, f64), Vec<crate::content_store::SourceId>, Vec<(String, crate::content_store::FileStat, crate::content_store::SourceId)>)> {
    let worker = spawn_chunks(scope, || {
        let (mut parse, mut batch) = ((0.0, 0.0), (0.0, 0.0));
        let add = |acc: &mut (f64, f64), from: (f64, f64)| {
            let now = thread_usage();
            acc.0 += now.0 - from.0;
            acc.1 += now.1 - from.1;
        };
        let mut pending = Vec::new();
        let mut pending_docs = Vec::new();
        let mut hits = Vec::new();
        let mut pending_nodes = 0;
        let mut read_sources = Vec::new();
        let mut t = thread_usage();
        // A file's memo entry when the run is memoised and the memo has it, else its document, built. An entry
        // without the base rows this run's cross-file part reads is a miss.
        let has_base = |e: &memo::Entry| e.rows.contains_key(OWN_DOCS) && stored.iter().all(|(rel, _)| e.rows.contains_key(&memo::base_key(rel)));
        let mut stats_read = Vec::new();
        let mut build = |d: &i64, path: &String, module: Option<Arc<str>>| -> Result<(Option<memo::Content>, std::result::Result<memo::Entry, syntax::Document>)> {
            worker_doing.set("parse");
            let _parse = crate::code_rule_dsl::logic::profile::span(|| "worker parse".into());
            // The stat index (git's): a file whose stat is the one recorded when its source was last read is that
            // source, and on a memo hit is neither read nor hashed.
            let stat = if memoised || recording {
                let _stat = crate::code_rule_dsl::logic::profile::span(|| "stat".into());
                crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::FileStat, 1);
                std::fs::metadata(root.join(path)).ok().map(|m| crate::content_store::FileStat::of(&m))
            } else {
                None
            };
            let known = stat.and_then(|now| stat_index.source(world::store_path(path), now));
            if let (true, Some(id), Some(memo)) = (memoised, known, memo) {
                let _lookup = crate::code_rule_dsl::logic::profile::span(|| "memo lookup".into());
                if let Some(entry) = memo.lookup(memo::Content::Stored(id), world::store_path(path), *d, &memo_program).map_err(|e| Error::Plan(e.to_string()))?.filter(|e| has_base(e)) {
                    return Ok((known.map(memo::Content::Stored), Ok(entry)));
                }
            }
            let read = crate::code_rule_dsl::logic::profile::span(|| "read".into());
            let source = std::fs::read_to_string(root.join(path)).map_err(|e| Error::Plan(format!("{path}: {e}")))?;
            crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, source.len() as u64);
            drop(read);
            let mut source_id = None;
            if let (true, Some(memo)) = (memoised || recording, memo) {
                // Read, the file is hashed whatever the index said: it may have changed since its stat.
                let _lookup = crate::code_rule_dsl::logic::profile::span(|| "memo lookup".into());
                crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, source.len() as u64);
                let content = memo.content(&source).map_err(|e| Error::Plan(e.to_string()))?;
                source_id = Some(content);
                if let (Some(stat), Some(id)) = (stat, content.stored())
                    && known != Some(id)
                {
                    stats_read.push((world::store_path(path).to_owned(), stat, id));
                }
            }
            let looked_up = known.is_some() && known == source_id.and_then(memo::Content::stored);
            if let (true, Some(id), Some(memo), false) = (memoised, source_id, memo, looked_up) {
                let _lookup = crate::code_rule_dsl::logic::profile::span(|| "memo lookup".into());
                if let Some(entry) = memo.lookup(id, world::store_path(path), *d, &memo_program).map_err(|e| Error::Plan(e.to_string()))?.filter(|e| has_base(e)) {
                    return Ok((source_id, Ok(entry)));
                }
            }
            let _build = crate::code_rule_dsl::logic::profile::span(|| "tree-sitter and facts".into());
            // Its own work: the file's input made from its source (the build's parts charge their own).
            crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, source.len() as u64);
            // A file of no grammar is here because `.scope text` reads it (`scoped`): text only, no tree.
            if crate::code_rule_dsl::language::label_of_path(path).is_none() {
                let facts = Arc::new(core::text_only(Arc::new(source)));
                let digest = core::digest(&facts);
                return Ok((source_id, Err(syntax::Document { key: *d as u32, path: path.clone(), core: format!("/c#core:{path}"), digest, facts, guest: None, module })));
            }
            let file = FileInput::new(path.clone(), label.to_owned(), Arc::new(source), None, None, "core").map_err(|e| Error::Plan(format!("{path}: {e:?}")))?;
            let built = match &tapes {
                // The closure sees `&tapes`, not that it is `None` without `tape-cache`: said here so the store's
                // tape and source frames are not linked.
                // A source collected since it was read is stored again with its tape (O95), under the id it has now.
                Some(cache) if cfg!(feature = "tape-cache") => core::build_cached_source(&file, cache, source_id.and_then(memo::Content::stored)).map(|(built, id)| {
                    if let Some(memo::Content::Stored(_)) = source_id {
                        source_id = id.map(memo::Content::Stored);
                    }
                    built
                }),
                Some(_) => unreachable!("no tape cache without `tape-cache`"),
                None => core::build(&file),
            };
            let facts = match built.map_err(|e| Error::Plan(format!("{path}: {e:?}")))? {
                Built::Facts(f) => Arc::new(f),
                Built::Unparsed => return Err(Error::Plan(format!("{path} did not parse"))),
            };
            let digest_span = crate::code_rule_dsl::logic::profile::span(|| "digest".into());
            let digest = core::digest(&facts);
            drop(digest_span);
            Ok((source_id, Err(syntax::Document { key: *d as u32, path: path.clone(), core: format!("/c#core:{path}"), digest, facts, guest: None, module })))
        };
        'batches: for asked in docs_in {
        let mut at = asked.iter().peekable();
        while let Some((d, path, module)) = at.next() {
            match build(d, path, module.clone()) {
                Ok((source_id, Ok(entry))) => {
                    read_sources.extend(source_id.and_then(memo::Content::stored));
                    hits.push((path.clone(), entry));
                }
                Ok((source_id, Err(doc))) => {
                    read_sources.extend(source_id.and_then(memo::Content::stored));
                    pending_nodes += doc.facts.len();
                    pending_docs.push((*d, path.clone(), source_id, doc.facts.len() as u64));
                    pending.push(doc);
                }
                Err(e) => {
                    let _ = send.send(Err(e));
                    break 'batches;
                }
            }
            if pending.is_empty() {
                // Only hits so far: handed on in batches, and at the end.
                if !hits.is_empty() && (hits.len() >= 1024 || at.peek().is_none()) {
                    // Reading, hashing and looking files up is the worker's parse time.
                    add(&mut parse, t);
                    t = thread_usage();
                    let hand = Ok((0, Vec::new(), Vec::new(), std::mem::take(&mut hits), Vec::new()));
                    worker_doing.set("hand on (memo hits)");
                    if crate::code_rule_dsl::logic::profile::wait("the main thread (a channel of one chunk)", &main_doing, || send.send(hand.map(Some))).is_err() {
                        break 'batches;
                    }
                }
                continue;
            }
            if pending_nodes < worker_target.load(std::sync::atomic::Ordering::Relaxed) && at.peek().is_some() {
                continue;
            }
            add(&mut parse, t);
            t = thread_usage();
            worker_doing.set("batch");
            let batch_span = crate::code_rule_dsl::logic::profile::span(|| "worker batch".into());
            // A chunk whose rules ask for guests keeps its documents: the guests are made from them (decision 0055).
            let kept_docs = if slice.is_some() { pending.clone() } else { Vec::new() };
            let built = syntax::with_nearest(&store, std::mem::take(&mut pending), &sets, &vocabulary, binding, &projection).map_err(|e| Error::Plan(e.to_string()));
            drop(batch_span);
            add(&mut batch, t);
            let n = std::mem::take(&mut pending_nodes);
            let (docs_of, hits_of) = (std::mem::take(&mut pending_docs), std::mem::take(&mut hits));
            let hand = built.map(|sources| (n, sources, docs_of, hits_of, kept_docs));
            worker_doing.set("hand on (a chunk)");
            if crate::code_rule_dsl::logic::profile::wait("the main thread (a channel of one chunk)", &main_doing, || send.send(hand.map(Some))).is_err() {
                break 'batches;
            }
            t = thread_usage();
        }
        // The batch's end: its documents are all handed on.
        if send.send(Ok(None)).is_err() {
            break;
        }
        }
        drop(send);
        worker_doing.set("done");
        (parse, batch, read_sources, stats_read)
    });
    // A chunk's file-local part: its memo hits replayed, its documents evaluated, its exports written to `x`.
    let mut process = |built: Chunk| -> Result<()> {
        let (chunk_nodes_built, sources, chunk_docs, hits, documents) = built;
        main_doing.set("local run");
        // The previous chunk's freed pages go back to the system: without this the footprint settles about 170 MB
        // over the live heap on the full corpus (550 MB against 300 to 350 MB), and the peak is 80 MB higher.
        {
            let _release = crate::code_rule_dsl::logic::profile::span(|| "release freed pages".into());
            crate::code_rule_dsl::prof::release();
        }
        // `RERAIL_PROF_CHUNKS=1`: the memory held as each chunk arrives (footprint, live heap, cross-file rows).
        if std::env::var_os("RERAIL_PROF_CHUNKS").is_some() {
            let live = crate::code_rule_dsl::prof::live_heap_mb();
            eprintln!(
                "chunk {chunks}: footprint {} MB (max {}), heap live {live} MB, outputs {} lines",
                crate::code_rule_dsl::prof::footprint_mb(),
                crate::code_rule_dsl::prof::max_footprint_mb(),
                outputs.values().map(Vec::len).sum::<usize>()
            );
        }
        phase("wait", statements.count());
        statements.enter("local");
        let (start_mb, start_max_mb) = (crate::code_rule_dsl::prof::footprint_mb(), crate::code_rule_dsl::prof::max_footprint_mb());
        // Memo hits: their rows go to `x` and their lines to the outputs as an evaluated chunk's would.
        if !hits.is_empty() {
            let _replay = crate::code_rule_dsl::logic::profile::span(|| "memo replay".into());
            memo_hits += hits.len();
            db.execute_batch("BEGIN")?;
            // The planner's tree counts for the files read from the memo (decision 0063): a chunk mix's measured
            // counts, scaled by the hits' nodes, as an evaluated chunk's are. Without them a run read whole from the
            // memo plans its cross-file part with no kinds (every `kind(n, "identifier")` looks empty), and a
            // join can start from a cross product of two kinds.
            // Hits come in batches that cross chunks, so they are grouped by file type, and each group takes the
            // counts of its own mix, else of a mix of its types, else of any mix measured under this program.
            if own {
                let mut groups: BTreeMap<&str, Vec<(&str, usize)>> = BTreeMap::new();
                for (path, e) in &hits {
                    let name = path.rsplit('/').next().unwrap_or(path);
                    groups.entry(name.rsplit_once('.').map_or("", |(_, ext)| ext)).or_default().push((path.as_str(), e.nodes as usize));
                }
                for (ext, files) in groups {
                    let hit_nodes: usize = files.iter().map(|(_, n)| n).sum();
                    let mix = chunk_mix(files.into_iter());
                    let keys = [mix.clone(), format!("types/{ext}"), "any".to_owned()];
                    let found = keys.iter().find_map(|k| {
                        own_once.get(k).cloned().or_else(|| own_counts(store, &memo_program, k, &mut own_sources)).filter(|(first, _)| *first > 0)
                    });
                    match found {
                        Some((first, measured)) => own_totals.add(&measured.scaled(hit_nodes as f64 / first as f64)),
                        None => {
                            memo_why.insert(format!("no tree counts for the memo hits' mix {mix}: the cross-file part plans without them"));
                        }
                    }
                }
            }
            // Each export's rows over the batch, as columns: one statement per relation writes them to `x`, from
            // a bound source (`rr_memo_w<width>`), not a statement per row.
            let mut batch: BTreeMap<String, (usize, Vec<Vec<i64>>)> = BTreeMap::new();
            for (_, entry) in hits {
                nodes += entry.nodes as usize;
                own_totals.add(&Own::taken(&entry.rows));
                for (rel, names) in &stored {
                    let Some((width, values)) = entry.rows.get(&memo::base_key(rel)) else { continue };
                    let all = super::super::sql::select::input_columns(rel).ok_or_else(|| Error::Plan(format!("no columns for {rel}")))?;
                    let held = base_rows.entry(rel.clone()).or_insert_with(|| (all.iter().map(|c| (*c).to_owned()).collect(), vec![None; all.len()]));
                    for name in names {
                        let from = all.iter().position(|c| c == name).filter(|c| *c < *width).ok_or_else(|| Error::Plan(format!("{rel} has no column {name}")))?;
                        let to = held.0.iter().position(|h| h == name).ok_or_else(|| Error::Plan(format!("{rel} has no column {name}")))?;
                        held.1[to].get_or_insert_with(Vec::new).extend(values.iter().skip(from).step_by(*width).copied());
                    }
                    // Kept whole, to be measured once after the chunks, so the cross-file part's statistics count them.
                    let full = replayed.entry(rel.clone()).or_insert_with(|| (all.iter().map(|c| (*c).to_owned()).collect(), vec![Vec::new(); *width]));
                    for (c, col) in full.1.iter_mut().enumerate() {
                        col.extend(values.iter().skip(c).step_by(*width).copied());
                    }
                }
                // Gathered by relation across the batch's hits: one statement per relation below, not one per row.
                for (rel, (width, values)) in entry.rows {
                    if values.is_empty() || !split.exports.contains(&rel) {
                        continue;
                    }
                    let cols = &mut batch.entry(rel).or_insert_with(|| (width, vec![Vec::new(); width])).1;
                    for row in values.chunks_exact(width) {
                        for (c, v) in row.iter().enumerate() {
                            cols[c].push(*v);
                        }
                    }
                }
                for (rel, (columns, lines)) in entry.lines {
                    outputs.entry(rel.clone()).or_default().extend(lines);
                    node_columns.insert(rel, columns);
                }
            }
            for (rel, (width, cols)) in batch {
                let types: Vec<&str> = lowered.program.decls[&rel].iter().map(|(_, t)| t.as_str()).collect();
                let names: Vec<String> = (0..width).map(|c| format!("c{c}")).collect();
                let rows = crate::code_rule_dsl::logic::native::component::columns_source(names.clone(), cols.into_iter().map(Some).collect());
                let via = format!("rr_memo_w{width}");
                modules.bind(&db, &via, rows.clone())?;
                let inserted = db.execute(&format!("INSERT OR IGNORE INTO x.{} SELECT {} FROM {via}", crate::code_rule_dsl::logic::sql::ident(&rel), names.join(", ")), [])?;
                totals.add_rows(&rel, inserted, rows.as_ref(), &types);
                // Released: the rows are in `x` now.
                modules.bind(&db, &via, crate::code_rule_dsl::logic::native::component::columns_source(names, vec![Some(Vec::new()); width]))?;
            }
            db.execute_batch("COMMIT")?;
        }
        if sources.is_empty() {
            phase("local", statements.count());
            statements.enter("wait");
            return Ok(());
        }
        nodes += chunk_nodes_built;
        chunks += 1;
        let mut stats = t_stats.clone();
        repository_stats(&mut stats);
        let mix = chunk_mix(chunk_docs.iter().map(|(_, path, _, n)| (path.as_str(), *n as usize)));
        let mut chunk_own: Option<Own> = None;
        if own {
            let chunk = match own_once.get(&mix) {
                Some((first, measured)) if *first > 0 => measured.scaled(chunk_nodes_built as f64 / *first as f64),
                _ => {
                    let _measure = crate::code_rule_dsl::logic::profile::span(|| format!("measure constants ({mix})"));
                    let measured = Own::measure(&sources, &constants);
                    own_once.insert(mix.clone(), (chunk_nodes_built, measured.clone()));
                    if memoised {
                        // Under its mix, its types, and as the program's last measured mix (a hit's fallbacks).
                        let types = mix.split('/').next().unwrap_or_default();
                        for key in [mix.clone(), format!("types/{types}"), "any".to_owned()] {
                            record_own_counts(store, &memo_program, &key, chunk_nodes_built, &measured, &mut own_sources)?;
                        }
                    }
                    measured
                }
            };
            chunk.apply(&mut stats);
            own_totals.add(&chunk);
            chunk_own = Some(chunk);
        }
        // Guests (decision 0055): the embed slice runs over the chunk, the guests it asks for are parsed, and the
        // batch is built again with them, until a round asks for none new. The batch bound below holds them.
        let mut host_of: HashMap<i64, i64> = HashMap::new();
        // A value document's files: those whose rules asked for it (decision 0058 §4, O96).
        let mut askers: HashMap<i64, Vec<i64>> = HashMap::new();
        let sources = match &slice {
            Some(slice) if !documents.is_empty() => {
                let _guests = crate::code_rule_dsl::logic::profile::span(|| "guests".into());
                let (mut docs, mut sources, mut made) = (documents, sources, guests::Guests::new(label));
                // The slice's planner statistics: what it reads, measured once, in the first round's batch. A later
                // round adds only the guests' rows, which are few beside the files' (each measurement sorts a column).
                let mut measured: BTreeMap<String, Vec<(String, i64, i64, i64, i64)>> = BTreeMap::new();
                for _ in 0..guests::DEPTH {
                    let mut round = stats.clone();
                    for (rel, source) in &sources {
                        if slice_reads.contains(rel) {
                            let m = measured.entry(rel.clone()).or_insert_with(|| {
                                let _m = crate::code_rule_dsl::logic::profile::span(|| "guest statistics".into());
                                syntax::statistics(source.as_ref())
                            });
                            round.measured(rel, m.iter().cloned().map(|(c, rows, ndv, sq, sqd)| (c, [rows, ndv, sq, sqd])), []);
                        }
                        modules.bind(&db, rel, Arc::clone(source))?;
                    }
                    let got = super::run_atoms_native_over(slice, &db, "main", round, BTreeSet::new(), modes)?;
                    let new = made.make(guests::requests(&got.outputs), &docs, tapes.as_ref()).map_err(Error::Plan)?;
                    if new.is_empty() {
                        break;
                    }
                    if own {
                        let ranges = new.iter().map(|d| i64::from(d.key)).map(|k| vec![SqlValue::Integer(k), SqlValue::Integer(k << 32), SqlValue::Integer((k << 32) | 0xffff_ffff)]).collect();
                        super::insert_many(&db, "INSERT OR IGNORE INTO s.stat_docrange", ranges)?;
                    }
                    docs.extend(new);
                    let _rebuild = crate::code_rule_dsl::logic::profile::span(|| "batch with guests".into());
                    sources = syntax::with_nearest(&store, docs.clone(), &sets, &vocabulary, binding, &projection).map_err(|e| Error::Plan(e.to_string()))?;
                }
                host_of = std::mem::take(&mut made.host_of);
                askers = made.askers();
                sources
            }
            _ => sources,
        };
        // The files a row is attributed to: its document's (a guest's is its host file's), or every file that asked
        // for its value document. Rows are sets, so a value's rows kept by each of its files restore as one.
        let owners = |v: i64| -> Vec<i64> {
            let d = memo::document_of(v);
            match askers.get(&d) {
                Some(files) => files.clone(),
                None => vec![host_of.get(&d).copied().unwrap_or(d)],
            }
        };
        // The base rows a memoised chunk hands on, per relation: the columns in `input_columns` order, and the rows.
        let mut base_seen: Vec<(String, Vec<usize>, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)> = Vec::new();
        let mut base_whole = true;
        // The semi-join's relations, taken after the run (`semijoin`), and the values the chunk's exports hold.
        let mut deferred: Vec<(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, Vec<(usize, usize)>)> = Vec::new();
        let mut mentioned: std::collections::HashSet<i64> = std::collections::HashSet::new();
        let mention = |mentioned: &mut std::collections::HashSet<i64>, rows: &dyn crate::code_rule_dsl::facts::vtab::Source| {
            let width = rows.columns().len();
            for r in 0..rows.len() {
                mentioned.extend((0..width).map(|c| rows.value(r, c)));
            }
        };
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
                // A semi-join's relation is taken after the run, the rows of the nodes the exports hold (`semijoin`).
                let mut pairs: Vec<(usize, usize)> = Vec::new();
                for name in names {
                    let c = have.iter().position(|h| h == name).ok_or_else(|| Error::Plan(format!("{rel} has no column {name}")))?;
                    let to = held.0.iter().position(|h| h == name).ok_or_else(|| Error::Plan(format!("{rel} has no column {name}")))?;
                    let col = held.1[to].get_or_insert_with(Vec::new);
                    if !semijoin.contains(&rel) {
                        col.extend((0..source.len()).map(|r| source.value(r, c)));
                    }
                    pairs.push((c, to));
                }
                if semijoin.contains(&rel) {
                    deferred.push((rel.clone(), Arc::clone(&source), pairs));
                }
                // Memoised, each row goes to its document's entry whole, in `input_columns` order, as its first
                // column names it (a node or a document key).
                if memoised {
                    let all = super::super::sql::select::input_columns(&rel).ok_or_else(|| Error::Plan(format!("no columns for {rel}")))?;
                    let at: Option<Vec<usize>> = all.iter().map(|c| have.iter().position(|h| h == c)).collect();
                    match at {
                        Some(at) => base_seen.push((rel.clone(), at, source.clone())),
                        None => base_whole = false,
                    }
                }
            }
            let key = (mix.clone(), rel.clone());
            let measured = match measured_once.get(&key) {
                Some((first, measured)) if *first > 0 => scaled_statistics(measured, *first, source.estimated_len()),
                _ => {
                    let _measure = crate::code_rule_dsl::logic::profile::span(|| format!("measure {rel} ({mix})"));
                    let measured = syntax::statistics(source.as_ref());
                    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Value, (source.len() * source.columns().len()) as u64);
                    measured_once.insert(key, (source.estimated_len(), measured.clone()));
                    measured
                }
            };
            // A memoised run measures the base relations the cross-file part reads once, over every file's rows (below).
            if split.base.contains(&rel) && !(memoised && stored.iter().any(|(r, _)| *r == rel)) && (own || !t_stats.knows(&rel) || crate::code_rule_dsl::logic::front::is_transduced(&rel) || rel == syntax::WITHIN || rel == syntax::SPAN_WITHIN || rel.starts_with(crate::code_rule_dsl::logic::strings::RELATION)) {
                totals.add(&rel, source.estimated_len(), &measured);
            }
            stats.measured(&rel, measured.into_iter().map(|(c, rows, ndv, sq, sqd)| (c, [rows, ndv, sq, sqd])), []);
            modules.bind(&db, &rel, source)?;
            bound.insert(rel);
        }
        let running = crate::code_rule_dsl::logic::profile::span(|| "run".into());
        // Without the memo (which attributes each row to its file once the run ends), an export the engine holds is
        // written to `x` and measured as soon as no later stratum reads it, and dropped: the chunk's exports are not
        // all held to its end.
        let got = if memoised {
            super::run_atoms_native_over(&local, &db, "main", stats, split.exports.iter().cloned().collect(), modes)?
        } else {
            let mut hand = |rel: &str| -> Result<()> {
                if !split.exports.iter().any(|e| e == rel) {
                    return Ok(());
                }
                let types: Vec<&str> = lowered.program.decls[rel].iter().map(|(_, t)| t.as_str()).collect();
                let cols = (0..types.len()).map(|i| format!("c{i}")).collect::<Vec<_>>().join(", ");
                let inserted = db.execute(&format!("INSERT OR IGNORE INTO x.{} SELECT {cols} FROM set_derived WHERE rel = ?1", crate::code_rule_dsl::logic::sql::ident(rel)), [rel])?;
                let rows = crate::code_rule_dsl::logic::native::component::derived(&db, rel).ok_or_else(|| Error::Plan(format!("{rel} is not held")))?;
                totals.add_rows(rel, inserted, rows.as_ref(), &types);
                if !deferred.is_empty() {
                    mention(&mut mentioned, rows.as_ref());
                }
                Ok(())
            };
            super::run_atoms_native_hooked(&local, &db, "main", stats, split.exports.iter().cloned().collect(), modes, super::Hooks { hand_off: Some(&mut hand), ..Default::default() })?
        };
        drop(running);
        if chunk_nodes_built > 0 {
            // What this chunk added at its peak (the lifetime peak, when it rose here; else the footprint now).
            let (now, max) = (crate::code_rule_dsl::prof::footprint_mb(), crate::code_rule_dsl::prof::max_footprint_mb());
            let added = if max > start_max_mb { max } else { now }.saturating_sub(start_mb).max(1);
            let per_node = added as f64 * 1048576.0 / chunk_nodes_built as f64;
            let env = |k: &str, d: f64| std::env::var(k).ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(d);
            let headroom = (env("RERAIL_RUN_MB", RUN_MB) - start_mb as f64).max(0.0) * 0.75;
            let budget = env("RERAIL_CHUNK_MB", CHUNK_MB).min(headroom).round();
            let next = ((budget * 1048576.0 / per_node) as usize).clamp(MIN_CHUNK_NODES.min(chunk_nodes), chunk_nodes);
            let chunk = env("RERAIL_CHUNK_MB", CHUNK_MB);
            chunk_target.store(next, std::sync::atomic::Ordering::Relaxed);
            crate::code_rule_dsl::logic::profile::decision("chunk size", || format!("a chunk of {chunk_nodes_built} nodes added {added} MB ({per_node:.0} bytes a node); the next are about {next} nodes ({budget} MB a chunk: the least of {chunk} MB and three quarters of the run's headroom above {start_mb} MB; at most {chunk_nodes})"));
        }
        let handed = got.handed.clone();
        let kept = got.kept.clone();
        let outputs_of = |rel: &str| prog.outputs.iter().any(|o| o == rel);
        // Per document of the chunk, what it hands on (`memo::Entry`), when the run is memoised. A row is its
        // document's by its first location column.
        let mut entries: HashMap<i64, memo::Entry> = if memoised { chunk_docs.iter().map(|(d, _, _, n)| (*d, memo::Entry { nodes: *n, ..memo::Entry::default() })).collect() } else { HashMap::new() };
        let mut attributable = memoised && base_whole && chunk_own.is_some();
        if let (true, Some(chunk)) = (attributable, &chunk_own) {
            // Each document's share of the chunk's tree statistics, by its nodes, as a chunk mix is scaled.
            for entry in entries.values_mut() {
                chunk.scaled(entry.nodes as f64 / chunk_nodes_built.max(1) as f64).put(&mut entry.rows);
            }
        }
        if memoised {
            for (rel, lines) in got.outputs.iter().filter(|(rel, _)| outputs_of(rel)) {
                let at = split.locations.get(rel).and_then(|l| l.first().copied());
                for line in lines {
                    let files = at.and_then(|c| line.split('\t').nth(c)).and_then(|v| v.parse::<i64>().ok()).map(owners).unwrap_or_default();
                    if files.is_empty() || !files.iter().all(|d| entries.contains_key(d)) {
                        memo_why.insert(format!("output {rel}: {}", if files.is_empty() { "no location" } else { "another chunk's document" }));
                        attributable = false;
                        continue;
                    }
                    for d in files {
                        let e = entries.get_mut(&d).expect("checked");
                        e.lines.entry(rel.clone()).or_insert_with(|| (got.nodes[rel].clone(), Vec::new())).1.push(line.clone());
                    }
                }
            }
        }
        native.extend(gather(got, &mut outputs, &mut node_columns, &outputs_of).into_iter().map(|k| format!("local:{k}")));
        let _export = crate::code_rule_dsl::logic::profile::span(|| "export to cross".into());
        let mut attribute = |rel: &str, rows: &dyn crate::code_rule_dsl::facts::vtab::Source| {
            if !attributable {
                return;
            }
            let Some(&at) = split.locations.get(rel).and_then(|l| l.first()) else {
                memo_why.insert(format!("export {rel}: no location column"));
                attributable = false;
                return;
            };
            let width = rows.columns().len();
            for r in 0..rows.len() {
                let files = owners(rows.value(r, at));
                if !files.iter().all(|d| entries.contains_key(d)) {
                    memo_why.insert(format!("export {rel}: {}", if files.is_empty() { "a row naming no document" } else { "a row naming another chunk's document" }));
                    attributable = false;
                    return;
                }
                for d in files {
                    let held = entries.get_mut(&d).expect("checked").rows.entry(rel.to_owned()).or_insert_with(|| (width, Vec::new()));
                    held.1.extend((0..width).map(|c| rows.value(r, c)));
                }
            }
        };
        db.execute_batch("BEGIN")?;
        for rel in split.exports.iter().filter(|r| !handed.contains(*r)) {
            // Rows another chunk wrote too (a file-local relation without a location column) are kept once. A kept
            // export goes from the engine to `x` in one statement and is measured in place; one a SQL stratum
            // derived is in its table.
            let types: Vec<&str> = lowered.program.decls[rel].iter().map(|(_, t)| t.as_str()).collect();
            if kept.contains(rel) {
                let cols = (0..types.len()).map(|i| format!("c{i}")).collect::<Vec<_>>().join(", ");
                let inserted = db.execute(&format!("INSERT OR IGNORE INTO x.{} SELECT {cols} FROM set_derived WHERE rel = ?1", crate::code_rule_dsl::logic::sql::ident(rel)), [rel])?;
                let rows = crate::code_rule_dsl::logic::native::component::derived(&db, rel).ok_or_else(|| Error::Plan(format!("{rel} is not held")))?;
                totals.add_rows(rel, inserted, rows.as_ref(), &types);
                attribute(rel, rows.as_ref());
                if !deferred.is_empty() {
                    mention(&mut mentioned, rows.as_ref());
                }
            } else {
                let inserted = db.execute(&format!("INSERT OR IGNORE INTO x.{q} SELECT * FROM temp.{q}", q = crate::code_rule_dsl::logic::sql::ident(rel)), [])?;
                let rows = Rows::read(&db, &format!("temp.{}", crate::code_rule_dsl::logic::sql::ident(rel)))?;
                totals.add_rows(rel, inserted, &rows, &types);
                attribute(rel, &rows);
                if !deferred.is_empty() {
                    mention(&mut mentioned, &rows);
                }
            }
        }
        db.execute_batch("COMMIT")?;
        // The semi-join: of each deferred base relation, the rows whose node an export row holds.
        for (rel, source, pairs) in std::mem::take(&mut deferred) {
            let have = source.columns();
            let key = have.iter().position(|h| h == "n").ok_or_else(|| Error::Plan(format!("{rel} has no node column")))?;
            let keep: Vec<usize> = (0..source.len()).filter(|&r| mentioned.contains(&source.value(r, key))).collect();
            let held = base_rows.get_mut(&rel).expect("deferred relations are held");
            for &(c, to) in &pairs {
                held.1[to].get_or_insert_with(Vec::new).extend(keep.iter().map(|&r| source.value(r, c)));
            }
        }
        // Each memoised file's base rows, once the exports name the nodes a semi-join keeps.
        if attributable {
            for entry in entries.values_mut() {
                for (rel, _) in &stored {
                    let width = super::super::sql::select::input_columns(rel).map_or(0, <[_]>::len);
                    entry.rows.insert(memo::base_key(rel), (width, Vec::new()));
                }
            }
            'base: for (rel, at, source) in &base_seen {
                let key = memo::base_key(rel);
                // A semi-join's relation: the rows of the nodes the chunk's exports hold, as the cross part keeps.
                let node = semijoin.contains(rel).then(|| source.columns().iter().position(|h| h == "n")).flatten();
                for r in (0..source.len()).filter(|&r| node.is_none_or(|n| mentioned.contains(&source.value(r, n)))) {
                    // A row is each of its owners' (a guest's is its host's; a value document's, each asking file's):
                    // rows are sets, so a row kept by several files restores as one.
                    let files = at.first().map_or_else(Vec::new, |c| owners(source.value(r, *c)));
                    if files.is_empty() || !files.iter().all(|f| entries.contains_key(f)) {
                        attributable = false;
                        break 'base;
                    }
                    for f in files {
                        let held = &mut entries.get_mut(&f).expect("checked").rows.get_mut(&key).expect("every entry has every base key").1;
                        held.extend(at.iter().map(|c| source.value(r, *c)));
                    }
                }
            }
        }
        // The chunk's native relations are written to `x`: none is read again.
        crate::code_rule_dsl::logic::native::component::forget_derived(&db);
        if memoised && attributable {
            let _remember = crate::code_rule_dsl::logic::profile::span(|| "memo remember".into());
            for (d, path, source_id, _) in &chunk_docs {
                let (Some(id), Some(entry), Some(memo)) = (source_id, entries.get(d), memo) else { continue };
                memo.remember(*id, world::store_path(path), *d, &memo_program, entry).map_err(|e| Error::Plan(format!("{path}: memo publication for source {id:?}: {e}")))?;
            }
        } else if memoised {
            memo_unattributed += 1;
        }
        // The chunk's sources are dropped before the next is built.
        for rel in &bound {
            if let Some(b) = modules.get(rel) {
                b.clear();
            }
        }
        crate::code_rule_dsl::logic::native::forget_inputs(&db);
        phase("local", statements.count());
        statements.enter("wait");
        Ok(())
    };
    // One batch's chunks, to its end; false when the builder has ended.
    let drain = |process: &mut dyn FnMut(Chunk) -> Result<()>| -> Result<bool> {
        let got = (|| loop {
            main_doing.set("wait");
            let next = crate::code_rule_dsl::logic::profile::wait("the worker's next chunk", &worker_doing, || match chunks_in.borrow().as_ref() {
                Some(rx) => rx.recv().map_err(|_| ()),
                None => Err(()),
            });
            match next {
                Err(()) => return Ok(false),
                Ok(Err(e)) => return Err(e),
                Ok(Ok(None)) => return Ok(true),
                Ok(Ok(Some(chunk))) => process(chunk)?,
            }
        })();
        if got.is_err() {
            chunks_in.borrow_mut().take();
        }
        got
    };
    drain(&mut process)?;
    // Requests answered inside the fixpoint (decision 0065 §2a). The cone of requests runs on a connection of its
    // own, in a thread of its own, and pauses; this thread answers: a directory's entries and a file's lines from
    // `World`, and a load by sending the document through the chunk path, as any file goes (the memo by content,
    // else parsed), then handing on its rows of the per-file relations the cone reads.
    if let (Some((load, reads)), Some(docs_tx)) = (&loading, &docs_out) {
        let _loading = crate::code_rule_dsl::logic::profile::span(|| "requests answered inside the fixpoint".into());
        let mut world = world::World::new(root, &prog);
        let corpus: BTreeSet<String> = world.canonical_corpus(docs.iter().map(|(_, p)| p));
        let mut all_paths: Vec<String> = docs.iter().map(|(_, p)| p.clone()).collect();
        let per_file: Vec<String> = load.answers.get(world::LOAD).into_iter().flatten().filter(|r| split.exports.contains(r)).cloned().collect();
        let world_rels: Vec<(String, usize)> = load.answers.values().flatten().filter(|r| !split.exports.contains(r)).filter_map(|r| load.decls.get(r).map(|d| (r.clone(), d.len()))).collect();
        // Those the cone takes as inputs start empty; the others are its own tables.
        let world_inputs: Vec<(String, usize)> = world_rels.iter().filter(|(r, _)| load.inputs.contains(r)).cloned().collect();
        // What the rest of the cross-file part reads of the cone; what was answered is kept here, as it is given.
        let kept: BTreeSet<String> = reads.clone();
        let mut answered: BTreeMap<String, Vec<Vec<i64>>> = world_rels.iter().map(|(r, _)| (r.clone(), Vec::new())).collect();
        let lowered_load = part(load.clone());
        let load_stats = t_stats.clone();
        let (ask_tx, ask_rx) = std::sync::mpsc::channel::<(String, Vec<Vec<i64>>)>();
        let (reply_tx, reply_rx) = std::sync::mpsc::channel::<std::result::Result<Vec<(String, Vec<Vec<i64>>)>, String>>();
        let (db_path, x_path, per_file_in, kept_in, world_in) = (path.clone(), cross_path.clone(), per_file.clone(), kept.clone(), world_inputs);
        let decls = load.decls.clone();
        let cone = scope.spawn(move || -> Result<Vec<(String, Vec<Vec<i64>>)>> {
            let db2 = Connection::open(&db_path)?;
            db2.execute_batch("PRAGMA synchronous=OFF")?;
            db2.execute("ATTACH ?1 AS x", [x_path.to_string_lossy()])?;
            db2.execute_batch("ATTACH ':memory:' AS s; CREATE TABLE s.stat_docrange(d INTEGER PRIMARY KEY, lo INTEGER, hi INTEGER);")?;
            crate::code_rule_dsl::logic::native::register(&db2)?;
            crate::code_rule_dsl::logic::native::register_symbols(&db2)?;
            let modules = Modules::default();
            let mut load_stats = load_stats;
            let mut bind = |rel: &str, source: Arc<dyn crate::code_rule_dsl::facts::vtab::Source>| -> Result<()> {
                load_stats.measured(rel, syntax::statistics(source.as_ref()).into_iter().map(|(c, rows, ndv, sq, sqd)| (c, [rows, ndv, sq, sqd])), []);
                modules.bind(&db2, rel, source)?;
                Ok(())
            };
            for rel in &per_file_in {
                let source = crate::code_rule_dsl::logic::native::component::table_source(&db2, "x", rel).ok_or_else(|| Error::Plan(format!("x.{rel} is not a table of integers")))?;
                bind(rel, source)?;
            }
            for (rel, width) in &world_in {
                bind(rel, crate::code_rule_dsl::logic::native::component::columns_source((0..*width).map(|c| format!("c{c}")).collect(), vec![Some(Vec::new()); *width]))?;
            }
            let mut answer = |rel: &str, rows: &[Vec<i64>]| -> Result<Vec<(String, Vec<Vec<i64>>)>> {
                ask_tx.send((rel.to_owned(), rows.to_vec())).map_err(|_| Error::Plan("the request channel closed".to_owned()))?;
                reply_rx.recv().map_err(|_| Error::Plan("the answer channel closed".to_owned()))?.map_err(Error::Plan)
            };
            let got = super::run_atoms_native_answering(&lowered_load, &db2, "main", load_stats, kept_in.clone(), modes, &mut answer)?;
            let mut out = Vec::new();
            for rel in &kept_in {
                out.push((rel.clone(), held_rows(&db2, &got, rel, decls[rel].len())?));
            }
            Ok(out)
        });
        // The answers turn request atoms back into text, warm runs included (nothing else registers them then).
        crate::code_rule_dsl::logic::native::register_symbols(&db)?;
        let text = |atom: i64| -> Result<String> { Ok(db.query_row("SELECT sym_text(?1)", [atom], |r| r.get(0))?) };
        let atoms = |texts: &[&str]| -> Result<Vec<i64>> { Ok(crate::code_rule_dsl::logic::native::intern_batch(&db, texts.iter().copied())?) };
        let mut pauses = 0;
        let mut answer_one = |rel: &str, rows: &[Vec<i64>], process: &mut dyn FnMut(Chunk) -> Result<()>| -> Result<Vec<(String, Vec<Vec<i64>>)>> {
            pauses += 1;
            if let Some((produced, _)) = WORLD.iter().find(|(_, req)| *req == rel) {
                let mut out = Vec::new();
                for row in rows {
                    let asked = text(row[0])?;
                    if *produced == "dir_entry" {
                        let entries = world.dir(&asked);
                        let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
                        for ((_, is_dir), name) in entries.iter().zip(atoms(&names)?) {
                            out.push(vec![row[0], name, i64::from(*is_dir)]);
                        }
                    } else {
                        let lines = world.file(&asked);
                        let texts: Vec<&str> = lines.iter().map(String::as_str).collect();
                        for (i, line) in atoms(&texts)?.into_iter().enumerate() {
                            out.push(vec![row[0], i as i64 + 1, line]);
                        }
                    }
                }
                return Ok(vec![((*produced).to_owned(), out)]);
            }
            if rel != world::LOAD {
                return Err(Error::Plan(format!("{rel}: a request this run does not answer")));
            }
            let mut fresh = Vec::new();
            let mut loaded_rows = Vec::new();
            for row in rows {
                let (asked, module, grammar) = (text(row[0])?, text(row[1])?, text(row[2])?);
                if let Some(doc) = world.load(&asked, &module, &grammar, label, &corpus) {
                    loaded_rows.push(vec![atoms(&[doc.path.as_str()])?[0], row[1], row[2]]);
                    fresh.push(doc);
                }
            }
            let mut out = vec![(world::LOADED.to_owned(), loaded_rows)];
            if fresh.is_empty() {
                out.extend(per_file.iter().map(|r| (r.clone(), Vec::new())));
                return Ok(out);
            }
            // A document's key is a function of its path (decision 0052).
            all_paths.extend(fresh.iter().map(|d| d.path.clone()));
            let keys = memo::doc_keys(&all_paths);
            let keys = &keys[keys.len() - fresh.len()..];
            let ranges = keys.iter().map(|d| vec![SqlValue::Integer(*d), SqlValue::Integer(d << 32), SqlValue::Integer((d << 32) | 0xffff_ffff)]).collect();
            super::insert_many(&db, "INSERT OR IGNORE INTO s.stat_docrange", ranges)?;
            loaded_docs.extend(keys.iter().copied().zip(fresh.iter().map(|d| d.path.clone())));
            docs_tx.send(keys.iter().copied().zip(fresh).map(|(k, d)| (k, d.path, Some(Arc::from(d.module.as_str())))).collect()).map_err(|_| Error::Plan("the chunk builder ended".to_owned()))?;
            if !drain(process)? {
                return Err(Error::Plan("the chunk builder ended before a load's documents".to_owned()));
            }
            // The new documents' rows of the per-file relations the cone reads, from `x`, by their first location:
            // one statement per relation for the whole batch, a range join on the key when that column leads it.
            let new: std::collections::HashSet<i64> = keys.iter().copied().collect();
            db.execute_batch("CREATE TEMP TABLE IF NOT EXISTS rr_load_new(lo INTEGER, hi INTEGER); DELETE FROM temp.rr_load_new;")?;
            super::insert_many(&db, "INSERT INTO temp.rr_load_new", keys.iter().map(|d| vec![SqlValue::Integer(d << 32), SqlValue::Integer((d << 32) | 0xffff_ffff)]).collect())?;
            for rel in &per_file {
                let at = split.locations.get(rel).and_then(|l| l.first().copied()).ok_or_else(|| Error::Plan(format!("{rel}: no location column")))?;
                let width = lowered.program.decls[rel].len();
                let table = crate::code_rule_dsl::logic::sql::ident(rel);
                let row = |r: &rusqlite::Row| (0..width).map(|c| r.get::<_, i64>(c)).collect::<rusqlite::Result<Vec<_>>>();
                let rows: Vec<Vec<i64>> = if at == 0 {
                    let cols = (0..width).map(|c| format!("t.c{c}")).collect::<Vec<_>>().join(", ");
                    db.prepare(&format!("SELECT {cols} FROM temp.rr_load_new n JOIN x.{table} t ON t.c0 BETWEEN n.lo AND n.hi"))?
                        .query_map([], row)?
                        .collect::<rusqlite::Result<_>>()?
                } else {
                    let cols = (0..width).map(|c| format!("c{c}")).collect::<Vec<_>>().join(", ");
                    let all: Vec<Vec<i64>> = db.prepare(&format!("SELECT {cols} FROM x.{table}"))?.query_map([], row)?.collect::<rusqlite::Result<_>>()?;
                    all.into_iter().filter(|r| new.contains(&memo::document_of(r[at]))).collect()
                };
                out.push((rel.clone(), rows));
            }
            Ok(out)
        };
        while let Ok((rel, rows)) = ask_rx.recv() {
            let reply = answer_one(&rel, &rows, &mut process).map_err(|e| e.to_string());
            if let Ok(given) = &reply {
                for (r, rows) in given {
                    if let Some(kept) = answered.get_mut(r) {
                        kept.extend(rows.iter().cloned());
                    }
                }
            }
            if reply_tx.send(reply).is_err() {
                break;
            }
        }
        drop(reply_tx);
        drop(answer_one);
        loading_rows = cone.join().map_err(|_| Error::Plan("the cone of requests panicked".to_owned()))??;
        loading_rows.extend(answered);
        let summary = world.summary(pauses);
        consulted.extend(world.consulted());
        crate::code_rule_dsl::logic::profile::decision("file system and loaded documents (t21)", || summary);
    }
    // The builder ends with its channel.
    docs_out = None;
    drop(docs_out);
    main_doing.set("done");
    crate::code_rule_dsl::logic::profile::wait("the worker's end", &worker_doing, || worker.join()).map_err(|_| Error::Plan("the chunk builder panicked".to_owned()))
    })?;
    (parse_cpu.0, parse_cpu.1, batch_cpu.0, batch_cpu.1) = (worker_cpu.0.0, worker_cpu.0.1, worker_cpu.1.0, worker_cpu.1.1);
    // The loaded documents are the run's documents from here on; their rows are not its findings.
    for (d, p) in &loaded_docs {
        paths.insert(*d, p.clone());
        loaded_keys.insert(*d);
    }
    docs.extend(loaded_docs.iter().cloned());
    // The run's manifest, then collection in the background while the cross-file part runs (decision 0053).
    let mut run_sources = worker_cpu.2.clone();
    run_sources.extend(own_sources.iter().copied());
    run_sources.extend(program_sources.iter().copied());
    if recording {
        let _stats = crate::code_rule_dsl::logic::profile::span(|| "record stats".into());
        store.record_corpus_stats(&canonical_root, &worker_cpu.3, started_ns).map_err(|e| Error::Plan(e.to_string()))?;
    }
    // Publish all source references before releasing the lease and starting collection (decision 0053).
    // The memo's relations, named and typed, so a SQL session can read the run's frames (docs/sql-access).
    // An export's entry holds rows (its columns, as JSON); a file-local output's holds lines (`{"lines": columns}`).
    let mut relations: Vec<(String, String)> = split
        .exports
        .iter()
        .filter_map(|rel| lowered.program.decls.get(rel).map(|cols| (rel.clone(), serde_json::to_string(cols).unwrap_or_default())))
        .collect();
    for rel in &local_outputs {
        if let Some(cols) = lowered.program.decls.get(rel).filter(|_| !relations.iter().any(|(r, _)| r == rel)) {
            relations.push((rel.clone(), serde_json::json!({ "lines": cols }).to_string()));
        }
    }
    // With the cross-file memo, the manifest also names the components' stored rows (decision 0076), known only
    // after the cross-file part: published then, the lease held until.
    let mut publish_later = None;
    let mut collecting = if recording && cross_plan.is_none() {
        record_run(store, root, &run_sources, &memo_program, &program_plans, &relations, reading)?
    } else if recording {
        publish_later = Some((relations, reading));
        None
    } else {
        drop(reading);
        None
    };
    statements.enter("stats");
    // An export no evaluated chunk measured (every file read from the memo, or none with rows): measured as empty.
    for rel in &split.exports {
        if !totals.rels.contains_key(rel) {
            let width = lowered.program.decls[rel].len();
            let types: Vec<&str> = lowered.program.decls[rel].iter().map(|(_, t)| t.as_str()).collect();
            let empty = crate::code_rule_dsl::logic::native::component::columns_source((0..width).map(|c| format!("c{c}")).collect(), vec![Some(Vec::new()); width]);
            totals.add_rows(rel, 0, empty.as_ref(), &types);
        }
    }
    // A memoised run's base relations, as the cross-file part reads them: every file's rows, read from the memo or
    // evaluated, measured once (a file read from the memo has no chunk to measure them in).
    if memoised {
        // A relation no file had rows of (none evaluated, or none with rows) is held empty: the cross-file part binds it.
        for (rel, names) in &stored {
            if !base_rows.contains_key(rel) {
                let have: Vec<String> = super::super::sql::select::input_columns(rel).unwrap_or_default().iter().map(|c| (*c).to_owned()).collect();
                let values = have.iter().map(|c| names.contains(c).then(Vec::new)).collect();
                base_rows.insert(rel.clone(), (have, values));
            }
        }
        for (rel, (columns, values)) in &base_rows {
            let (names, held): (Vec<String>, Vec<Option<Vec<i64>>>) = columns.iter().zip(values).filter(|(_, v)| v.is_some()).map(|(c, v)| (c.clone(), v.clone())).unzip();
            let rows = held.first().and_then(Option::as_ref).map_or(0, Vec::len);
            let source = crate::code_rule_dsl::logic::native::component::columns_source(names, held);
            let mut measured = syntax::statistics(source.as_ref());
            // A column no cross-file rule names is not held: the planner still asks for it, so it is stated as keys.
            let n = rows as i64;
            measured.extend(columns.iter().zip(values).filter(|(_, v)| v.is_none()).map(|(c, _)| (c.clone(), n, n, n, n)));
            totals.add(rel, rows, &measured);
        }
    }
    // The cross-file part's statistics: T's for the base relations, the chunks' totals for the rest.
    let mut cross_stats = t_stats;
    if own {
        own_totals.apply(&mut cross_stats);
    }
    for (rel, (columns, values)) in std::mem::take(&mut replayed) {
        let rows = crate::code_rule_dsl::logic::native::component::columns_source(columns, values.into_iter().map(Some).collect());
        let _measure = crate::code_rule_dsl::logic::profile::span(|| format!("measure {rel} (memo)"));
        totals.add(&rel, rows.estimated_len(), &syntax::statistics(rows.as_ref()));
    }
    totals.apply(&mut cross_stats);
    // Applied: the chunks' per-value counts are not read again.
    totals.vals = BTreeMap::new();
    repository_stats(&mut cross_stats);
    // RERAIL_DUMP_STATS=a,b: what the cross-file part plans with for those relations (a debugging aid).
    if let Ok(rels) = std::env::var("RERAIL_DUMP_STATS") {
        for rel in rels.split(',') {
            eprintln!("cross stats {}", cross_stats.describe(rel));
        }
    }
    // With no documents there was no batch to bind or measure base relations.
    // They are known empty, but global rules (including absence checks) still execute.
    if docs.is_empty() {
        for rel in &split.base {
            if repository.iter().any(|(name, _, _)| name == rel) {
                continue;
            }
            let columns = super::super::sql::select::input_columns(rel).ok_or_else(|| Error::Plan(format!("no columns for {rel}")))?;
            let empty = crate::code_rule_dsl::logic::native::component::columns_source(
                columns.iter().map(|name| (*name).to_owned()).collect(), vec![Some(Vec::new()); columns.len()]);
            cross_stats.measured(rel, columns.iter().map(|name| ((*name).to_owned(), [0, 0, 0, 0])), []);
            cross_inputs.push((rel.clone(), Arc::clone(&empty)));
            modules.bind(&db, rel, empty)?;
        }
    }

    // The cross-file part reads every input in place from `main`: the base relations held in memory, and the
    // exports, read from `x` once each.
    crate::code_rule_dsl::logic::profile::decision("held for the cross-file part", || {
        let mut held: Vec<(usize, String)> = base_rows
            .iter()
            .map(|(rel, (columns, values))| {
                let n: usize = values.iter().flatten().map(Vec::len).sum();
                let named: Vec<&str> = columns.iter().zip(values).filter(|(_, v)| v.is_some()).map(|(c, _)| c.as_str()).collect();
                (n, format!("{rel}({})", named.join(", ")))
            })
            .collect();
        held.sort_by(|a, b| b.0.cmp(&a.0));
        let total: usize = held.iter().map(|h| h.0).sum();
        let top: Vec<String> = held.iter().take(12).map(|(n, r)| format!("{r} {:.1} MB", *n as f64 * 8.0 / 1e6)).collect();
        format!("{} base relations, {:.1} MB of values in memory: {}", held.len(), total as f64 * 8.0 / 1e6, top.join("; "))
    });
    for (rel, (columns, values)) in std::mem::take(&mut base_rows) {
        // A base relation the cross-file part reads that T's statistics do not cover (a simplified rule moved its read
        // there, `parsed` after a filter was inlined): measured from the rows held. A column not held is read by no
        // rule; it is taken as a key.
        if !cross_stats.knows(&rel) {
            let n = values.iter().flatten().map(Vec::len).next().unwrap_or(0) as i64;
            let measured: Vec<(String, [i64; 4])> = columns
                .iter()
                .zip(&values)
                .map(|(c, v)| {
                    let m = match v {
                        Some(v) => {
                            let mut per: HashMap<i64, i64> = HashMap::new();
                            v.iter().for_each(|x| *per.entry(*x).or_default() += 1);
                            let sq: i64 = per.values().map(|k| k * k).sum();
                            [n, per.len() as i64, sq, sq]
                        }
                        None => [n, n, n, n],
                    };
                    (c.clone(), m)
                })
                .collect();
            cross_stats.measured(&rel, measured, []);
        }
        let source = crate::code_rule_dsl::logic::native::component::columns_source(columns, values);
        cross_inputs.push((rel.clone(), Arc::clone(&source)));
        modules.bind(&db, &rel, source)?;
    }
    // One fixpoint (no bounded rounds) reads each export from `x` when its first reader runs and drops it, and each
    // base relation held for it, after its last; rounds read them all, every round. A session's run keeps its
    // cross-file inputs for the next (`retain`, decision 0077), so it binds them all.
    let one_fixpoint = !cross.program.rules.is_empty() && session.is_none() && {
        let bounded = cross_rounds::bound(&cross.program, &split.same_doc)?;
        bounded.hops.is_empty() || cross_rounds::declared(&prog).is_none()
    };
    // The import components (decision 0076) run before the cross part, with no hooks: what they read is bound now.
    let eager: BTreeSet<&str> = cross_plan.iter().flat_map(|p| p.components.rules.iter()).flat_map(|r| r.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.as_str())).collect();
    for rel in &split.exports {
        if one_fixpoint && !eager.contains(rel.as_str()) {
            let width = lowered.program.decls[rel].len();
            let empty = crate::code_rule_dsl::logic::native::component::columns_source((0..width).map(|c| format!("c{c}")).collect(), vec![Some(Vec::new()); width]);
            modules.bind(&db, rel, empty)?;
            continue;
        }
        let source = crate::code_rule_dsl::logic::native::component::table_source(&db, "x", rel).ok_or_else(|| Error::Plan(format!("x.{rel} is not a table of integers")))?;
        cross_inputs.push((rel.clone(), Arc::clone(&source)));
        modules.bind(&db, rel, source)?;
    }
    // The cone of requests' rows and the answers, as the rest of the cross-file part reads them (decision 0065 §2a).
    if let Some((load, _)) = &loading {
        for (rel, rows) in std::mem::take(&mut loading_rows) {
            let width = load.decls[&rel].len();
            let names: Vec<String> = (0..width).map(|c| format!("c{c}")).collect();
            let columns = (0..width).map(|c| Some(rows.iter().map(|r| r[c]).collect())).collect();
            let source = crate::code_rule_dsl::logic::native::component::columns_source(names, columns);
            cross_stats.measured(&rel, syntax::statistics(source.as_ref()).into_iter().map(|(c, rows, ndv, sq, sqd)| (c, [rows, ndv, sq, sqd])), []);
            modules.bind(&db, &rel, source)?;
        }
    }
    // The chunks' freed pages go back before the cross part builds its own.
    crate::code_rule_dsl::prof::release();
    phase("stats", statements.count());
    statements.enter("cross");
    let mut rounds = cross_rounds::Rounds::default();
    // Strata a session's run evaluated, of all (decision 0077).
    let mut cross_strata: Option<(usize, usize)> = None;
    let mut cross_maintained = 0;
    let mut cross_derived_again = 0;
    let (mut cross_changed, mut cross_again) = (Vec::new(), BTreeMap::new());
    let mut cross_delta_counts = [0usize; 6];
    let mut cross_recursive = 0;
    // `returns`, held for the typing outputs (`corpus::typing`), when the cross-file part derives it: a program that
    // declares it and derives it per file (`python/typing.rr` in a rule's program) has no table of it here.
    let mut wanted: BTreeSet<String> = cross.program.decls.contains_key("returns").then(|| "returns".to_owned()).into_iter().filter(|r| !prog.outputs.contains(r)).collect();
    // The import components (decision 0076): derived first, from the exports, and each one's stored rows looked up.
    // The hits' rows are bound as `<rel>__cross` with `rr_cross_doc`; the misses are derived and stored after.
    let mut cross_state: Option<(cross_memo::Components, cross_memo::Found)> = None;
    if let Some(plan) = &cross_plan {
        let components_span = crate::code_rule_dsl::logic::profile::span(|| "import components".into());
        let held: BTreeSet<String> = cross_memo::HELD.iter().map(|r| (*r).to_owned()).collect();
        let got = super::run_atoms_native_over(&part(plan.components.clone()), &db, "main", cross_stats.clone(), held, modes)?;
        let edges: Vec<(i64, i64)> = held_rows(&db, &got, cross_memo::HELD[1], 2)?.into_iter().map(|r| (r[0], r[1])).collect();
        let mut keys: HashMap<i64, String> = HashMap::new();
        let mut rows = Vec::new();
        for r in held_rows(&db, &got, cross_memo::COMPONENT, 3)? {
            if !keys.contains_key(&r[2]) {
                let text: String = db.query_row("SELECT sym_text(?1)", [r[2]], |row| row.get(0))?;
                keys.insert(r[2], text);
            }
            rows.push((r[0], r[1], keys[&r[2]].clone()));
        }
        let outputs_of = |rel: &str| prog.outputs.iter().any(|o| o == rel);
        native.extend(gather(got, &mut outputs, &mut node_columns, &outputs_of).into_iter().map(|k| format!("components:{k}")));
        let components = cross_memo::Components::new(rows, &edges);
        let found = cross_memo::lookup(&store, plan, &components, &paths)?;
        // Every component found: a memoised relation's own rules derive nothing (each is guarded to the documents not
        // found), so only its copy from `<rel>__cross` runs, and only the copies something reads (decision 0076).
        let all_found = cross_mode == cross_memo::Mode::On && found.misses.is_empty() && !plan.memo.is_empty();
        if all_found {
            let before = cross.program.rules.len();
            cross_memo::keep_copies(&mut cross.program, plan, &wanted);
            crate::code_rule_dsl::logic::profile::decision("import components (decision 0076)", || {
                format!("every component found: {} rules of memoised relations not run, their copies only", before - cross.program.rules.len())
            });
        }
        let read: BTreeSet<&str> = cross.program.rules.iter().flat_map(|r| r.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.as_str())).collect();
        let mut replayed = Totals::default();
        let inputs = found.inputs(&store, plan, &components, cross_mode, |r| !all_found || read.contains(format!("{r}{}", cross_memo::SUFFIX).as_str()))?;
        let replayed_rows: usize = inputs.iter().skip(1).map(|(_, _, cols)| cols.first().map_or(0, Vec::len)).sum();
        for (rel, width, cols) in inputs {
            let types: Vec<String> = cross.program.decls[&rel].iter().map(|(_, t)| t.clone()).collect();
            let columns = (0..width).map(|i| format!("c{i}")).collect();
            let source = crate::code_rule_dsl::logic::native::component::columns_source(columns, cols.into_iter().map(Some).collect());
            replayed.add_rows(&rel, source.len(), source.as_ref(), &types.iter().map(String::as_str).collect::<Vec<_>>());
            modules.bind(&db, &rel, source)?;
        }
        replayed.apply(&mut cross_stats);
        // Held for storing, unless an output (written to its table, read from there): only when a component missed or
        // the run verifies. Every component found is replayed, and nothing is stored.
        if !found.misses.is_empty() || cross_mode == cross_memo::Mode::Verify {
            wanted.extend(plan.memo.keys().filter(|r| !prog.outputs.contains(*r)).cloned());
        }
        drop(components_span);
        crate::code_rule_dsl::logic::profile::decision("import components (decision 0076)", || {
            format!(
                "{} documents in {} components over {} edges; {} found ({}), {} derived",
                components.of.len(),
                found.hits.len() + found.misses.len(),
                edges.len(),
                found.hits.len(),
                if cross_mode == cross_memo::Mode::Verify { "derived again, to compare with their stored rows".to_owned() } else { format!("{replayed_rows} rows replayed") },
                found.misses.len()
            )
        });
        cross_state = Some((components, found));
    }
    // The memoised relations' derived rows, for storing (decision 0076).
    let storing = cross_state.as_ref().is_some_and(|(_, found)| !found.misses.is_empty() || cross_mode == cross_memo::Mode::Verify);
    let memo_rels: BTreeSet<String> = cross_plan.as_ref().filter(|_| storing).map(|p| p.memo.keys().cloned().collect()).unwrap_or_default();
    let mut memo_rows: BTreeMap<String, Vec<Vec<i64>>> = BTreeMap::new();
    let collect_rows = !memo_rels.is_empty();
    let mut collect = |db: &Connection, got: &Outcome, part: &front::Program| -> Result<()> {
        let derived: BTreeSet<&str> = part.rules.iter().map(|r| r.head.rel.as_str()).collect();
        for rel in derived.into_iter().filter(|r| memo_rels.contains(*r)) {
            let rows = held_rows(db, got, rel, part.decls[rel].len())?;
            memo_rows.entry(rel.to_owned()).or_default().extend(rows);
        }
        Ok(())
    };
    let mut returns = Vec::new();
    if !cross.program.rules.is_empty() {
        // RERAIL_DUMP_CROSS=<file>: the cross-file part's rules, one per line (a debugging aid).
        if let Ok(path) = std::env::var("RERAIL_DUMP_CROSS") {
            let _ = std::fs::write(path, cross.program.rules.iter().map(|r| format!("{r}\n")).collect::<String>());
        }
        let bounded = cross_rounds::bound(&cross.program, &split.same_doc)?;
        let outputs_of = |rel: &str| prog.outputs.iter().any(|o| o == rel);
        // Bounded rounds only for a program that declares them (decision 0056: a typing program); a rule's
        // cross-file part is one fixpoint, which no chain length cuts.
        let declared = cross_rounds::declared(&prog);
        if !bounded.hops.is_empty() && declared.is_none() {
            crate::code_rule_dsl::logic::profile::decision("cross-file rounds (typing::cross_rounds)", || {
                format!("none: the program declares no `.pragma \"{}\"`, so its {} hop relations are one fixpoint", cross_rounds::PRAGMA, bounded.hops.len())
            });
        }
        let k = declared.map(cross_rounds::k);
        if bounded.hops.is_empty() || k.is_none() {
            rounds = cross_rounds::Rounds { run: 1, converged: true, ..Default::default() };
            match session.as_ref().map(|s| &s.cross) {
                // A session's run evaluates the strata whose inputs changed, the others' rows kept (decision 0077).
                Some(cell) => {
                    let digests: HashMap<String, u64> = cross_inputs.iter().map(|(rel, source)| (rel.clone(), retain::digest(source.as_ref()))).collect();
                    let ckey = retain::key(&prepared_key, &cross.program);
                    let mut cell = cell.lock().expect("the session's cross-file part");
                    let cut = retain::cut(cell.as_ref(), ckey, &cross.program, &digests)?;
                    // The O125 spike's progress: each step of a session's run, with this thread's CPU.
                    let trace = std::env::var_os("RERAIL_SPIKE_DELTA").is_some();
                    let step = |what: &str| {
                        if trace {
                            let cpu = { let (u, s) = thread_usage(); u + s };
                            eprintln!("O125 step {what} at {:.1} s CPU", cpu);
                        }
                    };
                    step(&format!("cut: {} of {} strata to run; inputs changed: {}", cut.strata.0, cut.strata.1, cut.changed.len()));
                    if std::env::var_os("RERAIL_CROSS_VERIFY").is_some() {
                        let read: BTreeSet<&String> = cross.program.rules.iter().flat_map(|r| &r.body).filter_map(|l| l.atom()).map(|a| &a.rel).collect();
                        let undigested: Vec<&String> = cross.program.inputs.iter().filter(|r| !digests.contains_key(*r) && read.contains(r)).collect();
                        eprintln!("O125_VERIFY inputs read but not digested: {undigested:?}");
                    }
                    // Changed strata maintain their rows from deltas where they can (stage 2, `maintain`).
                    let maintained = maintain::rewrite(&cut, cell.as_ref(), &cross)?;
                    let part = &maintained.program;
                    step(&format!("rewrite: {} maintained ({} recursive), {} derived again, {} rules, {} old sources", maintained.maintained, maintained.recursive, maintained.derived_again, part.program.rules.len(), maintained.sources.len()));
                    let mut stats = cross_stats;
                    let clean = cell.as_ref().map(|kept| cut.sources(kept, &part.program)).unwrap_or_default();
                    for (rel, source) in clean.into_iter().chain(maintained.sources.iter().cloned()) {
                        let measured = syntax::statistics(source.as_ref());
                        // The constants the rules test it by are counted too: a column measured without them reads as
                        // holding none, so `kind__old(r, identifier)` was planned as one row and drove a delta rule.
                        let counts = constant_counts(&part.program, &rel, source.as_ref());
                        stats.measured(&rel, measured.iter().map(|(c, rows, ndv, sq, sqd)| (c.clone(), [*rows, *ndv, *sq, *sqd])), counts);
                        modules.bind(&db, &rel, source)?;
                    }
                    let derived = cut.derived(&cross.program)?;
                    let mut keep = wanted.clone();
                    // An output is written to its table, where the write phase reads it, and read back from there.
                    keep.extend(derived.iter().filter(|rel| !part.program.outputs.contains(*rel)).cloned());
                    // The O125 spike (`RERAIL_SPIKE_DELTA`): each maintained relation's overdeleted and inserted rows.
                    let spike = std::env::var_os("RERAIL_SPIKE_DELTA").is_some();
                    let probes: Vec<String> = if spike { part.program.decls.keys().filter(|r| r.ends_with("__del") || r.ends_with("__ins")).cloned().collect() } else { Vec::new() };
                    keep.extend(probes.iter().cloned());
                    step("bound; running");
                    if std::env::var_os("RERAIL_CROSS_DELTA_COUNTS").is_some() || std::env::var_os("RERAIL_CROSS_VERIFY").is_some() {
                        keep.extend(part.program.decls.keys().filter(|r| ["__del", "__ins", "__minus", "__plus"].iter().any(|x| r.ends_with(x))).cloned());
                    }
                    // `RERAIL_CROSS_DEMAND_SPLIT`: keep the demand split in a session (to measure what dropping it costs).
                    let _whole = std::env::var_os("RERAIL_CROSS_DEMAND_SPLIT").is_none().then(super::demand::whole_relations);
                    let verify_stats = std::env::var_os("RERAIL_CROSS_VERIFY").map(|_| stats.clone());
                    let got = super::run_atoms_native_over(part, &db, "main", stats, keep, modes)?;
                    step("ran");
                    collect(&db, &got, &part.program)?;
                    step("collected");
                    if spike {
                        for (rel, why) in &maintained.again {
                            eprintln!("O125 derived-again {rel}: {why}");
                        }
                        let (mut tdel, mut tins, mut tgone, mut tnew) = (0usize, 0usize, 0usize, 0usize);
                        let mut lines = Vec::new();
                        for p in probes.iter().filter(|p| p.ends_with("__del")) {
                            let rel = &p[..p.len() - 5];
                            let width = part.program.decls[p].len();
                            let del = if got.kept.contains(p.as_str()) { held_rows(&db, &got, p, width)?.len() } else { 0 };
                            let ins_name = format!("{rel}__ins");
                            let ins = if got.kept.contains(ins_name.as_str()) { held_rows(&db, &got, &ins_name, width)?.len() } else { 0 };
                            let new: std::collections::HashSet<Vec<i64>> = if got.kept.contains(rel) { held_rows(&db, &got, rel, width)?.into_iter().collect() } else { Default::default() };
                            let old: std::collections::HashSet<Vec<i64>> = cell
                                .as_ref()
                                .and_then(|k| k.old(rel))
                                .map(|cols| (0..cols.first().map_or(0, Vec::len)).map(|i| cols.iter().map(|c| c[i]).collect()).collect())
                                .unwrap_or_default();
                            let gone = old.difference(&new).count();
                            let fresh = new.difference(&old).count();
                            tdel += del;
                            tins += ins;
                            tgone += gone;
                            tnew += fresh;
                            lines.push((del, format!("{rel}: old {} new {} overdeleted {del} (gone {gone}, so {} put back) inserted {ins} (new {fresh})", old.len(), new.len(), del.saturating_sub(gone))));
                        }
                        lines.sort_by(|a, b| b.0.cmp(&a.0));
                        for (_, l) in lines.iter().take(25) {
                            eprintln!("O125 {l}");
                        }
                        eprintln!("O125 total: overdeleted {tdel}, gone {tgone}, put back {}, inserted {tins}, new {tnew}; strata {} of {} run, {} maintained ({} recursive), {} derived again", tdel.saturating_sub(tgone), cut.strata.0, cut.strata.1, maintained.maintained, maintained.recursive, maintained.derived_again);
                    }
                    if wanted.contains("returns") && got.kept.contains("returns") {
                        returns = held_rows(&db, &got, "returns", 2)?;
                    }
                    // A relation of no columns is a flag its rows' count holds, which columns do not: derived again.
                    let rows = derived
                        .iter()
                        .map(|rel| (rel, part.program.decls[rel].len()))
                        .filter(|(_, width)| *width > 0)
                        .map(|(rel, width)| Ok((rel.clone(), width, held_rows(&db, &got, rel, width)?)))
                        .collect::<Result<Vec<_>>>()?;
                    // `RERAIL_CROSS_DELTA_COUNTS`: the rows the maintained strata overdeleted (`__del`), inserted (`__ins`),
                    // and hold after the run, for measuring delete-and-rederive (spike O125).
                    if std::env::var_os("RERAIL_CROSS_DELTA_COUNTS").is_some() {
                        // [over-deleted, inserted by the insert rules, held after, input and derived-again deltas
                        // (`__minus` + `__plus`), truly deleted, truly inserted].
                        let mut counts = [0usize; 6];
                        let olds: HashMap<&str, usize> = maintained.sources.iter().map(|(rel, s)| (rel.as_str(), s.len())).collect();
                        let derived_here: BTreeSet<&String> = part.program.rules.iter().map(|r| &r.head.rel).collect();
                        let rows_of = |rel: &str| held_rows(&db, &got, rel, part.program.decls[rel].len()).unwrap_or_default();
                        for (rel, cols) in &part.program.decls {
                            if !derived_here.contains(rel) || cols.is_empty() {
                                continue;
                            }
                            if rel.ends_with("__minus") || rel.ends_with("__plus") {
                                counts[3] += rows_of(rel).len();
                            } else if rel.ends_with("__ins") {
                                counts[1] += rows_of(rel).len();
                            } else if !rel.ends_with("__del") && part.program.decls.contains_key(&format!("{rel}__del")) {
                                let after: std::collections::HashSet<Vec<i64>> = rows_of(rel).into_iter().collect();
                                let del = rows_of(&format!("{rel}__del"));
                                let gone = del.iter().filter(|r| !after.contains(*r)).count();
                                let old = olds.get(format!("{rel}__old").as_str()).copied().unwrap_or(0);
                                counts[0] += del.len();
                                counts[2] += after.len();
                                counts[4] += gone;
                                counts[5] += (after.len() + gone).saturating_sub(old);
                            }
                        }
                        cross_delta_counts = counts;
                    }
                    let verify_deltas: HashMap<String, std::collections::HashSet<Vec<i64>>> = if std::env::var_os("RERAIL_CROSS_VERIFY").is_some() {
                        part.program
                            .decls
                            .iter()
                            .filter(|(r, c)| (r.ends_with("__del") || r.ends_with("__ins")) && !c.is_empty() && part.program.rules.iter().any(|x| &x.head.rel == *r))
                            .map(|(r, c)| (r.clone(), held_rows(&db, &got, r, c.len()).unwrap_or_default().into_iter().collect()))
                            .collect()
                    } else {
                        HashMap::new()
                    };
                    let derived_outputs: BTreeMap<String, (Vec<String>, Vec<usize>)> = got.outputs.iter().map(|(rel, lines)| (rel.clone(), (lines.clone(), got.nodes[rel].clone()))).collect();
                    let clean_outputs = cell.as_ref().map(|kept| cut.outputs(kept, &cross.program.outputs)).unwrap_or_default();
                    native.extend(gather(got, &mut outputs, &mut node_columns, &outputs_of).into_iter().map(|k| format!("cross:{k}")));
                    for (rel, lines, nodes) in clean_outputs {
                        if outputs_of(&rel) {
                            outputs.entry(rel.clone()).or_default().extend(lines);
                            node_columns.insert(rel, nodes);
                        }
                    }
                    // `RERAIL_CROSS_VERIFY`: derive the run's strata again (the cut's program, no maintenance) and name the
                    // first relations, in stratum order, whose maintained rows differ (spike O125).
                    if let Some(vstats) = verify_stats.filter(|_| maintained.maintained > 0) {
                        let plain = cut.program(&cross);
                        let mut vkeep = wanted.clone();
                        vkeep.extend(derived.iter().filter(|rel| !plain.program.outputs.contains(*rel)).cloned());
                        let again = super::run_atoms_native_over(&plain, &db, "main", vstats, vkeep, modes)?;
                        let mut shown = 0;
                        for (rel, width, kept_rows) in &rows {
                            let want: std::collections::HashSet<Vec<i64>> = held_rows(&db, &again, rel, *width)?.into_iter().collect();
                            let have: std::collections::HashSet<Vec<i64>> = kept_rows.iter().cloned().collect();
                            if want != have && shown < 6 {
                                shown += 1;
                                let delta = |name: String| -> std::collections::HashSet<Vec<i64>> { verify_deltas.get(&name).cloned().unwrap_or_default() };
                                let old_rows: std::collections::HashSet<Vec<i64>> = maintained
                                    .sources
                                    .iter()
                                    .find(|(r, _)| *r == format!("{rel}__old"))
                                    .map(|(_, src)| (0..src.len()).map(|i| (0..*width).map(|c| src.value(i, c)).collect()).collect())
                                    .unwrap_or_default();
                                let (del, ins) = (delta(format!("{rel}__del")), delta(format!("{rel}__ins")));
                                for row in have.difference(&want).take(3) {
                                    eprintln!("O125_VERIFY   {rel}{row:?}: in old {}, in del {}, in ins {}", old_rows.contains(row), del.contains(row), ins.contains(row));
                                }
                                eprintln!(
                                    "O125_VERIFY {rel}: {} extra, {} missing; e.g. extra {:?}, missing {:?}",
                                    have.difference(&want).count(),
                                    want.difference(&have).count(),
                                    have.difference(&want).next(),
                                    want.difference(&have).next()
                                );
                            }
                        }
                        if let Ok(v) = std::env::var("O125_TRACE") {
                            let vs: Vec<i64> = v.split(',').map(|x| x.parse().unwrap()).collect();
                            let hit = |row: &Vec<i64>| vs.iter().all(|v| row.contains(v));
                            for (rel, width, kept_rows) in &rows {
                                let fresh = held_rows(&db, &again, rel, *width)?;
                                let olds = maintained.sources.iter().find(|(r, _)| *r == format!("{rel}__old")).map(|(_, src)| (0..src.len()).map(|i| (0..*width).map(|c| src.value(i, c)).collect::<Vec<i64>>()).filter(|r| hit(r)).collect::<Vec<_>>()).unwrap_or_default();
                                let (a, b): (Vec<_>, Vec<_>) = (kept_rows.iter().filter(|r| hit(r)).collect(), fresh.iter().filter(|r| hit(r)).collect());
                                let del: Vec<_> = verify_deltas.get(&format!("{rel}__del")).map(|d| d.iter().filter(|r| hit(r)).cloned().collect()).unwrap_or_default();
                                if !a.is_empty() || !b.is_empty() || !olds.is_empty() {
                                    eprintln!("O125_TRACE {rel}: old {:?} del {:?} maintained {:?} fresh {:?}", &olds[..olds.len().min(4)], &del[..del.len().min(4)], &a[..a.len().min(4)], &b[..b.len().min(4)]);
                                }
                            }
                            for (rel, src) in &cross_inputs {
                                let w = src.columns().len();
                                let now: Vec<Vec<i64>> = (0..src.len()).map(|i| (0..w).map(|c| src.value(i, c)).collect::<Vec<i64>>()).filter(|r| hit(r)).take(4).collect();
                                let olds = maintained.sources.iter().find(|(r, _)| *r == format!("{rel}__old")).map(|(_, src)| (0..src.len()).map(|i| (0..w).map(|c| src.value(i, c)).collect::<Vec<i64>>()).filter(|r| hit(r)).take(4).collect::<Vec<_>>());
                                if !now.is_empty() || olds.as_ref().is_some_and(|o| !o.is_empty()) {
                                    eprintln!("O125_TRACE input {rel}: now {now:?} old {olds:?}");
                                }
                            }
                        }
                        eprintln!("O125_VERIFY done: {shown} relations differ (first 6)");
                    }
                    let next = retain::after(cell.take(), ckey, digests, &cut, rows, &derived_outputs, &cross_inputs);
                    crate::code_rule_dsl::logic::profile::decision("cross-file part kept (decision 0077)", || {
                        format!(
                            "{} of {} strata run ({} maintained from deltas, {} of them recursive; {} derived again); inputs changed: {}; {} relations kept for the next run, {:.1} MB",
                            cut.strata.0,
                            cut.strata.1,
                            maintained.maintained,
                            maintained.recursive,
                            maintained.derived_again,
                            if cut.changed.is_empty() { "none".to_owned() } else { cut.changed.iter().take(8).cloned().collect::<Vec<_>>().join(", ") },
                            next.relations(),
                            next.bytes() as f64 / 1e6
                        )
                    });
                    cross_strata = Some(cut.strata);
                    cross_maintained = maintained.maintained;
                    cross_recursive = maintained.recursive;
                    cross_derived_again = maintained.derived_again;
                    cross_changed = cut.changed.clone();
                    cross_again = maintained.again.iter().fold(BTreeMap::new(), |mut m, (_, why)| {
                        *m.entry(*why).or_insert(0) += 1;
                        m
                    });
                    *cell = Some(next);
                }
                None => {
                    let exports: BTreeSet<&String> = split.exports.iter().collect();
                    let mut load = |rels: &[String]| -> Result<()> {
                        for rel in rels.iter().filter(|r| exports.contains(r)) {
                            let source = crate::code_rule_dsl::logic::native::component::table_source(&db, "x", rel).ok_or_else(|| Error::Plan(format!("x.{rel} is not a table of integers")))?;
                            modules.bind(&db, rel, source)?;
                        }
                        Ok(())
                    };
                    // `node` and `doc` decode the findings once the run ends.
                    let mut unload = |rels: &[String]| -> Result<()> {
                        for rel in rels.iter().filter(|r| !matches!(r.as_str(), "node" | "doc")) {
                            if let Some(b) = modules.get(rel) {
                                b.clear();
                            }
                        }
                        Ok(())
                    };
                    let hooks = if one_fixpoint { super::Hooks { load: Some(&mut load), unload: Some(&mut unload), ..Default::default() } } else { super::Hooks::default() };
                    let got = super::run_atoms_native_hooked(&cross, &db, "main", cross_stats, wanted.clone(), modes, hooks)?;
                    collect(&db, &got, &cross.program)?;
                    if wanted.contains("returns") && got.kept.contains("returns") {
                        returns = held_rows(&db, &got, "returns", 2)?;
                    }
                    native.extend(gather(got, &mut outputs, &mut node_columns, &outputs_of).into_iter().map(|k| format!("cross:{k}")));
                }
            }
        } else if let Some(cone) = cross_rounds::cone_split(&bounded.program, &bounded.hops) {
            // The rounds run the hops' cone; what reads it and no hop reads runs once, after the last round.
            // Held: what the rest reads, unless an output (written to its table, read from there).
            let mut held = wanted.clone();
            held.extend(cone.reads.iter().filter(|r| !outputs_of(r)).cloned());
            // What the rounds read and no round changes runs once, before them (`cross_rounds::pre_split`): its rows the
            // rounds read are inputs `<rel>__pre`, and the rest reads its rows from here.
            let mut round_stats = cross_stats.clone();
            let mut pre_rows: BTreeMap<String, Vec<Vec<i64>>> = BTreeMap::new();
            let round_prog = match cross_rounds::pre_split(&cone.cone) {
                Some(ps) => {
                    let pre_span = crate::code_rule_dsl::logic::profile::span(|| "before the rounds".into());
                    let pre_derived: BTreeSet<String> = ps.pre.rules.iter().map(|r| r.head.rel.clone()).collect();
                    let copied: BTreeSet<String> = ps.reads.iter().chain(cone.reads.iter().filter(|r| pre_derived.contains(*r))).cloned().collect();
                    let mut pre_held: BTreeSet<String> = held.iter().filter(|r| pre_derived.contains(*r)).cloned().collect();
                    pre_held.extend(copied.iter().cloned());
                    let decls = ps.pre.decls.clone();
                    let got = super::run_atoms_native_over(&part(ps.pre), &db, "main", cross_stats.clone(), pre_held, modes)?;
                    if wanted.contains("returns") && pre_derived.contains("returns") {
                        returns = held_rows(&db, &got, "returns", 2)?;
                    }
                    for rel in &copied {
                        pre_rows.insert(rel.clone(), held_rows(&db, &got, rel, decls[rel].len())?);
                    }
                    native.extend(gather(got, &mut outputs, &mut node_columns, &outputs_of).into_iter().map(|k| format!("cross:{k}")));
                    let mut pre = Totals::default();
                    for rel in &ps.reads {
                        let types: Vec<&str> = decls[rel].iter().map(|(_, t)| t.as_str()).collect();
                        let rows = &pre_rows[rel];
                        let columns = (0..types.len()).map(|i| format!("c{i}")).collect();
                        let values = (0..types.len()).map(|c| Some(rows.iter().map(|r| r[c]).collect())).collect();
                        let source = crate::code_rule_dsl::logic::native::component::columns_source(columns, values);
                        let name = format!("{rel}{}", cross_rounds::PRE);
                        pre.add_rows(&name, source.len(), source.as_ref(), &types);
                        modules.bind(&db, &name, source)?;
                    }
                    pre.apply(&mut round_stats);
                    crate::code_rule_dsl::logic::profile::decision("cross-file rounds (typing::cross_rounds)", || {
                        format!("{} relations derived once before the rounds, {} of them read by the rounds", pre_derived.len(), ps.reads.len())
                    });
                    drop(pre_span);
                    ps.rounds
                }
                None => cone.cone,
            };
            let cone_rules = if collect_rows { round_prog.clone() } else { front::Program::default() };
            let got = run_rounds(&db, &modules, part(round_prog), &bounded.hops, &held, &round_stats, modes, k.expect("declared"), &mut rounds)?;
            collect(&db, &got, &cone_rules)?;
            if wanted.contains("returns") && got.kept.contains("returns") {
                returns = held_rows(&db, &got, "returns", 2)?;
            }
            // The cone's final rows the rest reads, as `<rel>__done` inputs held in memory and measured.
            let mut rest = cone.rest;
            let done_span = crate::code_rule_dsl::logic::profile::span(|| "done inputs".into());
            let mut st = cross_stats.clone();
            let mut done = Totals::default();
            for rel in &cone.reads {
                let types: Vec<&str> = rest.decls[&format!("{rel}{}", cross_rounds::DONE)].iter().map(|(_, t)| t.as_str()).collect();
                let rows = match pre_rows.remove(rel) {
                    Some(rows) => rows,
                    None => held_rows(&db, &got, rel, types.len())?,
                };
                let columns = (0..types.len()).map(|i| format!("c{i}")).collect();
                let values = (0..types.len()).map(|c| Some(rows.iter().map(|r| r[c]).collect())).collect();
                let source = crate::code_rule_dsl::logic::native::component::columns_source(columns, values);
                let name = format!("{rel}{}", cross_rounds::DONE);
                done.add_rows(&name, source.len(), source.as_ref(), &types);
                modules.bind(&db, &name, source)?;
            }
            done.apply(&mut st);
            drop(done_span);
            native.extend(gather(got, &mut outputs, &mut node_columns, &outputs_of).into_iter().map(|k| format!("cross:{k}")));
            // A pending round's marks hold for the rest as for the cone.
            if rounds.pending && let Some(f) = rest.facts.get_mut(cross_rounds::UNCONVERGED) {
                f.push(vec![front::Value::Int(1)]);
            }
            crate::code_rule_dsl::logic::native::forget_inputs(&db);
            let rest_wanted: BTreeSet<String> = wanted.iter().filter(|w| rest.rules.iter().any(|r| r.head.rel == **w)).cloned().collect();
            let rest = part(rest);
            let got = super::run_atoms_native_over(&rest, &db, "main", st, rest_wanted.clone(), modes)?;
            collect(&db, &got, &rest.program)?;
            if rest_wanted.contains("returns") && got.kept.contains("returns") {
                returns = held_rows(&db, &got, "returns", 2)?;
            }
            native.extend(gather(got, &mut outputs, &mut node_columns, &outputs_of).into_iter().map(|k| format!("cross:{k}")));
        } else {
            let all_rules = if collect_rows { bounded.program.clone() } else { front::Program::default() };
            let got = run_rounds(&db, &modules, part(bounded.program), &bounded.hops, &wanted, &cross_stats, modes, k.expect("declared"), &mut rounds)?;
            collect(&db, &got, &all_rules)?;
            if wanted.contains("returns") && got.kept.contains("returns") {
                returns = held_rows(&db, &got, "returns", 2)?;
            }
            native.extend(gather(got, &mut outputs, &mut node_columns, &outputs_of).into_iter().map(|k| format!("cross:{k}")));
        }
        if !bounded.hops.is_empty() && k.is_some() {
            crate::code_rule_dsl::logic::profile::decision("cross-file rounds (typing::cross_rounds)", || {
                format!(
                    "{} hop relations ({}); {} rounds run, {}; hop rows per round {:?}",
                    bounded.hops.len(),
                    bounded.hops.iter().take(8).cloned().collect::<Vec<_>>().join(", "),
                    rounds.run,
                    if rounds.converged { "converged" } else { "NOT converged: chains crossing more files are cut" },
                    rounds.rows
                )
            });
        }
    }
    phase("cross", statements.count());
    let mut cross_counts = (0, 0);
    if let (Some(plan), Some((components, found))) = (&cross_plan, &cross_state) {
        cross_counts = (found.hits.len(), found.misses.len());
        let _storing = crate::code_rule_dsl::logic::profile::span(|| "store import components' rows".into());
        let nodes: BTreeMap<String, Vec<usize>> = plan
            .memo
            .keys()
            .map(|rel| (rel.clone(), cross.program.decls[rel].iter().enumerate().filter(|(_, (_, t))| t == "node").map(|(i, _)| i).collect()))
            .collect();
        let recorded = cross_memo::record(&store, plan, components, found, cross_mode, &memo_rows, &nodes, &paths)?;
        crate::code_rule_dsl::logic::profile::decision("import components stored (decision 0076)", || {
            format!("{} components' rows stored ({} rows); {} found components verified", recorded.stored, recorded.rows, recorded.verified)
        });
        run_sources.extend(found.sources.iter().copied().chain(recorded.sources.iter().copied()));
    }
    if let Some((relations, reading)) = publish_later {
        collecting = record_run(store, root, &run_sources, &memo_program, &program_plans, &relations, reading)?;
    }
    statements.enter("write");
    // Every output has its file, empty when nothing made a row: a run read whole from the memo hands no rows of a
    // file-local output no file made (`rr_embed_req`), which an evaluated run writes empty (O109).
    for rel in &prog.outputs {
        if !outputs.contains_key(rel) && let Some(cols) = prog.decls.get(rel) {
            outputs.insert(rel.clone(), Vec::new());
            node_columns.entry(rel.clone()).or_insert_with(|| cols.iter().enumerate().filter(|(_, (_, t))| t == "node").map(|(i, _)| i).collect());
        }
    }
    let mut rows = BTreeMap::new();
    // An output with no rows is written empty: a run whose files all came from the memo replays only the lines
    // there are.
    for rel in &prog.outputs {
        outputs.entry(rel.clone()).or_default();
        node_columns.entry(rel.clone()).or_default();
    }
    for (rel, lines) in &outputs {
        let node_columns = &node_columns[rel];
        let mut decoded: Vec<String> = lines
            .iter()
            // A row about a loaded document (its first node is one) is not the corpus's; a corpus row may name a
            // loaded document's node (a call's target in a stub).
            .filter(|row| loaded_keys.is_empty() || node_columns.first().is_none_or(|c| !row.split('\t').nth(*c).and_then(|v| v.parse::<i64>().ok()).is_some_and(|v| loaded_keys.contains(&(v >> 32)))))
            .map(|row| {
                row.split('\t')
                    .enumerate()
                    .map(|(i, v)| match v.parse::<i64>() {
                        // A guest's node (decision 0055): its document is no file of the run.
                        Ok(v) if node_columns.contains(&i) && !paths.contains_key(&(v >> 32)) && !first.contains_key(&(v >> 32)) => format!("guest{}:{}", v >> 32, v & 0xffff_ffff),
                        Ok(v) if node_columns.contains(&i) && (own || syntax::definition_ids()) => format!("{}:{}", paths[&(v >> 32)], syntax::shown_local(v & 0xffff_ffff)),
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
        if !decoded.is_empty() {
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
    // The run's atoms are written at its end (decision 0049 §4.3), and the profile says how often they were.
    store.flush_atoms().map_err(|e| Error::Plan(e.to_string()))?;
    let ((segments, atoms, bytes), pending) = store.atom_writes();
    crate::code_rule_dsl::logic::profile::decision("atoms", || {
        format!("{atoms} atoms appended in {segments} segments ({:.1} MB), {pending} pending at the end (decision 0049)", bytes as f64 / 1e6)
    });
    if memoised {
        crate::code_rule_dsl::logic::profile::decision("per-file memo use", || format!("{memo_hits} of {} files read from the memo; {memo_unattributed} chunks' rows not memoised (a row naming no document of its chunk){}", docs.len(), if memo_why.is_empty() { String::new() } else { format!(": {}", memo_why.iter().take(12).cloned().collect::<Vec<_>>().join("; ")) }));
    }
    if let Some(collecting) = collecting {
        let collection = crate::code_rule_dsl::logic::profile::Activity::default();
        collection.set("collecting the store (decision 0053)");
        let (passes, outcome) = crate::code_rule_dsl::logic::profile::wait("the store's collection", &collection, || collecting.join()).map_err(|_| Error::Plan("store collection panicked".to_owned()))?;
        crate::code_rule_dsl::logic::profile::decision("store collection (decision 0053)", || match outcome {
            Ok(summary) => format!("{passes} passes in the background: {summary}"),
            Err(e) => format!("{passes} passes, then stopped: {e}"),
        });
    }
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
        let lines: usize = programs.iter().map(|p| p.lines().filter(|l| !l.trim().is_empty()).count()).sum();
        header.push_str(&format!("\nProgram: {} lines in {} programs; rules are numbered as in the `rule #k` notes at the end.\n", lines, programs.len()));
        crate::code_rule_dsl::logic::profile::write(&dir, &header).map_err(|e| Error::Plan(format!("profile: {e}")))?;
    }
    Ok(Report { phases, files: docs.len(), nodes, native: native.len(), rows, chunks, rounds, typing: typed, text: (vocabulary.tests.len(), tested, text_cpu), typing_runs, phase_runs, memo: (memo_hits, memo_unattributed), cross: cross_counts, prepared: origin, cross_strata, cross_maintained: (cross_maintained, cross_recursive, cross_derived_again), cross_changed, cross_again, cross_delta_counts, consulted, files_observed: Default::default() })
}



/// The cross-file part's bounded rounds (`typing::cross_rounds`): `round` is the part with its hops reading
/// `<rel>__prev`, the previous round's rows held in memory (none in round 0). Each round's rows of the
/// hop relations are digested; an unchanged digest ends the rounds (the fixpoint), as does round `K`. The last
/// round's outcome is the part's.
/// A relation's rows after a run: held by the engine (`kept`), or in its table.
/// How many of `source`'s rows hold each integer constant a rule of `prog` tests `rel`'s columns by, by column name
/// (the planner's `const_freq`).
fn constant_counts(prog: &front::Program, rel: &str, source: &dyn crate::code_rule_dsl::facts::vtab::Source) -> Vec<(String, i64, i64)> {
    let mut wanted: BTreeMap<(usize, i64), i64> = BTreeMap::new();
    for a in prog.rules.iter().flat_map(|r| &r.body).filter_map(front::Literal::atom).filter(|a| a.rel == rel) {
        for (c, t) in a.args.iter().enumerate() {
            if let front::Term::Int(v) = t {
                wanted.insert((c, *v), 0);
            }
        }
    }
    if wanted.is_empty() {
        return Vec::new();
    }
    let names = source.columns();
    let by_column: BTreeSet<usize> = wanted.keys().map(|(c, _)| *c).filter(|c| *c < names.len()).collect();
    for c in by_column {
        let mut counts: HashMap<i64, i64> = HashMap::new();
        for row in 0..source.len() {
            *counts.entry(source.value(row, c)).or_default() += 1;
        }
        for ((_, v), n) in wanted.range_mut((c, i64::MIN)..=(c, i64::MAX)) {
            *n = counts.get(v).copied().unwrap_or(0);
        }
    }
    wanted.into_iter().filter(|((c, _), _)| *c < names.len()).map(|((c, v), n)| (names[c].clone(), v, n)).collect()
}

fn held_rows(db: &Connection, got: &Outcome, rel: &str, width: usize) -> Result<Vec<Vec<i64>>> {
    use crate::code_rule_dsl::logic::native::component;
    let source: Arc<dyn crate::code_rule_dsl::facts::vtab::Source> = if got.kept.contains(rel) {
        component::derived(db, rel).ok_or_else(|| Error::Plan(format!("{rel} is not held")))?
    } else {
        Arc::new(Rows::read(db, &crate::code_rule_dsl::logic::sql::ident(rel))?)
    };
    Ok((0..source.len()).map(|i| (0..width).map(|c| source.value(i, c)).collect()).collect())
}

#[allow(clippy::too_many_arguments)]
fn run_rounds(db: &Connection, modules: &Modules, mut round: lower::Lowered, hops: &BTreeSet<String>, wanted: &BTreeSet<String>, stats: &super::Stats, modes: &str, k: usize, report: &mut cross_rounds::Rounds) -> Result<Outcome> {
    // A program that declares `unconverged` (O41) gets one more round when round `k` ends unconverged, with the
    // fact `unconverged(1)`: its rules mark what that round still cannot decide as unknown.
    let pending_round = round.program.facts.contains_key(cross_rounds::UNCONVERGED);
    // Each round rebinds only the `__prev` inputs (and the pending round adds a fact): the other inputs stay loaded.
    let changing: BTreeSet<String> = hops.iter().map(|rel| format!("{rel}{}", cross_rounds::PREV)).chain([cross_rounds::UNCONVERGED.to_owned()]).collect();
    crate::code_rule_dsl::logic::native::keep_inputs(db, Some(changing));
    let out = rounds_loop(db, modules, &mut round, hops, wanted, stats, modes, report, k, pending_round);
    crate::code_rule_dsl::logic::native::keep_inputs(db, None);
    crate::code_rule_dsl::logic::native::forget_inputs(db);
    out
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
fn rounds_loop(db: &Connection, modules: &Modules, round: &mut lower::Lowered, hops: &BTreeSet<String>, wanted: &BTreeSet<String>, stats: &super::Stats, modes: &str, report: &mut cross_rounds::Rounds, k: usize, pending_round: bool) -> Result<Outcome> {
    use crate::code_rule_dsl::logic::native::component;
    let types = |rel: &str| -> Vec<String> { round.program.decls[rel].iter().map(|(_, t)| t.clone()).collect() };
    let mut prev_rows: BTreeMap<String, Vec<Vec<i64>>> = hops.iter().map(|rel| (rel.clone(), Vec::new())).collect();
    let mut prev_totals = Totals::default();
    for rel in hops {
        let (t, width) = (types(rel), types(rel).len());
        let empty = Rows { columns: (0..width).map(|i| format!("c{i}")).collect(), values: vec![Vec::new(); width] };
        prev_totals.add_rows(&format!("{rel}{}", cross_rounds::PREV), 0, &empty, &t.iter().map(String::as_str).collect::<Vec<_>>());
    }
    let mut last = None;
    let mut measured_lens: BTreeMap<String, usize> = BTreeMap::new();
    for r in 0..=k + 1 {
        let _round = crate::code_rule_dsl::logic::profile::span(|| format!("round {r}"));
        let bind = crate::code_rule_dsl::logic::profile::span(|| "bind previous rows".into());
        for (rel, rows) in &prev_rows {
            let width = types(rel).len();
            let columns = (0..width).map(|i| format!("c{i}")).collect();
            let values = (0..width).map(|c| Some(rows.iter().map(|r| r[c]).collect())).collect();
            modules.bind(db, &format!("{rel}{}", cross_rounds::PREV), component::columns_source(columns, values))?;
        }
        drop(bind);
        let mut st = stats.clone();
        prev_totals.apply(&mut st);
        // A hop relation the program outputs is written to its table, to be decoded; the others are held.
        let held: BTreeSet<String> = hops.iter().filter(|r| !round.program.outputs.contains(r)).chain(wanted).cloned().collect();
        let got = super::run_atoms_native_over(round, db, "main", st, held, modes)?;
        // This round's rows of the hop relations: from the engine, or their table.
        let measure = crate::code_rule_dsl::logic::profile::span(|| "hop rows: copy and measure".into());
        let mut rows: BTreeMap<String, Vec<Vec<i64>>> = BTreeMap::new();
        let mut sources: Vec<(&String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)> = Vec::with_capacity(hops.len());
        for rel in hops {
            let width = types(rel).len();
            let source: Arc<dyn crate::code_rule_dsl::facts::vtab::Source> = if got.kept.contains(rel) {
                component::derived(db, rel).ok_or_else(|| Error::Plan(format!("{rel} is not held")))?
            } else {
                Arc::new(Rows::read(db, &crate::code_rule_dsl::logic::sql::ident(rel))?)
            };
            // Sorted and distinct once: the digest and the next round's `__prev` read them so.
            let mut held: Vec<Vec<i64>> = (0..source.len()).map(|i| (0..width).map(|c| source.value(i, c)).collect()).collect();
            held.sort_unstable();
            held.dedup();
            rows.insert(rel.clone(), held);
            sources.push((rel, source));
        }
        // The next round's statistics of `<rel>__prev`: a relation is measured again only when its rows moved by more
        // than a tenth since it was last measured (a later round adds a few percent; each measurement sorts columns).
        {
            let _m = crate::code_rule_dsl::logic::profile::span(|| "measure".into());
            for (rel, source) in &sources {
                if measured_lens.get(*rel).is_some_and(|&m| source.len().abs_diff(m) * 10 <= m) {
                    continue;
                }
                let prev = format!("{rel}{}", cross_rounds::PREV);
                prev_totals.forget(&prev);
                prev_totals.forget(rel);
                let t = types(rel);
                let t: Vec<&str> = t.iter().map(String::as_str).collect();
                prev_totals.add_rows(&prev, source.len(), source.as_ref(), &t);
                // The next round prices the relation itself by these rows too, not by its seeds (`plan::prior_stats`).
                prev_totals.alias(&prev, rel);
                measured_lens.insert((*rel).clone(), source.len());
            }
        }
        drop(sources);
        drop(measure);
        let digest = crate::code_rule_dsl::logic::profile::span(|| "digest".into());
        let digest = { let _d = digest; cross_rounds::digest_sorted(&rows) };
        report.run = r + 1;
        report.rows.push(rows.values().map(Vec::len).sum());
        crate::code_rule_dsl::logic::profile::decision(&format!("round {r} hop rows"), || {
            // Documents whose rows of some hop relation changed since the round before (by the head's first
            // node column; a relation without one is not counted).
            let mut changed: BTreeSet<i64> = BTreeSet::new();
            for (rel, v) in &rows {
                if types(rel).first().map(String::as_str) != Some("node") {
                    continue;
                }
                let (now, before) = (v, &prev_rows[rel]);
                if now == before { continue; }
                let docs = |rs: &[Vec<i64>]| -> BTreeMap<i64, Vec<Vec<i64>>> {
                    let mut m: BTreeMap<i64, Vec<Vec<i64>>> = BTreeMap::new();
                    for r in rs { m.entry(r[0] >> 32).or_default().push(r.clone()); }
                    m
                };
                let (a, b) = (docs(now), docs(before));
                for d in a.keys().chain(b.keys()) {
                    if a.get(d) != b.get(d) { changed.insert(*d); }
                }
            }
            let docs: BTreeSet<i64> = rows.values().flatten().map(|r| r[0] >> 32).collect();
            format!("{}; documents changed {} of {}", rows.iter().map(|(rel, v)| format!("{rel} {}", v.len())).collect::<Vec<_>>().join(", "), changed.len(), docs.len())
        });
        if last == Some(digest) || r == k + 1 || (r == k && !pending_round) {
            report.converged = last == Some(digest);
            report.pending = r == k + 1 && !report.converged;
            return Ok(got);
        }
        if r == k {
            round.program.facts.get_mut(cross_rounds::UNCONVERGED).expect("declared").push(vec![front::Value::Int(1)]);
        }
        last = Some(digest);
        prev_rows = rows;
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
                // The statement this thread now runs, for a dump of a run that has not ended (decision 0084).
                if crate::code_rule_dsl::logic::profile::on() {
                    // SAFETY: for SQLITE_TRACE_STMT, P is the statement.
                    let text = unsafe { std::ffi::CStr::from_ptr(ffi::sqlite3_sql(p.cast::<ffi::sqlite3_stmt>())) }.to_string_lossy().split_whitespace().collect::<Vec<_>>().join(" ");
                    crate::code_rule_dsl::logic::profile::statement(Some(text));
                }
                return 0;
            }
            let profiled = crate::code_rule_dsl::logic::profile::on();
            if profiled {
                crate::code_rule_dsl::logic::profile::statement(None);
            }
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
    /// Drops `rel`'s measurements (to measure it again).
    fn forget(&mut self, rel: &str) {
        self.rels.remove(rel);
        self.vals.retain(|(r, _), _| r != rel);
    }
    /// `from`'s measurements also under the name `to` (the same rows, measured once).
    fn alias(&mut self, from: &str, to: &str) {
        if let Some(r) = self.rels.get(from).cloned() {
            self.rels.insert(to.to_owned(), r);
        }
        let vals: Vec<_> = self.vals.iter().filter(|((r, _), _)| r == from).map(|((_, c), v)| ((to.to_owned(), c.clone()), v.clone())).collect();
        self.vals.extend(vals);
    }
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
            // Per value of this chunk: n, documents, and the sum of squares of its count per document.
            let vals = self.vals.entry((rel.to_owned(), name.clone())).or_default();
            value_runs(rows, c, &doc, |v, count, docs| {
                let e = vals.entry(v).or_default();
                e[0] += count;
                e[1] += docs.len() as i64;
                e[2] += docs.iter().map(|k| k * k).sum::<i64>();
            });
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
    let (mut ndv, mut sq, mut sqd) = (0, 0, 0);
    value_runs(rows, c, doc, |_, n, docs| {
        ndv += 1;
        sq += n * n;
        sqd += docs.iter().map(|k| k * k).sum::<i64>();
    });
    (ndv, sq, sqd)
}

/// Column `c`'s values in runs, by one sort of (value, document): `f(v, n, per_doc)` for each distinct value v, with
/// its row count and its row count in each document it occurs in. No hash map per cell.
fn value_runs(rows: &dyn crate::code_rule_dsl::facts::vtab::Source, c: usize, doc: &dyn Fn(usize) -> i64, mut f: impl FnMut(i64, i64, &[i64])) {
    let mut pairs: Vec<(i64, i64)> = (0..rows.len()).map(|r| (rows.value(r, c), doc(r))).collect();
    pairs.sort_unstable();
    let mut per_doc = Vec::new();
    let mut at = 0;
    while at < pairs.len() {
        let v = pairs[at].0;
        let end = at + pairs[at..].partition_point(|p| p.0 == v);
        per_doc.clear();
        let mut d = at;
        while d < end {
            let de = d + pairs[d..end].partition_point(|p| p.1 == pairs[d].1);
            per_doc.push((de - d) as i64);
            d = de;
        }
        f(v, (end - at) as i64, &per_doc);
        at = end;
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


/// A corpus's files by language: Python's, Rust's and `.rr` rule files.
struct CorpusFiles {
    python: Vec<String>,
    rust: Vec<String>,
    rr: Vec<String>,
    /// Files of no grammar: after `scoped`, those a `.scope text` glob reads as text (`front::Program::text_globs`).
    text: Vec<String>,
}

impl CorpusFiles {
    /// The run's documents: the language's files, then the text ones, in path order.
    fn of(self, label: &str) -> Vec<String> {
        let mut files = match label {
            "rust" => self.rust,
            "rr" => self.rr,
            _ => self.python,
        };
        files.extend(self.text);
        files.sort();
        files
    }
}

/// The language a corpus run reads: the one its program names (`.language python`, `rust` or `rr`), else Rust or
/// Python by `Typing::rust`, or by whichever has more files.
fn corpus_language(language: Option<&str>, typing: &Typing, found: &CorpusFiles) -> &'static str {
    match language {
        Some("python") => "python",
        Some("rust") => "rust",
        Some("rr") => "rr",
        _ if typing.rust.unwrap_or(found.rust.len() > found.python.len()) => "rust",
        _ => "python",
    }
}

/// The files the program's policies select (`front::Program::doc_scopes`, their `.scope`): a document outside
/// every policy's scope is not parsed, and gives no facts.
/// A scope entry taken from a policy option (`.scope dirs option("…")`) reads the option's values, from the run's
/// `options`. A file of no grammar is read, as text only, when a `.scope text` glob matches it and a scope selects it.
fn scoped(mut found: CorpusFiles, prog: &front::Program, options: &serde_json::Value) -> CorpusFiles {
    let text_glob = |p: &String| prog.text_globs.iter().any(|g| crate::code_rule_dsl::logic::strings::test("glob", p, g));
    found.text.retain(text_glob);
    if prog.doc_scopes.is_none() {
        return found;
    }
    let scopes = front::resolve_scopes(&prog.doc_scopes, options);
    let before = found.python.len() + found.rust.len() + found.rr.len();
    for files in [&mut found.python, &mut found.rust, &mut found.rr, &mut found.text] {
        files.retain(|p| front::in_doc_scopes(&scopes, p));
    }
    crate::code_rule_dsl::logic::profile::decision("documents", || {
        format!("{} of {before} files, by the policies' `.scope`; {} read as text (`.scope text`)", found.python.len() + found.rust.len() + found.rr.len(), found.text.len())
    });
    found
}

/// The corpus's files under `root`, relative and in path order: Python's (`.py`), Rust's (`.rs`) and rule files
/// (`.rr`), skipping hidden directories and `target`.
fn corpus_files(root: &Path) -> Result<CorpusFiles> {
    let (mut python, mut rust, mut rr, mut text) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut classify = |relative: String| match crate::code_rule_dsl::language::label_of_path(&relative) {
        Some("python") => python.push(relative),
        Some("rust") => rust.push(relative),
        Some("rr") => rr.push(relative),
        _ => text.push(relative),
    };
    // `RERAIL_FILES=git`: the files Git sees under the root (HEAD's, less deleted, plus staged and untracked ones it
    // does not ignore), with no walk; the same directories skipped. Elsewhere, or outside a Git work tree, the walk.
    let git = std::env::var("RERAIL_FILES").is_ok_and(|v| v == "git").then(|| observe(|o| o.git(root))).flatten();
    if let Some(g) = git {
        let canonical = std::fs::canonicalize(root).map_err(|e| Error::Plan(format!("{}: {e}", root.display())))?;
        let prefix = canonical.strip_prefix(&g.worktree_root).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        let mut listed = 0;
        for file in g.worktree_files() {
            let relative = if prefix.is_empty() {
                file.as_str()
            } else {
                match file.strip_prefix(prefix.as_str()).and_then(|f| f.strip_prefix('/')) {
                    Some(f) => f,
                    None => continue,
                }
            };
            let dirs: Vec<&str> = relative.split('/').collect();
            if dirs[..dirs.len() - 1].iter().any(|d| d.starts_with('.') || *d == "target") {
                continue;
            }
            listed += 1;
            classify(relative.to_owned());
        }
        crate::code_rule_dsl::logic::profile::decision("phase 0: corpus files", || format!("{listed} files from Git ({} overlays: changed, staged or untracked), no walk", g.overlays.len()));
    } else {
    // Phase 0's observer lists each directory: again only when its stat changed (t23).
    let mut pending = vec![root.to_path_buf()];
    observe(|o| o.preload(root));
    while let Some(dir) = pending.pop() {
        let listing = observe(|o| o.listing(&dir)).ok_or_else(|| Error::Plan(format!("{}: not a readable directory", dir.display())))?;
        for (name, is_dir) in listing.iter() {
            let path = dir.join(name);
            if *is_dir {
                if !name.starts_with('.') && name != "target" {
                    pending.push(path);
                }
                continue;
            }
            let relative = path.strip_prefix(root).map_err(|e| Error::Plan(e.to_string()))?.to_string_lossy().into_owned();
            // The language registry's suffixes (`languages/<id>.json`): Python's include stubs (`.pyi`).
            classify(relative);
        }
    }
    }
    python.sort();
    rust.sort();
    rr.sort();
    text.sort();
    // `RERAIL_SAMPLE_EVERY=n`: every n-th file only, to iterate on a rule's cost quickly (its findings are a sample's).
    if let Some(n) = std::env::var("RERAIL_SAMPLE_EVERY").ok().and_then(|n| n.parse::<usize>().ok()).filter(|n| *n > 1) {
        for files in [&mut python, &mut rust, &mut rr] {
            *files = files.iter().step_by(n).cloned().collect();
        }
    }
    Ok(CorpusFiles { python, rust, rr, text })
}

/// A relation's column statistics (`syntax::statistics`: rows, ndv, sq, sqd) measured over `first` rows, scaled to
/// `rows`. A key column stays a key; a column with many values (over one row in 16 distinct: node ids, strings) gains
/// values in proportion, one with few (kinds, field names) keeps them and its postings grow. `sq` keeps the postings'
/// sizes against the values, and `sqd` grows with the documents, whose postings keep their sizes.
fn scaled_statistics(measured: &[(String, i64, i64, i64, i64)], first: usize, rows: usize) -> Vec<(String, i64, i64, i64, i64)> {
    let r = rows as f64 / first as f64;
    let n = rows as i64;
    measured
        .iter()
        .map(|(name, m_rows, ndv, sq, sqd)| {
            if n == 0 {
                return (name.clone(), 0, 0, 0, 0);
            }
            if ndv == m_rows {
                return (name.clone(), n, n, n, n);
            }
            let ndv_scaled = if ndv * 16 > *m_rows { (*ndv as f64 * r).round() as i64 } else { *ndv }.clamp(1, n);
            let sq_scaled = (*sq as f64 * r * r * (*ndv as f64 / ndv_scaled as f64)).round() as i64;
            let sqd_scaled = (*sqd as f64 * r).round() as i64;
            (name.clone(), n, ndv_scaled, sq_scaled.max(n), sqd_scaled.max(n))
        })
        .collect()
}

/// A digest as lowercase hex.
fn hex(digest: &[u8]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// A memo entry's rows holding its file's tree statistics (`Own::put`).
const OWN_KINDS: &str = "own:kinds";
const OWN_EDGES: &str = "own:edges";
const OWN_DOCS: &str = "own:docs";
const OWN_POSTINGS: &str = "own:postings:";

/// The base relations' statistics a chunk measures as it binds them, beyond each column's moments
/// (`syntax::statistics`), which T's `stats.py` made at ingestion: per kind atom its nodes and summed subtree sizes
/// (`stat_kind`), per (parent kind, child kind) its edges (`stat_edge`), the documents, and per (relation, column,
/// constant) of the program's constants its rows (`stat_val`'s `n`). Summed over the chunks for the cross-file part:
/// a document is in one chunk, so every count adds exactly.
/// The store name of a chunk mix's measured tree counts under the file-local program `program`.
fn own_name(program: &[u8; 32], mix: &str) -> String {
    format!("own-counts/{}/{mix}", memo::plan_digest(program).iter().map(|b| format!("{b:02x}")).collect::<String>())
}

/// A chunk mix's measured tree counts and the nodes they were measured over, stored by an earlier run.
/// Its source is named in the run's manifest (`held`), so collection keeps it (decision 0053).
fn own_counts(store: &ContentStore, program: &[u8; 32], mix: &str, held: &mut Vec<crate::content_store::SourceId>) -> Option<(usize, Own)> {
    let digest = store.binding(&own_name(program, mix)).ok()??;
    let source = store.source_by_hash(crate::content_store::HashKey::from_sha256_digest(digest)).ok()??;
    let own = Own::decode(&store.get_source(source).ok()?)?;
    held.push(source);
    Some(own)
}

/// Stores a chunk mix's measured tree counts, for a later run whose files of that mix are all read from the memo.
fn record_own_counts(store: &ContentStore, program: &[u8; 32], mix: &str, nodes: usize, own: &Own, held: &mut Vec<crate::content_store::SourceId>) -> Result<()> {
    use sha2::Digest as _;
    let plan = |e: crate::content_store::StoreError| Error::Plan(e.to_string());
    let bytes = own.encode(nodes);
    held.push(store.put_source(&bytes, &[]).map_err(plan)?);
    let digest: [u8; 32] = sha2::Sha256::digest(&bytes).into();
    let producer = memo::plan_digest(program);
    let record = crate::content_store::DerivationRecord { output: own_name(program, mix), producer, inputs: Vec::new(), digest, status: 0, cost: 0 };
    store.record_derivations(&[record]).map_err(plan)?;
    Ok(())
}

#[derive(Clone, Default)]
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
        use rustc_hash::{FxHashMap, FxHashSet};
        let mut own = Own::default();
        let of = |rel: &str| sources.iter().find(|(r, _)| r == rel).map(|(_, s)| s.as_ref());
        let column = |source: &dyn crate::code_rule_dsl::facts::vtab::Source, name: &str| source.columns().iter().position(|c| c == name);
        // Kinds by node id, then the tape's sizes and parents against them. Counted in hash maps (a chunk is a
        // quarter million nodes: an ordered map per node was most of this measure's time) and ordered once.
        let mut kind_of = FxHashMap::default();
        let mut kinds: FxHashMap<i64, (i64, i64)> = FxHashMap::default();
        let mut edges: FxHashMap<(i64, i64), i64> = FxHashMap::default();
        if let Some(node) = of("node")
            && let (Some(n), Some(k), Some(d)) = (column(node, "n"), column(node, "kind"), column(node, "d"))
        {
            kind_of.reserve(node.len());
            let mut docs = FxHashSet::default();
            for r in 0..node.len() {
                let kind = node.value(r, k);
                kind_of.insert(node.value(r, n), kind);
                kinds.entry(kind).or_default().0 += 1;
                docs.insert(node.value(r, d));
            }
            own.docs = docs.len() as i64;
        }
        if let Some(tape) = of("tape")
            && let (Some(n), Some(par), Some(size)) = (column(tape, "n"), column(tape, "par"), column(tape, "size"))
        {
            for r in 0..tape.len() {
                let Some(&kind) = kind_of.get(&tape.value(r, n)) else { continue };
                kinds.entry(kind).or_default().1 += tape.value(r, size);
                if let Some(&parent) = kind_of.get(&tape.value(r, par)) {
                    *edges.entry((parent, kind)).or_default() += 1;
                }
            }
        }
        own.kinds = kinds.into_iter().collect();
        own.edges = edges.into_iter().collect();
        for ((rel, col), values) in constants {
            let Some(source) = of(rel) else { continue };
            let Some(c) = column(source, col) else { continue };
            let wanted: FxHashSet<i64> = values.iter().copied().collect();
            let mut counts: FxHashMap<i64, i64> = FxHashMap::default();
            for r in 0..source.len() {
                let v = source.value(r, c);
                if wanted.contains(&v) {
                    *counts.entry(v).or_default() += 1;
                }
            }
            own.postings.extend(counts.into_iter().map(|(v, n)| ((rel.clone(), col.clone(), v), n)));
        }
        own
    }

    /// These counts for a chunk `r` times this one's size: every count is of nodes, edges, documents or rows, so
    /// each scales by `r`.
    fn scaled(&self, r: f64) -> Own {
        let s = |n: i64| (n as f64 * r).round() as i64;
        Own {
            kinds: self.kinds.iter().map(|(k, (n, size))| (*k, (s(*n), s(*size)))).collect(),
            edges: self.edges.iter().map(|(e, n)| (*e, s(*n))).collect(),
            docs: s(self.docs).max(1),
            postings: self.postings.iter().map(|(k, n)| (k.clone(), s(*n))).collect(),
        }
    }

    /// The counts and the nodes they were measured over, as bytes (`own_counts`).
    fn encode(&self, nodes: usize) -> Vec<u8> {
        let mut b = Vec::new();
        let int = |b: &mut Vec<u8>, v: i64| b.extend_from_slice(&v.to_le_bytes());
        let text = |b: &mut Vec<u8>, t: &str| {
            b.extend_from_slice(&(t.len() as u32).to_le_bytes());
            b.extend_from_slice(t.as_bytes());
        };
        b.extend_from_slice(b"own1");
        int(&mut b, nodes as i64);
        int(&mut b, self.docs);
        int(&mut b, self.kinds.len() as i64);
        for (k, (n, size)) in &self.kinds {
            [*k, *n, *size].into_iter().for_each(|v| int(&mut b, v));
        }
        int(&mut b, self.edges.len() as i64);
        for ((p, c), n) in &self.edges {
            [*p, *c, *n].into_iter().for_each(|v| int(&mut b, v));
        }
        int(&mut b, self.postings.len() as i64);
        for ((rel, col, v), n) in &self.postings {
            text(&mut b, rel);
            text(&mut b, col);
            int(&mut b, *v);
            int(&mut b, *n);
        }
        b
    }

    fn decode(b: &[u8]) -> Option<(usize, Own)> {
        let mut at = 4usize;
        if b.get(..4)? != b"own1" {
            return None;
        }
        let mut int = || -> Option<i64> {
            let v = i64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?);
            at += 8;
            Some(v)
        };
        let nodes = int()? as usize;
        let mut own = Own { docs: int()?, ..Own::default() };
        for _ in 0..int()? {
            let (k, n, size) = (int()?, int()?, int()?);
            own.kinds.insert(k, (n, size));
        }
        for _ in 0..int()? {
            let (p, c, n) = (int()?, int()?, int()?);
            own.edges.insert((p, c), n);
        }
        let postings = int()?;
        let text = |at: &mut usize| -> Option<String> {
            let n = u32::from_le_bytes(b.get(*at..*at + 4)?.try_into().ok()?) as usize;
            let t = String::from_utf8(b.get(*at + 4..*at + 4 + n)?.to_vec()).ok()?;
            *at += 4 + n;
            Some(t)
        };
        for _ in 0..postings {
            let rel = text(&mut at)?;
            let col = text(&mut at)?;
            let v = i64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?);
            let n = i64::from_le_bytes(b.get(at + 8..at + 16)?.try_into().ok()?);
            at += 16;
            own.postings.insert((rel, col, v), n);
        }
        Some((nodes, own))
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

    /// Into a memo entry's rows (decision 0052): a memoised file hands on its share of the tree statistics as an
    /// evaluated one does.
    fn put(&self, rows: &mut BTreeMap<String, (usize, Vec<i64>)>) {
        rows.insert(OWN_KINDS.into(), (3, self.kinds.iter().flat_map(|(k, (n, size))| [*k, *n, *size]).collect()));
        rows.insert(OWN_EDGES.into(), (3, self.edges.iter().flat_map(|((p, k), n)| [*p, *k, *n]).collect()));
        rows.insert(OWN_DOCS.into(), (1, vec![self.docs]));
        for ((rel, col, v), n) in &self.postings {
            rows.entry(format!("{OWN_POSTINGS}{rel}:{col}")).or_insert_with(|| (2, Vec::new())).1.extend([*v, *n]);
        }
    }

    /// From a memo entry's rows (`put`).
    fn taken(rows: &BTreeMap<String, (usize, Vec<i64>)>) -> Own {
        let mut own = Own::default();
        let flat = |key: &str| rows.get(key).map(|(_, v)| v.as_slice()).unwrap_or_default();
        own.kinds = flat(OWN_KINDS).chunks(3).map(|r| (r[0], (r[1], r[2]))).collect();
        own.edges = flat(OWN_EDGES).chunks(3).map(|r| ((r[0], r[1]), r[2])).collect();
        own.docs = flat(OWN_DOCS).first().copied().unwrap_or(0);
        for (key, (_, values)) in rows.range(OWN_POSTINGS.to_owned()..).take_while(|(k, _)| k.starts_with(OWN_POSTINGS)) {
            let Some((rel, col)) = key[OWN_POSTINGS.len()..].split_once(':') else { continue };
            own.postings.extend(values.chunks(2).map(|r| ((rel.to_owned(), col.to_owned(), r[0]), r[1])));
        }
        own
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

#[cfg(test)]
mod tests {
    /// Decision 0076: cross-file rows memoised per import component. b imports a; c imports nothing. An edit
    /// re-derives the edited file's component and its importers' only; `verify` derives everything again and
    /// compares; the contract refuses a relation that reads an undeclared cross-file one, and a row naming a node
    /// outside its closure.
    #[test]
    fn cross_file_rows_are_memoised_per_import_component() {
        let marker = format!("# {:?}\n", std::time::SystemTime::now());
        let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("cross-memo");
        std::fs::create_dir_all(&dir).unwrap();
        let write = |file: &str, body: &str| std::fs::write(dir.join(file), format!("{marker}{body}")).unwrap();
        write("a.py", "def f():\n    pass\n");
        write("b.py", "from a import f\ndef g():\n    pass\n");
        write("c.py", "def h():\n    pass\n");
        let program = |pragma: &str, extra: &str| {
            let rule = dir.join("uses.rr");
            std::fs::write(&rule, format!(r#"
.pragma "cross-memo" "{pragma}"
.decl def(n: node, q: symbol)
def(n, @dot(m, x)) :- kind(n, "function_definition"), field(n, "name", c), text(c, x), docof(n, d), doc(d, _, m).
.decl imp(n: node, q: symbol)
imp(n, @dot(m, x)) :- kind(n, "import_from_statement"), field(n, "module_name", mn), text(mn, m), field(n, "name", xn), text(xn, x).
.decl uses(n: node, f: node)
uses(n, f) :- imp(n, q), def(f, q).
{extra}
.output uses
"#)).unwrap();
            crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap()
        };
        let out = dir.join("out");
        let _no_session = super::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
        let run = |text: &str| super::run(text, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18);
        let uses = || std::fs::read_to_string(out.join("uses.csv")).unwrap();
        let text = program("uses", "");
        let first = run(&text).unwrap();
        assert_eq!(first.cross, (0, 3), "three components, none stored yet");
        let derived = uses();
        assert_eq!(derived.lines().count(), 1, "{derived}");
        assert!(derived.starts_with("b.py:"), "{derived}");
        assert_eq!(run(&text).unwrap().cross, (3, 0), "unchanged: every component found");
        assert_eq!(uses(), derived, "replayed rows are the derived ones");
        // An edit to c, which nothing imports: c alone is derived.
        write("c.py", "def h():\n    return 1\n");
        assert_eq!(run(&text).unwrap().cross, (2, 1));
        assert_eq!(uses(), derived);
        // An edit to a: a and b, which imports it, are derived; c is found.
        write("a.py", "def f():\n    return 2\n");
        assert_eq!(run(&text).unwrap().cross, (1, 2));
        assert_eq!(uses(), derived);
        // Verify: nothing replayed, every found component's stored rows equal to those derived.
        // SAFETY: only this test reads RERAIL_CROSS_MEMO while it is set (the others declare no pragma).
        unsafe { std::env::set_var("RERAIL_CROSS_MEMO", "verify") };
        let verified = run(&text);
        unsafe { std::env::remove_var("RERAIL_CROSS_MEMO") };
        assert_eq!(verified.unwrap().cross, (3, 0));
        assert_eq!(uses(), derived);
        // The contract: a memoised relation reading a cross-file relation the pragma does not name is refused.
        let reads = program("wide", ".decl wide(n: node)\nwide(n) :- uses(n, _).\n.output wide");
        let err = run(&reads).err().map(|e| e.to_string()).unwrap_or_default();
        assert!(err.contains("wide") && err.contains("uses"), "{err}");
        // A row about a whose other node is in b, which a does not import, is outside a's closure.
        let back = program("uses back", ".decl back(f: node, n: node)\nback(f, n) :- uses(n, f).\n.output back");
        let err = run(&back).err().map(|e| e.to_string()).unwrap_or_default();
        assert!(err.contains("back") && err.contains("outside its import closure"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(feature = "tape-cache")]
    #[test]
    fn a_warm_run_resumes_deferred_collection() {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("rerail-warm-collection-{}-{nonce}.sqlite", std::process::id()));
        let store = crate::content_store::ContentStore::open(&path).unwrap();
        let source = store.put_source(b"rooted", &[]).unwrap();
        let orphan_hash = crate::content_store::HashKey::sha256(b"unrooted");
        store.put_source(b"unrooted", &[]).unwrap();
        store.collect(100, 100).unwrap();
        let root = path.with_extension("root");
        let program = [7; 32];
        // Simulate another active run deferring the first run's background collection.
        let reader = store.hold_collection().unwrap();
        super::record_run(&store, &root, &[source], &program, &[], &[], None).unwrap().unwrap().join().unwrap().1.unwrap();
        assert!(store.corpus_collection_due().unwrap());
        // Same files and plan: the manifest is unchanged, but pending collection must still run.
        super::record_run(&store, &root, &[source], &program, &[], &[], Some(reader)).unwrap().unwrap().join().unwrap().1.unwrap();
        assert!(!store.corpus_collection_due().unwrap());
        assert_eq!(store.source_by_hash(orphan_hash).unwrap(), None);
    }

    #[test]
    fn a_chunks_mix_is_the_file_types_holding_a_tenth_of_its_nodes() {
        assert_eq!(super::chunk_mix([("a/b.py", 900), ("c.ts", 50), ("d.ts", 60)].into_iter()), "py,ts/4");
        assert_eq!(super::chunk_mix([("a/b.py", 950), ("c.ts", 50)].into_iter()), "py/4");
        assert_eq!(super::chunk_mix([("Makefile", 10), ("x.v1/README", 10)].into_iter()), "/1");
        // Many small files and a few large ones of one language are two mixes.
        assert_ne!(super::chunk_mix([("a.py", 20), ("b.py", 30)].into_iter()), super::chunk_mix([("c.py", 2000)].into_iter()));
    }
}
