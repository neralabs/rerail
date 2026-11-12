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
use crate::code_rule_dsl::typing::cross_rounds;
pub mod guests;
pub mod hiding;
pub mod exports;
pub mod combined;
pub mod scoping;
#[cfg(test)]
pub mod cache_architecture;
#[cfg(test)]
pub mod cache_tests;
pub use hiding::qualified;
pub mod memo;
pub mod kept_strata;
pub mod maintained;
pub mod cone_rounds;
// cratify: `references` is rerail-engine's (it mounts it)

/// The reference graph's base relations (`reference_edge`, `reference_open`), and their columns
/// (`sql::select::input_columns`).
pub const REFERENCE_RELATIONS: [&str; 2] = ["reference_edge", "reference_open"];

/// Base relations a program reads that are built across a run's files (the reference graph's,
/// `corpus::references`): each file added as its facts are built, the rows bound in the cross-file part.
pub trait CrossFileRelations: Send + Sync {
    /// Adds the file with key `key` at `path`, with its source and facts.
    fn add(&self, key: u32, path: &str, source: &Arc<String>, facts: &Arc<crate::code_rule_dsl::facts::core::Facts>) -> Result<()>;
    /// The relations the run's files give, with their columns and rows.
    fn relations(self: Box<Self>, store: &crate::content_store::ContentStore, read: &BTreeSet<String>) -> Result<Vec<(&'static str, Vec<String>, Vec<Option<Vec<i64>>>)>>;
}

/// What makes a program's cross-file relations over the files at `paths` of the dialect `label`, where it reads them.
pub(crate) type MakeCrossFile = fn(&str, &crate::code_rule_dsl::logic::front::Program, &BTreeSet<String>, &[String]) -> Result<Option<Box<dyn CrossFileRelations>>>;

static CROSS_FILE: std::sync::OnceLock<MakeCrossFile> = std::sync::OnceLock::new();

/// Registers what makes a program's cross-file relations (once, at load).
pub fn install_cross_file(make: MakeCrossFile) {
    let _ = CROSS_FILE.set(make);
}
pub mod prepared;
pub mod maintain;
pub mod supports;
pub mod recursive_spike;
pub mod retain;
pub mod keptstats;
pub mod input_delta;
pub mod bridge;
pub mod phases;
pub mod constants;
pub mod inputs;
pub use inputs::parameters as input_parameters;
pub mod world;
pub mod connections;
pub mod reuse;
pub mod phase0;
pub mod cross_memo;
pub mod matchers;
/// The prefix of an `ast` matcher's relation (decision 0106).
pub(in crate::code_rule_dsl) const AST_MATCHES: &str = matchers::PREFIX;
pub use matchers::columns as ast_columns;
pub mod typing;
// cratify: `base_tests` is rerail-device-metal's (it mounts it)
pub use typing::{Typing, TypingReport};
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
pub fn atomized_statistics(t: &Connection, db: &Connection, store: &ContentStore, first: &HashMap<i64, i64>, dialect: GrammarDialect) -> Result<()> {
    use lower::Domain;
    let tag = crate::code_rule_dsl::language::tape_tag(dialect).ok_or_else(|| Error::Plan("the dialect has no tape tag".to_owned()))?;
    let kinds = dialect.kinds();
    let kind = |s: &str| i64::from(crate::code_rule_dsl::facts::core::kind_atom(tag, kinds.kind_id(s)));
    let field = |s: &str| kinds.field_id(s).map_or(-1, |f| i64::from(syntax::field_atom(tag, f)));
    let domain = |tbl: &str, col: &str| match (tbl, col) {
        ("node", "kind") => Domain::Kind,
        ("field", "f") => Domain::Field,
        ("text" | "op" | "op_at" | "path_part" | "file_name", "s") | ("value", "v") | ("doc", "path" | "module") | ("pkg", "p") => Domain::Name,
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

pub use crate::code_rule_dsl::prof::thread_usage;

pub(crate) fn usage() -> (f64, f64, f64) {
    // wasm32: the one thread's time as user CPU, and no resident-set counter.
    #[cfg(target_family = "wasm")]
    {
        (crate::code_rule_dsl::prof::single_thread_cpu().as_secs_f64(), 0.0, 0.0)
    }
    // Windows: the process's user and kernel CPU, and its peak working set.
    #[cfg(windows)]
    {
        let (user, kernel) = crate::code_rule_dsl::prof::windows_cpu(false);
        (user.as_secs_f64(), kernel.as_secs_f64(), crate::code_rule_dsl::prof::windows_peak_working_set() as f64 / 1e6)
    }
    #[cfg(unix)]
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
pub struct Report {
    /// (phase, user, sys, peak RSS so far, SQLite statements run).
    pub phases: Vec<(&'static str, f64, f64, f64, usize)>,
    /// Per phase, the whole process's retired instructions while it ran (its workers' too; macOS): steadier than CPU
    /// time on a loaded machine, so a change is attributed to a phase by them.
    pub phase_instructions: Vec<(&'static str, u64)>,
    pub files: usize,
    pub nodes: usize,
    pub native: usize,
    pub rows: BTreeMap<String, usize>,
    pub chunks: usize,
    /// The cross-file part's bounded rounds (`typing::cross_rounds`).
    pub rounds: cross_rounds::Rounds,
    /// Declarations, summaries and terms (`corpus::typing`).
    pub typing: TypingReport,
    /// Text tests (decision 0037): relations, distinct strings tested, and the CPU testing them (seconds).
    pub text: (usize, usize, f64),
    /// The typing programs run for the rule's bridged relations (`bridge`), with their reports.
    pub typing_runs: bridge::Runs,
    /// The earlier phases this run read (decision 0083 §1), each its own run.
    pub phase_runs: Vec<phases::PhaseRun>,
    /// The per-file memo: files read from it, and chunks whose rows it could not keep (a row naming no document).
    pub memo: (usize, usize),
    /// The cross-file memo (decision 0076): import components found and components derived.
    pub cross: (usize, usize),
    /// Where the prepared program came from: `prepared` by this run, kept in this process's `memory`, or read from the
    /// `store` (decision 0073).
    pub prepared: &'static str,
    /// A session's run: the cross-file strata it evaluated, of all (decision 0077; `None` outside a session).
    pub cross_strata: Option<(usize, usize)>,
    /// A run with no session, its strata kept in the tape cache (decision 0077 §4): the cross-file strata it
    /// evaluated, of all.
    pub cross_kept: Option<(usize, usize)>,
    /// The cone of requests' strata evaluated, of all, when they are kept (t33 K3); `None` otherwise.
    pub(crate) cone_kept: Option<(usize, usize)>,
    /// The rounds the cone ran from the last run's final round (decision 0108); 0 when it ran the gated fixpoint.
    pub(crate) cone_rounds: usize,
    /// Of those, strata maintained from deltas, of them recursive, and strata derived again (decision 0077 §3).
    pub cross_maintained: (usize, usize, usize),
    /// A session's run: the cross part's inputs that changed, and why its strata derived again were (each count).
    pub cross_changed: Vec<String>,
    pub cross_again: BTreeMap<&'static str, usize>,
    /// With `RERAIL_CROSS_DELTA_COUNTS`: rows over-deleted, inserted and held by the maintained relations, the rows of
    /// the deltas of their inputs, and the rows truly deleted and inserted.
    pub cross_delta_counts: [usize; 6],
    /// What the run consulted, as (what, path, digest): each workspace document's bytes, and what its requests read
    /// (`world::World::consulted`). A run whose consulted things are as they were derives the same (`unchanged`).
    pub(crate) consulted: Vec<(String, String, String)>,
    /// Each document's rows of the per-file relations the requests' cone reads, as a digest, by path: node ids by
    /// their rank in the document, so an edit that renumbers nodes and nothing else leaves it. Two runs whose
    /// documents give the same digests, and whose requests are answered alike, load the same documents.
    pub(crate) cone_inputs: BTreeMap<String, String>,
    /// An output that is not a union of file-local rows, if any. With none, each document's rows are its own,
    /// whatever else is read, and can be replaced on their own.
    pub(crate) not_per_document: Option<String>,
    /// What phase 0's observer did during the run: stats, answers reused, files hashed, directories listed.
    pub files_observed: crate::content_store::observe::Counts,
}

/// Whether a run of `programs` over `root` that consulted `consulted` (`Report::consulted`) would consult the same
/// now: the same workspace documents with the same bytes, and every request answered alike (decision 0083 §2). It
/// reads what the run read, and derives nothing. A program whose inputs come from elsewhere too (declarations,
/// earlier phases, an explicit batch) is never judged unchanged.
pub(crate) fn unchanged(programs: &[&str], root: &Path, typing: &Typing, options: &serde_json::Value, consulted: &[(String, String, String)]) -> Result<bool> {
    Ok(changes(programs, root, typing, options, consulted)?.is_some_and(|c| c.is_empty()))
}

/// The workspace documents whose bytes changed since a run that consulted `consulted`, when nothing else it
/// consulted did: the same documents, by path, and every request's answer as it was (`World::still`). `None` when
/// anything else changed, or the run cannot be compared (its documents named, typing, phases read).
pub(crate) fn changes(programs: &[&str], root: &Path, typing: &Typing, options: &serde_json::Value, consulted: &[(String, String, String)]) -> Result<Option<Vec<String>>> {
    if options.get("inputFiles").is_some() || consulted.is_empty() {
        return Ok(None);
    }
    let flags: Vec<String> = options.get("env").and_then(|v| v.as_array()).into_iter().flatten().filter_map(|v| v.as_str().map(str::to_owned)).collect();
    let settings = crate::code_rule_dsl::logic::settings::settings(root, &flags).map_err(Error::Plan)?;
    let with_settings: Vec<String> = programs.iter().map(|p| crate::code_rule_dsl::logic::settings::with_settings(p, &settings)).collect();
    let programs: Vec<&str> = with_settings.iter().map(String::as_str).collect();
    let prog = combine(&programs, "")?;
    if prog.decls.contains_key("declared_ret") || !phases::wanted(&prog, None)?.is_empty() {
        return Ok(None);
    }
    let language = prog.settings.iter().find(|(key, _)| key == "language").map(|(_, value)| value.trim().trim_matches('"').to_owned());
    let found = scoped(corpus_files(root)?, &prog, options);
    let label = corpus_language(language.as_deref(), typing, &found);
    let files = found.of(label);
    let docs: Vec<&(String, String, String)> = consulted.iter().filter(|c| c.0 == "doc").collect();
    if files.len() != docs.len() || files.iter().zip(&docs).any(|(f, d)| *f != d.1) {
        return Ok(None);
    }
    // A document listed without a digest was not read (none of the run's rules reads a document): its edit changes nothing.
    let changed: Vec<String> = docs.iter().filter(|d| !d.2.is_empty() && world::bytes_digest(&root.join(&d.1)) != d.2).map(|d| d.1.clone()).collect();
    const READERS: [&str; 6] = ["input", "input_dir", "artifact", "base", "git_rev", "git_index"];
    let (read, requests): (Vec<_>, Vec<_>) = consulted.iter().filter(|c| c.0 != "doc").cloned().partition(|c| READERS.contains(&c.0.as_str()));
    Ok((inputs::still(&prog, root, &read) && world::World::new(root, &prog).still(&requests)).then_some(changed))
}

use std::collections::{BTreeMap, BTreeSet};

/// Rows of values as a relation of atoms (`c0`, `c1`, …), the store's atoms for strings.
fn atoms_source<'a>(store: &ContentStore, width: usize, rows: impl Iterator<Item = &'a Vec<front::Value>> + Clone) -> Result<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>> {
    let names: Vec<String> = (0..width).map(|c| format!("c{c}")).collect();
    let cols: Vec<&str> = names.iter().map(String::as_str).collect();
    let texts: Vec<&str> = rows.clone().flatten().filter_map(|v| if let front::Value::Str(s) = v { Some(s.as_str()) } else { None }).collect();
    let mut atoms = store.intern_atoms(&texts).map_err(|e| Error::Plan(e.to_string()))?.into_iter().map(i64::from);
    let mut relation = crate::code_rule_dsl::facts::vtab::Relation::new(&cols);
    for row in rows {
        let row: Vec<i64> = row.iter().map(|v| match v { front::Value::Int(i) => *i, _ => atoms.next().expect("an atom per string") }).collect();
        relation.push(&row);
    }
    Ok(Arc::new(relation))
}

/// Statistics under `columns`' names, in order (a kept count's columns are named as the source it was first made of).
fn rename(measured: Vec<(String, i64, i64, i64, i64)>, columns: &[String]) -> Vec<(String, i64, i64, i64, i64)> {
    measured.into_iter().zip(columns).map(|((_, a, b, c, d), name)| (name.clone(), a, b, c, d)).collect()
}

/// `old` with the rows of `minus` gone and those of `plus` added (a set: a row gone is gone however often it stood).
fn patched(old: &dyn crate::code_rule_dsl::facts::vtab::Source, plus: &dyn crate::code_rule_dsl::facts::vtab::Source, minus: &dyn crate::code_rule_dsl::facts::vtab::Source) -> Arc<dyn crate::code_rule_dsl::facts::vtab::Source> {
    let width = old.columns().len();
    let row = |s: &dyn crate::code_rule_dsl::facts::vtab::Source, i: usize| (0..width).map(|c| s.value(i, c)).collect::<Vec<i64>>();
    let gone: std::collections::HashSet<Vec<i64>> = (0..minus.len()).map(|i| row(minus, i)).collect();
    let cols = old.columns();
    let mut relation = crate::code_rule_dsl::facts::vtab::Relation::new(&cols.iter().map(String::as_str).collect::<Vec<_>>());
    for i in 0..old.len() {
        let r = row(old, i);
        if !gone.contains(&r) {
            relation.push(&r);
        }
    }
    for i in 0..plus.len() {
        relation.push(&row(plus, i));
    }
    Arc::new(relation)
}

/// A phase's rows as text, as values of `decl`'s types.
fn values_of(decl: &[(String, String)], rows: &[Vec<String>]) -> Vec<Vec<front::Value>> {
    rows.iter()
        .map(|r| r.iter().zip(decl).map(|(v, (_, t))| if t == "number" || t == "int" { v.parse::<i64>().map_or_else(|_| front::Value::Str(v.clone()), front::Value::Int) } else { front::Value::Str(v.clone()) }).collect())
        .collect()
}

/// `Report::cone_inputs`: each document's rows of `per_file` (the exports held in memory, `exports`), by path. A value naming a node of the
/// document is its rank among those its rows name; the rest (symbols, document keys, numbers) as they are.
fn cone_digests<'a>(exports: &exports::Exports, per_file: &[String], locations: &BTreeMap<String, Vec<usize>>, program: &front::Program, docs: impl Iterator<Item = &'a (i64, String)>) -> Result<BTreeMap<String, String>> {
    use sha2::{Digest, Sha256};
    let mut by_doc: HashMap<i64, Vec<(usize, Vec<i64>)>> = HashMap::new();
    for (i, rel) in per_file.iter().enumerate() {
        let Some(at) = locations.get(rel).and_then(|l| l.first().copied()) else { continue };
        let width = program.decls.get(rel).map_or(0, Vec::len);
        exports.visit(rel,width,|row| {
            by_doc.entry(memo::document_of(row[at])).or_default().push((i,row.to_vec()));
        });
    }
    let mut out = BTreeMap::new();
    for (d, path) in docs {
        let mut rows = by_doc.remove(d).unwrap_or_default();
        let nodes: BTreeSet<i64> = rows.iter().flat_map(|(_, r)| r.iter().copied()).filter(|v| *v >= 1 << 32 && v >> 32 == *d).collect();
        let rank: HashMap<i64, i64> = nodes.into_iter().enumerate().map(|(k, v)| (v, -1 - k as i64)).collect();
        for (_, r) in &mut rows {
            for v in r.iter_mut() {
                if let Some(k) = rank.get(v) {
                    *v = *k;
                }
            }
        }
        rows.sort();
        let mut h = Sha256::new();
        for (i, r) in &rows {
            h.update((*i as u64).to_le_bytes());
            for v in r {
                h.update(v.to_le_bytes());
            }
        }
        out.insert(path.clone(), h.finalize().iter().map(|b| format!("{b:02x}")).collect());
    }
    Ok(out)
}

pub mod session;

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

/// The `core.rr` relations a rule may read that the engine does not make yet (O76): each is empty. None now:
/// `string_annotation` is made (`facts::vtab::syntax::annotation`, Ruff's table) and `parsed` is a guest (0055).
const NOT_MADE: [(&str, &str); 0] = [];

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
            .filter(|r| need.contains(r.head.rel.as_str()))
            .flat_map(|r| r.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.to_string()))
            .filter(|rel| !need.contains(rel.as_str()))
            .collect();
        if more.is_empty() {
            break;
        }
        need.extend(more);
    }
    let mut slice = local.clone();
    slice.rules.retain(|r| need.contains(r.head.rel.as_str()));
    slice.outputs = wanted.iter().map(|r| (*r).to_owned()).collect();
    Some(slice)
}

pub fn run(program: &str, root: &Path, t_facts: Option<&Path>, out: &Path, modes: &str, binding: Binding, chunk_nodes: usize) -> Result<Report> {
    run_typed(&[program], root, t_facts, out, modes, binding, chunk_nodes, &Typing::default())
}

/// Every program of `programs` in one run: each file is read, parsed and batched once, and every rule runs over each
/// chunk (`combine`).
pub fn run_all(programs: &[&str], root: &Path, out: &Path, modes: &str, binding: Binding, chunk_nodes: usize) -> Result<Report> {
    run_typed(programs, root, None, out, modes, binding, chunk_nodes, &Typing::default())
}

/// The programs `programs` (`.rr` text, each with `extra` after it) as one program. One program is parsed as it is.
/// Of several, each one's own relations (the ones it derives or declares beyond its inputs) take a prefix, `p<i>_`,
/// so programs that name relations alike (a converter's `m650`, `exists_0`) do not meet. The base inputs and the
/// `violation` output are shared, and kind sets are named by their kinds (`ks<j>`), so programs asking for one set
/// share its tape pass. The simplifier then merges relations that are equal across programs (decision 0038).
pub fn combine(programs: &[&str], extra: &str) -> Result<front::Program> {
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
    let program = combine_stored(programs, extra, &key)?;
    let mut kept = cache.lock().expect("the combined programs");
    if kept.len() >= KEPT {
        kept.clear();
    }
    kept.insert(key, program.clone());
    Ok(program)
}

/// `combine_texts`, kept in the persistent store across processes for a pack of programs (`combined`, decision 0094
/// §6): keyed by the texts and checked against the library files the combine read.
fn combine_stored(programs: &[&str], extra: &str, key: &[u8; 32]) -> Result<front::Program> {
    let store = if cfg!(feature = "tape-cache") && programs.len() > 1 && std::env::var_os("RERAIL_DUMP_PROGRAMS").is_none() {
        tape_cache().ok().flatten()
    } else {
        None
    };
    let Some(store) = store else { return combine_texts(programs, extra) };
    // The run's one hold on collection (O108) is taken later; this read and write hold it themselves.
    let _reading = store.hold_collection().map_err(|e| Error::Plan(e.to_string()))?;
    let found = {
        let _lookup = crate::code_rule_dsl::logic::profile::span(|| "the combined program, from the store (decision 0094 §6)".into());
        store.settle(&combined::queued(key));
        combined::lookup(&store, key).map_err(|e| Error::Plan(e.to_string()))?
    };
    if let Some(program) = found {
        crate::code_rule_dsl::prof::count("corpus.combined_stored", 1);
        crate::code_rule_dsl::logic::profile::decision("combined program (decision 0094 §6)", || "read from the store: the same texts, and every library file they read reads the same".into());
        return Ok(program);
    }
    let (program, read) = crate::code_rule_dsl::logic::rr::recording(|| combine_texts(programs, extra));
    let program = program?;
    {
        let _keep = crate::code_rule_dsl::logic::profile::span(|| "keep the combined program in the store, queued".into());
        // Checked, made into facts and written by the store's writer (decision 0102); one that does not read back as
        // written is not kept.
        let (key, files, kept) = (*key, read.clone(), program.clone());
        store.write_back("the combined program", 0, vec![(combined::queued(&key), None)], Box::new(move |store| combined::remember(store, &key, &files, &kept).map(drop)));
    }
    crate::code_rule_dsl::logic::profile::decision("combined program (decision 0094 §6)", || format!("combined; kept in the store behind the run, with the {} library files it read", read.len()));
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
        // What the text includes, as phase 0 found it: its modules' paths and digests, and the graph's edges. A text
        // is a function of these, not of its `#include` lines alone.
        if let Ok(Some(graph)) = crate::code_rule_dsl::logic::rr::graph_digest(text) {
            hash.update(graph);
        }
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
    /// The `ast` atoms' matchers, by relation (`matchers::lower`, decision 0106).
    matchers: Vec<(String, String, Vec<String>)>,
    demanded: Vec<String>,
    pushed: Vec<String>,
    /// The exports handed on as parts (decision 0099, `file_local::factorize_exports`), a line each.
    factorized: Vec<String>,
    located: std::result::Result<Vec<String>, Vec<String>>,
    merged: (usize, usize),
    split: crate::code_rule_dsl::typing::file_local::Programs,
    interned: Vec<(Vec<String>, Vec<u32>)>,
}

/// A measurement (dev only, `RERAIL_SPLIT_DUMP=<file>`): each cross-file relation of the split, by how its rules read
/// other files, one line each: `relation  own  inherited  output|-  rule`. A rule's own class is the first that holds:
/// `recursive` (its stratum reads itself), `total` (an aggregate with no group variable), `grouped` (an aggregate by
/// a key that is not one document), `negation` (a negated atom on another or any document), `lookup` (atoms on two
/// or more documents, joined by values), `same-unit` (every atom on one document), `config` (no document at all).
/// A relation inherits the worst class of the relations its rules read.
fn dump_split(path: &std::path::Path, split: &crate::code_rule_dsl::typing::file_local::Programs) {
    use front::{Literal, Term};
    use std::io::Write;
    const ORDER: [&str; 10] = ["config", "same-unit", "union", "recursive-local", "lookup", "exists", "grouped", "total", "recursive", "-"];
    let rank = |c: &str| ORDER.iter().position(|o| *o == c).unwrap_or(0);
    let cross = &split.cross;
    let Ok((sccs, _)) = front::strata(cross) else { return };
    let scc_of = |rel: &str| sccs.iter().find(|s| s.contains(rel));
    let derived: BTreeSet<String> = cross.rules.iter().map(|r| r.head.rel.to_string()).collect();
    // The columns of a relation that name a document or a node of one (a node id holds its document).
    let carriers = |rel: &str| -> Vec<usize> {
        let mut c: BTreeSet<usize> = split.same_doc.get(rel).or_else(|| split.locations.get(rel)).cloned().unwrap_or_default().into_iter().collect();
        if let Some(cols) = cross.decls.get(rel) {
            c.extend(cols.iter().enumerate().filter(|(_, (_, t))| t == "node").map(|(i, _)| i));
        }
        c.into_iter().collect()
    };
    // Whether a relation's carriers are always on one document: known for base and file-local relations (their
    // same-document columns), and for derived ones the greatest fixpoint of their rules keeping them so.
    let one_doc_known = |rel: &str| !derived.contains(rel) && split.same_doc.contains_key(rel);
    // The classes of a rule's carrier variables, joined where an atom on one document holds both; `None` when some
    // literal is on any document (a wildcard or constant in every carrier).
    let classes = |rule: &front::Rule, unit: &BTreeSet<String>| -> Option<usize> {
        let mut parent: HashMap<String, String> = HashMap::new();
        fn find(p: &mut HashMap<String, String>, x: &str) -> String {
            let up = p.get(x).cloned().unwrap_or_else(|| x.to_owned());
            if up == x {
                p.insert(x.to_owned(), up.clone());
                return up;
            }
            let r = find(p, &up);
            p.insert(x.to_owned(), r.clone());
            r
        }
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for l in &rule.body {
            let Some(a) = l.atom() else { continue };
            let vars: Vec<String> = carriers(&a.rel).into_iter().filter_map(|c| a.args.get(c)).filter_map(|t| if let Term::Var(v) = t { Some(v.clone()) } else { None }).collect();
            if vars.is_empty() {
                if !carriers(&a.rel).is_empty() {
                    return None;
                }
                continue;
            }
            seen.extend(vars.iter().cloned());
            if one_doc_known(&a.rel) || unit.contains(a.rel.as_str()) {
                let r0 = find(&mut parent, &vars[0]);
                for v in &vars[1..] {
                    let r = find(&mut parent, v);
                    parent.insert(r, r0.clone());
                }
            }
        }
        // `$a = $b` names one value twice.
        for l in &rule.body {
            if let Literal::Cmp { l: Term::Var(a), op: "=", r: Term::Var(b) } = l {
                let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
                parent.insert(ra, rb);
            }
        }
        let roots: BTreeSet<String> = seen.iter().map(|v| find(&mut parent, v)).collect();
        Some(roots.len())
    };
    let mut unit: BTreeSet<String> = derived.clone();
    loop {
        let before = unit.len();
        let gone: Vec<String> = cross.rules.iter().filter(|r| unit.contains(r.head.rel.as_str()) && classes(r, &unit).is_none_or(|n| n > 1)).map(|r| r.head.rel.to_string()).collect();
        for g in gone {
            unit.remove(&g);
        }
        if unit.len() == before {
            break;
        }
    }
    let own = |rule: &front::Rule| -> &'static str {
        let scc = scc_of(&rule.head.rel);
        let recursive = rule.body.iter().filter_map(|l| l.atom()).any(|a| scc.is_some_and(|s| s.contains(a.rel.as_str())));
        let n = classes(rule, &unit);
        let one = n.is_some_and(|n| n <= 1);
        let others = |k: usize| -> BTreeSet<String> {
            let mut v = rule.head.vars();
            for (j, l) in rule.body.iter().enumerate() {
                if j != k {
                    v.extend(l.vars());
                }
            }
            v
        };
        // One positive atom with its documents projected away: a union of each document's rows.
        let union = matches!(rule.body.as_slice(), [Literal::Atom(a)] if !a.neg && !carriers(&a.rel).is_empty() && carriers(&a.rel).iter().all(|&c| matches!(a.args.get(c), Some(Term::Wild))));
        let mut worst = if n == Some(0) { "config" } else if one { "same-unit" } else if union { "union" } else { "lookup" };
        let mut bump = |c: &'static str, worst: &mut &'static str| {
            if rank(c) > rank(*worst) {
                *worst = c;
            }
        };
        for (k, l) in rule.body.iter().enumerate() {
            match l {
                Literal::Agg { var, over, atom, .. } if !one => {
                    let mut g = atom.vars();
                    g.remove(var);
                    if let Some(o) = over {
                        g.remove(o);
                    }
                    bump(if g.is_empty() { "total" } else { "grouped" }, &mut worst);
                }
                Literal::Atom(a) if !carriers(&a.rel).is_empty() && a.vars().is_disjoint(&others(k)) => bump("exists", &mut worst),
                _ => {}
            }
        }
        if recursive {
            if rank(worst) <= rank("same-unit") { "recursive-local" } else { "recursive" }
        } else {
            worst
        }
    };
    // Each relation's own class, then inherited through the relations it reads, in stratum order.
    let mut class: HashMap<String, (&'static str, &'static str)> = HashMap::new();
    let by_stratum = front::rules_by_stratum(cross, &sccs);
    for (scc, rules) in sccs.iter().zip(&by_stratum) {
        let mut own_worst = "config";
        let mut inherited = "config";
        let mut looped = false;
        for rule in rules.iter().copied() {
            let o = own(rule);
            looped |= o.starts_with("recursive");
            let o = if o == "recursive-local" { "same-unit" } else { o };
            if rank(o) > rank(own_worst) {
                own_worst = o;
            }
            for a in rule.body.iter().filter_map(|l| l.atom()) {
                if let Some((_, i)) = class.get(a.rel.as_str())
                    && rank(i) > rank(inherited)
                {
                    inherited = i;
                }
            }
        }
        // A recursive stratum is local when every rule of it stays on one document.
        if looped {
            own_worst = if rank(own_worst) <= rank("same-unit") { "recursive-local" } else { "recursive" };
        }
        let inherited = if rank(own_worst) > rank(inherited) { own_worst } else { inherited };
        for rel in scc {
            class.insert(rel.to_string(), (own_worst, inherited));
        }
    }
    let mut out = String::new();
    for rule in &cross.rules {
        let (o, i) = class.get(rule.head.rel.as_str()).copied().unwrap_or(("config", "config"));
        let output = cross.outputs.iter().any(|x| x == &rule.head.rel);
        out.push_str(&format!("{}\t{}\t{}\t{}\t{}\n", rule.head.rel, o, i, if output { "output" } else { "-" }, rule.to_string().replace('\n', " ")));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = f.write_all(out.as_bytes());
    }
}

/// The prepared programs kept in the process, by program key, given to `f`: a few of each, one per store whose atoms
/// it holds (`interned`), so a process serving two stores (a session's and another run's) does not prepare again on
/// each switch. A test's are its own thread's: tests run side by side, and one filling the cache clears it
/// (`PREPARED_KEPT`) under another that expects its program kept (`prepared::tests`).
fn with_prepared<R>(f: impl FnOnce(&mut HashMap<[u8; 32], Vec<Arc<Prepared>>>) -> R) -> R {
    #[cfg(not(any(test, feature = "test-hooks")))]
    {
        static PREPARED: std::sync::LazyLock<std::sync::Mutex<HashMap<[u8; 32], Vec<Arc<Prepared>>>>> = std::sync::LazyLock::new(Default::default);
        f(&mut PREPARED.lock().expect("the prepared programs"))
    }
    #[cfg(any(test, feature = "test-hooks"))]
    {
        thread_local! {
            static PREPARED: std::cell::RefCell<HashMap<[u8; 32], Vec<Arc<Prepared>>>> = std::cell::RefCell::default();
        }
        PREPARED.with(|kept| f(&mut kept.borrow_mut()))
    }
}

/// A few prepared programs at a time are kept.
const PREPARED_KEPT: usize = 16;
/// And a few stores' of each.
const PREPARED_STORES: usize = 4;

/// Keeps `p`, prepared under `key`, first among that key's.
fn keep_prepared(key: [u8; 32], p: Arc<Prepared>) {
    with_prepared(|kept| {
        if !kept.contains_key(&key) && kept.len() >= PREPARED_KEPT {
            kept.clear();
        }
        let all = kept.entry(key).or_default();
        all.retain(|q| q.interned != p.interned);
        all.insert(0, p);
        all.truncate(PREPARED_STORES);
    });
}

fn combine_texts(programs: &[&str], extra: &str) -> Result<front::Program> {
    use front::{Literal, Program};
    // A program reads only relations it declares: an error here, not the planner's panic later (`front::declared`).
    let parse = |text: &str| -> Result<Program> {
        let prog = crate::code_rule_dsl::logic::rr::parse(&if extra.is_empty() { text.to_owned() } else { format!("{text}\n{extra}") })?;
        front::declared(&prog)?;
        Ok(prog)
    };
    let parsing = crate::code_rule_dsl::logic::profile::span(|| format!("parse each program ({})", programs.len()));
    // Each text read once, for its modules and its program, when the two are of the same text (no `extra`).
    let mut texts: Vec<Option<crate::code_rule_dsl::logic::rr::Text>> = if programs.len() > 1 && extra.is_empty() {
        programs.iter().map(|text| crate::code_rule_dsl::logic::rr::read_text(text).map(Some).map_err(|e| in_policy(text, e))).collect::<std::result::Result<_, _>>()?
    } else {
        programs.iter().map(|_| None).collect()
    };
    // Each program's library modules (decision 0089), when it names them by `#include`.
    let modular: Vec<Option<crate::code_rule_dsl::logic::rr::Modules>> = if programs.len() > 1 {
        programs
            .iter()
            .zip(&texts)
            .map(|(text, read)| match read {
                Some(read) => Ok(read.modules()),
                None => crate::code_rule_dsl::logic::rr::modules(text),
            })
            .collect::<std::result::Result<_, _>>()?
    } else {
        vec![None]
    };
    let copied = std::env::var("RERAIL_LIBRARIES").is_ok_and(|v| v == "copied");
    let core = crate::code_rule_dsl::logic::rr::core_names();
    // A program that adds to no library relation is read with its libraries' declarations only: the run holds their
    // rules once (§5). One that asks (§3) is read whole, since its cone is copied from what it read.
    let asks = |modules: &crate::code_rule_dsl::logic::rr::Modules| {
        modules.heads.iter().any(|h| !core.contains(h) && modules.files.iter().any(|(_, _, names)| names.contains(h)))
    };
    let own_only: Vec<bool> = modular.iter().map(|m| !copied && m.as_ref().is_some_and(|m| !asks(m))).collect();
    let mut parsed = programs
        .iter()
        .zip(&own_only)
        .zip(texts.iter_mut())
        .map(|((text, &own_only), read)| {
            if let Some(read) = read.take() {
                let prog = read.lower(!own_only)?;
                front::declared(&prog)?;
                return Ok(prog);
            }
            if !own_only {
                return parse(text);
            }
            let prog = crate::code_rule_dsl::logic::rr::parse_own(&if extra.is_empty() { text.to_string() } else { format!("{text}\n{extra}") })?;
            front::declared(&prog)?;
            Ok(prog)
        })
        .collect::<std::result::Result<Vec<_>, Error>>()?;
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
    drop(parsing);
    let makes_guests = parsed.iter().any(|p| p.rules.iter().any(|r| guests::REQUESTS.contains(&r.head.rel.as_str())));
    let bounds: Vec<u32> = parsed.iter().map(hiding::bound).collect();
    // The libraries once (decision 0089): two or more programs name theirs as modules, and all are read under one
    // bound of the documents they show, so the library's relations are the same for each.
    // `RERAIL_LIBRARIES=copied` copies them into each program, as before, to compare.
    let shared = !copied && modular.iter().flatten().count() > 1 && bounds.windows(2).all(|w| w[0] == w[1]);
    // Not held once after all: a program read without its libraries' rules is read whole.
    if !shared {
        for ((prog, text), &own_only) in parsed.iter_mut().zip(programs).zip(&own_only) {
            if own_only {
                *prog = parse(text)?;
            }
        }
    }
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
    // A program's request (§3): its key's number and the relations it holds, named `q<k>_` for every asker.
    let mut requests: Vec<Option<(usize, BTreeSet<String>)>> = vec![None; parsed.len()];
    if shared {
        let _library = crate::code_rule_dsl::logic::profile::span(|| "the libraries, once (decision 0089)".into());
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
        // Each program's request (§3): the cone's library relations and the program's own relations they read, held
        // once per distinct key. `RERAIL_REQUESTS=copied` keeps one copy per program, to compare.
        let per_program = std::env::var("RERAIL_REQUESTS").is_ok_and(|v| v == "copied");
        let mut keys: BTreeMap<[u8; 32], usize> = BTreeMap::new();
        let mut copies: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
        let mut asking = 0;
        for (i, (prog, modules)) in parsed.iter_mut().zip(&modular).enumerate() {
            let Some(modules) = modules else { continue };
            let (dropped, cone) = own_part(prog, modules, core);
            kept += dropped;
            cones += cone.len();
            if cone.is_empty() {
                continue;
            }
            asking += 1;
            let (unit, key) = request_unit(prog, &cone, core, i, per_program);
            let n = keys.len();
            let k = *keys.entry(key).or_insert(n);
            for rel in cone.iter() {
                copies.entry(rel.clone()).or_default().insert(k);
            }
            if k != n {
                // Another program asked the same: read its copy.
                prog.rules.retain(|r| !unit.contains(r.head.rel.as_str()));
                prog.decls.retain(|r, _| !unit.contains(r));
                prog.facts.retain(|r, _| !unit.contains(r));
                prog.natives.retain(|r, _| !unit.contains(r));
                prog.lattice.retain(|r, _| !unit.contains(r));
                prog.nullary.retain(|r| !unit.contains(r));
            }
            requests[i] = Some((k, unit));
        }
        let rules = library.as_ref().map_or(0, |l| l.rules.len());
        crate::code_rule_dsl::logic::profile::decision("library relations", || {
            format!("{rules} library rules once; {kept} relations taken from the library instead of a program's copy; {cones} cone relations in {asking} programs' requests (§3)")
        });
        crate::code_rule_dsl::logic::profile::decision("requests (decision 0089 §3)", || {
            let most: Vec<String> = {
                let mut by: Vec<(&String, usize)> = copies.iter().map(|(r, ks)| (r, ks.len())).collect();
                by.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
                by.iter().take(8).map(|(r, n)| format!("{r} {n}")).collect()
            };
            format!("{} distinct requests of {asking} asking programs{}; copies per relation: {}", keys.len(), if per_program { " (RERAIL_REQUESTS=copied)" } else { "" }, most.join(", "))
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
        // The library's relations keep their names (decision 0089 §2). Named repository readers belong to their
        // policy (`own_names`): equal relation names alone do not mean equal input files or selectors.
        let own: BTreeSet<String> = if unit.is_some() { own_names(&prog, core) } else { BTreeSet::new() };
        let request = unit.and_then(|u| requests[u].as_ref());
        // The library held once is no policy's: canonical by construction, made over every document read (0091 §3).
        if unit.is_some() {
            policies.push(scoping::Policy { prefix: format!("p{i}_"), own: own.iter().map(|r| format!("p{i}_{r}")).collect(), named, library, scope });
        }
        let rename = |rel: &str| -> String {
            if let Some((k, held)) = request
                && held.contains(rel)
            {
                return format!("q{k}_{rel}");
            }
            if own.contains(rel) {
                return format!("p{i}_{rel}");
            }
            match rel.strip_prefix(axis).and_then(|set| set_of.get(set)) {
                Some(shared) => format!("{axis}{shared}"),
                None => rel.to_owned(),
            }
        };
        let atom = |a: &mut front::Atom| a.rel = rename(&a.rel).into();
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
        for (rel, words) in prog.qualifiers {
            all.qualifiers.entry(rename(&rel)).or_default().extend(words);
        }
        for (rel, cols) in prog.tapes {
            all.tapes.entry(rename(&rel)).or_default().extend(cols);
        }
        for (rel, mut native) in prog.natives {
            native.rename_reads(rename);
            all.natives.insert(rename(&rel), native);
        }
        // A rule `core.rr` gives (its head is the engine's) is taken once.
        for rule in prog.rules {
            if !(core.contains(rule.head.rel.as_str()) && all.rules.contains(&rule)) {
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
    // core.rr's `kind(n, k)` and `docof(n, d)` are projections of `node` in a part that reads them (`rr::lower`'s
    // `projections`) and inputs in one that does not: once one part projects them, they are no inputs of the combined
    // program. Both would give `kind` two domains.
    let projected: BTreeSet<&str> = all.rules.iter().filter(|r| r.body.len() == 1 && r.body[0].atom().is_some_and(|a| a.rel == "node")).map(|r| r.head.rel.as_str()).filter(|rel| matches!(*rel, "kind" | "docof")).collect();
    let projected: BTreeSet<String> = projected.into_iter().map(str::to_owned).collect();
    all.inputs.retain(|rel| !projected.contains(rel));
    // The helpers' columns, typed by the library's declarations, which each program was lowered without.
    crate::code_rule_dsl::logic::rr::retype_helpers(&mut all);
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

/// core.rr's functors only CPython gives, each of a node, and the relations the batches bind them as
/// (`facts::vtab::syntax::cpython`): `@py_unparse(n)` is the `s` of `cpython_unparse(n, s)`. `@unparse` is decision
/// 0058's, a tape's text (`rr::lower`, `unparse_of`), not intercepted here.
const CPYTHON_FUNCTORS: [(&str, &str); 2] = [("py_unparse", crate::code_rule_dsl::facts::core::cpython_relations::UNPARSE), ("ast_dump", crate::code_rule_dsl::facts::core::cpython_relations::AST_DUMP)];

/// Each call of a functor only CPython gives (`CPYTHON_FUNCTORS`) in a rule, lowered to a variable its relation binds
/// in the rule's body: `v(@sha256(@ast_dump(n))) :- kind(n, "call").` is `v(@sha256(x)) :- kind(n, "call"),
/// cpython_ast_dump(n, x).` (one variable per functor and argument). Where the node has no value (no `ast` node of its
/// kind at its span, or CPython raises) the rule has no row, as a functor's undefined value gives none. A call in an
/// aggregate's atom is refused: its relation would have to be read inside the aggregate.
fn cpython_functors(prog: &mut front::Program) -> Result<()> {
    fn lift(t: &mut front::Term, bound: &mut Vec<(&'static str, front::Term, String)>) {
        let front::Term::Call(f, args) = t else { return };
        args.iter_mut().for_each(|a| lift(a, bound));
        let (Some(&(_, rel)), [arg]) = (CPYTHON_FUNCTORS.iter().find(|(g, _)| g == f), args.as_slice()) else { return };
        let var = match bound.iter().find(|(r, a, _)| *r == rel && a == arg) {
            Some((_, _, v)) => v.clone(),
            None => {
                let v = format!("{rel}_{}", bound.len());
                bound.push((rel, arg.clone(), v.clone()));
                v
            }
        };
        *t = front::Term::Var(var);
    }
    fn calls(t: &front::Term) -> bool {
        matches!(t, front::Term::Call(f, args) if CPYTHON_FUNCTORS.iter().any(|(g, _)| g == f) || args.iter().any(calls))
    }
    let mut used = BTreeSet::new();
    for rule in &mut prog.rules {
        let mut bound = Vec::new();
        for l in &mut rule.body {
            match l {
                front::Literal::Atom(a) => a.args.iter_mut().for_each(|t| lift(t, &mut bound)),
                front::Literal::Cmp { l, r, .. } => {
                    lift(l, &mut bound);
                    lift(r, &mut bound);
                }
                front::Literal::Agg { atom, .. } if atom.args.iter().any(calls) => {
                    return Err(Error::Plan(format!("@py_unparse or @ast_dump in an aggregate over {}: bind it to a variable outside the aggregate", atom.rel)));
                }
                _ => {}
            }
        }
        rule.head.args.iter_mut().for_each(|t| lift(t, &mut bound));
        for (rel, arg, var) in bound {
            used.insert(rel);
            rule.body.push(front::Literal::Atom(front::Atom { rel: rel.to_owned().into(), args: vec![arg, front::Term::Var(var)], neg: false, gated: false }));
        }
    }
    for rel in used {
        let cols = crate::code_rule_dsl::facts::core::cpython_relations::columns(rel);
        prog.decls.entry(rel.to_owned()).or_insert_with(|| vec![(cols[0].to_owned(), "node".to_owned()), (cols[1].to_owned(), "sym".to_owned())]);
    }
    Ok(())
}

/// What the converter gives an authored file to read (`logic::convert`, test-only): the scope, the report gate, the
/// options, and the tape's syntax verdict (`tape_syntax_error(d)`: core.rr's `syntax_error` is the engine's, decision
/// 0047). Here so the runner, which is not test-only, can tell them from unbound core relations.
pub const GIVEN: [&str; 8] = ["in_scope", "out_scope", "report_when_gt", "option_on", "option_item", "option_value", "option_json", "tape_syntax_error"];

/// Fails, naming it, where `prog` reads one of core.rr's relations that nothing derives and the runner does not bind
/// (`value`, `str_part`, …): the planner would otherwise fail on its missing statistics (`no statistics for
/// value.c0`). A program's own inputs are its business (facts it is given later), so are the facts the converter
/// gives (`GIVEN`: core.rr's `option_item` among them, empty for an option of no items), and the relations not made
/// yet are empty (`NOT_MADE`, O76).
fn bound(prog: &front::Program) -> Result<()> {
    let derived: BTreeSet<&str> = prog.rules.iter().map(|r| r.head.rel.as_str()).chain(prog.facts.keys().map(String::as_str)).collect();
    let cpython = "CPython (core.rr: the isolated `ast.parse` worker), which the corpus runner does not run";
    // `env` is the run's settings (`tools::with_settings`): facts when there are any, else empty. The world's
    // relations (`dir_entry`, `file_line`, …, and `loaded`) are answered by the runner (`world`).
    let world = |rel: &str| rel == world::LOADED || crate::code_rule_dsl::logic::rr::WORLD.iter().any(|(r, _)| *r == rel);
    let given = |rel: &str| GIVEN.contains(&rel) || rel == "env" || world(rel) || NOT_MADE.iter().any(|(r, _)| *r == rel);
    // core.rr's literal parts are made in core.rr's shape only (`syntax::core_rr`): a program that declares its base
    // relations itself (T's shape) has none.
    let t_shape = syntax::Projection::of(prog, BTreeSet::new()).shape == syntax::Shape::T;
    let parts = |rel: &str| t_shape && rel.starts_with("str_part");
    for rule in &prog.rules {
        for a in rule.body.iter().filter_map(front::Literal::atom) {
            let rel = a.rel.as_str();
            if !parts(rel) && (derived.contains(rel) || super::super::sql::select::input_columns(rel).is_some() || !crate::code_rule_dsl::logic::rr::is_core(rel) || given(rel)) {
                continue;
            }
            let why = match rel {
                _ if parts(rel) => format!("core.rr's shape makes it (a program that includes core.rr); without it, it needs {cpython}"),
                "list_len" => "it counts a pattern's list variable ($$$X), and patterns do not lower".to_owned(),
                _ => "the corpus runner has no producer of it (the relations it binds: candidates/datalog/authored/README.md)".to_owned(),
            };
            return Err(Error::Plan(format!("{rel} is read, and nothing derives it or binds it: {why}")));
        }
    }
    Ok(())
}

/// Of a program whose text names its libraries as modules (decision 0089 §5), the part the run's library does not
/// hold, taken out of `prog`: what is left is the program's own. A library relation goes to the library unless the
/// program's own clauses add to it or it reads one they add to, through any relation (the request's cone, §3): the
/// program keeps its own copy of those, as before. A relation lowering made for library rules alone (every reader
/// of it the library's) goes with them. Returns the relations taken out, and the cone's.
fn own_part(prog: &mut front::Program, modules: &crate::code_rule_dsl::logic::rr::Modules, core: &BTreeSet<String>) -> (usize, BTreeSet<String>) {
    use front::Literal;
    // An input the program reads (`within`, which lowering keeps where a rule reads it) is a base relation, the run's:
    // its declaration stays with the program.
    let library: BTreeSet<&str> =
        modules.files.iter().flat_map(|(_, _, names)| names).map(String::as_str).filter(|r| !core.contains(*r) && !prog.inputs.iter().any(|i| i == r)).collect();
    let reads = |r: &front::Rule| -> Vec<String> {
        r.body
            .iter()
            .filter_map(|l| match l {
                Literal::Atom(a) | Literal::Agg { atom: a, .. } => Some(a.rel.to_string()),
                _ => None,
            })
            .collect()
    };
    let edges: Vec<(String, Vec<String>)> = prog
        .rules
        .iter()
        .map(|r| (r.head.rel.to_string(), reads(r)))
        .chain(prog.natives.iter().map(|(rel, n)| (rel.to_string(), n.reads().into_iter().map(str::to_owned).collect())))
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
        .map(String::as_str)
        .chain(prog.rules.iter().map(|r| r.head.rel.as_str()))
        .chain(prog.natives.keys().map(String::as_str))
        .filter(|r| !library.contains(*r) && !core.contains(*r) && !cone.contains(*r))
        .map(str::to_owned)
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
    prog.rules.retain(|r| !part.contains(r.head.rel.as_str()));
    prog.decls.retain(|r, _| !part.contains(r));
    prog.facts.retain(|r, _| !part.contains(r));
    prog.natives.retain(|r, _| !part.contains(r));
    prog.lattice.retain(|r, _| !part.contains(r));
    prog.nullary.retain(|r| !part.contains(r));
    let cones = cone.into_iter().filter(|r| library.contains(r.as_str())).collect();
    (taken, cones)
}

/// A program's relations that take its prefix (`p<i>_`) when programs are combined: what it declares, holds facts of
/// or derives, other than the base inputs, the shared outputs, `core.rr`'s (decision 0047), the bridged and the
/// guests' shared relations. A repository file a policy reads (`.input r(IO=…)`) is its own.
fn own_names(prog: &front::Program, core: &BTreeSet<String>) -> BTreeSet<String> {
    let shared_outputs = ["violation", "rr_diagnostic"];
    let file_inputs = file_inputs(prog);
    prog.decls
        .keys()
        .map(String::as_str)
        .chain(prog.facts.keys().map(String::as_str))
        .chain(prog.rules.iter().map(|r| r.head.rel.as_str()))
        .filter(|r| (!prog.inputs.iter().any(|i| i == r) || file_inputs.contains(*r)) && !shared_outputs.contains(r) && !core.contains(*r) && !bridge::is_bridged(r) && !phases::is_phased(r) && !guests::SHARED.contains(r))
        .map(str::to_owned)
        .collect()
}

/// The relations a policy reads from repository files (`.input r(IO=…)`): each is the policy's own.
fn file_inputs(prog: &front::Program) -> BTreeSet<String> {
    prog.settings.iter().filter(|(key, _)| key == "input").filter_map(|(_, text)| text.split_once('(').map(|(rel, _)| rel.to_owned())).collect()
}

/// A program's request (decision 0089 §3): the library relations of its cone (`cone`) and every relation of its own
/// they read, through rules and natives; and the request's key, the digest of what those relations are. Two programs
/// with one key ask the same, so they read one copy. A request that reads one of the program's own relations it does
/// not hold, or a repository file, is the program's alone (its index is in the key), as is every request when
/// `alone`.
fn request_unit(prog: &front::Program, cone: &BTreeSet<String>, core: &BTreeSet<String>, i: usize, alone: bool) -> (BTreeSet<String>, [u8; 32]) {
    use front::Literal;
    use sha2::Digest as _;
    let own = own_names(prog, core);
    let files = file_inputs(prog);
    let mut unit: BTreeSet<String> = cone.iter().filter(|r| own.contains(*r)).cloned().collect();
    let mut work: Vec<String> = unit.iter().cloned().collect();
    let mut outside = false;
    while let Some(rel) = work.pop() {
        let mut reads: Vec<String> = Vec::new();
        for r in prog.rules.iter().filter(|r| r.head.rel == rel) {
            for l in &r.body {
                if let Literal::Atom(a) | Literal::Agg { atom: a, .. } = l {
                    reads.push(a.rel.to_string());
                }
            }
        }
        if let Some(native) = prog.natives.get(&rel) {
            reads.extend(native.reads().into_iter().map(str::to_owned));
        }
        for r in reads {
            if files.contains(&r) {
                outside = true;
            }
            if own.contains(&r) && unit.insert(r.clone()) {
                work.push(r);
            }
        }
    }
    let mut hash = sha2::Sha256::new();
    if alone || outside {
        hash.update(format!("program {i}\n"));
    }
    let mut rules: Vec<String> = prog.rules.iter().filter(|r| unit.contains(r.head.rel.as_str())).map(|r| format!("{r:?}")).collect();
    rules.sort();
    for rule in rules {
        hash.update(rule);
        hash.update("\n");
    }
    for rel in &unit {
        hash.update(format!(
            "{rel} {:?} {:?} {:?} {:?} {}\n",
            prog.decls.get(rel),
            prog.facts.get(rel),
            prog.natives.get(rel),
            prog.lattice.get(rel),
            prog.nullary.contains(rel)
        ));
    }
    (unit, hash.finalize().into())
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
        // Held by the store's writer, behind the run (decision 0104), before any memo row the run queues after it.
        let (held, by) = (plans.to_vec(), owner.clone());
        store.write_back(
            "the run's hold on its plans",
            0,
            Vec::new(),
            Box::new(move |s| held.iter().try_for_each(|plan| s.hold_corpus_plan(*plan, &by, lease))),
        );
        store.hold_sources(&owner, lease)?;
        Ok(HeldPlan { store, owner })
    }
}

impl Drop for HeldPlan<'_> {
    fn drop(&mut self) {
        // The manifest names the plan now, or the run failed: either way the hold is done, released by the writer
        // after the manifest (queued before). A failed release lapses.
        self.store.release_holder(&self.owner);
        let owner = self.owner.clone();
        self.store.write_back("the run's hold released", 0, Vec::new(), Box::new(move |s| s.release_pins(&owner).map(|_| ())));
    }
}

/// Sources a queued write stores (decision 0102), for the manifest queued after it.
type Late = Arc<std::sync::Mutex<Vec<crate::content_store::SourceId>>>;

/// The store's collection (decision 0053), started once the run's manifest is written: `join` waits for it.
pub(in crate::code_rule_dsl) struct Collecting(std::sync::mpsc::Receiver<Option<std::thread::JoinHandle<(usize, std::result::Result<String, String>)>>>);

impl Collecting {
    /// The passes and their outcome, once the manifest is written and the passes ran; `None` when no pass was due.
    pub(in crate::code_rule_dsl) fn join(self) -> std::thread::Result<Option<(usize, std::result::Result<String, String>)>> {
        match self.0.recv() {
            Ok(Some(thread)) => thread.join().map(Some),
            _ => Ok(None),
        }
    }
}

/// Records the run's manifest and relations, then starts collection if a pass is due: queued behind the run's
/// writes (decision 0102), so the manifest names only what is written, and the lease on collection is let go
/// once it is.
fn record_run(store: &ContentStore, root: &Path, sources: &[crate::content_store::SourceId], program: &[u8; 32], also: &[[u8; 32]], relations: &[(String, String)], late: &Late, reading: Option<crate::content_store::CollectionLease>, outer: bool) -> Result<Option<Collecting>> {
    memo::off();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let (root, sources, also, relations) = (root.to_string_lossy().into_owned(), sources.to_vec(), also.to_vec(), relations.to_vec());
    let plan = memo::plan_digest(program);
    // A run inside another (a phase's, as phase 1's while the outer run lists its files; the constants') leaves
    // collection to the outer run, which collects after its last wave: here it would run beside the outer run's waves.
    let collect = !std::env::var("RERAIL_CORPUS_GC").is_ok_and(|v| v == "off") && outer;
    let mut policy = crate::content_store::CorpusPolicy::default();
    if let Some(mb) = std::env::var("RERAIL_CORPUS_HISTORY_MB").ok().and_then(|v| v.parse::<u64>().ok()) {
        (policy.high_water, policy.low_water) = (mb << 20, (mb << 20) / 4 * 3);
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let path = store.database_path().to_path_buf();
    let late = Arc::clone(late);
    let mut reading = reading;
    // Alone: collection, on its own connection, reads the manifest it starts after.
    store.write_back_alone(
        "the run's manifest",
        0,
        Vec::new(),
        Box::new(move |store| {
            // The sources of the writes queued before this one, written now (run again if the store was busy).
            let mut sources = sources.clone();
            sources.extend(late.lock().expect("the late sources").iter().copied());
            let recorded = store
                .record_corpus_run_naming(&root, &sources, crate::code_rule_dsl::build_ids::grammar_version(), i64::from(crate::code_rule_dsl::tape::TAPE_FORMAT_VERSION), plan, &also, now)
                .and_then(|_| store.record_corpus_relations(plan, &relations));
            // The manifest now protects the sources. Release before collection, including warm runs
            // that must resume a pass deferred by another active reader.
            drop(reading.take());
            recorded?;
            // No thread, no second connection, unless a pass has work (a run of fresh roots decides rarely).
            if !collect || !store.corpus_pass_wanted(&policy, now)? {
                let _ = tx.send(None);
                return Ok(());
            }
            let path = path.clone();
            let _ = tx.send(Some(std::thread::spawn(move || {
        // Collection works behind the run, as the writer does: on its own connection, beside the waves, which read
        // nothing it writes and wait for none of it (decision 0053). Its statements are not a wave's.
        crate::code_rule_dsl::logic::profile::behind_the_run();
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
            })));
            Ok(())
        }),
    );
    Ok(Some(Collecting(rx)))
}

#[cfg(any(test, feature = "test-hooks"))]
thread_local! {
    /// A test's own store, so that another test's collection in the shared default store does not evict its rows.
    pub static TEST_TAPE_CACHE: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Held by a test that starts the process's session (`session::start`) and by one whose runs must not see it: every
/// run uses a live session's store.
#[cfg(any(test, feature = "test-hooks"))]
pub static SESSION_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The store the corpus runner keeps tapes in: `RERAIL_TAPE_CACHE`, else one in the temporary
/// directory; `RERAIL_TAPE_CACHE=off` keeps none.
pub(in crate::code_rule_dsl) fn tape_cache() -> std::result::Result<Option<ContentStore>, crate::content_store::StoreError> {
    tape_cache_path().map(|path| ContentStore::open(&path)).transpose()
}

/// Runs `f` with phase 0's observer (`content_store::observe`), kept beside the tape cache (`<tape cache>.files`), or
/// in memory for the process when there is none. Every path the engine reads goes through it: rule files, corpus
/// files, what resolution rules list, read and load.
pub fn observe<T>(f: impl FnOnce(&mut crate::content_store::observe::Observer) -> T) -> T {
    // The engine's tests check paths by their `stat`: the journal's wait for its cookie, made by every run, is the
    // journal's own tests' to measure (`content_store::observe`). A test of the engine that wants it turns it on.
    #[cfg(any(test, feature = "test-hooks"))]
    {
        static OFF: std::sync::Once = std::sync::Once::new();
        OFF.call_once(|| crate::content_store::observe::set_journal(false));
    }
    let store = tape_cache_path().map(|p| crate::content_store::observe::store_beside(&p));
    // Each test thread its own observer, unless the test set its own tape cache: another test's run flushes a shared
    // one's journal, after which it answers for this test's edits until the next flush (decision 0105).
    #[cfg(any(test, feature = "test-hooks"))]
    let store = store.map(|p| {
        if TEST_TAPE_CACHE.with(|t| t.borrow().is_some()) {
            return p;
        }
        let thread: String = format!("{:?}", std::thread::current().id()).chars().filter(char::is_ascii_digit).collect();
        std::path::PathBuf::from(format!("{}.thread{thread}", p.display()))
    });
    crate::content_store::observe::with(store.as_deref(), f)
}

/// Closes a run's connection on a thread of its own: its close frees what the process kept for it (the bound
/// sources, the engine's relations: `vtab::connection_scope`), which is no step's work. Idle, it holds no lock.
fn close_beside(db: Connection) {
    let _ = std::thread::Builder::new().name("rerail-close".into()).spawn(move || drop(db));
}

/// The path `tape_cache` opens (`None`: `RERAIL_TAPE_CACHE=off`).
pub fn tape_cache_path() -> Option<std::path::PathBuf> {
    #[cfg(any(test, feature = "test-hooks"))]
    if let Some(path) = TEST_TAPE_CACHE.with(|p| p.borrow().clone()) {
        return Some(path);
    }
    match std::env::var("RERAIL_TAPE_CACHE") {
        Ok(v) if v == "off" => None,
        Ok(v) => Some(std::path::PathBuf::from(v)),
        #[cfg(any(test, feature = "test-hooks"))]
        Err(_) => Some(test_tape_cache().clone()),
        #[cfg(not(any(test, feature = "test-hooks")))]
        Err(_) => Some(std::env::temp_dir().join("rerail-tape-cache.sqlite")),
    }
}

/// A test process's own tape cache: the machine's is written by every other run on it, so a test's writes could wait
/// out its busy deadline behind them. Removed when the process exits; one a killed process left is removed by the
/// next test process to start.
#[cfg(any(test, feature = "test-hooks"))]
fn test_tape_cache() -> &'static std::path::PathBuf {
    static PATH: std::sync::LazyLock<std::path::PathBuf> = std::sync::LazyLock::new(|| {
        const PREFIX: &str = "rerail-tape-cache-test-";
        let dir = std::env::temp_dir();
        // Another process's, when it is gone.
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(pid) = name.strip_prefix(PREFIX).and_then(|rest| rest.split('.').next()).and_then(|p| p.parse::<i32>().ok()) else { continue };
            // SAFETY: signal 0 checks that the process exists and sends nothing.
            if pid != std::process::id() as i32 && unsafe { libc::kill(pid, 0) } != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        extern "C" fn remove() {
            let own = format!("{PREFIX}{}.", std::process::id());
            for entry in std::fs::read_dir(std::env::temp_dir()).into_iter().flatten().flatten() {
                if entry.file_name().to_string_lossy().starts_with(&own) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        // SAFETY: `remove` is a plain function that runs once, at exit.
        unsafe { libc::atexit(remove) };
        dir.join(format!("{PREFIX}{}.sqlite", std::process::id()))
    });
    &PATH
}

/// `run` with typing inputs and outputs (`typing`: declarations, dependency summaries, summarizing).
#[allow(clippy::too_many_arguments)]
pub fn run_typed(programs: &[&str], root: &Path, t_facts: Option<&Path>, out: &Path, modes: &str, binding: Binding, chunk_nodes: usize, typing: &Typing) -> Result<Report> {
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
pub fn run_with_options(
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
    // Whether the native strata may be placed on the device (`plan::placement`, decision 0095), for this run.
    struct Device(bool);
    impl Drop for Device {
        fn drop(&mut self) {
            super::placement::set_device(self.0);
        }
    }
    let _device = Device(super::placement::set_device(binding == Binding::Metal));
    // The corpus root and the rule library are watched (decision 0105): what the journal says is unchanged under
    // them is not statted.
    let library = crate::code_rule_dsl::logic::rr::library();
    let library = std::fs::canonicalize(&library).unwrap_or(library);
    let before = observe(|o| {
        o.begin();
        o.watch(&[root.to_path_buf(), library]);
        o.counts
    });
    // Phase 0 first: each text's include graph, from `bootstrap.rr`'s run, before anything reads a text (the later
    // reads keep it while its files hold).
    // The engine's own `components.rr` (read by the cross-file memo) is resolved in the same run, so no phase-0 run
    // starts inside this one.
    let seeding = crate::code_rule_dsl::logic::profile::span(|| "phase 0: the texts' include graphs".into());
    let mut seeds: Vec<phase0::Seed> = programs.iter().map(|t| crate::code_rule_dsl::logic::rr::text_seed(t)).collect::<std::result::Result<Vec<_>, _>>()?.into_iter().flatten().collect();
    if !phase0::active() {
        if let Ok(components) = crate::code_rule_dsl::logic::rr::library().join("components.rr").canonicalize() {
            seeds.push(phase0::Seed::File(components));
        }
    }
    if !seeds.is_empty() && !phase0::active() {
        phase0::graphs(&seeds)?;
    }
    drop(seeding);
    // Phase 0's cutoff (`reuse`): a run whose every read is as it was writes the outputs it wrote.
    let reuse = reuse::wanted(options, typing, t_facts);
    let kept = if reuse { reuse::reused(programs, root, out, modes, typing, options) } else { Ok(None) };
    let report = match kept {
        Ok(Some(report)) => Ok(report),
        Err(e) => Err(e),
        Ok(None) => run_observed(programs, root, t_facts, out, modes, binding, chunk_nodes, typing, options).and_then(|report| {
            if reuse {
                reuse::record(programs, root, out, modes, typing, options, &report)?;
            }
            Ok(report)
        }),
    };
    // Phase 0's observations, written once the run has read everything (they may lag the run, never lead it).
    let did = {
        let _s = crate::code_rule_dsl::logic::profile::span(|| "phase 0's observations written".into());
        observe(|o| {
            o.flush();
            o.counts.since(&before)
        })
    };
    // `RERAIL_OBSERVED`: what the run's observer did, on stderr (the profile is written before this).
    if std::env::var_os("RERAIL_OBSERVED").is_some() {
        eprintln!("rerail: files observed: {did}");
    }
    report.map(|mut r| {
        r.files_observed = did;
        r
    })
}
thread_local! {
    /// Runs under way on this thread: a run started inside another (a phase, the constants, phase 0) is nested.
    static RUNS_UNDER_WAY: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// The outermost run under way on this thread, by its token (`run_token`).
    static RUN_TOKEN: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// The outermost runs of the process, numbered from 1.
static RUNS_STARTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The token of the outermost run under way on this thread, which its nested runs share: the frames' spaces count a
/// run once, however many of its programs keep frames in them (`content_store::frames`). `0` outside a run.
pub(in crate::code_rule_dsl) fn run_token() -> u64 {
    RUN_TOKEN.with(std::cell::Cell::get)
}

/// Counts a run under way on this thread until dropped.
struct UnderWay;
impl UnderWay {
    fn enter() -> UnderWay {
        RUNS_UNDER_WAY.with(|n| {
            if n.get() == 0 {
                RUN_TOKEN.with(|t| t.set(RUNS_STARTED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1));
            }
            n.set(n.get() + 1)
        });
        UnderWay
    }
}
impl Drop for UnderWay {
    fn drop(&mut self) {
        RUNS_UNDER_WAY.with(|n| n.set(n.get() - 1));
    }
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
    // The store's writer held from the run's start to its end (decision 0102): the handles the run opens and closes on
    // the way (the combined program's) queue their writes and go, and the run's end waits for every one. Declared
    // first, so dropped last.
    let _writer = if cfg!(feature = "tape-cache") { tape_cache_path().map(|p| crate::content_store::write_back::hold(&p)) } else { None };
    // The run's settings, as `env` facts (t21 E4): `<root>/rerail.env`, then the `env` option's `key=value`s (the
    // command line's `--env`).
    let setting = crate::code_rule_dsl::logic::profile::span(|| "the run's settings".into());
    let flags: Vec<String> = options.get("env").and_then(|v| v.as_array()).into_iter().flatten().filter_map(|v| v.as_str().map(str::to_owned)).collect();
    let settings = crate::code_rule_dsl::logic::settings::settings(root, &flags).map_err(Error::Plan)?;
    let with_settings: Vec<String> = programs.iter().map(|p| crate::code_rule_dsl::logic::settings::with_settings(p, &settings)).collect();
    let programs: &[&str] = &with_settings.iter().map(String::as_str).collect::<Vec<_>>();
    drop(setting);
    // The main thread's own CPU (the chunk builder's jobs' is theirs, below; the jobs the main thread runs for the
    // pool while it waits are not its own, `exec::helped`) and the process's peak RSS.
    let main_usage = || { let (u, s) = thread_usage(); (u - rerail_accel::exec::helped(), s, usage().2) };
    // When the run began: the first phase counts combining.
    let began = main_usage();
    let mut phases: Vec<(&'static str, f64, f64, f64, usize)> = Vec::new();
    let mut phase_instructions: Vec<(&'static str, u64)> = Vec::new();
    let mut last = began;
    let mut last_instructions = crate::code_rule_dsl::logic::settings::process_counters().map_or(0, |c| c.0);
    let mut counted = 0;
    let mut phase = |name: &'static str, statements: usize| {
        let now = main_usage();
        let instructions = crate::code_rule_dsl::logic::settings::process_counters().map_or(0, |c| c.0);
        match phase_instructions.iter_mut().find(|p| p.0 == name) {
            Some(p) => p.1 += instructions - last_instructions,
            None => phase_instructions.push((name, instructions - last_instructions)),
        }
        last_instructions = instructions;
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
    // A run nested in another on this thread (phase 0's, a phase's, the constants', a bridged program's) and phase
    // 0's own run hold it only when no run does: nested, the profile is the outer run's, and such a run neither
    // resets nor writes it. Waiting for it would wait for the outer run, which waits for this one.
    let nested = RUNS_UNDER_WAY.with(std::cell::Cell::get) > 0;
    let _under_way = UnderWay::enter();
    let _profile = if !crate::code_rule_dsl::logic::profile::on() {
        None
    } else if phase0::active() || nested {
        crate::code_rule_dsl::logic::profile::one_run_if_free()
    } else {
        Some(crate::code_rule_dsl::logic::profile::one_run())
    };
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
    // The constants' own run runs no phase: a constant reads no input (`constants::foldable`), and a phase export is
    // one. Run here, a phase took the constants' option for its own, and wrote none of its exports.
    // Phase 1 (`sources.rr`, decision 0104) runs first: it loads the source files and the libraries their imports
    // reach, to the imports' fixed point. Its documents are this run's, and every later phase's run inside it: a phase's
    // own run, phase 0's (another tree) and the constants' (no documents) list none of theirs from it. A run given its
    // files (`inputFiles`, T's facts) has no phase 1.
    let language = prog.settings.iter().find(|(key, _)| key == "language").map(|(_, value)| value.trim().trim_matches('"').to_owned());
    let mut listing: Option<(Vec<String>, &'static str)> = None;
    let mut phase_one_run = None;
    let _phase_one = if t_facts.is_none()
        && options.get("inputFiles").is_none()
        && phases::declared(&prog).is_none()
        && !phases::in_phase()
        && !phase0::active()
        && options.get(constants::OWN).is_none()
        && std::env::var("RERAIL_SOURCES_PHASE").map_or(true, |v| v != "off")
    {
        let found = scoped(corpus_files(root)?, &prog, options);
        let label = corpus_language(language.as_deref(), typing, &found);
        let files = found.of(label);
        // The observer's facts: each source file of the run's grammar, its module, and its content's digest (read
        // only where the observer cannot vouch for it).
        let stated: Vec<(String, String, String, String)> = files
            .iter()
            .filter(|p| crate::code_rule_dsl::language::label_of_path(p) == Some(label))
            .map(|p| (p.clone(), crate::code_rule_dsl::facts::vtab::syntax::module(p), label.to_owned(), world::bytes_digest(&root.join(p))))
            .filter(|(_, _, _, c)| !c.is_empty())
            .collect();
        listing = Some((files, label));
        if stated.is_empty() {
            None
        } else {
            let _s = crate::code_rule_dsl::logic::profile::span(|| "phase 1: the source files".into());
            let (one, run) = phases::sources(&stated, label, root, out, modes, binding, chunk_nodes, typing, options)?;
            let (made, libraries) = (one.documents.len(), one.libraries.len());
            crate::code_rule_dsl::logic::profile::decision("phase 1 (sources.rr)", || {
                format!("{made} of {} source files are its documents; {libraries} library documents the imports reached", stated.len())
            });
            phase_one_run = Some(run);
            Some(phases::PhaseOneHeld::enter(one))
        }
    } else {
        None
    };
    let (phased, mut phase_runs) = if options.get(constants::OWN).is_some() {
        (Default::default(), Vec::new())
    } else {
        phases::facts(&prog, root, out, modes, binding, chunk_nodes, typing, options)?
    };
    phase_runs.splice(0..0, phase_one_run);
    // A phase's exports are the run's inputs, not its facts: their rows are bound as relations once the program is
    // prepared, so an edit that changes them leaves the program, its prepared plans and its keys as they were.
    // The split takes them as constants of the run (`file_local::run_constant`).
    let add_bridged = |prog: &mut front::Program| {
        for (rel, rows) in &bridged {
            prog.facts.entry(rel.clone()).or_default().extend(rows.iter().cloned());
        }
        for rel in phased.keys() {
            if !prog.inputs.contains(rel) {
                prog.inputs.push(rel.clone());
            }
        }
    };
    add_bridged(&mut prog);
    // The Git functors answer from the repository holding this run's root (`strings::git`), for this run only.
    let _git = crate::code_rule_dsl::logic::strings::git::scope(root);
    // The run's constants (`constants`): derived first, over no documents, and given as facts. In the constants' own
    // run, its outputs are those relations.
    let fold_constants = |prog: &mut front::Program| -> Result<Vec<(String, String, String)>> {
        match options.get(constants::OWN).and_then(|v| v.as_array()) {
            Some(wanted) => {
                constants::as_outputs(prog, &wanted.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect::<Vec<_>>());
                Ok(Vec::new())
            }
            None => constants::fold(prog, programs, root, out, modes, binding, chunk_nodes, typing, options),
        }
    };
    phase("combine", 0);
    let mut folded = fold_constants(&mut prog)?;
    phase("constants", 0);
    let t = t_facts.map(|t_facts| Connection::open_with_flags(t_facts, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)).transpose()?;
    // The program's `.language` (above) picks the corpus's files: `.rr` files only when it names `rr` (decision 0061).
    // The library documents phase 1's imports reached, for a program that reads them (`sources__library`): each a
    // loaded document of this run, `(canonical path, module)`.
    let mut libraries: Vec<(String, String)> = Vec::new();
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
                    Some(name) if let Some(label) = crate::code_rule_dsl::language::label_named(name) => label,
                    Some(other) => return Err(Error::Plan(format!("explicit input batch: unsupported corpus language {other:?}"))),
                    None if files.iter().filter(|p| p.ends_with(".rs")).count() > files.len() / 2 => "rust",
                    None => "python",
                };
                (files, label)
            } else {
                let (files, label) = match listing.take() {
                    Some(listed) => listed,
                    None => {
                        let found = scoped(corpus_files(root)?, &prog, options);
                        let label = corpus_language(language.as_deref(), typing, &found);
                        (found.of(label), label)
                    }
                };
                // Phase 1's documents are this run's (above); the constants' own run reads none.
                let one = options.get(constants::OWN).is_none().then(|| phases::phase_one(root, label)).flatten();
                let files = match one {
                    Some(one) => {
                        let made: std::collections::HashSet<&str> = one.documents.iter().map(|(p, _, _)| p.as_str()).collect();
                        // A file phase 1 made no document of (it did not load or parse, or is of no grammar: `.scope
                        // text`) is listed as before, so that the run reports it.
                        let mut listed: Vec<String> = one.documents.iter().map(|(p, _, _)| p.clone()).collect();
                        listed.extend(files.iter().filter(|p| !made.contains(p.as_str())).cloned());
                        listed.sort();
                        if prog.decls.contains_key("sources__library") {
                            libraries = one.libraries.iter().map(|(p, m, _, _)| (p.clone(), m.clone())).collect();
                        }
                        listed
                    }
                    None => files,
                };
                (files, label)
            };
            // The libraries come after the tree's files: a document's key is a function of its path (decision 0052),
            // so a memoised row's node ids stay valid.
            let mut files = files;
            files.extend(libraries.iter().map(|(p, _)| p.clone()));
            let keys = memo::doc_keys(&files);
            (keys.into_iter().zip(files).collect(), HashMap::new(), label)
        }
    };
    // Taken once the files are listed: phase 1, run while they are, is a run of its own, with its session and plans.
    // Plans are kept across this run's chunks (decision 0038), and a session's across its runs (decision 0090 step 5).
    let across = session::generation();
    let _plans = super::KeepPlans::start(across);
    // And the values derived from them (`logic::derived`): each chunk's join specs.
    let _derived = crate::code_rule_dsl::logic::derived::Keep::start(across);
    // A session's run (`session`, decision 0063): its store and its memo, held for the run.
    let mut held = session::current();
    let mut docs = docs;
    // What the run consults (`Report::consulted`): first the workspace documents, when they are files under `root`.
    let mut consulted: Vec<(String, String, String)> =
        if t.is_none() { docs.iter().map(|(_, p)| ("doc".to_owned(), p.clone(), world::bytes_digest(&root.join(p)))).collect() } else { Vec::new() };
    let mut paths: HashMap<i64, String> = docs.iter().cloned().collect();
    // The documents requests loaded (decision 0065 §2a), whose rows are not the run's findings: their keys.
    let mut loaded_keys: std::collections::HashSet<i64> = std::collections::HashSet::new();
    // The library documents phase 1 handed on are loaded documents of this run, named by their modules.
    let given_modules: HashMap<String, Arc<str>> = libraries.iter().map(|(p, m)| (p.clone(), Arc::from(m.as_str()))).collect();
    loaded_keys.extend(docs.iter().filter(|(_, p)| given_modules.contains_key(p)).map(|(d, _)| *d));
    // Declarations and dependency summaries, as `declared_ret` facts (`corpus::typing`).
    let mut typed = TypingReport::default();
    let mut declared = BTreeMap::new();
    if prog.decls.contains_key("declared_ret") {
        let files: Vec<String> = docs.iter().map(|(_, p)| p.clone()).filter(|p| !text_document(p)).collect();
        let (facts, d) = typing::facts(typing, root, &files, &mut typed)?;
        if !facts.is_empty() {
            prog = combine(programs, &facts)?;
            texts = texts_digest(programs, &facts);
            add_bridged(&mut prog);
            folded = fold_constants(&mut prog)?;
        }
        declared = d;
    }
    consulted.extend(folded);
    // Typing may rebuild the combined program. Publish external facts after that rebuild.
    external_inputs(&mut prog, root)?;
    // The repository files, artifacts and base revision the readers read are consulted too (`inputs::still`).
    let read = inputs::take_consulted();
    if t.is_none() {
        consulted.extend(read);
    }
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
    // The run's connections, set up by their owner and passed through (`connections`): a session's are kept from run
    // to run, readied here, before the scratch files are made again; a run of its own opens its own. (A run that
    // fails leaves its session new ones to open.)
    let handed = connections::Handed::take(session.as_ref().map(|s| Arc::clone(&s.connections)))?;
    let own_kept = session.as_ref().map(|s| Arc::clone(&s.own));
    let (path, cross_path) = match &session {
        Some(s) => (s.path.clone(), out.join("cross.sqlite")),
        None => (out.join("store.sqlite"), out.join("cross.sqlite")),
    };
    // A session's store is kept from run to run; the cross-file scratch never is. A failed run leaves its write-ahead
    // log beside the store; reopening a new store over it reads stale pages.
    let scratch = [&path, &cross_path].into_iter().filter(|p| session.is_none() || **p == cross_path);
    for p in scratch.flat_map(|p| ["", "-wal", "-shm"].map(|s| std::path::PathBuf::from(format!("{}{s}", p.display())))) {
        if p.exists() {
            std::fs::remove_file(&p).map_err(|e| Error::Plan(e.to_string()))?;
        }
    }
    // The persistent store: tapes kept across runs, keyed by content (`RERAIL_TAPE_CACHE` names it, `off` parses
    // every file), and the run's atoms and each file's memoised rows (decision 0052): atoms are never given out again,
    // so a memoised row's symbols mean the same in every run. The run's SQL works in `store.sqlite`, on the same
    // atoms. Without the store, or reproducing T, the atoms are the run's own; in a session, the session's store's.
    // Without the `tape-cache` feature (the browser build) there is no store to keep: `tapes` is `None` where the
    // compiler can see it, so the memo, the run record and their zstd frames are not linked.
    let tapes = if cfg!(feature = "tape-cache") { handed.tapes().map_err(|e| Error::Plan(e.to_string()))? } else { &None };
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
    // `shortlex_min<>` orders the run's symbols by their text: the store's atoms (decision 0108).
    crate::code_rule_dsl::logic::native::declare("shortlex_min", Arc::new(crate::code_rule_dsl::logic::native::ShortlexMin(store.atoms_handle())));
    let db = handed.run(&path)?;
    // The run's databases are scratch, made again by every run (removed above): a commit need not reach the disk. Set
    // when SQL first runs on it (`vtab::setup_now`).
    crate::code_rule_dsl::facts::vtab::defer_setup(&db, "PRAGMA synchronous=OFF;");
    super::ready_native(&db)?;
    // The cost statistics read before the first wave (decision 0104), the executor's prices fitted from them.
    crate::code_rule_dsl::logic::costs::start();
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
    // A run that reads the file system answers a load with the loaded documents' rows of each export its cone of
    // requests reads, found by the export's location column (decision 0065 §2a). Factorizing changes which relations
    // are exports: a reader of a factorized export can itself become file-local and exported with no location (phase
    // 0's `p0_file_t_shape(f) :- p0_file(f), p0_declares_doc(_, f)`, by a path), which no load can answer. Such a
    // run hands its exports on whole.
    let factorizing = crate::code_rule_dsl::typing::file_local::factorize_on(options) && !loading_run;
    let mut section: Option<crate::code_rule_dsl::logic::profile::Span> = None;
    drop(section.take()); section = Some(crate::code_rule_dsl::logic::profile::span(|| "the program's key".into()));
    let prepared_key: [u8; 32] = {
        use sha2::Digest as _;
        let mut hash = sha2::Sha256::new();
        hash.update(texts);
        hash.update(format!("{label}\0{:?}\0{:?}\0{:?}\0{}", prog.facts, prog.inputs, prog.outputs, cross_declared.is_some()).as_bytes());
        // Whether exports are handed on factorized (decision 0099): another choice is another prepared program, and
        // another file-local part, so another per-file memo key.
        if !factorizing {
            hash.update(b"\0factorize:off");
        }
        hash.finalize().into()
    };
    // The lowered program's atoms are the store's: the same strings must still be the same atoms.
    let holds = |p: &Prepared| {
        p.interned.iter().all(|(strings, atoms)| {
            let strings: Vec<&str> = strings.iter().map(String::as_str).collect();
            store.intern_atoms(&strings).is_ok_and(|got| got.iter().map(|a| u32::from(*a)).eq(atoms.iter().copied()))
        })
    };
    drop(section.take()); section = Some(crate::code_rule_dsl::logic::profile::span(|| "the prepared program, kept or stored".into()));
    let kept = with_prepared(|kept| kept.get(&prepared_key).and_then(|all| all.iter().find(|p| holds(p)).cloned()));
    // Kept across processes in the persistent store, under the programs' text (decision 0073); a session's or T's run
    // keeps it in memory only.
    // `Some(None)`: kept, under a source the store does not hold yet, which the store's writer stores with the
    // prepared program (decision 0104: a run's own threads only read).
    let durable_text = (tapes.is_some() && t.is_none() && session.is_none()).then(|| programs.join("\0"));
    let durable: Option<Option<crate::content_store::SourceId>> = match &durable_text {
        Some(text) => Some(memo::stored_source(store, text).map_err(|e| Error::Plan(e.to_string()))?),
        None => None,
    };
    // The sources the stored program lives under: the program text's, its facts document's and its groups of automata
    // (decisions 0073 to 0075). No file roots them, so the run's manifest names them, and collection keeps them while
    // it keeps the run.
    let mut program_sources: Vec<crate::content_store::SourceId> = durable.flatten().into_iter().collect();
    // Those a queued write stores (the prepared program's documents, decision 0102): known once it is written, and
    // read by the manifest, which is queued after it.
    let late_sources: Late = Default::default();
    // And the plans it is stored under, which the manifest names too: an earlier build's go once no kept run names them.
    let mut program_plans: Vec<[u8; 32]> = if durable.is_some() { prepared::plans(&prepared_key) } else { Vec::new() };
    // The combined program's entry (`combined`), when the store keeps it: the manifest names it too.
    // Their sources found (or stored) by the store's writer: the manifest, queued after, reads them.
    if durable.is_some() {
        let (text, plan) = combined::named(&texts);
        memo::named_source(store, text, false, &late_sources);
        program_plans.push(plan);
    }
    // The plans' whole-program choices (inline or store, demand), kept under the prepared program across processes
    // (`plan::choices`, decision 0090 §5); the manifest names their entries too.
    let _choices = durable.and(tape_cache_path()).map(|path| super::choices::Durable::start(&path, prepared_key));
    if durable.is_some() {
        let (text, plan) = super::choices::named(&prepared_key);
        memo::named_source(store, text, true, &late_sources);
        program_plans.push(plan);
    }
    let (kept, from) = match (kept, durable) {
        (Some(p), _) => (Some(p), "memory"),
        (None, Some(Some(source))) => {
            let _lookup = crate::code_rule_dsl::logic::profile::span(|| "the prepared program, from the store (decision 0073)".into());
            // One this process queued is read once written (`settle`; it is kept in memory, so this is rare).
            store.settle(&prepared::queued(&prepared_key));
            let found = prepared::lookup(store, source, dialect, &prepared_key).map_err(|e| Error::Plan(e.to_string()))?.filter(|(p, _)| holds(p));
            (found.map(|(p, documents)| {
                program_sources.extend(documents);
                Arc::new(p)
            }), "store")
        }
        (None, _) => (None, ""),
    };
    drop(section.take());
    let reused = kept.is_some();
    let prepared = match kept {
        Some(p) => {
            crate::code_rule_dsl::prof::count("corpus.prepared_reused", 1);
            if from == "store" {
                keep_prepared(prepared_key, Arc::clone(&p));
            }
            p
        }
        None => {
            let mut interned: Vec<(Vec<String>, Vec<u32>)> = Vec::new();
            // @py_unparse(n) and @ast_dump(n) read the relations the batches bind them as.
            cpython_functors(&mut prog)?;
            // The base relations it reads and does not derive are inputs, `.input` or not, as an authored file's are
            // (`convert::overlay`): the batches bind them, where a relation of its own would be an empty table.
            base_inputs(&mut prog);
            // Only the rules an output reads, directly or through other rules: a rule that includes a library runs the part
            // of it the rule uses, not all of it (T's front prunes the same way).
            let preparing = crate::code_rule_dsl::logic::profile::span(|| "prune, simplify, demand".into());
            prog = front::prune(prog);
            // Bodies simplified first (decision 0038): implied atoms and set-member kind tests drop, identical relations merge.
            crate::code_rule_dsl::logic::simplify::simplify(&mut prog);
            prog = front::prune(prog);
            // Tests a rule negates of the nodes it has bound, computed for those nodes only, not every node (magic sets).
            let (mut prog, demanded) = crate::code_rule_dsl::logic::magic::demand_negated_tests(prog);
            drop(preparing);
            // The `ast` atoms become relations the transducer's matchers fill per chunk (decision 0106).
            // A Python program's tree-shaped parts become matchers first (decision 0106 §4), unless `RERAIL_AST_PARTS=off`.
            if dialect == crate::code_rule_dsl::language::GrammarDialect::Python && !std::env::var("RERAIL_AST_PARTS").is_ok_and(|v| v == "off") {
                // Parts whose field nodes the rule reads again join through the matcher's captures (decision 0107),
                // unless `RERAIL_AST_PART_CAPTURES=off`.
                let capture = !std::env::var("RERAIL_AST_PART_CAPTURES").is_ok_and(|v| v == "off");
                let rewritten = matchers::absorb(&mut prog, capture);
                if rewritten > 0 {
                    crate::code_rule_dsl::logic::profile::decision("tree patterns as matchers (decision 0106)", || format!("{rewritten} rules' kind and field parts run as the transducer's matchers"));
                }
            }
            let matchers = matchers::lower(&mut prog);
            // Long text where no body fixes the node's kind: twin rules read it for the nodes they bind (decision 0109).
            if prog.decls.contains_key("text") {
                crate::code_rule_dsl::logic::strings::node_subject_tests(&mut prog);
                let twins = syntax::long_text_twins(&mut prog, &matchers);
                if twins > 0 {
                    crate::code_rule_dsl::logic::profile::decision("long text twins (decision 0109)", || format!("{twins} rules read text past the inline bound for the nodes they bind"));
                }
            }
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
            bound(&lowered.program)?;
            drop(_step);
            // The file-local part of each cross-file rule runs in the chunks, which export its rows, not their base relations.
            let _step = crate::code_rule_dsl::logic::profile::span(|| "push the file-local joins into the chunks".into());
            let pushed = crate::code_rule_dsl::typing::file_local::push_local_joins(&mut lowered.program);
            drop(_step);
            // An export that is a join of parts meeting at a key it projects away is handed on as its parts (decision 0099).
            let _step = crate::code_rule_dsl::logic::profile::span(|| "factorize the exports".into());
            let factorized = if factorizing { crate::code_rule_dsl::typing::file_local::factorize_exports(&mut lowered.program) } else { Vec::new() };
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
            let read: BTreeSet<String> = lowered.program.rules.iter().flat_map(|r| r.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.to_string())).collect();
            // An input no rule reads is never bound or measured: it is not an input of what runs.
            let outputs = lowered.program.outputs.clone();
            lowered.program.inputs.retain(|rel| read.contains(rel) || outputs.contains(rel));
            let _step = crate::code_rule_dsl::logic::profile::span(|| "split the file-local and cross-file parts".into());
            let split = crate::code_rule_dsl::typing::file_local::programs(&lowered.program);
            drop(_step);
            if let Some(path) = std::env::var_os("RERAIL_SPLIT_DUMP") {
                dump_split(std::path::Path::new(&path), &split);
            }
            // One prepared program, shared by the process's cache, the store's writer and this run (each a copy before:
            // three of a 16 k-rule program's four forms at once, about 0.4 GB of the Python pack's peak).
            let prepared = Arc::new(Prepared { prog, vocabulary: Arc::new(vocabulary), lowered, sets, matchers, demanded, pushed, factorized, located, merged, split, interned });
            keep_prepared(prepared_key, Arc::clone(&prepared));
            if let (Some(stored), Some(text)) = (durable, durable_text.clone()) {
                let _keep = crate::code_rule_dsl::logic::profile::span(|| "keep the prepared program in the store (decision 0073), queued".into());
                // Made into facts, encoded and written by the store's writer (decision 0102); its documents' sources
                // reach the manifest, queued after it.
                let (kept, key, late) = (Arc::clone(&prepared), prepared_key, Arc::clone(&late_sources));
                store.write_back(
                    "the prepared program",
                    0,
                    vec![(prepared::queued(&prepared_key), None)],
                    Box::new(move |store| {
                        let source = match stored {
                            Some(id) => id,
                            None => {
                                let id = memo::source(store, &text)?;
                                late.lock().expect("the late sources").push(id);
                                id
                            }
                        };
                        if let Some(documents) = prepared::remember(store, source, &key, &kept)? {
                            late.lock().expect("the late sources").extend(documents);
                        }
                        Ok(())
                    }),
                );
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
    let Prepared { prog, vocabulary, lowered, sets, matchers, demanded, pushed, factorized, located, merged: (remerged, reshared), split: shared_split, interned: _ } = &*prepared;
    let (remerged, reshared) = (*remerged, *reshared);
    // The run takes the split's programs apart (`part` below): its own copy, the kept one shared.
    let mut split = shared_split.clone();
    // `RERAIL_DUMP_MATCHERS=<file>`: the rules reading a matcher's relation, and each matcher (a debugging aid), from the prepared program, made or kept.
    if let Ok(path) = std::env::var("RERAIL_DUMP_MATCHERS") {
        let mut text = String::new();
        for rule in prog.rules.iter().filter(|r| r.body.iter().any(|l| l.atom().is_some_and(|a| a.rel.starts_with(matchers::PREFIX)))) {
            text.push_str(&format!("{rule}\n"));
        }
        for (rel, matcher, captures) in matchers.iter() {
            text.push_str(&format!("{rel}\t{captures:?}\t{matcher}\n"));
        }
        let _ = std::fs::write(path, text);
    }
    // A program none of whose rules reads a document runs over none: its file-local rules make only constants (which
    // both parts derive: `file_local::constant_relations`), and the cross-file part reads no base relation but the
    // run's constants (an earlier phase's exports, the options). A later phase reading only an earlier one's exports
    // is such a program: an edit costs it what the exports' delta does, not a pass over every file.
    let constants_only = {
        let constant = crate::code_rule_dsl::typing::file_local::constant_relations(&lowered.program);
        split.local.rules.iter().all(|r| constant.contains(r.head.rel.as_str()))
    };
    if t.is_none() && constants_only && split.base.iter().all(|r| crate::code_rule_dsl::typing::file_local::run_constant(&prog, r)) && !docs.is_empty() {
        crate::code_rule_dsl::logic::profile::decision("documents", || format!("none read: no rule reads a document, so the run is over none of the {} files", docs.len()));
        docs.clear();
    }
    // The text tests' plans compiled once for the process (decision 0075), their groups of automata read from the store
    // when an earlier process determinized them.
    let compiling = crate::code_rule_dsl::logic::profile::span(|| "compile the text tests".into());
    let compiled = match durable {
        Some(_) => {
            // The groups' sources reach the manifest with the other late ones: those it stores are the writer's.
            let groups = prepared::StoredDfas(store, Arc::clone(&late_sources));
            vocabulary.compile(Some(&groups))
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
    if !factorizing {
        crate::code_rule_dsl::logic::profile::decision("factorized store (decision 0099)", || {
            if loading_run { "off: a run that reads the file system answers its loads by the exports' locations".to_owned() } else { "off (RERAIL_FACTORIZE=off or the run's `factorize: off`): every export handed on whole".to_owned() }
        });
    }
    for line in factorized.iter() {
        crate::code_rule_dsl::logic::profile::decision("factorized store (decision 0099)", || line.clone());
    }
    if remerged > 0 {
        crate::code_rule_dsl::logic::profile::decision("merged after the joins were pushed", || format!("{remerged} relations merged into an identical one ({reshared} classes shared across programs)"));
    }
    let unlocated: Vec<&String> = split.exports.iter().chain(split.local.outputs.iter()).filter(|r| split.locations.get(*r).is_none_or(Vec::is_empty)).collect();
    // A file-local part reading the run's options (`file_local::option_input`) is memoised under them too.
    drop(section.take()); section = Some(crate::code_rule_dsl::logic::profile::span(|| "the memo's program and the components' plan".into()));
    // A digest under the rows of the phases' exports that `rules` read, which are not in the program's text.
    let under_phased = |digest: [u8; 32], rules: &[front::Rule]| -> [u8; 32] {
        let by_rows: BTreeSet<&str> = rules.iter().flat_map(|r| &r.body).filter_map(|l| l.atom()).map(|a| a.rel.as_str()).filter(|r| phased.contains_key(*r)).collect();
        if by_rows.is_empty() { digest } else { memo::with_rows(digest, by_rows.iter().map(|r| (*r, &phased[*r]))) }
    };
    let memo_program = {
        let digest = memo::program_digest(&split.local);
        let reads_options = split.local.rules.iter().flat_map(|r| &r.body).filter_map(|l| l.atom()).any(|a| crate::code_rule_dsl::typing::file_local::option_input(&a.rel));
        let digest = if reads_options { memo::with_options(digest, options) } else { digest };
        // Under the earlier phases' rows only those rules read that a memoised row depends on (an export's or an
        // output's): a constant the per-file part derives for nothing it keeps (phase 1's keys) does not change a
        // file's entry. Nor does a constant the cross-file part reads (phase 1's `py_own`): its rows name no document,
        // and a file's entry holds none of them; one a per-file rule reads is reached through that rule.
        let constant = crate::code_rule_dsl::typing::file_local::constant_relations(&split.local);
        let mut kept: BTreeSet<&str> = split.exports.iter().chain(split.local.outputs.iter()).map(String::as_str).filter(|r| !constant.contains(*r)).collect();
        loop {
            let more: Vec<&str> = split.local.rules.iter().filter(|r| kept.contains(r.head.rel.as_str())).flat_map(|r| r.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.as_str())).filter(|r| !kept.contains(r)).collect();
            if more.is_empty() {
                break;
            }
            kept.extend(more);
        }
        let reaching: Vec<front::Rule> = split.local.rules.iter().filter(|r| kept.contains(r.head.rel.as_str())).cloned().collect();
        under_phased(digest, &reaching)
    };
    // The components' stored rows are a function of the phases' exports the cross-file rules read too (P3's
    // `types__attr`, `types__tmap`: a library's stubs, which no component's documents hold), so the plan is digested
    // under those as well. Coarse: an export that changes misses every component, not only those that read the rows
    // that changed.
    let cross_digest = under_phased(memo_program, &split.cross.rules);
    let cross_plan = match &cross_declared {
        Some(declared) => cross_memo::split(&mut split.cross, declared, &cross_digest, &split.same_doc)?,
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
    let read: BTreeSet<String> = lowered.program.rules.iter().flat_map(|r| r.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.to_string())).collect();
    // The reference graph's relations, for a program that reads them: its files are summarized as the chunks are
    // built, and linked once they all are (`references`). W's graph is Python's.
    let references = match CROSS_FILE.get() { Some(make) => make(label, &prog, &read, &docs.iter().map(|(_, p)| p.clone()).filter(|p| !text_document(p)).collect::<Vec<_>>())?, None => None };
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
    // Preparation's splits are done: the kept ones (`file_local::split_memo`) go.
    crate::code_rule_dsl::typing::file_local::forget_splits();
    crate::code_rule_dsl::logic::profile::decision("nearest ancestors", || {
        let names: Vec<&str> = sets.iter().map(|(n, _)| n.as_str()).collect();
        format!("{} sets, each made per looked-up node, or for the batch by the transducer's file-local FIX (bound {binding:?}) where the planner expects more lookups than that costs (decision 0105): {}", sets.len(), names.join(", "))
    });
    // Requests answered inside the fixpoint (decision 0065 §2a): the cross-file part's cone of requests runs first,
    // pausing for answers. A load's answer is the loaded document's rows of every per-file relation the cone reads,
    // made by the chunk path as any file's are; the rest of the cross-file part reads the cone's rows as inputs.
    drop(section.take()); section = Some(crate::code_rule_dsl::logic::profile::span(|| "the cone of requests".into()));
    let loading: Option<(front::Program, BTreeSet<String>)> = if loading_run {
        let mut whole = split.cross.clone();
        whole.answers = world::answers(&whole);
        let cone = crate::code_rule_dsl::logic::answers::split(&whole).ok_or_else(|| Error::Plan("a program that reads the file system makes no request in its cross-file part".to_owned()))?;
        // A constant of the run (an earlier phase's export, the observer's facts) is one set of rows for the run, not
        // a document's: the cone reads it as it is bound, and a load does not answer it.
        let per_file: BTreeSet<String> = cone
            .load
            .rules
            .iter()
            .flat_map(|r| &r.body)
            .filter_map(|l| l.atom())
            .map(|a| a.rel.to_string())
            .filter(|r| (split.exports.contains(r) || split.base.contains(r)) && !crate::code_rule_dsl::typing::file_local::run_constant(&prog, r))
            .collect();
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
    // Each output the run writes, other than what the requests' cone makes (what it asks), a union of file-local
    // rows: every rule of one made in the cross-file part one atom of a file-local export.
    let not_per_document = prog
        .outputs
        .iter()
        .filter(|o| split.cross.outputs.contains(o) && !loading.as_ref().is_some_and(|(load, _)| load.rules.iter().any(|r| &r.head.rel == *o)))
        .find(|o| !split.cross.rules.iter().filter(|r| &r.head.rel == *o).all(|r| matches!(r.body.as_slice(), [front::Literal::Atom(a)] if !a.neg && split.exports.iter().any(|x| x == &a.rel))))
        .cloned();
    let (mut local, mut cross) = (part(split.local), part(split.cross));
    // The run's constants made of earlier phases' exports (`v_enum(q, n) :- struct__base(q, _, b), …, n = count :
    // { struct__member(q, x, "") }`): one answer for the run, so derived once, before the chunks, by the same engine
    // over the bound exports, and bound beside them (`run_constants`). The split derives a constant in both parts
    // (`file_local::constant_relations`), which for one of the program's facts costs little; for one of a phase's
    // exports, every chunk read them all again: over the workspace, 116.6 M rows of `struct__member` for 36 k
    // answers, 183 s of a 966 s run. The memo's key is made before (`memo_program`): it holds the rules as written.
    let run_constants: Option<(lower::Lowered, Vec<String>)> = {
        let constant = crate::code_rule_dsl::typing::file_local::constant_relations(&lowered.program);
        let of_phase = |r: &str| phased.contains_key(r);
        // Those whose rules read an export, or another such: to a fixpoint. An output stays where the split put it.
        let mut once: BTreeSet<String> = BTreeSet::new();
        loop {
            let more: BTreeSet<String> = local
                .program
                .rules
                .iter()
                .filter(|r| constant.contains(r.head.rel.as_str()) && !once.contains(r.head.rel.as_str()) && !prog.outputs.iter().any(|x| x == &r.head.rel))
                .filter(|r| r.body.iter().filter_map(front::Literal::atom).any(|a| of_phase(&a.rel) || once.contains(a.rel.as_str())))
                .map(|r| r.head.rel.to_string())
                .collect();
            if more.is_empty() {
                break;
            }
            once.extend(more);
        }
        // Their rules, and those of the constants they read.
        let mut cone = once.clone();
        loop {
            let more: BTreeSet<String> = local
                .program
                .rules
                .iter()
                .filter(|r| cone.contains(r.head.rel.as_str()))
                .flat_map(|r| r.body.iter().filter_map(front::Literal::atom).map(|a| a.rel.to_string()))
                .filter(|rel| constant.contains(rel.as_str()) && !cone.contains(rel.as_str()))
                .collect();
            if more.is_empty() {
                break;
            }
            cone.extend(more);
        }
        (!once.is_empty()).then(|| {
            let mut p = local.program.clone();
            p.rules.retain(|r| cone.contains(r.head.rel.as_str()));
            p.gates.retain(|(rel, _)| cone.contains(rel));
            p.lattice.retain(|rel, _| cone.contains(rel));
            p.outputs = once.iter().cloned().collect();
            for part in [&mut local.program, &mut cross.program] {
                part.rules.retain(|r| !once.contains(r.head.rel.as_str()));
                for rel in &once {
                    if !part.inputs.contains(rel) {
                        part.inputs.push(rel.clone());
                    }
                }
            }
            crate::code_rule_dsl::logic::profile::decision("constants of the phases' exports", || {
                format!("{} relations derived once, before the chunks, and bound beside the exports: {}", once.len(), once.iter().cloned().collect::<Vec<_>>().join(", "))
            });
            (part(p), once.into_iter().collect())
        })
    };
    drop(section.take()); section = Some(crate::code_rule_dsl::logic::profile::span(|| "guests and the tree's projection".into()));
    let slice = embed_slice(&lowered.program).map(part);
    let slice_reads: BTreeSet<String> = slice.iter().flat_map(|s| &s.program.rules).flat_map(|r| &r.body).filter_map(|l| l.atom()).map(|a| a.rel.to_string()).collect();
    crate::code_rule_dsl::logic::profile::decision("guests", || match &slice {
        Some(s) => format!("an embed slice of {} rules runs per chunk, up to {} rounds, before the file-local part (decision 0055)", s.program.rules.len(), guests::DEPTH),
        None => "no rule asks for a guest".into(),
    });
    // The tree's projection: `core.rr`'s for a program that includes it, T's for one that declares its base relations
    // itself (decision 0046); relations no rule reads are not made.
    let projection = syntax::Projection::of(&prog, read.clone()).with_long_text(&prog, &matchers);
    crate::code_rule_dsl::logic::profile::decision("base relations", || format!("{:?} shape (decision 0046)", projection.shape));
    // A run that reads nothing of its documents builds none: every relation its outputs need, through any rule, is
    // derived, given as facts (an earlier phase's exports, the run's constants) or a native function. A main
    // program over phase exports alone (`defs(q, k) :- struct__def(q, k)`) built and projected every document
    // for nothing: a third of the documents' cost of struct and values run with such a program. Anything else that
    // might read them (T's facts, requests' loads, guests, text or AST patterns) keeps them.
    {
        let p = &lowered.program;
        let not_of_documents = |rel: &str| {
            p.rules.iter().any(|r| r.head.rel == rel) || p.facts.contains_key(rel) || p.natives.contains_key(rel) || phased.contains_key(rel)
        };
        let needs = front::needed(p, p.outputs.clone());
        let why: Option<String> = if t.is_some() {
            Some("T's facts".into())
        } else if loading.is_some() {
            Some("requests' loads".into())
        } else if slice.is_some() || reads_guests {
            Some("guests".into())
        } else if !p.rx_spans.is_empty() || !p.ast_patterns.is_empty() {
            Some("text or AST patterns".into())
        } else {
            needs.iter().find(|rel| !not_of_documents(rel)).map(|rel| format!("{rel} is read"))
        };
        let reads_documents = why.is_some();
        if let Some(why) = &why {
            crate::code_rule_dsl::logic::profile::decision("documents", || format!("built: {why}"));
        }
        if !reads_documents && !docs.is_empty() {
            crate::code_rule_dsl::logic::profile::decision("documents", || format!("none built: no rule reads a relation of the {} documents", docs.len()));
            // Still listed in what the run consulted, without a digest: the workspace's files are what they were
            // (a file added or gone runs it again), but no edit of one changes what it derives (`changes_of`).
            let dropped: BTreeSet<String> = docs.drain(..).map(|(_, p)| p).collect();
            for c in consulted.iter_mut().filter(|c| c.0 == "doc" && dropped.contains(&c.1)) {
                c.2.clear();
            }
        }
    }
    // T's corpus statistics, read once: each chunk plans with them and its sources' own, measured in memory, and the
    // cross-file part with them and the chunks' totals. Without T, every chunk measures its own (`Own`).
    drop(section.take()); section = Some(crate::code_rule_dsl::logic::profile::span(|| "statistics read or measured".into()));
    let t_stats = match &t {
        Some(t) => {
            atomized_statistics(t, &db, &store, &first, dialect)?;
            super::Stats::of_schema(&db, &lowered.program, "s")?
        }
        None => {
            // A document's nodes are `d << 32 | index`: the SQL strata's range reads compute its interval (no table).
            super::docs_by_shift(&db);
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
    // The guest relations' postings per mix, measured after the guest rounds (`guests::READS`).
    let mut guest_postings_once: HashMap<String, (usize, BTreeMap<(String, String, i64), i64>)> = HashMap::new();
    // The stored tree counts this run read or wrote, for its manifest.
    let mut own_sources: Vec<crate::content_store::SourceId> = Vec::new();
    let mut measured_once: HashMap<(String, String), (usize, Vec<(String, i64, i64, i64, i64)>)> = HashMap::new();
    // `x` is the run's scratch (deleted at the start of the next): no journal and no syncs, since a run that stops
    // leaves nothing to recover. The browser build keeps it in memory, where its in-memory filesystem is anyway, so
    // its pages skip the filesystem's calls; so are the temp tables the strata make.
    // Attached, with the pragmas below, when SQL first runs on the connection (`vtab::setup_now`): a run whose
    // strata are all native runs none of it.
    #[cfg(target_family = "wasm")]
    let attach_x = "ATTACH ':memory:' AS x;".to_owned();
    #[cfg(not(target_family = "wasm"))]
    let attach_x = format!("ATTACH {} AS x;", format!("'{}'", cross_path.to_string_lossy().replace('\'', "''")));
    // `x` is this run's: tables an earlier run over the same store read (`table_source`, cached by the store's path
    // and the table's name) are another run's rows. Without this a second run read the first's cross-file tables:
    // a cone's loaded documents, there at the first run's end, counted as the workspace's.
    crate::code_rule_dsl::logic::native::forget_inputs(&db);
    crate::code_rule_dsl::facts::vtab::defer_setup_undone(&db, &format!("{attach_x} PRAGMA x.journal_mode = OFF; PRAGMA x.synchronous = OFF; PRAGMA temp_store = MEMORY;"), "DETACH x;");
    // Each base relation the cross-file part reads is kept in memory, across the chunks, as the columns its rules
    // name (and those decoding a finding's location reads); the others are never read. (Was: written to `x` and
    // read back, 0.75 s on Python.)
    let decodes = cross.program.outputs.iter().any(|o| o == "finding");
    // The text tests (`strtest_<k>(s)`) every cross-file rule reads joined: each positive read's string is bound by
    // another positive atom of its body, so a test row of a string no file of the run holds derives nothing. A memoised
    // chunk may then keep a test row it cannot trace to a file's text with every file of the chunk (`memo`).
    let joined_tests: BTreeSet<String> = {
        use crate::code_rule_dsl::logic::front::{Literal, Term};
        use crate::code_rule_dsl::logic::strings::RELATION;
        let mut tests: BTreeSet<String> = cross.program.rules.iter().flat_map(|r| r.body.iter().filter_map(|l| l.atom())).map(|a| a.rel.to_string()).filter(|r| r.starts_with(RELATION)).collect();
        for rule in &cross.program.rules {
            for (i, l) in rule.body.iter().enumerate() {
                let Some(a) = l.atom().filter(|a| !a.neg && a.rel.starts_with(RELATION)) else { continue };
                let Some(Term::Var(v)) = a.args.first() else { continue };
                let joined = rule.body.iter().enumerate().any(|(j, m)| {
                    j != i && matches!(m, Literal::Atom(b) if !b.neg && !b.rel.starts_with(RELATION) && b.args.iter().any(|t| matches!(t, Term::Var(x) if x == v)))
                });
                if !joined {
                    tests.remove(a.rel.as_str());
                }
            }
        }
        tests
    };
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
    // The phases' exports the program reads, as relations of atoms (`add_bridged`).
    let repository = {
        let mut repository = repository;
        let phased_read: BTreeSet<&String> = phased.keys().filter(|rel| read.contains(*rel)).collect();
        for rel in phased_read {
            let width = prog.decls[rel].len();
            // Read as `syntax::measured` reads it: its statistics counted from its rows.
            let flat = |s: &dyn crate::code_rule_dsl::facts::vtab::Source| -> Vec<i64> { (0..s.len()).flat_map(|r| (0..width).map(move |c| (r, c))).map(|(r, c)| s.value(r, c)).collect() };
            let rows = &phased[rel];
            let source = atoms_source(&store, width, rows.iter())?;
            let mut counts = crate::code_rule_dsl::logic::plan::statkeep::RelStats::new(source.columns());
            counts.delta(&flat(source.as_ref()), &[], width);
            let measured = counts.statistics(&|_| true);
            repository.push((rel.clone(), source, measured));
        }
        repository
    };
    for rel in split.base.iter().filter(|rel| !REFERENCE_RELATIONS.contains(&rel.as_str()) && !repository.iter().any(|(r, _, _)| r == *rel)) {
        let cols = projection.columns(rel).ok_or_else(|| Error::Plan(format!("no columns for {rel}")))?;
        let decode: &[&str] = match rel.as_str() {
            "node" if decodes => &["d", "n"],
            "doc" if decodes => &["d", "path"],
            _ => &[],
        };
        let atoms = || cross.program.rules.iter().flat_map(|r| &r.body).filter_map(front::Literal::atom).filter(|a| &a.rel == rel);
        let mut named: Vec<&str> = cols
            .iter()
            .enumerate()
            .filter(|(i, c)| decode.contains(c) || atoms().any(|a| a.args.get(*i).is_some_and(|t| !matches!(t, front::Term::Wild))))
            .map(|(_, c)| *c)
            .collect();
        // A relation read with every column a wildcard (`text(_, _)` in a negated conjunction) is read for whether it
        // has rows: its first column is held, so the cross-file part reads every chunk's rows and measures them (not
        // the last chunk's binding, or none where the files came from the per-file memo).
        if named.is_empty() && atoms().next().is_some() {
            named.push(cols[0]);
        }
        if named.is_empty() {
            continue;
        }
        stored.push((rel.clone(), named.iter().map(|c| (*c).to_owned()).collect()));
    }
    // Decide after projection planning: the split's base list can still contain unused inputs.
    // This changes the local frame's identity, not the cross-file program's semantic identity.
    let memo_tree_stats = !stored.is_empty() || cross.program.rules.iter().flat_map(|r| &r.body)
        .any(|l| matches!(l, front::Literal::Nearest { .. }));
    let memo_program = memo::with_tree_demand(memo_program, memo_tree_stats);
    // The run holds its memo plan and the sources it reads until its manifest names them (O95): a collector running
    // meanwhile would otherwise take the rows this run writes, or the sources they and its tapes are of.
    let _held = match memo {
        Some(memo::Memo::Stored(_)) if memoised => Some(HeldPlan::new(store, &[&[memo::plan_digest(&memo_program)][..], &program_plans].concat()).map_err(|e| Error::Plan(e.to_string()))?),
        _ => None,
    };
    // A run that reads the reference graph is not memoised: the graph needs every file's facts
    // (`references::Files::add`), which a memo hit does not build.
    let memoised = memoised && references.is_none();
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
    // The exports the chunks hand on, each row once, held in memory for the cross-file part to read in place
    // (`exports`). Was: a table of `x` per export, filled by SQL and read back.
    let exports = std::cell::RefCell::new(exports::Exports::default());
    drop(section.take());
    phase("stats", statements.count());
    statements.enter("parse");
    let modules = Modules::default();
    // Every input the cross-file part reads, as bound: a session keeps their digests (decision 0077).
    let mut cross_inputs: Vec<(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)> = Vec::new();
    for (rel, source, _) in &repository {
        modules.bind(&db, rel, Arc::clone(source))?;
        cross_inputs.push((rel.clone(), Arc::clone(source)));
    }
    // The run's constants of the phases' exports (`run_constants`): derived over the bound exports, copied out of the
    // engine, measured and bound as they are.
    let mut repository = repository;
    if let Some((consts, rels)) = &run_constants {
        let _span = crate::code_rule_dsl::logic::profile::span(|| "derive the constants of the phases' exports".into());
        let mut stats = t_stats.clone();
        for (rel, _, measured) in &repository {
            stats.measured(rel, measured.iter().map(|(c, rows, ndv, sq, sqd)| (c.clone(), [*rows, *ndv, *sq, *sqd])), []);
        }
        let got = super::run_atoms_native_over(consts, &db, "main", stats, rels.iter().cloned().collect(), modes)?;
        for rel in rels {
            let width = lowered.program.decls[rel].len();
            let names: Vec<String> = (0..width).map(|c| format!("c{c}")).collect();
            // Kept in the engine, or written to its table (a stratum the native engine declined): `held_rows`.
            let rows = held_rows(&db, &got, rel, width)?;
            let values: Vec<Option<Vec<i64>>> = (0..width).map(|c| Some(rows.iter().map(|r| r[c]).collect())).collect();
            let source = crate::code_rule_dsl::logic::native::component::columns_source(names, values);
            let measured = syntax::statistics(source.as_ref());
            modules.bind(&db, rel, Arc::clone(&source))?;
            cross_inputs.push((rel.clone(), Arc::clone(&source)));
            repository.push((rel.clone(), source, measured));
        }
        crate::code_rule_dsl::logic::native::component::release(&db, rels);
    }
    let repository = repository;
    let repository_stats = |stats: &mut super::Stats| {
        for (rel, _, measured) in &repository {
            stats.measured(rel, measured.iter().map(|(c, rows, ndv, sq, sqd)| (c.clone(), [*rows, *ndv, *sq, *sqd])), []);
        }
    };
    let mut base_rows: BTreeMap<String, (Vec<String>, Vec<Option<Vec<i64>>>)> = BTreeMap::new();
    // The base rows as sources, once made (`columns_source`): measured and bound as one, so they keep their statistics.
    let mut base_sources: BTreeMap<String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>> = BTreeMap::new();
    // Each one's columns by name, `None` for one not held (it reads as 0): its rows in a memo entry read as these
    // (`input_delta`).
    let mut base_held: HashMap<String, Vec<Option<String>>> = HashMap::new();
    // Their digests, when a session's run took them (`retain::kept_source`).
    let mut input_digests: HashMap<String, u64> = HashMap::new();
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
    // Chunk k+1 is built (read, parsed, `nearest` bound) by the chunk builder's jobs while chunk k's file-local part
    // runs (E12, list step 24): at most one chunk waits built, so memory is two chunks'. The builder's jobs' CPU is
    // measured on their threads and reported as `parse` and `batch`; the main thread's own as the other phases.
    // A chunk: its nodes and sources (none when it is only memo hits), its documents (key, path, source, nodes) and
    // the memo hits read since the last chunk.
    type Chunk = (usize, Vec<(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)>, Vec<(i64, String, Option<memo::Content>, u64)>, Vec<(String, memo::Entry, Option<memo::Content>)>, Vec<syntax::Document>);
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
        // An earlier run of this process may have queued the root's stats: read once they are written, before the
        // first wave (decision 0104).
        store.settle(&stats_key(&canonical_root));
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
    docs_out.send(docs.iter().map(|(d, p)| (*d, p.clone(), given_modules.get(p).cloned())).collect()).map_err(|_| Error::Plan("the chunk builder's channel".to_owned()))?;
    let mut docs_out = loading.is_some().then_some(docs_out);
    // What loading made: each loaded document (key, path), and the rows the rest of the cross-file part reads of the
    // cone and of the answers.
    let mut loaded_docs: Vec<(i64, String)> = Vec::new();
    let mut cone_inputs: BTreeMap<String, String> = BTreeMap::new();
    let no_loads = options.get("phase_no_loads").and_then(serde_json::Value::as_bool) == Some(true);
    let mut loading_rows: Vec<(String, Vec<Vec<i64>>)> = Vec::new();
    let mut cone_kept: Option<(usize, usize)> = None;
    let mut cone_rounds = 0;
    #[cfg(target_family = "wasm")]
    if docs_out.is_some() {
        return Err(Error::Plan("requests answered inside the fixpoint need threads (decision 0065 §2a)".to_owned()));
    }
    // A session's base relations' statistics, kept by file across its runs (decision 0096 §3): each with every column
    // `input_columns` names. `RERAIL_STATS_KEEP=off` measures them as before.
    // Without a session, the tape cache's memo keeps them: read from the blob the last run of this plan wrote.
    let keeping = memoised && !stored.is_empty() && std::env::var("RERAIL_STATS_KEEP").map_or(true, |v| v != "off");
    let mut keeper: Option<keptstats::Keeper> = match (session.as_ref(), memo) {
        _ if !keeping => None,
        (Some(s), _) => Some(s.stats.lock().expect("the session's statistics").remove(&memo_program).unwrap_or_default()),
        (None, Some(memo::Memo::Stored(store))) => {
            let _s = crate::code_rule_dsl::logic::profile::span(|| "read the kept statistics".into());
            let mut k = store.stat_blob(memo_program).ok().flatten().and_then(|b| keptstats::Keeper::decode(&b)).unwrap_or_default();
            // The contents its last run had not stored yet, by their sources now.
            k.resolve(|digests| store.sources_by_hashes(&digests.iter().map(|d| crate::content_store::HashKey::from_sha256_digest(*d)).collect::<Vec<_>>()).unwrap_or_else(|_| vec![None; digests.len()]));
            Some(k)
        }
        _ => None,
    };
    if let Some(k) = keeper.as_mut() {
        // By the key their rows have in a memo entry: a base relation's (`base_key`), and an export's own name.
        let rels: Vec<(String, Vec<String>)> = stored
            .iter()
            .map(|(rel, _)| (memo::base_key(rel), super::super::sql::select::input_columns(rel).unwrap_or_default().iter().map(|c| (*c).to_owned()).collect()))
            .chain(split.exports.iter().map(|rel| (rel.clone(), (0..lowered.program.decls[rel].len()).map(|c| format!("c{c}")).collect())))
            .collect();
        // An export's rows' document: its first node column's, as the chunks' totals read it (`Totals::add_rows`).
        let doc_at = split.exports.iter().map(|rel| (rel.clone(), lowered.program.decls[rel].iter().position(|(_, t)| t == "node"))).collect();
        // A session's exports counted by file too (0096 step 3). `RERAIL_COUNT_EXPORTS=off` holds every hit's rows
        // again.
        // With them each base relation's held columns, and the files' tree counts.
        let keep = (session.is_some() && std::env::var("RERAIL_COUNT_EXPORTS").map_or(true, |v| v != "off")).then(|| keptstats::Keep {
            exports: split.exports.iter().cloned().collect(),
            base: stored
                .iter()
                .map(|(rel, names)| {
                    let all = super::super::sql::select::input_columns(rel).unwrap_or_default();
                    (rel.clone(), memo::base_key(rel), names.iter().filter_map(|n| all.iter().position(|c| c == n)).collect())
                })
                .collect(),
        });
        k.begin(&rels, doc_at, keep);
    }
    // The memo hits' exports are not measured while the keeper counts them: their statistics are its (after the chunks).
    let mut hits_unmeasured = false;
    // The hits whose export rows the counted exports hold, not held again (0096 step 3); not skipped once the cone of
    // requests has started (its loads read the rows the chunks hold).
    let held_by_count: std::cell::RefCell<Vec<(String, memo::Content, i64)>> = std::cell::RefCell::new(Vec::new());
    let skip_counted = std::cell::Cell::new(true);
    // The counted rows installed as the run's (`finish`); else the skipped hits whose base rows and tree counts are
    // taken from the memo after all (with the statistics).
    let installed = std::cell::Cell::new(false);
    let given_up: std::cell::RefCell<Vec<(String, memo::Content, i64)>> = std::cell::RefCell::new(Vec::new());
    // The tree counts of the files the keeper counted that were not skipped: what its sum holds beyond the skipped.
    let mut own_seen = Own::default();
    let keeper = std::cell::RefCell::new(keeper);
    // The counted exports given up: the hits they held are held from the memo now, as the replay would have.
    let hold_after_all = || -> Result<()> {
        if let Some(k) = keeper.borrow_mut().as_mut() {
            k.fail("the counted exports given up this run");
        }
        let skipped = std::mem::take(&mut *held_by_count.borrow_mut());
        if skipped.is_empty() {
            return Ok(());
        }
        given_up.borrow_mut().extend(skipped.iter().cloned());
        let _s = crate::code_rule_dsl::logic::profile::span(|| "hold the hits' exports after all".into());
        let memo = memo.ok_or_else(|| Error::Plan("counted exports without a memo".to_owned()))?;
        for (path, content, key) in skipped {
            let entry = memo.lookup(content, world::store_path(&path), key, &memo_program).map_err(|e| Error::Plan(e.to_string()))?.ok_or_else(|| Error::Plan(format!("{path}: its memo entry is gone, and the counted exports were given up")))?;
            for (rel, (width, values)) in &entry.rows {
                if values.is_empty() || !split.exports.contains(rel) {
                    continue;
                }
                let cols: Vec<Option<Vec<i64>>> = (0..*width).map(|c| Some(values.iter().skip(c).step_by(*width).copied().collect())).collect();
                let rows = crate::code_rule_dsl::logic::native::component::columns_source((0..*width).map(|c| format!("c{c}")).collect(), cols);
                exports.borrow_mut().add(rel, *width, rows.as_ref());
            }
        }
        crate::code_rule_dsl::logic::profile::decision("exports counted by file (0096 step 3)", || "given up this run: the hits' exports held from the memo".to_owned());
        Ok(())
    };
    let stats_read = parking_lot::Mutex::new(Vec::new());
    let looking = memoised || recording;
    /// A file as the run found it before reading it in a wave (decision 0104): its stat, its content as the memo
    /// knows it (`None`: the store holds no such source yet, and its tape's write stores it), its entry, not
    /// decoded, and its text when it was read to look it up in a wave.
    struct Before {
        stat: Option<crate::content_store::FileStat>,
        content: Option<memo::Content>,
        found: Option<memo::Found>,
        text: Option<String>,
        /// The content's SHA-256, when it was read to look it up.
        digest: Option<[u8; 32]>,
        /// Its tape, when the memo has no entry for it and the store has the tape (decision 0104): read before the
        /// first wave, decompressed in the file's wave.
        tape: Option<crate::content_store::TapeFrame>,
    }
    // Files looked up together: stat each, find the entries of those the stat index knows in one statement, read and
    // hash the others, find their contents in one statement and their entries in one more. A few statements for any
    // number of files, and nothing written: a source found is pinned by the store's writer, a new one is stored with
    // its tape.
    let look_up = |window: &[(i64, String, Option<Arc<str>>)], keep: bool| -> Result<Vec<Before>> {
        use rayon::prelude::*;
        use sha2::{Digest as _, Sha256};
        // Inside the waves (a load's files), the store is not read (decision 0104): each file is new to the run, its
        // source and tape handed to the writer as it is parsed.
        let reading = !crate::code_rule_dsl::logic::profile::inside_waves();
        let stat = |path: &String| -> Option<crate::content_store::FileStat> {
            if !looking {
                return None;
            }
            let _stat = crate::code_rule_dsl::logic::profile::span(|| "stat".into());
            crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::FileStat, 1);
            std::fs::metadata(root.join(path)).ok().map(|m| crate::content_store::FileStat::of(&m))
        };
        // The stat index (git's): a file whose stat is the one recorded when its source was last read is that
        // source, and on a memo hit is neither read nor hashed.
        let stats: Vec<Option<crate::content_store::FileStat>> = window.par_iter().map(|(_, path, _)| stat(path)).collect();
        let known: Vec<Option<crate::content_store::SourceId>> = window.iter().zip(&stats).map(|((_, path, _), stat)| stat.and_then(|now| stat_index.source(world::store_path(path), now))).collect();
        let mut content: Vec<Option<memo::Content>> = known.iter().map(|k| k.map(memo::Content::Stored)).collect();
        let mut found: Vec<Option<memo::Found>> = (0..window.len()).map(|_| None).collect();
        let mut digest: Vec<Option<[u8; 32]>> = vec![None; window.len()];
        if let (true, true, Some(memo)) = (memoised, reading, memo) {
            let _lookup = crate::code_rule_dsl::logic::profile::span(|| "memo lookup".into());
            let asked: Vec<usize> = (0..window.len()).filter(|i| known[*i].is_some()).collect();
            let keys: Vec<(memo::Content, &str)> = asked.iter().map(|i| (memo::Content::Stored(known[*i].expect("known")), world::store_path(&window[*i].1))).collect();
            for (i, f) in asked.into_iter().zip(memo.found_all(&memo_program, &keys).map_err(|e| Error::Plan(e.to_string()))?) {
                found[i] = f;
            }
        }
        // The others are read and hashed, whatever the index said: a file may have changed since its stat.
        let rest: Vec<usize> = (0..window.len()).filter(|i| found[*i].is_none()).collect();
        let read: Vec<Option<String>> = rest
            .par_iter()
            .map(|i| {
                let path = &window[*i].1;
                let _read = crate::code_rule_dsl::logic::profile::span(|| "read".into());
                let source = match std::fs::read_to_string(root.join(path)) {
                    Ok(source) => source,
                    // A text file that is not UTF-8 (an image, an archive) is no document: its wave finds it so.
                    Err(e) if e.kind() == std::io::ErrorKind::InvalidData && text_document(path) => return Ok(None),
                    Err(e) => return Err(Error::Plan(format!("{path}: {e}"))),
                };
                crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, source.len() as u64);
                Ok(Some(source))
            })
            .collect::<Result<_>>()?;
        let (rest, texts): (Vec<usize>, Vec<String>) = rest.into_iter().zip(read).filter_map(|(i, t)| t.map(|t| (i, t))).unzip();
        if let (true, Some(memo)) = (looking, memo) {
            let _lookup = crate::code_rule_dsl::logic::profile::span(|| "memo lookup".into());
            crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, texts.iter().map(|t| t.len() as u64).sum());
            let digests: Vec<[u8; 32]> = texts.par_iter().map(|t| Sha256::digest(t.as_bytes()).into()).collect();
            // A content the store does not hold is new: its source is written with its tape, behind the run.
            // A session's memo is in memory: asked in the waves too.
            let contents: Vec<Option<memo::Content>> = if reading || matches!(memo, memo::Memo::Held(_)) {
                memo.found(&digests).map_err(|e| Error::Plan(e.to_string()))?.into_iter().zip(&digests).map(|(c, d)| c.or(Some(memo::Content::Unstored(*d)))).collect()
            } else {
                digests.iter().map(|d| Some(memo::Content::Unstored(*d))).collect()
            };
            for ((i, c), d) in rest.iter().zip(&contents).zip(&digests) {
                digest[*i] = Some(*d);
                if let (Some(stat), Some(c)) = (stats[*i], c)
                    && !matches!(c, memo::Content::Held(_))
                    && known[*i].map(memo::Content::Stored) != Some(*c)
                {
                    stats_read.lock().push((world::store_path(&window[*i].1).to_owned(), stat, *c));
                }
            }
            // A file the index knew, read again because its entry was missing, is not looked up a second time. In
            // the waves every content is looked up by its digest, in the frames held in memory (decision 0104).
            if memoised {
                let asked: Vec<usize> = (0..rest.len()).filter(|r| contents[*r].is_some() && !(known[rest[*r]].is_some() && known[rest[*r]] == contents[*r].and_then(memo::Content::stored))).collect();
                let keys: Vec<(memo::Content, &str)> = asked.iter().map(|r| (contents[*r].expect("a content"), world::store_path(&window[rest[*r]].1))).collect();
                for (r, f) in asked.into_iter().zip(memo.found_all(&memo_program, &keys).map_err(|e| Error::Plan(e.to_string()))?) {
                    found[rest[r]] = f;
                }
            }
            for (i, c) in rest.iter().zip(contents) {
                content[*i] = c;
            }
        }
        // The tapes of the files the memo has no entry for, of those whose sources the store holds: one read.
        let mut tape: Vec<Option<crate::content_store::TapeFrame>> = vec![None; window.len()];
        if let (true, Some(cache), Some(lang)) = (reading, tapes.as_ref().filter(|_| cfg!(feature = "tape-cache")), crate::code_rule_dsl::language::dialect_by_label(label)) {
            let asked: Vec<usize> = (0..window.len()).filter(|i| found[*i].is_none() && matches!(content[*i], Some(memo::Content::Stored(_)))).collect();
            if let (false, Some(tag)) = (asked.is_empty(), crate::code_rule_dsl::language::tape_tag(lang)) {
                let _tapes = crate::code_rule_dsl::logic::profile::span(|| "tapes, by their sources".into());
                let ids: Vec<crate::content_store::SourceId> = asked.iter().map(|i| content[*i].and_then(memo::Content::stored).expect("stored")).collect();
                let frames = cache
                    .tape_frames_for(&ids, i64::from(tag), crate::code_rule_dsl::build_ids::grammar_version(), i64::from(crate::code_rule_dsl::tape::TAPE_FORMAT_VERSION))
                    .map_err(|e| Error::Plan(e.to_string()))?;
                for (i, f) in asked.into_iter().zip(frames) {
                    tape[i] = f;
                }
            }
        }
        let mut texts: HashMap<usize, String> = if keep { rest.into_iter().zip(texts).collect() } else { HashMap::new() };
        Ok(stats
            .into_iter()
            .zip(content)
            .zip(found)
            .zip(digest)
            .zip(tape)
            .enumerate()
            .map(|(i, ((((stat, content), found), digest), tape))| Before { stat, content, found, text: texts.remove(&i), digest, tape })
            .collect())
    };
    // The corpus's files are looked up before the first wave: a wave reads no memo from SQLite, and writes none
    // (decision 0104). A session's memo is in memory, so it is looked up where it is read. A load's files are
    // known only in the waves, and are looked up there.
    let before: parking_lot::Mutex<HashMap<String, Before>> = parking_lot::Mutex::new(match memo {
        Some(memo::Memo::Stored(_)) if looking => {
            let _before = crate::code_rule_dsl::logic::profile::span(|| "memo lookup, before the first wave".into());
            let batch: Vec<(i64, String, Option<Arc<str>>)> = docs.iter().map(|(d, p)| (*d, p.clone(), given_modules.get(p).cloned())).collect();
            let looked = look_up(&batch, false)?;
            batch.into_iter().map(|(_, p, _)| p).zip(looked).collect()
        }
        _ => HashMap::new(),
    });
    // The entries the statistics keeper counted for files changed or gone since (`keptstats`), found before the
    // first wave too: it counts them out in the waves, and reads none of them from SQLite there. One the memo no
    // longer holds is kept as missing (`None`), not looked up again in the waves: a keeper kept in a tape cache
    // shared by runs over other roots counts files this run never sees, whose entries may be gone.
    let counted_before: HashMap<String, Option<memo::Found>> = match (memo, keeper.borrow().as_ref()) {
        (Some(memo @ memo::Memo::Stored(_)), Some(k)) if looking => {
            let _before = crate::code_rule_dsl::logic::profile::span(|| "memo lookup, before the first wave".into());
            let now = before.lock();
            let changed: Vec<(String, memo::Content)> = k.counted_files().into_iter().filter(|(p, c, _)| now.get(p).is_none_or(|b| b.content != Some(*c))).map(|(p, c, _)| (p, c)).collect();
            drop(now);
            let keys: Vec<(memo::Content, &str)> = changed.iter().map(|(p, c)| (*c, world::store_path(p))).collect();
            let found = memo.found_all(&memo_program, &keys).map_err(|e| Error::Plan(e.to_string()))?;
            changed.iter().map(|(p, _)| p.clone()).zip(found).collect()
        }
        _ => HashMap::new(),
    };
    // The entry a file was counted under: found before the first wave, else (a session's, in memory) looked up.
    let counted_entry = |path: &str, c: memo::Content, key: i64| -> Option<memo::Entry> {
        match counted_before.get(path) {
            Some(found) => found.as_ref()?.entry(key).ok().flatten(),
            None => memo.and_then(|m| m.lookup(c, world::store_path(path), key, &memo_program).ok().flatten()),
        }
    };
    // The keeper's end, once every file of the run was seen: the files gone counted out, the statistics as kept (else
    // `None`), and the run's exports the counted ones (else the skipped hits held after all).
    let finish = || -> Result<Option<BTreeMap<String, crate::code_rule_dsl::logic::plan::statkeep::RelStats>>> {
        let kept = keeper.borrow_mut().as_mut().and_then(|k| {
            let ended = k.end(&|path, c, key| counted_entry(path, c, key));
            let (counted_in, counted_out) = k.counted;
            if std::env::var_os("RERAIL_STATS_KEEP_TRACE").is_some() {
                eprintln!("statkeep {:02x}{:02x}: in {counted_in} out {counted_out} {:?}", memo_program[0], memo_program[1], ended.as_ref().err());
            }
            crate::code_rule_dsl::logic::profile::decision("statistics kept by file", || match &ended {
                Ok(_) => format!("{counted_in} files counted in, {counted_out} out; the base relations' statistics are read as kept, none measured"),
                Err(why) => format!("not kept this run ({why}): measured, and every file counted in again next run"),
            });
            ended.ok()
        });
        let skipped = held_by_count.borrow().len();
        let installed = {
            let k = keeper.borrow();
            match k.as_ref().filter(|_| kept.is_some()).and_then(|k| k.counted_exports()) {
                Some(counted) if skipped > 0 => {
                    let _s = crate::code_rule_dsl::logic::profile::span(|| "the counted exports".into());
                    for rel in &split.exports {
                        let width = lowered.program.decls[rel].len();
                        exports.borrow_mut().install(rel, width, counted.source(rel, width));
                    }
                    crate::code_rule_dsl::logic::profile::decision("exports counted by file (0096 step 3)", || format!("{skipped} memo hits neither held again nor counted: {} export rows, {} base rows counted", counted.rows(), k.as_ref().and_then(|k| k.counted_base()).map_or(0, |(b, _)| b.rows())));
                    installed.set(true);
                    true
                }
                _ => false,
            }
        };
        if !installed && skipped > 0 {
            hold_after_all()?;
        }
        Ok(kept)
    };
    let mut finished: Option<Option<BTreeMap<String, crate::code_rule_dsl::logic::plan::statkeep::RelStats>>> = None;
    // Each document this run reads, with its content as the memo knows it (`input_delta`): a session's next run
    // reads its input deltas from the documents whose content changed.
    let run_docs_held: std::sync::Mutex<input_delta::Docs> = std::sync::Mutex::new(HashMap::new());
    let run_docs = &run_docs_held;
    // The run's waves: from its first chunk to the cross-file part's last stratum. SQL a wave's thread runs or waits on
    // meanwhile is a hazard in the profile (decision 0104); before is the run's beginning, after its end.
    {
        // The cone of requests' connection and store, opened before the first wave (decision 0104).
        let (slot, _) = handed.cone();
        connections::Cone::ready(&slot, &path, durable.is_some().then(|| store.database_path()))?;
        // The atoms' lease, held to the run's end: on a new store, a string not in memory is new (decision 0104).
        let complete = store.atoms_handle().ready_for_run().map_err(|e| Error::Plan(e.to_string()))?;
        crate::code_rule_dsl::logic::profile::decision("atoms before the first wave (decision 0104)", || if complete { "every stored atom held: lookups read nothing".into() } else { "the store's atoms are too many to read whole: lookups read its key".into() });
        // And every kept frame (decision 0104): the waves read them from memory.
        let (frames, bytes) = store.preload_frames().map_err(|e| Error::Plan(e.to_string()))?;
        if frames > 0 {
            crate::code_rule_dsl::logic::profile::decision("frames read before the first wave (decision 0104)", || format!("{frames} frames, {:.1} MB encoded", bytes as f64 / 1048576.0));
        }
    }
    let mut waves = Some(crate::code_rule_dsl::logic::profile::waves());
    // A file's memo entry when the run is memoised and the memo has it, else its document, built. An entry
    // without the base rows this run's cross-file part reads is a miss.
    let memo_rows: BTreeSet<String> = split.exports.iter().cloned()
        .chain(stored.iter().map(|(r, _)| memo::base_key(r))).collect();
    let memo_row_needed = |r: &str| r == OWN_DOCS || memo_rows.contains(r)
        || (memo_tree_stats && (r == OWN_KINDS || r == OWN_EDGES || r.starts_with(OWN_POSTINGS)));
    crate::code_rule_dsl::logic::profile::decision("memo consumer projection", || format!(
        "{} demanded relations; tree distributions {}", memo_rows.len(), if memo_tree_stats { "retained" } else { "omitted" }));
    let has_base = |e: &memo::Entry| e.rows.contains_key(OWN_DOCS) && stored.iter().all(|(rel, _)| e.rows.contains_key(&memo::base_key(rel)));
    // A window's files resolved together: as found before the first wave, or (a load's) looked up now. A hit's
    // entry is decoded here; a miss's file is read here, and is a new content if it changed since it was found.
    /// A file to build: its text, its content as the memo knows it, the text's digest and its tape, when they are
    /// known.
    struct Unbuilt {
        text: String,
        content: Option<memo::Content>,
        digest: Option<[u8; 32]>,
        tape: Option<crate::content_store::TapeFrame>,
    }
    type Resolved = (Option<memo::Content>, std::result::Result<memo::Entry, Unbuilt>);
    let resolve = |window: &[(i64, String, Option<Arc<str>>)]| -> Result<Vec<Option<Resolved>>> {
        let _parse = crate::code_rule_dsl::logic::profile::span(|| "worker parse".into());
        let mut taken: Vec<Option<Before>> = {
            let mut before = before.lock();
            window.iter().map(|(_, p, _)| before.remove(p)).collect()
        };
        let late: Vec<usize> = (0..window.len()).filter(|i| taken[*i].is_none()).collect();
        if !late.is_empty() {
            let asked: Vec<(i64, String, Option<Arc<str>>)> = late.iter().map(|i| window[*i].clone()).collect();
            for (i, b) in late.into_iter().zip(look_up(&asked, true)?) {
                taken[i] = Some(b);
            }
        }
        window
            .iter()
            .zip(taken)
            .map(|((d, path, _), b)| {
                let b = b.expect("every file looked up");
                if let (true, Some(found)) = (memoised, b.found) {
                    let _decode = crate::code_rule_dsl::logic::profile::span(|| "memo decode".into());
                    if let Some(entry) = found.projected(*d, &memo_row_needed).map_err(|e| Error::Plan(e.to_string()))?.filter(|e| has_base(e)) {
                        return Ok(Some((b.content, Ok(entry))));
                    }
                }
                let (text, content, digest, tape) = match b.text {
                    Some(text) => (text, b.content, b.digest, b.tape),
                    None => {
                        let _read = crate::code_rule_dsl::logic::profile::span(|| "read".into());
                        let text = match std::fs::read_to_string(root.join(path)) {
                            Ok(text) => text,
                            // A text file that is not UTF-8 (an image, an archive) is no document.
                            Err(e) if e.kind() == std::io::ErrorKind::InvalidData && text_document(path) => return Ok(None),
                            Err(e) => return Err(Error::Plan(format!("{path}: {e}"))),
                        };
                        crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, text.len() as u64);
                        let now = if looking { std::fs::metadata(root.join(path)).ok().map(|m| crate::content_store::FileStat::of(&m)) } else { None };
                        if now.is_some() && now == b.stat {
                            (text, b.content, b.digest, b.tape)
                        } else {
                            // Changed since it was found: a new content, by this text's digest.
                            use sha2::{Digest as _, Sha256};
                            let d: [u8; 32] = Sha256::digest(text.as_bytes()).into();
                            let content = match memo {
                                Some(memo::Memo::Stored(_)) if looking => Some(memo::Content::Unstored(d)),
                                Some(memo::Memo::Held(_)) if looking => Some(memo::Content::Held(d)),
                                _ => None,
                            };
                            (text, content, Some(d), None)
                        }
                    }
                };
                Ok(Some((content, Err(Unbuilt { text, content, digest, tape }))))
            })
            .collect()
    };
    // The matchers a chunk runs (decision 0106). A chunk's documents keep their tapes for them, and for a program that
    // reads W's values, f-string trees or string annotations (`syntax::taped`).
    let chunk_matchers = matchers::compiled(&matchers, &prog.policies).map_err(Error::Plan)?;
    let keep_tapes = !chunk_matchers.is_empty() || read.iter().any(|rel| syntax::taped(rel));
    if !chunk_matchers.is_empty() {
        crate::code_rule_dsl::logic::profile::decision("ast matchers (decision 0106)", || format!("{} matchers run by the transducer over each chunk's tapes", chunk_matchers.len()));
    }
    let build_doc = |d: &i64, path: &String, module: Option<Arc<str>>, unbuilt: Unbuilt| -> Result<(Option<memo::Content>, syntax::Document)> {
        let Unbuilt { text: source, content: source_id, digest, tape } = unbuilt;
        let _build = crate::code_rule_dsl::logic::profile::span(|| "tree-sitter and facts".into());
        // Its own work: the file's input made from its source (the build's parts charge their own).
        crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, source.len() as u64);
        // A file of no grammar is here because `.scope text` reads it (`scoped`): text only, no tree, its lines W's
        // for a file of no language (`core::text`).
        if text_document(path) {
            let facts = Arc::new(core::text(Arc::new(source)));
            let digest = core::digest(&facts);
            return Ok((source_id, syntax::Document { key: *d as u32, path: path.clone(), core: format!("/c#core:{path}"), digest, facts, guest: None, module, tape: None }));
        }
        let file = core::Source { language: label.to_owned(), source: Arc::new(source) };
        // Tapes kept across runs where the run has the persistent store (`tape_cache`).
        let built = match &tapes {
            // The closure sees `&tapes`, not that it is `None` without `tape-cache`: said here so the store's
            // tape and source frames are not linked.
            // In the waves no SQL is read or written (decision 0104): the tape was found before the first wave, or the
            // file is parsed and its source and tape are the writer's, behind the run; the manifest names the sources
            // it stores (`late_sources`).
            Some(cache) if cfg!(feature = "tape-cache") => core::build_in_wave_with_tape(&file, cache, digest, tape.as_ref(), Some(&late_sources), keep_tapes),
            Some(_) => unreachable!("no tape cache without `tape-cache`"),
            None => core::build_with_tape(&file, keep_tapes),
        };
        // The tape is kept beside the facts only where the program runs matchers over it (decision 0106).
        let (built, tape) = match built {
            Ok((built, tape)) => (Ok(built), tape),
            Err(e) => (Err(e), None),
        };
        let facts = match built.map_err(|e| Error::Plan(format!("{path}: {e:?}")))? {
            Built::Facts(f) => Arc::new(f),
            Built::Unparsed => return Err(Error::Plan(format!("{path} did not parse"))),
        };
        if let Some(references) = &references {
            references.add(*d as u32, path, &file.source, &facts)?;
        }
        // The tape, in the file's context: kept only for a program that reads W's values, f-string trees or string
        // annotations off it, or runs matchers over it (decision 0106).
        let tape = tape.map(|doc| (doc, Arc::new(file) as Arc<dyn crate::code_rule_dsl::facts::core::SourceFile>));
        let digest_span = crate::code_rule_dsl::logic::profile::span(|| "digest".into());
        let digest = core::digest(&facts);
        drop(digest_span);
        Ok((source_id, syntax::Document { key: *d as u32, path: path.clone(), core: format!("/c#core:{path}"), digest, facts, guest: None, module, tape }))
    };
    // Files resolved a window at a time (`resolve`), so a window's sources are all the builder holds unparsed: one
    // window at a time, in order, each a job of its own, and each file's tree and facts then built as a job.
    const WINDOW: usize = 256;
    let resolver = parking_lot::Mutex::new(resolve);

    // A document: a memo hit, or built from its source; `None` for a file that is no document.
    type Made = Result<Option<(Option<memo::Content>, std::result::Result<memo::Entry, syntax::Document>)>>;
    type Sources = Vec<(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)>;
    // A chunk's documents, hits and kept documents while it is projected.
    type Cutting = (usize, Vec<(i64, String, Option<memo::Content>, u64)>, Vec<(String, memo::Entry, Option<memo::Content>)>, Vec<syntax::Document>);
    let build = |docs: &Docs, i: usize| -> Made {
        // A window that failed resolves none of its files: its first holds the error, which ends the builder.
        let resolved = docs.resolved.lock().remove(&i).ok_or_else(|| Error::Plan("a file of a window that failed".to_owned()))?;
        let Some((content, outcome)) = resolved else { return Ok(None) };
        let (d, path, module) = &docs.asked[i];
        match outcome {
            Ok(entry) => Ok(Some((content, Ok(entry)))),
            Err(u) => build_doc(d, path, module.clone(), u).map(|(c, doc)| Some((c, Err(doc)))),
        }
    };
    let project = |docs: Vec<syntax::Document>| -> (Result<Sources>, (f64, f64)) {
        let from = thread_usage();
        let built = {
            let _batch = crate::code_rule_dsl::logic::profile::span(|| "worker batch".into());
            syntax::with_matchers(&store, docs, &sets, &chunk_matchers, &vocabulary, binding, &projection).map_err(|e| Error::Plan(e.to_string()))
        };
        let now = thread_usage();
        (built, (now.0 - from.0, now.1 - from.1))
    };
    /// A batch of documents the builder reads (the corpus's, then each load's): its files, its windows resolved so
    /// far (one at a time), and each document built and not yet taken.
    struct Docs {
        asked: Vec<(i64, String, Option<Arc<str>>)>,
        resolved: parking_lot::Mutex<BTreeMap<usize, Option<Resolved>>>,
        /// Files up to here are resolved; a window's job is running.
        resolved_to: std::sync::atomic::AtomicUsize,
        resolving: std::sync::atomic::AtomicBool,
        built: parking_lot::Mutex<BTreeMap<usize, Made>>,
    }
    /// A chunk cut, on its way to the main thread: handed on in the order cut, once its projection is done.
    enum Out {
        Ready(Result<Chunk>),
        Projecting(usize, Cutting),
    }
    /// The builder's own state: the batch it cuts, the next document it takes, what it has cut, and what waits for
    /// the main thread (one message, as a channel of one).
    struct Cut {
        docs_in: std::sync::mpsc::Receiver<Vec<(i64, String, Option<Arc<str>>)>>,
        docs: Option<Arc<Docs>>,
        next: usize,
        spawned: usize,
        pending: Vec<syntax::Document>,
        pending_docs: Vec<(i64, String, Option<memo::Content>, u64)>,
        hits: Vec<(String, memo::Entry, Option<memo::Content>)>,
        pending_nodes: usize,
        out: std::collections::VecDeque<Out>,
        posted: usize,
        handed: std::collections::VecDeque<Result<Option<Chunk>>>,
        /// A document failed: its error is the batch's last message, and the builder ends.
        failed: bool,
        /// No batch will come: the builder has ended.
        ended: bool,
        /// A chunk or the memo's hits have been handed on: a corpus with no document hands on one empty chunk.
        sent: bool,
        read_sources: Vec<crate::content_store::SourceId>,
        parse: (f64, f64),
        batch: (f64, f64),
    }
    /// The chunk builder (decision 0100 §12): no thread of its own. Each window's files are resolved, each document
    /// built, and each chunk projected, as a job on the one pool; whichever job ends, or the main thread when it asks
    /// for a chunk, advances the builder: the documents taken in order and cut into chunks, handed on as cut.
    struct Builder<'a, B, P, R> {
        cut: parking_lot::Mutex<Cut>,
        /// Something may have changed since the builder last advanced (a job ended, or a chunk was taken).
        dirty: std::sync::atomic::AtomicBool,
        /// The main thread stopped (its error, or its end): nothing more is started.
        stopped: std::sync::atomic::AtomicBool,
        projected: parking_lot::Mutex<BTreeMap<usize, std::thread::Result<(Result<Sources>, (f64, f64))>>>,
        /// The documents' and windows' jobs' CPU: parse time.
        jobs_cpu: parking_lot::Mutex<(f64, f64)>,
        /// At most this many documents built ahead of the next taken, and chunks projected at once beside the one
        /// waiting for the main thread.
        ahead: usize,
        in_flight: usize,
        resolve: &'a parking_lot::Mutex<R>,
        build: &'a B,
        project: &'a P,
        run_docs: &'a std::sync::Mutex<input_delta::Docs>,
        target: &'a std::sync::atomic::AtomicUsize,
        keep_docs: bool,
        doing: &'a crate::code_rule_dsl::logic::profile::Activity,
    }
    impl<'a, B, P, R> Builder<'a, B, P, R>
    where
        B: Fn(&Docs, usize) -> Made + Sync,
        P: Fn(Vec<syntax::Document>) -> (Result<Sources>, (f64, f64)) + Sync,
        R: FnMut(&[(i64, String, Option<Arc<str>>)]) -> Result<Vec<Option<Resolved>>> + Send,
    {
        /// Advances the builder as far as it goes now. A caller that finds another advancing leaves it to them: they
        /// look again before they let go.
        fn advance<'s>(&'s self, jobs: &crate::threads::Jobs<'s, '_>)
        where
            'a: 's,
        {
            use std::sync::atomic::Ordering::SeqCst;
            self.dirty.store(true, SeqCst);
            while self.dirty.load(SeqCst) {
                let Some(mut c) = self.cut.try_lock() else { return };
                while self.dirty.swap(false, SeqCst) {
                    let from = thread_usage();
                    while self.step(&mut c, jobs) {}
                    let now = thread_usage();
                    c.parse.0 += now.0 - from.0;
                    c.parse.1 += now.1 - from.1;
                }
            }
        }

        /// The main thread's next message: a chunk, a batch's end (`None`) or an error; `Err` once the builder has
        /// ended. Meanwhile the main thread runs the pool's jobs: the documents' and chunks' among them.
        fn next<'s>(&'s self, jobs: &crate::threads::Jobs<'s, '_>) -> std::result::Result<Result<Option<Chunk>>, ()>
        where
            'a: 's,
        {
            rerail_accel::exec::help_until_as("the main thread: its next chunk", || {
                self.advance(jobs);
                let mut c = self.cut.lock();
                if let Some(r) = c.handed.pop_front() {
                    drop(c);
                    self.advance(jobs);
                    return Some(Ok(r));
                }
                (c.ended && c.out.is_empty()).then_some(Err(()))
            })
        }

        fn add(acc: &mut (f64, f64), from: (f64, f64)) {
            let now = thread_usage();
            acc.0 += now.0 - from.0;
            acc.1 += now.1 - from.1;
        }

        /// One step; whether it changed anything.
        fn step<'s>(&'s self, c: &mut Cut, jobs: &crate::threads::Jobs<'s, '_>) -> bool
        where
            'a: 's,
        {
            use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};
            if self.stopped.load(Relaxed) {
                return false;
            }
            // Hand on what is ready at the front, in order: one message waits for the main thread at a time.
            let mut progress = false;
            while c.handed.is_empty() {
                match c.out.front() {
                    None => break,
                    Some(Out::Ready(_)) => {
                        let Some(Out::Ready(r)) = c.out.pop_front() else { unreachable!("the front is ready") };
                        c.handed.push_back(r.map(Some));
                    }
                    Some(Out::Projecting(id, _)) => {
                        let Some(done) = self.projected.lock().remove(id) else { break };
                        let Some(Out::Projecting(_, (n, docs_of, hits_of, kept))) = c.out.pop_front() else { unreachable!("the front is projecting") };
                        c.handed.push_back(match done {
                            Ok((built, (user, sys))) => {
                                c.batch.0 += user;
                                c.batch.1 += sys;
                                built.map(|sources| Some((n, sources, docs_of, hits_of, kept)))
                            }
                            Err(_) => Err(Error::Plan("the chunk builder panicked".to_owned())),
                        });
                    }
                }
                progress = true;
            }
            if c.failed || c.ended {
                if c.failed && c.out.is_empty() {
                    c.ended = true;
                }
                return progress;
            }
            // A batch to cut: the next one sent; none until one is, and the builder ends when the channel closes.
            let Some(docs) = c.docs.clone() else {
                return match c.docs_in.try_recv() {
                    Ok(asked) => {
                        c.docs = Some(Arc::new(Docs { asked, resolved: Default::default(), resolved_to: 0.into(), resolving: false.into(), built: Default::default() }));
                        (c.next, c.spawned) = (0, 0);
                        true
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => progress,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        c.ended = true;
                        self.doing.set("done");
                        true
                    }
                };
            };
            let len = docs.asked.len();
            let horizon = (c.next + self.ahead).min(len);
            // The next window, once the last is resolved, while a document within `ahead` needs it.
            let to = docs.resolved_to.load(Acquire);
            if to < horizon && !docs.resolving.swap(true, Acquire) {
                let docs = Arc::clone(&docs);
                jobs.spawn("a window of files resolved", move |jobs| {
                    let from = thread_usage();
                    let end = (to + WINDOW).min(docs.asked.len());
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (&mut *self.resolve.lock())(&docs.asked[to..end])));
                    Self::add(&mut self.jobs_cpu.lock(), from);
                    match r {
                        Ok(Ok(r)) => docs.resolved.lock().extend((to..end).zip(r)),
                        Ok(Err(e)) => {
                            docs.built.lock().insert(to, Err(e));
                        }
                        Err(_) => {
                            docs.built.lock().insert(to, Err(Error::Plan("the chunk builder panicked".to_owned())));
                        }
                    }
                    docs.resolved_to.store(end, Release);
                    docs.resolving.store(false, Release);
                    self.advance(jobs);
                });
                progress = true;
            }
            // Each document within `ahead` whose window is resolved, built as a job: the longest first (by its
            // source's bytes; a memo hit is none). Other threads take a thread's jobs oldest first, and a wait takes
            // its own thread's newest: the long builds start soonest, on other threads, and a waiting thread runs the
            // short ones, so it is not held long past what it waits for (`rerail_accel::exec::waits`).
            let to = docs.resolved_to.load(Acquire);
            let mut ready: Vec<(usize, usize)> = {
                let (resolved, built) = (docs.resolved.lock(), docs.built.lock());
                (c.spawned..horizon.min(to).max(c.spawned))
                    .filter(|i| !built.contains_key(i))
                    .map(|i| (resolved.get(&i).and_then(Option::as_ref).map_or(0, |(_, r)| r.as_ref().err().map_or(0, |u| u.text.len())), i))
                    .collect()
            };
            c.spawned = c.spawned.max(horizon.min(to));
            ready.sort_by(|a, b| b.cmp(a));
            for (_, i) in ready {
                let docs = Arc::clone(&docs);
                jobs.spawn("a document built", move |jobs| {
                    let from = thread_usage();
                    let made = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (self.build)(&docs, i))).unwrap_or_else(|_| Err(Error::Plan("the chunk builder panicked".to_owned())));
                    Self::add(&mut self.jobs_cpu.lock(), from);
                    docs.built.lock().insert(i, made);
                    self.advance(jobs);
                });
                progress = true;
            }
            // The next document is taken unless the front of what is cut holds the builder: ready and waiting for the
            // main thread, or more than `in_flight` chunks projecting.
            let projecting = c.out.iter().filter(|o| matches!(o, Out::Projecting(..))).count();
            let held = match c.out.front() {
                None => false,
                Some(Out::Ready(_)) => true,
                Some(Out::Projecting(id, _)) => projecting > self.in_flight || self.projected.lock().contains_key(id),
            };
            if held {
                self.doing.set(if matches!(c.out.front(), Some(Out::Projecting(..))) && projecting > self.in_flight { "wait for a chunk's projection" } else { "hand on (a chunk)" });
                return progress;
            }
            if c.next < len {
                let Some(made) = docs.built.lock().remove(&c.next) else {
                    self.doing.set("parse");
                    return progress;
                };
                let i = c.next;
                c.next += 1;
                self.take(c, jobs, &docs, i, made);
                return true;
            }
            // A corpus with no document still runs, over empty base relations: its outputs are what the program
            // derives from none (no rows, for a rule that reads a document), not an error.
            if !c.sent {
                self.cut_chunk(c, jobs);
                return true;
            }
            // Every document taken: the batch ends once all it cut is handed on.
            if c.out.is_empty() && c.handed.is_empty() {
                c.handed.push_back(Ok(None));
                c.docs = None;
                return true;
            }
            progress
        }

        /// Takes document `i`: a hit, or a document for the next chunk, which is cut once it is large enough.
        fn take<'s>(&'s self, c: &mut Cut, jobs: &crate::threads::Jobs<'s, '_>, docs: &Docs, i: usize, made: Made)
        where
            'a: 's,
        {
            let last = i + 1 == docs.asked.len();
            let (d, path, _) = &docs.asked[i];
            match made {
                Ok(Some((source_id, Ok(entry)))) => {
                    c.read_sources.extend(source_id.and_then(memo::Content::stored));
                    if let Some(content) = source_id {
                        self.run_docs.lock().expect("the run's documents").insert(path.clone(), (content, *d));
                    }
                    c.hits.push((path.clone(), entry, source_id));
                }
                Ok(Some((source_id, Err(doc)))) => {
                    c.read_sources.extend(source_id.and_then(memo::Content::stored));
                    if let Some(content) = source_id {
                        self.run_docs.lock().expect("the run's documents").insert(path.clone(), (content, *d));
                    }
                    c.pending_nodes += doc.facts.len();
                    c.pending_docs.push((*d, path.clone(), source_id, doc.facts.len() as u64));
                    c.pending.push(doc);
                }
                Ok(None) => {}
                Err(e) => {
                    // The error is the last message: what was cut and not yet handed on is dropped.
                    c.out.clear();
                    c.out.push_back(Out::Ready(Err(e)));
                    c.failed = true;
                    return;
                }
            }
            if c.pending.is_empty() {
                // Only hits so far: handed on in batches, and at the end.
                if !c.hits.is_empty() && (c.hits.len() >= 1024 || last) {
                    c.sent = true;
                    c.out.push_back(Out::Ready(Ok((0, Vec::new(), Vec::new(), std::mem::take(&mut c.hits), Vec::new()))));
                }
                return;
            }
            if c.pending_nodes < self.target.load(std::sync::atomic::Ordering::Relaxed) && !last {
                return;
            }
            self.cut_chunk(c, jobs);
        }

        /// Cuts the documents pending into a chunk, projected as a job.
        fn cut_chunk<'s>(&'s self, c: &mut Cut, jobs: &crate::threads::Jobs<'s, '_>)
        where
            'a: 's,
        {
            c.sent = true;
            // A chunk whose rules ask for guests keeps its documents: the guests are made from them (decision 0055).
            let kept_docs = if self.keep_docs { c.pending.clone() } else { Vec::new() };
            let n = std::mem::take(&mut c.pending_nodes);
            let (docs_of, hits_of) = (std::mem::take(&mut c.pending_docs), std::mem::take(&mut c.hits));
            let pending = std::mem::take(&mut c.pending);
            let id = c.posted;
            c.posted += 1;
            jobs.spawn("a chunk projected", move |jobs| {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (self.project)(pending)));
                self.projected.lock().insert(id, r);
                self.advance(jobs);
            });
            c.out.push_back(Out::Projecting(id, (n, docs_of, hits_of, kept_docs)));
        }

        /// Stops it: the main thread's error or end. Jobs running finish; nothing more starts.
        fn stop(&self) {
            self.stopped.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
    struct Stopper<'b>(&'b std::sync::atomic::AtomicBool);
    impl Drop for Stopper<'_> {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
    let builders = crate::threads::max().max(2);
    let builder = Builder {
        cut: parking_lot::Mutex::new(Cut {
            docs_in,
            docs: None,
            next: 0,
            spawned: 0,
            pending: Vec::new(),
            pending_docs: Vec::new(),
            hits: Vec::new(),
            pending_nodes: 0,
            out: Default::default(),
            posted: 0,
            handed: Default::default(),
            failed: false,
            ended: false,
            sent: false,
            read_sources: Vec::new(),
            parse: (0.0, 0.0),
            batch: (0.0, 0.0),
        }),
        dirty: false.into(),
        stopped: false.into(),
        projected: Default::default(),
        jobs_cpu: Default::default(),
        ahead: builders * 8,
        in_flight: 2,
        resolve: &resolver,
        build: &build,
        project: &project,
        run_docs,
        target: &worker_target,
        keep_docs: slice.is_some(),
        doing: &worker_doing,
    };
    let builder = &builder;
    let worker_cpu = crate::threads::scope(|jobs| -> Result<((f64, f64), (f64, f64), Vec<crate::content_store::SourceId>, Vec<(String, crate::content_store::FileStat, memo::Content)>)> {
    // The main thread's error or end stops the builder: no job it starts after is left running.
    let _stop = Stopper(&builder.stopped);
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
            // What an edit's run restores of the files it did not change (t31 step 0.3): every hit's rows, today.
            crate::code_rule_dsl::logic::profile::decision("memo replay", || {
                let values: usize = hits.iter().map(|(_, e, _)| e.rows.values().map(|(_, v)| v.len()).sum::<usize>()).sum();
                format!("{} entries replayed, {:.1} MB of values", hits.len(), values as f64 * 8.0 / 1e6)
            });
            // Each accepted frame contains its own statistics. Do not also add a scaled chunk estimate:
            // that counted the same warm files twice, and loaded statistics no consumer requested.
            // Each export's rows over the batch, as columns, in two parts: a hit's rows of its own document, which
            // no other file's entry holds, are appended to the export unchecked (`Exports::append_own`); the others
            // (a value document's, kept by each file that asked for it) are kept once, through the export's set.
            // With the statistics kept by file failed, every row goes through the set and is measured.
            let unchecked = keeper.borrow().as_ref().is_some_and(|k| !k.failed());
            let exported: std::collections::HashSet<&str> = split.exports.iter().map(String::as_str).collect();
            let mut batch: BTreeMap<String, (usize, Option<usize>, Vec<Vec<i64>>, Vec<Vec<i64>>)> = BTreeMap::new();
            let mut own_docs: Vec<i64> = Vec::new();
            for (path, entry, content) in hits {
                // The statistics kept by file (decision 0096 §3): a content new to the keeper is counted in.
                if let (Some(k), Some(memo)) = (keeper.borrow_mut().as_mut(), memo) {
                    let _ = memo;
                    k.file(&path, content, entry.key, &entry, &|c, key| counted_entry(&path, c, key));
                }
                // The session's counted exports hold its export rows (0096 step 3): they are not held again. Listed,
                // so that a run whose keeper gives up later holds them after all.
                let counted = skip_counted.get() && content.is_some() && keeper.borrow().as_ref().is_some_and(|k| k.exports_held());
                if counted {
                    hits_unmeasured = true;
                    held_by_count.borrow_mut().push((path.clone(), content.expect("checked"), entry.key));
                }
                nodes += entry.nodes as usize;
                // Its tree counts and base rows are the keeper's too, when counted.
                if !counted {
                    own_totals.add_rows(&entry.rows);
                    if keeper.borrow().is_some() { own_seen.add_rows(&entry.rows); }
                }
                for (rel, names) in stored.iter().filter(|_| !counted) {
                    let Some((width, values)) = entry.rows.get(&memo::base_key(rel)) else { continue };
                    let all = super::super::sql::select::input_columns(rel).ok_or_else(|| Error::Plan(format!("no columns for {rel}")))?;
                    let held = base_rows.entry(rel.clone()).or_insert_with(|| (all.iter().map(|c| (*c).to_owned()).collect(), vec![None; all.len()]));
                    for name in names {
                        let from = all.iter().position(|c| c == name).filter(|c| *c < *width).ok_or_else(|| Error::Plan(format!("{rel} has no column {name}")))?;
                        let to = held.0.iter().position(|h| h == name).ok_or_else(|| Error::Plan(format!("{rel} has no column {name}")))?;
                        held.1[to].get_or_insert_with(Vec::new).extend(values.iter().skip(from).step_by(*width).copied());
                    }
                    // Measured with the evaluated files' rows once the chunks are done ("measure the base rows
                    // held"). They were also kept whole and measured again, so a memo hit's rows counted twice.
                }
                // Gathered by relation across the batch's hits: one statement per relation below, not one per row.
                own_docs.push(entry.key);
                for (rel, (width, values)) in entry.rows {
                    if counted || values.is_empty() || !exported.contains(rel.as_str()) {
                        continue;
                    }
                    let at = split.locations.get(&rel).and_then(|l| l.first().copied()).filter(|a| *a < width && unchecked);
                    let (_, _, own_cols, shared_cols) = batch.entry(rel).or_insert_with(|| (width, at, vec![Vec::new(); width], vec![Vec::new(); width]));
                    for row in values.chunks_exact(width) {
                        let cols = if at.is_some_and(|a| memo::document_of(row[a]) == entry.key) { &mut *own_cols } else { &mut *shared_cols };
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
            for (rel, (width, at, own_cols, cols)) in batch {
                if let Some(at) = at {
                    exports.borrow_mut().append_own(&rel, width, at, own_cols, own_docs.iter().copied());
                }
                if cols.first().is_none_or(Vec::is_empty) {
                    hits_unmeasured |= unchecked;
                    continue;
                }
                let types: Vec<&str> = lowered.program.decls[&rel].iter().map(|(_, t)| t.as_str()).collect();
                let names: Vec<String> = (0..width).map(|c| format!("c{c}")).collect();
                let rows = crate::code_rule_dsl::logic::native::component::columns_source(names.clone(), cols.into_iter().map(Some).collect());
                let inserted = exports.borrow_mut().add(&rel, width, rows.as_ref());
                if unchecked {
                    hits_unmeasured = true;
                } else {
                    totals.add_rows(&rel, inserted, rows.as_ref(), &types);
                }
            }
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
                            record_own_counts(store, own_kept.as_ref(), &memo_program, &key, chunk_nodes_built, &measured, &late_sources);
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
                let documents_len = documents.len();
                let (mut docs, mut sources, mut made) = (documents, sources, guests::Guests::new(label, projection.shape));
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
                    docs.extend(new);
                    let _rebuild = crate::code_rule_dsl::logic::profile::span(|| "batch with guests".into());
                    sources = syntax::with_matchers(&store, docs.clone(), &sets, &chunk_matchers, &vocabulary, binding, &projection).map_err(|e| Error::Plan(e.to_string()))?;
                }
                host_of = std::mem::take(&mut made.host_of);
                askers = made.askers();
                // The guest relations' constant postings, measured above before any guest was made (none then),
                // again over the batch with them: priced at none, `parsed($s, "annotation", $r)` read after bindings
                // looked like a filter, and each binding read every annotation guest (O143). Once per mix, scaled.
                if own && docs.len() > documents_len {
                    let postings = match guest_postings_once.get(&mix) {
                        Some((first, p)) if *first > 0 => p.iter().map(|(k, n)| (k.clone(), (*n as f64 * chunk_nodes_built as f64 / *first as f64).round() as i64)).collect(),
                        _ => {
                            let p = Own::postings(&sources, &constants, |r| guests::READS.contains(&r));
                            guest_postings_once.insert(mix.clone(), (chunk_nodes_built, p.clone()));
                            p
                        }
                    };
                    for ((rel, col, v), n) in postings {
                        stats.measured(&rel, [], [(col, v, n)]);
                    }
                }
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
        // The chunk's text relations (`strings::TEXT_COLUMNS`), whose strings a text test's rows are: a memoised
        // chunk attributes each test row to the files whose text holds its string.
        let mut text_sources: Vec<(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)> = Vec::new();
        for (rel, source) in sources {
            if memoised && crate::code_rule_dsl::logic::strings::TEXT_COLUMNS.iter().any(|(r, _)| *r == rel) {
                text_sources.push((rel.clone(), Arc::clone(&source)));
            }
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
                    // Counted only when profiling, and a source made per looked-up node (an axis, `within`) by its
                    // estimate: its `len` makes every row, every node's ancestors in the chunk (8 s of the pack's
                    // cold run, profiled or not), which its statistics, read off the tree, never needed.
                    if crate::code_rule_dsl::logic::profile::on() {
                        let rows = if source.made_on_lookup() { source.estimated_len() } else { source.len() };
                        crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Value, (rows * source.columns().len()) as u64);
                    }
                    measured_once.insert(key, (source.estimated_len(), measured.clone()));
                    measured
                }
            };
            // The relations the batches make (T has no statistics of them): the transducer's and the tree axes',
            // containment, and the text tests' and regexes'.
            let made_here = |rel: &str| {
                crate::code_rule_dsl::logic::front::is_transduced(rel)
                    || rel == syntax::WITHIN
                    || rel == syntax::SPAN_WITHIN
                    || [crate::code_rule_dsl::logic::strings::RELATION, crate::code_rule_dsl::logic::strings::MATCHES, crate::code_rule_dsl::logic::strings::GROUPS]
                        .iter()
                        .any(|p| rel.starts_with(p))
            };
            // A memoised run measures the base relations the cross-file part reads once, over every file's rows (below).
            if split.base.contains(&rel) && !(memoised && stored.iter().any(|(r, _)| *r == rel)) && (own || !t_stats.knows(&rel) || made_here(&rel)) {
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
                let _span = crate::code_rule_dsl::logic::profile::span(|| "hand off an export (copy, statistics)".into());
                let types: Vec<&str> = lowered.program.decls[rel].iter().map(|(_, t)| t.as_str()).collect();
                let rows = crate::code_rule_dsl::logic::native::component::derived(&db, rel).ok_or_else(|| Error::Plan(format!("{rel} is not held")))?;
                let inserted = exports.borrow_mut().add(rel, types.len(), rows.as_ref());
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
        if memoised && !base_whole {
            memo_why.insert("a base relation the cross-file part reads has no stored columns".to_owned());
        }
        if memoised && chunk_own.is_none() {
            memo_why.insert("no tree counts measured for the chunk".to_owned());
        }
        if let (true, Some(chunk)) = (attributable, &chunk_own) {
            // Each document's share of the chunk's tree statistics, by its nodes, as a chunk mix is scaled.
            for entry in entries.values_mut() {
                let share = entry.nodes as f64 / chunk_nodes_built.max(1) as f64;
                if memo_tree_stats {
                    chunk.scaled(share).put(&mut entry.rows);
                } else {
                    entry.rows.insert(OWN_DOCS.into(), (1, vec![((chunk.docs as f64 * share).round() as i64).max(1)]));
                }
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
            // The rows by file, gathered here and handed to the entries once: rows come in runs of one document,
            // so a run's owners are found once.
            let mut by_file: HashMap<i64, Vec<i64>> = HashMap::new();
            let mut run: Option<(i64, Vec<i64>)> = None;
            let mut row = Vec::with_capacity(width);
            for r in 0..rows.len() {
                let v = rows.value(r, at);
                let d = memo::document_of(v);
                if run.as_ref().is_none_or(|(rd, _)| *rd != d) {
                    let files = owners(v);
                    if files.is_empty() || !files.iter().all(|d| entries.contains_key(d)) {
                        memo_why.insert(format!("export {rel}: {}", if files.is_empty() { "a row naming no document" } else { "a row naming another chunk's document" }));
                        attributable = false;
                        return;
                    }
                    run = Some((d, files));
                }
                row.clear();
                row.extend((0..width).map(|c| rows.value(r, c)));
                for f in &run.as_ref().expect("set").1 {
                    by_file.entry(*f).or_default().extend_from_slice(&row);
                }
            }
            for (f, values) in by_file {
                entries.get_mut(&f).expect("checked").rows.entry(rel.to_owned()).or_insert_with(|| (width, Vec::new())).1.extend(values);
            }
        };
        // Exports remain in native memory; this boundary performs no SQL writes.
        for rel in split.exports.iter().filter(|r| !handed.contains(*r)) {
            // Rows another chunk wrote too (a file-local relation without a location column) are kept once. A kept
            // export goes from the engine to `x` in one statement and is measured in place; one a SQL stratum
            // derived is in its table.
            let types: Vec<&str> = lowered.program.decls[rel].iter().map(|(_, t)| t.as_str()).collect();
            // A relation the engine holds is read from it (an output is not written to a table to be read back).
            if kept.contains(rel) || crate::code_rule_dsl::logic::native::component::derived(&db, rel).is_some() {
                let rows = crate::code_rule_dsl::logic::native::component::derived(&db, rel).ok_or_else(|| Error::Plan(format!("{rel} is not held")))?;
                let inserted = { let _s = crate::code_rule_dsl::logic::profile::span(|| "hold the export".into()); exports.borrow_mut().add(rel, types.len(), rows.as_ref()) };
                { let _s = crate::code_rule_dsl::logic::profile::span(|| "totals".into()); totals.add_rows(rel, inserted, rows.as_ref(), &types); }
                { let _s = crate::code_rule_dsl::logic::profile::span(|| "attribute to files".into()); attribute(rel, rows.as_ref()); }
                if !deferred.is_empty() {
                    mention(&mut mentioned, rows.as_ref());
                }
            } else {
                let rows = Rows::of(&db, rel, &format!("temp.{}", crate::code_rule_dsl::logic::sql::ident(rel)))?;
                let inserted = { let _s = crate::code_rule_dsl::logic::profile::span(|| "hold the export".into()); exports.borrow_mut().add(rel, types.len(), &rows) };
                { let _s = crate::code_rule_dsl::logic::profile::span(|| "totals".into()); totals.add_rows(rel, inserted, &rows, &types); }
                { let _s = crate::code_rule_dsl::logic::profile::span(|| "attribute to files".into()); attribute(rel, &rows); }
                if !deferred.is_empty() {
                    mention(&mut mentioned, &rows);
                }
            }
        }
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
            let _s = crate::code_rule_dsl::logic::profile::span(|| "base rows for the memo".into()); 
            for entry in entries.values_mut() {
                for (rel, _) in &stored {
                    let width = super::super::sql::select::input_columns(rel).map_or(0, <[_]>::len);
                    entry.rows.insert(memo::base_key(rel), (width, Vec::new()));
                }
            }
            // A text test's rows are strings (`strtest_<k>(s)`, decision 0037), no document's: each is the files' whose
            // text holds it (`memo::owner`, as `value_type`'s values are). Found once, for the strings the chunk's
            // string-keyed rows hold.
            let wanted: std::collections::HashSet<i64> = base_seen
                .iter()
                .filter_map(|(rel, at, source)| match memo::owner(rel) {
                    memo::Owner::Text(i) => at.get(i).map(|&c| (c, source)),
                    memo::Owner::Node(_) => None,
                })
                .flat_map(|(c, source)| (0..source.len()).map(move |r| source.value(r, c)))
                .collect();
            let mut string_files: HashMap<i64, BTreeSet<i64>> = HashMap::new();
            if !wanted.is_empty() {
                for (rel, source) in &text_sources {
                    let (Some(columns), Some((_, texts))) = (super::super::sql::select::input_columns(rel), crate::code_rule_dsl::logic::strings::TEXT_COLUMNS.iter().find(|(r, _)| r == rel)) else { continue };
                    let have = source.columns();
                    let at = |name: &str| have.iter().position(|h| h == name);
                    let Some(owner) = columns.first().and_then(|c| at(c)) else { continue };
                    let texts: Vec<usize> = texts.iter().filter_map(|&c| columns.get(c).and_then(|c| at(c))).collect();
                    for r in 0..source.len() {
                        for &c in &texts {
                            let v = source.value(r, c);
                            if wanted.contains(&v) {
                                string_files.entry(v).or_default().extend(owners(source.value(r, owner)));
                            }
                        }
                    }
                }
            }
            'base: for (rel, at, source) in &base_seen {
                let (strings, owner) = match memo::owner(rel) {
                    memo::Owner::Text(i) => (true, i),
                    memo::Owner::Node(i) => (false, i),
                };
                // A value no file's text holds (`value_type` of a constant a rule made): every file of the chunk
                // keeps it, as a rule-made string a joined test reads is kept.
                let made_value = !rel.starts_with(crate::code_rule_dsl::logic::strings::RELATION) && strings;
                let key = memo::base_key(rel);
                // A semi-join's relation: the rows of the nodes the chunk's exports hold, as the cross part keeps.
                let node = semijoin.contains(rel).then(|| source.columns().iter().position(|h| h == "n")).flatten();
                let Some(&first) = at.get(owner) else {
                    memo_why.insert(format!("base {rel}: a row naming no document"));
                    attributable = false;
                    break 'base;
                };
                // The rows by file, handed to the entries once; a run of one document's rows finds its owners once.
                let mut by_file: HashMap<i64, Vec<i64>> = HashMap::new();
                let mut run: Option<(i64, Vec<i64>)> = None;
                let mut row = Vec::with_capacity(at.len());
                for r in (0..source.len()).filter(|&r| node.is_none_or(|n| mentioned.contains(&source.value(r, n)))) {
                    // A row is each of its owners' (a guest's is its host's; a value document's, each asking file's):
                    // rows are sets, so a row kept by several files restores as one.
                    let v = source.value(r, first);
                    let files: Vec<i64> = if strings {
                        // A string of another chunk's file (a path `doc` gives every chunk) is that chunk's to keep.
                        let files: Vec<i64> = string_files.get(&v).map_or_else(Vec::new, |f| f.iter().copied().filter(|f| entries.contains_key(f)).collect());
                        if files.is_empty() && string_files.contains_key(&v) {
                            continue;
                        }
                        // A string no file's text holds (a rule made it): every file of the chunk keeps the row,
                        // where the cross-file rules read the test only joined.
                        if files.is_empty() && (made_value || joined_tests.contains(rel.as_str())) { entries.keys().copied().collect() } else { files }
                    } else {
                        let d = memo::document_of(v);
                        match &run {
                            Some((rd, files)) if *rd == d => files.clone(),
                            _ => owners(v),
                        }
                    };
                    if files.is_empty() || !files.iter().all(|f| entries.contains_key(f)) {
                        memo_why.insert(format!("base {rel}: {}", if files.is_empty() { "a row naming no document" } else { "a row naming another chunk's document" }));
                        attributable = false;
                        break 'base;
                    }
                    row.clear();
                    row.extend(at.iter().map(|c| source.value(r, *c)));
                    for f in &files {
                        by_file.entry(*f).or_default().extend_from_slice(&row);
                    }
                    if !strings {
                        run = Some((memo::document_of(v), files));
                    }
                }
                for (f, values) in by_file {
                    entries.get_mut(&f).expect("checked").rows.get_mut(&key).expect("every entry has every base key").1.extend(values);
                }
            }
        }
        // The chunk's native relations are written to `x`: none is read again.
        crate::code_rule_dsl::logic::native::component::forget_derived(&db);
        if memoised && attributable {
            let _remember = crate::code_rule_dsl::logic::profile::span(|| "memo remember".into());
            // The chunk's entries are written as one wave, not one transaction per document.
            let mut publish: Vec<(memo::Content, String, i64, memo::Entry)> = Vec::with_capacity(chunk_docs.len());
            for (d, path, source_id, _) in &chunk_docs {
                // Counted in before it is remembered: the content it replaces is the memo's still.
                if let (Some(k), Some(entry), Some(memo)) = (keeper.borrow_mut().as_mut(), entries.get(d), memo) {
                    own_seen.add(&Own::taken(&entry.rows));
                    k.file(path, *source_id, *d, entry, &|c, key| counted_entry(path, c, key));
                }
                // Read no more here: handed to the store's writer (decision 0102).
                let (Some(id), Some(entry)) = (source_id, entries.remove(d)) else { continue };
                publish.push((*id, world::store_path(path).to_owned(), *d, entry));
            }
            if let Some(memo) = memo {
                let n = publish.len();
                memo.remember_all(&memo_program, publish).map_err(|e| Error::Plan(format!("memo publication of {n} documents: {e}")))?;
            }
        } else if memoised {
            memo_unattributed += 1;
            if let Some(k) = keeper.borrow_mut().as_mut() {
                k.fail("a chunk's rows not attributed to its files");
            }
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
            let next = crate::code_rule_dsl::logic::profile::wait("the worker's next chunk", &worker_doing, || builder.next(jobs));
            match next {
                Err(()) => return Ok(false),
                Ok(Err(e)) => return Err(e),
                Ok(Ok(None)) => return Ok(true),
                Ok(Ok(Some(chunk))) => process(chunk)?,
            }
        })();
        if got.is_err() {
            builder.stop();
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
        let (ask_tx, ask_rx) = std::sync::mpsc::channel::<(u64, String, Vec<Vec<i64>>)>();
        let (reply_tx, reply_rx) = std::sync::mpsc::channel::<(u64, std::result::Result<Vec<(String, Vec<Vec<i64>>)>, String>)>();
        // The rows the cone starts from: the counted exports less those of the files not seen yet (gone, or a load
        // of the last run's, not yet loaded), when the chunks did not hold the hits' rows again; from here on they do.
        let counted_view: Option<Vec<(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)>> = {
            let k = keeper.borrow();
            match (k.as_ref().filter(|k| k.exports_held() && !held_by_count.borrow().is_empty()), memo) {
                (Some(k), Some(memo)) => {
                    let mut minus = exports::Counted::default();
                    let whole = k.unseen().into_iter().all(|(path, c, key)| match counted_entry(&path, c, key) {
                        Some(e) => per_file.iter().all(|rel| e.rows.get(rel).is_none_or(|(w, v)| minus.patch(rel, *w, v, 1))),
                        None => false,
                    });
                    let counted = k.counted_exports().expect("held");
                    whole.then(|| per_file.iter().map(|r| (r.clone(), counted.source_without(r, lowered.program.decls[r].len(), &minus))).collect())
                }
                _ => None,
            }
        };
        if counted_view.is_none() && !held_by_count.borrow().is_empty() {
            hold_after_all()?;
        }
        skip_counted.set(false);
        let per_file_in: Vec<(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)> =
            counted_view.unwrap_or_else(|| per_file.iter().map(|r| (r.clone(), exports.borrow().snapshot(r, lowered.program.decls[r].len()))).collect());
        // The constants of the run the cone reads (the observer's facts, an earlier phase's exports), bound in its own
        // connection as the main one binds them.
        let constants_in: Vec<(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)> = load
            .rules
            .iter()
            .flat_map(|r| &r.body)
            .filter_map(|l| l.atom())
            .map(|a| a.rel.to_string())
            .collect::<BTreeSet<String>>()
            .into_iter()
            .filter(|r| phased.contains_key(r))
            .map(|r| Ok((r.clone(), atoms_source(&store, prog.decls[&r].len(), phased[&r].iter())?)))
            .collect::<Result<_>>()?;
        let (db_path, x_path, kept_in, world_in) = (path.clone(), cross_path.clone(), kept.clone(), world_inputs);
        // The cone's relations are kept like any (t33 K1), in a fact space of their own; its stratum that asks and
        // answers is clean while what it reads and its answers are as they were (K3).
        // The job's own handle on the store: it shares the store's writer.
        let cone_store: Option<std::path::PathBuf> = durable.is_some().then(|| store.database_path().to_path_buf());
        let (cone_slot, cone_kept_conns) = handed.cone();
        let cone_space = kept_strata::part_space(root, "cone");
        let decls = load.decls.clone();
        // The cone as rounds of its stratified program, maintained by delta (decision 0108): with a store to keep
        // them in. `RERAIL_CONE_ROUNDS=off` runs the gated fixpoint alone.
        let rounds_wanted = cone_store.is_some()
            && std::env::var("RERAIL_CONE_ROUNDS").map_or(true, |v| v != "off" && v != format!("off-{}", phases::current().as_deref().unwrap_or("none")));
        let round = rounds_wanted.then(|| cone_rounds::RoundProgram::of(load, &per_file)).flatten();
        let lowered_round = round.as_ref().map(|r| part(r.program.clone()));
        let per_file_at: Vec<usize> = per_file.iter().map(|rel| split.locations.get(rel).and_then(|l| l.first().copied()).unwrap_or(0)).collect();
        let corpus_docs: std::collections::HashSet<i64> = docs.iter().map(|(k, _)| *k).collect();
        let world_answer_rels: BTreeSet<String> = world_rels.iter().map(|(r, _)| r.clone()).collect();
        // The rounds' frames: a fact space of their own, by phase and program (a phase may run several cones, whose
        // inputs share names), as the cross part's is by phase (`kept_strata::space`).
        let rounds_phase = phases::current().map_or_else(|| "cone/rounds".to_owned(), |phase| format!("cone/{phase}"));
        let rounds_root = root.to_path_buf();
        let rounds_space_of = move |key: &[u8; 32]| kept_strata::part_space(&rounds_root, &format!("{rounds_phase}/{}", crate::content_store::observe::hex(key)));
        // What the journal names as changed since the last run kept its rounds (decision 0105): the roots and cursors
        // recorded with them, the paths named since, and the digest of the file system's answers kept with them.
        let rounds_record = round.as_ref().map(|r| format!("cone-rounds:{}", crate::content_store::observe::hex(&rounds_space_of(&retain::key(&[0; 32], &r.program)))));
        let rounds_journal: Option<(Vec<(std::path::PathBuf, String)>, Vec<std::path::PathBuf>, u64)> = rounds_record.as_ref().and_then(|key| {
            observe(|o| {
                let (read, _) = o.run_record(key)?;
                let mut lines = read.lines();
                let cursors: Vec<String> = lines.next()?.split('\t').map(str::to_owned).collect();
                let roots: Vec<std::path::PathBuf> = lines.next()?.split('\t').map(std::path::PathBuf::from).collect();
                let world: u64 = lines.next()?.parse().ok()?;
                let named = o.changed_since(&roots, &cursors)?;
                Some((roots.into_iter().zip(cursors).collect(), named, world))
            })
        });
        let rounds_world: Arc<std::sync::Mutex<Option<u64>>> = Arc::default();
        let rounds_world_in = Arc::clone(&rounds_world);
        let rounds_withdrawn: Arc<std::sync::Mutex<std::collections::HashSet<i64>>> = Arc::default();
        let rounds_withdrawn_in = Arc::clone(&rounds_withdrawn);
        let rounds_journal_in = rounds_journal.clone();
        // Beside the main thread, which answers its requests: a job of the pool's that takes turns with this one.
        let modes = modes.to_owned();
        let cone = crate::threads::beside("the cone of requests", move || -> Result<(Vec<(String, Vec<Vec<i64>>)>, Option<(usize, usize)>, usize)> {
            let db2 = connections::Cone::take_db(&cone_slot, &db_path)?;
            let statements = Statements::on_as(&db2, ".cone");
            statements.enter("cone");
            // When SQL first runs on it (`vtab::setup_now`), as the run's.
            crate::code_rule_dsl::facts::vtab::defer_setup_undone(&db2, &format!("PRAGMA synchronous=OFF; ATTACH '{}' AS x;", x_path.to_string_lossy().replace('\'', "''")), "DETACH x;");
            super::docs_by_shift(&db2);
            let modules = Modules::default();
            let mut load_stats = load_stats;
            let mut bind = |rel: &str, source: Arc<dyn crate::code_rule_dsl::facts::vtab::Source>| -> Result<()> {
                load_stats.measured(rel, syntax::statistics(source.as_ref()).into_iter().map(|(c, rows, ndv, sq, sqd)| (c, [rows, ndv, sq, sqd])), []);
                modules.bind(&db2, rel, source)?;
                Ok(())
            };
            let mut cone_inputs: Vec<(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)> = Vec::new();
            // The corpus's rows of each per-file relation, for the rounds.
            let corpus_rows: Vec<(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, usize)> = per_file_in.iter().zip(&per_file_at).map(|((r, s), at)| (r.clone(), Arc::clone(s), *at)).collect();
            for (rel, source) in per_file_in {
                cone_inputs.push((rel.clone(), Arc::clone(&source)));
                bind(&rel, source)?;
            }
            let constants_at: BTreeMap<String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>> = constants_in.iter().map(|(r, s)| (r.clone(), Arc::clone(s))).collect();
            for (rel, source) in constants_in {
                cone_inputs.push((rel.clone(), Arc::clone(&source)));
                bind(&rel, source)?;
            }
            for (rel, width) in &world_in {
                let empty = crate::code_rule_dsl::logic::native::component::columns_source((0..*width).map(|c| format!("c{c}")).collect(), vec![Some(Vec::new()); *width]);
                cone_inputs.push((rel.clone(), Arc::clone(&empty)));
                bind(rel, empty)?;
            }
            let cone_handle = cone_store.as_deref().map(|p| connections::Cone::take_store(&cone_slot, p)).transpose().map_err(|e| Error::Plan(e.to_string()))?;
            let mut cone_kept = cone_handle.as_ref().map(|s| kept_strata::Kept::new(s, cone_space, &cone_inputs));
            // Requests go to the main thread with a ticket, and their answers come back with it (decision 0114 §3).
            let asks = std::cell::RefCell::new(Asks { tx: ask_tx, rx: reply_rx, next: 0, early: BTreeMap::new(), posted: HashMap::new(), recorded: None });
            // A request answered in place (the rounds, a stratum needed at once): it waits for its own ticket.
            let mut answer = |rel: &str, rows: &[Vec<i64>]| -> Result<Vec<(String, Vec<Vec<i64>>)>> { asks.borrow_mut().ask(rel, rows) };
            // The rounds from the last run's final round, kept in the store.
            let rounds_key = round.as_ref().map(|r| retain::key(&[0; 32], &r.program));
            let rounds_space = rounds_key.as_ref().map(&rounds_space_of).unwrap_or_default();
            let rounds_in = cone_rounds::Inputs { per_file: &corpus_rows, corpus: &corpus_docs, constants: &constants_at };
            // Only what the stored frames hold that is current is known here; rows are read when a round needs them.
            let mut from_rounds = None;
            if let (Some(r), Some(store), Some(key)) = (&round, &cone_handle, rounds_key) {
                let _s = crate::code_rule_dsl::logic::profile::span(|| "the cone's rounds, from the frames".into());
                let k = retain::Retained::from_frames(store, rounds_space, key, &r.program, &rounds_in.constant_digests())?;
                let missing = r.missing(&k);
                if missing.is_empty() {
                    from_rounds = Some(k);
                } else {
                    crate::code_rule_dsl::logic::profile::decision("the cone's rounds (decision 0108)", || format!("the frames hold no final round: {} of it not current ({})", missing.len(), missing.iter().take(6).cloned().collect::<Vec<_>>().join(", ")));
                }
            }
            let mut keep_rounds: BTreeSet<String> = kept_in.clone();
            if let Some(r) = &round {
                keep_rounds.extend(r.read());
            }
            let rounds_stats = load_stats.clone();
            // A file-system request still answered as the last run kept it: under a root the record holds, and no
            // path the journal names since is it, above it, or (for a directory) an entry of it. Only over the
            // answers the record was kept with.
            let journal = rounds_journal_in.as_ref().filter(|_| std::env::var("RERAIL_CONE_STILL").map_or(true, |v| v != "off")).filter(|(_, _, world)| from_rounds.as_ref().is_some_and(|k| cone_rounds::world_digest(|rel| k.input_digest(rel)) == *world));
            let db_text = &db2;
            let still_fn = journal.map(|(roots, named, _)| {
                move |req: &str, key: &[i64]| -> bool {
                    let Some(text) = key.first().and_then(|a| crate::code_rule_dsl::logic::native::sym_text_of(db_text, *a).ok()) else { return false };
                    let path = std::path::Path::new(&text);
                    roots.iter().any(|(r, _)| path.starts_with(r)) && named.iter().all(|n| !path.starts_with(n) && !(req == "rr_dir_req" && n.parent() == Some(path)))
                }
            });
            let still_ref = still_fn.as_ref().map(|f| f as &dyn Fn(&str, &[i64]) -> bool);
            let on = lowered_round.as_ref().zip(rounds_key).zip(cone_handle.as_ref()).map(|((lowered, key), store)| cone_rounds::On { store, space: rounds_space, db: &db2, modules: &modules, lowered, key, stats: &rounds_stats, keep: &keep_rounds, modes: &modes, still: still_ref });
            let mut gave_up = None;
            if let (Some(r), Some(on), Some(k)) = (&round, &on, from_rounds) {
                match r.run(on, &rounds_in, cone_rounds::Start::Kept(k), cone_rounds::Answers::default(), &mut answer) {
                    Ok(done) => {
                        let mut out = Vec::new();
                        for rel in kept_in.iter().filter(|r| !lowered_round.as_ref().is_some_and(|l| l.program.inputs.contains(*r))) {
                            let store = cone_handle.as_ref().expect("rounds run with a store");
                            out.push((rel.clone(), done.next.read_derived(store, rel)?.ok_or_else(|| Error::Plan(format!("{rel}: not kept by the cone's rounds")))?));
                        }
                        // The answers' relations as the rounds ended: the kept answers that still stand among them.
                        out.extend(done.produced.iter().filter(|(r, _)| kept_in.contains(r) || world_answer_rels.contains(r)).cloned());
                        crate::code_rule_dsl::logic::profile::decision("the cone's rounds (decision 0108)", || {
                            format!("{} rounds from the last run's; {} of {} strata run ({} maintained, {} derived again: {:?}); answered: {:?}; {} file-system requests kept as the journal names no change there; {} rows written back", done.rounds, done.strata.0, done.strata.1, done.maintained, done.derived_again, done.again_why, done.answered, done.still, done.written)
                        });
                        *rounds_world_in.lock().unwrap_or_else(|e| e.into_inner()) = Some(done.world);
                        rounds_withdrawn_in.lock().unwrap_or_else(|e| e.into_inner()).extend(done.withdrawn.iter().copied());
                        return Ok((out, Some(done.strata), done.rounds));
                    }
                    Err(cone_rounds::Gave::Err(e)) => return Err(e),
                    Err(cone_rounds::Gave::Up(why)) => gave_up = Some(why),
                }
            }
            if let Some(why) = &gave_up {
                crate::code_rule_dsl::logic::profile::decision("the cone's rounds (decision 0108)", || format!("given up ({why}): the gated fixpoint runs"));
            }
            // The gated fixpoint, its answers recorded for a closing round.
            asks.borrow_mut().recorded = Some(cone_rounds::Answers::default());
            let mut recording = |rel: &str, rows: &[Vec<i64>]| -> Result<Vec<(String, Vec<Vec<i64>>)>> { asks.borrow_mut().ask(rel, rows) };
            let mut posting = Posting(&asks);
            let mut fixpoint_keeps = kept_in.clone();
            if let Some(r) = &round {
                fixpoint_keeps.extend(r.asks().cloned());
            }
            let got = super::run_atoms_native_answering(&lowered_load, &db2, "main", load_stats, fixpoint_keeps, &modes, &mut recording, Some(&mut posting), cone_kept.as_mut().map(|k| k as &mut dyn super::KeptStrata))?;
            let recorded = asks.borrow_mut().recorded.take().unwrap_or_default();
            let cone_strata = cone_kept.as_ref().and_then(|k| k.strata);
            if let Some(k) = cone_kept {
                k.finish();
            }
            let mut out = Vec::new();
            for rel in &kept_in {
                out.push((rel.clone(), held_rows(&db2, &got, rel, decls[rel].len())?));
            }
            // The closing round over the fixpoint's answers, kept for the next run's rounds; only when the fixpoint
            // ran its strata (a cone kept whole, K3, asked nothing of this run).
            if let (Some(r), Some(on)) = (&round, &on)
                && cone_strata.is_none_or(|(ran, _)| ran > 0)
            {
                let mut asks = BTreeMap::new();
                for ask in r.asks() {
                    asks.insert(ask.clone(), held_rows(&db2, &got, ask, decls[ask].len())?);
                }
                drop(recording);
                let mut refuse = |rel: &str, _: &[Vec<i64>]| -> Result<Vec<(String, Vec<Vec<i64>>)>> { Err(Error::Plan(format!("the closing round asked {rel}"))) };
                match r.run(on, &rounds_in, cone_rounds::Start::Closing(asks), recorded, &mut refuse) {
                    Ok(done) => {
                        crate::code_rule_dsl::logic::profile::decision("the cone's rounds (decision 0108)", || format!("closing round over the fixpoint's answers kept ({} of {} strata; {} rows written back)", done.strata.0, done.strata.1, done.written));
                        *rounds_world_in.lock().unwrap_or_else(|e| e.into_inner()) = Some(done.world);
                    }
                    Err(cone_rounds::Gave::Up(why)) => crate::code_rule_dsl::logic::profile::decision("the cone's rounds (decision 0108)", || format!("the closing round is not kept: {why}")),
                    Err(cone_rounds::Gave::Err(e)) => crate::code_rule_dsl::logic::profile::decision("the cone's rounds (decision 0108)", || format!("the closing round failed: {e}")),
                }
            }
            drop(statements);
            drop(modules);
            connections::Cone::finish(&cone_slot, cone_kept_conns, &db_path, db2, cone_store.zip(cone_handle));
            Ok((out, cone_strata, 0))
        });
        // The answers turn request atoms back into text, warm runs included (nothing else registers them then).
        crate::code_rule_dsl::logic::native::register_symbols(&db)?;
        let text = |atom: i64| -> Result<String> { Ok(crate::code_rule_dsl::logic::native::sym_text_of(&db, atom)?) };
        let atoms = |texts: &[&str]| -> Result<Vec<i64>> { Ok(crate::code_rule_dsl::logic::native::intern_batch(&db, texts.iter().copied())?) };
        let mut pauses = 0;
        // Where an answer's time goes (decision 0114 §6): reported once the cone ends.
        let times = std::cell::RefCell::new(AnswerTimes::default());
        let mut answer_one = |rel: &str, rows: &[Vec<i64>], process: &mut dyn FnMut(Chunk) -> Result<()>| -> Result<Vec<(String, Vec<Vec<i64>>)>> {
            pauses += 1;
            let t_call = std::time::Instant::now();
            let _counted = Finally(|| {
                let mut t = times.borrow_mut();
                t.calls += 1;
                t.rows += rows.len();
                t.total += t_call.elapsed().as_secs_f64();
            });
            if let Some((produced, _)) = WORLD.iter().find(|(_, req)| *req == rel) {
                let _s = crate::code_rule_dsl::logic::profile::span(|| format!("answer {rel}"));
                let mut out = Vec::new();
                for row in rows {
                    let asked = text(row[0])?;
                    if *produced == "dir_entry" {
                        let entries = world.dir(&asked);
                        let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
                        for ((_, is_dir), name) in entries.iter().zip(atoms(&names)?) {
                            out.push(vec![row[0], name, i64::from(*is_dir)]);
                        }
                    } else if *produced == "file_digest" {
                        if let Some(d) = world.digest(&asked) {
                            out.push(vec![row[0], atoms(&[d.as_str()])?[0]]);
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
            let _s = crate::code_rule_dsl::logic::profile::span(|| format!("answer {rel}"));
            let mut fresh = Vec::new();
            let mut again: std::collections::HashSet<i64> = std::collections::HashSet::new();
            let mut loaded_rows = Vec::new();
            // A run of some documents on their own (`phases::maintain`) follows no load: the documents loaded are the
            // last run's, kept.
            // Taken in an order of the requests themselves, not of their atoms: where the load budget binds, a
            // top-level module before a submodule, then by module and path (decision 0108).
            let mut asked_rows: Vec<(String, String, String, &Vec<i64>)> = Vec::new();
            for row in rows.iter().filter(|_| !no_loads) {
                asked_rows.push((text(row[0])?, text(row[1])?, text(row[2])?, row));
            }
            asked_rows.sort_by(|a, b| (a.1.matches('.').count(), &a.1, &a.0).cmp(&(b.1.matches('.').count(), &b.1, &b.0)));
            let t_take = std::time::Instant::now();
            let _taken = Finally(|| times.borrow_mut().take += t_take.elapsed().as_secs_f64());
            for (asked, module, grammar, row) in asked_rows {
                // A load asked again (its stratum's requests answered again, t33 K3) is answered alike: the same
                // document, its rows from the exports, not sent again.
                if let Some(doc) = world.loaded_as(&asked) {
                    if let Some((key, _)) = loaded_docs.iter().find(|(_, p)| *p == doc.path) {
                        loaded_rows.push(vec![atoms(&[doc.path.as_str()])?[0], row[1], row[2]]);
                        again.insert(*key);
                    }
                    continue;
                }
                if let Some(doc) = world.load(&asked, &module, &grammar, label, &corpus) {
                    loaded_rows.push(vec![atoms(&[doc.path.as_str()])?[0], row[1], row[2]]);
                    fresh.push(doc);
                }
            }
            let mut out = vec![(world::LOADED.to_owned(), loaded_rows)];
            if fresh.is_empty() && again.is_empty() {
                out.extend(per_file.iter().map(|r| (r.clone(), Vec::new())));
                return Ok(out);
            }
            if fresh.is_empty() {
                for rel in &per_file {
                    let at = split.locations.get(rel).and_then(|l| l.first().copied()).ok_or_else(|| Error::Plan(format!("{rel}: no location column")))?;
                    let width = lowered.program.decls[rel].len();
                    out.push((rel.clone(), exports.borrow().rows_of(rel, width, at, &again)));
                }
                return Ok(out);
            }
            // A document's key is a function of its path (decision 0052).
            all_paths.extend(fresh.iter().map(|d| d.path.clone()));
            let keys = memo::doc_keys(&all_paths);
            let keys = &keys[keys.len() - fresh.len()..];
            loaded_docs.extend(keys.iter().copied().zip(fresh.iter().map(|d| d.path.clone())));
            {
                let mut t = times.borrow_mut();
                t.fresh += fresh.len();
                t.again += again.len();
            }
            drop(_taken);
            docs_tx.send(keys.iter().copied().zip(fresh).map(|(k, d)| (k, d.path, Some(Arc::from(d.module.as_str())))).collect()).map_err(|_| Error::Plan("the chunk builder ended".to_owned()))?;
            // The chunks' file-local evaluation, on this thread (`process`), apart from the wait for them to be built.
            let t_drain = std::time::Instant::now();
            let mut evaluate = 0.0;
            let drained = drain(&mut |c: Chunk| {
                let t = std::time::Instant::now();
                let r = process(c);
                evaluate += t.elapsed().as_secs_f64();
                r
            });
            {
                let mut t = times.borrow_mut();
                t.evaluate += evaluate;
                t.built += t_drain.elapsed().as_secs_f64() - evaluate;
            }
            if !drained? {
                return Err(Error::Plan("the chunk builder ended before a load's documents".to_owned()));
            }
            let t_rows = std::time::Instant::now();
            let _rows = Finally(|| times.borrow_mut().rows_of += t_rows.elapsed().as_secs_f64());
            // The new documents' rows of the per-file relations the cone reads, from the exports the chunks added
            // (`exports`, held in memory), by their first location column.
            let mut new: std::collections::HashSet<i64> = keys.iter().copied().collect();
            new.extend(again);
            for rel in &per_file {
                let at = split.locations.get(rel).and_then(|l| l.first().copied()).ok_or_else(|| Error::Plan(format!("{rel}: no location column")))?;
                let width = lowered.program.decls[rel].len();
                out.push((rel.clone(), exports.borrow().rows_of(rel, width, at, &new)));
            }
            Ok(out)
        };
        let mut idle = 0.0;
        while let Some((ticket, rel, rows)) = {
            let t = std::time::Instant::now();
            let got = cone.wait_for("the main thread: the cone's requests", || match ask_rx.try_recv() {
                Ok(asked) => Some(Some(asked)),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(None),
            });
            idle += t.elapsed().as_secs_f64();
            got
        } {
            let reply = answer_one(&rel, &rows, &mut process).map_err(|e| e.to_string());
            if let Ok(given) = &reply {
                for (r, rows) in given {
                    if let Some(kept) = answered.get_mut(r) {
                        kept.extend(rows.iter().cloned());
                    }
                }
            }
            if reply_tx.send((ticket, reply)).is_err() {
                break;
            }
        }
        drop(reply_tx);
        drop(answer_one);
        {
            let mut t = times.into_inner();
            t.idle = idle;
            t.report();
        }
        (loading_rows, cone_kept, cone_rounds) = cone.join().map_err(|_| Error::Plan("the cone of requests panicked".to_owned()))??;
        // The documents the rounds loaded and took back (decision 0108): neither the run's documents nor rows of its
        // exports.
        let gone = std::mem::take(&mut *rounds_withdrawn.lock().unwrap_or_else(|e| e.into_inner()));
        if !gone.is_empty() {
            loaded_docs.retain(|(d, _)| !gone.contains(d));
            for rel in &per_file {
                let width = load.decls[rel].len();
                let at = split.locations.get(rel).and_then(|l| l.first().copied()).unwrap_or(0);
                let held = exports.borrow().snapshot(rel, width);
                let mut row = vec![0i64; width];
                let all: Vec<usize> = (0..width).collect();
                let mut kept_rows = Vec::with_capacity(held.len());
                for i in 0..held.len() {
                    held.row_at(i, &all, &mut row);
                    if width == 0 || !gone.contains(&memo::document_of(row[at])) {
                        kept_rows.push(row.clone());
                    }
                }
                exports.borrow_mut().install(rel, width, cone_rounds::of_rows(width, kept_rows.iter()));
            }
            crate::code_rule_dsl::logic::profile::decision("the cone's rounds (decision 0108)", || format!("{} documents loaded and taken back: no load the run's own rows ask for names them", gone.len()));
        }
        // The rounds kept: the journal's cursors of the roots read this run are recorded with them (a root not read
        // this run keeps the cursor it was recorded with: what changed since is still named from there).
        if let (Some(key), Some(world)) = (&rounds_record, *rounds_world.lock().unwrap_or_else(|e| e.into_inner())) {
            observe(|o| {
                let mut at = o.watched();
                for (root, cursor) in rounds_journal.iter().flat_map(|(old, _, _)| old) {
                    if !at.iter().any(|(r, _)| r == root) {
                        at.push((root.clone(), cursor.clone()));
                    }
                }
                if !at.is_empty() {
                    let cursors: Vec<&str> = at.iter().map(|(_, c)| c.as_str()).collect();
                    let roots: Vec<String> = at.iter().map(|(r, _)| r.to_string_lossy().into_owned()).collect();
                    o.put_run_record(key, &format!("{}\n{}\n{world}", cursors.join("\t"), roots.join("\t")), &[]);
                }
            });
        }
        // What the main thread answered, for the relations the cone did not hand on whole (its rounds do).
        for (rel, rows) in answered {
            if !loading_rows.iter().any(|(r, _)| *r == rel) {
                loading_rows.push((rel, rows));
            }
        }
        // Every file seen: the keeper ends here, so the digests read the run's exports (the counted ones).
        finished = Some(finish()?);
        cone_inputs = cone_digests(&exports.borrow(), &per_file, &split.locations, &lowered.program, docs.iter().chain(&loaded_docs))?;
        let summary = world.summary(pauses);
        consulted.extend(world.consulted());
        crate::code_rule_dsl::logic::profile::decision("file system and loaded documents (t21)", || summary);
    } else {
        // No requests: no cone, so each document reads into it nothing.
        cone_inputs = docs.iter().map(|(_, p)| (p.clone(), String::new())).collect();
    }
    // The builder ends with its channel.
    docs_out = None;
    drop(docs_out);
    main_doing.set("done");
    crate::code_rule_dsl::logic::profile::wait("the worker's end", &worker_doing, || {
        rerail_accel::exec::help_until_as("the main thread: the builder's end", || {
            builder.advance(jobs);
            builder.cut.lock().ended.then_some(())
        })
    });
    // The builder's CPU: its own steps and its documents' jobs as parse time, its chunks' projections as batch time.
    let mut c = builder.cut.lock();
    let jobs_cpu = *builder.jobs_cpu.lock();
    Ok(((c.parse.0 + jobs_cpu.0, c.parse.1 + jobs_cpu.1), c.batch, std::mem::take(&mut c.read_sources), std::mem::take(&mut *stats_read.lock())))
    })?;
    (parse_cpu.0, parse_cpu.1, batch_cpu.0, batch_cpu.1) = (worker_cpu.0.0, worker_cpu.0.1, worker_cpu.1.0, worker_cpu.1.1);
    // The reference graph over every chunk's files, bound for the cross-file part with its statistics.
    if let Some(references) = references {
        for (rel, columns, values) in references.relations(store, &read)? {
            let source = crate::code_rule_dsl::logic::native::component::columns_source(columns, values);
            totals.add(rel, source.len(), &syntax::statistics(source.as_ref()));
            cross_inputs.push((rel.to_owned(), Arc::clone(&source)));
            modules.bind(&db, rel, source)?;
        }
    }
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
        // Behind the run (decision 0104): a new content's source is written by then, by the tape's write queued before.
        let (root, read) = (canonical_root.clone(), worker_cpu.3.clone());
        store.write_back(
            "the files' stats",
            read.len() * 64,
            vec![(stats_key(&root), None)],
            Box::new(move |store| {
                let mut ids = Vec::with_capacity(read.len());
                // A busy-store retry must leave the captured inputs available.
                for (path, stat, content) in &read {
                    if let Some(id) = content.resolve(store)? {
                        ids.push((path.clone(), *stat, id));
                    }
                }
                store.record_corpus_stats(&root, &ids, started_ns)
            }),
        );
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
        record_run(store, root, &run_sources, &memo_program, &program_plans, &relations, &late_sources, reading, !phases::in_phase() && !phase0::active() && options.get(constants::OWN).is_none())?
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
        let _s = crate::code_rule_dsl::logic::profile::span(|| "measure the base rows held".into());
        // A relation no file had rows of (none evaluated, or none with rows) is held empty: the cross-file part binds it.
        for (rel, names) in &stored {
            if !base_rows.contains_key(rel) {
                let have: Vec<String> = projection.columns(rel).unwrap_or_default().iter().map(|c| (*c).to_owned()).collect();
                let values = have.iter().map(|c| names.contains(c).then(Vec::new)).collect();
                base_rows.insert(rel.clone(), (have, values));
            }
        }
        // Measured through the source the cross-file part binds: it keeps its statistics, so a session's next run
        // reads them for its `r__old` (0094 §4). A column no cross-file rule names is not held: the planner still
        // asks for it, so it is stated as a key.
        // In a session, rows as the last run had them are its source.
        let parts = session.as_ref().map(|s| s.cross.lock().expect("the session's cross-file parts"));
        // Kept by file (`keptstats`): every file's rows counted in or out as its content changed, none measured.
        let kept = match finished.take() {
            Some(kept) => kept,
            None => finish()?,
        };
        let mut keeper = keeper.into_inner();
        // The base relations' rows and the tree counts of the hits not held again (0096 step 3): the counted rows are
        // the run's, and the kept tree counts beyond those of the files it saw. Given up, the skipped hits' are taken
        // from the memo, as the replay would have.
        match keeper.as_ref().and_then(|k| k.counted_base().zip(k.counted_own())).filter(|_| installed.get()) {
            Some(((counted, rels), own)) => {
                let _s = crate::code_rule_dsl::logic::profile::span(|| "the counted base rows".into());
                for (rel, _, from) in rels {
                    let Some(held) = base_rows.get_mut(rel) else { continue };
                    let all = super::super::sql::select::input_columns(rel).unwrap_or_default();
                    let mut columns = counted.columns_each(rel, from.len()).into_iter();
                    for c in from {
                        let to = held.0.iter().position(|h| h == all[*c]).ok_or_else(|| Error::Plan(format!("{rel} has no column {}", all[*c])))?;
                        held.1[to] = Some(columns.next().expect("one a column"));
                    }
                }
                let mut skipped = own.clone();
                skipped.sub(&own_seen);
                own_totals.add(&skipped);
            }
            None => {
                let memo = memo.filter(|_| !given_up.borrow().is_empty());
                for (path, content, key) in std::mem::take(&mut *given_up.borrow_mut()).into_iter().filter(|_| memo.is_some()) {
                    let entry = memo.expect("checked").lookup(content, world::store_path(&path), key, &memo_program).map_err(|e| Error::Plan(e.to_string()))?.ok_or_else(|| Error::Plan(format!("{path}: its memo entry is gone")))?;
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
                    }
                }
            }
        }
        // The exports' statistics as kept by file: a memo hit's rows are not measured again. An export whose kept rows
        // are not the rows held (a row handed to two files is held once) is measured from the rows held, as is every
        // export when the keeper was given up after the hits went unmeasured.
        if hits_unmeasured || kept.is_some() {
            let _s = crate::code_rule_dsl::logic::profile::span(|| "the exports' statistics, kept or measured".into());
            let tested = constants_tested(&lowered.program);
            let (mut from_kept, mut measured_held) = (0usize, Vec::<String>::new());
            for rel in &split.exports {
                let decl = &lowered.program.decls[rel];
                let types: Vec<&str> = decl.iter().map(|(_, t)| t.as_str()).collect();
                let held = exports.borrow().len(rel);
                match kept.as_ref().and_then(|m| m.get(rel)).filter(|r| r.rows() == held as i64) {
                    Some(r) => {
                        totals.forget(rel);
                        totals.add(rel, held, &r.statistics(&|_| true));
                        // The planner prices a constant by its count (`const_freq`): a value column's counts as kept
                        // while few-valued, else those of the constants the rules test, counted in the rows held.
                        let mut count: Vec<(usize, i64)> = Vec::new();
                        for c in (0..decl.len()).filter(|c| types[*c] != "node") {
                            match r.exact(c) {
                                Some(vals) => totals.consts.extend(vals.map(|(v, n)| (rel.clone(), format!("c{c}"), v, n))),
                                None => count.extend(tested.get(rel.as_str()).into_iter().flatten().filter(|(tc, _)| *tc == c)),
                            }
                        }
                        if !count.is_empty() {
                            let src = exports.borrow_mut().source(rel, decl.len());
                            for (c, v) in count {
                                let n = (0..src.len()).filter(|&i| src.value(i, c) == v).count();
                                totals.consts.push((rel.clone(), format!("c{c}"), v, n as i64));
                            }
                        }
                        from_kept += 1;
                    }
                    None if hits_unmeasured => {
                        let _m = crate::code_rule_dsl::logic::profile::span(|| format!("measure export {rel}"));
                        totals.forget(rel);
                        let src = exports.borrow_mut().source(rel, decl.len());
                        totals.add_rows(rel, src.len(), src.as_ref(), &types);
                        measured_held.push(rel.clone());
                    }
                    None => {}
                }
            }
            crate::code_rule_dsl::logic::profile::decision("exports' statistics kept by file", || format!("{from_kept} kept; {} measured from the rows held: {}", measured_held.len(), measured_held.join(", ")));
        }
        match (session.as_ref(), keeper.take(), memo) {
            (Some(s), Some(k), _) => {
                s.stats.lock().expect("the session's statistics").insert(memo_program, k);
            }
            // Written as one blob with the run, whose memo entries it names by content (0096 §2): a later process
            // patches it. A run that could not keep them writes it empty, so the next counts every file in.
            (None, Some(k), Some(memo::Memo::Stored(store))) if kept.is_none() || k.counted != (0, 0) => {
                let _s = crate::code_rule_dsl::logic::profile::span(|| "write the kept statistics, queued".into());
                store.queue_stat_blob(memo_program, k.encode());
            }
            _ => {}
        }
        for (rel, (columns, values)) in std::mem::take(&mut base_rows) {
            let rows = values.iter().flatten().map(Vec::len).next().unwrap_or(0);
            base_held.insert(rel.clone(), columns.iter().zip(&values).map(|(c, v)| v.is_some().then(|| c.clone())).collect());
            // A semi-join's relation holds only the rows its exports name: measured, as the keeper counts every row.
            let statistics = kept.as_ref().and_then(|m| m.get(&memo::base_key(&rel))).filter(|r| !semijoin.contains(&rel) && r.rows() == rows as i64).map(|r| {
                let held: BTreeSet<&str> = columns.iter().zip(&values).filter(|(_, v)| v.is_some()).map(|(c, _)| c.as_str()).collect();
                let by_name: HashMap<String, (String, i64, i64, i64, i64)> = r.statistics(&|c| held.contains(c)).into_iter().map(|m| (m.0.clone(), m)).collect();
                let n = rows as i64;
                columns.iter().map(|c| by_name.get(c).cloned().unwrap_or((c.clone(), n, n, n, n))).collect::<Vec<_>>()
            });
            let source = match &statistics {
                Some(st) => crate::code_rule_dsl::logic::native::component::columns_source_measured(columns, values, st.clone()),
                None => crate::code_rule_dsl::logic::native::component::columns_source(columns, values),
            };
            let source = retain::kept_source(parts.as_deref(), &rel, source, &mut input_digests);
            let statistics = statistics.unwrap_or_else(|| syntax::statistics(source.as_ref()));
            totals.add(&rel, rows, &statistics);
            base_sources.insert(rel, source);
        }
    }
    // What the chunks handed on: each export's rows, summed over the chunks and the memo's hits (decision 0099's measure).
    crate::code_rule_dsl::logic::profile::decision("rows handed on to the cross-file part", || {
        let mut by: Vec<(usize, &String)> = split.exports.iter().filter_map(|r| totals.rels.get(r).map(|t| (t.0, r))).collect();
        by.sort_unstable_by(|a, b| b.cmp(a));
        let all: usize = by.iter().map(|b| b.0).sum();
        format!("{all} rows of {} exports; the largest: {}", by.len(), by.iter().take(8).map(|(n, r)| format!("{r} {n}")).collect::<Vec<_>>().join(", "))
    });
    // The cross-file part's statistics: T's for the base relations, the chunks' totals for the rest.
    let applying = crate::code_rule_dsl::logic::profile::span(|| "apply the chunks' totals".into());
    let mut cross_stats = t_stats;
    if own {
        if memo_tree_stats {
            own_totals.apply(&mut cross_stats);
        } else {
            cross_stats.nodes = nodes as f64;
            cross_stats.docs = Some(own_totals.docs);
        }
    }
    totals.apply(&mut cross_stats);
    // Applied: the chunks' per-value counts are not read again.
    totals.vals = BTreeMap::new();
    repository_stats(&mut cross_stats);
    drop(applying);
    // RERAIL_DUMP_STATS=a,b: what the cross-file part plans with for those relations (a debugging aid); `exports`
    // names every export.
    if let Ok(rels) = std::env::var("RERAIL_DUMP_STATS") {
        for rel in rels.split(',') {
            if rel == "exports" {
                split.exports.iter().for_each(|e| eprintln!("cross stats {}", cross_stats.describe(e)));
            } else {
                eprintln!("cross stats {}", cross_stats.describe(rel));
            }
        }
    }
    // With no documents there was no batch to bind or measure base relations.
    // They are known empty, but global rules (including absence checks) still execute.
    if docs.is_empty() {
        for rel in &split.base {
            if repository.iter().any(|(name, _, _)| name == rel) {
                continue;
            }
            let columns = projection.columns(rel).ok_or_else(|| Error::Plan(format!("no columns for {rel}")))?;
            let empty = crate::code_rule_dsl::logic::native::component::columns_source(
                columns.iter().map(|name| (*name).to_owned()).collect(), vec![Some(Vec::new()); columns.len()]);
            cross_stats.measured(rel, columns.iter().map(|name| ((*name).to_owned(), [0, 0, 0, 0])), []);
            cross_inputs.push((rel.clone(), Arc::clone(&empty)));
            modules.bind(&db, rel, empty)?;
        }
    }

    // The cross-file part reads every input in place from `main`: the base relations held in memory, and the
    // exports, read from `x` once each.
    // Without the memo the base rows are still columns: sources now.
    {
        let parts = session.as_ref().map(|s| s.cross.lock().expect("the session's cross-file parts"));
        for (rel, (columns, values)) in std::mem::take(&mut base_rows) {
            base_held.insert(rel.clone(), columns.iter().zip(&values).map(|(c, v)| v.is_some().then(|| c.clone())).collect());
            let source = crate::code_rule_dsl::logic::native::component::columns_source(columns, values);
            base_sources.insert(rel.clone(), retain::kept_source(parts.as_deref(), &rel, source, &mut input_digests));
        }
    }
    // What the memo entries written by this run hold (t31 A2): the largest relations, and any join pushed into the
    // chunks (`fl<n>_<rule>`, decision 0051) that exports more rows than four per node of its files, the shape of a
    // composition left unfiltered (F823's `outer`, 15 M rows for one 291 KB file).
    if crate::code_rule_dsl::logic::profile::on() {
        let (nodes, by_rel) = memo::sizes();
        if nodes > 0 {
            let total: u64 = by_rel.values().map(|v| v.1).sum();
            let mut top: Vec<(&String, &(u64, u64))> = by_rel.iter().collect();
            top.sort_by(|a, b| b.1.1.cmp(&a.1.1));
            crate::code_rule_dsl::logic::profile::decision("memo entries written", || {
                let named: Vec<String> = top.iter().take(8).map(|(r, (rows, values))| format!("{r} {:.1} MB ({rows} rows, {:.0}%)", *values as f64 * 8.0 / 1e6, 100.0 * *values as f64 / total.max(1) as f64)).collect();
                format!("{:.1} MB of values over {nodes} nodes: {}", total as f64 * 8.0 / 1e6, named.join("; "))
            });
            for (rel, (rows, _)) in &top {
                let pushed = rel.strip_prefix("fl").and_then(|s| s.split_once('_')).is_some_and(|(n, _)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
                if pushed && *rows > 4 * nodes {
                    crate::code_rule_dsl::logic::profile::decision("pushed joins over 4 rows per node", || format!("{rel}: {rows} rows over {nodes} nodes ({:.1} per node)", *rows as f64 / nodes as f64));
                }
            }
        }
    }
    crate::code_rule_dsl::logic::profile::decision("held for the cross-file part", || {
        let mut held: Vec<(usize, String)> = base_sources
            .iter()
            .map(|(rel, source)| (source.len() * source.columns().len(), format!("{rel}({})", source.columns().join(", "))))
            .collect();
        held.sort_by(|a, b| b.0.cmp(&a.0));
        let total: usize = held.iter().map(|h| h.0).sum();
        let top: Vec<String> = held.iter().take(12).map(|(n, r)| format!("{r} {:.1} MB", *n as f64 * 8.0 / 1e6)).collect();
        format!("{} base relations, at most {:.1} MB of values in memory: {}", held.len(), total as f64 * 8.0 / 1e6, top.join("; "))
    });
    let binding_base = crate::code_rule_dsl::logic::profile::span(|| "bind the base rows held".into());
    for (rel, source) in std::mem::take(&mut base_sources) {
        // A base relation the cross-file part reads that T's statistics do not cover (a simplified rule moved its read
        // there, `parsed` after a filter was inlined): measured from the rows held (kept with them). A column not held
        // is read by no rule; it is taken as a key.
        if !cross_stats.knows(&rel) {
            cross_stats.measured(&rel, syntax::statistics(source.as_ref()).into_iter().map(|(c, rows, ndv, sq, sqd)| (c, [rows, ndv, sq, sqd])), []);
        }
        cross_inputs.push((rel.clone(), Arc::clone(&source)));
        modules.bind(&db, &rel, source)?;
    }
    // One fixpoint (no bounded rounds) reads each export from `x` when its first reader runs and drops it, and each
    // base relation held for it, after its last; rounds read them all, every round. A session's run keeps its
    // cross-file inputs for the next (`retain`, decision 0077), so it binds them all.
    drop(binding_base);
    let binding_exports = crate::code_rule_dsl::logic::profile::span(|| "bind the exports".into());
    // Without a session, a run with the store keeps its cross-file part as a session does, in the store's frames
    // (t33 K4): it starts from them and maintains them from the delta of its inputs. `RERAIL_CROSS_K4=off` keeps the
    // strata as before (whole strata run or kept, `kept_strata`), to compare.
    let k4 = durable.is_some() && std::env::var("RERAIL_CROSS_K4").map_or(true, |v| v != "off");
    let one_fixpoint = !cross.program.rules.is_empty() && session.is_none() && !k4 && {
        let bounded = cross_rounds::bound(&cross.program, &split.same_doc)?;
        bounded.hops.is_empty() || cross_rounds::declared(&prog).is_none()
    };
    // The import components (decision 0076) run before the cross part, with no hooks: what they read is bound now.
    let eager: BTreeSet<&str> = cross_plan.iter().flat_map(|p| p.components.rules.iter()).flat_map(|r| r.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.as_str())).collect();
    // A new process that maintains its cross-file part from the store (t32 4.1) runs it as a session does: its
    // inputs bound before the run, not loaded as their first reader runs. Not a part whose requests the run answers
    // as it goes (0065 §2a, phase 0's loads): their answers are the loader's, which no kept row stands for.
    let kept_part_on = session.is_none()
        && durable.is_some()
        && one_fixpoint
        && kept_strata::part_wanted()
        && cross.program.answers.is_empty()
        && crate::code_rule_dsl::logic::answers::produced(&cross.program).is_empty()
        && !phase0::active();
    for rel in &split.exports {
        if one_fixpoint && !kept_part_on && !eager.contains(rel.as_str()) {
            let width = lowered.program.decls[rel].len();
            let empty = crate::code_rule_dsl::logic::native::component::columns_source((0..width).map(|c| format!("c{c}")).collect(), vec![Some(Vec::new()); width]);
            modules.bind(&db, rel, empty)?;
            continue;
        }
        let source = exports.borrow_mut().source(rel, lowered.program.decls[rel].len());
        // As a base relation's (`retain::kept_source`): the last run's copy if its rows are the same, else this one with
        // the last run's sorted columns carried into it (decision 0101 §5).
        let source = {
            let parts = session.as_ref().map(|s| s.cross.lock().expect("the session's cross-file parts"));
            retain::kept_source(parts.as_deref(), rel, source, &mut input_digests)
        };
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
            // Loading answers are inputs to maintenance as well as to execution. Omitting them here makes a
            // retained output look unchanged after an import edit, although its bound input has new rows.
            let source = {
                let parts = session.as_ref().map(|s| s.cross.lock().expect("the session's cross-file parts"));
                retain::kept_source(parts.as_deref(), &rel, source, &mut input_digests)
            };
            if let Some((_, held)) = cross_inputs.iter_mut().find(|(r, _)| r == &rel) {
                *held = Arc::clone(&source);
            } else {
                cross_inputs.push((rel.clone(), Arc::clone(&source)));
            }
            cross_stats.measured(&rel, syntax::statistics(source.as_ref()).into_iter().map(|(c, rows, ndv, sq, sqd)| (c, [rows, ndv, sq, sqd])), []);
            modules.bind(&db, &rel, source)?;
        }
    }
    drop(binding_exports);
    // The chunks' freed pages go back before the cross part builds its own.
    { let _s = crate::code_rule_dsl::logic::profile::span(|| "release the chunks' pages".into()); crate::code_rule_dsl::prof::release(); }
    phase("stats", statements.count());
    statements.enter("cross");
    let mut rounds = cross_rounds::Rounds::default();
    // Strata a session's run evaluated, of all (decision 0077).
    let mut cross_strata: Option<(usize, usize)> = None;
    let mut cross_kept: Option<(usize, usize)> = None;
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
                let text = crate::code_rule_dsl::logic::native::symbol_text(&db, r[2])?;
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
        // No component found (a new store): `rr_cross_doc` is empty and every guard passes, so none is run.
        if found.hits.is_empty() && cross_mode != cross_memo::Mode::Verify {
            let dropped = cross_memo::drop_guards(&mut cross.program);
            crate::code_rule_dsl::logic::profile::decision("import components (decision 0076)", || format!("no component found: {dropped} guards on `rr_cross_doc` not run"));
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
    // A session's outputs whose strata were clean, written by its last run of this program to this directory: their
    // files are as they were, and their row counts. And that program's key, to record what this run writes.
    let mut unchanged_outputs: HashMap<String, usize> = HashMap::new();
    let mut written_key: Option<[u8; 32]> = None;
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
            // A new process keeps its cross-file part in the store, and maintains it as a session does (t32 4.1):
            // the session's branch, its kept part read from the store's frames and written back after the run.
            let kept_part: Option<std::sync::Mutex<HashMap<[u8; 32], retain::Retained>>> =
                kept_part_on.then(Default::default);
            // A run with no session keeps its part in the frames (K4): held here for the run, read from them and
            // written back to them. (`kept_part` is t32 4.1's form of it, taken only with `RERAIL_CROSS_K4=off`.)
            let from_frames: Option<std::sync::Mutex<HashMap<[u8; 32], retain::Retained>>> = (session.is_none() && k4).then(Default::default);
            match session.as_ref().map(|s| &s.cross).or(kept_part.as_ref()).or(from_frames.as_ref()) {
                // A session's run evaluates the strata whose inputs changed, the others' rows kept (decision 0077).
                Some(cell) => {
                    // A session maintains its cross-file part from the rows the last run kept: each relation is
                    // stored, not recomputed in its readers (decision 0099), so the next run has its old rows. An
                    // inlined one left none, and every stratum over it was derived again (about 100 a step at O125,
                    // against none stored). A relation the program declares `inline` stays so;
                    // `RERAIL_SESSION_INLINE=on` inlines as a full run does, to compare.
                    let mut stored = cross.clone();
                    let inline_too = std::env::var("RERAIL_SESSION_INLINE").is_ok_and(|v| v == "on");
                    // The relations the program declares `inline`: inlined in a session as in a full run. The session
                    // keeps no rows of theirs (the run's `Outcome::inlined` names them), and the strata over them are
                    // derived again each run: the cost the author chose by writing `inline`.
                    let declared_inline: BTreeSet<String> = cross.program.decls.keys().filter(|r| cross.program.qualified(r, "inline")).cloned().collect();
                    // Its recursive relations keep each row's level, to be maintained by them next run (decision 0090;
                    // `levels`: only the session asks for them, so a full run holds none).
                    let levels = maintain::levels_wanted();
                    for rel in cross.program.rules.iter().map(|r| r.head.rel.clone()).collect::<std::collections::BTreeSet<_>>() {
                        if levels {
                            stored.program.qualifiers.entry(rel.to_string()).or_default().insert("levels".to_owned());
                        }
                        if !inline_too && !declared_inline.contains(rel.as_str()) {
                            stored.program.qualifiers.entry(rel.to_string()).or_default().insert("no_inline".to_owned());
                        }
                    }
                    let cross = stored;
                    // Exact maintenance by counting (the default), each rule's derivations held as its support
                    // relation (`supports`); `RERAIL_MAINTAIN=dred` compares with delete-and-rederive.
                    let counted;
                    let cross = if supports::counting() {
                        let (c, through, kept) = supports::supports(&cross);
                        crate::code_rule_dsl::logic::profile::decision("maintenance by counting (supports)", || format!("{through} rules through support relations, {kept} kept their form"));
                        if std::env::var_os("RERAIL_SPIKE_DELTA").is_some() {
                            eprintln!("O125 counting: {through} rules through supports, {kept} kept their form");
                        }
                        counted = c;
                        &counted
                    } else {
                        &cross
                    };
                    // Each linear cost of a session's run is its own span (decision 0090, step 1).
                    let span = |name: &'static str| crate::code_rule_dsl::logic::profile::span(move || format!("session/{name}"));
                    let s = span("digest inputs");
                    // A base relation's digest was taken when its source was made (`retain::kept_source`).
                    let digests: HashMap<String, u64> = cross_inputs
                        .iter()
                        .map(|(rel, source)| (rel.clone(), input_digests.get(rel).copied().unwrap_or_else(|| retain::digest(source.as_ref()))))
                        .collect();
                    drop(s);
                    let ckey = retain::key(&prepared_key, &cross.program);
                    let mut cells = cell.lock().expect("the session's cross-file part");
                    // Taken out while the run uses it; a run that fails leaves no kept part for this program.
                    let mut cell = cells.remove(&ckey);
                    let mut kept_frames = None;
                    if kept_part.is_some() {
                        let (loaded, p) = kept_strata::load(store, root, ckey, &cross.program, &cross_inputs, &digests)?;
                        (cell, kept_frames) = (loaded, Some(p));
                    }
                    // The phase's space (`kept_strata::space`): a relation two programs of one phase derive alike (its
                    // rules' identity) is one frame, kept once; phase 1's inputs and the pack's are in spaces apart.
                    let frames_space = kept_strata::space(root);
                    if cell.is_none() && from_frames.is_some() {
                        let _s = span("the kept part, from the frames");
                        cell = Some(retain::Retained::from_frames(store, frames_space, ckey, &cross.program, &digests)?);
                    }
                    let mut cut = retain::cut(cell.as_ref(), ckey, &cross.program, &digests)?;
                    if kept_frames.is_some() {
                        cross_kept = Some(cut.strata);
                    }
                    if let Some(kept) = cell.as_mut().filter(|_| from_frames.is_some()) {
                        if let Some((rels, rows)) = kept.load(store, &cut, &cross.program)? {
                            crate::code_rule_dsl::logic::profile::decision("the kept part, from the frames (t33 K4)", || format!("{rels} relations' rows read ({rows} rows) for the strata that run"));
                        } else {
                            cut = retain::cut(None, ckey, &cross.program, &digests)?;
                        }
                    }
                    let was_frames = cell.as_ref().filter(|_| from_frames.is_some()).and_then(|k| k.frames().cloned());
                    // An input with the rows it had is read from the last run's source, with its indexes and statistics.
                    let s = span("bind unchanged inputs");
                    let same = cell.as_ref().map(|kept| kept.same_inputs(ckey, &digests)).unwrap_or_default();
                    for (rel, source) in same {
                        if let Some(input) = cross_inputs.iter_mut().find(|(r, _)| *r == rel) {
                            modules.bind(&db, &rel, Arc::clone(&source))?;
                            input.1 = source;
                        }
                    }
                    drop(s);
                    written_key = Some(ckey);
                    if let Some(s) = session.as_ref() {
                        let written = s.written.lock().expect("the session's written outputs");
                        // Not the findings nor typing's outputs, which this run reads as well as writes.
                        for rel in cross.program.outputs.iter().filter(|r| cut.clean.contains(*r) && !matches!(r.as_str(), "finding" | "type_of")) {
                            let file = out.join(format!("{rel}.csv"));
                            if let Some((_, n)) = written.get(&file).filter(|(k, _)| *k == ckey)
                                && file.exists()
                            {
                                unchanged_outputs.insert(rel.clone(), *n);
                            }
                        }
                    }
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
                        let read: BTreeSet<&str> = cross.program.rules.iter().flat_map(|r| &r.body).filter_map(|l| l.atom()).map(|a| a.rel.as_str()).collect();
                        let undigested: Vec<&String> = cross.program.inputs.iter().filter(|r| !digests.contains_key(*r) && read.contains(r.as_str())).collect();
                        eprintln!("O125_VERIFY inputs read but not digested: {undigested:?}");
                    }
                    // Changed strata maintain their rows from deltas where they can (stage 2, `maintain`).
                    let s = span("rewrite and old sources");
                    // An input's delta, asked for by the maintained strata that read it (`maintain::rewrite`), in this
                    // order: read from the changed documents' memo entries
                    // (`input_delta`), where its rows are each one document's, so no unchanged document's row is
                    // read; found by hashing its documents (`retain::document_delta`); else made by the diff rules.
                    // `RERAIL_DIFF_INPUTS` leaves every one to the rules.
                    let by_rules_only = std::env::var_os("RERAIL_DIFF_INPUTS").is_some();
                    let changes = match (cell.as_ref(), session.as_ref()) {
                        (Some(kept), Some(held)) if !by_rules_only => {
                            let was = kept.docs(&memo_program);
                            if was.is_none() && std::env::var_os("RERAIL_STRATA_LIVE").is_some() {
                                eprintln!("RERAIL_STRATA_LIVE changed documents: the kept run holds no documents of this program");
                            }
                            was.and_then(|was| input_delta::changed(was, &run_docs_held.lock().expect("the run's documents"), &held.memo, &memo_program))
                        }
                        _ => None,
                    };
                    let (mut from_entries, mut from_entries_rows, mut hashed) = (0usize, 0usize, Vec::<String>::new());
                    let mut input_docs: HashMap<String, Arc<retain::DocDigests>> = HashMap::new();
                    let mut answer = |rel: &str| -> Option<(Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)> {
                        if by_rules_only {
                            return None;
                        }
                        let kept = cell.as_ref()?;
                        let (_, new) = cross_inputs.iter().find(|(r, _)| r == rel)?;
                        if let Some(changes) = &changes {
                            let _s = crate::code_rule_dsl::logic::profile::span(|| "session/input delta from the changed documents".into());
                            let read: Option<(String, Vec<Option<usize>>)> = if let Some(held) = base_held.get(rel) {
                                super::super::sql::select::input_columns(rel)
                                    .map(|all| (memo::base_key(rel), held.iter().map(|h| h.as_ref().and_then(|n| all.iter().position(|c| c == n))).collect()))
                            } else if split.exports.iter().any(|e| e == rel) {
                                Some((rel.to_owned(), (0..new.columns().len()).map(Some).collect()))
                            } else {
                                None
                            };
                            // Its rows add up: the old less those gone, with those new, are its rows now.
                            let adds_up = |(gone, plus): &(Vec<Vec<i64>>, Vec<Vec<i64>>)| kept.input_source(rel).is_some_and(|old| old.len() + plus.len() == new.len() + gone.len());
                            let live = std::env::var_os("RERAIL_STRATA_LIVE").is_some();
                            let read_some = read.is_some();
                            let delta = read.and_then(|(key, cols)| input_delta::delta(changes, &key, &cols));
                            if live {
                                let why = match &delta {
                                    _ if !read_some => "not a base input or an export".to_owned(),
                                    None => "a row not its document's alone".to_owned(),
                                    Some(d) if !adds_up(d) => format!(
                                        "rows do not add up: old {:?} + new {} != now {} + gone {}",
                                        kept.input_source(rel).map(|o| o.len()), d.1.len(), new.len(), d.0.len()
                                    ),
                                    Some(_) => "from the changed documents' entries".to_owned(),
                                };
                                eprintln!("RERAIL_STRATA_LIVE input delta {rel}: {why}");
                            }
                            if let Some((gone, plus)) = delta.filter(adds_up) {
                                from_entries += 1;
                                from_entries_rows += gone.len() + plus.len();
                                let width = new.columns().len();
                                // Named as the planner reads `r__plus`'s and `r__minus`'s statistics (`sql_col`).
                                let rows_source = |rows: &[Vec<i64>], suffix: &str| {
                                    let name = format!("{rel}__{suffix}");
                                    crate::code_rule_dsl::logic::native::component::columns_source((0..width).map(|c| super::sql_col(&name, c)).collect(), (0..width).map(|c| Some(rows.iter().map(|r| r[c]).collect())).collect())
                                };
                                return Some((rows_source(&plus, "plus"), rows_source(&gone, "minus")));
                            }
                        }
                        let live = std::env::var_os("RERAIL_STRATA_LIVE").is_some();
                        let Some(old) = kept.input_source(rel) else {
                            if live {
                                eprintln!("RERAIL_STRATA_LIVE input delta {rel}: by the rules (no old rows kept)");
                            }
                            return None;
                        };
                        let _s = crate::code_rule_dsl::logic::profile::span(|| "session/input delta by document".into());
                        let Some((delta, docs)) = retain::document_delta(rel, old.as_ref(), kept.input_docs(rel).map(|d| d.as_ref()), new.as_ref()) else {
                            if live {
                                eprintln!("RERAIL_STRATA_LIVE input delta {rel}: by the rules (no document delta)");
                            }
                            return None;
                        };
                        input_docs.insert(rel.to_owned(), docs);
                        hashed.push(rel.to_owned());
                        Some(delta)
                    };
                    let mut maintained = maintain::rewrite(&cut, cell.as_ref(), &cross, &mut answer)?;
                    if maintain::levels_wanted() {
                        let program = &mut maintained.program.program;
                        for rel in program.rules.iter().map(|r| r.head.rel.clone()).collect::<BTreeSet<_>>() {
                            program.qualifiers.entry(rel.to_string()).or_default().insert("levels".to_owned());
                        }
                    }
                    crate::code_rule_dsl::logic::profile::decision("input deltas", || match &changes {
                        Some(c) => format!("{} documents changed; {from_entries} inputs' deltas read from their memo entries ({from_entries_rows} rows), {} found by hashing their documents: {}", c.len(), hashed.len(), hashed.join(", ")),
                        None => format!("no kept documents: {} inputs' deltas found by hashing their documents", hashed.len()),
                    });
                    if std::env::var_os("RERAIL_SPIKE_DELTA").is_some() {
                        let read: usize = changes.iter().flatten().flat_map(|(a, b)| [a, b]).flatten().map(|e| e.rows.values().map(|(w, r)| r.len() / (*w).max(1)).sum::<usize>()).sum();
                        eprintln!("O125 input deltas: {} documents changed ({read} rows in their entries), {from_entries} from entries ({from_entries_rows} rows), by hashing {hashed:?}", changes.as_ref().map_or(0, Vec::len));
                    }
                    drop(s);
                    // What the session keeps for the next run is stored, not inlined into its readers (decision
                    // 0099): its rows are read back to be kept, and maintained from next run as `r__old`. So are the
                    // measurement aids' deltas. A relation the program declares `inline` is the exception (above): it
                    // stays inlined, and is not read back (`Outcome::inlined`). `RERAIL_SESSION_INLINE=on` inlines as a
                    // full run does, to compare (as above).
                    if !std::env::var("RERAIL_SESSION_INLINE").is_ok_and(|v| v == "on") {
                        let aids = ["RERAIL_SPIKE_DELTA", "RERAIL_CROSS_DELTA_COUNTS", "RERAIL_CROSS_VERIFY"].iter().any(|v| std::env::var_os(v).is_some());
                        let program = &mut maintained.program.program;
                        let mut stored: BTreeSet<String> = cut.derived(&cross.program)?.into_iter().chain(wanted.iter().cloned()).collect();
                        if aids {
                            stored.extend(program.decls.keys().filter(|r| ["__del", "__ins", "__minus", "__plus"].iter().any(|x| r.ends_with(x))).cloned());
                        }
                        for rel in stored.into_iter().filter(|r| !declared_inline.contains(r)) {
                            let q = program.qualifiers.entry(rel).or_default();
                            q.remove("inline");
                            q.insert("no_inline".to_owned());
                        }
                    }
                    // RERAIL_DUMP_MAINTAINED=<file>: the maintained program's rules, appended each run (a debugging aid).
                    if let Ok(path) = std::env::var("RERAIL_DUMP_MAINTAINED") {
                        use std::io::Write as _;
                        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                            let _ = writeln!(f, "=== run");
                            for r in &maintained.program.program.rules {
                                let _ = writeln!(f, "{r}");
                            }
                        }
                    }
                    let part = &maintained.program;
                    step(&format!("rewrite: {} maintained ({} recursive), {} derived again, {} rules, {} old sources", maintained.maintained, maintained.recursive, maintained.derived_again, part.program.rules.len(), maintained.sources.len()));
                    let mut stats = cross_stats;
                    let s = span("bind and measure kept");
                    let clean = cell.as_ref().map(|kept| cut.sources(kept, &part.program)).unwrap_or_default();
                    let constants = constants_tested(&part.program);
                    let mut measured_kept = cell.as_ref().map(retain::Retained::measured_all).unwrap_or_default();
                    // Each kept relation bound as `r__old`, and its statistics kept by delta if a run kept them: this
                    // run's `r`, maintained from it, has them patched by its delta (decision 0096 step 4).
                    let mut bound_old: HashMap<String, (Arc<crate::code_rule_dsl::logic::native::component::InRel>, Option<retain::Patched>)> = HashMap::new();
                    // The kept relations whose statistics are made when first planned with (decision 0101 §4), and
                    // `RERAIL_EAGER_KEPT_STATS` to make them all at binding, as before.
                    let lazy_kept = std::env::var_os("RERAIL_EAGER_KEPT_STATS").is_none();
                    let mut lazies: Vec<(String, Arc<crate::code_rule_dsl::logic::native::component::InRel>, Arc<crate::code_rule_dsl::logic::plan::LazyStats<LazyKept>>)> = Vec::new();
                    for (rel, source, held) in clean.into_iter().chain(maintained.sources.iter().cloned()) {
                        let _bind = crate::code_rule_dsl::logic::profile::span(|| format!("bind {}", if held.is_some() { "kept" } else { "source" }));
                        // A kept relation's statistics, measured when it was first bound so, while it is kept as it is.
                        let tested = constants.get(rel.as_str()).cloned().unwrap_or_default();
                        // An input's old rows are its rows of the last run: their statistics are those that run planned with,
                        // when they cover every constant the rules test it by (decision 0094 §4).
                        let carried = rel.strip_suffix("__old").and_then(|of| cell.as_ref()?.input_stats(of)).filter(|(cols, vals)| {
                            cols.len() == source.columns().len() && tested.iter().all(|(c, v)| vals.iter().any(|(vc, vv, _)| vc == c && vv == v))
                        });
                        // A derived relation kept is read in place, with its indexes (decision 0090 step 3).
                        if let Some(held) = &held {
                            crate::code_rule_dsl::logic::native::component::adopt(&db, &rel, Arc::clone(held));
                        }
                        if let Some((cols, vals)) = carried {
                            let names: Vec<String> = (0..cols.len()).map(|c| super::sql_col(&rel, c)).collect();
                            stats.measured(&rel, names.iter().cloned().zip(cols.iter().copied()), vals.iter().map(|(c, v, n)| (names[*c].clone(), *v, *n)));
                        } else {
                            let patched = held.as_ref().and_then(|h| cell.as_ref().and_then(|k| k.patched(&rel, h)).filter(|p| p.tested == tested).cloned());
                            // Kept, and known neither by delta nor as measured for these constants: its statistics are
                            // made when a stratum first plans with it (decision 0101 §4), as the eager path below would
                            // make them. Most strata a step reaches are quiet or join an empty delta and never do.
                            let measured_before = held.as_ref().and_then(|h| cell.as_ref().and_then(|k| k.measured(&rel, h)).filter(|m| m.2 == tested).cloned());
                            if patched.is_none() && measured_before.is_none() && lazy_kept
                                && let Some(h) = &held
                            {
                                let maintains = rel.strip_suffix("__old").is_some_and(|of| part.program.decls.contains_key(&format!("{of}__del")));
                                let (h2, source2, tested2, rel2) = (Arc::clone(h), Arc::clone(&source), tested.clone(), rel.clone());
                                let wanted = constants.get(rel.as_str()).cloned();
                                let lazy = crate::code_rule_dsl::logic::plan::LazyStats::new(move || {
                                    let as_cols = |m: &[(String, i64, i64, i64, i64)]| m.iter().map(|(c, rows, ndv, sq, sqd)| (c.clone(), [*rows, *ndv, *sq, *sqd])).collect::<Vec<_>>();
                                    // Counted from the engine's postings of it, and kept by this run's delta (decision 0096
                                    // step 4), if this run maintains it; else, or with no index to count from, measured.
                                    // Kept by the engine as its rows came (`InRel::kept_stats`): read, not counted.
                                    if let Some(stats) = h2.kept_stats(source2.columns()) {
                                        let counts = kept_counts(&stats, &tested2, source2.as_ref());
                                        let cols = as_cols(&stats.statistics(&|_| true));
                                        if maintains {
                                            let p = retain::Patched { at: Arc::as_ptr(&h2) as *const u8 as usize, stats, counts: counts.clone(), tested: tested2.clone() };
                                            return (cols, counts, LazyKept::Patched(p));
                                        }
                                        let measured = stats.statistics(&|_| true);
                                        return (cols, counts.clone(), LazyKept::Measured(measured, counts, tested2));
                                    }
                                    if maintains {
                                        let _c = crate::code_rule_dsl::logic::profile::span(|| format!("count {rel2}"));
                                        if let Some(stats) = super::statkeep::RelStats::from_postings(source2.columns(), h2.rows() as i64, &mut |c, visit| h2.each_posting(c, visit)) {
                                            let counts = constant_counts(Some(&tested2), source2.as_ref());
                                            let p = retain::Patched { at: Arc::as_ptr(&h2) as *const u8 as usize, stats, counts: counts.clone(), tested: tested2.clone() };
                                            return (as_cols(&p.stats.statistics(&|_| true)), counts, LazyKept::Patched(p));
                                        }
                                    }
                                    let _m = crate::code_rule_dsl::logic::profile::span(|| format!("measure {rel2}"));
                                    let measured = syntax::statistics(source2.as_ref());
                                    let counts = constant_counts(wanted.as_ref(), source2.as_ref());
                                    (as_cols(&measured), counts.clone(), LazyKept::Measured(measured, counts, tested2))
                                });
                                stats.lazily(&rel, h.rows(), lazy.clone());
                                if let Some(of) = rel.strip_suffix("__old") {
                                    bound_old.insert(of.to_owned(), (Arc::clone(h), None));
                                }
                                lazies.push((rel.clone(), Arc::clone(h), lazy));
                                modules.bind(&db, &rel, source)?;
                                continue;
                            }
                            // Not kept by delta yet: counted from the engine's postings of it, in place of measuring it,
                            // and patched by this run's delta (decision 0096 step 4): a relation's first count costs
                            // about what a measure did, once.
                            let patched = patched.or_else(|| {
                                // Only a relation this run maintains (it has an overdelete): one derived again is measured.
                                let of = rel.strip_suffix("__old")?;
                                let h = held.as_ref().filter(|_| part.program.decls.contains_key(&format!("{of}__del")))?;
                                let _c = crate::code_rule_dsl::logic::profile::span(|| format!("count {rel}"));
                                let names = source.columns();
                                // One the engine kept no index on (no later stratum read it) is measured: its count is
                                // taken from this run's relation, made with its indexes.
                                let stats = super::statkeep::RelStats::from_postings(names, h.rows() as i64, &mut |c, visit| h.each_posting(c, visit))?;
                                let counts = constant_counts(Some(&tested), source.as_ref());
                                Some(retain::Patched { at: Arc::as_ptr(h) as *const u8 as usize, stats, counts, tested: tested.clone() })
                            });
                            if let (Some(h), Some(of)) = (&held, rel.strip_suffix("__old")) {
                                bound_old.insert(of.to_owned(), (Arc::clone(h), patched.clone()));
                            }
                            let known = match patched {
                                Some(p) => Some((p.stats.statistics(&|_| true), p.counts, tested.clone())),
                                None => held.as_ref().and_then(|h| cell.as_ref().and_then(|k| k.measured(&rel, h)).filter(|m| m.2 == tested).cloned()),
                            };
                            let (measured, counts) = match known {
                                Some((measured, counts, _)) => (measured, counts),
                                None => {
                                    let _m = crate::code_rule_dsl::logic::profile::span(|| format!("measure {rel}"));
                                    let measured = syntax::statistics(source.as_ref());
                                    // The constants the rules test it by are counted too: a column measured without them
                                    // reads as holding none, so `kind__old(r, identifier)` was planned as one row and drove
                                    // a delta rule.
                                    let counts = constant_counts(constants.get(rel.as_str()), source.as_ref());
                                    if let Some(h) = &held {
                                        measured_kept.insert((rel.clone(), Arc::as_ptr(h) as *const u8 as usize), (measured.clone(), counts.clone(), tested));
                                    }
                                    (measured, counts)
                                }
                            };
                            stats.measured(&rel, measured.iter().map(|(c, rows, ndv, sq, sqd)| (c.clone(), [*rows, *ndv, *sq, *sqd])), counts);
                        }
                        modules.bind(&db, &rel, source)?;
                    }
                    drop(s);
                    // Each input's statistics as this run plans with them: next run's `__old`'s (`retain::InputStats`).
                    let input_stats: HashMap<String, retain::InputStats> = cross_inputs.iter().filter_map(|(r, src)| Some((r.clone(), stats.of_relation(r, src.columns().len())?))).collect();
                    let derived = cut.derived(&cross.program)?;
                    let mut keep = wanted.clone();
                    // An output is written to its table, where the write phase reads it, and read back from there.
                    keep.extend(derived.iter().filter(|rel| !part.program.outputs.contains(*rel)).cloned());
                    // The O125 spike (`RERAIL_SPIKE_DELTA`): each maintained relation's overdeleted and inserted rows.
                    let spike = std::env::var_os("RERAIL_SPIKE_DELTA").is_some();
                    let probes: Vec<String> = if spike { part.program.decls.keys().filter(|r| r.ends_with("__del") || r.ends_with("__ins")).cloned().collect() } else { Vec::new() };
                    keep.extend(probes.iter().cloned());
                    // A delta made in the run: what a stratum's maintenance reads besides old rows.
                    let in_run_delta = |r: &str| ["__plus", "__minus", "__del", "__ins", "__redo", "__lost", "__held"].iter().any(|x| r.ends_with(x)) || r.contains("__grp");
                    if spike {
                        keep.extend(part.program.decls.keys().filter(|r| in_run_delta(r)).cloned());
                    }
                    step("bound; running");
                    if std::env::var_os("RERAIL_CROSS_DELTA_COUNTS").is_some() || std::env::var_os("RERAIL_CROSS_VERIFY").is_some() {
                        keep.extend(part.program.decls.keys().filter(|r| ["__del", "__ins", "__minus", "__plus", "__cand", "__chk", "__gone", "__held", "__redo"].iter().any(|x| r.ends_with(x))).cloned());
                    }
                    // `RERAIL_CROSS_DEMAND_SPLIT`: keep the demand split in a session (to measure what dropping it costs).
                    let _whole = std::env::var_os("RERAIL_CROSS_DEMAND_SPLIT").is_none().then(super::demand::whole_relations);
                    let verify_stats = std::env::var_os("RERAIL_CROSS_VERIFY").map(|_| stats.clone());
                    let s = span("plan and evaluate");
                    let got = super::run_atoms_native_over(part, &db, "main", stats, keep, modes)?;
                    drop(s);
                    // Decision 0090 §4: the maintained strata the run chose to derive again, by their deltas' size.
                    let run_again = got.again;
                    step("ran");
                    collect(&db, &got, &part.program)?;
                    step("collected");
                    if spike {
                        for (rel, why) in &maintained.again {
                            eprintln!("O125 derived-again {rel}: {why}");
                        }
                        // The strata an early cutoff would skip: every delta a stratum reads (`__plus`, `__minus`, its
                        // own `__del`, `__ins`, an aggregate's groups, …) came out empty. "Given": a delta bound as an
                        // input (an earlier phase's, or found by its documents). "No delta": derived again.
                        let delta_rows: HashMap<&str, Option<usize>> = part
                            .program
                            .decls
                            .keys()
                            .filter(|r| in_run_delta(r))
                            .map(|r| (r.as_str(), got.kept.contains(r.as_str()).then(|| held_rows(&db, &got, r, part.program.decls[r].len()).map(|v| v.len()).unwrap_or(0))))
                            .collect();
                        if let Ok((sccs, _)) = crate::code_rule_dsl::logic::front::strata(&part.program) {
                            let (mut quiet, mut moving, mut none, mut given) = (0, 0, 0, 0);
                            let by_stratum = crate::code_rule_dsl::logic::front::rules_by_stratum(&part.program, &sccs);
                            for (scc, rules) in sccs.iter().zip(&by_stratum) {
                                let reads: BTreeSet<&str> = rules.iter().flat_map(|r| r.body.iter().filter_map(|l| match l {
                                    crate::code_rule_dsl::logic::front::Literal::Atom(a) | crate::code_rule_dsl::logic::front::Literal::Agg { atom: a, .. } => Some(a.rel.as_str()),
                                    _ => None,
                                })).filter(|r| delta_rows.contains_key(r)).collect();
                                if reads.is_empty() {
                                    none += 1;
                                } else if reads.iter().any(|r| delta_rows[r].is_none()) {
                                    given += 1;
                                } else if reads.iter().all(|r| delta_rows[r] == Some(0)) {
                                    quiet += 1;
                                } else {
                                    moving += 1;
                                }
                            }
                            let empty = delta_rows.values().filter(|n| **n == Some(0)).count();
                            eprintln!("O125 cutoff: {} strata: {quiet} read only empty deltas, {moving} a non-empty one, {given} a given delta, {none} no delta; {empty} of {} deltas empty", sccs.len(), delta_rows.len());
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
                                .and_then(|k| k.old_rows(rel))
                                .map(|rows| rows.into_iter().collect())
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
                    let s = span("copy derived out");
                    if wanted.contains("returns") && got.kept.contains("returns") {
                        returns = held_rows(&db, &got, "returns", 2)?;
                    }
                    // A relation of no columns is a flag its rows' count holds, which columns do not: derived again.
                    // A relation the engine holds is kept as it is, with its indexes (decision 0090 step 3); one the
                    // run left in a table (an output) is read from there. One the run recomputed in its readers
                    // (decision 0099) has neither: it is not kept, as one past the budget, and the next run derives it.
                    // The relations of the strata the native engine ran this run (each a key `a/b/…`).
                    let ran_native: BTreeSet<&str> = got.native.iter().flat_map(|k| k.split('/')).collect();
                    // A view of one input is not kept (decision 0090): the next run reads it over the input's rows.
                    let rows = derived
                        .iter()
                        .filter(|rel| !got.inlined.contains(rel.as_str()) && !maintained.views.contains(rel.as_str()))
                        .map(|rel| (rel, part.program.decls[rel].len()))
                        .filter(|(_, width)| *width > 0)
                        .map(|(rel, width)| {
                            // An output of a stratum maintained by levels (decision 0090) is still held with its rows' levels, which its
                            // table does not keep: taken from the engine too.
                            let held = got.kept.contains(rel.as_str()).then(|| crate::code_rule_dsl::logic::native::component::held(&db, rel)).flatten()
                                .or_else(|| {
                                    // Only one a native stratum of this run made (`Outcome::native`): any other the engine
                                    // holds may be a copy left by an earlier run (a stratum that ran in SQL this time).
                                    ran_native.contains(rel.as_str()).then(|| crate::code_rule_dsl::logic::native::component::held(&db, rel)).flatten().filter(|h| h.has_levels())
                                });
                            let held = match held {
                                Some(h) => h,
                                None => {
                                    let types = part.program.decls[rel].iter().map(|(_, t)| if t == "sym" { b's' } else { b'i' }).collect();
                                    crate::code_rule_dsl::logic::native::component::InRel::flat(&held_rows(&db, &got, rel, width)?, types)
                                }
                            };
                            Ok((rel.clone(), width, held))
                        })
                        .collect::<Result<Vec<_>>>()?;
                    // `RERAIL_CROSS_DELTA_COUNTS`: the rows the maintained strata overdeleted (`__del`), inserted (`__ins`),
                    // and hold after the run, for measuring delete-and-rederive (spike O125).
                    if std::env::var_os("RERAIL_CROSS_DELTA_COUNTS").is_some() {
                        // [over-deleted, inserted by the insert rules, held after, input and derived-again deltas
                        // (`__minus` + `__plus`), truly deleted, truly inserted].
                        let mut counts = [0usize; 6];
                        let olds: HashMap<&str, usize> = maintained.sources.iter().map(|(rel, s, _)| (rel.as_str(), s.len())).collect();
                        let derived_here: BTreeSet<&str> = part.program.rules.iter().map(|r| r.head.rel.as_str()).collect();
                        let rows_of = |rel: &str| held_rows(&db, &got, rel, part.program.decls[rel].len()).unwrap_or_default();
                        for (rel, cols) in &part.program.decls {
                            if !derived_here.contains(rel.as_str()) || cols.is_empty() {
                                continue;
                            }
                            if rel.ends_with("__minus") || rel.ends_with("__plus") {
                                counts[3] += rows_of(rel).len();
                            } else if rel.ends_with("__ins") {
                                counts[1] += rows_of(rel).len();
                            } else if part.program.decls.contains_key(&format!("{rel}__gone")) && part.program.decls.contains_key(&format!("{rel}__cand")) {
                                // Levels (decision 0090): candidates, those checked, gone, rederived and inserted.
                                let n = |s: &str| rows_of(&format!("{rel}__{s}")).len();
                                if std::env::var("RERAIL_CROSS_DELTA_COUNTS").is_ok_and(|v| v == "each") {
                                    eprintln!("O125 levels rows {rel}: old {}, candidates {}, checked {}, gone {}, rederived {}, inserted {}", olds.get(format!("{rel}__old").as_str()).copied().unwrap_or(0), n("cand"), n("chk"), n("gone"), n("redo"), n("ins"));
                                }
                                counts[0] += n("gone");
                            } else if !rel.ends_with("__del") && part.program.decls.contains_key(&format!("{rel}__del")) {
                                let after: std::collections::HashSet<Vec<i64>> = rows_of(rel).into_iter().collect();
                                let del = rows_of(&format!("{rel}__del"));
                                let gone = del.iter().filter(|r| !after.contains(*r)).count();
                                let old = olds.get(format!("{rel}__old").as_str()).copied().unwrap_or(0);
                                if std::env::var("RERAIL_CROSS_DELTA_COUNTS").is_ok_and(|v| v == "each") {
                                    let ins = part.program.decls.contains_key(&format!("{rel}__ins")).then(|| rows_of(&format!("{rel}__ins")).len()).unwrap_or(0);
                                    eprintln!("O125 delta rows {rel}: old {old}, overdeleted {}, gone {gone}, inserted {ins}, after {}", del.len(), after.len());
                                }
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
                    // An output whose file stands is not read back (`unchanged_outputs`).
                    let read_back: Vec<String> = cross.program.outputs.iter().filter(|o| !unchanged_outputs.contains_key(*o)).cloned().collect();
                    for rel in unchanged_outputs.keys() {
                        outputs.entry(rel.clone()).or_default();
                    }
                    let mut clean_outputs = cell.as_ref().map(|kept| cut.outputs(kept, &read_back)).unwrap_or_default();
                    // A clean output read back from the store has its rows, not its lines: decoded from the rows.
                    if let Some(kept) = cell.as_ref() {
                        let rows: Vec<(String, Arc<crate::code_rule_dsl::logic::native::component::InRel>)> = read_back
                            .iter()
                            .filter(|o| cut.clean.contains(*o) && !clean_outputs.iter().any(|(r, _, _)| r == *o))
                            .filter_map(|o| kept.kept(o).map(|(h, _)| (o.clone(), Arc::clone(h))))
                            .collect();
                        if !rows.is_empty() {
                            let (lines, nodes) = super::decode_kept(&db, &cross, rows)?;
                            clean_outputs.extend(lines.into_iter().map(|(rel, l)| {
                                let n = nodes.get(&rel).cloned().unwrap_or_default();
                                (rel, l, n)
                            }));
                        }
                    }
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
                        // `O125_VERIFY_RULES=<rel>`: the rules for `rel` the verifier derives with, and the maintained ones.
                        if let Ok(r) = std::env::var("O125_VERIFY_RULES") {
                            for x in plain.program.rules.iter().filter(|x| x.head.rel == r) {
                                eprintln!("O125_VERIFY_RULES plain {x}");
                            }
                            for x in part.program.rules.iter().filter(|x| x.head.rel == r) {
                                eprintln!("O125_VERIFY_RULES maintained {x}");
                            }
                        }
                        let mut vkeep = wanted.clone();
                        vkeep.extend(derived.iter().filter(|rel| !plain.program.outputs.contains(*rel)).cloned());
                        let again = super::run_atoms_native_over(&plain, &db, "main", vstats, vkeep, modes)?;
                        let mut shown = 0;
                        let rows_of = |h: &Arc<crate::code_rule_dsl::logic::native::component::InRel>, width: usize| -> Vec<Vec<i64>> {
                            let s = h.source((0..width).map(|c| format!("c{c}")).collect());
                            (0..s.len()).map(|i| (0..width).map(|c| s.value(i, c)).collect()).collect()
                        };
                        let rows: Vec<(String, usize, Vec<Vec<i64>>)> = rows.iter().map(|(r, w, h)| (r.clone(), *w, rows_of(h, *w))).collect();
                        for (rel, width, kept_rows) in &rows {
                            let want: std::collections::HashSet<Vec<i64>> = held_rows(&db, &again, rel, *width)?.into_iter().collect();
                            let have: std::collections::HashSet<Vec<i64>> = kept_rows.iter().cloned().collect();
                            if want != have && shown < 6 {
                                shown += 1;
                                let delta = |name: String| -> std::collections::HashSet<Vec<i64>> { verify_deltas.get(&name).cloned().unwrap_or_default() };
                                let old_rows: std::collections::HashSet<Vec<i64>> = maintained
                                    .sources
                                    .iter()
                                    .find(|(r, _, _)| *r == format!("{rel}__old"))
                                    .map(|(_, src, _)| (0..src.len()).map(|i| (0..*width).map(|c| src.value(i, c)).collect()).collect())
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
                                let olds = maintained.sources.iter().find(|(r, _, _)| *r == format!("{rel}__old")).map(|(_, src, _)| (0..src.len()).map(|i| (0..*width).map(|c| src.value(i, c)).collect::<Vec<i64>>()).filter(|r| hit(r)).collect::<Vec<_>>()).unwrap_or_default();
                                let (a, b): (Vec<_>, Vec<_>) = (kept_rows.iter().filter(|r| hit(r)).collect(), fresh.iter().filter(|r| hit(r)).collect());
                                let del: Vec<_> = verify_deltas.get(&format!("{rel}__del")).map(|d| d.iter().filter(|r| hit(r)).cloned().collect()).unwrap_or_default();
                                if !a.is_empty() || !b.is_empty() || !olds.is_empty() {
                                    eprintln!("O125_TRACE {rel}: old {:?} del {:?} maintained {:?} fresh {:?}", &olds[..olds.len().min(4)], &del[..del.len().min(4)], &a[..a.len().min(4)], &b[..b.len().min(4)]);
                                }
                            }
                            for (rel, src) in &cross_inputs {
                                let w = src.columns().len();
                                let now: Vec<Vec<i64>> = (0..src.len()).map(|i| (0..w).map(|c| src.value(i, c)).collect::<Vec<i64>>()).filter(|r| hit(r)).take(4).collect();
                                let olds = maintained.sources.iter().find(|(r, _, _)| *r == format!("{rel}__old")).map(|(_, src, _)| (0..src.len()).map(|i| (0..w).map(|c| src.value(i, c)).collect::<Vec<i64>>()).filter(|r| hit(r)).take(4).collect::<Vec<_>>());
                                if !now.is_empty() || olds.as_ref().is_some_and(|o| !o.is_empty()) {
                                    eprintln!("O125_TRACE input {rel}: now {now:?} old {olds:?}");
                                }
                            }
                        }
                        eprintln!("O125_VERIFY done: {shown} relations differ (first 6)");
                    }
                    drop(s);
                    // A maintained relation's statistics as next run binds it (`r__old`): those of the relation it
                    // started as, patched by its delta; counted from its rows once, the first run it is maintained.
                    let s = span("patch the maintained relations' statistics");
                    // The lazy statistics a plan made are kept as the eager ones are: by delta, or as measured. One no
                    // plan read is left to next run, which binds this run's relation lazily in turn.
                    let mut unmade: BTreeSet<String> = BTreeSet::new();
                    let lazy_made = lazies.len();
                    let mut made = 0;
                    for (rel, h, lazy) in lazies.drain(..) {
                        let of = rel.strip_suffix("__old").map(str::to_owned);
                        match lazy.made().map(|m| &m.2) {
                            Some(LazyKept::Patched(p)) => {
                                made += 1;
                                if let Some(entry) = of.as_ref().and_then(|of| bound_old.get_mut(of)) {
                                    entry.1 = Some(p.clone());
                                }
                            }
                            Some(LazyKept::Measured(m, counts, tested)) => {
                                made += 1;
                                measured_kept.insert((rel.clone(), Arc::as_ptr(&h) as *const u8 as usize), (m.clone(), counts.clone(), tested.clone()));
                            }
                            None => unmade.extend(of),
                        }
                    }
                    step(&format!("kept statistics: {made} of {lazy_made} lazy relations planned with"));
                    let mut patched: HashMap<String, retain::Patched> = HashMap::new();
                    for (rel, width, new) in &rows {
                        let Some((old, kept)) = bound_old.get(rel) else { continue };
                        if unmade.contains(rel) {
                            continue;
                        }
                        let name = format!("{rel}__old");
                        let next = match kept {
                            Some(p) => p.after(old, new, *width),
                            // Held on unchanged (a quiet stratum's, decision 0101 §1): nothing to count yet. Next run
                            // finds what was measured of it by the same relation, or binds it lazily, and it is
                            // counted once it changes (O125 step 1: 238 counts instead of 521).
                            None if std::ptr::eq(Arc::as_ptr(old), Arc::as_ptr(new)) => None,
                            // Kept by the engine as its rows came (`InRel::kept_stats`): read, not counted.
                            None if let Some(stats) = new.kept_stats((0..*width).map(|c| super::sql_col(&name, c)).collect()) => {
                                let tested = constants.get(name.as_str()).cloned().unwrap_or_default();
                                let counts = kept_counts(&stats, &tested, new.source(stats.columns().to_vec()).as_ref());
                                Some(retain::Patched { at: Arc::as_ptr(new) as *const u8 as usize, stats, counts, tested })
                            }
                            None if new.delta_since(old).is_some() => new.live_rows().map(|live| {
                                let _c = crate::code_rule_dsl::logic::profile::span(|| format!("count {rel}"));
                                let names: Vec<String> = (0..*width).map(|c| super::sql_col(&name, c)).collect();
                                // From its postings, else its rows.
                                let stats = super::statkeep::RelStats::from_postings(names.clone(), new.rows() as i64, &mut |c, visit| new.each_posting(c, visit)).unwrap_or_else(|| {
                                    let mut stats = super::statkeep::RelStats::paired(names.clone());
                                    stats.documents(&live, *width, 1);
                                    stats
                                });
                                let tested = constants.get(name.as_str()).cloned().unwrap_or_default();
                                let counts = constant_counts(Some(&tested), new.source(names).as_ref());
                                retain::Patched { at: Arc::as_ptr(new) as *const u8 as usize, stats, counts, tested }
                            }),
                            None => None,
                        };
                        if let Some(p) = next {
                            patched.insert(name, p);
                        }
                    }
                    drop(s);
                    let s = span("retain");
                    if recursive_spike::on() {
                        recursive_spike::run(&cross.program, cell.as_ref(), &rows, &cross_inputs, &|a, b| crate::code_rule_dsl::logic::native::component::cat_symbols(&db, a, b));
                    }
                    let next = retain::after(cell.take(), ckey, digests.clone(), &cut, rows, &derived_outputs, &cross_inputs, measured_kept, input_stats, input_docs, (run_docs_held.lock().expect("the run's documents").clone(), memo_program), patched);
                    if let Some(was) = &was_frames {
                        let written = next.write_frames(store, was, &cross.program, &cut, &cross_inputs, &digests)?;
                        crate::code_rule_dsl::logic::profile::decision("the kept part, to the frames (t33 K4)", || format!("{written} rows of what changed queued to the store"));
                    }
                    drop(s);
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
                    if std::env::var_os("RERAIL_KEPT_BYTES").is_some() {
                        for (kind, rows, values, levels, index) in next.held() {
                            eprintln!("KEPT {kind}: {rows} rows, values {:.2} MB, levels {:.2} MB, sets and indexes {:.2} MB", values as f64 / 1e6, levels as f64 / 1e6, index as f64 / 1e6);
                        }
                        if crate::code_rule_dsl::logic::native::component::probed::on() {
                            eprintln!("{}", next.index_report(&crate::code_rule_dsl::logic::native::component::probed::take()));
                        }
                    }
                    // A session's run reports its strata here; a new process's, as `cross_kept` (above, or here when kept
                    // in the frames).
                    if from_frames.is_some() {
                        cross_kept = Some(cut.strata);
                    } else if kept_part.is_none() {
                        cross_strata = Some(cut.strata);
                    }
                    cross_maintained = maintained.maintained;
                    cross_recursive = maintained.recursive;
                    cross_derived_again = maintained.derived_again;
                    cross_changed = cut.changed.clone();
                    cross_again = maintained.again.iter().fold(BTreeMap::new(), |mut m, (_, why)| {
                        *m.entry(*why).or_insert(0) += 1;
                        m
                    });
                    if run_again > 0 {
                        cross_again.insert("input deltas past θ", run_again);
                    }
                    // Every program's kept part shares the one budget: past it, only this program's is kept.
                    if next.bytes() + cells.values().map(retain::Retained::bytes).sum::<usize>() > retain::budget() {
                        cells.clear();
                    }
                    if let Some(p) = kept_frames.take() {
                        kept_strata::keep(store, p, &next);
                    }
                    cells.insert(ckey, next);
                }
                None => {
                    let exported: BTreeSet<&String> = split.exports.iter().collect();
                    // Without a session, the strata are kept in the tape cache from run to run (decision 0077 §4).
                    let mut kept_strata = (durable.is_some() && one_fixpoint).then(|| kept_strata::Kept::new(store, kept_strata::space(root), &cross_inputs));
                    let loaded = kept_strata.as_ref().map(|k| std::rc::Rc::clone(&k.loaded));
                    let mut load = |rels: &[String]| -> Result<()> {
                        for rel in rels.iter().filter(|r| exported.contains(r)) {
                            let src = exports.borrow_mut().source(rel, lowered.program.decls[rel].len());
                            if let Some(loaded) = &loaded {
                                loaded.borrow_mut().insert(rel.clone(), Arc::clone(&src));
                            }
                            modules.bind(&db, rel, src)?;
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
                    let strata = kept_strata.as_mut().map(|k| k as &mut dyn super::KeptStrata);
                    let hooks = if one_fixpoint { super::Hooks { load: Some(&mut load), unload: Some(&mut unload), strata, ..Default::default() } } else { super::Hooks::default() };
                    let got = super::run_atoms_native_hooked(&cross, &db, "main", cross_stats, wanted.clone(), modes, hooks)?;
                    if let Some(k) = kept_strata {
                        let written = k.written_rows;
                        cross_kept = k.strata;
                        k.finish();
                        crate::code_rule_dsl::logic::profile::decision("cross-file strata kept across runs (decision 0077 §4)", || format!("{written} rows of the relations whose rows changed written to the store"));
                    }
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
                    let pre_derived: BTreeSet<String> = ps.pre.rules.iter().map(|r| r.head.rel.to_string()).collect();
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
    drop(waves.take());
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
        let _s = crate::code_rule_dsl::logic::profile::span(|| "record the run in the store".into());
        collecting = record_run(store, root, &run_sources, &memo_program, &program_plans, &relations, &late_sources, reading, !phases::in_phase() && !phase0::active() && options.get(constants::OWN).is_none())?;
    }
    statements.enter("write");
    let writing = crate::code_rule_dsl::logic::profile::span(|| "write the outputs".into());
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
    // A node column's value that is no node: `unset` (-1) is written as it is, and a `@place` (`rr::lower`'s
    // `PLACE`, `-2 - (path << 32 | line)`) as `@place("path", line)`.
    let mut places: HashMap<i64, String> = HashMap::new();
    for (rel, lines) in &outputs {
        for (i, v) in lines.iter().flat_map(|row| row.split('\t').enumerate()) {
            if let Ok(v) = v.parse::<i64>()
                && v < -1
                && node_columns[rel].contains(&i)
                && !places.contains_key(&v)
            {
                let n = -2 - v;
                let path: String = db.query_row("SELECT sym_text(?1)", [n >> 32], |r| r.get(0))?;
                places.insert(v, format!("@place({}, {})", crate::code_rule_dsl::logic::rr::quote(&path), n & 0xffff_ffff));
            }
        }
    }
    for (rel, lines) in &outputs {
        if let Some(n) = unchanged_outputs.get(rel) {
            rows.insert(rel.clone(), *n);
            continue;
        }
        let node_columns = &node_columns[rel];
        let mut decoded: Vec<String> = lines
            .iter()
            // A row about a loaded document (its first node is one) is not the corpus's; a corpus row may name a
            // loaded document's node (a call's target in a stub).
            .filter(|row| loaded_keys.is_empty() || node_columns.first().is_none_or(|c| !row.split('\t').nth(*c).and_then(|v| v.parse::<i64>().ok()).is_some_and(|v| loaded_keys.contains(&(v >> 32)))))
            .map(|row| {
                row.split('\t')
                    .enumerate()
                    // A node column holding no document's node (`unset`'s -1) is written as it is.
                    .map(|(i, v)| match v.parse::<i64>() {
                        Ok(v) if node_columns.contains(&i) && v < -1 => places[&v].clone(),
                        // A guest's node (decision 0055): its document is no file of the run.
                        Ok(v) if node_columns.contains(&i) && v >= 0 && !paths.contains_key(&(v >> 32)) && !first.contains_key(&(v >> 32)) => format!("guest{}:{}", v >> 32, v & 0xffff_ffff),
                        Ok(v) if node_columns.contains(&i) && (own || syntax::definition_ids()) && paths.contains_key(&(v >> 32)) => format!("{}:{}", paths[&(v >> 32)], syntax::shown_local(v & 0xffff_ffff)),
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
        if !decoded.is_empty() {
            text.push('\n');
        }
        std::fs::write(out.join(format!("{rel}.csv")), text).map_err(|e| Error::Plan(e.to_string()))?;
        rows.insert(rel.clone(), decoded.len());
    }
    // What a session's run of this program wrote, for its next (`unchanged_outputs`); any other run's writes forget it.
    if let Some(s) = session.as_ref() {
        let mut written = s.written.lock().expect("the session's written outputs");
        for (rel, n) in &rows {
            let file = out.join(format!("{rel}.csv"));
            match written_key {
                Some(key) => written.insert(file, (key, *n)),
                None => written.remove(&file),
            };
        }
    }
    if prog.decls.contains_key("declared_ret") {
        let fn_q = split.exports.iter().any(|e| e == "fn_q").then(|| exports.borrow_mut().source("fn_q", 2));
        typing::outputs(typing, &db, &store, root, outputs.get("type_of"), &returns, &declared, fn_q.as_deref(), &mut typed)?;
    }
    drop(writing);
    phase("write", statements.count());
    let rss = usage().2;
    phases.insert(1, ("parse (worker)", parse_cpu.0, parse_cpu.1, rss, 0));
    phases.insert(2, ("batch (worker)", batch_cpu.0, batch_cpu.1, rss, 0));
    let (tested, text_cpu) = *vocabulary.tested.lock().unwrap();
    drop(statements);
    // The run's atoms are written at its end (decision 0049 §4.3), behind the run as any write (decision 0102): every
    // queued write that names an atom appends the pending atoms before it commits, so this writes those no later
    // write did. The profile says how often they were.
    store.write_back("the run's atoms", 0, Vec::new(), Box::new(|store| store.flush_atoms()));
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
        let collected = crate::code_rule_dsl::logic::profile::wait("the store's collection", &collection, || collecting.join()).map_err(|_| Error::Plan("store collection panicked".to_owned()))?;
        if let Some((passes, outcome)) = collected {
            crate::code_rule_dsl::logic::profile::decision("store collection (decision 0053)", || match outcome {
                Ok(summary) => format!("{passes} passes in the background: {summary}"),
                Err(e) => format!("{passes} passes, then stopped: {e}"),
            });
        }
    }
    // The run ends with its writes in the store: what the writer did beside the run, and the first write that failed
    // (decision 0102).
    if durable.is_some() {
        let waited = crate::code_rule_dsl::logic::profile::span(|| "the store's writer, the run's last writes".into());
        // The run's atoms still pending, appended by the writer with its last writes.
        store.queue_atoms_flush();
        let outcome = store.written();
        drop(waited);
        let w = store.written_so_far();
        let (by_what, peak, held) = store.written_by_what();
        crate::code_rule_dsl::logic::profile::decision("write-back by what (decision 0102)", || {
            let rows: Vec<String> = by_what.iter().map(|(what, jobs, bytes, cpu)| format!("{what}: {jobs} jobs, {:.1} MB, {cpu:.3} s", *bytes as f64 / 1048576.0)).collect();
            format!("queue peak {:.1} MB; back-pressure held producers {} times, {:.3} s; {}", peak as f64 / 1048576.0, held.0, held.1, rows.join("; "))
        });
        crate::code_rule_dsl::logic::profile::decision("write-back (decision 0102)", || format!("{} writes queued behind the run in {} transactions, {:.1} MB, {:.3} s of the writer's CPU", w.jobs, w.batches, w.bytes as f64 / 1048576.0, w.cpu));
        // By kind, over the process so far.
        let kinds = crate::content_store::write_back::written_by_kind();
        let text = || kinds.iter().map(|(k, n, b, c)| format!("{k}: {n} jobs, {:.1} MB, {c:.3} s", *b as f64 / 1048576.0)).collect::<Vec<_>>().join("; ");
        crate::code_rule_dsl::logic::profile::decision("write-back by kind, this process so far (decision 0102)", text);
        if std::env::var_os("RERAIL_WRITES").is_some() {
            eprintln!("RERAIL_WRITES {}", text());
        }
        outcome.map_err(|e| Error::Plan(e.to_string()))?;
    }
    // And the observer's (decision 0105), queued behind the run as the store's are.
    if !phase0::active() {
        crate::content_store::observe::written();
    }
    store.atoms_handle().run_over();
    crate::code_rule_dsl::logic::profile::decision("chunks", || format!("{chunks} chunks of about {chunk_nodes} nodes; the next chunk is parsed and batched on a worker while this one runs"));
    if profiling && let Some(dir) = crate::code_rule_dsl::logic::profile::directory().map(|d| if phase0::active() { d.join("phase-0") } else { d }) {
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
        crate::code_rule_dsl::logic::profile::report::write(&dir, &header).map_err(|e| Error::Plan(format!("profile: {e}")))?;
    }
    // The index, when `@git_tracked` read it: it moves between runs (`strings::git`).
    if t.is_none() {
        let _s = crate::code_rule_dsl::logic::profile::span(|| "the Git index consulted".into());
        consulted.extend(crate::code_rule_dsl::logic::strings::git::consulted(root));
    }
    let report = Report { phases, phase_instructions, files: docs.len(), nodes, native: native.len(), rows, chunks, rounds, typing: typed, text: (vocabulary.tests.len(), tested, text_cpu), typing_runs, phase_runs, memo: (memo_hits, memo_unattributed), cross: cross_counts, prepared: origin, cross_strata, cross_kept, cone_kept, cone_rounds, cross_maintained: (cross_maintained, cross_recursive, cross_derived_again), cross_changed, cross_again, cross_delta_counts, consulted, cone_inputs, not_per_document, files_observed: Default::default() };
    handed.succeeded();
    Ok(report)
}



/// The cross-file part's bounded rounds (`typing::cross_rounds`): `round` is the part with its hops reading
/// `<rel>__prev`, the previous round's rows held in memory (none in round 0). Each round's rows of the
/// hop relations are digested; an unchanged digest ends the rounds (the fixpoint), as does round `K`. The last
/// round's outcome is the part's.
/// A relation's rows after a run: held by the engine (`kept`), or in its table.
/// How many of `source`'s rows hold each integer constant a rule of `prog` tests `rel`'s columns by, by column name
/// (the planner's `const_freq`).
/// Per relation, the integer constants its atoms test, by column (`constant_counts`'s): one pass over the rules,
/// not one per relation.
fn constants_tested(prog: &front::Program) -> HashMap<&str, BTreeSet<(usize, i64)>> {
    let mut by: HashMap<&str, BTreeSet<(usize, i64)>> = HashMap::new();
    for a in prog.rules.iter().flat_map(|r| &r.body).filter_map(front::Literal::atom) {
        for (c, t) in a.args.iter().enumerate() {
            if let front::Term::Int(v) = t {
                by.entry(a.rel.as_str()).or_default().insert((c, *v));
            }
        }
    }
    // A consumer may test a projection, while statistics are collected on its source. Propagate those
    // demands through single-atom projections before counting. A worklist also handles alias chains/cycles.
    let mut projections: HashMap<(&str, usize), Vec<(&str, usize)>> = HashMap::new();
    for rule in &prog.rules {
        let [front::Literal::Atom(a)] = rule.body.as_slice() else { continue };
        if a.neg { continue; }
        for (hc, term) in rule.head.args.iter().enumerate() {
            let front::Term::Var(v) = term else { continue };
            for (bc, arg) in a.args.iter().enumerate() {
                if matches!(arg, front::Term::Var(x) if x == v) {
                    projections.entry((&rule.head.rel, hc)).or_default().push((&a.rel, bc));
                }
            }
        }
    }
    let mut queue: Vec<(&str, usize, i64)> = by.iter().flat_map(|(r, cols)| cols.iter().map(move |(c, v)| (*r, *c, *v))).collect();
    while let Some((rel, col, value)) = queue.pop() {
        for &(source, at) in projections.get(&(rel, col)).into_iter().flatten() {
            if by.entry(source).or_default().insert((at, value)) { queue.push((source, at, value)); }
        }
    }
    by
}

/// The rows of each constant `tested`, from the kept statistics' exact counts where its column has them (a column
/// of few values, as the ones rules test by constants are), else counted from `source` (`constant_counts`).
fn kept_counts(stats: &super::statkeep::RelStats, tested: &BTreeSet<(usize, i64)>, source: &dyn crate::code_rule_dsl::facts::vtab::Source) -> Vec<(String, i64, i64)> {
    let names = stats.columns();
    let mut exact: HashMap<usize, HashMap<i64, i64>> = HashMap::new();
    for &(c, _) in tested {
        if c < names.len() && !exact.contains_key(&c) {
            let Some(e) = stats.exact(c) else { return constant_counts(Some(tested), source) };
            exact.insert(c, e.collect());
        }
    }
    tested.iter().filter(|(c, _)| *c < names.len()).map(|&(c, v)| (names[c].clone(), v, exact[&c].get(&v).copied().unwrap_or(0))).collect()
}

fn constant_counts(tested: Option<&BTreeSet<(usize, i64)>>, source: &dyn crate::code_rule_dsl::facts::vtab::Source) -> Vec<(String, i64, i64)> {
    let mut wanted: BTreeMap<(usize, i64), i64> = tested.into_iter().flatten().map(|k| (*k, 0)).collect();
    if wanted.is_empty() {
        return Vec::new();
    }
    let names = source.columns();
    // A source whose rows do not change keeps its counts (a relation kept from an earlier run, `InRel`).
    let asked: Vec<(usize, i64)> = wanted.keys().copied().filter(|(c, _)| *c < names.len()).collect();
    if let Some(counts) = source.value_counts(&asked) {
        return asked.iter().zip(counts).map(|((c, v), n)| (names[*c].clone(), *v, n)).collect();
    }
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
    // What the engine holds is read from it; only a SQL stratum's relation is in a table.
    let source: Arc<dyn crate::code_rule_dsl::facts::vtab::Source> = if let Some(held) = component::derived(db, rel) {
        held
    } else if got.kept.contains(rel) {
        return Err(Error::Plan(format!("{rel} is not held")));
    } else {
        Arc::new(Rows::of(db, rel, &crate::code_rule_dsl::logic::sql::ident(rel))?)
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
            let source: Arc<dyn crate::code_rule_dsl::facts::vtab::Source> = if let Some(held) = component::derived(db, rel) {
                held
            } else if got.kept.contains(rel) {
                return Err(Error::Plan(format!("{rel} is not held")));
            } else {
                Arc::new(Rows::of(db, rel, &crate::code_rule_dsl::logic::sql::ident(rel))?)
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
/// as tab-separated rows (phase, runs, ms, vm steps, full-scan steps, sorts, autoindex rows, text); another connection's
/// (`on_as`) to `<file><suffix>`.
pub(super) struct Statements {
    db: *mut rusqlite::ffi::sqlite3,
    suffix: &'static str,
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
    pub(super) fn on(db: &Connection) -> Self {
        Self::on_as(db, "")
    }
    pub(super) fn on_as(db: &Connection, suffix: &'static str) -> Self {
        use rusqlite::ffi;
        unsafe extern "C" fn trace(event: u32, state: *mut std::ffi::c_void, p: *mut std::ffi::c_void, x: *mut std::ffi::c_void) -> std::ffi::c_int {
            // SAFETY: the state outlives the callback (removed on drop).
            let state = unsafe { &*state.cast::<StatementState>() };
            if event == ffi::SQLITE_TRACE_STMT as u32 {
                state.count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if crate::code_rule_dsl::logic::profile::strict() {
                    // SAFETY: for SQLITE_TRACE_STMT, P is the statement.
                    let text = unsafe { std::ffi::CStr::from_ptr(ffi::sqlite3_sql(p.cast::<ffi::sqlite3_stmt>())) }.to_string_lossy();
                    crate::code_rule_dsl::logic::profile::strict_statement("run", &text);
                }
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
                crate::code_rule_dsl::logic::profile::sql_in_wave("run", &text, ns);
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
        Statements { db, suffix, state, span: Default::default() }
    }
    pub(super) fn count(&self) -> usize {
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
            let mut file = file;
            file.push(self.suffix);
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
    /// Constants' counts of columns whose statistics are given whole (`add`): (relation, column, value, count).
    consts: Vec<(String, String, i64, i64)>,
}
impl Totals {
    /// Drops `rel`'s measurements (to measure it again).
    fn forget(&mut self, rel: &str) {
        self.rels.remove(rel);
        self.vals.retain(|(r, _), _| r != rel);
        self.consts.retain(|(r, ..)| r != rel);
    }
    /// `from`'s measurements also under the name `to` (the same rows, measured once).
    fn alias(&mut self, from: &str, to: &str) {
        if let Some(r) = self.rels.get(from).cloned() {
            self.rels.insert(to.to_owned(), r);
        }
        let vals: Vec<_> = self.vals.iter().filter(|((r, _), _)| r == from).map(|((_, c), v)| ((to.to_owned(), c.clone()), v.clone())).collect();
        self.vals.extend(vals);
        let consts: Vec<_> = self.consts.iter().filter(|(r, ..)| r == from).map(|(_, c, v, n)| (to.to_owned(), c.clone(), *v, *n)).collect();
        self.consts.extend(consts);
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
            if super::sketch::shadowing() {
                let mut sketch = super::sketch::Sketch::default();
                for (v, e) in per {
                    sketch.add(*v, e[0]);
                }
                super::sketch::shadow(&format!("totals {rel}"), col, rows, ndv, sq, &sketch);
            }
            stats.measured(rel, [(col.clone(), [rows, ndv.max(1), sq, sqd])], per.iter().map(|(v, [n, _, _])| (col.clone(), *v, *n)));
        }
        for (rel, col, v, n) in &self.consts {
            stats.measured(rel, [], [(col.clone(), *v, *n)]);
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
    /// The relation `rel`'s rows: a view the run read in place from its input (`vtab::view_columns`), else its table
    /// `table`, read by SQL.
    fn of(db: &Connection, rel: &str, table: &str) -> Result<Self> {
        if let Some(values) = crate::code_rule_dsl::facts::vtab::view_columns(db, rel) {
            return Ok(Rows { columns: (0..values.len()).map(|i| format!("c{i}")).collect(), values });
        }
        Self::read(db, table)
    }
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


/// Whether the corpus runner reads the file at `path` as text only: no dialect has its suffix, so it is in the run
/// only because a `.scope text` glob reads it (`scoped`), and it has `doc` and the source text's relations and no tree.
fn text_document(path: &str) -> bool {
    crate::code_rule_dsl::language::label_of_path(path).is_none()
}

/// A corpus's files by language: Python's, Rust's and `.rr` rule files, and the text-only documents.
struct CorpusFiles {
    /// Each dialect's files, by its label (the language registry's).
    by_label: BTreeMap<&'static str, Vec<String>>,
    /// Files of no grammar: after `scoped`, those a `.scope text` glob reads as text (`front::Program::text_globs`).
    text: Vec<String>,
}

impl CorpusFiles {
    /// The run's documents: the language's files, then the text ones, in path order.
    fn of(mut self, label: &str) -> Vec<String> {
        let mut files = self.by_label.remove(label).unwrap_or_default();
        files.extend(self.text);
        files.sort();
        files
    }

    /// How many files of `label` there are.
    fn count(&self, label: &str) -> usize {
        self.by_label.get(label).map_or(0, Vec::len)
    }

    /// How many files of any grammar there are.
    fn parsed(&self) -> usize {
        self.by_label.values().map(Vec::len).sum()
    }
}

/// The language a corpus run reads: the one its program names (`.language`: any dialect of the registry this build
/// parses), else Rust or Python by `Typing::rust`, or by whichever has more files.
fn corpus_language(language: Option<&str>, typing: &Typing, found: &CorpusFiles) -> &'static str {
    match language {
        Some(name) if let Some(label) = crate::code_rule_dsl::language::label_named(name) => label,
        _ if typing.rust.unwrap_or(found.count("rust") > found.count("python")) => "rust",
        _ => "python",
    }
}

/// The documents JSON policies select, each its `scope`'s `include` (every file where it has none) less its `exclude`;
/// `None` where a policy has no `scope` (it reads every file) or there is no policy.
fn json_scopes(policies: &[serde_json::Value]) -> Option<Vec<(Vec<String>, Vec<String>)>> {
    let globs = |v: Option<&serde_json::Value>| -> Vec<String> { v.and_then(|v| v.as_array()).into_iter().flatten().filter_map(|g| g.as_str().map(str::to_owned)).collect() };
    if policies.is_empty() {
        return None;
    }
    policies.iter().map(|p| p.get("scope").map(|s| (globs(s.get("include")), globs(s.get("exclude"))))).collect()
}

/// The dialects a run of `program` reads, when its `.language` names a language of several (TypeScript's: `.ts`,
/// `.tsx` and `.js` files): those with files under `root` in the program's scope, the named one first. A program is
/// lowered for one dialect (its kind names are that grammar's ids), so each is a run of its own. Empty when the
/// language has one dialect: the run reads it as before.
pub fn run_dialects(root: &Path, program: &str, options: &serde_json::Value) -> Result<Vec<&'static str>> {
    use crate::code_rule_dsl::language::{CodeRuleLanguage, label};
    let prog = combine(&[program], "")?;
    let Some(name) = prog.settings.iter().find(|(key, _)| key == "language").map(|(_, value)| value.trim().trim_matches('"').to_owned()) else {
        return Ok(Vec::new());
    };
    let Some(language) = CodeRuleLanguage::ALL.into_iter().find(|l| l.name() == name) else { return Ok(Vec::new()) };
    if language.dialects().len() < 2 {
        return Ok(Vec::new());
    }
    let found = scoped(corpus_files(root)?, &prog, options);
    let mut labels: Vec<&'static str> = language.dialects().iter().filter_map(|d| label(*d)).filter(|l| found.count(l) > 0).collect();
    labels.sort_by_key(|l| *l != name);
    Ok(labels)
}

/// The files the program's policies select (`front::Program::doc_scopes`, their `.scope`): a document outside
/// every policy's scope is not parsed, and gives no facts.
/// A scope entry taken from a policy option (`.scope dirs option("…")`) reads the option's values, from the run's
/// `options`. A file of no grammar is read, as text only, when a `.scope text` glob matches it and a scope selects it.
fn scoped(mut found: CorpusFiles, prog: &front::Program, options: &serde_json::Value) -> CorpusFiles {
    let text_glob = |p: &String| prog.text_globs.iter().any(|g| crate::code_rule_dsl::logic::strings::test("glob", p, g));
    found.text.retain(text_glob);
    // A program of JSON policies (`.policy {…}`) and no `.scope`: their `scope`s (decision 0107).
    let doc_scopes = prog.doc_scopes.clone().or_else(|| json_scopes(&prog.policies));
    if doc_scopes.is_none() {
        return found;
    }
    let scopes = front::resolve_scopes(&doc_scopes, options);
    let before = found.parsed();
    for files in found.by_label.values_mut().chain([&mut found.text]) {
        files.retain(|p| front::in_doc_scopes(&scopes, p));
    }
    crate::code_rule_dsl::logic::profile::decision("documents", || {
        format!("{} of {before} files, by the policies' `.scope`; {} read as text (`.scope text`)", found.parsed(), found.text.len())
    });
    found
}

/// The files Git sees under `root`, relative, and how many of them Git's status changed (staged, untracked,
/// deleted): `None` outside a work tree, or where Git sees none under it.
///
/// Decision 0105: the list is kept as the observer's record (`git-files:<root>`), with what Git read for it: the
/// root, the `.git` file and the ignore files above the root, the work tree's `HEAD` and index, and the refs,
/// config and info shared with other work trees. Where the journal names a change in none of those, the list is
/// as the status found it, and the status is not read again (a status stats every file in the index).
fn git_files(root: &Path) -> Option<(Vec<String>, usize)> {
    use std::path::PathBuf;
    let real = observe(|o| o.canonical(root))?;
    let key = format!("git-files:{}", real.display());
    // What the record read: the journal's cursors before the status and the roots they are of (a line each, by tab),
    // then a path a line.
    let kept = observe(|o| {
        let (read, files) = o.run_record(&key)?;
        let mut lines = read.lines();
        let cursors: Vec<String> = lines.next()?.split('\t').map(str::to_owned).collect();
        let watched: Vec<PathBuf> = lines.next()?.split('\t').map(PathBuf::from).collect();
        let read: Vec<PathBuf> = lines.map(PathBuf::from).collect();
        let named = o.changed_since(&watched, &cursors)?;
        // The repository's own directories, of which only what `read` names counts (not its objects).
        let git_dirs: Vec<PathBuf> = watched.iter().skip(2).cloned().collect();
        Some((read, git_dirs, named, String::from_utf8(files).ok()?))
    });
    if let Some((read, git_dirs, named, text)) = kept {
        let mut lines = text.lines();
        let overlays: usize = lines.next().and_then(|l| l.parse().ok()).unwrap_or(0);
        let under: Vec<String> = lines.map(str::to_owned).collect();
        // A change named in what Git reads besides the files (its index, refs, ignore files), or at a directory above
        // one, may change the list. A file named under the root leaves it as it was when it is still a file the list
        // holds, or still absent and not in the list; a directory named, or one the list has files under, may not.
        let listed: std::collections::BTreeSet<&str> = under.iter().map(String::as_str).collect();
        let holds = named.iter().all(|n| {
            if read.iter().any(|p| (n.starts_with(p) && *p != real) || (p.starts_with(n) && *p != *n)) {
                return false;
            }
            if git_dirs.iter().any(|d| n.starts_with(d)) {
                return true;
            }
            let Ok(relative) = n.strip_prefix(&real) else { return true };
            let relative = relative.to_string_lossy();
            let inside = format!("{relative}/");
            if n.is_dir() || listed.range(inside.as_str()..).next().is_some_and(|f| f.starts_with(inside.as_str())) {
                return false;
            }
            listed.contains(relative.as_ref()) == n.is_file()
        });
        if holds {
            crate::code_rule_dsl::logic::profile::decision("phase 0: Git's view of the corpus (decision 0105)", || {
                format!("{} files as the last status found them: of {} paths the journal names, none changes the list", under.len(), named.len())
            });
            return (!under.is_empty()).then_some((under, overlays));
        }
    }
    // Watched before the status is read: a change while it reads is named to the next run.
    let places = crate::git_workspace::locations(&real);
    let cursors = places.as_ref().and_then(|(work, own, common)| observe(|o| o.mark(&[real.clone(), work.clone(), own.clone(), common.clone()])));
    let found = observe(|o| o.git(root)).and_then(|g| {
        let prefix = real.strip_prefix(&g.worktree_root).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        let under: Vec<String> = g
            .worktree_files()
            .into_iter()
            .filter_map(|file| if prefix.is_empty() { Some(file) } else { file.strip_prefix(prefix.as_str()).and_then(|f| f.strip_prefix('/')).map(str::to_owned) })
            .collect();
        (!under.is_empty()).then(|| (under, g.overlays.len()))
    });
    if let (Some((work, own, common)), Some(cursors), Some((under, overlays))) = (&places, cursors, &found) {
        let mut read: Vec<PathBuf> = vec![real.clone(), work.join(".git"), own.join("HEAD"), own.join("index")];
        read.extend(real.ancestors().filter(|d| d.starts_with(work) && *d != real.as_path()).map(|d| d.join(".gitignore")));
        read.extend(["HEAD", "refs", "packed-refs", "config", "info"].iter().map(|f| common.join(f)));
        let watched = [&real, work, own, common].iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>().join("\t");
        let read = [cursors.join("\t"), watched].into_iter().chain(read.iter().map(|p| p.to_string_lossy().into_owned())).collect::<Vec<_>>().join("\n");
        let text = std::iter::once(overlays.to_string()).chain(under.iter().cloned()).collect::<Vec<_>>().join("\n");
        observe(|o| o.put_run_record(&key, &read, text.as_bytes()));
    }
    found
}

/// The corpus's files under `root`, relative and in path order: Python's (`.py`), Rust's (`.rs`) and rule files
/// (`.rr`), skipping hidden directories and `target`.
fn corpus_files(root: &Path) -> Result<CorpusFiles> {
    let (mut by_label, mut text): (BTreeMap<&'static str, Vec<String>>, Vec<String>) = (BTreeMap::new(), Vec::new());
    let mut classify = |relative: String| match crate::code_rule_dsl::language::label_of_path(&relative) {
        Some(label) => by_label.entry(label).or_default().push(relative),
        // A hidden file (`.env`, `.hidden.md`) is no text document, as in W.
        None if !relative.rsplit('/').next().is_some_and(|name| name.starts_with('.')) => text.push(relative),
        None => {}
    };
    // In a Git work tree, the files Git sees under the root (HEAD's, less deleted, plus staged and untracked ones it
    // does not ignore), with no walk; the same directories skipped. Outside one, or where Git cannot read it, the walk.
    // A root Git sees no file under (one it ignores, such as fetched fixtures) is not described by Git: walked.
    let git = git_files(root);
    if let Some((under, overlays)) = git {
        let mut listed = 0;
        for relative in &under {
            let relative = relative.as_str();
            let dirs: Vec<&str> = relative.split('/').collect();
            if dirs[..dirs.len() - 1].iter().any(|d| d.starts_with('.') || *d == "target") {
                continue;
            }
            listed += 1;
            classify(relative.to_owned());
        }
        crate::code_rule_dsl::logic::profile::decision("phase 0: corpus files", || format!("{listed} files from Git ({overlays} overlays: changed, staged or untracked), no walk"));
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
            if !path.is_file() {
                continue;
            }
            let relative = path.strip_prefix(root).map_err(|e| Error::Plan(e.to_string()))?.to_string_lossy().into_owned();
            // The language registry's suffixes (`languages/<id>.json`): Python's include stubs (`.pyi`).
            classify(relative);
        }
    }
    }
    by_label.values_mut().for_each(|files| files.sort());
    text.sort();
    // `RERAIL_SAMPLE_EVERY=n`: every n-th file only, to iterate on a rule's cost quickly (its findings are a sample's).
    if let Some(n) = std::env::var("RERAIL_SAMPLE_EVERY").ok().and_then(|n| n.parse::<usize>().ok()).filter(|n| *n > 1) {
        // Documents read as text too: a rule that reads every text document (a check over the repository's docs)
        // would otherwise read all of them in every sample.
        for files in by_label.values_mut().chain([&mut text]) {
            *files = files.iter().step_by(n).cloned().collect();
        }
    }
    Ok(CorpusFiles { by_label, text })
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

/// The tree counts a session's runs stored or read (`Session::own`), by name (`own_name`), with their stored source
/// once it is known: its runs read them from memory, as the store's writer writes them behind them (decision 0102).
pub(in crate::code_rule_dsl) type OwnKept = Arc<parking_lot::Mutex<HashMap<String, (usize, Own, Option<crate::content_store::SourceId>)>>>;
/// The tree counts kept at most (each a few kilobytes).
const OWN_KEPT_MAX: usize = 4096;

/// A chunk mix's measured tree counts and the nodes they were measured over, stored by an earlier run (or this
/// process's, from memory). Its source is named in the run's manifest (`held`), so collection keeps it (decision
/// 0053); one still queued is named by its write (`record_own_counts`).
fn own_counts(store: &ContentStore, kept: Option<&OwnKept>, program: &[u8; 32], mix: &str, held: &mut Vec<crate::content_store::SourceId>) -> Option<(usize, Own)> {
    let key = own_name(program, mix);
    if let Some((nodes, own, source)) = kept.and_then(|k| k.lock().get(&key).cloned()) {
        held.extend(source);
        return Some((nodes, own));
    }
    let digest = store.binding(&key).ok()??;
    let source = store.source_by_hash(crate::content_store::HashKey::from_sha256_digest(digest)).ok()??;
    let (nodes, own) = Own::decode(&store.get_source(source).ok()?)?;
    held.push(source);
    if let Some(kept) = kept {
        keep_own(kept, key, nodes, own.clone(), Some(source));
    }
    Some((nodes, own))
}

fn keep_own(kept: &OwnKept, key: String, nodes: usize, own: Own, source: Option<crate::content_store::SourceId>) {
    let mut kept = kept.lock();
    if kept.len() >= OWN_KEPT_MAX && !kept.contains_key(&key) {
        kept.clear();
    }
    kept.insert(key, (nodes, own, source));
}

/// Stores a chunk mix's measured tree counts, for a later run whose files of that mix are all read from the memo:
/// kept in memory now, written behind the run (decision 0102). The write names its source in the run's manifest
/// (`late`, read by the manifest's job, queued after this one).
fn record_own_counts(store: &ContentStore, kept: Option<&OwnKept>, program: &[u8; 32], mix: &str, nodes: usize, own: &Own, late: &Late) {
    let key = own_name(program, mix);
    if let Some(kept) = kept {
        keep_own(kept, key.clone(), nodes, own.clone(), None);
    }
    let kept = kept.cloned();
    let bytes = own.encode(nodes);
    let producer = memo::plan_digest(program);
    let late = Arc::clone(late);
    store.write_back(
        "the tree counts",
        bytes.len(),
        Vec::new(),
        Box::new(move |store| {
            use sha2::Digest as _;
            let source = store.put_source(&bytes, &[])?;
            late.lock().expect("the late sources").push(source);
            let digest: [u8; 32] = sha2::Sha256::digest(&bytes).into();
            let record = crate::content_store::DerivationRecord { output: key.clone(), producer, inputs: Vec::new(), digest, status: 0, cost: 0 };
            store.record_derivations(&[record])?;
            if let Some(kept) = &kept
                && let Some(entry) = kept.lock().get_mut(&key)
            {
                entry.2 = Some(source);
            }
            Ok(())
        }),
    );
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
            if !prog.inputs.iter().any(|x| x == &atom.rel) {
                continue;
            }
            for (i, arg) in atom.args.iter().enumerate() {
                if let front::Term::Int(v) = arg {
                    out.entry((atom.rel.to_string(), super::sql_col(&atom.rel, i))).or_default().insert(*v);
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
        own.postings = Self::postings(sources, constants, |_| true);
        own
    }

    /// The program's constants' postings in the relations `of_rel` keeps: rows per (relation, column, value).
    fn postings(
        sources: &[(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)],
        constants: &BTreeMap<(String, String), BTreeSet<i64>>,
        of_rel: impl Fn(&str) -> bool,
    ) -> BTreeMap<(String, String, i64), i64> {
        use rustc_hash::{FxHashMap, FxHashSet};
        let mut out = BTreeMap::new();
        for ((rel, col), values) in constants {
            if !of_rel(rel) {
                continue;
            }
            let Some(source) = sources.iter().find(|(r, _)| r == rel).map(|(_, s)| s.as_ref()) else { continue };
            let Some(c) = source.columns().iter().position(|n| n == col) else { continue };
            let wanted: FxHashSet<i64> = values.iter().copied().collect();
            let mut counts: FxHashMap<i64, i64> = FxHashMap::default();
            for r in 0..source.len() {
                let v = source.value(r, c);
                if wanted.contains(&v) {
                    *counts.entry(v).or_default() += 1;
                }
            }
            out.extend(counts.into_iter().map(|(v, n)| ((rel.clone(), col.clone(), v), n)));
        }
        out
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

    /// `other` taken away (a file's share counted out): counts that reach 0 are dropped, as a sum without it holds.
    fn sub(&mut self, other: &Own) {
        for (k, (n, size)) in &other.kinds {
            let e = self.kinds.entry(*k).or_default();
            e.0 -= n;
            e.1 -= size;
            if *e == (0, 0) {
                self.kinds.remove(k);
            }
        }
        for (e, n) in &other.edges {
            let c = self.edges.entry(*e).or_default();
            *c -= n;
            if *c == 0 {
                self.edges.remove(e);
            }
        }
        self.docs -= other.docs;
        for (key, n) in &other.postings {
            let c = self.postings.entry(key.clone()).or_default();
            *c -= n;
            if *c == 0 {
                self.postings.remove(key);
            }
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

    /// Add a frame directly to the run's totals. No per-file maps are reconstructed.
    fn add_rows(&mut self, rows: &BTreeMap<String, (usize, Vec<i64>)>) {
        let flat = |key: &str| rows.get(key).map(|(_, v)| v.as_slice()).unwrap_or_default();
        for r in flat(OWN_KINDS).chunks_exact(3) {
            let k = self.kinds.entry(r[0]).or_default(); k.0 += r[1]; k.1 += r[2];
        }
        for r in flat(OWN_EDGES).chunks_exact(3) { *self.edges.entry((r[0], r[1])).or_default() += r[2]; }
        self.docs += flat(OWN_DOCS).first().copied().unwrap_or(0);
        for (key, (_, values)) in rows.range(OWN_POSTINGS.to_owned()..).take_while(|(k, _)| k.starts_with(OWN_POSTINGS)) {
            let Some((rel, col)) = key[OWN_POSTINGS.len()..].split_once(':') else { continue };
            for r in values.chunks_exact(2) { *self.postings.entry((rel.into(), col.into(), r[0])).or_default() += r[1]; }
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
pub mod tests {

    #[test]
    fn constant_demands_reach_projected_input_columns() {
        let p=crate::code_rule_dsl::logic::rr::parse(".decl raw(d:number,n:number,k:number)\n.input raw\n.decl tags(n:number,k:number)\ntags(n,k) :- raw(_,n,k).\n.decl alias(n:number,k:number)\nalias(n,k) :- tags(n,k).\n.decl out(n:number)\nout(n) :- alias(n,9).\n").unwrap();
        let demands=super::constants_tested(&p);
        assert!(demands["alias"].contains(&(1,9)));
        assert!(demands["tags"].contains(&(1,9)));
        assert!(demands["raw"].contains(&(2,9)),"measure the actual source column before planning its consumers");
    }

    #[test]
    fn memo_tree_statistics_accumulate_once_without_per_file_maps() {
        use super::*;
        let mut first = Own::default();
        first.kinds.insert(1,(10,20)); first.edges.insert((1,2),5); first.docs=1;
        first.postings.insert(("kind".into(),"k".into(),1),10);
        let mut rows=BTreeMap::new(); first.put(&mut rows);
        let mut totals=Own::default(); totals.add_rows(&rows); totals.add_rows(&rows);
        assert_eq!(totals.kinds[&1],(20,40)); assert_eq!(totals.edges[&(1,2)],10);
        assert_eq!(totals.docs,2); assert_eq!(totals.postings.values().sum::<i64>(),20);
        let projected=BTreeMap::from([(OWN_DOCS.into(),(1,vec![1]))]);
        let mut narrow=Own::default(); narrow.add_rows(&projected);
        assert_eq!(narrow.docs,1); assert!(narrow.kinds.is_empty()); assert!(narrow.edges.is_empty());
    }

    /// Phase 0 alone over `RERAIL_P0_ROOT`: the corpus's files (from Git in a Git work tree, else the walk) and
    /// every Python document's digest, with the observer's counts and the CPU. Run it twice for the warm figure; the
    /// observer's store is `<RERAIL_TAPE_CACHE>.files`.
    #[test]
    #[ignore = "a benchmark: RERAIL_P0_ROOT=<a worktree>"]
    fn benchmark_phase_0() {
        let root = std::path::PathBuf::from(std::env::var("RERAIL_P0_ROOT").expect("RERAIL_P0_ROOT"));
        let before = super::observe(|o| {
            o.begin();
            o.counts
        });
        let start = super::thread_usage();
        let files = super::corpus_files(&root).unwrap();
        let listed = super::thread_usage();
        let mut digests = 0;
        for f in files.by_label.get("python").into_iter().flatten() {
            digests += super::observe(|o| o.digest(&root.join(f))).map_or(0, |_| 1);
        }
        let digested = super::thread_usage();
        let did = super::observe(|o| {
            o.flush();
            o.counts.since(&before)
        });
        let flushed = super::thread_usage();
        let cpu = |a: (f64, f64), b: (f64, f64)| (b.0 - a.0) + (b.1 - a.1);
        println!(
            "phase 0 over {}: {} python, {} other files; {digests} digests; files {:.2} s, digests {:.2} s, flush {:.2} s CPU; peak RSS {:.0} MB\n  {did}",
            root.display(),
            files.count("python"),
            files.parsed() - files.count("python") + files.text.len(),
            cpu(start, listed),
            cpu(listed, digested),
            cpu(digested, flushed),
            super::usage().2
        );
    }
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
        // A memoised relation is stored, though inlining would unfold it into its readers (decision 0099): its derived
        // rows are read from its table to be recorded ("no such table: near" before).
        let near = program("uses near", ".decl near(n: node, f: node)\nnear(n, f) :- uses(n, f).\n.decl far(n: node)\nfar(n) :- near(n, _).\n.output far");
        // SAFETY: only this test reads RERAIL_INLINE_ONLY while it is set (the others declare no pragma).
        unsafe { std::env::set_var("RERAIL_INLINE_ONLY", "near") };
        let stored = run(&near);
        unsafe { std::env::remove_var("RERAIL_INLINE_ONLY") };
        stored.unwrap();
        assert_eq!(std::fs::read_to_string(out.join("far.csv")).unwrap().lines().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Decision 0099, the factorized store across the chunk boundary. `outer`'s join of `inside(f, p)` and
    /// `bound(p, x)` is handed on as its two parts: the findings are those of the product handed on whole, the memo's
    /// entries hold the parts (whose join is the product's rows) and not the product, and a run whose choice differs
    /// from the warm memo's (`factorize: off` then on, and back) misses the memo and finds the same.
    #[cfg(feature = "tape-cache")]
    #[test]
    #[ignore = "O-factorize-nothing: `uses` is no longer file-local, so no export is handed on as its parts and the factorized run's plan is the unfactorized one's"]
    fn an_export_handed_on_as_its_parts_finds_the_same_and_a_changed_choice_misses_the_memo() {
        use serde_json::{Value, json};
        let marker = format!("# {:?}\n", std::time::SystemTime::now());
        let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("factorized");
        std::fs::create_dir_all(&dir).unwrap();
        let body = "def f():\n    x = 1\n    y = 2\n    print(x, y)\n    def g():\n        print(x, y)\n        z = x\n    return y\n";
        std::fs::write(dir.join("a.py"), format!("{marker}{body}")).unwrap();
        std::fs::write(dir.join("b.py"), format!("{marker}{body}def h():\n    x = 3\n    print(x)\n")).unwrap();
        let rule = dir.join("outer.rr");
        // `outer`: a statement of a function's body, and a name the function assigns that some file reads. Its
        // file-local join, `encl(f, p), bound(p, x)`, meets at the function `p`, which it projects away. Its
        // cross-file filter, `named(x)`, binds only `x`: one that bound `f` and `x` both (`uses(f, x)`) would keep the
        // join in the cross part, unpushed (t31 A1), and nothing would be handed on to factorize.
        std::fs::write(&rule, r#"
.decl defined(s: symbol)
defined(s) :- kind(n, "identifier"), text(n, s).
.decl named(x: symbol)
named(x) :- kind(i, "identifier"), text(i, x), defined(x).
.decl encl(f: node, p: node)
encl(f, p) :- kind(p, "function_definition"), child(p, b, _), kind(b, "block"), child(b, f, _).
.decl bound(p: node, x: symbol)
bound(p, x) :- kind(p, "function_definition"), child(p, b, _), kind(b, "block"), child(b, e, _), child(e, a, _), kind(a, "assignment"), field(a, "left", l), text(l, x).
.decl outer(f: node, x: symbol)
outer(f, x) :- named(x), encl(f, p), bound(p, x).
.output outer
"#).unwrap();
        let program = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
        let out = dir.join("out");
        let store = dir.join("store.sqlite");
        let _no_session = super::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(store.clone()));
        let run = |on: bool| {
            let options = if on { Value::Null } else { json!({"factorize": "off"}) };
            let report = super::run_with_options(&[&program], &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18, &super::Typing::default(), &options).unwrap();
            (report.memo.0, std::fs::read_to_string(out.join("outer.csv")).unwrap())
        };
        let (hits, whole) = run(false);
        assert_eq!(hits, 0, "a new memo");
        assert!(whole.lines().count() >= 4, "{whole}");
        assert_eq!(run(false), (2, whole.clone()), "the same choice: every file from the memo");
        assert_eq!(run(true), (0, whole.clone()), "factorized: another file-local part, so every file a miss, and the same findings");
        assert_eq!(run(true), (2, whole.clone()), "the factorized run's own entries");
        // The rows the memo holds: of the parts, as the last run left them; then of the product, after a run with it.
        let bound = |prefix: &str| -> (Vec<String>, Vec<Vec<Value>>) {
            let mut executor = crate::code_rule_dsl::sql_executor::CodeRuleSqlExecutor::new(store.to_string_lossy().into_owned()).unwrap();
            let got: Value = serde_json::from_str(&executor.bind_corpus(dir.to_string_lossy().into_owned(), None).unwrap()).unwrap();
            let mut names: Vec<String> = got["relations"].as_object().unwrap().keys().filter(|r| r.starts_with(prefix)).cloned().collect();
            names.sort();
            let rows = names
                .iter()
                .map(|r| serde_json::from_str::<Value>(&executor.query(format!("SELECT * FROM \"{r}\""), None, None).unwrap()).unwrap()["rows"].as_array().unwrap().clone())
                .collect::<Vec<Vec<Value>>>()
                .into_iter()
                .flatten()
                .map(|r| r.as_array().unwrap().clone())
                .collect();
            (names, rows)
        };
        let (parts, _) = bound("fz");
        assert_eq!(parts.len(), 2, "two parts handed on: {parts:?}");
        assert!(bound("fl").0.iter().all(|r| !r.contains("_outer")), "no product handed on");
        // Each part's rows: (the head's column, the key); a node column names the row's document.
        let part = |name: &str| bound(name).1.into_iter().map(|r| (r[0].to_string(), r[1].to_string())).collect::<std::collections::BTreeSet<_>>();
        let (a, b) = (part(&parts[0]), part(&parts[1]));
        let mut joined: std::collections::BTreeSet<(String, String)> = std::collections::BTreeSet::new();
        for (u, k) in &a {
            for (v, k2) in &b {
                if k == k2 {
                    joined.insert((u.clone(), v.clone()));
                }
            }
        }
        // The parts are reduced: every row stored joins.
        assert!(a.iter().all(|(_, k)| b.iter().any(|(_, k2)| k == k2)) && b.iter().all(|(_, k)| a.iter().any(|(_, k2)| k == k2)));
        assert_eq!(run(false), (2, whole.clone()), "back to the product: its entries are still kept");
        let (products, _) = bound("fl");
        let product: Vec<&String> = products.iter().filter(|r| r.contains("_outer")).collect();
        assert_eq!(product.len(), 1, "{products:?} of {:?}", bound("").0);
        let rows: std::collections::BTreeSet<(String, String)> = bound(product[0]).1.into_iter().map(|r| (r[0].to_string(), r[1].to_string())).collect();
        assert!(!rows.is_empty());
        // The product's rows, in either order of the head's columns, are the parts' join.
        let flipped: std::collections::BTreeSet<(String, String)> = joined.iter().map(|(u, v)| (v.clone(), u.clone())).collect();
        assert!(rows == joined || rows == flipped, "the parts' join is the product: {} rows against {}", joined.len(), rows.len());
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// t33 K4: with no session, a run starts from the cross-file part its store's frames hold and maintains it from
    /// the delta of its inputs, as a session does from memory. After each edit the findings are those of a run with
    /// an empty store, and an edit's strata are maintained from deltas; an import cycle made and broken maintains a
    /// recursive stratum.
    #[test]
    fn the_cross_file_part_is_maintained_from_its_frames_in_a_new_process() {
        use crate::code_rule_dsl::typing::file_local::Binding;
        // It ran past its timeout once in 34 runs under load, with no output: a run past 60 s leaves its stacks.
        let _watchdog = watchdog("k4", std::time::Duration::from_secs(60));
        let _no_session = super::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let base = std::env::temp_dir().join(format!("rerail-k4-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let out = base.join("out");
        let findings = || {
            let mut rows: Vec<String> = std::fs::read_to_string(out.join("violation.csv")).unwrap_or_default().lines().map(str::to_owned).collect();
            rows.sort();
            rows
        };
        // Each run is a new process's as far as the cross-file part goes: no session, the store on disk. One with a
        // store of its own keeps nothing.
        // Each scenario keeps its own store: relations are shared by identity across programs (0103), so a later
        // scenario's first run would find clean those of an earlier one whose inputs have the same rows.
        let fresh = std::cell::Cell::new(0);
        let scenario = std::cell::Cell::new(0);
        let run = |texts: &[&str], src: &std::path::Path, kept: bool| {
            let path = if kept {
                base.join(format!("kept{}.sqlite", scenario.get()))
            } else {
                fresh.set(fresh.get() + 1);
                base.join(format!("fresh{}.sqlite", fresh.get()))
            };
            super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(path));
            let report = super::run_all(texts, src, &out, "program", Binding::Cpu, 1 << 18).unwrap();
            (report, findings())
        };
        let edit = |texts: &[&str], src: &std::path::Path, edits: &[(&str, &str)]| {
            scenario.set(scenario.get() + 1);
            let (first, expected) = run(texts, src, true);
            let (ran, all) = first.cross_kept.expect("a run with a store keeps its cross-file part");
            assert_eq!(ran, all, "the first run evaluates every stratum");
            assert_eq!(run(texts, src, false).1, expected);
            let (mut maintained, mut recursive) = (0, 0);
            for (path, text) in edits {
                std::fs::write(src.join(path), text).unwrap();
                let (report, got) = run(texts, src, true);
                let (_, want) = run(texts, src, false);
                assert_eq!(got, want, "after editing {path}, the findings are those of a run with an empty store");
                eprintln!("{path}: {:?} strata, {:?} maintained (recursive, derived again; {:?}); {} findings", report.cross_kept, report.cross_maintained, report.cross_again, got.len());
                maintained += report.cross_maintained.0;
                recursive += report.cross_maintained.1;
            }
            (maintained, recursive)
        };
        // Joins and negations: two bandit rules.
        let src = base.join("src");
        std::fs::create_dir_all(src.join("pkg")).unwrap();
        std::fs::write(src.join("pkg/__init__.py"), "").unwrap();
        std::fs::write(src.join("pkg/loader.py"), "import pickle as p\n\ndef load(b):\n    return p.loads(b)\n").unwrap();
        std::fs::write(src.join("main.py"), "from pkg.loader import load\nload(b'x')\n").unwrap();
        std::fs::write(src.join("other.py"), "import json\njson.loads('1')\n").unwrap();
        let rules = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).join("../docs/rule-syntax/packs/python");
        let programs: Vec<String> = ["bandit-b301-pickle.rr", "bandit-b307-eval.rr"].iter().map(|r| crate::code_rule_dsl::tools::flatten(&rules.join(r)).unwrap()).collect();
        let texts: Vec<&str> = programs.iter().map(String::as_str).collect();
        let (maintained, _) = edit(
            &texts,
            &src,
            &[
                ("other.py", "import json\nimport pickle\njson.loads('1')\npickle.loads(b'2')\n"),
                ("main.py", "from pkg.loader import load\nload(b'x')\neval('1')\n"),
                ("pkg/loader.py", "import marshal as p\n\ndef load(b):\n    return p.loads(b)\n"),
            ],
        );
        assert!(maintained > 0, "an edit's strata are maintained from deltas");
        // Recursion: an import cycle, broken and closed again.
        let src = base.join("cycle");
        std::fs::create_dir_all(&src).unwrap();
        for (path, text) in [("a.py", "from b import x\n"), ("b.py", "from c import y\n"), ("c.py", "from a import z\n"), ("d.py", "x = 1\n")] {
            std::fs::write(src.join(path), text).unwrap();
        }
        let rule = base.join("reach.rr");
        std::fs::write(
            &rule,
            ".policy \"test-import-cycle\" revision 1\n.description \"An import that closes a cycle of imports.\"\n.language python\n.mode steer\n\
             .scope include \"**/*.py\"\n#include \"python/names.rr\"\n.decl dep(a: number, b: number)\ndep(a, b) :- impmod(i, m), docof(i, a), doc(b, _, m).\n\
             .decl reach(a: number, b: number)\nreach(a, b) :- dep(a, b).\nreach(a, c) :- reach(a, b), dep(b, c).\n\
             .violation CYC(at: node) code \"CYC\" \"This import closes a cycle of imports.\"\nCYC(i) :- impmod(i, m), docof(i, a), doc(b, _, m), reach(b, a).\n",
        )
        .unwrap();
        let program = crate::code_rule_dsl::tools::flatten(&rule).unwrap();
        let (_, recursive) = edit(&[program.as_str()], &src, &[("c.py", "from d import z\n"), ("d.py", "from a import w\n"), ("b.py", "x = 2\n"), ("b.py", "from c import y\n")]);
        assert!(findings().len() >= 3, "the cycle's imports: {:?}", findings());
        assert!(recursive > 0, "the import cycle's stratum is maintained");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A guard that, when the test it guards runs past `after`, samples this process's stacks (macOS `sample`) into
    /// a file beside the temp dir and says where: a hang leaves what each thread was doing. Dropped, it does nothing.
    pub(super) fn watchdog(name: &str, after: std::time::Duration) -> Watchdog {
        let done = std::sync::Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
        let seen = std::sync::Arc::clone(&done);
        let name = name.to_owned();
        std::thread::spawn(move || {
            let (lock, cv) = &*seen;
            let (finished, _) = cv.wait_timeout_while(lock.lock().unwrap(), after, |d| !*d).unwrap();
            if *finished {
                return;
            }
            let out = std::env::temp_dir().join(format!("rerail-watchdog-{name}-{}.txt", std::process::id()));
            let sampled = std::process::Command::new("sample").arg(std::process::id().to_string()).arg("3").arg("-file").arg(&out).output();
            eprintln!("watchdog {name}: still running after {after:?}; stacks in {} ({:?})", out.display(), sampled.map(|o| o.status));
        });
        Watchdog(done)
    }

    pub(super) struct Watchdog(std::sync::Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>);

    impl Drop for Watchdog {
        fn drop(&mut self) {
            let (lock, cv) = &*self.0;
            *lock.lock().unwrap_or_else(|e| e.into_inner()) = true;
            cv.notify_all();
        }
    }

    /// Decision 0077 §4: with no session, the cross part's strata are kept in the tape cache. A run over unchanged
    /// files evaluates fewer strata than the first; an edit that changes no cross-file row leaves them clean too; an
    /// edit that does derives what a run with an empty cache derives.
    #[test]
    fn cross_file_strata_are_kept_across_runs_in_the_tape_cache() {
        let marker = format!("# {:?}\n", std::time::SystemTime::now());
        let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("kept-strata");
        std::fs::create_dir_all(&dir).unwrap();
        let write = |file: &str, body: &str| std::fs::write(dir.join(file), format!("{marker}{body}")).unwrap();
        write("a.py", "def f():\n    pass\n");
        write("b.py", "from a import f\ndef g():\n    pass\n");
        write("c.py", "def h():\n    pass\n");
        let rule = dir.join("uses.rr");
        std::fs::write(&rule, r#"
.decl def(n: node, q: symbol)
def(n, @dot(m, x)) :- kind(n, "function_definition"), field(n, "name", c), text(c, x), docof(n, d), doc(d, _, m).
.decl imp(n: node, q: symbol)
imp(n, @dot(m, x)) :- kind(n, "import_from_statement"), field(n, "module_name", mn), text(mn, m), field(n, "name", xn), text(xn, x).
.decl uses(n: node, f: node)
uses(n, f) :- imp(n, q), def(f, q).
.decl used(f: node)
used(f) :- uses(_, f).
.decl unused(f: node)
unused(f) :- def(f, _), !used(f).
.output uses
.output unused
"#).unwrap();
        let text = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
        let out = dir.join("out");
        let _no_session = super::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let run = |store: &str| {
            super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join(store)));
            let report = super::run(&text, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
            let read = |f: &str| std::fs::read_to_string(out.join(f)).unwrap();
            (report.cross_kept, read("uses.csv") + &read("unused.csv"))
        };
        let (first, derived) = run("kept.sqlite");
        let (ran, all) = first.expect("a run with a tape cache keeps its cross-file strata");
        assert_eq!(ran, all, "the first run evaluates every stratum");
        let (again, rows) = run("kept.sqlite");
        assert_eq!(rows, derived);
        assert!(again.unwrap().0 < all, "unchanged: strata kept ({again:?} of {all})");
        // c's function returns: no cross-file row changes.
        write("c.py", "def h():\n    return 1\n");
        let (edited, rows) = run("kept.sqlite");
        assert_eq!(rows, derived);
        assert!(edited.unwrap().0 < all, "no cross-file row changed: strata kept ({edited:?} of {all})");
        // b no longer imports f: uses is empty, f unused; as a run with no kept strata derives.
        write("b.py", "def g():\n    pass\n");
        let (_, rows) = run("kept.sqlite");
        let (_, fresh) = run("fresh.sqlite");
        assert_eq!(rows, fresh);
        assert_ne!(rows, derived);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The cone of requests is kept like any relation (t33 K1, K3): its stratum that asks for directories, files and
    /// loads and answers them inside its fixpoint is clean while what it reads and its answers are as they were. An
    /// edit that changes neither evaluates none of the cone; a new import, or a library file whose own imports
    /// change, runs it, and the documents loaded are those a run with nothing kept loads.
    #[test]
    fn the_cone_of_requests_is_kept_while_its_reads_and_answers_are() {
        let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("cone-kept");
        let _ = std::fs::remove_dir_all(&dir);
        let (ws, lib) = (dir.join("ws"), dir.join("ts/stdlib"));
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(lib.join("VERSIONS"), "builtins: 3.0-\na: 3.0-\nb: 3.0-\nc: 3.0-\n").unwrap();
        std::fs::write(lib.join("builtins.pyi"), "class object: ...\n").unwrap();
        for m in ["a", "b", "c"] {
            std::fs::write(lib.join(format!("{m}.pyi")), format!("def {m}() -> None: ...\n")).unwrap();
        }
        std::fs::write(ws.join("rerail.env"), "python.typeshed = ../ts\n").unwrap();
        std::fs::write(ws.join("main.py"), "import a\n").unwrap();
        let rule = dir.join("p.rr");
        std::fs::write(&rule, "#include \"python/modules.rr\"\n.decl n_loaded(p: symbol)\nn_loaded(p) :- loaded(p, _, _).\n.output n_loaded\n").unwrap();
        let text = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
        let out = dir.join("out");
        let _no_session = super::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let run = |store: &str| {
            super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join(store)));
            let report = super::run(&text, &ws, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
            let mut loaded: Vec<String> = std::fs::read_to_string(out.join("n_loaded.csv")).unwrap().lines().map(|l| l.rsplit('/').next().unwrap().to_owned()).collect();
            loaded.sort();
            (report.cone_kept.expect("a run with a tape cache keeps its cone's strata"), loaded)
        };
        let ((ran, all), first) = run("kept.sqlite");
        // The first run evaluates the cone's strata that phase 1 did not: phase 1 (`sources.rr`) resolves the same
        // imports first, and a relation is kept under its identity, not its program's (decision 0103).
        assert!(ran > 0 && ran <= all, "the first run evaluates the cone ({ran} of {all} strata)");
        assert_eq!(first, ["a.pyi", "builtins.pyi"]);
        // A comment: the cone reads nothing new, and its answers are as they were.
        std::fs::write(ws.join("main.py"), "import a\n# a note\n").unwrap();
        let ((ran, all), same) = run("kept.sqlite");
        assert_eq!((ran, &same), (0, &first), "an edit that changes no read and no answer evaluates none of the cone (of {all})");
        // A new import: a read changes, and the cone runs.
        std::fs::write(ws.join("main.py"), "import a\nimport b\n# a note\n").unwrap();
        let ((ran, _), grown) = run("kept.sqlite");
        assert!(ran > 0, "a new import runs the cone");
        assert_eq!(grown, run("fresh-1.sqlite").1, "the documents loaded are those a run with nothing kept loads");
        assert_eq!(grown, ["a.pyi", "b.pyi", "builtins.pyi"]);
        // A library file's own import: the answer to its load changes, and the cone runs.
        std::fs::write(lib.join("b.pyi"), "import c\ndef b() -> None: ...\n").unwrap();
        let ((ran, _), deeper) = run("kept.sqlite");
        assert!(ran > 0, "a loaded document's change is a changed answer: the cone runs");
        assert_eq!(deeper, ["a.pyi", "b.pyi", "builtins.pyi", "c.pyi"]);
        assert_eq!(deeper, run("fresh-2.sqlite").1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Decision 0108: the cone run as rounds from the last run's final round finds what the gated fixpoint over
    /// nothing kept finds, edit after edit: a new import, a library's own import, an import taken away (its load is
    /// not asked again), and a library file that goes.
    #[test]
    fn the_cone_maintained_by_rounds_loads_what_a_fresh_cone_loads() {
        let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("cone-rounds");
        let _ = std::fs::remove_dir_all(&dir);
        let (ws, lib) = (dir.join("ws"), dir.join("ts/stdlib"));
        std::fs::create_dir_all(ws.join("pkg")).unwrap();
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(lib.join("VERSIONS"), "builtins: 3.0-\na: 3.0-\nb: 3.0-\nc: 3.0-\nd: 3.0-\n").unwrap();
        std::fs::write(lib.join("builtins.pyi"), "class object: ...\n").unwrap();
        for m in ["a", "b", "c", "d"] {
            std::fs::write(lib.join(format!("{m}.pyi")), format!("def {m}() -> None: ...\n")).unwrap();
        }
        std::fs::write(ws.join("rerail.env"), "python.typeshed = ../ts\n").unwrap();
        std::fs::write(ws.join("main.py"), "import a\nimport pkg.m\n").unwrap();
        std::fs::write(ws.join("pkg/__init__.py"), "").unwrap();
        std::fs::write(ws.join("pkg/m.py"), "import b\n").unwrap();
        let rule = dir.join("p.rr");
        std::fs::write(&rule, "#include \"python/modules.rr\"\n.decl n_loaded(p: symbol)\nn_loaded(p) :- loaded(p, _, _).\n.output n_loaded\n").unwrap();
        let text = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
        let out = dir.join("out");
        let _no_session = super::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let fresh = std::cell::Cell::new(0);
        let run = |store: &str| {
            super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join(store)));
            let report = super::run(&text, &ws, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
            let mut loaded: Vec<String> = std::fs::read_to_string(out.join("n_loaded.csv")).unwrap().lines().map(|l| l.rsplit('/').next().unwrap().to_owned()).collect();
            loaded.sort();
            (report.cone_rounds, loaded)
        };
        let both = |what: &str, want: &[&str]| {
            let (rounds, kept) = run("kept.sqlite");
            fresh.set(fresh.get() + 1);
            let (none, again) = run(&format!("fresh-{}.sqlite", fresh.get()));
            assert!(rounds > 0, "{what}: the kept run runs rounds");
            assert_eq!(none, 0, "{what}: a run with nothing kept runs the fixpoint");
            assert_eq!(kept, again, "{what}: rounds load what the fixpoint loads");
            assert_eq!(kept, want, "{what}");
        };
        assert_eq!(run("kept.sqlite"), (0, vec!["a.pyi".to_owned(), "b.pyi".to_owned(), "builtins.pyi".to_owned()]), "the first run is the fixpoint");
        std::fs::write(ws.join("main.py"), "import a\nimport pkg.m\nimport c\n").unwrap();
        both("a new import", &["a.pyi", "b.pyi", "builtins.pyi", "c.pyi"]);
        std::fs::write(lib.join("c.pyi"), "import d\ndef c() -> None: ...\n").unwrap();
        both("a library's own import", &["a.pyi", "b.pyi", "builtins.pyi", "c.pyi", "d.pyi"]);
        std::fs::write(ws.join("pkg/m.py"), "x = 1\n").unwrap();
        both("an import taken away", &["a.pyi", "builtins.pyi", "c.pyi", "d.pyi"]);
        std::fs::remove_file(lib.join("d.pyi")).unwrap();
        both("a library file gone", &["a.pyi", "builtins.pyi", "c.pyi"]);
        // The workspace's own module now: the library it shadowed was loaded from the last run's gates, and is taken
        // back (decision 0108).
        std::fs::write(ws.join("c.py"), "x = 2\n").unwrap();
        both("a library shadowed by the workspace's module", &["a.pyi", "builtins.pyi"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// t32 4.1: a new process maintains its cross-file part from the rows the last process kept, as a session does.
    /// After an edit that changes a cross-file row, a changed stratum is maintained from its deltas, not derived
    /// again, and the outputs are a run's with an empty store.
    #[test]
    fn a_new_process_maintains_the_cross_file_part_from_the_kept_rows() {
        let marker = format!("# {:?}\n", std::time::SystemTime::now());
        let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("kept-part");
        std::fs::create_dir_all(&dir).unwrap();
        let write = |file: &str, body: &str| std::fs::write(dir.join(file), format!("{marker}{body}")).unwrap();
        write("a.py", "def f():\n    pass\ndef k():\n    pass\n");
        write("b.py", "from a import f\ndef g():\n    pass\n");
        write("c.py", "from a import k\ndef h():\n    pass\n");
        let rule = dir.join("uses.rr");
        std::fs::write(&rule, r#"
.decl def(n: node, q: symbol)
def(n, @dot(m, x)) :- kind(n, "function_definition"), field(n, "name", c), text(c, x), docof(n, d), doc(d, _, m).
.decl imp(n: node, q: symbol)
imp(n, @dot(m, x)) :- kind(n, "import_from_statement"), field(n, "module_name", mn), text(mn, m), field(n, "name", xn), text(xn, x).
.decl uses(n: node, f: node)
uses(n, f) :- imp(n, q), def(f, q).
.decl used(f: node)
used(f) :- uses(_, f).
.decl unused(f: node)
unused(f) :- def(f, _), !used(f).
.output unused
"#).unwrap();
        let text = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
        let out = dir.join("out");
        let _no_session = super::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let run = |store: &str| {
            super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join(store)));
            let report = super::run(&text, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
            (report.cross_maintained.0, std::fs::read_to_string(out.join("unused.csv")).unwrap())
        };
        let (cold, derived) = run("kept.sqlite");
        assert_eq!(cold, 0, "nothing kept: every stratum derived");
        // c no longer imports k: k is unused; uses and used are maintained from their deltas.
        write("c.py", "def h():\n    pass\n");
        let (maintained, rows) = run("kept.sqlite");
        let (_, fresh) = run("fresh.sqlite");
        assert_eq!(rows, fresh);
        assert_ne!(rows, derived);
        assert!(maintained > 0, "a changed stratum maintained from the kept rows ({maintained})");
        // And back: the next process maintains from what this one kept.
        write("c.py", "from a import k\ndef h():\n    pass\n");
        let (again, rows) = run("kept.sqlite");
        assert_eq!(rows, derived);
        assert!(again > 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Decision 0103 (t33 K1): a relation is kept as a frame under its identity, not its program's. A second program
    /// with the same rules for `def`, `imp` and `uses` finds them clean on its first run, and the first program's
    /// relations are still clean after it. A rule edit makes new only the relations it reaches.
    #[test]
    fn a_relation_two_programs_share_is_kept_once_and_a_rule_edit_reaches_only_its_readers() {
        let marker = format!("# {:?}\n", std::time::SystemTime::now());
        let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("frames-shared");
        std::fs::create_dir_all(&dir).unwrap();
        let write = |file: &str, body: &str| std::fs::write(dir.join(file), format!("{marker}{body}")).unwrap();
        write("a.py", "def f():\n    pass\n");
        write("b.py", "from a import f\ndef g():\n    pass\n");
        let shared = r#"
.decl def(n: node, q: symbol)
def(n, @dot(m, x)) :- kind(n, "function_definition"), field(n, "name", c), text(c, x), docof(n, d), doc(d, _, m).
.decl imp(n: node, q: symbol)
imp(n, @dot(m, x)) :- kind(n, "import_from_statement"), field(n, "module_name", mn), text(mn, m), field(n, "name", xn), text(xn, x).
.decl uses(n: node, f: node)
uses(n, f) :- imp(n, q), def(f, q).
.output uses
"#;
        let unused = ".decl used(f: node)\nused(f) :- uses(_, f).\n.decl unused(f: node)\nunused(f) :- def(f, _), !used(f).\n.output unused\n";
        let named = ".decl named(q: symbol)\nnamed(q) :- uses(_, f), def(f, q).\n.output named\n";
        let out = dir.join("out");
        let _no_session = super::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("kept.sqlite")));
        let run = |extra: &str| {
            let rule = dir.join("p.rr");
            std::fs::write(&rule, format!("{shared}{extra}")).unwrap();
            let text = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
            let report = super::run(&text, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
            report.cross_kept.expect("a run with a tape cache keeps its cross-file strata")
        };
        let (ran, all) = run(unused);
        assert_eq!(ran, all, "the first run evaluates every stratum");
        let (ran_b, all_b) = run(named);
        assert!(ran_b < all_b, "the shared relations are clean for another program ({ran_b} of {all_b})");
        let (ran_a, _) = run(unused);
        assert!(ran_a < all, "the first program's relations are still kept ({ran_a} of {all})");
        // An edit to `unused`'s rule: `uses` and what it reads stay clean.
        let (edited, all_e) = run(&unused.replace("!used(f)", "!used(f), f != 0"));
        assert!(edited < all_e, "a rule edit reruns only what it reaches ({edited} of {all_e})");
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
        super::record_run(&store, &root, &[source], &program, &[], &[], &Default::default(), None, true).unwrap().unwrap().join().unwrap().unwrap().1.unwrap();
        assert!(store.corpus_collection_due().unwrap());
        // Same files and plan: the manifest is unchanged, but pending collection must still run.
        super::record_run(&store, &root, &[source], &program, &[], &[], &Default::default(), Some(reader), true).unwrap().unwrap().join().unwrap().unwrap().1.unwrap();
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

    /// Decision 0089: a library is held once, and a request's cone once per distinct request. Two programs ask
    /// `compare.rr` the same, a third asks something else, a fourth asks nothing and reads the library's own copy.
    #[test]
    fn libraries_are_held_once_and_equal_requests_share_one_cone() {
        let ask = |kind: &str, own: &str| {
            format!(
                "#include \"python/compare.rr\"\ncmp_same_ask(a, b) :- kind(a, \"{kind}\"), kind(b, \"{kind}\"), a < b.\n.decl {own}(a: node)\n{own}(a) :- cmp_same(a, _).\n.output {own}\n"
            )
        };
        let quiet = "#include \"python/compare.rr\"\n.decl quiet(a: node)\nquiet(a) :- cmp_same(a, _).\n.output quiet\n".to_owned();
        let texts = [ask("call", "one"), ask("call", "two"), ask("name", "three"), quiet];
        let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
        let prog = super::combine_texts(&texts, "").unwrap();
        let heads = |rel: &str| prog.rules.iter().filter(|r| r.head.rel == rel).count();
        // The library's rule once, unprefixed, whatever the programs read.
        assert_eq!(heads("cmp_same"), 1);
        // Programs 0 and 1 ask the same: one copy of the cone; program 2 its own.
        let copies: std::collections::BTreeSet<&str> = prog.rules.iter().map(|r| r.head.rel.as_str()).filter(|r| r.starts_with('q') && r.ends_with("_cmp_same")).collect();
        assert_eq!(copies.len(), 2, "{copies:?}");
        assert!(copies.iter().all(|r| heads(r) == 1));
        assert!(!prog.rules.iter().any(|r| r.head.rel.starts_with('p') && r.head.rel.ends_with("_cmp_same")));
        // Each asker reads its request's copy; the quiet one reads the library's.
        let reads = |head: &str| -> Vec<String> {
            prog.rules.iter().filter(|r| r.head.rel == head).flat_map(|r| r.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.to_string())).collect()
        };
        let (one, two, three) = (reads("p0_one"), reads("p1_two"), reads("p2_three"));
        assert_eq!(one, two);
        assert_ne!(one, three);
        assert_eq!(reads("p3_quiet"), ["cmp_same"]);
    }
}

/// What a kept relation's lazy statistics leave the session (decision 0101 §4): its form kept by delta, or its
/// measure.
pub(in crate::code_rule_dsl) enum LazyKept {
    Patched(retain::Patched),
    Measured(Vec<(String, i64, i64, i64, i64)>, Vec<(String, i64, i64)>, BTreeSet<(usize, i64)>),
}

/// A reader's refusal of one of a run's programs, named by the program's `.policy` (a run reads texts, not files).
fn in_policy(text: &str, e: crate::code_rule_dsl::logic::front::Error) -> crate::code_rule_dsl::logic::front::Error {
    use crate::code_rule_dsl::logic::front::Error;
    let name = text.lines().find_map(|l| l.trim().strip_prefix(".policy ").and_then(|r| r.split('"').nth(1)).map(str::to_owned));
    match (e, name) {
        (Error::Syntax(m), Some(n)) => Error::Syntax(format!("policy {n}: {m}")),
        (e, _) => e,
    }
}

/// Where the requests answered inside the fixpoint spent their time (decision 0114 §6), on the thread that
/// answers them: requests taken (resolved and ordered), chunks waited for (built on workers), chunks evaluated
/// here, answer rows read back, and the time this thread waited for the cone's next request.
#[derive(Default)]
struct AnswerTimes {
    calls: usize,
    rows: usize,
    fresh: usize,
    again: usize,
    total: f64,
    take: f64,
    built: f64,
    evaluate: f64,
    rows_of: f64,
    idle: f64,
}

impl AnswerTimes {
    fn report(&self) {
        let text = || {
            format!(
                "{} requests ({} rows), {} documents loaded, {} answered again: {:.3} s answering (taking {:.3} s, waiting for chunks built on workers {:.3} s, evaluating them on this thread {:.3} s, answer rows {:.3} s); {:.3} s waiting for the cone's next request",
                self.calls, self.rows, self.fresh, self.again, self.total, self.take, self.built, self.evaluate, self.rows_of, self.idle
            )
        };
        crate::code_rule_dsl::logic::profile::decision("requests answered inside the fixpoint (decision 0114 §6)", text);
        if std::env::var_os("RERAIL_STRATA_COUNTS").is_some() {
            eprintln!("RERAIL_ANSWERS {}", text());
        }
    }
}

/// Runs its closure when dropped: a measurement taken on every way out of a block.
struct Finally<F: FnMut()>(F);

impl<F: FnMut()> Drop for Finally<F> {
    fn drop(&mut self) {
        (self.0)();
    }
}

/// The cone's side of its requests (decision 0114 §3): each posted with a ticket to the thread that answers them,
/// and its answer taken back by that ticket, in place (`ask`) or as it comes (`Posting`). The fixpoint's answers
/// are recorded for a closing round (`recorded`).
struct Asks {
    tx: std::sync::mpsc::Sender<(u64, String, Vec<Vec<i64>>)>,
    rx: std::sync::mpsc::Receiver<(u64, std::result::Result<Vec<(String, Vec<Vec<i64>>)>, String>)>,
    next: u64,
    /// Answers come before they were taken.
    early: BTreeMap<u64, Vec<(String, Vec<Vec<i64>>)>>,
    /// Each ticket's request, until its answer comes.
    posted: HashMap<u64, (String, Vec<Vec<i64>>)>,
    recorded: Option<cone_rounds::Answers>,
}

impl Asks {
    fn post(&mut self, rel: &str, rows: &[Vec<i64>]) -> Result<u64> {
        let ticket = self.next;
        self.next += 1;
        self.tx.send((ticket, rel.to_owned(), rows.to_vec())).map_err(|_| Error::Plan("the request channel closed".to_owned()))?;
        self.posted.insert(ticket, (rel.to_owned(), rows.to_vec()));
        Ok(ticket)
    }

    /// An answer come: recorded, and kept until taken.
    fn came(&mut self, ticket: u64, reply: std::result::Result<Vec<(String, Vec<Vec<i64>>)>, String>) -> Result<()> {
        let given = reply.map_err(Error::Plan)?;
        if let (Some(rec), Some((rel, rows))) = (self.recorded.as_mut(), self.posted.remove(&ticket)) {
            rec.record(&rel, &rows, &given);
        }
        self.early.insert(ticket, given);
        Ok(())
    }

    /// What has come, waiting up to `wait` for the first answer when none has.
    fn poll(&mut self, wait: std::time::Duration) -> Result<()> {
        if self.early.is_empty() && !wait.is_zero() {
            match self.rx.recv_timeout(wait) {
                Ok((t, r)) => self.came(t, r)?,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Err(Error::Plan("the answer channel closed".to_owned())),
            }
        }
        loop {
            match self.rx.try_recv() {
                Ok((t, r)) => self.came(t, r)?,
                Err(std::sync::mpsc::TryRecvError::Empty) => return Ok(()),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return Err(Error::Plan("the answer channel closed".to_owned())),
            }
        }
    }

    /// A request answered in place: posted, then its own answer waited for (the others come are kept).
    fn ask(&mut self, rel: &str, rows: &[Vec<i64>]) -> Result<Vec<(String, Vec<Vec<i64>>)>> {
        let ticket = self.post(rel, rows)?;
        loop {
            if let Some(given) = self.early.remove(&ticket) {
                return Ok(given);
            }
            let rx = &self.rx;
            let (t, r) = rerail_accel::exec::help_until_as("the cone: an answer", || match rx.try_recv() {
                Ok(got) => Some(Ok(got)),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err(Error::Plan("the answer channel closed".to_owned()))),
            })?;
            self.came(t, r)?;
        }
    }
}

/// The cone's fixpoint's requests, posted and taken as they come (`plan::Asking`).
struct Posting<'r>(&'r std::cell::RefCell<Asks>);

impl super::Asking for Posting<'_> {
    fn post(&mut self, rel: &str, rows: &[Vec<i64>]) -> Result<u64> {
        self.0.borrow_mut().post(rel, rows)
    }
    fn arrived(&mut self, wait: std::time::Duration) -> Result<Vec<(u64, Vec<(String, Vec<Vec<i64>>)>)>> {
        let mut a = self.0.borrow_mut();
        a.poll(wait)?;
        Ok(std::mem::take(&mut a.early).into_iter().collect())
    }
}

/// The queued write of a root's stats: the next run of this process reads them once they are written.
fn stats_key(root: &str) -> crate::content_store::write_back::Key {
    crate::content_store::write_back::Key::of(&[b"corpus stats", root.as_bytes()])
}
