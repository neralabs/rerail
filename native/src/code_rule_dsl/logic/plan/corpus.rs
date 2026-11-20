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

use super::{Error, Outcome, Result, lower, output};

use crate::code_rule_dsl::facts::core::{self, Built};
use crate::code_rule_dsl::facts::vtab::{Modules, syntax};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::logic::front;
use crate::code_rule_dsl::logic::program_image::fields::{Edit, ProgramFields};
use crate::code_rule_dsl::logic::rr::WORLD;
pub mod added;
pub mod guests;
pub mod hiding;
pub mod exports;
pub mod combined;
pub mod scoping;
#[cfg(test)]
pub mod cache_architecture;
// Its tests run in the engine's test binary (its facade of this module mounts `corpus/cache_tests.rs`): they read what the
// engine registers as it loads (the languages, their `.rr` libraries).
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
    /// Adds the file with key `key` at `path`, with its source, facts and tape (the run's own, read in place: the file
    /// is not parsed again).
    fn add(&self, key: u32, path: &str, source: &Arc<String>, facts: &Arc<crate::code_rule_dsl::facts::core::Facts>, tape: &crate::code_rule_dsl::tape::TapeDoc) -> Result<()>;
    /// The relations the run's files give, with their columns and rows.
    fn relations(self: Box<Self>, store: &crate::content_store::ContentStore, read: &BTreeSet<String>) -> Result<Vec<(&'static str, Vec<String>, Vec<Option<Vec<i64>>>)>>;
}

/// What makes a program's cross-file relations over the files at `paths` of the dialect `label`, where it reads them.
pub(crate) type MakeCrossFile = fn(&str, &dyn ProgramFields, &BTreeSet<String>, &[String]) -> Result<Option<Box<dyn CrossFileRelations>>>;

static CROSS_FILE: std::sync::OnceLock<MakeCrossFile> = std::sync::OnceLock::new();

/// Registers what makes a program's cross-file relations (once, at load).
pub fn install_cross_file(make: MakeCrossFile) {
    let _ = CROSS_FILE.set(make);
}
pub mod prepared;
pub mod maintain;
pub mod retain;
pub mod keptstats;
pub mod input_delta;
pub mod phases;
pub mod constants;
pub mod inputs;
pub use inputs::parameters as input_parameters;
pub mod world;
pub mod texts;
mod link;
mod kept_link;
mod shape;
pub mod connections;
pub mod reuse;

/// The switches that change the combined program (`combine_tail`, with the `.rr` lowering its `rr::parse` runs): its
/// key (`texts_digest`, in memory and in the store) holds their values, so a combined program kept or stored under
/// one value is not another's. Every prepared key holds the combined program's.
pub const COMBINING: &[&str] = &[
    // `combine_tail`: the copies to compare (`=copied`) and decision 0091's measurement (`FactSpaces`).
    "RERAIL_LIBRARIES",
    "RERAIL_REQUESTS",
    "RERAIL_SCOPE_GUARDS",
    // The combine's tail kept or run whole (`=whole`), or checked against the whole (`=check`, decision 0393).
    "RERAIL_COMBINE_TAIL",
    // The `.rr` lowering: source attributes answered eagerly (`sources_on_request`), node ids compared raw
    // (`front::finish`'s `order_nodes`).
    "RERAIL_NODE_SOURCES",
    "RERAIL_ORDER_RAW",
    // Node ids by definition or in preorder (`syntax::definition_ids`, read by `order_nodes`).
    "RERAIL_NODE_IDS",
    // Identical relations merged (`combining::same_relations`, in combining and again in preparing).
    "RERAIL_SAME",
];

/// The engine switches read while a program is lowered and prepared (`run_observed`, before the prepared program is
/// kept): each changes the prepared program, so each is in its key. The run's other switches are not: the key also
/// keys a session's kept frames, which a diagnostic switch turned on mid-session must not make cold. A switch read in
/// lowering or preparation is listed here or as changing none, which a test checks
/// (`every_switch_lowering_reads_is_classified`).
pub const PREPARING: &[&str] = &[
    // Lowering and its passes.
    "RERAIL_TEXT_NODES",
    "RERAIL_SPAN_PREFILTER",
    "RERAIL_NODE_ANCHORS",
    "RERAIL_NEAREST_BUFFERS",
    "RERAIL_FACTORIZE",
    "RERAIL_EXPORT_DEMAND",
    "RERAIL_CROSS_MEMO",
    "RERAIL_REACH_INDEX",
    // `simplify`.
    "RERAIL_EXISTENCE_COUNTS",
    "RERAIL_INLINE_FILTERS",
    "RERAIL_JOIN_KEYS",
    "RERAIL_PROJECT_COLUMNS",
    // `run_observed`'s preparation, and its passes (`preparation`): by parts (decision 0345) or whole.
    "RERAIL_PREPARE",
    "RERAIL_TEXT_TESTS",
    "RERAIL_DEMAND",
    "RERAIL_PUSH_EQUAL_COLUMNS",
    "RERAIL_AST_PARTS",
    "RERAIL_AST_PART_CAPTURES",
    "RERAIL_RULE_ORDER",
    // Guests' copies kept unfolded (`combining::FoldOwn`).
    "RERAIL_SHOWN_COPIES",
];

pub mod phase0;
pub mod cross_memo;
pub mod switches;
pub mod matchers;
pub mod preparation;
pub(super) mod product_demand;
/// The prefix of an `ast` matcher's relation (decision 0106).
pub(in crate::code_rule_dsl) const AST_MATCHES: &str = matchers::PREFIX;
pub use matchers::columns as ast_columns;
pub mod typing;
// `base_tests` is the engine's (its facade of this module mounts it): it compiles rule documents through the engine.
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
use crate::code_rule_dsl::logic::program_image::passes::Parts as _;

pub fn usage() -> (f64, f64, f64) {
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
    /// Per phase of the run's thread, its clocks at the phase's bounds (decision 0288): wall, the thread's own CPU and
    /// helping, the process's CPU, its threads' time ready with no core, and its involuntary switches.
    pub phase_clock: Vec<(&'static str, crate::code_rule_dsl::logic::profile::PhaseClock)>,
    pub files: usize,
    pub nodes: usize,
    pub native: usize,
    pub rows: BTreeMap<String, usize>,
    pub chunks: usize,
    /// Declarations, summaries and terms (`corpus::typing`).
    pub typing: TypingReport,
    /// Text tests (decision 0037): relations, distinct strings tested, and the CPU testing them (seconds).
    pub text: (usize, usize, f64),
    /// The earlier phases this run read (decision 0083 §1), each its own run.
    pub phase_runs: Vec<phases::PhaseRun>,
    /// The plans this run and the runs nested in it used (its memo plan, its prepared programs, its phases', its
    /// constants' and phase 0's runs'): the outermost run's record names them all, so they stay live while a root's
    /// last runs name it (`keep_corpus_runs`). One process records a run per nested run, so recorded alone the
    /// phases' plans fell out of a root's last two runs after every process and were prepared again by the next.
    pub plans: Vec<[u8; 32]>,
    /// The per-file memo: files read from it, and chunks whose rows it could not keep (a row naming no document).
    pub memo: (usize, usize),
    /// Of the files the memo held, those read as partial hits (decision 0225 §2): their entries kept but the
    /// relations an edit changed, which their chunk derives again.
    pub memo_partial: usize,
    /// Of the files the memo held, those read for their outputs alone (decision 0117 step 1), and the values of
    /// their export and base rows decoded all the same: none, as the kept inputs hold those rows.
    pub memo_kept: (usize, usize),
    /// The statistics keeper's end (decision 0096 §3): the files it counted in and out this run, or why it gave them
    /// up (the next run counts every file in again). `None`: no keeper ran.
    pub stats_kept: Option<std::result::Result<(usize, usize), String>>,
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
    pub cone_kept: Option<(usize, usize)>,
    /// The files built in its waves whose tape was read before the first wave, and those parsed (Q22): a run over
    /// unchanged files parses none, whatever its programs.
    pub file_tapes: (usize, usize),
    /// The guests chunks of partial hits did not make, their cones unable to see them (`guests::guests_seen`).
    pub guests_unseen: usize,
    /// The rounds the cone ran from the last run's final round (decision 0145); 0 when it ran the gated fixpoint.
    pub cone_rounds: usize,
    /// The requests a language's answer answered this run (`language::Answerer`), each with its key's first column (a
    /// path), in the order asked.
    pub language_asked: Vec<(String, String)>,
    /// Of those, strata maintained from deltas, of them recursive, and strata derived again (decision 0077 §3).
    pub cross_maintained: (usize, usize, usize),
    /// Of those, the strata passed on as quiet, and the relations they held with no column measured (`Outcome::quiet`).
    pub cross_quiet: (usize, usize),
    /// The cross-file relations handed off as the run went, their frames made once no later stratum read them and
    /// their rows let go (Q74): a run not driven by deltas.
    pub cross_handed: usize,
    /// The cross-file inputs let go after their last reader, their frames written before the run (Q74).
    pub cross_unloaded: usize,
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
/// `graphs`: each program text's include graph's digest, as the run found it (`run_handed`).
pub(crate) fn unchanged(programs: &[&str], graphs: &[Option<[u8; 32]>], read: Option<&Arc<crate::code_rule_dsl::logic::program_image::Version>>, root: &Path, typing: &Typing, options: &serde_json::Value, consulted: &[(String, String, String)], summary: Option<Summary>) -> Result<bool> {
    Ok(changes_of(programs, graphs, read, root, typing, options, consulted, summary)?.is_some_and(|c| c.is_empty()))
}

/// The parts of a run's combined program that `changes` reads, and nothing else: whether it reads `declared_ret`,
/// its settings (its language, its artifacts), its scopes and text globs, its JSON policies (their scopes), and its
/// `env` facts (the world's roots). A function of the texts and the rule files they read: the whole-run cutoff's
/// record keeps it beside the run's (decision 0275), so its check does not combine the program again to read them.
#[derive(Clone, Debug, Default)]
pub(crate) struct Summary {
    declared_ret: bool,
    env: Vec<front::List<front::Value>>,
    settings: Vec<(String, String)>,
    policies: Vec<serde_json::Value>,
    doc_scopes: Option<Vec<(Vec<String>, Vec<String>)>>,
    text_globs: Vec<String>,
}

impl Summary {
    /// The summary of the program `programs` combine to, as `changes` combines them (the root's settings in each).
    pub(crate) fn of(programs: &[&str], graphs: &[Option<[u8; 32]>], read: Option<&Arc<crate::code_rule_dsl::logic::program_image::Version>>, root: &Path, options: &serde_json::Value) -> Result<Summary> {
        Ok(Self::from_fields(&*combined_with_settings(programs, graphs, read, root, options)?))
    }

    fn from_fields(p: &(impl ProgramFields + ?Sized)) -> Summary {
        Summary {
            declared_ret: p.declares("declared_ret"),
            env: p.fact_rows("env"),
            settings: p.settings(),
            policies: p.policies(),
            doc_scopes: p.doc_scopes(),
            text_globs: p.text_globs(),
        }
    }

    /// Its bytes: the parts as JSON.
    pub(crate) fn encode(&self) -> Option<Vec<u8>> {
        let env: Vec<Vec<serde_json::Value>> = self.env.iter().map(|row| row.iter().map(|v| match v {
            front::Value::Int(i) => serde_json::json!({ "i": i }),
            // An atom's value at run time is its integer (decision 0270); the settings' rows hold none.
            front::Value::Branded(b) => serde_json::json!({ "i": b.value() }),
            front::Value::Str(s) => serde_json::json!({ "s": s.as_str() }),
        }).collect()).collect();
        let parts = serde_json::json!({
            "declared_ret": self.declared_ret,
            "env": env,
            "settings": self.settings,
            "policies": self.policies,
            "doc_scopes": self.doc_scopes,
            "text_globs": self.text_globs,
        });
        serde_json::to_vec(&parts).ok()
    }

    pub(crate) fn decode(bytes: &[u8]) -> Option<Summary> {
        let v: serde_json::Value = serde_json::from_slice(bytes).ok()?;
        let mut env = Vec::new();
        for row in v["env"].as_array()? {
            let mut out = front::List::default();
            for c in row.as_array()? {
                out.push(match (c.get("i").and_then(serde_json::Value::as_i64), c.get("s").and_then(serde_json::Value::as_str)) {
                    (Some(i), _) => front::Value::Int(i),
                    (_, Some(s)) => front::Value::Str(front::Name::new(s)),
                    _ => return None,
                });
            }
            env.push(out);
        }
        Some(Summary {
            declared_ret: v["declared_ret"].as_bool()?,
            env,
            settings: serde_json::from_value(v["settings"].clone()).ok()?,
            policies: serde_json::from_value(v["policies"].clone()).ok()?,
            doc_scopes: serde_json::from_value(v["doc_scopes"].clone()).ok()?,
            text_globs: serde_json::from_value(v["text_globs"].clone()).ok()?,
        })
    }
}

/// The summary's fields as a program's, for the readers `changes` shares with the run (`scoped`, `World`, `inputs`).
impl ProgramFields for Summary {
    fn input_names(&self) -> Vec<String> {
        Vec::new()
    }
    fn output_names(&self) -> Vec<String> {
        Vec::new()
    }
    fn declares(&self, rel: &str) -> bool {
        rel == "declared_ret" && self.declared_ret
    }
    fn columns_of(&self, rel: &str) -> Option<Vec<(String, String)>> {
        self.declares(rel).then(Vec::new)
    }
    fn declared_names(&self) -> Vec<String> {
        if self.declared_ret { vec!["declared_ret".to_owned()] } else { Vec::new() }
    }
    fn fact_names(&self) -> Vec<String> {
        if self.env.is_empty() { Vec::new() } else { vec!["env".to_owned()] }
    }
    fn states_facts(&self, rel: &str) -> bool {
        rel == "env" && !self.env.is_empty()
    }
    fn fact_rows(&self, rel: &str) -> Vec<front::List<front::Value>> {
        if rel == "env" { self.env.clone() } else { Vec::new() }
    }
    fn about(&self, _: &[&str]) -> crate::code_rule_dsl::logic::program_facts::Facts {
        Default::default()
    }
    fn settings(&self) -> Vec<(String, String)> {
        self.settings.clone()
    }
    fn policies(&self) -> Vec<serde_json::Value> {
        self.policies.clone()
    }
    fn text_globs(&self) -> Vec<String> {
        self.text_globs.clone()
    }
    fn doc_scopes(&self) -> Option<Vec<(Vec<String>, Vec<String>)>> {
        self.doc_scopes.clone()
    }
}

/// The program `programs` combine to, as `changes` combines them: the root's settings (`<root>/rerail.env`, the `env`
/// option) as the rows the run adds (decision 0249), the texts as the run's keys read them, messages masked
/// (`texts_digest`, decision 0225 §3).
fn combined_with_settings(programs: &[&str], graphs: &[Option<[u8; 32]>], read: Option<&Arc<crate::code_rule_dsl::logic::program_image::Version>>, root: &Path, options: &serde_json::Value) -> Result<Arc<crate::code_rule_dsl::logic::program_image::Version>> {
    let flags: Vec<String> = options.get("env").and_then(|v| v.as_array()).into_iter().flatten().filter_map(|v| v.as_str().map(str::to_owned)).collect();
    let settings = crate::code_rule_dsl::logic::settings::settings(root, &flags).map_err(Error::Plan)?;
    let masked = masked(programs);
    Ok(combine_version(programs, &masked.iter().map(|t| t.as_ref()).collect::<Vec<_>>(), graphs, &added::Added::of(&settings, options), read, &front::Lists::default())?.0)
}

/// The workspace documents whose bytes changed since a run that consulted `consulted`, when nothing else it
/// consulted did: the same documents, by path, and every request's answer as it was (`World::still`). `None` when
/// anything else changed, or the run cannot be compared (its documents named, typing, phases read). `read`: the
/// program the run was handed (`run_read`), whose `programs` are its key, not a text to read.
pub(crate) fn changes(programs: &[&str], graphs: &[Option<[u8; 32]>], read: Option<&Arc<crate::code_rule_dsl::logic::program_image::Version>>, root: &Path, typing: &Typing, options: &serde_json::Value, consulted: &[(String, String, String)]) -> Result<Option<Vec<String>>> {
    changes_of(programs, graphs, read, root, typing, options, consulted, None)
}

/// `changes`, reading the program's parts off `summary` where one is given (the whole-run cutoff's record), else off
/// the program combined again.
#[allow(clippy::too_many_arguments)]
fn changes_of(programs: &[&str], graphs: &[Option<[u8; 32]>], read: Option<&Arc<crate::code_rule_dsl::logic::program_image::Version>>, root: &Path, typing: &Typing, options: &serde_json::Value, consulted: &[(String, String, String)], summary: Option<Summary>) -> Result<Option<Vec<String>>> {
    let trace = |why: String| if std::env::var_os("RERAIL_REUSE_TRACE").is_some() { eprintln!("rerail: changes: {why}"); };
    // A run given no file of its own (`inputFiles: []`, a phase's: phase 1 loads its documents by request) has no
    // listed documents to compare; what it loaded is among its requests, checked below. Other named files are not
    // compared.
    let none_named = options.get("inputFiles").and_then(|v| v.as_array()).is_some_and(Vec::is_empty);
    if (options.get("inputFiles").is_some() && !none_named) || consulted.is_empty() {
        trace(format!("not comparable: input files {}, {} consulted", options.get("inputFiles").is_some(), consulted.len()));
        return Ok(None);
    }
    let prog = match summary {
        Some(summary) => summary,
        None => Summary::from_fields(&*combined_with_settings(programs, graphs, read, root, options)?),
    };
    // A program reading earlier phases is comparable: its record names every phase's program and what each phase
    // consulted (`reuse::record`, O-cutoff-phases).
    if prog.declared_ret {
        trace("not comparable: declared_ret".to_owned());
        return Ok(None);
    }
    let language = prog.settings.iter().find(|(key, _)| key == "language").map(|(_, value)| value.trim().trim_matches('"').to_owned());
    let found = scoped(corpus_files(root)?, &prog, options);
    let label = corpus_language(language.as_deref(), typing, &found);
    let files = if none_named { Default::default() } else { found.of(label) };
    let docs: Vec<&(String, String, String)> = consulted.iter().filter(|c| c.0 == "doc").collect();
    // The workspace's documents are listed by their path from the root; a library document a phase loaded is named
    // by its absolute path, and is checked by its digest below, as every document is (O-cutoff-phases).
    let workspace: Vec<&&(String, String, String)> = docs.iter().filter(|d| !Path::new(&d.1).is_absolute()).collect();
    if files.len() != workspace.len() || files.iter().zip(&workspace).any(|(f, d)| *f != d.1) {
        trace(format!("the documents differ: {} files, {} kept", files.len(), workspace.len()));
        return Ok(None);
    }
    // A document listed without a digest was not read (none of the run's rules reads a document): its edit changes nothing.
    let changed: Vec<String> = docs.iter().filter(|d| !d.2.is_empty() && world::bytes_digest(&root.join(&d.1)) != d.2).map(|d| d.1.clone()).collect();
    const READERS: [&str; 6] = ["input", "input_dir", "artifact", "base", "git_rev", "git_index"];
    let (read, requests): (Vec<_>, Vec<_>) = consulted.iter().filter(|c| c.0 != "doc").cloned().partition(|c| READERS.contains(&c.0.as_str()));
    let mut world = world::World::new(root, &prog);
    let moved = world.changed(&requests);
    let (inputs_still, world_still) = (inputs::still(&prog, root, &read), moved.is_none());
    if !(inputs_still && world_still) {
        trace(format!("inputs as they were {inputs_still} ({} read), requests as they were {world_still} ({} requests; first changed {moved:?})", read.len(), requests.len()));
    } else if !changed.is_empty() {
        trace(format!("{} documents changed: {:?}", changed.len(), changed.iter().take(3).collect::<Vec<_>>()));
    }
    Ok((inputs_still && world_still).then_some(changed))
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
        let row: Vec<i64> = row.iter().map(|v| match v { front::Value::Int(i) => *i, front::Value::Branded(a) => a.value(), front::Value::Str(_) => atoms.next().expect("an atom per string") }).collect();
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

/// `Report::cone_inputs`: each document's rows of `per_file` (the exports held in memory, `exports`), by path. A value naming a node of the
/// document is its rank among those its rows name; the rest (symbols, document keys, numbers) as they are.
fn cone_digests<'a>(exports: &exports::Exports, per_file: &[String], locations: &BTreeMap<String, Vec<usize>>, program: &super::fields::Fields<'_>, docs: impl Iterator<Item = &'a (i64, String)>) -> Result<BTreeMap<String, String>> {
    use sha2::{Digest, Sha256};
    let mut by_doc: HashMap<i64, Vec<(usize, Vec<i64>)>> = HashMap::new();
    for (i, rel) in per_file.iter().enumerate() {
        let Some(at) = locations.get(rel).and_then(|l| l.first().copied()) else { continue };
        let width = program.arity(rel).unwrap_or(0);
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
/// The program is split at its file-local relations (`typing::file_local::programs_of`, list step 29). Documents are
/// taken in chunks of about `chunk_nodes` nodes, each built on a worker thread while the previous one runs (list
/// step 24): its file-local part is run, the exported relations are appended to `cross.sqlite` in `out` (schema `x`),
/// the base columns the cross-file part names are appended in memory, and the chunk is dropped. The cross-file part
/// runs once, over the base relations held in memory and the exports read once from `x`, all bound in `main`. Memory
/// is two chunks', the held base columns, and the cross-file part's own.
/// Repository readers publish typed rows once, before file-local partitioning.
fn external_inputs(prog: &mut Edit<'_>, shape: &shape::Shape, root: &Path) -> Result<()> {
    inputs::populate(prog, shape, root)?;
    // `core.rr` relations the engine does not make yet, read as empty, each named in the profile (open item O76).
    for (rel, what) in NOT_MADE {
        if prog.input_names().iter().any(|i| i == rel) {
            crate::code_rule_dsl::logic::profile::decision("relations not made yet (O76)", || format!("{rel} is empty: {what}"));
            prog.b.retain_inputs(|i| i != rel);
        }
    }
    Ok(())
}

/// A chunk's mix, for which statistics measured on one chunk stand for another, scaled by nodes: the file types
/// (extensions) holding a tenth of its source bytes or more, in order, and its files' size, as the power of 4 of its mean
/// bytes per file. Per-document relations (`doc`, `path_part`) grow with files, not nodes, so a chunk of many small
/// files is not a chunk of a few large ones scaled. Documents are `(path, source bytes)`, independent of the tape representation.
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

/// The key of a chunk's guest rounds' kept rows (`retain::Retained`): held for the chunk's rounds alone, never stored
/// (decision 0331).
const GUEST_ROUNDS_KEY: [u8; 32] = *b"rerail.guest-rounds/v1\0\0\0\0\0\0\0\0\0\0";

/// The embed slice of a lowered program (decisions 0055, 0058): the rules its requests need (`embed`, `embed_value`
/// and the demands), with those as its outputs; `None` when no rule makes a request. It is cut from the whole
/// program, not its file-local part, and runs over each chunk: a guest or a value is made from its chunk's
/// documents, so a demand whose context the split would run cross-file (`q(_, x)`, a value's nodes) is still asked
/// in the chunk.
fn inputs_of(v: &crate::code_rule_dsl::logic::program_image::Version) -> Vec<&str> {
    v.inputs().map(|a| v.text(a)).collect()
}

fn embed_slice(v: &Arc<crate::code_rule_dsl::logic::program_image::Version>) -> Result<Option<Arc<crate::code_rule_dsl::logic::program_image::Version>>> {
    use crate::code_rule_dsl::logic::program_image::RuleId;
    let derives = |r: &str| v.rel(r).is_some_and(|id| v.heads(id).next().is_some());
    let wanted: Vec<&str> = guests::LIBRARY.iter().copied().chain(guests::DEMANDS).filter(|r| derives(r)).collect();
    if wanted.is_empty() {
        return Ok(None);
    }
    // A request rule (an attribute's, `Program::asked_by`, decision 0174) is taken only once one of the relations
    // whose readers it serves is in the slice, as `front::prune` keeps it. A request relation is one for the whole
    // program (`rr_value_req`: every reader's request, never inlined since 0291), so the slice, following reads,
    // took every reader's request and what its body reads: 584 rules where 488 were (decision 0331).
    let asked = v.asked();
    // Each rule's gate, by its place among its relation's rules; a relation whose rules do not match the record
    // keeps them ungated (as `prune`'s does).
    let mut by_head: BTreeMap<&str, Vec<RuleId>> = BTreeMap::new();
    for r in v.rules() {
        by_head.entry(v.rel_name(r.head().rel())).or_default().push(r.id());
    }
    let mut gate: HashMap<RuleId, &BTreeSet<String>> = HashMap::new();
    for (req, readers) in asked.iter().filter(|(req, _)| !wanted.contains(&req.as_str())) {
        if let Some(ids) = by_head.get(req.as_str()).filter(|ids| ids.len() == readers.len()) {
            gate.extend(ids.iter().copied().zip(readers));
        }
    }
    let rule_of: HashMap<RuleId, crate::code_rule_dsl::logic::program_image::RuleRef<'_>> = v.rules().map(|r| (r.id(), r)).collect();
    let inputs = inputs_of(v);
    let about = crate::code_rule_dsl::logic::program_image::About::of(v);
    let has = |rel: &str| by_head.contains_key(rel) || inputs.contains(&rel) || v.rel(rel).is_some_and(|id| about.native(id).is_some());
    // `RERAIL_DUMP_SLICE=<file>`: each relation of the slice, the relation whose rule read it first, and the slice's
    // rules (`<file>.rules`), to compare one program's slice with another's (a debugging aid).
    let dump = std::env::var_os("RERAIL_DUMP_SLICE");
    let mut by: BTreeMap<String, String> = BTreeMap::new();
    let mut need: BTreeSet<String> = BTreeSet::new();
    let mut kept: BTreeSet<RuleId> = BTreeSet::new();
    let mut todo: Vec<String> = wanted.iter().map(|r| (*r).to_owned()).collect();
    loop {
        let take = |id: RuleId, kept: &mut BTreeSet<RuleId>, todo: &mut Vec<String>, by: &mut BTreeMap<String, String>| {
            kept.insert(id);
            let r = rule_of[&id];
            let head = v.rel_name(r.head().rel());
            for a in r.reads() {
                let rel = v.rel_name(a.rel());
                if dump.is_some() {
                    by.entry(rel.to_owned()).or_insert_with(|| head.to_owned());
                }
                todo.push(rel.to_owned());
            }
        };
        while let Some(rel) = todo.pop() {
            if !need.insert(rel.clone()) {
                continue;
            }
            for &id in by_head.get(rel.as_str()).into_iter().flatten() {
                if !gate.contains_key(&id) {
                    take(id, &mut kept, &mut todo, &mut by);
                }
            }
        }
        // The request rules of a needed relation one of whose readers is needed: their bodies are read, which may
        // need more readers.
        // A reader the version no longer has (merged, renamed or inlined since the requests were made) keeps them,
        // as `prune` does.
        let open: Vec<RuleId> = gate.iter().filter(|(id, readers)| !kept.contains(*id) && need.contains(v.rel_name(rule_of[*id].head().rel())) && readers.iter().any(|b| need.contains(b) || !has(b))).map(|(id, _)| *id).collect();
        for id in open {
            take(id, &mut kept, &mut todo, &mut by);
        }
        if todo.is_empty() {
            break;
        }
    }
    if let Some(path) = dump {
        let rules = |rel: &str| by_head.get(rel).map_or(0, |ids| ids.iter().filter(|id| kept.contains(id)).count());
        let text: String = need.iter().map(|rel| format!("{rel}\t{}\t{}\n", rules(rel), by.get(rel).map_or("(a root)", String::as_str))).collect();
        let _ = std::fs::write(&path, text);
        // Each request relation's record against its rules: gated only where they match.
        let gates: String = asked.iter().map(|(req, readers)| format!("{req}\trecord {}\trules {}\tgated {}\n", readers.len(), by_head.get(req.as_str()).map_or(0, Vec::len), by_head.get(req.as_str()).is_some_and(|ids| ids.iter().any(|id| gate.contains_key(id))))).collect();
        let mut at = path.clone();
        at.push(".gates");
        let _ = std::fs::write(at, gates);
        // Each reader the record names: whether the version still has it (a gone one keeps its requests) and whether
        // the slice needs it.
        let readers: BTreeSet<&String> = asked.values().flatten().flatten().collect();
        let text: String = readers.iter().map(|b| format!("{b}\thas {}\tneeded {}\n", has(b.as_str()), need.contains(b.as_str()))).collect();
        let mut at = path.clone();
        at.push(".readers");
        let _ = std::fs::write(at, text);
        let mut rules: Vec<String> = kept.iter().map(|id| format!("{}.\n", v.show_rule(rule_of[id]))).collect();
        rules.sort();
        let mut at = path.clone();
        at.push(".rules");
        let _ = std::fs::write(at, rules.concat());
    }
    // A part of the program's version (decision 0244, 0298's selection): the rules taken, the wanted relations its
    // outputs, every other field the program's.
    v.part_rules(|id| kept.contains(&id), &inputs, &wanted).map(Some).map_err(Error::Plan)
}

pub fn run(program: &str, root: &Path, t_facts: Option<&Path>, out: &Path, modes: &str, binding: Binding, chunk_nodes: usize) -> Result<Report> {
    run_typed(&[program], root, t_facts, out, modes, binding, chunk_nodes, &Typing::default())
}

/// Every program of `programs` in one run: each file is read, parsed and batched once, and every rule runs over each
/// chunk (`combine`).
pub fn run_all(programs: &[&str], root: &Path, out: &Path, modes: &str, binding: Binding, chunk_nodes: usize) -> Result<Report> {
    run_typed(programs, root, None, out, modes, binding, chunk_nodes, &Typing::default())
}

/// The programs `programs` (`.rr` text) as one program. One program is parsed as it is.
/// Of several, each one's own relations (the ones it derives or declares beyond its inputs) take a prefix, `p<i>_`,
/// so programs that name relations alike (a converter's `m650`, `exists_0`) do not meet. The base inputs and the
/// `violation` output are shared, and kind sets are named by their kinds (`ks<j>`), so programs asking for one set
/// share its tape pass. The simplifier then merges relations that are equal across programs (decision 0038).
pub fn combine(programs: &[&str]) -> Result<Arc<crate::code_rule_dsl::logic::program_image::Version>> {
    let graphs: Vec<Option<[u8; 32]>> = programs.iter().map(|t| crate::code_rule_dsl::logic::rr::graph_digest(t).ok().flatten()).collect();
    let masked = masked(programs);
    combine_version(programs, &masked.iter().map(|t| t.as_ref()).collect::<Vec<_>>(), &graphs, &added::Added::default(), None, &front::Lists::default()).map(|(program, _)| program)
}

/// The run's program texts, each read once (`rr::read_seed`, in `run_with_options`): their include graphs' digests,
/// and their library files and names, resolved when the whole-run cutoff first asks (`reuse::rule_files`). Held by
/// the run, and gone with it.
pub struct Texts<'a> {
    pub programs: &'a [&'a str],
    /// Each text's include graph's digest (`phase0::Graph::digest`), `None` for a text that includes nothing.
    pub graphs: Vec<Option<[u8; 32]>>,
    /// Phase 0's graphs for the texts that include something, one run of `bootstrap.rr` for all of them.
    pub derived: Vec<Arc<phase0::Graph>>,
    /// The program the run was handed, already read (`run_read`): the run holds it, and `programs` is its key.
    pub read: Option<&'a Arc<crate::code_rule_dsl::logic::program_image::Version>>,
    /// The source texts of the run inside which this one runs (phase 1's: the outer run's, decision 0300), which its
    /// builder borrows.
    pub sources: Option<&'a texts::Sources>,
    reads: std::cell::RefCell<Vec<crate::code_rule_dsl::logic::rr::TextRead>>,
    names: std::cell::OnceCell<Vec<crate::code_rule_dsl::logic::rr::TextNames>>,
}

impl<'a> Texts<'a> {
    /// A program already read, keyed by `key` (`read_key`): it includes nothing to derive a graph of, and its names
    /// are its own (`rr::TextNames::of_program`), as a print of it had them.
    pub fn of_read(key: &'a [&'a str], read: &'a Arc<crate::code_rule_dsl::logic::program_image::Version>) -> Self {
        let names = std::cell::OnceCell::new();
        let _ = names.set(vec![crate::code_rule_dsl::logic::rr::TextNames::of_program(&**read)]);
        Texts { programs: key, graphs: vec![None], derived: Vec::new(), read: Some(read), sources: None, reads: std::cell::RefCell::new(Vec::new()), names }
    }

    /// `programs`, each parsed once, and phase 0's graphs derived for those that include something.
    pub fn of(programs: &'a [&'a str]) -> Result<Self> {
        let reads: Vec<crate::code_rule_dsl::logic::rr::TextRead> = programs.iter().map(|t| crate::code_rule_dsl::logic::rr::read_seed(t)).collect::<std::result::Result<_, _>>()?;
        let seeds: Vec<phase0::Seed> = reads.iter().filter_map(|r| r.seed.clone()).collect();
        let derived = if phase0::active() { Vec::new() } else { phase0::graphs(&seeds)? };
        let mut at = derived.iter();
        let graphs = reads.iter().map(|r| r.seed.as_ref().and_then(|_| at.next()).map(|g| g.digest())).collect();
        Ok(Texts { programs, graphs, derived, read: None, sources: None, reads: std::cell::RefCell::new(reads), names: std::cell::OnceCell::new() })
    }

    /// Each text's library files, declared relations and phase (`rr::TextRead::resolved`).
    pub fn names(&self) -> Result<&[crate::code_rule_dsl::logic::rr::TextNames]> {
        if self.names.get().is_none() {
            let reads = std::mem::take(&mut *self.reads.borrow_mut());
            let names = reads.into_iter().map(crate::code_rule_dsl::logic::rr::TextRead::resolved).collect::<std::result::Result<Vec<_>, _>>()?;
            let _ = self.names.set(names);
        }
        Ok(self.names.get().expect("set above"))
    }
}

/// A combined program kept for the process (`COMBINED`): its version (`combine_version`'s).
#[derive(Default)]
struct Combined {
    version: Option<Arc<crate::code_rule_dsl::logic::program_image::Version>>,
}

/// The combined programs kept for the process by their texts' digest: a function of the texts alone (`rr::parse`
/// reads no file), so a session or a test that runs one program over many roots parses and hides it once.
fn combined_kept() -> &'static std::sync::Mutex<HashMap<[u8; 32], Combined>> {
    static COMBINED: std::sync::OnceLock<std::sync::Mutex<HashMap<[u8; 32], Combined>>> = std::sync::OnceLock::new();
    COMBINED.get_or_init(Default::default)
}

/// `edit` of the program `key` kept for the process (`combined_kept`). A few programs at a time are kept.
fn keep_combined(key: [u8; 32], edit: impl FnOnce(&mut Combined)) {
    const KEPT: usize = 32;
    let mut kept = combined_kept().lock().expect("the combined programs");
    if kept.len() >= KEPT && !kept.contains_key(&key) {
        kept.clear();
    }
    edit(kept.entry(key).or_default());
}

/// One program handed to its run (`run_read`), combined as `combine_version` combines it, past the process's own kept
/// programs: what the store gave or a miss made, what hiding it makes, and what the store then keeps under its texts'
/// digest. For the tests of decision 0275.
#[cfg(any(test, feature = "test-hooks"))]
pub fn combine_handed(read: &Arc<crate::code_rule_dsl::logic::program_image::Version>) -> Result<(Arc<crate::code_rule_dsl::logic::program_image::Version>, Arc<crate::code_rule_dsl::logic::program_image::Version>, Option<Arc<crate::code_rule_dsl::logic::program_image::Version>>)> {
    let text = crate::code_rule_dsl::logic::rr::print(&read.program(&front::Lists::default()).map_err(Error::Plan)?)?;
    let none = added::Added::default();
    let key = texts_digest(&[text.as_str()], &[None], &none);
    let made = combine_stored_version(&[text.as_str()], &none, &key, &front::Lists::default(), Some(read))?;
    let hidden = combine_tail(&[text.as_str()], &none, Some(read), None)?.0;
    let kept = phases::reading_store().and_then(|store| {
        store.settle(&combined::queued(&key));
        combined::lookup_version(&store, &key, &front::Lists::default()).ok().flatten()
    });
    Ok((made, hidden, kept))
}

/// `combine`, the texts' include graphs' digests given (`Texts::graphs`), each program given the rows `extra` adds
/// (`added::Added`), with the digest of its texts, messages masked (`masked`, decision 0225 §3), and those rows
/// (`texts_digest`) it is kept by: a run keys what it prepares by the same digest. `read` is the program the one text
/// prints from, already lowered (`Texts::read`): the program itself, not its text lowered again. The image is made
/// once per combined program and kept for the process (`COMBINED`).
fn combine_version(programs: &[&str], masked: &[&str], graphs: &[Option<[u8; 32]>], extra: &added::Added, read: Option<&Arc<crate::code_rule_dsl::logic::program_image::Version>>, lists: &front::Lists) -> Result<(Arc<crate::code_rule_dsl::logic::program_image::Version>, [u8; 32])> {
    let key = texts_digest(masked, graphs, extra);
    if let Some(v) = combined_kept().lock().expect("the combined programs").get(&key).and_then(|k| k.version.clone()) {
        crate::code_rule_dsl::prof::count("corpus.combined_reused", 1);
        return Ok((v, key));
    }
    // The one text a run was handed the program of: its hidden program kept under the texts' digest too, as a pack's
    // is (decision 0275): the lowered program is the key, and hiding it again is the step a hit saves. The rows the
    // run adds (decision 0249) are in the key.
    let one = read.filter(|_| programs.len() == 1);
    let v = combine_stored_version(programs, extra, &key, lists, one)?;
    keep_combined(key, |k| k.version = Some(Arc::clone(&v)));
    Ok((v, key))
}

/// The programs linked into one (`combine_linked_as`), and the combine's tail run on it (`tail_version`), with the kept
/// tail run it was made over or kept, if any (`tail_kept`).
fn combine_tail(programs: &[&str], extra: &added::Added, read: Option<&Arc<crate::code_rule_dsl::logic::program_image::Version>>, images_used: Option<&mut Vec<crate::content_store::HashKey>>) -> Result<(Arc<crate::code_rule_dsl::logic::program_image::Version>, Option<[u8; 32]>)> {
    let (v, tail) = combine_linked_as::<link::Versioned>(programs, extra, read, images_used)?;
    // `RERAIL_COMBINE_LINK=check`: the splice (decision 0373) against the programs linked whole.
    if combine_link_mode() == "check" {
        let (whole, whole_tail) = crate::code_rule_dsl::logic::profile::quiet(|| combine_linked_spliced::<link::Versioned>(programs, extra, read, None, false))?;
        assert_eq!((whole.size(), whole.digest()), (v.size(), v.digest()), "the combine's splice is its whole link: {}", crate::code_rule_dsl::logic::program_image::differences(&whole, &v));
        assert_eq!(whole_tail.shape(), tail.shape(), "the combine's splice has its whole link's tail");
        crate::code_rule_dsl::logic::profile::decision("the combine's last link (decision 0373)", || "checked: the splice is the whole link (RERAIL_COMBINE_LINK=check)".into());
    }
    // The parts linked as trees beside (decision 0281's path, step 7a-2): the same program.
    let (v, run) = match &tail {
        Tail::Many { kept: Some(kept), .. } if combine_tail_mode() != "whole" => {
            let kept = *kept;
            let (spliced, run) = tail_kept(v.clone(), &tail, kept)?;
            // `RERAIL_COMBINE_TAIL=check`: the tail kept (decision 0393) against the tail run whole.
            if combine_tail_mode() == "check" {
                let whole = crate::code_rule_dsl::logic::profile::quiet(|| tail_version(v, &tail))?;
                assert_eq!((whole.size(), whole.digest()), (spliced.size(), spliced.digest()), "the combine's kept tail is its whole tail: {}", crate::code_rule_dsl::logic::program_image::differences(&whole, &spliced));
                crate::code_rule_dsl::logic::profile::decision("the combine's tail (decision 0393)", || "checked: the kept tail is the whole tail (RERAIL_COMBINE_TAIL=check)".into());
            }
            (spliced, run)
        }
        _ => (tail_version(v, &tail)?, None),
    };
    Ok((v, run))
}

/// The combine kept in the persistent store across processes for a pack of programs (`combined`, decision 0094 §6):
/// keyed by the texts and checked against the library files the combine read. The store's entry opened as a version
/// (`combined::lookup_version`), or the combine's tail run on the version (`combine_tail`) and its image kept as the
/// entry (`combined::record_version`).
fn combine_stored_version(programs: &[&str], extra: &added::Added, key: &[u8; 32], lists: &front::Lists, read: Option<&Arc<crate::code_rule_dsl::logic::program_image::Version>>) -> Result<Arc<crate::code_rule_dsl::logic::program_image::Version>> {
    let store = if cfg!(feature = "tape-cache") && (programs.len() > 1 || read.is_some()) && std::env::var_os("RERAIL_DUMP_PROGRAMS").is_none() {
        phases::reading_store()
    } else {
        None
    };
    let Some(store) = store else { return combine_tail(programs, extra, read, None).map(|(v, _)| v) };
    let _reading = store.hold_collection().map_err(|e| Error::Plan(e.to_string()))?;
    let found = {
        let _lookup = crate::code_rule_dsl::logic::profile::span(|| "the combined program, from the store (decision 0094 §6)".into());
        store.settle(&combined::queued(key));
        combined::lookup_version(&store, key, lists).map_err(|e| Error::Plan(e.to_string()))?
    };
    if let Some(v) = found {
        crate::code_rule_dsl::prof::count("corpus.combined_stored", 1);
        crate::code_rule_dsl::logic::profile::decision("combined program (decision 0094 §6)", || "read from the store: the same texts, and every library file they read reads the same".into());
        return Ok(v);
    }
    let mut images = Vec::new();
    let (made, read) = crate::code_rule_dsl::logic::rr::recording(|| combine_tail(programs, extra, read, Some(&mut images)));
    let (v, run) = made?;
    {
        let _keep = crate::code_rule_dsl::logic::profile::span(|| "keep the combined program in the store, queued".into());
        // Its image, as data, written by the store's writer (decisions 0102, 0306), naming the tail run it was made
        // over or kept: a run that reads the entry names that run as the one that made it did (Q101).
        combined::record_version_ran(&store, "the combined program", key, &read, &v, &images, run);
    }
    crate::code_rule_dsl::logic::profile::decision("combined program (decision 0094 §6)", || format!("combined; kept in the store behind the run, with the {} library files it read", read.len()));
    Ok(v)
}

/// The version given `edit`'s edit of its fields (`fields::Edit`, decision 0276 §3): what the edit states, the rest
/// the version's. An edit that states nothing leaves the version as it is.
fn edit_fields(v: &mut Arc<crate::code_rule_dsl::logic::program_image::Version>, edit: impl FnOnce(&mut Edit<'_>) -> Result<()>) -> Result<()> {
    let next = {
        let mut e = Edit::on(v);
        edit(&mut e)?;
        e.finish().map_err(Error::Plan)?
    };
    *v = next;
    Ok(())
}

/// The digest of `combine`'s texts and the rows it adds: what a combined program is a function of. `graphs`: each
/// text's include graph's digest (`Texts::graphs`).
/// `programs` are the texts with their messages masked (`masked`): a finding's message by its placeholders alone, its
/// words the writer's, not the program's (decision 0225 §3), so an edit of them finds the program combined, prepared
/// and memoised.
fn texts_digest(programs: &[&str], graphs: &[Option<[u8; 32]>], extra: &added::Added) -> [u8; 32] {
    use sha2::Digest as _;
    let mut hash = sha2::Sha256::new();
    // Under another value of a switch `combine_tail` reads, the same texts combine to another program (`COMBINING`).
    // Only those set: an unset switch keeps the key every run has had.
    for switch in COMBINING {
        if let Some(v) = std::env::var_os(switch) {
            hash.update(format!("\0{switch}={}", v.to_string_lossy()).as_bytes());
        }
    }
    debug_assert_eq!(programs.len(), graphs.len(), "a graph digest per program text");
    for (text, graph) in programs.iter().copied().zip(graphs.iter().copied()) {
        hash.update((text.len() as u64).to_le_bytes());
        hash.update(text.as_bytes());
        // What the text includes, as phase 0 found it: its modules' paths and digests, and the graph's edges. A text
        // is a function of these, not of its `#include` lines alone.
        if let Some(graph) = graph {
            hash.update(graph);
        }
    }
    // The rows the run adds, none for most runs (`added`).
    if !extra.is_empty() {
        extra.hash(&mut hash);
    }
    hash.finalize().into()
}

/// A program prepared for a run (`run_with_options`): pruned, simplified, demanded, lowered, its joins pushed into the
/// chunks, located and split. A function of the combined program's texts, what the run added to it (facts, inputs,
/// outputs), the stage and the dialect; and of the store's atoms, so `interned` is every intern call lowering made,
/// asked again of the store on reuse.
#[derive(Clone)]
pub struct Prepared {
    /// The program before lowering, as the run's readers read it (decision 0281's path, step R): the image of the
    /// tree the preparation passes before lowering made, the last tree the run holds.
    pub source: Arc<crate::code_rule_dsl::logic::program_image::Version>,
    pub vocabulary: Arc<crate::code_rule_dsl::logic::strings::Vocabulary>,
    pub lowered: lower::Lowered,
    pub sets: Vec<(String, Vec<i64>)>,
    /// The `ast` atoms' matchers, by relation (`matchers::lower`, decision 0106).
    pub matchers: Vec<(String, String, Vec<String>)>,
    pub demanded: Vec<String>,
    pub pushed: Vec<String>,
    /// The exports handed on as parts (decision 0099, `file_local::FactorizeExports`), a line each.
    pub factorized: Vec<String>,
    pub located: std::result::Result<Vec<String>, Vec<String>>,
    pub merged: (usize, usize),
    pub split: crate::code_rule_dsl::typing::file_local::Programs,
    pub interned: Vec<(Vec<String>, Vec<u32>)>,
    /// The decisions preparing made (decision 0269), kept with the program: a run that reuses it states them.
    pub decided: Vec<crate::code_rule_dsl::logic::profile::Decided>,
    /// The sources its programs' texts are kept under, by the store that keeps them (its path): a run that takes it
    /// from memory names them in its manifest as one that read it from the store does.
    pub documents: Arc<parking_lot::Mutex<Vec<(std::path::PathBuf, Vec<crate::content_store::SourceId>)>>>,
    /// The cone of requests of `split`'s cross-file part (`cone_of`), made by the first run that loads documents
    /// and shared by the later ones: it is the split's, as the split is the program's.
    pub(in crate::code_rule_dsl) cone: std::sync::OnceLock<std::result::Result<Arc<Cone>, String>>,
    /// The cross-file part as a session maintains it (`maintained_cross`), made by the first run that keeps it, for
    /// the part it was made of (by digest) and the process's settings of it: a later run of the same part shares it.
    pub(in crate::code_rule_dsl) maintained: std::sync::OnceLock<(MaintainedOf, lower::Lowered)>,
    /// The run's file-local and cross-file parts, and its constants' program (`run_constants`), made by the first run
    /// for the constants it found (`once`): a later run finding the same ones shares them.
    pub(in crate::code_rule_dsl) parts: std::sync::OnceLock<(BTreeSet<String>, RunParts)>,
    /// The per-file memo's strata of the file-local part (`memo::strata`), with the digest of the part they are of
    /// (`memo::program_digest`): made by the first run, or read with the prepared program from the store (decision
    /// 0246 §4); a run whose part is that one adds its options, phases' rows and projection to them.
    pub(in crate::code_rule_dsl) memo_strata: std::sync::OnceLock<([u8; 32], memo::Strata)>,
}

impl Prepared {
    /// The sources its texts are kept under in the store at `path`, when known (`documents`).
    fn documents_in(&self, path: &std::path::Path) -> Option<Vec<crate::content_store::SourceId>> {
        self.documents.lock().iter().find(|(p, _)| p == path).map(|(_, d)| d.clone())
    }

    fn keep_documents(&self, path: &std::path::Path, documents: &[crate::content_store::SourceId]) {
        let mut all = self.documents.lock();
        all.retain(|(p, _)| p != path);
        all.push((path.to_path_buf(), documents.to_vec()));
    }
}

/// A run's parts made from the prepared split (`Prepared::parts`): file-local, cross-file, and the constants'.
type RunParts = (lower::Lowered, lower::Lowered, Option<(lower::Lowered, Vec<String>)>);

/// What `Prepared::maintained` is made of: the cross-file part's digest and `maintained_cross`'s settings.
type MaintainedOf = (u64, [bool; 2]);

/// The cross-file part's requests answered inside the fixpoint (decision 0065 §2a, `cone_of`): the cone's program
/// (`load`) and the relations it reads, the rest of the part, and each one's version (decision 0244).
pub(in crate::code_rule_dsl) struct Cone {
    load: Arc<crate::code_rule_dsl::logic::program_image::Version>,
    reads: BTreeSet<String>,
    rest: Arc<crate::code_rule_dsl::logic::program_image::Version>,
}

/// A measurement (dev only, `RERAIL_SPLIT_DUMP=<file>`): each cross-file relation of the split, by how its rules read
/// other files, one line each: `relation  own  inherited  output|-  rule`. A rule's own class is the first that holds:
/// `recursive` (its stratum reads itself), `total` (an aggregate with no group variable), `grouped` (an aggregate by
/// a key that is not one document), `negation` (a negated atom on another or any document), `lookup` (atoms on two
/// or more documents, joined by values), `same-unit` (every atom on one document), `config` (no document at all).
/// A relation inherits the worst class of the relations its rules read.
fn dump_split(path: &std::path::Path, split: &crate::code_rule_dsl::typing::file_local::Programs) {
    use crate::code_rule_dsl::logic::program_image::{CmpOp, Lit, LitRef, RuleRef, Slot, TermRef, Version};
    use std::io::Write;
    // The two parts' rules beside it, a line each (printed from their versions, 0276 §6), and the exports.
    let rules = |v: &Version| v.rules().map(|r| format!("{}\n", v.show_rule(r))).collect::<String>();
    // One set of files a program: a batch (`run-policies`) splits several, each to the next free number.
    let stem = path.file_stem().map_or_else(|| "split".into(), |s| s.to_string_lossy().into_owned());
    let n = (0..).find(|n| !path.with_file_name(format!("{stem}-{n}.cross.rr")).exists()).unwrap_or(0);
    let numbered = path.with_file_name(format!("{stem}-{n}.txt"));
    let path = numbered.as_path();
    let _ = std::fs::write(path.with_extension("local.rr"), rules(&split.versions.local));
    let _ = std::fs::write(path.with_extension("cross.rr"), rules(&split.versions.cross));
    let _ = std::fs::write(path.with_extension("exports.txt"), split.exports.join("\n"));
    const ORDER: [&str; 10] = ["config", "same-unit", "union", "recursive-local", "lookup", "exists", "grouped", "total", "recursive", "-"];
    let rank = |c: &str| ORDER.iter().position(|o| *o == c).unwrap_or(0);
    let cross = &split.versions.cross;
    let placed = super::body::Placed::whole(Arc::clone(cross));
    let (Ok(f), Ok(sccs)) = (placed.fields(), placed.components()) else { return };
    let scc_of = |rel: &str| sccs.iter().find(|s| s.contains(rel));
    let head = |r: RuleRef<'_>| cross.rel_name(r.head().rel());
    let derived: BTreeSet<String> = cross.rules().map(|r| head(r).to_owned()).collect();
    // The columns of a relation that name a document or a node of one (a node id holds its document).
    let carriers = |rel: &str| -> Vec<usize> {
        let mut c: BTreeSet<usize> = split.same_doc.get(rel).or_else(|| split.locations.get(rel)).cloned().unwrap_or_default().into_iter().collect();
        c.extend(f.types(rel).iter().enumerate().filter(|(_, t)| **t == "node").map(|(i, _)| i));
        c.into_iter().collect()
    };
    // The variables of a term, of a literal.
    fn term_vars(t: TermRef<'_>, out: &mut BTreeSet<Slot>) {
        match t {
            TermRef::Var(s) => {
                out.insert(s);
            }
            TermRef::Call(_, args) => args.for_each(|a| term_vars(a, out)),
            _ => {}
        }
    }
    let lit_vars = |l: LitRef<'_>| -> BTreeSet<Slot> {
        let mut out = BTreeSet::new();
        match l.view() {
            Lit::Atom(a) => a.args().for_each(|t| term_vars(t, &mut out)),
            Lit::Cmp { l, r, .. } => {
                term_vars(l, &mut out);
                term_vars(r, &mut out);
            }
            Lit::Agg { var, over, atom, .. } => {
                out.insert(var);
                out.extend(over);
                atom.args().for_each(|t| term_vars(t, &mut out));
            }
            Lit::Nearest { n, a, .. } => out.extend([n, a]),
            Lit::Ast { document, node, .. } => out.extend([document, node]),
        }
        out
    };
    // Whether a relation's carriers are always on one document: known for base and file-local relations (their
    // same-document columns), and for derived ones the greatest fixpoint of their rules keeping them so.
    let one_doc_known = |rel: &str| !derived.contains(rel) && split.same_doc.contains_key(rel);
    // The classes of a rule's carrier variables, joined where an atom on one document holds both; `None` when some
    // literal is on any document (a wildcard or constant in every carrier).
    let classes = |rule: RuleRef<'_>, unit: &BTreeSet<String>| -> Option<usize> {
        let mut parent: HashMap<Slot, Slot> = HashMap::new();
        fn find(p: &mut HashMap<Slot, Slot>, x: Slot) -> Slot {
            let up = *p.entry(x).or_insert(x);
            if up == x {
                return up;
            }
            let r = find(p, up);
            p.insert(x, r);
            r
        }
        let mut seen: BTreeSet<Slot> = BTreeSet::new();
        for a in rule.reads() {
            let rel = cross.rel_name(a.rel());
            let args: Vec<TermRef<'_>> = a.args().collect();
            let vars: Vec<Slot> = carriers(rel).into_iter().filter_map(|c| args.get(c).and_then(|t| t.var())).collect();
            if vars.is_empty() {
                if !carriers(rel).is_empty() {
                    return None;
                }
                continue;
            }
            seen.extend(vars.iter().copied());
            if one_doc_known(rel) || unit.contains(rel) {
                let r0 = find(&mut parent, vars[0]);
                for v in &vars[1..] {
                    let r = find(&mut parent, *v);
                    parent.insert(r, r0);
                }
            }
        }
        // `$a = $b` names one value twice.
        for l in rule.body() {
            if let Lit::Cmp { l: TermRef::Var(a), op: CmpOp::Eq, r: TermRef::Var(b) } = l.view() {
                let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
                parent.insert(ra, rb);
            }
        }
        let roots: BTreeSet<Slot> = seen.iter().map(|v| find(&mut parent, *v)).collect();
        Some(roots.len())
    };
    let mut unit: BTreeSet<String> = derived.clone();
    loop {
        let before = unit.len();
        let gone: Vec<String> = cross.rules().filter(|r| unit.contains(head(*r)) && classes(*r, &unit).is_none_or(|n| n > 1)).map(|r| head(r).to_owned()).collect();
        for g in gone {
            unit.remove(&g);
        }
        if unit.len() == before {
            break;
        }
    }
    let own = |rule: RuleRef<'_>| -> &'static str {
        let scc = scc_of(head(rule));
        let recursive = rule.reads().any(|a| scc.is_some_and(|s| s.contains(cross.rel_name(a.rel()))));
        let n = classes(rule, &unit);
        let one = n.is_some_and(|n| n <= 1);
        let lits: Vec<LitRef<'_>> = rule.body().collect();
        let others = |k: usize| -> BTreeSet<Slot> {
            let mut v = BTreeSet::new();
            rule.head().args().for_each(|t| term_vars(t, &mut v));
            for (j, l) in lits.iter().enumerate() {
                if j != k {
                    v.extend(lit_vars(*l));
                }
            }
            v
        };
        // One positive atom with its documents projected away: a union of each document's rows.
        let union = match lits.as_slice() {
            [l] => match l.view() {
                Lit::Atom(a) => {
                    let c = carriers(cross.rel_name(a.rel()));
                    let args: Vec<TermRef<'_>> = a.args().collect();
                    !a.neg() && !c.is_empty() && c.iter().all(|&c| matches!(args.get(c), Some(TermRef::Wild)))
                }
                _ => false,
            },
            _ => false,
        };
        let mut worst = if n == Some(0) { "config" } else if one { "same-unit" } else if union { "union" } else { "lookup" };
        let bump = |c: &'static str, worst: &mut &'static str| {
            if rank(c) > rank(*worst) {
                *worst = c;
            }
        };
        for (k, l) in lits.iter().enumerate() {
            match l.view() {
                Lit::Agg { var, over, atom, .. } if !one => {
                    let mut g = BTreeSet::new();
                    atom.args().for_each(|t| term_vars(t, &mut g));
                    g.remove(&var);
                    if let Some(o) = over {
                        g.remove(&o);
                    }
                    bump(if g.is_empty() { "total" } else { "grouped" }, &mut worst);
                }
                Lit::Atom(a) if !carriers(cross.rel_name(a.rel())).is_empty() && lit_vars(*l).is_disjoint(&others(k)) => bump("exists", &mut worst),
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
    for scc in sccs.iter() {
        let mut own_worst = "config";
        let mut inherited = "config";
        let mut looped = false;
        for rule in cross.rules().filter(|r| scc.contains(head(*r))) {
            let o = own(rule);
            looped |= o.starts_with("recursive");
            let o = if o == "recursive-local" { "same-unit" } else { o };
            if rank(o) > rank(own_worst) {
                own_worst = o;
            }
            for a in rule.reads() {
                if let Some((_, i)) = class.get(cross.rel_name(a.rel()))
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
    for rule in cross.rules() {
        let rel = head(rule);
        let (o, i) = class.get(rel).copied().unwrap_or(("config", "config"));
        out.push_str(&format!("{rel}\t{o}\t{i}\t{}\t{}\n", if f.is_output(rel) { "output" } else { "-" }, cross.show_rule(rule).replace('\n', " ")));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = f.write_all(out.as_bytes());
    }
}

/// The prepared programs kept in the process, by program key, given to `f`: a few of each, one per store whose atoms
/// it holds (`interned`), so a process serving two stores (a session's and another run's) does not prepare again on
/// each switch. A test's are its own thread's: tests run side by side, and one filling the cache clears it
/// (`PREPARED_KEPT`) under another that expects its program kept (`prepared::tests`).
pub fn with_prepared<R>(f: impl FnOnce(&mut HashMap<[u8; 32], Vec<Arc<Prepared>>>) -> R) -> R {
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

/// `combine` of one program already read (`rr::read_file`): the program itself, as a run holds it, never printed and
/// read again (decision 0249).
pub fn combine_read_program(read: &Arc<crate::code_rule_dsl::logic::program_image::Version>) -> Result<Arc<crate::code_rule_dsl::logic::program_image::Version>> {
    combine_tail(&[""], &added::Added::default(), Some(read), None).map(|(v, _)| v)
}

/// The programs `programs` (`.rr` text) combined, past the process's and the store's kept programs.
pub fn combine_texts(programs: &[&str]) -> Result<Arc<crate::code_rule_dsl::logic::program_image::Version>> {
    combine_tail(programs, &added::Added::default(), None, None).map(|(v, _)| v)
}

/// What the combine does to the linked program, on the whole (decision 0281's path, step 7a-1): one program's
/// relations that are one merged; several programs' projected inputs, helpers retyped, attributes on request, the
/// bound once where the libraries are held once, the fact spaces, and the relations that are one merged.
#[derive(Debug)]
enum Tail {
    One,
    /// `kept`: the kept link's family (decision 0373), where the link is kept: the tail's run is kept under it too
    /// (decision 0393).
    Many { hide: Option<u32>, policies: Vec<scoping::Policy>, kept: Option<TailKept> },
}

impl Tail {
    /// What the tail does, without where its run is kept: two tails of one shape make one program.
    fn shape(&self) -> String {
        match self {
            Tail::One => "one".to_owned(),
            Tail::Many { hide, policies, .. } => format!("{hide:?} {policies:?}"),
        }
    }
}

/// The family the combine's tail run is kept under (decision 0393): the kept link's (decision 0373).
#[derive(Clone, Copy)]
struct TailKept([u8; 32]);

impl std::fmt::Debug for TailKept {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("kept")
    }
}

/// The fact spaces' decision (decision 0091).
fn fact_spaces_decision(report: &scoping::Report) {
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
}

/// What the combine does to the linked program, on the version. Of one program, the requests asked where the
/// documents are, and the relations that are one merged. Of several, first `kind` and `docof` no inputs where a part
/// projects them, then each step below.
fn tail_version(v: Arc<crate::code_rule_dsl::logic::program_image::Version>, tail: &Tail) -> Result<Arc<crate::code_rule_dsl::logic::program_image::Version>> {
    let p = tail_passes(crate::code_rule_dsl::logic::program_image::manager::Pipeline::new("combine", v), tail)?;
    Ok(p.finish().map_err(Error::Plan)?.0)
}

/// `tail_version` as a run kept under the kept link's family (decision 0393): the last run's opened from the image
/// store, and where the link is an edit of its input (`manager::rebase`), the passes run again over the edit
/// (`Pipeline::again`): each step's analyses carried across it, and a pass that can (`ProgramPass::again`) writing
/// the edit onto its last output. The run is kept again, behind the run.
fn tail_kept(v: Arc<crate::code_rule_dsl::logic::program_image::Version>, tail: &Tail, kept: TailKept) -> Result<(Arc<crate::code_rule_dsl::logic::program_image::Version>, Option<[u8; 32]>)> {
    use crate::code_rule_dsl::logic::program_image::manager::{Delta, Pipeline, Runs as _, rebase};
    let Some(store) = crate::code_rule_dsl::logic::rr::image_store() else { return Ok((tail_version(v, tail)?, None)) };
    let key = {
        use sha2::Digest as _;
        sha2::Sha256::new()
            .chain_update(b"rerail/combine tail run v2\0")
            .chain_update(kept.0)
            .chain_update(crate::code_rule_dsl::build_ids::engine().as_bytes())
            .chain_update(tail.shape().as_bytes())
            .finalize()
            .into()
    };
    let runs = crate::code_rule_dsl::logic::program_text::StoredRuns(&store);
    // The run the key names (decision 0385): kept under its own input's identity (`anchor`), the key a replaceable
    // pointer to it, so a run kept later replaces it where a run's own facts are kept once.
    let anchor = |run: &crate::code_rule_dsl::logic::program_image::manager::Run| -> [u8; 32] {
        use sha2::Digest as _;
        sha2::Sha256::new().chain_update(b"rerail/combine tail run kept\0").chain_update(key).chain_update(crate::code_rule_dsl::logic::program_image::manager::identity(run.input())).finalize().into()
    };
    let last = {
        let _s = crate::code_rule_dsl::logic::profile::span(|| "the combine's last tail run, from the store (decision 0393)".into());
        let named = preparation::parts::kept_facts_alone(&store, &key).and_then(|f| f.get("anchor").and_then(|r| r.first()).and_then(|r| match r.first() {
            Some(front::Value::Str(h)) => preparation::parts::unhex(h),
            _ => None,
        }));
        // Its versions a few edits deep made flat (decision 0385): the edit runs over an edit, not whole.
        named.and_then(|a| runs.open(&a, "combine").ok().flatten().map(|r| (a, crate::code_rule_dsl::logic::program_image::manager::Run::shallow(r))))
    };
    let rebased = {
        let _s = crate::code_rule_dsl::logic::profile::span(|| "the link, as an edit of the last tail run's input (decision 0393)".into());
        last.and_then(|(a, last)| rebase(last.input(), &v).ok().flatten().map(|r| (a, last, r)))
    };
    let (p, edit, over) = match rebased {
        Some((a, last, r)) => {
            let n = Delta::of(last.input(), &r).size();
            crate::code_rule_dsl::logic::profile::decision("the combine's tail (decision 0393)", || format!("run again over an edit of the last run's input ({n} rules)"));
            (Pipeline::again(last, r), Some(n), Some(a))
        }
        None => {
            crate::code_rule_dsl::logic::profile::decision("the combine's tail (decision 0393)", || "run whole: no last run of the link's family, or the link is no edit of its input".into());
            (Pipeline::keeping("combine", v), None, None)
        }
    };
    let run = tail_passes(p, tail)?.done().map_err(Error::Plan)?;
    for s in run.stages() {
        crate::code_rule_dsl::logic::profile::decision("the combine's tail: a pass (decision 0393)", || format!("{}: {:?}, {:?}, {} rules visited", s.pass, s.ran, s.form, s.visited));
    }
    // Kept where it pays (decision 0385): a run made whole, or one whose edit from the kept run's input reached
    // `TAIL_REKEPT` rules. Otherwise the kept run stands, and the next edit is an edit of its input as this one was:
    // nothing is written.
    // The run this one was made over (it stands) or the one kept here: the combined program's entry names it.
    let named = match edit {
        Some(n) if n < TAIL_REKEPT => {
            crate::code_rule_dsl::logic::profile::decision("the combine's tail run, kept (decision 0385)", || format!("no: the kept run stands, the edit from its input {n} rules (kept again from {TAIL_REKEPT})"));
            over
        }
        _ => {
            let _s = crate::code_rule_dsl::logic::profile::span(|| "keep the combine's tail run (decision 0393)".into());
            let at = anchor(&run);
            crate::code_rule_dsl::logic::program_text::record_runs(&store, &[(at, &run)]);
            let pointer: crate::code_rule_dsl::logic::program_facts::Facts = std::iter::once(("anchor".to_owned(), vec![vec![preparation::parts::str(&preparation::parts::hex(&at))].into()])).collect();
            preparation::parts::keep_facts(&store, &key, &pointer);
            crate::code_rule_dsl::logic::profile::decision("the combine's tail run, kept (decision 0385)", || match edit {
                Some(n) => format!("yes: the edit from the kept run's input reached {n} rules"),
                None => "yes: a run made whole".into(),
            });
            Some(at)
        }
    };
    let out = run.output().clone();
    let out = match out.base_bytes() {
        Some(_) => out,
        None => out.fold().map_err(Error::Plan)?,
    };
    Ok((out, named))
}

/// How many rules an edit of the kept tail run's input reaches before the run over it is kept in its place
/// (decision 0385): below it, each edit runs again from the kept run, and keeping costs nothing.
const TAIL_REKEPT: usize = 64;

/// The tail's passes on `p` (`tail_version`), their decisions stated.
fn tail_passes<'s>(mut p: crate::code_rule_dsl::logic::program_image::manager::Pipeline<'s>, tail: &Tail) -> Result<crate::code_rule_dsl::logic::program_image::manager::Pipeline<'s>> {
    use preparation::{combining, hide, on_request, whole};
    let mut spaces_report = None;
    if let Tail::Many { hide: bound, policies, .. } = tail {
        // core.rr's `kind(n, k)` and `docof(n, d)` are projections of `node` in a part that reads them (`rr::lower`'s
        // `projections`) and inputs in one that does not: once one part projects them, they are no inputs of the
        // combined program. Both would give `kind` two domains.
        p.program(&mut whole::Projected);
        // The helpers' columns, typed by the library's declarations, which each program was lowered without.
        p.program(&mut whole::RetypeHelpers);
        // The attributes on request over the combined program's readers (decision 0174; undone in each part).
        for round in crate::code_rule_dsl::logic::rr::lower::ON_REQUEST {
            p.program(&mut on_request::SourcesOnRequest { round });
        }
        // A request the cross-file part would derive asks within the node's document instead (O-cross-file-answers,
        // decision 0286); an answer still cross-file after that is made eager (0174 §8b).
        p = requests_where_documents_are(p)?;
        // The combined program bounded once to the documents it shows (decision 0058 §1, 0089 §4): a library's rules
        // are the same for every program, so one analysis gives what each program's would.
        if let Some(bound) = *bound {
            let rules = p.current().map_err(Error::Plan)?.size();
            let _hiding = crate::code_rule_dsl::logic::profile::span(|| format!("bound the combined program to the documents it shows ({rules} rules; decision 0058)"));
            p.program(&mut hide::Hide::new(bound));
        }
        // The library's relations once for the run, each policy's rules over its own scope's documents (decision 0091).
        let spacing = crate::code_rule_dsl::logic::profile::span(|| "fact spaces".into());
        let mut spaces = combining::FactSpaces::new(policies);
        p.program(&mut spaces);
        drop(spacing);
        spaces_report = Some(spaces.report);
    }
    if matches!(tail, Tail::One) {
        p = requests_where_documents_are(p)?;
    }
    p.current().map_err(Error::Plan)?;
    if let Some(report) = &spaces_report {
        fact_spaces_decision(report);
    }
    combining::same_relations(&mut p).map_err(Error::Plan)?;
    for d in p.decisions() {
        crate::code_rule_dsl::logic::profile::decision(&d.what, || d.detail.clone());
    }
    Ok(p)
}

/// A text's version with the library's cone it asks for (decision 0089 §3), as `rr::Text::lowered_in` makes it on
/// the tree (decision 0281's path, step 7b): its answers on request undone (`link::undone_on_request`), the cone
/// copied from the library's version (`rr::with_cone_version`), and the attributes on request made again over both
/// (`on_request::SourcesOnRequest`, decision 0174), its cone's readers asking too. No tree is decoded.
pub(crate) fn cone_on_version(
    own: &Arc<crate::code_rule_dsl::logic::program_image::Version>,
    library: &Arc<crate::code_rule_dsl::logic::program_image::Version>,
    asked: &BTreeSet<String>,
    declared: &BTreeSet<String>,
) -> std::result::Result<Arc<crate::code_rule_dsl::logic::program_image::Version>, String> {
    let undone = link::undone_on_request(own).map_err(|e| e.to_string())?;
    let coned = crate::code_rule_dsl::logic::rr::with_cone_version(&undone, library, asked, declared)?;
    let mut p = crate::code_rule_dsl::logic::program_image::manager::Pipeline::new("a text's cone", coned);
    for round in crate::code_rule_dsl::logic::rr::lower::ON_REQUEST {
        p.program(&mut preparation::on_request::SourcesOnRequest { round });
    }
    p.current()?;
    for d in p.decisions() {
        crate::code_rule_dsl::logic::profile::decision(&d.what, || d.detail.clone());
    }
    Ok(p.finish()?.0)
}

/// The requests `sources_on_request` made, narrowed where the split finds them cross-file to the asked node's own
/// document (`file_local::RequestsFileLocal`): a node's attribute is its document's, answered in its chunk while the
/// chunk's batch holds the text (O-cross-file-answers). Then the answers still cross-file made eager
/// (`answers_where_documents_are_version`). A program that answers no attribute on request is left as it is. Each a pass of `p`: a kept tail keeps them and the steps
/// after them (decision 0393).
fn requests_where_documents_are<'s>(mut p: crate::code_rule_dsl::logic::program_image::manager::Pipeline<'s>) -> Result<crate::code_rule_dsl::logic::program_image::manager::Pipeline<'s>> {
    use crate::code_rule_dsl::logic::rr::lower::ON_REQUEST;
    let v = std::sync::Arc::clone(p.current().map_err(Error::Plan)?);
    let asks = ON_REQUEST.iter().flat_map(|round| round.iter()).any(|a| v.rel(a.req).is_some());
    if !asks {
        return Ok(p);
    }
    if !std::env::var("RERAIL_NODE_SOURCES").is_ok_and(|v| v == "eager") {
        let requests: Vec<&'static str> = ON_REQUEST.iter().flat_map(|round| round.iter()).map(|a| a.req).filter(|r| v.rel(r).is_some()).collect();
        let mut pass = crate::code_rule_dsl::typing::file_local::RequestsFileLocal { requests, narrowed: Vec::new() };
        p.program(&mut pass);
        p.current().map_err(Error::Plan)?;
    }
    // In the same pipeline (decision 0393): a kept tail keeps the steps after this one too.
    p.program(&mut AnswersWhereDocumentsAre);
    Ok(p)
}

/// `answers_where_documents_are_version` as a program pass: the rounds it undoes, written as one edit of its input
/// (their headers and the heads they drop), and nothing where it undoes none.
struct AnswersWhereDocumentsAre;

impl crate::code_rule_dsl::logic::program_image::manager::ProgramPass for AnswersWhereDocumentsAre {
    const NAME: &'static str = "answers where documents are";
    /// Over an edit (decision 0385): the edit as it is, where the last run undid no round, the edit leaves the header
    /// as it was, and it defines no answer a round is undone for, nor what one reads, down.
    fn again(&mut self, cx: &crate::code_rule_dsl::logic::program_image::manager::Cx<'_>, last: &crate::code_rule_dsl::logic::program_image::manager::Last<'_>, w: &mut crate::code_rule_dsl::logic::program_image::manager::ProgramW<'_>) -> std::result::Result<bool, String> {
        use crate::code_rule_dsl::logic::program_image::manager::{Reads, edit_as_is};
        use crate::code_rule_dsl::logic::rr::lower::{Answer, ON_REQUEST};
        if last.delta.touched != Reads::RULES || !Arc::ptr_eq(last.input, last.output) {
            return Ok(false);
        }
        // The split places a relation from its rules and what they read, down: where the edit defines none of the
        // answers' relations nor what they read, it places them as it did, file-local, and is not asked for.
        let v = cx.v();
        let mut down: std::collections::BTreeSet<&str> = Default::default();
        let mut todo: Vec<&str> = ON_REQUEST.iter().flat_map(|round| round.iter()).filter(|a| matches!(a.answer, Answer::Function(_))).map(|a| a.rel).collect();
        while let Some(rel) = todo.pop() {
            if !down.insert(rel) {
                continue;
            }
            let Some(id) = v.rel(rel) else { continue };
            for r in v.heads(id) {
                todo.extend(r.body().filter_map(|l| l.atom()).map(|a| v.rel_name(a.rel())).filter(|x| !down.contains(x)));
            }
        }
        let head = |of: &crate::code_rule_dsl::logic::program_image::Version, r: crate::code_rule_dsl::logic::program_image::RuleRef<'_>| down.contains(of.rel_name(r.head().rel()));
        if last.delta.removed.iter().any(|id| last.input.rule(id).is_none_or(|r| head(last.input, r))) || last.delta.added.iter().any(|&id| v.rule(id).is_none_or(|r| head(v, r))) {
            return Ok(false);
        }
        edit_as_is(cx, last, w)
    }
    fn run(&mut self, _cx: &crate::code_rule_dsl::logic::program_image::manager::Cx<'_>, w: &mut crate::code_rule_dsl::logic::program_image::manager::ProgramW<'_>) -> std::result::Result<(), String> {
        let v = w.input().clone();
        let out = answers_where_documents_are_version(v.clone()).map_err(|e| e.to_string())?;
        if Arc::ptr_eq(&out, &v) {
            return Ok(());
        }
        let b = w.on()?;
        b.state_fields_as(&v, &out);
        b.retain(|r| out.contains(r.id()));
        Ok(())
    }
}

/// The attributes answered on request by a function of the node's document (decision 0174: `value`, `node_text`)
/// whose answers are not file-local, made eager again: their rounds undone, so the projection makes them for every
/// node. A function's answer reads the node's document from the chunk's batch, which the cross-file part does not
/// hold: there `@rr_node_value` answered nothing, without an error, and every reader of `value` saw no value (the
/// Python pack: `__all__`'s and `exports`' readers ask across modules, so `value` went cross-file, and `1 if a else
/// 0`'s branches compared the same in RUF034). Until a cross-file answer reads its documents (0174 §6), an attribute a
/// cross-file reader asks for is eager. Each round whose function's answer a cross-file reader asks for is undone
/// (`link::undone_with` over `undo_round`), by the version's file-local split (`FileLocal`), to a fixpoint
/// (decision 0281's path, step 7b).
fn answers_where_documents_are_version(mut v: Arc<crate::code_rule_dsl::logic::program_image::Version>) -> Result<Arc<crate::code_rule_dsl::logic::program_image::Version>> {
    use crate::code_rule_dsl::logic::rr::lower::{Answer, ON_REQUEST, undo_round};
    loop {
        let split = crate::code_rule_dsl::logic::program_image::manager::analysis::<crate::code_rule_dsl::typing::file_local::FileLocal>(&v).map_err(Error::Plan)?;
        let declared = |rel: &str| v.declares(rel);
        let Some((round, rel)) = ON_REQUEST.iter().find_map(|round| round.iter().find(|a| matches!(a.answer, Answer::Function(_)) && declared(a.req) && !split.local.contains(a.rel)).map(|a| (*round, a.rel))) else {
            return Ok(v);
        };
        crate::code_rule_dsl::logic::profile::decision(&format!("{rel} on request"), || format!("no: its answer is not file-local (a cross-file reader asks), and the cross-file part holds no documents to read; {} eager (decision 0174 §6)", round.iter().map(|a| a.rel).collect::<Vec<_>>().join(", ")));
        v = link::undone_with(&v, |head, rules| undo_round(head, rules, round))?;
    }
}

/// The programs `programs` linked into one (`link::Part`: versions, `link::Versioned`, decision 0281's path, step
/// 7a-2), and what the combine's tail does to that (`Tail`). `images_used`: where the aliases of the module and
/// program images the combine read or kept are added (decision 0246 §6), for the combined program's entry to name.
fn combine_linked_as<P: link::Part>(programs: &[&str], extra: &added::Added, read: Option<&Arc<crate::code_rule_dsl::logic::program_image::Version>>, images_used: Option<&mut Vec<crate::content_store::HashKey>>) -> Result<(P::Linked, Tail)> {
    combine_linked_spliced::<P>(programs, extra, read, images_used, combine_link_mode() != "whole")
}

/// `RERAIL_COMBINE_LINK`: `whole` links every program each run (no splice, decision 0373); `check` splices and links
/// whole beside it, and checks the two are one program with one tail.
fn combine_link_mode() -> String {
    std::env::var("RERAIL_COMBINE_LINK").unwrap_or_default()
}

/// `RERAIL_COMBINE_TAIL`: `whole` runs the combine's tail whole each run (no kept run, decision 0393); `check` runs it
/// kept and whole beside it, and checks the two are one program.
fn combine_tail_mode() -> String {
    std::env::var("RERAIL_COMBINE_TAIL").unwrap_or_default()
}

/// `combine_linked_as`, splicing into the last link where `splice` (decision 0373).
fn combine_linked_spliced<P: link::Part>(programs: &[&str], extra: &added::Added, read: Option<&Arc<crate::code_rule_dsl::logic::program_image::Version>>, images_used: Option<&mut Vec<crate::content_store::HashKey>>, splice: bool) -> Result<(P::Linked, Tail)> {
    // Each program is given the rows the run adds once it is read (decision 0249): rows, not text after it, stated on
    // its version. A program reads only relations it declares: an error here, not the planner's panic later
    // (`front::declared_on`).
    let given_version = |v: std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>| -> Result<std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>> {
        let mut b = crate::code_rule_dsl::logic::program_image::Builder::on(&v);
        extra.add_to(&mut b);
        let v = if b.is_empty() { v } else { b.finish().map_err(Error::Plan)?.0 };
        front::declared_on(&v)?;
        Ok(v)
    };
    let parse = |text: &str| -> Result<P> { P::of_version(given_version(crate::code_rule_dsl::logic::rr::parse_version(text)?)?) };
    let parsing = crate::code_rule_dsl::logic::profile::span(|| format!("parse each program ({})", programs.len()));
    // Each text read once, for its modules and its program.
    let reading = crate::code_rule_dsl::logic::profile::span(|| "read each text (tape, statements, includes)".to_owned());
    // Each program read and lowered on the pool's threads: what each read from the library is noted here after, in
    // the programs' order (`rr::recording`, decision 0094 §6), and the first program's error in that order is the one
    // returned, as one thread gives them.
    use rayon::prelude::*;
    use crate::code_rule_dsl::logic::rr::{recorded_on_worker, replay};
    // The run's context on each job's thread (`run_state::Carry`): its observer and store, a test's own included.
    // Without it a read on the pool asked the process's store, not the test's.
    let carry = crate::code_rule_dsl::logic::run_state::Carry::here();
    let mut texts: Vec<Option<crate::code_rule_dsl::logic::rr::Text>> = if programs.len() > 1 {
        use crate::code_rule_dsl::logic::rr::{read_parsed, read_seed};
        let parsed: Vec<_> = programs
            .par_iter()
            .map(|text| {
                let _in = carry.enter();
                read_seed(text)
            })
            .collect();
        // Every program's include graph by one run of phase 0, here, before the reads: each read on the pool then
        // finds its graph kept. Derived there, each was a nested run of its own on the run's store, beside the others
        // (`content store writer is busy`; a memo made under another atom table than its writer's).
        let seeds: Vec<_> = parsed.iter().filter_map(|p| p.as_ref().ok().and_then(|r| r.seed.clone())).collect();
        if !seeds.is_empty() {
            // An error is left to the read that meets it, which reports it in its program's place.
            let _ = crate::code_rule_dsl::logic::plan::corpus::phase0::graphs(&seeds);
        }
        let each: Vec<_> = programs
            .par_iter()
            .zip(parsed)
            .map(|(text, parsed)| {
                let _in = carry.enter();
                recorded_on_worker(|| parsed.and_then(|r| read_parsed(r.statements)).map(Some).map_err(|e| in_policy(text, e)))
            })
            .collect();
        each.into_iter().map(|(got, read)| {
            replay(read);
            got
        }).collect::<std::result::Result<_, _>>()?
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
    drop(reading);
    // The run's library files, each once, includes first, and the library lowered from them, before the programs: a
    // program that asks nothing of it is lowered against its modules (decision 0246 §2), not with its declarations.
    let mut library_files: Vec<std::path::PathBuf> = Vec::new();
    let mut lowered_library: Option<std::sync::Arc<crate::code_rule_dsl::logic::rr::Library>> = None;
    // The store rule images are read from and kept in (decision 0264): the engine build's, dropped at the end of the
    // combine, which writes what it queued.
    let mut image_store: Option<std::sync::Arc<crate::content_store::ContentStore>> = None;
    if !copied && modular.iter().flatten().count() > 1 {
        let _library = crate::code_rule_dsl::logic::profile::span(|| "the libraries, once (decision 0089)".into());
        let mut declared_in: BTreeMap<&str, &std::path::Path> = BTreeMap::new();
        for modules in modular.iter().flatten() {
            for (path, _, names) in &modules.files {
                if library_files.contains(path) {
                    continue;
                }
                for name in names.iter().filter(|n| !core.contains(*n)) {
                    if let Some(other) = declared_in.insert(name, path) {
                        return Err(Error::Plan(format!("{name} is declared by two library files of the run, {} and {}", other.display(), path.display())));
                    }
                }
                library_files.push(path.clone());
            }
        }
        image_store = crate::code_rule_dsl::logic::rr::image_store();
        lowered_library = Some(crate::code_rule_dsl::logic::rr::lower_library(&library_files, image_store.as_deref())?);
    }
    let in_scope = lowered_library.as_deref().filter(|l| !l.modules.is_empty());
    // Each program lowered in scope is kept in the store as its image (decision 0246 §6).
    let images = image_store.filter(|_| in_scope.is_some());
    let lowering = crate::code_rule_dsl::logic::profile::span(|| "lower each program".to_owned());
    // Each distinct scope seeded once (decision 0246 §2), before the programs lowered in it.
    let mut scoped: Vec<crate::code_rule_dsl::logic::rr::Scoped> = Vec::new();
    let scope_of: Vec<Option<usize>> = match in_scope {
        Some(library) => texts.iter().map(|t| t.as_ref().and_then(|t| t.files())).map(|files| {
            let files = files?;
            let keys = library.scope(&files).1;
            Some(scoped.iter().position(|s| s.keys == keys).unwrap_or_else(|| {
                scoped.push(library.scoped(&files));
                scoped.len() - 1
            }))
        }).collect(),
        None => vec![None; texts.len()],
    };
    // The combine's last link (decision 0373): each program's slot is its image's key, and the run's shape the key the
    // link is kept under. Where it is the last run's, only the programs whose image changed are lowered.
    let per_program = std::env::var("RERAIL_REQUESTS").is_ok_and(|v| v == "copied");
    let keeping: Option<(Vec<[u8; 32]>, [u8; 32])> = (|| {
        let library = in_scope?;
        images.as_ref()?;
        if !splice || !P::KEEPS || programs.len() < 2 || read.is_some() || copied {
            return None;
        }
        let keys = programs.iter().zip(&scope_of).zip(&texts).map(|((text, k), t)| t.as_ref().and_then(|_| scoped[(*k)?].program_image(text).sha256_bytes())).collect::<Option<Vec<_>>>()?;
        Some((keys, kept_link::family(programs.len(), extra, &library.image_keys())))
    })();
    if let (Some((keys, family)), Some(store), Some(library)) = (&keeping, images.as_deref(), in_scope)
        && let Some(got) = combine_spliced::<P>(store, family, keys, programs, &mut texts, &scope_of, &scoped, library, &modular, extra, per_program)?
    {
        drop(lowering);
        if let Some(out) = images_used {
            out.extend(library.image_keys());
            out.extend(programs.iter().zip(&scope_of).filter_map(|(text, k)| Some(scoped[(*k)?].program_image(text))));
        }
        return Ok(got);
    }
    // Each program's kept image, read for all of them at once (a few statements, not three each from the pool's
    // threads), its lease renewed in one more.
    let mut prefetched: Vec<Option<Option<crate::code_rule_dsl::logic::rr::images::Found>>> = (0..programs.len()).map(|_| None).collect();
    if let Some(store) = images.as_deref().filter(|_| in_scope.is_some() && !(read.is_some() && programs.len() == 1)) {
        let _s = crate::code_rule_dsl::logic::profile::span(|| "the programs' images, from the store".to_owned());
        let asked: Vec<(usize, crate::content_store::HashKey)> = programs.iter().zip(&scope_of).zip(&texts).enumerate().filter_map(|(i, ((text, k), t))| Some((i, scoped[(*k)?].program_image(text))).filter(|_| t.is_some())).collect();
        let found = crate::code_rule_dsl::logic::rr::images::read_all(Some(store), &asked.iter().map(|(_, k)| *k).collect::<Vec<_>>());
        let ids: Vec<crate::content_store::SourceId> = found.iter().flatten().filter_map(|(_, id)| *id).collect();
        crate::code_rule_dsl::logic::rr::renew_images(store, &ids);
        for ((i, _), f) in asked.into_iter().zip(found) {
            prefetched[i] = Some(f);
        }
    }
    let each: Vec<_> = programs
        .par_iter()
        .zip(&own_only)
        .zip(texts.par_iter_mut())
        .zip(&scope_of)
        .zip(prefetched.into_par_iter())
        .map(|((((text, &own_only), slot), scope_of), found)| {
            let _in = carry.enter();
            recorded_on_worker(|| -> Result<P> {
            if let Some(read) = read.filter(|_| programs.len() == 1) {
                return P::of_version(given_version(Arc::clone(read))?);
            }
            if let Some(read) = slot.take() {
                if let Some(library) = in_scope && let Some(k) = *scope_of {
                    let _in_scope = crate::code_rule_dsl::logic::profile::span(|| "lower the program in its libraries' scope (decision 0246 §2)".to_owned());
                    // Its library's relations are the library's, declared there, not in the program: it is handed the
                    // run's rows (0249) but not checked for its own declarations (`front::declared`).
                    return P::lowered_in(read, text, library, &scoped[k], images.as_deref(), found, extra);
                }
                return P::of_version(given_version(read.lower_version(!own_only)?)?);
            }
            if !own_only {
                return parse(text);
            }
            P::of_version(given_version(crate::code_rule_dsl::logic::rr::parse_own_version(text)?)?)
            })
        })
        .collect();
    let mut parsed = each
        .into_iter()
        .map(|(got, read)| {
            replay(read);
            got
        })
        .collect::<std::result::Result<Vec<_>, Error>>()?;
    drop(lowering);
    if let (Some(out), Some(library), Some(_)) = (images_used, in_scope, images.as_ref()) {
        out.extend(library.image_keys());
        out.extend(programs.iter().zip(&scope_of).filter_map(|(text, k)| Some(scoped[(*k)?].program_image(text))));
    }
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
    let guests_of: Vec<bool> = parsed.iter().map(|p| guests::REQUESTS.iter().any(|r| p.has_head(r))).collect();
    let makes_guests = guests_of.iter().any(|&g| g);
    let bounds: Vec<u32> = parsed.iter().map(|p| hiding::bound(p.head())).collect();
    // The libraries once (decision 0089): two or more programs name theirs as modules, and all are read under one
    // bound of the documents they show, so the library's relations are the same for each.
    // `RERAIL_LIBRARIES=copied` copies them into each program, as before, to compare.
    let shared = !copied && modular.iter().flatten().count() > 1 && bounds.windows(2).all(|w| w[0] == w[1]);
    // Not held once after all: a program read without its libraries' rules is read whole.
    if !shared {
        for ((prog, text), &own_only) in parsed.iter_mut().zip(programs).zip(&own_only) {
            if own_only || in_scope.is_some() {
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
    let mut library: Option<P> = None;
    // A program's request (§3): its key's number and the relations it holds, named `q<k>_` for every asker.
    let mut requests: Vec<Option<(usize, BTreeSet<String>)>> = vec![None; parsed.len()];
    // Each program's request as its interface reads it (decision 0373): its key, number, and whether it asked first.
    let mut asked: Vec<Option<([u8; 32], usize, bool)>> = vec![None; parsed.len()];
    let mut first_askers: BTreeMap<[u8; 32], (usize, usize)> = BTreeMap::new();
    if shared {
        for (i, modules) in modular.iter().enumerate() {
            let Some(modules) = modules else { continue };
            for name in &modules.declared {
                if let Some(file) = modules.files.iter().find(|(_, _, names)| names.contains(name) && !core.contains(name)) {
                    return Err(Error::Plan(format!("program #{i} declares {name}, which {} it includes declares", file.0.display())));
                }
            }
        }
        library = Some(P::library(lowered_library.as_ref().expect("made with the programs' modules"))?);
        let mut cones = 0;
        let mut kept = 0;
        // Each program's request (§3): the cone's library relations and the program's own relations they read, held
        // once per distinct key. `RERAIL_REQUESTS=copied` keeps one copy per program, to compare.
        let mut keys: BTreeMap<[u8; 32], usize> = BTreeMap::new();
        let mut copies: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
        let mut asking = 0;
        for (i, (prog, modules)) in parsed.iter_mut().zip(&modular).enumerate() {
            let Some(modules) = modules else { continue };
            let (dropped, cone) = own_part(prog, modules, core)?;
            kept += dropped;
            cones += cone.len();
            if cone.is_empty() {
                continue;
            }
            asking += 1;
            let (unit, key) = request_unit(prog, &cone, core, i, per_program);
            let n = keys.len();
            let k = *keys.entry(key).or_insert(n);
            asked[i] = Some((key, k, k == n));
            if k == n {
                first_askers.insert(key, (k, i));
            }
            for rel in cone.iter() {
                copies.entry(rel.clone()).or_default().insert(k);
            }
            if k != n {
                // Another program asked the same: read its copy.
                drop_relations(prog, &unit)?;
            }
            requests[i] = Some((k, unit));
        }
        let rules = library.as_ref().map_or(0, |l| l.rule_count());
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
    // Programs combined have their node attributes answered on request over the combined program (below), each
    // part's own requests undone first: before hiding, which copies a relation's rules into its grammar-scoped copies
    // (`rr_in_<g>_token`). Undone after it, the copies kept the answer's rules, reading the request's inputs
    // (`rr_token_anon`) that the undo no longer declared, and the combined program had nothing to answer them (Q67,
    // the data-layer pack).
    if parsed.len() > 1 {
        for p in parsed.iter_mut() {
            p.undo_on_request()?;
        }
    }
    // A run that makes guests bounds each program's rules to the documents it shows (decision 0058 §1); a run that
    // makes none is left as written. With the libraries once, the combined program is bounded once (below).
    if makes_guests && !shared {
        let _hiding = crate::code_rule_dsl::logic::profile::span(|| {
            format!("bound each program to the documents it shows ({} programs, {} rules; decision 0058)", parsed.len(), parsed.iter().map(|p| p.rule_count()).sum::<usize>())
        });
        for (i, p) in parsed.iter_mut().enumerate() {
            let _one = crate::code_rule_dsl::logic::profile::span(|| format!("program #{i} ({} rules)", p.rule_count()));
            let bound = hiding::bound(p.head());
            p.hide(bound)?;
        }
    }
    let _merging = crate::code_rule_dsl::logic::profile::span(|| "merge the programs".into());
    if parsed.len() == 1 {
        let one = parsed.pop().expect("one program");
        return Ok((one.alone()?, Tail::One));
    }
    let mut all = link::Merged::new(P::fields());
    let mut sets: BTreeMap<Vec<String>, String> = BTreeMap::new();
    // The library part's names for its kind sets, as the merge renamed them.
    let mut library_sets: BTreeMap<String, String> = BTreeMap::new();
    let core = crate::code_rule_dsl::logic::rr::core_names();
    let mut policies: Vec<scoping::Policy> = Vec::new();
    // A library's rule may ask for guests too (`vue/view.rr`'s view, decision 0222): read once, it is no program's.
    let makes_guests = makes_guests || library.as_ref().is_some_and(|l| guests::REQUESTS.iter().any(|r| l.has_head(r)));
    // The node attributes answered on request (decision 0174) are the combined program's, made once below over all
    // its readers: a part lowered on its own (a program's own part, without the library's rules) could not see
    // which library relations read an attribute, so its requests read them as they are, not through their widened
    // copies, and a request was recursive with its attribute (PYI062 with PYI059: `literal_down` negates
    // `typing_subscript`, which reads `value` through `exports`, and `rr_value_req` read `literal_leaf`).
    if let Some(l) = library.as_mut() {
        l.undo_on_request()?;
    }
    // Each part's own requests were undone before hiding (above).
    // The library, when held once, is the first part: its names are the run's (decision 0089 §2).
    let units = library.into_iter().map(|l| (None, l)).chain(parsed.into_iter().enumerate().map(|(i, p)| (Some(i), p)));
    let mut rules: Vec<P::Rules> = Vec::new();
    let mut interfaces: Vec<[u8; 32]> = Vec::new();
    for (unit, part) in units {
        let i = unit.unwrap_or(usize::MAX);
        let prog = part.head();
        let scope = prog.doc_scopes();
        let (named, library) = front::globals(&prog.settings());
        // The documents read: those some policy's scope selects; every one when a policy has none.
        all.doc_scopes = match (i, all.doc_scopes.take(), scope.clone()) {
            (0, _, mine) => mine,
            (_, Some(mut a), Some(b)) => {
                a.extend(b);
                Some(a)
            }
            _ => None,
        };
        let globs = prog.text_globs();
        for g in &globs {
            if !all.text_globs.contains(g) {
                all.text_globs.push(g.clone());
            }
        }
        // Kind sets by their kinds: this program's name for each, and the shared one. A set the program reads but
        // does not hold (a library rule its request copied, `rr::with_cone_version`) is the library's: named as the
        // library part names it, not left on a name the merge renamed (PR #508's afeb076e5, t47).
        let mut set_of: BTreeMap<String, String> = if unit.is_some() { library_sets.clone() } else { BTreeMap::new() };
        let kinds_of = prog.kindsets();
        for (name, kinds) in kinds_of.clone() {
            let n = sets.len();
            let shared = sets.entry(kinds.clone()).or_insert_with(|| format!("ks{n}")).clone();
            all.kindsets.insert(shared.clone(), kinds);
            set_of.insert(name, shared);
        }
        if unit.is_none() {
            library_sets = set_of.clone();
        }
        // The library's relations keep their names (decision 0089 §2). Named repository readers belong to their
        // policy (`own_names`): equal relation names alone do not mean equal input files or selectors.
        let own: BTreeSet<String> = if unit.is_some() { own_names(&part, core) } else { BTreeSet::new() };
        let request = unit.and_then(|u| requests[u].clone());
        // The library held once is no policy's: canonical by construction, made over every document read (0091 §3).
        if unit.is_some() {
            policies.push(scoping::Policy { prefix: format!("p{i}_"), own: own.iter().map(|r| format!("p{i}_{r}")).collect(), named, library, scope });
        }
        let core_rules = if keeping.is_some() && unit.is_some() { part.shown(core) } else { Vec::new() };
        let names = link::Names { i, own, request, set_of };
        let (header, renamed) = part.into_parts(&names);
        if keeping.is_some() && let Some(u) = unit {
            interfaces.push(kept_link::interface(&[&u, &bounds[u], &guests_of[u], &asked[u], &policies.last(), &globs, &kinds_of, &names.set_of, &names.own, &names.request, &crate::code_rule_dsl::logic::program_image::fields::Shown(P::header_fields(&header)), &core_rules]));
        }
        rules.push(renamed);
        all.merge(P::header_fields(&header), &names);
    }
    let (all, counts) = P::link_counted(all.finish(), rules, core)?;
    let hide = (shared && makes_guests).then_some(bounds[0]);
    // The link kept for the next run's splice (decision 0373), where the libraries are held once.
    let mut kept = None;
    if let (Some((keys, family)), Some(store), Some(counts), Some(v), true) = (&keeping, images.as_deref(), counts, P::linked_version(&all), shared && interfaces.len() == programs.len()) {
        let slots = keys.iter().zip(&interfaces).map(|(k, f)| kept_link::Slot { key: *k, interface: *f }).collect();
        kept_link::record(store, family, v, &kept_link::Record { counts, slots, requests: first_askers, sets, bound: bounds[0], hide, policies: policies.clone() });
        kept = Some(TailKept(*family));
    }
    Ok((all, Tail::Many { hide, policies, kept }))
}

/// `combine_linked_as` as a splice into the combine's last link (decision 0373): the programs whose image changed
/// lowered and read as the merge reads each (`own_part`, the request, `undo_on_request`, the header's kind sets,
/// globals and policy), and where each one's interface is its slot's, its rules written in its slot's place and every
/// other program's copied from the last link (`Part::spliced`). `None` where nothing is kept for the run's shape, or a
/// program's interface changed: the programs are then linked whole, and the link kept again.
#[allow(clippy::too_many_arguments)]
fn combine_spliced<P: link::Part>(
    store: &ContentStore,
    family: &[u8; 32],
    keys: &[[u8; 32]],
    programs: &[&str],
    texts: &mut [Option<crate::code_rule_dsl::logic::rr::Text>],
    scope_of: &[Option<usize>],
    scoped: &[crate::code_rule_dsl::logic::rr::Scoped],
    library: &crate::code_rule_dsl::logic::rr::Library,
    modular: &[Option<crate::code_rule_dsl::logic::rr::Modules>],
    extra: &added::Added,
    per_program: bool,
) -> Result<Option<(P::Linked, Tail)>> {
    let Some((last, mut rec, sources)) = kept_link::lookup(store, family) else { return Ok(None) };
    // Kept as the programs' images are, in the same store: its lease renewed where a run reads it (decision 0250).
    crate::code_rule_dsl::logic::rr::renew_images(store, &sources);
    // The header's rules, the library's, then each program's.
    if rec.slots.len() != programs.len() || rec.counts.len() != programs.len() + 2 {
        return Ok(None);
    }
    let changed: Vec<usize> = (0..programs.len()).filter(|&i| rec.slots[i].key != keys[i]).collect();
    let _span = crate::code_rule_dsl::logic::profile::span(|| format!("the changed programs spliced into the combine's last link ({} of {}; decision 0373)", changed.len(), programs.len()));
    let core = crate::code_rule_dsl::logic::rr::core_names();
    let mut parts: Vec<(usize, P::Rules)> = Vec::new();
    for &i in &changed {
        let (Some(read), Some(k), Some(modules)) = (texts[i].take(), scope_of[i], modular[i].as_ref()) else { return Ok(None) };
        let mut part = P::lowered_in(read, programs[i], library, &scoped[k], Some(store), None, extra)?;
        let bound = hiding::bound(part.head());
        if bound != rec.bound {
            return Ok(None);
        }
        let guests = guests::REQUESTS.iter().any(|r| part.has_head(r));
        for name in &modules.declared {
            if let Some(file) = modules.files.iter().find(|(_, _, names)| names.contains(name) && !core.contains(name)) {
                return Err(Error::Plan(format!("program #{i} declares {name}, which {} it includes declares", file.0.display())));
            }
        }
        let (_, cone) = own_part(&mut part, modules, core)?;
        let (mut request, mut asked) = (None, None);
        if !cone.is_empty() {
            let (unit, key) = request_unit(&part, &cone, core, i, per_program);
            // A request no earlier program asks first would be numbered, and copied, otherwise.
            let Some(&(k, first)) = rec.requests.get(&key).filter(|(_, first)| *first <= i) else { return Ok(None) };
            if first != i {
                drop_relations(&mut part, &unit)?;
            }
            asked = Some((key, k, first == i));
            request = Some((k, unit));
        }
        part.undo_on_request()?;
        let prog = part.head();
        let scope = prog.doc_scopes();
        let (named, library_names) = front::globals(&prog.settings());
        let globs = prog.text_globs();
        let kinds_of = prog.kindsets();
        let mut set_of: BTreeMap<String, String> = BTreeMap::new();
        for (name, kinds) in &kinds_of {
            let Some(shared) = rec.sets.get(kinds) else { return Ok(None) };
            set_of.insert(name.clone(), shared.clone());
        }
        let own = own_names(&part, core);
        let policy = scoping::Policy { prefix: format!("p{i}_"), own: own.iter().map(|r| format!("p{i}_{r}")).collect(), named, library: library_names, scope };
        let core_rules = part.shown(core);
        let names = link::Names { i, own, request, set_of };
        let (header, renamed) = part.into_parts(&names);
        let interface = kept_link::interface(&[&i, &bound, &guests, &asked, &Some(&policy), &globs, &kinds_of, &names.set_of, &names.own, &names.request, &crate::code_rule_dsl::logic::program_image::fields::Shown(P::header_fields(&header)), &core_rules]);
        if interface != rec.slots[i].interface {
            crate::code_rule_dsl::logic::profile::decision("the combine's last link (decision 0373)", || format!("linked again: program #{i}'s interface changed"));
            return Ok(None);
        }
        rec.slots[i].key = keys[i];
        parts.push((2 + i, renamed));
    }
    let Some((linked, counts)) = P::spliced(&last, &rec.counts, parts, core)? else { return Ok(None) };
    crate::code_rule_dsl::logic::profile::decision("the combine's last link (decision 0373)", || format!("spliced: {} of {} programs changed, each of the same interface", changed.len(), programs.len()));
    if !changed.is_empty()
        && let Some(v) = P::linked_version(&linked)
    {
        rec.counts = counts;
        kept_link::record(store, family, v, &rec);
    }
    Ok(Some((linked, Tail::Many { hide: rec.hide, policies: rec.policies, kept: Some(TailKept(*family)) })))
}

/// `simplify::merge_identical` between two `drop_repeated_rules`: the relations merged, the classes shared across
/// programs, the rules dropped.
pub fn merge_and_drop_repeats(prog: &mut front::Program) -> (usize, usize, usize) {
    // One merge finds every class at once: its congruence is the greatest, and it compares a relation's rules as a
    // set, so a repeated rule does not keep a relation from its twin. The merge's renaming can make rules repeat;
    // they are dropped after it, and make nothing more mergeable. (A loop of merges until no rule repeated was 2-3
    // congruences a call, 1.5 s of the dl1 batch's 13.8 s CPU.)
    let mut repeated = drop_repeated_rules(prog);
    let (merged, shared) = crate::code_rule_dsl::logic::simplify::merge_identical(prog);
    repeated += drop_repeated_rules(prog);
    (merged, shared, repeated)
}

/// Drops each rule equal to an earlier one: a relation is a set, so a second copy of a rule derives nothing, yet it
/// was planned and run as a rule of its own. Hiding's clones made them (at O125 step 07, 407 of the cross part's
/// 1,483 rules, `rr_in_annotation_0_bv`'s most). Returns how many it dropped.
pub fn drop_repeated_rules(prog: &mut front::Program) -> usize {
    use std::hash::{Hash, Hasher};
    let before = prog.rules.len();
    let mut seen: HashMap<u64, Vec<usize>> = HashMap::new();
    // Each rule kept with the rules it stands for: itself and its repeats, whose readers it serves (the request
    // record, `Program::set_rules_from`).
    let mut kept: Vec<(usize, Vec<usize>)> = Vec::with_capacity(before);
    for (i, r) in prog.rules.iter().enumerate() {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        r.hash(&mut h);
        let at = seen.entry(h.finish()).or_default();
        match at.iter().find(|&&k| prog.rules[kept[k].0] == *r) {
            Some(&k) => kept[k].1.push(i),
            None => {
                at.push(kept.len());
                kept.push((i, vec![i]));
            }
        }
    }
    if kept.len() == before {
        return 0;
    }
    let rules: Vec<(front::Rule, Vec<usize>)> = kept.into_iter().map(|(i, from)| (prog.rules[i].clone(), from)).collect();
    prog.set_rules_from(rules);
    before - prog.rules.len()
}

/// What the converter gives an authored file to read (`logic::convert`, test-only): the scope, the report gate, the
/// options, and the tape's syntax verdict (`tape_syntax_error(d)`: core.rr's `syntax_error` is the engine's, decision
/// 0141). Here so the runner, which is not test-only, can tell them from unbound core relations.
pub const GIVEN: [&str; 8] = ["in_scope", "out_scope", "report_when_gt", "option_on", "option_item", "option_value", "option_json", "tape_syntax_error"];

/// Fails, naming it, where `prog` reads one of core.rr's relations that nothing derives and the runner does not bind
/// (`value`, `str_part`, …): the planner would otherwise fail on its missing statistics (`no statistics for
/// value.c0`). A program's own inputs are its business (facts it is given later), so are the facts the converter
/// gives (`GIVEN`: core.rr's `option_item` among them, empty for an option of no items), and the relations not made
/// yet are empty (`NOT_MADE`, O76).
fn bound(v: &crate::code_rule_dsl::logic::program_image::Version) -> std::result::Result<(), String> {
    let derived = |rel: &str| v.rel(rel).is_some_and(|r| v.heads(r).next().is_some()) || v.find(rel).and_then(|a| v.facts(a)).is_some_and(|f| !f.about());
    let cpython = "CPython (core.rr: the isolated `ast.parse` worker), which the corpus runner does not run";
    // `env` is the run's settings (`tools::with_settings`): facts when there are any, else empty. The world's
    // relations (`dir_entry`, `file_line`, …, and `loaded`) are answered by the runner (`world`).
    let world = |rel: &str| rel == world::LOADED || crate::code_rule_dsl::logic::rr::WORLD.iter().any(|(r, _)| *r == rel);
    let given = |rel: &str| GIVEN.contains(&rel) || rel == "env" || world(rel) || NOT_MADE.iter().any(|(r, _)| *r == rel);
    // core.rr's literal parts are made in core.rr's shape only (`syntax::core_rr`): a program that declares its base
    // relations itself (T's shape, `syntax::Projection::of`: not every relation core.rr declares is declared) has none.
    let core = crate::code_rule_dsl::facts::core::core_names();
    let t_shape = core.is_empty() || !core.iter().all(|r| v.decl_of(r.as_str()).is_some());
    let parts = |rel: &str| t_shape && rel.starts_with("str_part");
    for rule in v.rules() {
        for a in rule.body().filter_map(|l| l.atom()) {
            let rel = v.rel_name(a.rel());
            if !parts(rel) && (derived(rel) || super::super::sql::select::input_columns(rel).is_some() || !crate::code_rule_dsl::logic::rr::is_core(rel) || given(rel)) {
                continue;
            }
            let why = match rel {
                _ if parts(rel) => format!("core.rr's shape makes it (a program that includes core.rr); without it, it needs {cpython}"),
                "list_len" => "it counts a pattern's list variable ($$$X), and patterns do not lower".to_owned(),
                _ => "the corpus runner has no producer of it (the relations it binds: candidates/datalog/authored/README.md)".to_owned(),
            };
            return Err(format!("{rel} is read, and nothing derives it or binds it: {why}"));
        }
    }
    Ok(())
}

/// Of a program whose text names its libraries as modules (decision 0089 §5), the part the run's library does not
/// hold, taken out of `prog`: what is left is the program's own. A library relation goes to the library unless the
/// program's own clauses add to it or it reads one they add to, through any relation (the request's cone, §3): the
/// program keeps its own copy of those, as before. A relation lowering made for library rules alone (every reader
/// of it the library's) goes with them. Returns the relations taken out, and the cone's.
fn own_part<P: link::Part>(of: &mut P, modules: &crate::code_rule_dsl::logic::rr::Modules, core: &BTreeSet<String>) -> Result<(usize, BTreeSet<String>)> {
    let rules = of.rules();
    let prog = of.head();
    let inputs = prog.input_names();
    let natives = prog.natives();
    let declared = prog.declared_names();
    // An input the program reads (`within`, which lowering keeps where a rule reads it) is a base relation, the run's:
    // its declaration stays with the program.
    let library: BTreeSet<&str> =
        modules.files.iter().flat_map(|(_, _, names)| names).map(String::as_str).filter(|r| !core.contains(*r) && !inputs.iter().any(|i| i == r)).collect();
    let edges: Vec<(String, Vec<String>)> = rules
        .iter()
        .cloned()
        .chain(natives.iter().map(|(rel, n)| (rel.to_string(), n.reads().into_iter().map(str::to_owned).collect())))
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
    let candidates: BTreeSet<String> = declared
        .iter()
        .map(String::as_str)
        .chain(rules.iter().map(|(head, _)| head.as_str()))
        .chain(natives.keys().map(String::as_str))
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
    let taken = part.iter().filter(|r| declared.contains(*r) || rules.iter().any(|(head, _)| head == *r)).count();
    let cones = cone.into_iter().filter(|r| library.contains(r.as_str())).collect();
    drop_relations(of, &part)?;
    Ok((taken, cones))
}

/// `relations` taken out of `part`: their rules, declarations, rows, natives, lattices and nullary marks.
fn drop_relations<P: link::Part>(part: &mut P, relations: &BTreeSet<String>) -> Result<()> {
    use crate::code_rule_dsl::logic::program_facts::Field;
    part.drop_heads(&|head| relations.contains(head));
    part.edit_head(&mut |prog| {
        for r in prog.declared_names() {
            if relations.contains(&r) {
                prog.undeclare(&r);
            }
        }
        for r in prog.fact_names() {
            if relations.contains(&r) {
                prog.drop_facts(&r);
            }
        }
        let natives = prog.natives();
        if natives.keys().any(|r| relations.contains(r)) {
            let kept: BTreeMap<_, _> = natives.into_iter().filter(|(r, _)| !relations.contains(r)).collect();
            prog.set_field(Field::Natives(&kept));
        }
        let lattice = prog.lattice();
        if lattice.keys().any(|r| relations.contains(r)) {
            let kept: BTreeMap<_, _> = lattice.into_iter().filter(|(r, _)| !relations.contains(r)).collect();
            prog.set_field(Field::Lattice(&kept));
        }
        let nullary = prog.nullary();
        if nullary.iter().any(|r| relations.contains(r)) {
            let kept: BTreeSet<String> = nullary.into_iter().filter(|r| !relations.contains(r)).collect();
            prog.set_field(Field::Nullary(&kept));
        }
    })
}

/// A program's relations that take its prefix (`p<i>_`) when programs are combined: what it declares, holds facts of
/// or derives, other than the base inputs, the shared outputs, `core.rr`'s (decision 0141), the earlier phases'
/// exports and the guests' shared relations. A repository file a policy reads (`.input r(IO=…)`) is its own.
fn own_names<P: link::Part>(part: &P, core: &BTreeSet<String>) -> BTreeSet<String> {
    let prog = part.head();
    let heads: Vec<String> = part.rules().into_iter().map(|(head, _)| head).collect();
    // `violation`, and each finding's relation (`rr::is_diagnostic`, named by its policy).
    let shared_outputs = |r: &str| r == "violation" || crate::code_rule_dsl::logic::rr::is_diagnostic(r);
    let file_inputs = file_inputs(prog);
    let inputs = prog.input_names();
    let (declared, facts) = (prog.declared_names(), prog.fact_names());
    declared
        .iter()
        .map(String::as_str)
        .chain(facts.iter().map(String::as_str))
        .chain(heads.iter().map(String::as_str))
        .filter(|r| (!inputs.iter().any(|i| i == r) || file_inputs.contains(*r)) && !shared_outputs(r) && !core.contains(*r) && !phases::is_phased(r) && !guests::SHARED.contains(r))
        .map(str::to_owned)
        .collect()
}

/// The relations a policy reads from repository files (`.input r(IO=…)`): each is the policy's own.
fn file_inputs(prog: &dyn crate::code_rule_dsl::logic::program_image::fields::ProgramFields) -> BTreeSet<String> {
    prog.settings().iter().filter(|(key, _)| key == "input").filter_map(|(_, text)| text.split_once('(').map(|(rel, _)| rel.to_owned())).collect()
}

/// A program's request (decision 0089 §3): the library relations of its cone (`cone`) and every relation of its own
/// they read, through rules and natives; and the request's key, the digest of what those relations are. Two programs
/// with one key ask the same, so they read one copy. A request that reads one of the program's own relations it does
/// not hold, or a repository file, is the program's alone (its index is in the key), as is every request when
/// `alone`.
fn request_unit<P: link::Part>(part: &P, cone: &BTreeSet<String>, core: &BTreeSet<String>, i: usize, alone: bool) -> (BTreeSet<String>, [u8; 32]) {
    use sha2::Digest as _;
    let own = own_names(part, core);
    let prog = part.head();
    let rules = part.rules();
    let files = file_inputs(prog);
    let (natives, lattice, nullary) = (prog.natives(), prog.lattice(), prog.nullary());
    let mut unit: BTreeSet<String> = cone.iter().filter(|r| own.contains(*r)).cloned().collect();
    let mut work: Vec<String> = unit.iter().cloned().collect();
    let mut outside = false;
    while let Some(rel) = work.pop() {
        let mut reads: Vec<String> = rules.iter().filter(|(head, _)| *head == rel).flat_map(|(_, reads)| reads.iter().cloned()).collect();
        if let Some(native) = natives.get(&rel) {
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
    let mut rules: Vec<String> = part.shown(&unit);
    rules.sort();
    for rule in rules {
        hash.update(rule);
        hash.update("\n");
    }
    for rel in &unit {
        hash.update(format!(
            "{rel} {:?} {:?} {:?} {:?} {}\n",
            prog.columns_of(rel),
            prog.states_facts(rel).then(|| prog.fact_rows(rel)),
            natives.get(rel),
            lattice.get(rel),
            nullary.contains(rel)
        ));
    }
    (unit, hash.finalize().into())
}

/// Sources a queued write stores (decision 0102), for the manifest queued after it.
pub type Late = Arc<std::sync::Mutex<Vec<crate::content_store::SourceId>>>;

/// The cross part's new atoms (the names it made, `@cat`), to the writer as its rows are derived: a wave of their
/// own, written while the part's kept rows are framed (`retain`), not after them in the part's last wave (decision
/// 0296). None pending (a warm run's), nothing queued.
fn cross_atoms_ahead(store: &ContentStore) {
    if store.atoms_handle().pending() {
        store.queue_atoms_flush();
        crate::content_store::write_back::end_wave(crate::content_store::write_back::Boundary::CrossDerived);
    }
}

/// The store's collection (decision 0053), started once the run's manifest is written: `join` waits for it.
pub struct Collecting(std::thread::JoinHandle<Option<(usize, std::result::Result<String, String>)>>);

impl Collecting {
    /// The passes and their outcome, once the manifest is written and the passes ran; `None` when no pass was due.
    pub fn join(self) -> std::thread::Result<Option<(usize, std::result::Result<String, String>)>> {
        self.0.join()
    }
}

/// `local` with only the rules `wanted` are derived from (decision 0225 §2): what a chunk of partial memo hits runs.
/// A part of `local`'s version by the cone's relations (decision 0244), found from the version's rules: no tree is
/// decoded or copied. `local` is a split's file-local part, never maintained, so it always has one.
fn wanted_cone(local: &lower::Lowered, wanted: &BTreeSet<String>) -> lower::Lowered {
    use crate::code_rule_dsl::logic::program_image::{Builder, Lit, PartFields};
    let closure = |reads: &dyn Fn(&BTreeSet<String>) -> BTreeSet<String>| {
        let mut cone = wanted.clone();
        loop {
            let more: BTreeSet<String> = reads(&cone).into_iter().filter(|r| !cone.contains(r)).collect();
            if more.is_empty() {
                return cone;
            }
            cone.extend(more);
        }
    };
    {
        let v = local.version.clone();
        let cone = closure(&|cone| {
            v.rules()
                .filter(|r| cone.contains(v.rel_name(r.head().rel())))
                .flat_map(|r| r.body().filter_map(|l| match l.view() { Lit::Atom(a) | Lit::Agg { atom: a, .. } => Some(v.rel_name(a.rel()).to_owned()), _ => None }).collect::<Vec<_>>())
                .collect()
        });
        let inside = |rel: &str| cone.contains(rel);
        let inputs: Vec<&str> = v.inputs().map(|a| v.text(a)).collect();
        let outputs: Vec<&str> = v.outputs().map(|a| v.text(a)).filter(|o| cone.contains(*o)).collect();
        let answers: BTreeMap<String, Vec<String>> = crate::code_rule_dsl::logic::program_image::manager::analysis::<crate::code_rule_dsl::logic::program_image::About>(&v).expect("a version's about rows are read").answers.iter()
            .map(|(req, by)| (v.rel_name(*req).to_owned(), v.rel_name(*by).to_owned()))
            .fold(BTreeMap::new(), |mut m: BTreeMap<String, Vec<String>>, (req, by)| { m.entry(req).or_default().push(by); m });
        let kept: BTreeMap<String, Vec<String>> = answers.iter().filter(|(r, _)| cone.contains(r.as_str())).map(|(r, b)| (r.clone(), b.clone())).collect();
        let made = v.part(inside, &inputs, &outputs, PartFields { natives: &|_| true, lattice: &inside, gates: &inside }).ok().and_then(|p| {
            if kept.len() == answers.len() {
                return Some(p);
            }
            let mut b = Builder::on(&p);
            b.set_answers(&kept);
            b.finish_overlay().ok()
        });
        let made = made.expect("a part of a version by its relations is made");
        local.with_version(made)
    }
}

/// The root's runs whose files' tapes a run reads before its first wave (Q22): its own last, and the phases' between.
const TAPE_RUNS: usize = 4;

/// Records the run's manifest and relations, then starts collection if a pass is due: queued behind the run's
/// writes (decision 0102), so the manifest names only what is written, and the lease on collection is let go
/// once it is.
#[allow(clippy::too_many_arguments)]
pub fn record_run(store: &ContentStore, root: &Path, sources: &[crate::content_store::SourceId], program: &[u8; 32], also: &[[u8; 32]], lane: Option<[u8; 32]>, set: Option<[u8; 32]>, relations: &[(String, String)], late: &Late, reading: Option<crate::content_store::CollectionLease>, outer: bool) -> Result<Option<Collecting>> {
    memo::off();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let (root, sources, also, relations) = (root.to_string_lossy().into_owned(), sources.to_vec(), also.to_vec(), relations.to_vec());
    let plan = memo::plan_digest(program);
    let batch = phases::batch_plans(&root, plan, &also);
    // A batch collects at its first run alone: the last batch's final manifest names all that batch's plans then. Later,
    // the root's protected manifests (its last two) are this batch's, and name only the policies run so far: the last
    // batch's plans of those still to run were dead, and collected under them (O-store-single-program).
    let first_of_batch = batch.as_ref().is_none_or(Vec::is_empty);
    let also: Vec<[u8; 32]> = also.into_iter().chain(batch.into_iter().flatten()).collect();
    // A run inside another (a phase's, as phase 1's while the outer run lists its files; the constants') leaves
    // collection to the outer run, which collects after its last wave: here it would run beside the outer run's waves.
    let collect = !std::env::var("RERAIL_CORPUS_GC").is_ok_and(|v| v == "off") && outer && first_of_batch;
    // Every lane names a set (decision 0302 §4: a lane with none is an older binary's). A run inside another, or one
    // whose programs name none, is its lane's own set: never superseded, protected as before. A run inside another
    // would otherwise supersede the outer run's lane.
    let set = set.filter(|_| outer).or(lane);
    // The frames a keep decision leaves unreachable are a utility upkeep job's (decision 0227): one a process for the
    // store, finished at the outermost run's end (`upkeep::finish_soon`) if the scheduler has not run it.
    if collect {
        crate::content_store::upkeep::MemoSweep::register_once(store.database_path());
    }
    // `also`: the outer run's names its nested runs' plans too (`Report::plans`).
    let mut policy = crate::content_store::CorpusPolicy::default();
    if let Some(mb) = std::env::var("RERAIL_CORPUS_HISTORY_MB").ok().and_then(|v| v.parse::<u64>().ok()) {
        (policy.high_water, policy.low_water) = (mb << 20, (mb << 20) / 4 * 3);
    }
    // The sources stored by writes queued before the manifest's, and the texts named meanwhile: the record's item
    // takes them as the writer applies it (`take_named_sources`), with these, named on the run's side.
    let mut sources = sources;
    sources.extend(late.lock().expect("the late sources").iter().copied());
    let key = manifest_key(&root, &plan);
    let manifest = Manifest {
        root,
        sources,
        grammar_version: crate::code_rule_dsl::build_ids::grammar_version(),
        tape_format: i64::from(crate::code_rule_dsl::tape::formats::stored_version()),
        plan,
        also,
        lane,
        set,
        now,
        relations,
    };
    // The run's record's item (decision 0306), after every write the run queued: the manifest names only what is
    // written, and the lease on collection is let go once it is (below).
    store.record("the run's manifest", vec![key], &manifest, None, apply_manifest);
    if !collect {
        // A run that collects nothing does not wait for its manifest: the writer lets its lease go once the run's
        // writes are applied. Waiting was phase 1's run waiting on every write it queued (0.23 s of an edit).
        if let Some(reading) = reading {
            store.keep_until_written(reading);
        }
        return Ok(None);
    }
    // Collection works behind the run, as the writer does: on its own connection, beside the waves, which read
    // nothing it writes and wait for none of it (decision 0053). It starts once the manifest is written, which it
    // reads, and the run's lease (which kept collection from what the manifest now names) is let go.
    let path = store.database_path().to_path_buf();
    let thread = rerail_accel::exec::service("rerail-collect", None, move || -> Option<(usize, std::result::Result<String, String>)> {
        crate::code_rule_dsl::logic::profile::behind_the_run();
        let store = match ContentStore::open(&path) {
            Ok(store) => store,
            Err(e) => return Some((0, Err(e.to_string()))),
        };
        store.settle(&key);
        drop(reading);
        match store.corpus_pass_wanted(&policy, now) {
            Ok(true) => {}
            Ok(false) => return None,
            Err(e) => return Some((0, Err(e.to_string()))),
        }
        drop(store);
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
            // The observer's store, with what the passes left live (`observe::collect`).
            let live = store.live_plans()?;
            let swept = crate::content_store::observe::collect(&crate::content_store::observe::store_beside(&path), |plan| live.contains(plan)).map_err(crate::content_store::StoreError::from)?;
            let swept = format!("; the observer's store: {} cursors, {} answers, {} records, {} output directories and {} rows of its own journal dropped", swept.cursors, swept.answers, swept.records, swept.outputs, swept.changes);
            Ok(match kept {
                Some(k) => format!(
                    "{} manifests kept, {} evicted; history {:.1} MB (high water {} MB), protected {:.1} MB; {tapes} tapes, {sources} sources and {plan_rows} memo rows of dead plans deleted{}",
                    k.manifests,
                    k.evicted,
                    k.history_bytes as f64 / 1048576.0,
                    policy.high_water >> 20,
                    k.protected_bytes as f64 / 1048576.0,
                    if idle < 2 { "; more left for later passes" } else { "" }
                ) + &swept,
                None => format!("no decision due; {tapes} tapes, {sources} sources and {plan_rows} memo rows deleted{swept}"),
            })
        })()
        .map_err(|e| e.to_string());
        Some((passes, outcome))
    })
    .map_err(|e| Error::Plan(format!("a collection thread: {e}")))?;
    Ok(Some(Collecting(thread)))
}

/// The key a run's manifest's item is queued under: its root's and plan's.
fn manifest_key(root: &str, plan: &[u8; 32]) -> crate::content_store::write_back::Key {
    crate::content_store::write_back::Key::of(&[b"the run's manifest", root.as_bytes(), plan])
}

/// A run's manifest, as its record holds it (decision 0306).
#[derive(serde::Serialize, serde::Deserialize)]
struct Manifest {
    root: String,
    sources: Vec<crate::content_store::SourceId>,
    grammar_version: i64,
    tape_format: i64,
    plan: [u8; 32],
    also: Vec<[u8; 32]>,
    lane: Option<[u8; 32]>,
    /// The lane's policy set (decision 0302).
    set: Option<[u8; 32]>,
    now: i64,
    relations: Vec<(String, String)>,
}

/// Records a run's manifest and its relations, naming the sources the run's writes stored before it.
fn apply_manifest(cx: &crate::content_store::record::Apply<'_>, body: crate::content_store::record::codec::Body) -> crate::content_store::StoreResult<()> {
    let mut m: Manifest = crate::content_store::record::codec::from_body("the run's manifest", body)?;
    m.sources.extend(cx.store.take_named_sources());
    let new = cx.store.record_corpus_run_policy(&m.root, &m.sources, m.grammar_version, m.tape_format, m.plan, &m.also, m.lane, m.set, m.now)?;
    // A warm run may move an identical manifest's timestamps (`profile::WARM_WRITES_ALLOWED`), never record a new one.
    if new {
        crate::code_rule_dsl::logic::profile::warm_fault("a new manifest recorded", || format!("{}, {} sources", m.root, m.sources.len()));
    }
    cx.store.record_corpus_relations(m.plan, &m.relations)
}

#[cfg(any(test, feature = "test-hooks"))]
pub mod test_store;

#[cfg(any(test, feature = "test-hooks"))]
thread_local! {
    /// A test's own store, so that another test's collection in the shared default store does not evict its rows.
    pub static TEST_TAPE_CACHE: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) };
    /// The thread's next memoised run of a program (not of a phase it reads) stops with an error once its first
    /// entries are published (and written), as a run killed then would: what it leaves is a stopped run's.
    pub static STOP_AFTER_ENTRIES: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}


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
        OFF.call_once(|| {
            // Explicit workload caches use production observer semantics. Ordinary unit
            // tests keep stat-based isolation; RERAIL_JOURNAL still controls benchmarks.
            if std::env::var_os("RERAIL_TAPE_CACHE").is_none() {
                crate::content_store::observe::set_journal(false);
            }
        });
    }
    let store = tape_cache_path().map(|p| crate::content_store::observe::store_beside(&p));
    // Each test thread its own observer, unless the test set its own tape cache: another test's run flushes a shared
    // one's journal, after which it answers for this test's edits until the next flush (decision 0105).
    #[cfg(any(test, feature = "test-hooks"))]
    let store = store.map(|p| {
        // An explicit workload cache keeps production observer semantics; a test's own store is its alone.
        if TEST_TAPE_CACHE.with(|t| t.borrow().is_some())
            || std::env::var_os("RERAIL_TAPE_CACHE").is_some()
            || tape_cache_path().is_some_and(|own| test_store::is_own(&own))
        {
            return p;
        }
        let thread: String = format!("{:?}", std::thread::current().id()).chars().filter(char::is_ascii_digit).collect();
        std::path::PathBuf::from(format!("{}.thread{thread}", p.display()))
    });
    crate::content_store::observe::with(store.as_deref(), f)
}

/// A batch of runs (`tools::run_policies`) is one run for the file observer while it lives: each root is asked of
/// the journal once for the batch, one cookie's wait each, where every policy's run asked again. The batch's runs
/// read the tree as of the batch's first ask; what changes under it meanwhile is named to the next batch.
pub struct ObservedBatch(());

impl ObservedBatch {
    pub fn begin() -> ObservedBatch {
        observe(|o| o.begin());
        ObservedBatch(())
    }
}

impl Drop for ObservedBatch {
    fn drop(&mut self) {
        observe(|o| {
            o.flush();
            o.written();
        });
    }
}

/// Closes a run's connection beside the run (a job on the pool): its close frees what the process kept for it (the bound
/// sources, the engine's relations: `vtab::connection_scope`), which is no step's work. Idle, it holds no lock. Until
/// it is done the file stays open: a run that replaces its files waits for it first (`closes_done`).
fn close_beside(db: Connection) {
    *CLOSING.0.lock().expect("the closes beside") += 1;
    rerail_accel::exec::drop_beside(Closing(Some(db)));
}

/// The connections closing beside runs (`close_beside`), and their end.
static CLOSING: (std::sync::Mutex<usize>, std::sync::Condvar) = (std::sync::Mutex::new(0), std::sync::Condvar::new());

/// A connection closing beside the run: counted until its close is done.
struct Closing(Option<Connection>);

impl Drop for Closing {
    fn drop(&mut self) {
        drop(self.0.take());
        let (n, done) = &CLOSING;
        *n.lock().expect("the closes beside") -= 1;
        done.notify_all();
    }
}

/// Waits until every connection closing beside a run is closed: then no file of a run that ended is open. A file
/// replaced under a connection still closing would be its, and the connection's close deletes its write-ahead log by
/// name: the new store's. On a thread of the pool it runs the pool's jobs meanwhile (a close may be one of them).
pub fn closes_done() {
    let (n, done) = &CLOSING;
    let mut left = n.lock().expect("the closes beside");
    while *left > 0 {
        if rayon::current_thread_index().is_some() {
            drop(left);
            if !matches!(rayon::yield_now(), Some(rayon::Yield::Executed)) {
                std::thread::yield_now();
            }
            left = n.lock().expect("the closes beside");
        } else {
            left = done.wait_timeout(left, std::time::Duration::from_millis(10)).expect("the closes beside").0;
        }
    }
}

/// The store the process's runs use instead of the machine's while one is set (`use_store`): testing mode's
/// (decision 0130), whose runs must not read what other runs left, nor wait behind their writes.
static STORE: std::sync::RwLock<Option<std::path::PathBuf>> = std::sync::RwLock::new(None);

/// While the guard lives, every run of the process (without `RERAIL_TAPE_CACHE`) uses the store at `path`.
pub fn use_store(path: std::path::PathBuf) -> UsedStore {
    let was = STORE.write().expect("the process's store").replace(path);
    UsedStore(was)
}

/// `use_store`'s guard: the store before it comes back when it drops.
pub struct UsedStore(Option<std::path::PathBuf>);

impl Drop for UsedStore {
    fn drop(&mut self) {
        *STORE.write().expect("the process's store") = self.0.take();
    }
}

/// Whether the process names the store its runs use (`RERAIL_TAPE_CACHE`, `use_store`), so a test's runs use that
/// one and not their own copy (`test_store::carried`).
#[cfg(any(test, feature = "test-hooks"))]
fn names_its_store() -> bool {
    std::env::var_os("RERAIL_TAPE_CACHE").is_some() || STORE.read().expect("the process's store").is_some()
}

/// The path `tape_cache` opens (`None`: `RERAIL_TAPE_CACHE=off`).
pub fn tape_cache_path() -> Option<std::path::PathBuf> {
    #[cfg(any(test, feature = "test-hooks"))]
    if let Some(path) = TEST_TAPE_CACHE.with(|p| p.borrow().clone()).or_else(test_store::handed) {
        return Some(path);
    }
    if std::env::var_os("RERAIL_TAPE_CACHE").is_none()
        && let Some(path) = STORE.read().expect("the process's store").clone()
    {
        return Some(path);
    }
    match std::env::var("RERAIL_TAPE_CACHE") {
        Ok(v) if v == "off" => None,
        Ok(v) => Some(std::path::PathBuf::from(v)),
        // A test's own store, a copy of the template (`test_store`); the process's only off a test's thread.
        #[cfg(any(test, feature = "test-hooks"))]
        Err(_) => Some(test_store::own().unwrap_or_else(|| test_tape_cache().clone())),
        #[cfg(not(any(test, feature = "test-hooks")))]
        Err(_) => Some(default_store().clone()),
    }
}

/// The store a run uses when none is named: the project's for this build (`build_store`), the project being the
/// working directory. Every stored key carries the engine's build, so another build's entries are of no use to a run;
/// and another project's runs would leave their atoms and plans in it, and hold its writer.
#[cfg(not(any(test, feature = "test-hooks")))]
fn default_store() -> &'static std::path::PathBuf {
    static PATH: std::sync::LazyLock<std::path::PathBuf> = std::sync::LazyLock::new(|| {
        let cwd = std::env::current_dir().ok().and_then(|d| d.canonicalize().ok()).unwrap_or_default();
        build_store("rerail-stores", Some(&cwd.to_string_lossy()))
    });
    &PATH
}

/// A store of this engine build's in `$TMPDIR/<dir>/`: `<build>.sqlite`, or `<build>-<key>.sqlite` for one `key` (a
/// project) of several. Another build's files there, unused for a day, are removed (with their `-wal`, `-shm` and
/// lock files, which start with the same name).
pub fn build_store(dir: &str, key: Option<&str>) -> std::path::PathBuf {
    use std::hash::{Hash, Hasher};
    let dir = std::env::temp_dir().join(dir);
    let _ = std::fs::create_dir_all(&dir);
    let build = crate::code_rule_dsl::build_ids::engine();
    let build = &build[..build.len().min(16)];
    let day = std::time::Duration::from_secs(24 * 3600);
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let old = entry.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > day);
        if !name.starts_with(build) && old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    match key {
        // The hasher's keys are fixed: within a build, a project's name is the same in every process.
        Some(key) => {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            key.hash(&mut h);
            dir.join(format!("{build}-{:016x}.sqlite", h.finish()))
        }
        None => dir.join(format!("{build}.sqlite")),
    }
}

/// The process's store, for what asks off a test's thread (a test's is its own, `test_store`): the machine's is
/// written by every other run on it, so a test's writes could wait out its busy deadline behind them. Removed when
/// the process exits; one a killed process left is removed by the next test process to start.
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
/// The documents a run with no store reads from which its text tests' groups of automata are determinized: below,
/// they run on the lazy DFA, whose states are built as its strings reach them.
const DENSE_TEXT_DOCS: usize = 64;

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
/// The least a chunk evaluating beside others is sized to (`RERAIL_CHUNK_FLOOR_MB`): fewer chunks at once, not
/// smaller ones, below it (Q74).
const CHUNK_FLOOR_MB: f64 = 64.0;
/// A chunk's growth that sizes the next (Q74): a smaller one is within the pages freed before it, which it reused.
const CHUNK_CREDIBLE_MB: usize = 16;

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
    run_handed(programs, None, None, root, t_facts, out, modes, binding, chunk_nodes, typing, options)
}

/// `run_with_options` of a program already read (`tools::run_program`, which read the file): the run holds that
/// program, and is keyed by its digest (`read_key`), never by a print of it (decision 0249).
#[allow(clippy::too_many_arguments)]
pub fn run_read(
    read: &Arc<crate::code_rule_dsl::logic::program_image::Version>,
    root: &Path,
    out: &Path,
    modes: &str,
    binding: Binding,
    chunk_nodes: usize,
    typing: &Typing,
    options: &serde_json::Value,
) -> Result<Report> {
    run_read_handing(read, None, root, out, modes, binding, chunk_nodes, typing, options)
}

/// `run_read` inside another run that hands it the source texts it read (`sources`, decision 0300): phase 1's.
#[allow(clippy::too_many_arguments)]
pub(super) fn run_read_handing(
    read: &Arc<crate::code_rule_dsl::logic::program_image::Version>,
    sources: Option<&texts::Sources>,
    root: &Path,
    out: &Path,
    modes: &str,
    binding: Binding,
    chunk_nodes: usize,
    typing: &Typing,
    options: &serde_json::Value,
) -> Result<Report> {
    let key = read_key(read)?;
    run_handed(&[key.as_str()], Some(read), sources, root, None, out, modes, binding, chunk_nodes, typing, options)
}

/// A program edited as a tree, as the run is handed it (`run_read`): encoded, until its readers edit its version
/// (decision 0276 step 6).
pub fn handed_version(prog: &front::Program) -> Result<Arc<crate::code_rule_dsl::logic::program_image::Version>> {
    crate::code_rule_dsl::logic::program_image::Builder::of_program(prog).and_then(|b| b.finish()).map(|(v, _)| v).map_err(Error::Plan)
}

/// A handed program's key, in the place of a text: its version's digest (`choices::key`, decision 0173 §7), named so
/// that it reads as no program. Every key a run makes of its texts is made of this.
fn read_key(read: &Arc<crate::code_rule_dsl::logic::program_image::Version>) -> Result<String> {
    let version = super::body::Placed::whole(Arc::clone(read));
    Ok(format!("rerail.read-program {}", crate::content_store::observe::hex(&super::choices::key(&version))))
}

#[allow(clippy::too_many_arguments)]
fn run_handed(
    programs: &[&str],
    read: Option<&Arc<crate::code_rule_dsl::logic::program_image::Version>>,
    sources: Option<&texts::Sources>,
    root: &Path,
    t_facts: Option<&Path>,
    out: &Path,
    modes: &str,
    binding: Binding,
    chunk_nodes: usize,
    typing: &Typing,
    options: &serde_json::Value,
) -> Result<Report> {
    // The corpus root by one name, its canonical one, as each part keyed by it names it (the observer's roots, a
    // `World`'s, the run's reuse and constants keys): a root named by an alias (`/tmp`, `/var` on macOS) was read
    // by the alias and watched by both names, and the run after a cold one wrote the alias's first cursor
    // (`RERAIL_WARM`), the cold run having watched the directory above it, the store's own.
    let root = &std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    // The `RERAIL_*` switches stay as they are until the run ends: a test sets them only while no run reads them.
    let _switches = switches::reading();
    // Upkeep yields to the run (decision 0190 §8): this process's, and other processes' over its store and the cost
    // statistics, stop at their next check, their work left due, until the run ends.
    let store_file = tape_cache_path();
    let costs_file = crate::code_rule_dsl::logic::costs::file();
    let files: Vec<&Path> = store_file.iter().chain(&costs_file).map(std::path::PathBuf::as_path).collect();
    let stopping = crate::code_rule_dsl::logic::profile::span(|| "upkeep stops for the run (the store's collection)".into());
    let mut under_way = Some(rerail_accel::upkeep::run_begins(&files));
    drop(stopping);
    let lifecycle_start = usage();
    // Library and test callers do not pass through the CLI/Node pool setup.
    // Initialize before phase 0 or a parallel read can create rayon's default pool.
    crate::threads::configure(false);
    // Whether the native strata may be placed on the device (`plan::placement`, decision 0095), for this run.
    struct Device(super::placement::Binding);
    impl Drop for Device {
        fn drop(&mut self) {
            super::placement::set_binding(self.0);
        }
    }
    let was = super::placement::binding();
    super::placement::set_device(binding == Binding::Metal);
    let _device = Device(was);
    // The corpus root and the rule library are watched (decision 0105): what the journal says is unchanged under
    // them is not statted.
    let library = crate::code_rule_dsl::logic::rr::library();
    let library = std::fs::canonicalize(&library).unwrap_or(library);
    // Phase 0's root is an empty placeholder (`phase0`): asked up front, the journal waited for a cookie of a busy
    // temporary directory every run (2 s when the event came late). A read under it is watched as it comes
    // (`watch_above`).
    let watched: Vec<std::path::PathBuf> = if phase0::active() { vec![library] } else { vec![root.to_path_buf(), library] };
    let watching = crate::code_rule_dsl::logic::profile::span(|| "watch the corpus and the library".into());
    let before = observe(|o| {
        o.begin();
        o.watch(&watched);
        o.counts
    });
    drop(watching);
    let lifecycle_observed = usage();
    // Phase 0 first: each text's include graph, from `bootstrap.rr`'s run, before anything reads a text (the later
    // reads keep it while its files hold).
    // Resolve `components.rr` only when the program or one of its modules can use
    // it, before the main run takes its execution lock.
    let seeding = crate::code_rule_dsl::logic::profile::span(|| "phase 0: the texts' include graphs".into());
    // Each text read once here (`Texts`): its seed and its graph, its names when the cutoff asks.
    let texts = match read {
        Some(read) => Texts::of_read(programs, read),
        None => Texts::of(programs)?,
    };
    let texts = Texts {
        sources,
        ..texts
    };
    let mut component_seeded = false;
    if !phase0::active() {
        let graphs = &texts.derived;
        // Includes alone do not imply component demand. Inspect each observed
        // module once; comments can conservatively request preparation. Missing
        // or malformed modules remain the parser's error, not a proof of no demand.
        let uses_components = |text: &str| text.contains("cross-memo") || text.contains("rr_cx_");
        let modules: BTreeSet<_> = graphs.iter().flat_map(|g| g.digests.keys()).collect();
        // A handed program, by its own settings and relations: its key is no text to search.
        let read_uses = |v: &Arc<crate::code_rule_dsl::logic::program_image::Version>| {
            use crate::code_rule_dsl::logic::program_image::fields::ProgramFields;
            ProgramFields::settings(&**v).iter().any(|(k, s)| k == "pragma" && s.contains("cross-memo"))
                || v.declared_names().iter().any(|r| r.starts_with("rr_cx_"))
                || v.rules().any(|r| r.body().filter_map(|l| l.atom()).any(|a| v.rel_name(a.rel()).starts_with("rr_cx_")))
        };
        component_seeded = read.map_or_else(|| programs.iter().any(|t| uses_components(t)), read_uses) || modules.into_iter().any(|path| {
            observe(|o| o.read(path)).is_none_or(|(bytes, _)| {
                std::str::from_utf8(&bytes).map_or(true, uses_components)
            })
        });
        if component_seeded {
            if let Ok(components) = crate::code_rule_dsl::logic::rr::library().join("components.rr").canonicalize() {
                phase0::graph(&phase0::Seed::File(components))?;
            }
        }
    }
    drop(seeding);
    let lifecycle_seeded = usage();
    // Phase 0's cutoff (`reuse`): a run whose every read is as it was writes the outputs it wrote.
    let reuse = reuse::wanted(options, typing, t_facts);
    let kept = if reuse { reuse::reused(&texts, root, out, modes, typing, options) } else { Ok(None) };
    let report = match kept {
        // A profiled run that the cutoff answered writes its profile too: the lifecycle below appends to it.
        Ok(Some(report)) if crate::code_rule_dsl::logic::profile::on() && !phase0::active() && RUNS_UNDER_WAY.with(std::cell::Cell::get) == 0 => {
            match crate::code_rule_dsl::logic::profile::directory() {
                Some(dir) => crate::code_rule_dsl::logic::profile::report::write(&dir, "Profile of a corpus run the whole-run cutoff answered: every read as the last run left it, its outputs written again\n")
                    .map_err(|e| Error::Plan(format!("profile: {e}")))
                    .map(|()| report),
                None => Ok(report),
            }
        }
        Ok(Some(report)) => Ok(report),
        Err(e) => Err(e),
        Ok(None) => run_observed(&texts, root, t_facts, out, modes, binding, chunk_nodes, typing, options).and_then(|report| {
            if reuse {
                reuse::record(&texts, root, out, modes, typing, options, &report)?;
            }
            Ok(report)
        }),
    };
    let lifecycle_executed = usage();
    // Phase 0's observations, written once the run has read everything (they may lag the run, never lead it).
    let did = {
        let _s = crate::code_rule_dsl::logic::profile::span(|| "phase 0's observations written".into());
        let did = observe(|o| {
            o.flush();
            // Flush queues the final observations. Waiting before it leaves a cold
            // observer store when the process exits immediately after this run.
            if !phase0::active() { o.written(); }
            o.counts.since(&before)
        });
        // The outermost run's last writes, its whole-run record (`reuse::record`) and this flush, are queued after
        // `run_observed` waited for the observer: wait for them too, or the process ends with them queued, and the
        // next run finds no record (every warm run evaluated again).
        if !phase0::active() && RUNS_UNDER_WAY.with(std::cell::Cell::get) == 0 {
            crate::content_store::observe::written();
        }
        did
    };
    // `RERAIL_OBSERVED`: what the run's observer did, on stderr (the profile is written before this).
    if std::env::var_os("RERAIL_OBSERVED").is_some() {
        eprintln!("rerail: files observed: {did}");
    }
    let lifecycle_written = usage();
    crate::code_rule_dsl::logic::profile::decision("run lifecycle CPU", || {
        let cpu = |a: (f64,f64,f64), b: (f64,f64,f64)| b.0 + b.1 - a.0 - a.1;
        serde_json::json!({"root":root,"componentGraphRequested":component_seeded,"observer":cpu(lifecycle_start,lifecycle_observed),
            "phase0":cpu(lifecycle_observed,lifecycle_seeded),"execution":cpu(lifecycle_seeded,lifecycle_executed),
            "observerFlush":cpu(lifecycle_executed,lifecycle_written)}).to_string()
    });
    // The run is over: upkeep may go on. What the outermost run left due as utility work (the frames its keep
    // decision left unreachable, decision 0227) is done before it returns, unless another run has begun.
    drop(under_way.take());
    if !phase0::active() && RUNS_UNDER_WAY.with(std::cell::Cell::get) == 0 && store_file.is_some() {
        let _s = crate::code_rule_dsl::logic::profile::span(|| "the run's utility upkeep (decision 0227)".into());
        crate::code_rule_dsl::logic::profile::behind(rerail_accel::upkeep::finish_soon);
    }
    if report.is_ok() && crate::code_rule_dsl::logic::profile::on() && !phase0::active()
        && RUNS_UNDER_WAY.with(std::cell::Cell::get) == 0
        && let Some(dir) = crate::code_rule_dsl::logic::profile::directory()
    {
        use std::io::Write as _;
        let cpu = |a: (f64,f64,f64), b: (f64,f64,f64)| b.0 + b.1 - a.0 - a.1;
        // A run the whole-run cutoff answered derived nothing and wrote no report of its own: its spans are the report.
        if !dir.join("profile.txt").exists() {
            std::fs::create_dir_all(&dir).map_err(|e| Error::Plan(format!("profile lifecycle: {e}")))?;
            crate::code_rule_dsl::logic::profile::report::write(&dir, "Profile of a corpus run that derived nothing (the whole-run cutoff answered it)\n\n").map_err(|e| Error::Plan(format!("profile lifecycle: {e}")))?;
        }
        let mut file = std::fs::OpenOptions::new().append(true).open(dir.join("profile.txt")).map_err(|e| Error::Plan(format!("profile lifecycle: {e}")))?;
        writeln!(file, "\n== Whole run lifecycle (process CPU, including workers)\nobserver {:.6} s; include graphs {:.6} s; execution {:.6} s; final observations {:.6} s\nThese intervals partition the run; do not add them to nested span totals.",
            cpu(lifecycle_start,lifecycle_observed),cpu(lifecycle_observed,lifecycle_seeded),
            cpu(lifecycle_seeded,lifecycle_executed),cpu(lifecycle_executed,lifecycle_written))
            .map_err(|e| Error::Plan(format!("profile lifecycle: {e}")))?;
    }
    report.map(|mut r| {
        r.files_observed = did;
        r
    })
}

thread_local! {
    /// Runs under way on this thread: a run started inside another (a phase, the constants, phase 0) is nested.
    static RUNS_UNDER_WAY: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Carries the run's nesting to a job on another thread (`run_state`): a run there is nested in this one.
pub(in crate::code_rule_dsl) fn carried(c: &mut crate::code_rule_dsl::logic::run_state::Capture) {
    c.copy(&["RUNS_UNDER_WAY"], &RUNS_UNDER_WAY);
}

/// Counts a run under way on this thread until dropped.
struct UnderWay;
impl UnderWay {
    fn enter() -> UnderWay {
        RUNS_UNDER_WAY.with(|n| n.set(n.get() + 1));
        UnderWay
    }
}
impl Drop for UnderWay {
    fn drop(&mut self) {
        RUNS_UNDER_WAY.with(|n| n.set(n.get() - 1));
    }
}


/// `programs` with their messages masked (`rr::message_masked`), as the keys read them.
fn masked<'a>(programs: &[&'a str]) -> Vec<std::borrow::Cow<'a, str>> {
    programs.iter().map(|t| crate::code_rule_dsl::logic::rr::message_masked(t)).collect()
}

/// The run's message templates, by (policy, clause): its texts' (`rr::message_templates`), which a program read from
/// the store may predate; then the program's own `.message` settings, by clause, for a finding no text declares.
fn message_templates(programs: &[&str], settings: &[(String, String)]) -> HashMap<(String, String), String> {
    let mut templates: HashMap<(String, String), String> = HashMap::new();
    for (_, text) in settings.iter().filter(|(k, _)| k == crate::code_rule_dsl::logic::rr::lower::findings::MESSAGE) {
        if let Some((clause, template)) = crate::code_rule_dsl::logic::rr::setting_template(text) {
            templates.entry((String::new(), clause)).or_insert(template);
        }
    }
    for text in programs {
        for (policy, clause, template) in crate::code_rule_dsl::logic::rr::message_templates(text) {
            templates.insert((policy, clause), template);
        }
    }
    templates
}

/// `part`'s rows of a finding's relation (`rr::lower`'s `diagnostic_rule`) with their message made from their policy's
/// and clause's template and their last `holes` columns' values (`findings::HOLE`), and those columns dropped: the
/// message's words are the writer's, not the program's (decision 0225 §3).
fn compose_messages(part: &output::Rows, holes: usize, templates: &HashMap<(String, String), String>) -> output::Rows {
    use crate::code_rule_dsl::logic::rr::lower::findings::{message_of, MESSAGE_COLUMN};
    let w = part.width();
    let kept = w.saturating_sub(holes);
    let text = |i: usize, c: usize| match part.cell(i, c) {
        output::Cell::Text(t) => t.to_owned(),
        output::Cell::Int(v) => v.to_string(),
    };
    let mut composed = output::Rows::new(part.types()[..kept].to_vec());
    for i in 0..part.len() {
        let values: Vec<String> = (kept..w).map(|c| text(i, c)).collect();
        let (policy, clause) = (text(i, 0), text(i, 1));
        let message = match templates.get(&(policy, clause.clone())).or_else(|| templates.get(&(String::new(), clause))) {
            Some(t) => message_of(t, &values),
            None => values.concat(),
        };
        let cells: Vec<output::Cell<'_>> = (0..kept).map(|c| if c == MESSAGE_COLUMN { output::Cell::Text(&message) } else { part.cell(i, c) }).collect();
        composed.push(&cells);
    }
    composed
}

#[allow(clippy::too_many_arguments)]
fn run_observed(
    given: &Texts<'_>,
    root: &Path,
    t_facts: Option<&Path>,
    out: &Path,
    modes: &str,
    binding: Binding,
    chunk_nodes: usize,
    typing: &Typing,
    options: &serde_json::Value,
) -> Result<Report> {
    // The run's writes are waves of its own (decision 0259): each sealed at a boundary of the run (`end_wave`), the
    // last as the run ends, and the store takes each in one transaction.
    let _waves = crate::content_store::write_back::run_waves();
    let programs = given.programs;
    // The run's distinct lists (decision 0172): every program it reads from a stored text takes its lists from here,
    // so the lists its programs have in common are made once. It goes with the run.
    let lists = Arc::new(front::Lists::default());
    let process_cpu = || { let u = usage(); u.0 + u.1 };
    let process_started = process_cpu();
    // The store's writer held from the run's start to its end (decision 0102): the handles the run opens and closes on
    // the way (the combined program's) queue their writes and go, and the run's end waits for every one. Declared
    // first, so dropped last.
    let _writer = if cfg!(feature = "tape-cache") { tape_cache_path().map(|p| crate::content_store::write_back::hold(&p)) } else { None };
    // The run's settings, as `env` facts (t21 E4): `<root>/rerail.env`, then the `env` option's `key=value`s (the
    // command line's `--env`).
    let setting = crate::code_rule_dsl::logic::profile::span(|| "the run's settings".into());
    let flags: Vec<String> = options.get("env").and_then(|v| v.as_array()).into_iter().flatten().filter_map(|v| v.as_str().map(str::to_owned)).collect();
    let settings = crate::code_rule_dsl::logic::settings::settings(root, &flags).map_err(Error::Plan)?;
    // The settings are `env` rows, given to each program that declares `env` once it is read (decision 0249): the
    // program read stays the program, and the texts are the files' own.
    let mut added = added::Added::of(&settings, options);
    let read = given.read;
    // The texts as the run's keys read them, messages masked (`texts_digest`, decision 0225 §3), each masked once.
    let masked_texts = masked(programs);
    let masked: &[&str] = &masked_texts.iter().map(|t| t.as_ref()).collect::<Vec<_>>();
    drop(setting);
    // What the run is of, whichever build runs it: its manifest's lane (`record_corpus_run_lane`).
    let lane: [u8; 32] = {
        use sha2::Digest as _;
        let mut h = sha2::Sha256::new();
        h.update(b"rerail.corpus-lane/v2\0");
        for p in programs {
            h.update((p.len() as u64).to_le_bytes());
            h.update(p.as_bytes());
        }
        added.hash(&mut h);
        h.finalize().into()
    };
    // The lane's policy set (decision 0302): the programs' policies by name (`.policy "<name>"`: an edit of a body
    // keeps them) and the run's options (its dialect, settings, phase marks). A run of the set's programs edited,
    // under the same options, supersedes the set's older lanes over the root, whoever runs it (the command line, a
    // session, a batch). Programs that name no policy name no set: the run's lane is its own (`record_run`).
    let policy: Option<[u8; 32]> = {
        let names: std::collections::BTreeSet<&str> = programs
            .iter()
            .flat_map(|t| t.lines().filter_map(|l| l.trim_start().strip_prefix(".policy")?.trim_start().strip_prefix('"')?.split('"').next()))
            .collect();
        (!names.is_empty()).then(|| {
            use sha2::Digest as _;
            let mut h = sha2::Sha256::new();
            h.update(b"rerail.corpus-policy/v2\0");
            for n in &names {
                h.update((n.len() as u64).to_le_bytes());
                h.update(n.as_bytes());
            }
            h.update(options.to_string().as_bytes());
            h.finalize().into()
        })
    };
    // The main thread's own CPU (the chunk builder's jobs' is theirs, below; the jobs the main thread runs for the
    // pool while it waits are not its own, `exec::helped`) and the process's peak RSS.
    let main_usage = || { let (u, s) = thread_usage(); (u - rerail_accel::exec::helped(), s, usage().2) };
    // When the run began: the first phase counts combining.
    let began = main_usage();
    let mut process_last = process_cpu();
    let mut process_phases = vec![("settings", process_last - process_started)];
    let mut phases: Vec<(&'static str, f64, f64, f64, usize)> = Vec::new();
    let mut phase_instructions: Vec<(&'static str, u64)> = Vec::new();
    let mut last = began;
    let mut last_instructions = crate::code_rule_dsl::logic::settings::process_counters().map_or(0, |c| c.0);
    let mut counted = 0;
    let timeline = std::env::var_os("RERAIL_TIMELINE").is_some();
    // A timeline (dev, `RERAIL_TIMELINE`): the run's start, and each section's end as it ends, with the process's CPU
    // it took and the process's instructions retired so far (`i=`, G).
    if timeline {
        eprintln!("RERAIL_TIMELINE {} start: @{:.3} i={:.3}", phases::current().unwrap_or_else(|| "main".into()), crate::code_rule_dsl::logic::profile::timeline_clock(), last_instructions as f64 / 1e9);
    }
    // Decision 0288: each phase's clocks at its bounds; and, for the run that holds the profile, its thread's time by span.
    let mut phase_clock: Vec<(&'static str, crate::code_rule_dsl::logic::profile::PhaseClock)> = Vec::new();
    let mut clock_last = crate::code_rule_dsl::logic::profile::snapshot();
    // The command's time before its run (reading its rules and settings), taken by the command's first run.
    if RUNS_UNDER_WAY.with(std::cell::Cell::get) == 0 && !phase0::active() && let Some(start) = crate::code_rule_dsl::logic::profile::take_command_start() {
        phase_clock.push(("before", crate::code_rule_dsl::logic::profile::PhaseClock::between(&start, &clock_last)));
    }
    let tracking = std::cell::Cell::new(false);
    let mut phase = |name: &'static str, statements: usize| {
        let process_now = process_cpu();
        let clock_now = crate::code_rule_dsl::logic::profile::snapshot();
        let c = crate::code_rule_dsl::logic::profile::PhaseClock::between(&clock_last, &clock_now);
        clock_last = clock_now;
        match phase_clock.iter_mut().find(|p| p.0 == name) {
            Some(p) => p.1.add(&c),
            None => phase_clock.push((name, c)),
        }
        if tracking.get() {
            crate::code_rule_dsl::logic::profile::run_thread_phase(name);
        }
        // The timeline's line for this section's end (above).
        if timeline {
            eprintln!("RERAIL_TIMELINE {} {name}: {:.3} s CPU @{:.3} i={:.3} mb={}/{}", phases::current().unwrap_or_else(|| "main".into()), process_now - process_last, crate::code_rule_dsl::logic::profile::timeline_clock(), crate::code_rule_dsl::logic::profile::retired_instructions() as f64 / 1e9, crate::code_rule_dsl::prof::footprint_mb(), crate::code_rule_dsl::prof::max_footprint_mb());
        }
        process_phases.push((name, process_now - process_last));
        process_last = process_now;
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
    // A run nested in another on this thread (phase 0's, a phase's, the constants') and phase
    // 0's own run hold it only when no run does: nested, the profile is the outer run's, and such a run neither
    // resets nor writes it. Waiting for it would wait for the outer run, which waits for this one. So does a run
    // behind the run on another thread (`profile::behind`: the test store's template, filled while a profiled test
    // holds the profile and waits for it).
    let nested = RUNS_UNDER_WAY.with(std::cell::Cell::get) > 0 || crate::code_rule_dsl::logic::profile::is_behind();
    let _under_way = UnderWay::enter();
    let _stage = crate::code_rule_dsl::logic::profile::run_begins();
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
    // The run's thread charged to its spans (decision 0288): the run that holds the profile, not one nested in it.
    if profiling && !nested && !phase0::active() {
        // Tracked from the command's start (`command_begins`): what came before the run is its phase `before`.
        if crate::code_rule_dsl::logic::profile::tracked() {
            crate::code_rule_dsl::logic::profile::run_thread_phase("before");
        } else {
            crate::code_rule_dsl::logic::profile::track_run_thread();
        }
        tracking.set(true);
    }
    let combining = crate::code_rule_dsl::logic::profile::span(|| "parse the program".into());
    // The combined program as a version (decision 0281's path, step 7a): the readers before preparation read its
    // fields where they are (`fields`), and its rules' shape (`shape::Shape`); an edit of them is a version's edit.
    let (mut v, mut texts) = combine_version(programs, masked, &given.graphs, &added, read, &lists)?;
    let mut shape = shape::Shape::of(&v);
    drop(combining);
    // Earlier phases' exports it reads (`phases`, decision 0083 §1): each phase runs first, in its own fact space.
    // The constants' own run runs no phase: a constant reads no input (`constants::foldable`), and a phase export is
    // one. Run here, a phase took the constants' option for its own, and wrote none of its exports.
    // Phase 1 (`sources.rr`, decision 0104) runs first: it loads the source files and the libraries their imports
    // reach, to the imports' fixed point. Its documents are this run's, and every later phase's run inside it: a phase's
    // own run, phase 0's (another tree) and the constants' (no documents) list none of theirs from it. A run given its
    // files (`inputFiles`, T's facts) has no phase 1.
    let language = v.setting_values("language").into_iter().next().map(|value| value.trim_matches('"').to_owned());
    let mut listing: Option<(Vec<String>, &'static str)> = None;
    let mut phase_one_run = None;
    // The source texts the digests read (decision 0300): this run's builder takes them, and phase 1's, handed them,
    // borrows them.
    let own_sources = texts::Sources::default();
    let source_texts = texts::Given { own: &own_sources, handed: given.sources };
    let phase_one = (|| {
        let prog: &crate::code_rule_dsl::logic::program_image::Version = &v;
        Ok::<_, Error>(if t_facts.is_none()
            && options.get("inputFiles").is_none()
            && phases::declared(prog).is_none()
            && !phases::in_phase()
            && !phase0::active()
            && options.get(constants::OWN).is_none()
            && std::env::var("RERAIL_SOURCES_PHASE").map_or(true, |v| v != "off")
        {
            let found = scoped(corpus_files(root)?, prog, options);
            let label = corpus_language(language.as_deref(), typing, &found);
            let files = found.of(label);
            // The observer's facts: each source file of the run's grammar, its module, and its content's digest (read
            // only where the observer cannot vouch for it).
            // A Python file's module is the name its package root gives it (`package_root_modules`), as imports name it.
            let renamed = package_root_modules(root, &files);
            let of_label: Vec<&String> = files.iter().filter(|p| crate::code_rule_dsl::language::label_of_path(p) == Some(label)).collect();
            // The run's one read of each (decision 0300): the bytes are held for phase 1's and this run's builders.
            let digests = world::bytes_digests_held(&of_label.iter().map(|p| root.join(p)).collect::<Vec<_>>(), Some(&own_sources));
            let stated: Vec<(String, String, String, String)> = of_label
                .into_iter()
                .zip(digests)
                .filter(|(_, c)| !c.is_empty())
                .map(|(p, c)| {
                    let module = renamed.get(p).cloned().unwrap_or_else(|| crate::code_rule_dsl::facts::vtab::syntax::module(p));
                    (p.clone(), module, label.to_owned(), c)
                })
                .collect();
            listing = Some((files, label));
            if stated.is_empty() {
                None
            } else {
                let _s = crate::code_rule_dsl::logic::profile::span(|| "phase 1: the source files".into());
                let (one, run) = phases::sources(&stated, &own_sources, label, root, out, modes, binding, chunk_nodes, typing, options)?;
                let (made, libraries) = (one.documents.len(), one.libraries.len());
                crate::code_rule_dsl::logic::profile::decision("phase 1 (sources.rr)", || {
                    format!("{made} of {} source files are its documents; {libraries} library documents the imports reached", stated.len())
                });
                phase_one_run = Some(run);
                Some(phases::PhaseOneHeld::enter(one))
            }
        } else {
            None
        })
    })();
    let _phase_one = phase_one?;
    let (phased, mut phase_runs) = if options.get(constants::OWN).is_some() {
        (Default::default(), Vec::new())
    } else {
        phases::facts(&*v, &shape, root, out, modes, binding, chunk_nodes, typing, options)?
    };
    phase_runs.splice(0..0, phase_one_run);
    // A phase's exports are the run's inputs, not its facts: their rows are bound as relations once the program is
    // prepared, so an edit that changes them leaves the program, its prepared plans and its keys as they were.
    // The split takes them as constants of the run (`file_local::run_constant`).
    let add_phased = |prog: &mut Edit<'_>| {
        for rel in phased.keys() {
            prog.b.add_input(rel);
        }
    };
    edit_fields(&mut v, |p| Ok(add_phased(p)))?;
    // The Git functors answer from the repository holding this run's root (`strings::git`), for this run only.
    let _git = crate::code_rule_dsl::logic::strings::git::scope(root);
    // The run's constants (`constants`): derived first, over no documents, and given as facts. In the constants' own
    // run, its outputs are those relations.
    let fold_constants = |v: &mut Arc<crate::code_rule_dsl::logic::program_image::Version>, added: &added::Added| -> Result<(Vec<(String, String, String)>, Vec<[u8; 32]>)> {
        match options.get(constants::OWN).and_then(|v| v.as_array()) {
            Some(wanted) => {
                let wanted: Vec<String> = wanted.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect();
                *v = constants::as_outputs_of(v, &wanted)?;
                Ok(Default::default())
            }
            None => {
                let (consulted, plans, _) = constants::fold(v, programs, read, added, root, out, modes, binding, chunk_nodes, typing, options)?;
                Ok((consulted, plans))
            }
        }
    };
    phase("combine", 0);
    let (mut folded, mut constants_plans) = fold_constants(&mut v, &added)?;
    shape = shape::Shape::of(&v);
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
                        let found = scoped(corpus_files(root)?, &*v, options);
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
                        if v.declares("sources__library") {
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
    // A library document is of the grammar phase 1 loaded it in, the run's (`sources.rr`'s `library`): a C run's
    // header (`.h`, C++'s by its suffix) is parsed as C, not read as text.
    let library_paths: std::collections::HashSet<&str> = libraries.iter().map(|(p, _)| p.as_str()).collect();
    let text_document = |path: &str, label: &str| text_document(path, label) && !library_paths.contains(path);
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
        if t.is_none() {
            let _span = crate::code_rule_dsl::logic::profile::span(|| "observe document digests".into());
            let digests = world::bytes_digests(&docs.iter().map(|(_, p)| root.join(p)).collect::<Vec<_>>());
            docs.iter().zip(digests).map(|((_, p), d)| ("doc".to_owned(), p.clone(), d)).collect()
        } else { Vec::new() };
    let mut paths: HashMap<i64, String> = docs.iter().cloned().collect();
    // The documents requests loaded (decision 0065 §2a), whose rows are not the run's findings: their keys.
    let mut loaded_keys: std::collections::HashSet<i64> = std::collections::HashSet::new();
    // The library documents phase 1 handed on are loaded documents of this run, named by their modules.
    let mut given_modules: HashMap<String, Arc<str>> = libraries.iter().map(|(p, m)| (p.clone(), Arc::from(m.as_str()))).collect();
    loaded_keys.extend(docs.iter().filter(|(_, p)| given_modules.contains_key(p)).map(|(d, _)| *d));
    // And the tree's Python files named by their package roots, not their paths (`package_root_modules`): `doc` and
    // `pkg` then agree with the imports that name them. A file's memoised rows hold its `doc` row, so the memo is
    // keyed by these names too (below).
    let renamed_modules: BTreeMap<String, String> = if t.is_none() {
        let tree: Vec<String> = docs.iter().map(|(_, p)| p.clone()).filter(|p| !given_modules.contains_key(p)).collect();
        package_root_modules(root, &tree)
    } else {
        BTreeMap::new()
    };
    given_modules.extend(renamed_modules.iter().map(|(p, m)| (p.clone(), Arc::from(m.as_str()))));
    // Declarations and dependency summaries, as `declared_ret` facts (`corpus::typing`).
    let mut typed = TypingReport::default();
    let mut declared = BTreeMap::new();
    if v.declares("declared_ret") {
        let files: Vec<String> = docs.iter().map(|(_, p)| p.clone()).filter(|p| !text_document(p, label)).collect();
        let (facts, d) = typing::facts(typing, root, &files, &mut typed)?;
        if !facts.is_empty() {
            added = added.with("declared_ret", facts.into_iter().map(|(q, t)| vec![front::Value::Str(q.into()), front::Value::Str(t.into())]));
            (v, texts) = combine_version(programs, masked, &given.graphs, &added, read, &lists)?;
            edit_fields(&mut v, |p| Ok(add_phased(p)))?;
            (folded, constants_plans) = fold_constants(&mut v, &added)?;
            shape = shape::Shape::of(&v);
        }
        declared = d;
    }
    consulted.extend(folded);
    // The plans of the runs nested in this one (`Report::plans`): its phases', its constants' and phase 0's.
    let nested_plans: Vec<[u8; 32]> = {
        let mut p: Vec<[u8; 32]> = phase_runs.iter().flat_map(|r| r.report.plans.iter().copied())
            .chain(constants_plans)
            .chain(given.derived.iter().flat_map(|g| g.plans.iter().copied()))
            .collect();
        p.sort();
        p.dedup();
        p
    };
    // Typing may rebuild the combined program. Publish external facts after that rebuild.
    edit_fields(&mut v, |p| external_inputs(p, &shape, root))?;
    // Withholding rules (`inputs::withhold`) changes what they make.
    shape = shape::Shape::of(&v);
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
    let roots: Vec<crate::code_rule_dsl::logic::program_image::RelId> = v.output_names().iter().filter(|o| !guests::REQUESTS.contains(&o.as_str())).filter_map(|o| v.rel(o)).collect();
    let need = crate::code_rule_dsl::logic::program_image::analyses::needed(&v, &crate::code_rule_dsl::logic::program_image::About::of(&v), roots);
    let reads_guests = v.rules().any(|r| need[r.head().rel().index()] && r.reads().any(|a| guests::READS.contains(&v.rel_name(a.rel()))));
    crate::code_rule_dsl::logic::profile::decision("guest requests", || {
        if reads_guests { "made: a rule the outputs need reads guests".into() } else { "none: no rule the outputs need reads a guest".into() }
    });
    let requests: Vec<&str> = guests::LIBRARY.iter().copied().chain(guests::DEMANDS).filter(|_| reads_guests).chain(world_requests).collect();
    edit_fields(&mut v, |p| {
        for &rel in requests.iter().filter(|rel| shape.made.contains(**rel)) {
            p.b.add_output(rel);
        }
        Ok(())
    })?;
    let dialect = crate::code_rule_dsl::language::dialect_by_label(label).ok_or_else(|| Error::Plan(format!("no dialect {label}")))?;
    if !dialect.built() {
        return Err(Error::Plan(format!("{label} is not in this build (its `lang-{label}` feature is off)")));
    }
    std::fs::create_dir_all(out).map_err(|e| Error::Plan(e.to_string()))?;
    // The outputs are rerail's own: outside the file observer, for this process and the next (decision 0284).
    observe(|o| o.own_output(out));
    // A long-lived process (the browser's engine, `session`) keeps its store, its last program compiled and each
    // file's file-local rows from run to run: an edit evaluates the files it changed and the cross-file part.
    let session = held.as_mut().filter(|_| t.is_none() && !v.declares("declared_ret"));
    // The run's connections, set up by their owner and passed through (`connections`): a session's are kept from run
    // to run, readied here, before the scratch files are made again; a run of its own opens its own. (A run that
    // fails leaves its session new ones to open.)
    let handed = {
        let _span = crate::code_rule_dsl::logic::profile::span(|| "prepare connections".into());
        connections::Handed::take(session.as_ref().map(|s| Arc::clone(&s.connections)))?
    };
    let (path, cross_path) = match &session {
        Some(s) => (s.path.clone(), out.join("cross.sqlite")),
        None => (out.join("store.sqlite"), out.join("cross.sqlite")),
    };
    // A session's store is kept from run to run; the cross-file scratch never is. A failed run leaves its write-ahead
    // log beside the store; reopening a new store over it reads stale pages.
    let scratch = [&path, &cross_path].into_iter().filter(|p| session.is_none() || **p == cross_path);
    // An earlier run's connections may be closing still (`close_beside`): not on the files replaced here.
    closes_done();
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
    let tapes = if cfg!(feature = "tape-cache") {
        let _span = crate::code_rule_dsl::logic::profile::span(|| "open tape store".into());
        handed.tapes().map_err(|e| Error::Plan(e.to_string()))?
    } else { &None };
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
    // The run's guests' keys (decision 0391): the store's, read before the first wave, which find and claim them in
    // memory; without a store, the run's own, shared by its chunks.
    let guest_keys = match tapes.as_ref() {
        Some(cache) => cache.guest_keys().map_err(|e| Error::Plan(e.to_string()))?,
        None => crate::content_store::guest_keys::GuestKeys::in_memory(),
    };
    // `shortlex_min<>` orders the run's symbols by their text: the store's atoms (decision 0108).
    crate::code_rule_dsl::logic::native::declare("shortlex_min", Arc::new(crate::code_rule_dsl::logic::native::ShortlexMin(store.atoms_handle())));
    let db = {
        let _span = crate::code_rule_dsl::logic::profile::span(|| "open run connection".into());
        handed.run(&path)?
    };
    // The run's databases are scratch, made again by every run (removed above): a commit need not reach the disk. Set
    // when SQL first runs on it (`vtab::setup_now`).
    crate::code_rule_dsl::facts::vtab::defer_setup(&db, "PRAGMA synchronous=OFF;");
    super::ready_native(&db)?;
    // The cost statistics kept and written; read when the run first has work to price (decision 0104, 0285).
    crate::code_rule_dsl::logic::costs::start();
    let statements = Statements::on(&db);
    // The program's preparation (kept or read, split, its parts' plans), until the files are parsed.
    statements.enter("prepare");
    // The program as this run prepares it, kept for the process (`Prepared`): a session or a test running one program
    // over many roots prepares it once. The key is what preparing reads besides the store's atoms, which are asked
    // again below.
    // Cross-file rows memoised per import component (decision 0076): the program asks, and its documents' language
    // has import rules in `components.rr`. Not for a discovery round of `world`, which runs part of the program; the
    // final round's world facts are program facts, so they are in the plan's digest.
    let cross_mode = cross_memo::mode();
    // With requests answered inside the fixpoint too: the cone runs first, so every loaded document's exports, the
    // component rules' only reads, are in before the components are (decision 0065 §2a).
    // The copies hiding made for guests in the run's own language are folded where the program is prepared (decision
    // 0058, `FoldOwn`): a function of the combined program and the run's language, which the prepared key names, so a
    // program read prepared is not folded first.
    let loading_run = world::wanted(&*v, &shape);
    let cross_declared = cross_memo::declared(&*v).filter(|_| cross_mode != cross_memo::Mode::Off && label == "python");
    // A program that reads the components' relations itself (`rr_cx_reach`: a closure-local lookup) has them with or
    // without the memo.
    let reads_components = shape.read.iter().any(|rel| rel.starts_with("rr_cx_"));
    if reads_components && label != "python" {
        return Err(Error::Plan(format!("the import components' relations (`rr_cx_*`) are Python's; this run's documents are {label}")));
    }
    if cross_declared.is_some() || reads_components {
        v = cross_memo::add_to(&v)?;
        // The constants' own run outputs the constants alone (`constants::as_outputs`): it runs over no documents, so
        // it makes no components, and its prune drops their rules.
        if options.get(constants::OWN).is_some() {
            edit_fields(&mut v, |p| Ok(p.b.retain_outputs(|o| o != cross_memo::COMPONENT)))?;
        }
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
        // The version's facts, inputs and outputs (its rules are the texts'), as values.
        let facts: Vec<(String, Vec<front::List<front::Value>>)> = v.fact_names().into_iter().map(|rel| {
            let rows = v.fact_values(&rel, &lists).unwrap_or_default();
            (rel, rows)
        }).collect();
        hash.update(format!("{label}\0{facts:?}\0{:?}\0{:?}\0{}", v.input_names(), v.output_names(), cross_declared.is_some()).as_bytes());
        // Whether exports are handed on factorized (decision 0099): another choice is another prepared program, and
        // another file-local part, so another per-file memo key.
        if !factorizing {
            hash.update(b"\0factorize:off");
        }
        // The switches that change what is prepared: a program prepared under one value is not another's. Only
        // these: the key also keys a session's kept frames, so a diagnostic switch turned on mid-session must not
        // make the next run cold.
        for switch in PREPARING {
            hash.update(format!("\0{switch}={}", std::env::var(switch).unwrap_or_default()).as_bytes());
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
    // The texts with their messages masked, as the key is (`texts_digest`, decision 0225 §3: a message's edit finds
    // the program stored), and the rows the run added (as their digest: a key, never read as a program).
    let durable_text = (tapes.is_some() && t.is_none() && session.is_none()).then(|| {
        let mut text = masked.join("\0");
        if !added.is_empty() {
            text.push_str(&format!("\0added {}", crate::content_store::observe::hex(&added.digest())));
        }
        text
    });
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
    // And the keys its maintained parts keep their frames under (the strata's and the cone's, `prepared_key`; the
    // cross part's and the rounds', `part_keys`, as they are made): a part's frames are kept while a kept manifest
    // names its key (decision 0227, `collect_unreachable_memo`), not by the runs that used them.
    if durable.is_some() {
        program_plans.push(prepared_key);
    }
    let part_keys: std::sync::Mutex<Vec<[u8; 32]>> = Default::default();
    // The pipelines' runs the run keeps (decision 0333): the manifest names them, and the next run finds them so.
    let kept_runs: std::sync::Mutex<Vec<[u8; 32]>> = Default::default();
    // The last run's, found through the root's newest manifest, which named them.
    let last_named: Vec<[u8; 32]> = if durable.is_some() {
        let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        store.corpus_run_also(&root.to_string_lossy()).map_err(|e| Error::Plan(e.to_string()))?
    } else {
        Vec::new()
    };
    let last_run = |name: &'static str| -> Option<crate::code_rule_dsl::logic::program_image::manager::Run> {
        use crate::code_rule_dsl::logic::program_image::manager::Runs as _;
        let _opening = crate::code_rule_dsl::logic::profile::span(|| format!("the last run of {name} (decision 0333)"));
        last_named.iter().find_map(|d| crate::code_rule_dsl::logic::program_text::StoredRuns(store).open(&preparation::run_key(name, d), name).ok().flatten())
    };
    // The combined program's entry (`combined`), when the store keeps it: the manifest names it too.
    // Their sources found (or stored) by the store's writer: the manifest, queued after, reads them.
    // The pins the combine made (`combined::record_version`), released once the run's manifest names the entry.
    let _combined_pins = durable.is_some().then(|| combined::Pins::new(store, &texts));
    // And the rule files' entries read before the run (`rr::read_file`, decision 0275), by the outermost run: a nested
    // one (phase 0's) did not read them.
    let rule_files = if durable.is_some() && !nested { combined::take_read_before() } else { Vec::new() };
    let _rule_file_pins: Vec<combined::Pins> = rule_files.iter().map(|k| combined::Pins::new(store, k)).collect();
    if durable.is_some() {
        for key in std::iter::once(&texts).chain(&rule_files) {
            let (text, plan) = combined::named(key);
            memo::named_source(store, text, false, Some(&combined::queued(key)), &late_sources);
            // And its program's text (decision 0155), a source of its own.
            combined::name_program(store, key, &late_sources);
            if !program_plans.contains(&plan) {
                program_plans.push(plan);
            }
        }
    }
    // The plans' whole-program choices (inline or store, demand), kept under the prepared program across processes
    // (`plan::choices`, decision 0090 §5); the manifest names their entries too.
    let _choices = durable.and(tape_cache_path()).map(|path| super::choices::Durable::start(&path, prepared_key, Arc::clone(&late_sources), Arc::clone(&lists)));
    if durable.is_some() {
        let (text, plan) = super::choices::named(&prepared_key);
        memo::named_source(store, text, true, None, &late_sources);
        program_plans.push(plan);
    }
    // Keeps `prepared` in the store (decision 0073): made into facts and encoded here, written by the store's writer
    // in the run's record (decisions 0102, 0306), which names its documents' sources. A later run of this process that
    // holds it in memory asks the store for them once (`documents_in`, below).
    let keep_durably = |prepared: &Arc<Prepared>| {
        let (Some(stored), Some(text)) = (durable, durable_text.clone()) else { return };
        let _keep = crate::code_rule_dsl::logic::profile::span(|| "keep the prepared program in the store (decision 0073), queued".into());
        prepared::record(store, stored, text, &prepared_key, prepared);
    };
    let (kept, from) = match (kept, durable) {
        (Some(p), durable) => {
            // Its texts' sources, which a run that read it from the store names: known where this process wrote or
            // read it in this store; else (its write still queued, or another store's) settled and asked of the
            // store, with no program read. Left out, the manifest of a process's second run differed from its first.
            if let Some(Some(source)) = durable {
                let documents = match p.documents_in(store.database_path()) {
                    Some(d) => d,
                    None => {
                        store.settle(&prepared::queued(&prepared_key));
                        match p.documents_in(store.database_path()) {
                            Some(d) => d,
                            None => {
                                let d = prepared::documents(store, source, &prepared_key).map_err(|e| Error::Plan(e.to_string()))?;
                                p.keep_documents(store.database_path(), &d);
                                d
                            }
                        }
                    }
                };
                program_sources.extend(documents);
            }
            (Some(p), "memory")
        }
        // The store's, under the programs' text; or, where the store has none, the one the engine ships (decision 0264).
        (None, Some(source)) => {
            let _lookup = crate::code_rule_dsl::logic::profile::span(|| "the prepared program, from the store (decision 0073)".into());
            // One this process queued is read once written (`settle`; it is kept in memory, so this is rare).
            store.settle(&prepared::queued(&prepared_key));
            let found = prepared::lookup(store, source, dialect, &prepared_key, &lists).map_err(|e| Error::Plan(e.to_string()))?;
            if std::env::var_os("RERAIL_PREPARED_TRACE").is_some() {
                eprintln!("RERAIL_PREPARED_TRACE ({}) {} {label}: {}", phases::current().unwrap_or_else(|| "main".into()), prepared_key[..6].iter().map(|b| format!("{b:02x}")).collect::<String>(), match &found { None => "not in the store", Some((p, ..)) if !holds(p) => "in the store, its atoms not the store's", Some((_, _, prepared::Found::Moved)) => "read, its atoms moved to the store's", Some(_) => "read" });
            }
            let found = found.filter(|(p, ..)| holds(p));
            (found.map(|(p, documents, how)| {
                p.keep_documents(store.database_path(), &documents);
                program_sources.extend(documents);
                let p = Arc::new(p);
                // Moved to this store's atoms: kept here, so the next run reads it as it is.
                if how == prepared::Found::Moved {
                    keep_durably(&p);
                }
                p
            }), "store")
        }
        (None, _) => (None, ""),
    };
    drop(section.take());
    let reused = kept.is_some();
    let prepared = match kept {
        Some(p) => {
            crate::code_rule_dsl::prof::count("corpus.prepared_reused", 1);
            // The combined program is the prepared one's source only: the run reads the prepared forms from here on,
            // so it is not held through the run (a prepared run moves it into its `Prepared`).
            drop(v);
            if from == "store" {
                keep_prepared(prepared_key, Arc::clone(&p));
            }
            crate::code_rule_dsl::logic::profile::reused(if from == "store" { "store" } else { "memory" }, &p.decided);
            // The pipelines' runs the run that prepared it kept (decision 0333): this run prepares nothing, so it names
            // the last runs of its lane, as that run's manifest named them. Named by the preparing run alone, they made
            // the next run's record differ (Q101), and a manifest recorded without them lost them to the run after.
            if durable.is_some() {
                let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
                for d in store.corpus_lane_also(&root.to_string_lossy(), &lane).map_err(|e| Error::Plan(e.to_string()))? {
                    let sources = |name: &'static str| crate::code_rule_dsl::logic::program_text::run_sources(store, &preparation::run_key(name, &d)).map_err(Error::Plan);
                    if let (Some(front), Some(prepared)) = (sources(preparation::FRONT)?, sources(preparation::PREPARATION)?) {
                        kept_runs.lock().expect("the kept runs").push(d);
                        program_sources.extend(front.into_iter().chain(prepared));
                    }
                }
            }
            p
        }
        None => {
            // What preparing decides is the prepared program's, kept with it (decision 0269).
            let deciding = crate::code_rule_dsl::logic::profile::Deciding::open();
            let mut interned: Vec<(Vec<String>, Vec<u32>)> = Vec::new();
            // The copies hiding made for guests in the run's own language, now that it is known (decision 0058).
            if v.declared_names().iter().any(|r| r.starts_with("rr_in_")) {
                let _fold = crate::code_rule_dsl::logic::profile::span(|| "fold the copies of guests in the run's language".into());
                v = preparation::combining::fold_own_guests(v, label).map_err(Error::Plan)?;
                #[cfg(debug_assertions)]
                {
                    // What the run read of the program before the fold, the same after it.
                    assert_eq!(world::wanted(&*v, &shape::Shape::of(&v)), loading_run, "folding changed whether the run loads");
                    assert!(cross_declared.is_none() || cross_memo::declared(&*v).is_some(), "folding dropped the cross memo's pragma");
                }
            }
            // The prepared program's one image (decision 0281's path, step 6): made here, from the program as the rule
            // texts give it; every pass after it runs on the manager over the version (the functors only CPython gives,
            // the base inputs, the prunes and `simplify`, the reach form, the demand, the text passes, steps 6c–6e; the
            // program lowered for the dialect, `nearest` read from the transducer, the bound check, the file-local joins
            // pushed, the exports factorized and cut to their demand, the rows located, the merges, the inputs the rules
            // read, the rule order), and the run's readers read the version (`Lowered::placed`), the split's parts views
            // of it. A program the image cannot hold is an error, not a run read from the tree.
            let version = v;
            // The functors only CPython gives, the base relations as inputs, the prunes and `simplify` (decision 0038);
            // transitive closures held as each start's bitmap, not as rows (decision 0119); tests a rule negates of the
            // nodes it has bound computed for those nodes only (magic sets); the program's own demand (`.pragma
            // "magic-transform" "r, …"`, decision 0145; `RERAIL_DEMAND=off` computes the listed relations in full); a
            // closure the demand put in a cycle of other relations rows again; the `ast` atoms as the matchers'
            // relations, text tests as the vocabulary's (step 6c).
            let preparing = crate::code_rule_dsl::logic::profile::span(|| "prune, simplify, reach, demand, text".into());
            // By parts (decision 0345, `RERAIL_PREPARE=parts`): each part prepared alone against its interface, kept
            // under its own key, and the parts linked; no pipeline run is kept (0333's P is the other answer to Q18).
            let by_parts = preparation::parts::on();
            let (version, preparation::Front { reaches: (reaches, throughs), demanded, demand, undone, matchers, vocabulary }, front_run) = if by_parts {
                let (v, f, kept) = preparation::parts::front(version, dialect, store, durable.is_some(), &lists).map_err(Error::Plan)?;
                program_sources.extend(kept);
                (v, f, None)
            } else {
                let (v, f, run) = preparation::front(version, dialect, last_run(preparation::FRONT)).map_err(Error::Plan)?;
                (v, f, Some(run))
            };
            drop(preparing);
            // A measurement aid: `RERAIL_FRONT_DUMP=<dir>` writes the program as the passes before lowering leave it,
            // one file per phase and mode (`<phase>.whole`, `<phase>.parts`), to compare the two (decision 0345).
            if let Some(dir) = std::env::var_os("RERAIL_FRONT_DUMP") {
                let name = format!("{}.{}", phases::current().unwrap_or_else(|| if phase0::active() { "phase0".into() } else { "main".into() }), if by_parts { "parts" } else { "whole" });
                let mut rules: Vec<String> = version.rules().map(|r| format!("{}\n", version.show_rule(r))).collect();
                rules.sort();
                let sorted = |l: &[String]| {
                    let mut l = l.to_vec();
                    l.sort();
                    l.join(" ")
                };
                let mut decls: Vec<String> = version.declared_names().into_iter().map(|r| format!("{r} {:?}", version.columns_of(&r).unwrap_or_default())).collect();
                decls.sort();
                rules.push(format!("~inputs {}\n~outputs {}\n~decls {}\n", sorted(&version.input_names()), sorted(&version.output_names()), decls.join(" | ")));
                let mut matched: Vec<String> = matchers.iter().map(|m| format!("{m:?}")).collect();
                matched.sort();
                let demand_shown = demand.as_ref().map(|(l, o)| (sorted(l), o.as_ref().map(|t| (t.magic_rules, sorted(&t.full), t.adornments.len()))));
                rules.push(format!("~reaches {reaches} {throughs}\n~demanded {}\n~undone {}\n~matchers {}\n~demand {demand_shown:?}\n", sorted(&demanded), sorted(&undone), matched.join(" | ")));
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::fs::write(std::path::Path::new(&dir).join(name), rules.concat());
            }
            if reaches + throughs > 0 {
                crate::code_rule_dsl::logic::profile::decision("reach index (decision 0119)", || format!("{reaches} closures and {throughs} relations through them held as bitmaps"));
            }
            if let Some((listed, outcome)) = &demand {
                let listed = listed.join(", ");
                match outcome {
                    Ok(r) => crate::code_rule_dsl::logic::profile::decision("the program's demand (decision 0145)", || format!("{listed}: adornments {}; computed in full where demand would not hold: {}; {} magic rules", r.adornments.join(", "), if r.full_for_negation.is_empty() { "none".to_owned() } else { r.full_for_negation.join(", ") }, r.magic_rules)),
                    Err(e) => crate::code_rule_dsl::logic::profile::decision("the program's demand (decision 0145)", || format!("{listed}: not applied ({e}); computed in full")),
                }
            }
            if !undone.is_empty() {
                crate::code_rule_dsl::logic::profile::decision("reach index (decision 0119)", || format!("{} made rows again after demand, in cycles: {}", undone.len(), undone.join(", ")));
            }
            let lowering = crate::code_rule_dsl::logic::profile::span(|| "lower the program".into());
            // One base, the run's and the store's: the text passes leave an overlay.
            let source = if version.base_bytes().is_some() { version.clone() } else { version.fold().map_err(Error::Plan)? };
            let (version, _, ops) = lower::lower_version(version, dialect, &mut |s| {
                let atoms: Vec<u32> = store.intern_atoms(s).map_err(|e| Error::Plan(e.to_string()))?.into_iter().map(u32::from).collect();
                interned.push((s.iter().map(|x| (*x).to_owned()).collect(), atoms.clone()));
                Ok(atoms)
            })?;
            drop(lowering);
            // Every pass since lowering carries the request record (`Program::set_rules_from`): a relation whose rules
            // no longer match it is ungated in `prune` and the embed slice, which then take every reader's request.
            for (req, readers, rules) in version.asked_against_rules().into_iter().filter(|(_, r, n)| r != n) {
                crate::code_rule_dsl::logic::profile::decision("the request record (decision 0331)", || format!("{req}: {readers} rules recorded, {rules} rules: ungated"));
            }
            let _step = crate::code_rule_dsl::logic::profile::span(|| "the prepared program's passes".into());
            // By parts (decision 0345 stage 2): the rewriting passes per part against its interface, kept per part.
            let (version, found, prepared_run) = if by_parts {
                let (v, f, kept) = preparation::parts::run(version, factorizing, dialect, store, durable.is_some(), &lists, bound).map_err(Error::Plan)?;
                program_sources.extend(kept);
                (v, f, None)
            } else {
                let (v, f, run) = preparation::run(version, factorizing, last_run(preparation::PREPARATION), bound).map_err(Error::Plan)?;
                (v, f, Some(run))
            };
            // Both runs kept behind the run, under one digest of this one's, which its manifest names (decision 0333).
            if durable.is_some()
                && let Some(front_run) = front_run
                && let Some(prepared_run) = prepared_run
            {
                let named = preparation::runs_digest(&front_run);
                kept_runs.lock().expect("the kept runs").push(named);
                // Encoded here, written by the store's writer as data (decision 0306).
                crate::code_rule_dsl::logic::program_text::record_runs(store, &[(preparation::run_key(preparation::FRONT, &named), &front_run), (preparation::run_key(preparation::PREPARATION, &named), &prepared_run)]);
            }
            let sets = found.sets.clone();
            let factorized = found.factorized.clone();
            let pushed = found.pushed.clone();
            let merged = (found.merged, found.shared);
            let located = found.located.clone();
            drop(_step);
            // The domains of the program as it now is: those `lower` found are of the program before the rewrites above
            // (pushed joins, factorized and demanded exports, merged rules), which rename and combine relations
            // (`fl256_P`). Every part takes them from here: the split's, the frames' and the memo's symbol columns
            // (`lower::atom_columns`, decision 0170).
            let domains = lower::domains_of(&version)?;
            let _step = crate::code_rule_dsl::logic::profile::span(|| "split the file-local and cross-file parts".into());
            let split = crate::code_rule_dsl::typing::file_local::programs_of(version.clone()).map_err(Error::Plan)?;
            // The run's program: the image alone; no tree is kept.
            let lowered = lower::Lowered::of_placed(super::body::Placed::whole(version), domains, ops, dialect);
            drop(_step);
            // An attribute's answer by a function of the node's document runs where the documents are (decision 0174
            // §6, `answers_where_documents_are`): in the cross-file part it answers nothing, without an error.
            {
                use crate::code_rule_dsl::logic::rr::lower::{Answer, ON_REQUEST};
                let v = &lowered.version;
                let answered = |rel: &str| v.rules().any(|r| v.rel_name(r.head().rel()) == rel);
                if let Some(a) = ON_REQUEST.iter().flat_map(|r| r.iter()).find(|a| matches!(a.answer, Answer::Function(_)) && answered(a.rel) && !split.file_local.contains(a.rel)) {
                    return Err(Error::Plan(format!("`{}` is answered on request in the cross-file part, which holds no documents to read it from (decision 0174 §6)", a.rel)));
                }
            }
            if let Some(path) = std::env::var_os("RERAIL_SPLIT_DUMP") {
                dump_split(std::path::Path::new(&path), &split);
            }
            // The per-file memo's strata of the file-local part, kept with the prepared program (decision 0246 §4):
            // made before the store's writer is handed it.
            let memo_strata = std::sync::OnceLock::new();
            {
                let _s = crate::code_rule_dsl::logic::profile::span(|| "the per-file memo's strata".into());
                // The local part's version: every split has one (0281 step 4).
                let v = Arc::clone(&split.versions.local);
                let rels: BTreeSet<String> = split.exports.iter().cloned().chain(v.outputs().map(|o| v.text(o).to_owned())).collect();
                let strata = memo::strata(&super::body::Placed::whole(Arc::clone(&v)), &rels).map_err(Error::Plan)?;
                let _ = memo_strata.set((memo::program_digest(&v), strata));
            }
            // One prepared program, shared by the process's cache, the store's writer and this run (each a copy before:
            // three of a 16 k-rule program's four forms at once, about 0.4 GB of the Python pack's peak).
            let decided = deciding.close();
            let prepared = Arc::new(Prepared { source, vocabulary: Arc::new(vocabulary), lowered, sets, matchers, demanded, pushed, factorized, located, merged, split, interned, documents: Default::default(), cone: Default::default(), maintained: Default::default(), parts: Default::default(), decided, memo_strata });
            if std::env::var_os("RERAIL_PREPARED_TRACE").is_some() {
                eprintln!("RERAIL_PREPARED_TRACE ({}) {} {label}: prepared, durable {:?}", phases::current().unwrap_or_else(|| "main".into()), prepared_key[..6].iter().map(|b| format!("{b:02x}")).collect::<String>(), durable.map(|d| d.is_some()));
            }
            keep_prepared(prepared_key, Arc::clone(&prepared));
            keep_durably(&prepared);
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
    let Prepared { source, vocabulary, lowered, sets, matchers, demanded, pushed, factorized, located, merged: (remerged, reshared), split: shared_split, interned: _, documents: _, cone: shared_cone, maintained: shared_maintained, parts: shared_parts, decided: _, memo_strata: shared_strata } = &*prepared;
    let (remerged, reshared) = (*remerged, *reshared);
    // The prepared program's fields besides its rules, read from its source's version (decision 0276 §3).
    let head: &crate::code_rule_dsl::logic::program_image::Version = source;
    let head_outputs = head.output_names();
    let head_inputs = head.input_names();
    let head_settings = ProgramFields::settings(head);
    // The run takes the split's programs apart (`part` below): its own copy, the kept one shared.
    let split = shared_split.clone();
    // The prepared program's fields, read from its version (decision 0281's path, step 4): no reader below reads its
    // tree's declarations.
    let prepared_placed = lowered.placed()?;
    let pf = prepared_placed.fields()?;
    // `RERAIL_DUMP_MATCHERS=<file>`: the rules reading a matcher's relation, and each matcher (a debugging aid), from the prepared program, made or kept.
    if let Ok(path) = std::env::var("RERAIL_DUMP_MATCHERS") {
        let mut text = String::new();
        for rule in source.rules().filter(|r| r.body().any(|l| l.atom().is_some_and(|a| source.rel_name(a.rel()).starts_with(matchers::PREFIX)))) {
            text.push_str(&format!("{}\n", source.show_rule(rule)));
        }
        for (rel, matcher, captures) in matchers.iter() {
            text.push_str(&format!("{rel}\t{captures:?}\t{matcher}\n"));
        }
        let _ = std::fs::write(path, text);
    }
    // A program none of whose rules reads a document runs over none: its file-local rules make only constants (which
    // both parts derive: `file_local::FileLocal::constants`), and the cross-file part reads no base relation but the
    // run's constants (an earlier phase's exports, the options). A later phase reading only an earlier one's exports
    // is such a program: an edit costs it what the exports' delta does, not a pass over every file.
    // The parts' versions (decision 0244), each dropped where the run edits its tree (the cross memo's split, the cone
    // of requests, the run's constants): that part's run makes its image of the tree, as before (t36 item 7 step 9).
    let (local_version, mut cross_version) = (split.versions.local.clone(), split.versions.cross.clone());
    // The parts as the split made them, which the memo's and the components' keys are digests of (decision 0173 §7).
    let local_v = Arc::clone(&local_version);
    let cross_v = Arc::clone(&cross_version);
    // The file-local part's fields, as the split made it: the run reads the part's rules and fields from its version.
    let local_placed = super::body::Placed::whole(Arc::clone(&local_v));
    let local_f = local_placed.fields()?;
    // The base relations' layouts (core.rr's, decision 0361 item 5): the columns they answer are not counted when a
    // chunk's base relations are measured (`sampled_statistics_unanswered`).
    let base_shapes = super::shapes::Shapes::of_version(&local_v);
    let constants_only = {
        // The split's (`Programs::constants`): empty where no document is read, and then so is the file-local part.
        let constant = &split.constants;
        local_v.rules().all(|r| constant.contains(local_v.rel_name(r.head().rel())))
    };
    if t.is_none() && constants_only && split.base.iter().all(|r| crate::code_rule_dsl::typing::file_local::run_constant(head, r)) && !docs.is_empty() {
        crate::code_rule_dsl::logic::profile::decision("documents", || format!("none read: no rule reads a document, so the run is over none of the {} files", docs.len()));
        docs.clear();
    }
    // The text tests' plans compiled once for the process (decision 0075), their groups of automata images (0264):
    // read from the engine's bundle or its build's image store when it or an earlier process determinized them.
    // A group is determinized where that pays: kept in the image store for later runs, or for a run over many
    // documents. A small run with no store searches few strings, and runs a group on the lazy DFA
    // (O-vocabulary-dfa-reuse).
    let compiling = crate::code_rule_dsl::logic::profile::span(|| "compile the text tests".into());
    let groups = crate::code_rule_dsl::logic::rr::ImageDfas(crate::code_rule_dsl::logic::rr::image_store());
    let dense = groups.0.is_some() || docs.len() >= DENSE_TEXT_DOCS;
    let compiled = vocabulary.compile(Some(&groups), dense);
    drop(compiling);
    if let Some(c) = compiled.filter(|c| c.read + c.built + c.lazy > 0) {
        crate::code_rule_dsl::logic::profile::decision("text tests compiled (decision 0075)", || format!("groups of automata: {} read (the bundle, the build's image store or this process), {} determinized, {} on the lazy DFA", c.read, c.built, c.lazy));
    }
    // The run's text tests, each string tested once: the batches' and every program the run evaluates (`Hooks::texts`).
    let vocabulary = Arc::new(vocabulary.again());
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
    let unlocated: Vec<&str> = split.exports.iter().map(String::as_str).chain(local_f.outputs()).filter(|r| split.locations.get(*r).is_none_or(Vec::is_empty)).collect();
    // A file-local part reading the run's options (`file_local::option_input`) is memoised under them too.
    drop(section.take()); section = Some(crate::code_rule_dsl::logic::profile::span(|| "the memo's program and the components' plan".into()));
    // A digest under the rows of the phases' exports among `reads`, which are not in the program's text.
    let under_phased = |digest: [u8; 32], reads: &BTreeSet<&str>| -> [u8; 32] {
        let by_rows: BTreeSet<&str> = reads.iter().copied().filter(|r| phased.contains_key(*r)).collect();
        if by_rows.is_empty() { digest } else { memo::with_rows(digest, by_rows.iter().map(|r| (*r, &phased[*r]))) }
    };
    let memo_local = memo::program_digest(&local_v);
    let memo_program = {
        let digest = memo_local;
        let reads_options = local_v.rules().flat_map(|r| r.reads()).any(|a| crate::code_rule_dsl::typing::file_local::option_input(local_v.rel_name(a.rel())));
        let digest = if reads_options { memo::with_options(digest, options) } else { digest };
        // Under the earlier phases' rows only those rules read that a memoised row depends on (an export's or an
        // output's): a constant the per-file part derives for nothing it keeps (phase 1's keys) does not change a
        // file's entry. Nor does a constant the cross-file part reads (phase 1's `py_own`): its rows name no document,
        // and a file's entry holds none of them; one a per-file rule reads is reached through that rule.
        // The constants are the split's (`Programs::constants`): on the file-local part's relations, the part's own
        // (its rules read no cross-file relation).
        // (Checked against the part's own in a debug build, where the split is made: `file_local::assemble`.)
        let constant = &split.constants;
        let mut kept: BTreeSet<&str> = split.exports.iter().map(String::as_str).chain(local_v.outputs().map(|a| local_v.text(a))).filter(|r| !constant.contains(*r)).collect();
        // What the rules of every kept relation read, reached from the kept ones.
        let mut todo: Vec<&str> = kept.iter().copied().collect();
        let mut reaching: BTreeSet<&str> = BTreeSet::new();
        while let Some(rel) = todo.pop() {
            let Some(id) = local_v.rel(rel) else { continue };
            for rule in local_v.heads(id) {
                for a in rule.reads() {
                    let read = local_v.rel_name(a.rel());
                    reaching.insert(read);
                    if kept.insert(read) {
                        todo.push(read);
                    }
                }
            }
        }
        under_phased(digest, &reaching)
    };
    // Each relation a file's entry holds, by its identity (decision 0225 §2): a rule's edit misses only the relations
    // derived from it, in every file; the others are read from the entry.
    // The tree's projection: `core.rr`'s for a program that includes it, T's for one that declares its base relations
    // itself (decision 0046); relations no rule reads are not made. Made before the memo's identities: an entry's
    // rows are its relations' only under the projection they were made under (`Projection::rows_digest`).
    let read: BTreeSet<String> = prepared_placed.v.rules().flat_map(|r| r.reads().map(|a| prepared_placed.v.rel_name(a.rel()).to_owned()).collect::<Vec<_>>()).collect();
    let projection = super::text_demand::projection(source, read.clone(), &matchers);
    let base_rows = projection.rows_digest();
    let memo_ids = {
        let rels: BTreeSet<String> = split.exports.iter().cloned().chain(local_v.outputs().map(|o| local_v.text(o).to_owned())).collect();
        // The strata are the prepared program's, for its file-local part: made once, kept with it.
        let kept = shared_strata.get().filter(|(digest, _)| *digest == memo_local).map(|(_, s)| s);
        let made;
        let strata = match kept {
            Some(s) => {
                crate::code_rule_dsl::prof::count("corpus.memo_strata_kept", 1);
                s
            }
            None => {
                crate::code_rule_dsl::prof::count(if shared_strata.get().is_some() { "corpus.memo_strata_other_part" } else { "corpus.memo_strata_made" }, 1);
                made = memo::strata(&local_placed, &rels).map_err(Error::Plan)?;
                let _ = shared_strata.set((memo_local, made.clone()));
                &made
            }
        };
        debug_assert!(phased.keys().all(|r| local_v.rel(r).is_none_or(|id| local_v.heads(id).next().is_none())), "an earlier phase's export the file-local part derives");
        strata.identities(options, &phased, &base_rows)
    };
    let memo_space = memo::space_digest(&memo_ids, &local_placed, phases::current().as_deref());
    // The components' stored rows are a function of the phases' exports the cross-file rules read too (P3's
    // `types__attr`, `types__tmap`: a library's stubs, which no component's documents hold), so the plan is digested
    // under those as well. Coarse: an export that changes misses every component, not only those that read the rows
    // that changed.
    let cross_digest = {
        let reads: BTreeSet<&str> = cross_v.rules().flat_map(|r| r.reads()).map(|a| cross_v.rel_name(a.rel())).collect();
        under_phased(memo_program, &reads)
    };
    let cross_plan = match &cross_declared {
        Some(declared) => {
            let (plan, version) = cross_memo::split(&cross_version, &lowered.domains, declared, &cross_digest, &split.same_doc)?;
            if !Arc::ptr_eq(&version, &cross_version) {
                cross_version = version;
            }
            plan
        }
        None => None,
    };
    crate::code_rule_dsl::logic::profile::decision("cross-file memo (decision 0076)", || match (&cross_declared, &cross_plan) {
        (None, _) if cross_memo::declared(head).is_none() => "off: the program declares no `.pragma \"cross-memo\"`".to_owned(),
        (None, _) => format!("off ({})", if cross_mode == cross_memo::Mode::Off { "RERAIL_CROSS_MEMO=off" } else { "no import rules for the run's language" }),
        (Some(_), None) => "off: the component rules did not reach the cross-file part".to_owned(),
        (Some(_), Some(plan)) => format!("{} relations memoised per import component ({}){}", plan.memo.len(), plan.memo.keys().cloned().collect::<Vec<_>>().join(", "), if cross_mode == cross_memo::Mode::Verify { "; verifying: derived afresh and compared" } else { "" }),
    });
    // The file-local outputs, whose memo entries hold lines (recorded for a SQL session below).
    let local_outputs: Vec<String> = local_f.outputs().map(str::to_owned).collect();
    // `declared_ret` rows come from outside the memo's key (`corpus::typing`): a program whose rules still read them
    // after pruning is not memoised (a round of `world` does not read them).
    let reads_declared = source.rules().any(|r| r.body().filter_map(|l| l.atom()).any(|a| source.rel_name(a.rel()) == "declared_ret"));
    // The memo: the session's, in memory, or the tape cache's (decision 0052).
    let memo = match (&session, &tapes) {
        (Some(s), _) => Some(memo::Memo::Held(&s.memo)),
        (None, Some(cache)) if t.is_none() => Some(memo::Memo::Stored(cache)),
        _ => None,
    };
    let memoised = memo.is_some() && t.is_none() && unlocated.is_empty() && !reads_declared && std::env::var("RERAIL_CORPUS_MEMO").map_or(true, |v| v != "off") && options.get("memo").and_then(|v| v.as_str()) != Some("off");
    crate::code_rule_dsl::logic::profile::decision("per-file memo", || match (&located, memoised) {
        (_, true) => format!("each file's file-local rows are memoised under its content, path and the run's plan{}; {} relations given a document column; {}", if session.is_some() { ", in this session's memory" } else { "" }, located.as_ref().map_or(0, Vec::len), match prepared_placed.fields().map(|f| crate::code_rule_dsl::typing::file_local::moved_in(&f)).unwrap_or_default() {
            m if m.is_empty() => "none moved to the cross-file part".to_owned(),
            m => format!("moved to the cross-file part, their rows naming no document (decision 0290): {m:?}"),
        }),
        (Err(blocked), _) => format!("off: rows of {blocked:?} name no document"),
        _ => format!("off ({})", if memo.is_none() { "no persistent store" } else if t.is_some() { "T's facts" } else if unlocated.is_empty() { "typing, RERAIL_CORPUS_MEMO=off or the run's `memo: off`" } else { "a relation without a location column" }),
    });
    // The reference graph's relations, for a program that reads them: its files are summarized as the chunks are
    // built, and linked once they all are (`references`). W's graph is Python's.
    let references = match CROSS_FILE.get() { Some(make) => make(label, head, &read, &docs.iter().map(|(_, p)| p.clone()).filter(|p| !text_document(p, label)).collect::<Vec<_>>())?, None => None };
    let part_of = |version| lowered.with_version(version);
    crate::code_rule_dsl::logic::profile::decision("file-local split", || {
        format!(
            "{} rules run per chunk over its files (file-local); {} rules run once over all files (cross-file); {} relations exported to the cross part, {} base relations kept for it; exports and file-local outputs without a location column: {:?}",
            local_v.rules().count(),
            cross_version.rules().count(),
            split.exports.len(),
            split.base.len(),
            split.exports.iter().map(String::as_str).chain(local_f.outputs().filter(|o| !split.exports.iter().any(|x| x == o))).filter(|e| split.locations.get(*e).is_none_or(Vec::is_empty)).collect::<BTreeSet<_>>()
        )
    });
    crate::code_rule_dsl::logic::profile::decision("cross-file causes", || {
        // Root causes only: a relation cross-file because it reads another is listed under that one.
        let causes = &split.causes;
        let mut roots: Vec<String> = causes.iter().filter(|(_, why)| !why.starts_with("reads cross-file ")).map(|(rel, why)| format!("{rel}: {why}")).collect();
        let total = roots.len();
        roots.truncate(60);
        format!("{total} relations cross-file of their own ({} by reading them): {}", causes.len() - total, roots.join("; "))
    });
    crate::code_rule_dsl::logic::profile::decision("nearest ancestors", || {
        let names: Vec<&str> = sets.iter().map(|(n, _)| n.as_str()).collect();
        format!("{} sets, each made per looked-up node, or for the batch by the transducer's file-local FIX (bound {binding:?}) where the planner expects more lookups than that costs (decision 0143): {}", sets.len(), names.join(", "))
    });
    // Requests answered inside the fixpoint (decision 0065 §2a): the cross-file part's cone of requests runs first,
    // pausing for answers. A load's answer is the loaded document's rows of every per-file relation the cone reads,
    // made by the chunk path as any file's are; the rest of the cross-file part reads the cone's rows as inputs.
    drop(section.take()); section = Some(crate::code_rule_dsl::logic::profile::span(|| "the cone of requests".into()));
    let loading: Option<(Arc<crate::code_rule_dsl::logic::program_image::Version>, BTreeSet<String>)> = if loading_run {
        // The prepared split's cone, made once; a run whose cross memo split the cross-file part (above) makes its own.
        // Each of the cross-file part's version.
        let cone = match cross_declared.is_none() {
            true => shared_cone
                .get_or_init(|| cone_of(&super::body::Placed::whole(shared_split.versions.cross.clone()), &split.exports, &split.base, head).map(Arc::new).map_err(|e| e.to_string()))
                .clone()
                .map_err(Error::Plan)?,
            false => Arc::new(cone_of(&super::body::Placed::whole(cross_version.clone()), &split.exports, &split.base, head)?),
        };
        cross_version = cone.rest.clone();
        Some((cone.load.clone(), cone.reads.clone()))
    } else {
        None
    };
    // Each output the run writes, other than what the requests' cone makes (what it asks), a union of file-local
    // rows: every rule of one made in the cross-file part one atom of a file-local export.
    // Read from the parts' versions: the cross-file part's as the run made it.
    let cross_now = Arc::clone(&cross_version);
    let load_heads: BTreeSet<String> = match &loading {
        Some((v, ..)) => v.rules().map(|r| v.rel_name(r.head().rel()).to_owned()).collect(),
        None => BTreeSet::new(),
    };
    let union_of_exports = |r: crate::code_rule_dsl::logic::program_image::RuleRef<'_>| {
        let mut body = r.body();
        match (body.next().map(|l| l.view()), body.next()) {
            (Some(crate::code_rule_dsl::logic::program_image::Lit::Atom(a)), None) => !a.neg() && split.exports.iter().any(|x| x == cross_now.rel_name(a.rel())),
            _ => false,
        }
    };
    let cross_outputs: BTreeSet<&str> = cross_now.outputs().map(|a| cross_now.text(a)).collect();
    let not_per_document = head_outputs
        .iter()
        .filter(|o| cross_outputs.contains(o.as_str()) && !load_heads.contains(o.as_str()))
        .find(|o| cross_now.rel(o).is_some_and(|id| !cross_now.heads(id).all(&union_of_exports)))
        .cloned();
    let (mut local, mut cross) = (part_of(local_version), part_of(cross_version));
    // The run's constants made of earlier phases' exports (`v_enum(q, n) :- struct__base(q, _, b), …, n = count :
    // { struct__member(q, x, "") }`): one answer for the run, so derived once, before the chunks, by the same engine
    // over the bound exports, and bound beside them (`run_constants`). The split derives a constant in both parts
    // (`file_local::FileLocal::constants`), which for one of the program's facts costs little; for one of a phase's
    // exports, every chunk read them all again: over the workspace, 116.6 M rows of `struct__member` for 36 k
    // answers, 183 s of a 966 s run. The memo's key is made before (`memo_program`): it holds the rules as written.
    // The split's (`Programs::constants`): empty only where no document is read, and then the file-local part has no
    // rules for it to filter.
    let constant = &split.constants;
    let of_phase = |r: &str| phased.contains_key(r);
    // Those whose rules read an export, or another such: to a fixpoint. An output stays where the split put it.
    let mut once: BTreeSet<String> = BTreeSet::new();
    {
        // Read from the file-local part's version: the split's, which `local` is.
        loop {
            let more: BTreeSet<String> = local_v
                .rules()
                .map(|r| (local_v.rel_name(r.head().rel()), r))
                .filter(|(h, _)| constant.contains(*h) && !once.contains(*h) && !head_outputs.iter().any(|x| x == h))
                .filter(|(_, r)| r.reads().any(|a| of_phase(local_v.rel_name(a.rel())) || once.contains(local_v.rel_name(a.rel()))))
                .map(|(h, _)| h.to_owned())
                .collect();
            if more.is_empty() {
                break;
            }
            once.extend(more);
        }
    }
    // The parts less the constants are the prepared split's, made once for each set of constants (`Prepared::parts`):
    // a later run shares them, and the trees their readers decode. A run whose cross memo split the cross-file part
    // makes its own.
    let parts_kept = cross_declared.is_none();
    let kept_parts = parts_kept.then(|| shared_parts.get().filter(|(of, _)| *of == once).map(|(_, p)| p.clone())).flatten();
    let once_made = once.clone();
    let run_constants: Option<(lower::Lowered, Vec<String>)> = if let Some((l, c, constants)) = kept_parts {
        (local, cross) = (l, c);
        constants
    } else {
        // Their rules, and those of the constants they read.
        let mut cone = once.clone();
        loop {
            let more: BTreeSet<String> = local_v
                .rules()
                .filter(|r| cone.contains(local_v.rel_name(r.head().rel())))
                .flat_map(|r| r.reads().map(|a| local_v.rel_name(a.rel()).to_owned()))
                .filter(|rel| constant.contains(rel.as_str()) && !cone.contains(rel.as_str()))
                .collect();
            if more.is_empty() {
                break;
            }
            cone.extend(more);
        }
        (!once.is_empty()).then(|| -> Result<_> {
            // Each a part of its version, by the same relations (decision 0244): the constants' of the file-local part's,
            // its rules, gates and lattices those of `cone`; the parts', their rules less those of `once`.
            use crate::code_rule_dsl::logic::program_image::PartFields;
            let (in_cone, not_once, all) = (|rel: &str| cone.contains(rel), |rel: &str| !once.contains(rel), |_: &str| true);
            let outputs: Vec<String> = once.iter().cloned().collect();
            let v = &local_v;
            let inputs: Vec<String> = v.inputs().map(|a| v.text(a).to_owned()).collect();
            let made = v.part(&in_cone, &inputs, &outputs, PartFields { natives: &all, lattice: &in_cone, gates: &in_cone }).map_err(Error::Plan)?;
            for part in [&mut local, &mut cross] {
                let v = part.version.clone();
                let mut inputs: Vec<String> = v.inputs().map(|a| v.text(a).to_owned()).collect();
                for rel in &once {
                    if !inputs.contains(rel) {
                        inputs.push(rel.clone());
                    }
                }
                let outputs: Vec<String> = v.outputs().map(|a| v.text(a).to_owned()).collect();
                let kept = v.part(&not_once, &inputs, &outputs, PartFields { natives: &all, lattice: &all, gates: &all }).map_err(Error::Plan)?;
                part.version = kept;
            }
            crate::code_rule_dsl::logic::profile::decision("constants of the phases' exports", || {
                format!("{} relations derived once, before the chunks, and bound beside the exports: {}", once.len(), once.iter().cloned().collect::<Vec<_>>().join(", "))
            });
            Ok((part_of(made), once.into_iter().collect()))
        }).transpose()?
    };
    if parts_kept && shared_parts.get().is_none() {
        let _ = shared_parts.set((once_made, (local.clone(), cross.clone(), run_constants.clone())));
    }
    if crate::code_rule_dsl::logic::version_trace::on() {
        crate::code_rule_dsl::logic::version_trace::note_versions("run.local", &shared_split.versions.local, &local.version);
        crate::code_rule_dsl::logic::version_trace::note_versions("run.cross", &shared_split.versions.cross, &cross.version);
    }
    drop(section.take()); section = Some(crate::code_rule_dsl::logic::profile::span(|| "guests and the tree's projection".into()));
    let slice = embed_slice(&prepared_placed.v)?.map(part_of);
    // The embed slice's requests' identities (decision 0225 §2), apart from the file-local part's: an entry holds a
    // file's requests under them (`memo::GUESTS`).
    let slice_ids: BTreeMap<String, [u8; 32]> = match &slice {
        Some(s) => {
            let rels: BTreeSet<String> = guests::LIBRARY.iter().chain(&guests::DEMANDS).map(|r| (*r).to_owned()).collect();
            memo::identities(&s.placed()?, &rels, options, &phased, &base_rows).map_err(Error::Plan)?.into_iter().map(|(r, id)| {
                use sha2::{Digest as _, Sha256};
                let mut h = Sha256::new();
                h.update(b"rerail.embed-slice-identity/v1\0");
                h.update(id);
                (r, h.finalize().into())
            }).collect()
        }
        None => BTreeMap::new(),
    };
    let slice_reads: BTreeSet<String> = slice.iter().map(|s| &s.version).flat_map(|v| v.rules().flat_map(|r| r.reads().map(|a| v.rel_name(a.rel()).to_owned()).collect::<Vec<_>>()).collect::<Vec<_>>()).collect();
    crate::code_rule_dsl::logic::profile::decision("guests", || match &slice {
        Some(s) => format!("an embed slice of {} rules runs per chunk, up to {} rounds, before the file-local part (decision 0055)", s.version.size(), guests::DEPTH),
        None => "no rule asks for a guest".into(),
    });
    crate::code_rule_dsl::logic::profile::decision("base relations", || format!("{:?} shape (decision 0046)", projection.shape));
    // A run that reads nothing of its documents builds none: every relation its outputs need, through any rule, is
    // derived, given as facts (an earlier phase's exports, the run's constants) or a native function. A main
    // program over phase exports alone (`defs(q, k) :- struct__def(q, k)`) built and projected every document
    // for nothing: a third of the documents' cost of struct and values run with such a program. Anything else that
    // might read them (T's facts, requests' loads, guests, text or AST patterns) keeps them.
    {
        // Read from the prepared program's version; its text and AST patterns from its source's fields (`head`), as
        // the run's projection reads them.
        let p = head;
        let not_of_documents = |rel: &str| pf.derives(rel) || pf.facts(rel).is_some() || pf.native(rel).is_some() || phased.contains_key(rel);
        let needed = crate::code_rule_dsl::logic::program_image::manager::analysis::<crate::code_rule_dsl::logic::program_image::analyses::Needed>(&prepared_placed.v).map_err(Error::Plan)?;
        // The outputs by name too, as `front::needed` has its roots (one no relation names is still read).
        let needs: BTreeSet<&str> = needed.0.iter().map(|r| prepared_placed.v.rel_name(r)).chain(pf.outputs()).collect();
        let why: Option<String> = if t.is_some() {
            Some("T's facts".into())
        } else if loading.is_some() {
            Some("requests' loads".into())
        } else if slice.is_some() || reads_guests {
            Some("guests".into())
        } else if !p.rx_spans().map_err(Error::Plan)?.is_empty() || !p.ast_patterns().map_err(Error::Plan)?.is_empty() {
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
            super::Stats::of_schema(&db, &prepared_placed, "s")?
        }
        None => {
            // A document's nodes are `d << 32 | index`: the SQL strata's range reads compute its interval (no table).
            super::docs_by_shift(&db);
            super::Stats::empty_of(&pf)
        }
    };
    let own = t.is_none();
    crate::code_rule_dsl::logic::profile::decision("statistics", || {
        if own { "the first chunk measures its sources' statistics; later chunks scale them by their size (a relation first seen later is measured then); the planner prices each chunk's plans from them".into() } else { "T's corpus statistics, read once, with the first chunk's sources' own, scaled to each chunk".into() }
    });
    if let Some(n) = std::env::var("RERAIL_SAMPLE_EVERY").ok().filter(|n| n != "1") {
        crate::code_rule_dsl::logic::profile::decision("sampling", || format!("every {n}th file only (RERAIL_SAMPLE_EVERY): findings are a sample's"));
    }
    let constants = Own::constants_of(&prepared_placed.v);
    let mut own_totals = Own::default();
    // Measured once per mix (`Own::measure` and `syntax::statistics` read every row, sorting every column): the
    // first chunk of a mix, with its nodes or rows, scaled to each later chunk of that mix. A chunk's mix is the file
    // types holding a tenth of its nodes or more (`chunk_mix`), so a chunk of TypeScript after chunks of Python is
    // measured again, and a stray file is not a new mix.
    let mut own_once: HashMap<String, (usize, Own)> = HashMap::new();
    // The guest relations' postings per mix, measured after the guest rounds (`guests::READS`).
    // Each with the chunk that measured it (Q88, `Once`).
    let guest_postings_once: HashMap<String, Once<BTreeMap<(String, String, i64), i64>>> = HashMap::new();
    let measured_once: HashMap<(String, String), Once<Vec<(String, i64, i64, i64, i64)>>> = HashMap::new();
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
    // The run's cross-file part as a version: what the run reads of it below.
    let cross_run = cross.version.clone();
    // The run's cross part as a version: what the run reads of it, not its tree (t36 item 7).
    let run_placed = super::body::Placed::whole(Arc::clone(&cross_run));
    let run_f = run_placed.fields()?;
    let decodes = cross_run.outputs().any(|o| cross_run.text(o) == "finding");
    let mut stored: Vec<(String, Vec<String>)> = Vec::new();
    // The repository inputs (`facts::repository`) the program reads: made once, bound once beside every chunk's.
    let repository: Vec<(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, Vec<(String, i64, i64, i64, i64)>)> = {
        let files: Vec<String> = docs.iter().map(|(_, p)| p.clone()).collect();
        let reads = |r: &str| read.contains(r) && head_inputs.iter().any(|i| i == r);
        crate::code_rule_dsl::facts::repository::relations(&store, root, &files, &crate::code_rule_dsl::facts::repository::with_declared(&head_settings, options), &reads)
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
    // The phases' exports the program reads, as relations of atoms (`add_phased`).
    let repository = {
        let mut repository = repository;
        let phased_read: BTreeSet<&String> = phased.keys().filter(|rel| read.contains(*rel)).collect();
        for rel in phased_read {
            let width = head.columns_of(rel).expect("a phase's export the program declares").len();
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
        let atoms = || cross_run.rules().flat_map(|r| r.reads()).filter(|a| cross_run.rel_name(a.rel()) == rel.as_str());
        let mut named: Vec<&str> = cols
            .iter()
            .enumerate()
            .filter(|(i, c)| decode.contains(c) || atoms().any(|a| a.args().nth(*i).is_some_and(|t| !matches!(t, crate::code_rule_dsl::logic::program_image::TermRef::Wild))))
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
    let memo_tree_stats = !stored.is_empty() || cross_run.rules().flat_map(|r| r.body())
        .any(|l| matches!(l.view(), crate::code_rule_dsl::logic::program_image::Lit::Nearest { .. }));
    let memo_rules = memo_program;
    // The whole file-local program's key: of what is kept of every file together (the kept statistics, the tree
    // counts, a session's inputs), which an edit of any rule changes. Under the plan the entries are kept under
    // (`plan_digest`: the engine build, the grammar, the node ids' scheme), as what it keeps counts those entries:
    // another build's keeper counted entries this build's plan does not hold, so it could not count a file out
    // (Q71), and its counts are of rows this build may make otherwise.
    let memo_whole = memo::plan_digest(&memo::with_modules(memo::with_tree_demand(memo_program, memo_tree_stats), &renamed_modules));
    // The frames' space: the plan without the program (decision 0225 §2), so an entry made before a rule's edit is
    // read for the relations the edit did not reach (`memo_ids`).
    let memo_program = memo::with_modules(memo::with_tree_demand(memo_space, memo_tree_stats), &renamed_modules);
    // The memo plan's parts: a plan that differs between two runs of the same tree misses every entry.
    if std::env::var_os("RERAIL_STATS_KEEP_TRACE").is_some() {
        let hex = |d: &[u8; 32]| d[..4].iter().map(|b| format!("{b:02x}")).collect::<String>();
        eprintln!(
            "rerail: memo plan {} of {}: rules {} (local {}, {} phased relations read), tree demand {memo_tree_stats}, {} modules renamed",
            hex(&memo::plan_digest(&memo_program)), phases::current().as_deref().unwrap_or("the program"), hex(&memo_rules), hex(&memo_local), phased.len(), renamed_modules.len()
        );
    }
    // Its entries' symbol columns, for a store this process forks from (decision 0170): carried with its key.
    // (`lowered.domains`: of the program as prepared, after its rewrites.)
    let memo_plan = memo::Program::new(memo_program, &lowered.domains);
    // No hold on the run's plans or the sources it reads (decision 0306 step 4): its collection lease (`reading`) keeps
    // every collector off the store until its record, with the manifest naming them, is applied.
    // A run that reads the reference graph is not memoised: the graph needs every file's facts
    // (`references::Files::add`), which a memo hit does not build.
    let memoised = memoised && references.is_none();
    // The base relations a chunk keeps only for the nodes its exports hold (`file_local::semijoin_bases`): read by
    // the cross part only as lookups of those nodes.
    let semijoin: BTreeSet<String> = crate::code_rule_dsl::typing::file_local::semijoin_bases(&cross_run, &split.base)
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
    // The exports the chunks hand on, each row once, held in memory for the cross-file part to read in place
    // (`exports`). Was: a table of `x` per export, filled by SQL and read back.
    let exports = std::cell::RefCell::new(exports::Exports::default());
    drop(section.take());
    phase("prepare", statements.count());
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
    // The atoms' lease, held to the run's end: on a new store, a string not in memory is new (decision 0104). Taken
    // before the run's first engine run (the constants below), so no engine run's end flushes the symbols it made: the
    // store's writer writes them behind the run. Taken after, a phase that ran past `DUE_AFTER` (a loaded machine) had
    // the constants' run write its symbols ahead of the waves, which the strict mode refuses.
    let complete = {
        let _s = crate::code_rule_dsl::logic::profile::span(|| "the atoms' lease (decision 0104)".into());
        store.atoms_handle().ready_for_run().map_err(|e| Error::Plan(e.to_string()))?
    };
    let mapped = store.atoms_handle().forked();
    crate::code_rule_dsl::logic::profile::decision("atoms before the first wave (decision 0104)", || match (complete, mapped) {
        (true, false) => "the store's atom blocks mapped to their committed end (decision 0401): lookups probe them where they lie".into(),
        (true, true) => "forked from the store's (decision 0170): the atoms the run's stored rows name are found in its mapped blocks as they are read".into(),
        (false, _) => "inside another run's waves: the blocks mapped as they were".into(),
    });
    let mut repository = repository;
    if let Some((consts, rels)) = &run_constants {
        let _span = crate::code_rule_dsl::logic::profile::span(|| "derive the constants of the phases' exports".into());
        let mut stats = t_stats.clone();
        for (rel, _, measured) in &repository {
            stats.measured(rel, measured.iter().map(|(c, rows, ndv, sq, sqd)| (c.clone(), [*rows, *ndv, *sq, *sqd])), []);
        }
        let got = texts_run(consts, &db, stats, rels.iter().cloned().collect(), modes, &vocabulary)?;
        for rel in rels {
            let width = pf.width(rel);
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
    // Decision 0458: the documents the cross part reads its base relations off, where it reads any: the evaluated
    // chunks' (`Evaluated::documents`, their guests' with them), and the memo hits', each made from its stored tape
    // once the chunks are done (`CorpusHit`).
    let mut corpus_docs: Vec<syntax::Document> = Vec::new();
    let mut corpus_hits: Vec<CorpusHit> = Vec::new();
    // The base rows as sources, once made (`columns_source`): measured and bound as one, so they keep their statistics.
    let mut base_sources: BTreeMap<String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>> = BTreeMap::new();
    // Their digests, when a session's run took them (`retain::kept_source`).
    let mut input_digests: HashMap<String, u64> = HashMap::new();
    // A semi-join's base relations' statistics (`semijoin`): no keeper counts them (it counts every row, the relation
    // holds those its exports name), so each run measured them, sorting every column (8 sorts of 430k-460k rows on
    // an edit of ep2). Measured once at their rows, they are kept beside the input's frame (`write_input_kept`)
    // and read with it on the kept route (`kept_inputs`): by relation, the rows' digest and the statistics.
    let mut kept_input_stats: HashMap<String, (u64, Vec<(String, i64, i64, i64, i64)>)> = HashMap::new();
    let mut semijoin_stats: HashMap<String, Vec<(String, i64, i64, i64, i64)>> = HashMap::new();
    // The atoms the inputs' digests read their content by (`retain::cells_of`, decision 0212).
    let input_atoms = store.atoms_handle();
    // The values the program tests, of any relation and column: per value, the totals count only those (and every
    // value of a column of few, `ValTotals`).
    let tested_values: rustc_hash::FxHashSet<i64> = constants_tested_in(&prepared_placed.v).into_values().flatten().map(|(_, v)| v).collect();
    let mut totals = Totals::new(tested_values.clone());
    let (mut nodes, mut native, mut chunks) = (0, BTreeSet::new(), 0);
    let (mut memo_hits, mut memo_unattributed) = (0usize, 0usize);
    // Why a chunk's rows were not memoised: the relation, and what its row named.
    let mut memo_why: BTreeSet<String> = BTreeSet::new();
    // Each output's parts, as the runs handed them (decision 0239): rows, shared, never copied or printed.
    let mut outputs: BTreeMap<String, Vec<Arc<output::Rows>>> = BTreeMap::new();
    let mut node_columns: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let gather = |got: Outcome, outputs: &mut BTreeMap<String, Vec<Arc<output::Rows>>>, node_columns: &mut BTreeMap<String, Vec<usize>>, keep: &dyn Fn(&str) -> bool| {
        let _gather = crate::code_rule_dsl::logic::profile::span(|| "gather outputs".into());
        for (rel, rows) in got.outputs {
            if keep(&rel) {
                outputs.entry(rel.clone()).or_default().push(rows);
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
    type Chunk = (usize, Vec<(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)>, Vec<(i64, String, Option<memo::Content>, u64, usize, Option<[u8; 32]>)>, Vec<(String, memo::Entry, Option<memo::Content>, Option<Arc<str>>)>, Vec<syntax::Document>);
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
    let mut language_asked: Vec<(String, String)> = Vec::new();
    #[cfg(target_family = "wasm")]
    if docs_out.is_some() {
        return Err(Error::Plan("requests answered inside the fixpoint need threads (decision 0065 §2a)".to_owned()));
    }
    // A session's base relations' statistics, kept by file across its runs (decision 0096 §3): each with every column
    // `input_columns` names. `RERAIL_STATS_KEEP=off` measures them as before.
    // Without a session, the tape cache's memo keeps them: read from the blob the last run of this plan wrote.
    let keeping = memoised && !stored.is_empty() && std::env::var("RERAIL_STATS_KEEP").map_or(true, |v| v != "off");
    // The space's last program (`memo::last_program`) is this run's already: "none" is marked only before the run's
    // first entry is written, and the program again at its end only then. A run that writes no entry writes neither
    // (decision 0250; the run after a cold one wrote both).
    // Read once, before the first wave (decision 0104), with the kept statistics.
    let last_before = match memo {
        Some(memo::Memo::Stored(store)) if memoised => memo::last_program(store, &memo_program),
        _ => None,
    };
    let last_unmarked = std::cell::Cell::new(last_before == Some(memo_whole));
    let mut keeper: Option<keptstats::Keeper> = match (session.as_ref(), memo) {
        _ if !keeping => None,
        // Kept under the whole program, and valid while it is the last whose entries the memo's space holds: another
        // program's run since rewrote them (decision 0225 §2).
        (Some(s), _) => Some(s.stats.lock().expect("the session's statistics").remove(&memo_program).filter(|(whole, _)| *whole == memo_whole).map(|(_, k)| k).unwrap_or_default()),
        (None, Some(memo::Memo::Stored(store))) => {
            let _s = crate::code_rule_dsl::logic::profile::span(|| "read the kept statistics".into());
            let last = last_before;
            // Unknown until this run ends: a run that stops before leaves the next one counting every file in.
            if !last_unmarked.get() {
                memo::set_last_program(store, &memo_program, None);
            }
            // The last program's, whichever it was: a relation a rule's edit reached since is counted again from
            // scratch, the others are as counted (`Keeper::begin`, by identity; decision 0225 §2).
            let mut k = last.and_then(|w| store.stat_blob(w).ok().flatten()).and_then(|b| keptstats::Keeper::decode(&b)).unwrap_or_default();
            // Its constants are the store's atoms: this process's, mapped (decision 0170), in one call.
            let atoms = store.atoms_handle();
            if atoms.holds_by_reading() {
                if k.map_symbols(&|key, names| memo_plan.syms.columns(key, names.len()), atoms.forked(), |v| atoms.to_local(crate::content_store::StoreIds::as_stored(v)).map(drop)).is_err() {
                    k = keptstats::Keeper::default();
                }
            }
            // The contents its last run had not stored yet, by their sources now.
            k.resolve(|digests| store.sources_by_hashes(&digests.iter().map(|d| crate::content_store::HashKey::from_sha256_digest(*d)).collect::<Vec<_>>()).unwrap_or_else(|_| vec![None; digests.len()]));
            Some(k)
        }
        _ => None,
    };
    if let Some(k) = keeper.as_mut() {
        // By the key their rows have in a memo entry: an export's own name. (A base relation's statistics are kept by
        // its documents' digest, decision 0458: no entry holds its rows.)
        let rels: Vec<(String, Vec<String>)> = split.exports.iter().map(|rel| (rel.clone(), (0..pf.width(rel)).map(|c| format!("c{c}")).collect())).collect();
        // An export's rows' document: its first node column's, as the chunks' totals read it (`Totals::add_rows`).
        let doc_at = split.exports.iter().map(|rel| (rel.clone(), pf.types(rel).iter().position(|t| *t == "node"))).collect();
        // A session's exports counted by file too (0096 step 3). `RERAIL_COUNT_EXPORTS=off` holds every hit's rows
        // again.
        // With them the files' tree counts.
        let keep = (session.is_some() && std::env::var("RERAIL_COUNT_EXPORTS").map_or(true, |v| v != "off")).then(|| keptstats::Keep {
            exports: split.exports.iter().cloned().collect(),
        });
        k.begin(&rels, &memo_ids, doc_at, keep);
    }
    // Decision 0117 step 1: a run with no session keeps its cross-file part's inputs in the store's frames, so an
    // unchanged file's export and base rows are there already. Its memo hit reads only its entry's outputs (nodes,
    // tree counts and lines), and its rows come from the kept inputs after the waves. Not for a run that answers
    // requests as it goes (its cone reads the chunks' rows before the waves end). `RERAIL_KEPT_INPUTS=off` decodes
    // every hit whole, as before.
    if std::env::var_os("RERAIL_STATS_KEEP_TRACE").is_some() {
        eprintln!(
            "rerail: kept route: session {}, durable {}, memoised {memoised}, loading {}, keeper {}",
            session.is_some(), durable.is_some(), loading.is_some(), keeper.is_some()
        );
    }
    let kept_route = session.is_none()
        && durable.is_some()
        && memoised
        && loading.is_none()
        && keeper.is_some()
        && std::env::var("RERAIL_CROSS_K4").map_or(true, |v| v != "off")
        && std::env::var("RERAIL_KEPT_INPUTS").map_or(true, |v| v != "off");
    if std::env::var_os("RERAIL_STATS_KEEP_TRACE").is_some() && !kept_route {
        eprintln!("rerail: no kept route: session {}, durable {}, memoised {memoised}, loading {}, keeper {}", session.is_some(), durable.is_some(), loading.is_some(), keeper.is_some());
    }
    // And only when the cross part kept its inputs (decision 0104: the waves read no entry of the store): its first
    // run, or the first after a rule edit, decodes its hits whole as it finds them. The maintained cross program the
    // kept inputs' key is made of is made once, here, and handed to them.
    let mut early_maintained: Option<lower::Lowered> = None;
    let kept_route = kept_route && {
        let maintained = maintained_cross_of(shared_maintained, &cross)?;
        let (ckey, binds) = retain::keys(&prepared_key, maintained.digest()?);
        part_keys.lock().expect("the parts' keys").push(ckey);
        // The exports alone: the base relations are read off the corpus batch (decision 0458).
        let rels: Vec<String> = split.exports.clone();
        let found = retain::kept_inputs_stored(store, kept_strata::space(root), binds, &maintained.placed()?, &rels)?;
        early_maintained = Some(maintained);
        match found {
            Ok(()) => true,
            Err(why) => {
                crate::code_rule_dsl::logic::profile::decision("the kept inputs (decision 0117 steps 1, 2)", || format!("not taken ({why}): the memo hits decoded whole as they are found"));
                if std::env::var_os("RERAIL_STATS_KEEP_TRACE").is_some() {
                    eprintln!("rerail: the kept inputs not taken: {why}");
                }
                false
            }
        }
    };
    // The files the keeper counted, with the content and key it counted each under: one the same now is unchanged.
    // None while the keeper counts some relations again: each file's entry must hold their rows.
    let counted_now: HashMap<String, (memo::Content, i64)> = match keeper.as_ref().filter(|k| kept_route && !k.has_fresh()) {
        Some(k) => k.counted_files().into_iter().map(|(p, c, key)| (p, (c, key))).collect(),
        None => HashMap::new(),
    };
    // The hits read for their outputs alone, with what finds their whole entry if the kept inputs fail.
    let kept_hits: parking_lot::Mutex<HashMap<String, (memo::Content, i64, memo::Found)>> = parking_lot::Mutex::new(HashMap::new());
    // The kept inputs read at the waves' end, which the cross part adopts as its old sources; and the maintained
    // cross program their key was made of, which it uses.
    let mut kept_loaded: Option<HashMap<String, retain::KeptInput>> = None;
    // The kept inputs' deltas as their owners changed (decision 0117 step 2): `r__plus` and `r__minus` as given, their
    // digests by owner after the edit, and the base relations bound as the kept rows with their delta.
    let mut given_deltas: HashMap<String, (Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)> = HashMap::new();
    let mut kept_input_docs: HashMap<String, Arc<retain::DocDigests>> = HashMap::new();
    let mut base_layered: HashMap<String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>> = HashMap::new();
    let maintained_early: std::cell::RefCell<Option<lower::Lowered>> = std::cell::RefCell::new(None);
    // The memo hits' exports are not measured while the keeper counts them: their statistics are its (after the chunks).
    let mut hits_unmeasured = false;
    let mut memo_kept = (0usize, 0usize);
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
            let entry = memo.lookup(content, world::store_path(&path), key, &memo_plan).map_err(|e| Error::Plan(e.to_string()))?.ok_or_else(|| Error::Plan(format!("{path}: its memo entry is gone, and the counted exports were given up")))?;
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
        /// Whether `found` is the entry's outputs alone (decision 0117 step 1): the file is as the keeper counted it.
        outputs: bool,
    }
    // The tapes of the files the run's waves will load (a phase's, its loads'), read before the first wave (decision
    // 0104), by source: set below, once the memo's entries the waves may find are known.
    let wave_tapes: std::sync::OnceLock<HashMap<crate::content_store::SourceId, crate::content_store::TapeFrame>> = std::sync::OnceLock::new();
    // The same tapes by their text's hash (Q22), for a file the run knows by its digest alone (no memo, or one named
    // by digest in a wave).
    let mut by_text: HashMap<crate::content_store::HashKey, (crate::content_store::SourceId, crate::content_store::TapeFrame)> = HashMap::new();
    /// A memo hit the corpus batch makes a document of (decision 0458): its key, path and module, and its guests'
    /// requests by round, as its entry kept them (`memo::GUESTS`).
    struct CorpusHit {
        key: i64,
        path: String,
        module: Option<Arc<str>>,
        rounds: Vec<(usize, String, output::Rows)>,
    }
    // The memo hits' tapes, by path, where the cross part reads base relations (decision 0458): read with the misses'
    // (before the first wave, or from those read then), and its document made from it once the chunks are done.
    let hit_tapes: parking_lot::Mutex<HashMap<String, crate::content_store::TapeFrame>> = parking_lot::Mutex::new(HashMap::new());
    // The files built with a tape read before the first wave, and those parsed (`Report::file_tapes`).
    let file_tapes = (std::sync::atomic::AtomicUsize::new(0), std::sync::atomic::AtomicUsize::new(0));
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
        let mut outputs = vec![false; window.len()];
        if let (true, true, Some(memo)) = (memoised, reading, memo) {
            let _lookup = crate::code_rule_dsl::logic::profile::span(|| "memo lookup".into());
            let asked: Vec<usize> = (0..window.len()).filter(|i| known[*i].is_some()).collect();
            // A file as the keeper counted it, on the kept route: its outputs alone (decision 0117 step 1). One whose
            // outputs the store does not keep (an entry written before them) is found whole.
            let unchanged = |i: usize| counted_now.get(&window[i].1).is_some_and(|c| keptstats::same(*c, (memo::Content::Stored(known[i].expect("known")), window[i].0)));
            let (light, mut asked): (Vec<usize>, Vec<usize>) = asked.into_iter().partition(|i| kept_route && unchanged(*i));
            if kept_route && std::env::var_os("RERAIL_STATS_KEEP_TRACE").is_some() {
                // Why a file is not unchanged: not counted, or counted under another content or key.
                let (mut uncounted, mut other) = (0usize, Vec::new());
                for &i in &asked {
                    match counted_now.get(&window[i].1) {
                        None => uncounted += 1,
                        Some(c) => other.push(format!("{}: counted {:?}, now {:?}", window[i].1, c, (memo::Content::Stored(known[i].expect("known")), window[i].0))),
                    }
                }
                eprintln!("rerail: kept route lookups: {} unchanged, {} not ({uncounted} not counted, {} counted otherwise{}); {} counted in all", light.len(), asked.len(), other.len(), other.first().map_or(String::new(), |o| format!(", e.g. {o}")), counted_now.len());
            }
            if !light.is_empty() {
                let keys: Vec<(memo::Content, &str)> = light.iter().map(|i| (memo::Content::Stored(known[*i].expect("known")), world::store_path(&window[*i].1))).collect();
                for (i, f) in light.into_iter().zip(memo.found_outputs_all(&memo_plan, &keys).map_err(|e| Error::Plan(e.to_string()))?) {
                    match f {
                        Some(f) => {
                            found[i] = Some(f);
                            outputs[i] = true;
                        }
                        None => asked.push(i),
                    }
                }
            }
            let keys: Vec<(memo::Content, &str)> = asked.iter().map(|i| (memo::Content::Stored(known[*i].expect("known")), world::store_path(&window[*i].1))).collect();
            for (i, f) in asked.into_iter().zip(memo.found_all(&memo_plan, &keys).map_err(|e| Error::Plan(e.to_string()))?) {
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
                // Held from the run's digests (decision 0300): taken here only where this window's texts are kept.
                if let Some(source) = source_texts.text(&root.join(path), keep) {
                    return Ok(Some(source));
                }
                let source = match std::fs::read_to_string(root.join(path)) {
                    Ok(source) => source,
                    // A text file that is not UTF-8 (an image, an archive) is no document: its wave finds it so.
                    Err(e) if e.kind() == std::io::ErrorKind::InvalidData && text_document(path, label) => return Ok(None),
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
                // A file the index knew by this stat, read in the waves where the store is not asked, has its content
                // named by its digest (`Unstored`): recording it again wrote the same stat on every warm run
                // (`RERAIL_WARM`). It is trusted as the index's source, as a known file the memo finds is, unread. A
                // known file found to be another stored source (a stat that lied) is recorded.
                let known_unstored = known[*i].is_some() && matches!(c, Some(memo::Content::Unstored(_)));
                if let (Some(stat), Some(c)) = (stats[*i], c)
                    && !matches!(c, memo::Content::Held(_))
                    && !known_unstored
                    && known[*i].map(memo::Content::Stored) != Some(*c)
                {
                    stats_read.lock().push((world::store_path(&window[*i].1).to_owned(), stat, *c));
                }
            }
            // A file the index knew, read again because its entry was missing, is not looked up a second time. In
            // the waves every content is looked up by its digest, in the frames held in memory (decision 0104).
            if memoised {
                let asked: Vec<usize> = (0..rest.len()).filter(|r| contents[*r].is_some() && !(known[rest[*r]].is_some() && known[rest[*r]] == contents[*r].and_then(memo::Content::stored))).collect();
                // A file read because its stat could not be trusted (git's racy rule, `RACY_NS`: written just before
                // the last run recorded it) whose content is the one the keeper counted: its outputs alone, as a file
                // the index knew (decision 0117 step 1). Else an edit right after a run on freshly written files
                // decoded every hit whole.
                let counted_same = |r: usize| contents[r].is_some_and(|c| counted_now.get(&window[rest[r]].1).is_some_and(|k| keptstats::same(*k, (c, window[rest[r]].0))));
                let (light, mut asked): (Vec<usize>, Vec<usize>) = asked.into_iter().partition(|r| kept_route && counted_same(*r));
                if !light.is_empty() {
                    let keys: Vec<(memo::Content, &str)> = light.iter().map(|r| (contents[*r].expect("a content"), world::store_path(&window[rest[*r]].1))).collect();
                    for (r, f) in light.into_iter().zip(memo.found_outputs_all(&memo_plan, &keys).map_err(|e| Error::Plan(e.to_string()))?) {
                        match f {
                            Some(f) => {
                                found[rest[r]] = Some(f);
                                outputs[rest[r]] = true;
                            }
                            None => asked.push(r),
                        }
                    }
                }
                let keys: Vec<(memo::Content, &str)> = asked.iter().map(|r| (contents[*r].expect("a content"), world::store_path(&window[rest[*r]].1))).collect();
                for (r, f) in asked.into_iter().zip(memo.found_all(&memo_plan, &keys).map_err(|e| Error::Plan(e.to_string()))?) {
                    found[rest[r]] = f;
                }
            }
            for (i, c) in rest.iter().zip(contents) {
                content[*i] = c;
            }
            // In the waves the store is not read, so a content is named by its digest: but a file the stat index
            // knows is that source, as it is before the first wave. Named by its digest instead, every unchanged
            // file of a load (a phase's files) was a content new to the statistics keeper: each run counted all of
            // them out and in again, decoding every entry (decision 0116, step 3).
            if !reading {
                for &i in &rest {
                    if let (Some(id), Some(memo::Content::Unstored(_))) = (known[i], content[i]) {
                        content[i] = Some(memo::Content::Stored(id));
                    }
                }
            }
        }
        // The tapes of the files the memo has no entry for, of those whose sources the store holds: one read.
        let mut tape: Vec<Option<crate::content_store::TapeFrame>> = vec![None; window.len()];
        if let (true, Some(cache), Some(lang)) = (reading, tapes.as_ref().filter(|_| cfg!(feature = "tape-cache")), crate::code_rule_dsl::language::dialect_by_label(label)) {
            // A hit's too, where the cross part reads base relations off every file's tape (decision 0458).
            let asked: Vec<usize> = (0..window.len()).filter(|i| (found[*i].is_none() || !stored.is_empty()) && matches!(content[*i], Some(memo::Content::Stored(_)))).collect();
            if let (false, Some(tag)) = (asked.is_empty(), crate::code_rule_dsl::language::tape_tag(lang)) {
                let _tapes = crate::code_rule_dsl::logic::profile::span(|| "tapes, by their sources".into());
                let ids: Vec<crate::content_store::SourceId> = asked.iter().map(|i| content[*i].and_then(memo::Content::stored).expect("stored")).collect();
                let frames = cache
                    .tape_frames_for(&ids, i64::from(tag), crate::code_rule_dsl::build_ids::grammar_version(), i64::from(crate::code_rule_dsl::tape::formats::stored_version()))
                    .map_err(|e| Error::Plan(e.to_string()))?;
                for (i, f) in asked.into_iter().zip(frames) {
                    match (found[i].is_some(), f) {
                        (true, Some(f)) => {
                            hit_tapes.lock().insert(window[i].1.clone(), f);
                        }
                        (true, None) => {}
                        (false, f) => tape[i] = f,
                    }
                }
            }
        } else if let Some(held) = wave_tapes.get().filter(|_| !reading) {
            // In a wave, from those read before the first: a file that misses the memo (a new plan's run over an
            // older store) is decompressed, not parsed again; a hit's, for the corpus batch (decision 0458).
            for i in (0..window.len()).filter(|i| found[*i].is_none() || !stored.is_empty()) {
                let frame = content[i].and_then(memo::Content::stored).and_then(|id| held.get(&id)).cloned();
                match (found[i].is_some(), frame) {
                    (true, Some(f)) => {
                        hit_tapes.lock().insert(window[i].1.clone(), f);
                    }
                    (true, None) => {}
                    (false, f) => tape[i] = f,
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
            .map(|(i, ((((stat, content), found), digest), tape))| Before { stat, content, found, text: texts.remove(&i), digest, tape, outputs: outputs[i] })
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
            if std::env::var_os("RERAIL_STATS_KEEP_TRACE").is_some() {
                eprintln!("statkeep before the first wave: {} files, {} known by stat, {} found", looked.len(), looked.iter().filter(|b| matches!(b.content, Some(memo::Content::Stored(_)))).count(), looked.iter().filter(|b| b.found.is_some()).count());
            }
            batch.into_iter().map(|(_, p, _)| p).zip(looked).collect()
        }
        _ => HashMap::new(),
    });
    // The entries the statistics keeper counted for files changed or gone since (`keptstats`), found before the
    // first wave too: it counts them out in the waves, and reads none of them from SQLite there.
    // A file looked up here and not found is kept as `None`: the waves do not ask SQLite again for what it has not.
    // That includes a file whose content is unchanged but whose entry the run's plan does not have (one the memo
    // dropped): the keeper counts it out against the entry it was counted under. (Not a new build's: its keeper
    // starts empty, `memo_whole` being of the build too.)
    let counted_before: HashMap<String, Option<memo::Found>> = match (memo, keeper.borrow().as_ref()) {
        (Some(memo @ memo::Memo::Stored(_)), Some(k)) if looking => {
            let _before = crate::code_rule_dsl::logic::profile::span(|| "memo lookup, before the first wave".into());
            let now = before.lock();
            // Each file the keeper will count out: one it counted under another content or document key than the
            // run's (`keptstats::same`, the keeper's own test), one whose entry the run's plan does not have
            // (`found` is none), or one the run does not have.
            let key_now: HashMap<&str, i64> = docs.iter().map(|(d, p)| (p.as_str(), *d)).collect();
            let kept = |p: &String, c: memo::Content, k: i64| match (now.get(p), key_now.get(p.as_str())) {
                (Some(b), Some(&d)) => b.found.is_some() && b.content.is_some_and(|content| keptstats::same((c, k), (content, d))),
                _ => false,
            };
            let changed: Vec<(String, memo::Content)> = k.counted_files().into_iter().filter(|(p, c, k)| !kept(p, *c, *k)).map(|(p, c, _)| (p, c)).collect();
            drop(now);
            let keys: Vec<(memo::Content, &str)> = changed.iter().map(|(p, c)| (*c, world::store_path(p))).collect();
            let found = memo.found_all(&memo_plan, &keys).map_err(|e| Error::Plan(e.to_string()))?;
            changed.iter().map(|(p, _)| p.clone()).zip(found).collect()
        }
        _ => HashMap::new(),
    };
    // Decision 0458: the files the kept part's base relations were read off, as the run that kept it had them (its
    // documents' table), and for each one changed or gone since, its old source, tape and entry (its guests' requests):
    // all read here, before the first wave (decision 0104). The changed files' old and new documents are then a base
    // relation's two versions (below). `None` where a run cannot say what changed: no table (the cross part's first
    // run, or the first after a rule edit), a file's content not looked up, or an old source the store no longer
    // holds; what reads a base relation then runs again from its reads.
    struct OldBase {
        key: i64,
        bytes: Arc<crate::content_store::CachedBytes>,
        frame: Option<crate::content_store::TapeFrame>,
        entry: Option<memo::Entry>,
    }
    let base_binds: Option<[u8; 32]> = if session.is_none() && durable.is_some() && memoised && !stored.is_empty() {
        let maintained = maintained_cross_of(shared_maintained, &cross)?;
        Some(retain::keys(&prepared_key, maintained.digest()?).1)
    } else {
        None
    };
    type OldBases = Option<(BTreeSet<String>, HashMap<String, OldBase>)>;
    // The base relations' statistics kept beside the table, each with the digest of the rows it counts, and the
    // table's own digest (the documents it names): decision 0458.
    type BaseStatsKept = (Option<u64>, HashMap<String, (u64, super::statkeep::RelStats)>);
    // The compare, given each file's document key and content now (`now_of`): before the first wave where the run
    // knows its files then, else (a run that lists its files as it goes, and reads its memo in the waves) once the
    // chunks are done, from the contents they found (`run_docs`).
    // The documents' table and the statistics kept beside it, read before the first wave in every run that keeps
    // them (decision 0104), a run inside another's waves too: they name no file of this run.
    // With them, the corpus batch's documents' views' counts (decision 0464).
    let (base_table, mut base_stats_kept, base_counts): (Option<Vec<retain::BaseDocument>>, BaseStatsKept, retain::BaseCounts) = match (base_binds, memo, looking) {
        (Some(binds), Some(memo::Memo::Stored(_)), true) => {
            let _s = crate::code_rule_dsl::logic::profile::span(|| "the base relations' table and statistics (decision 0458)".into());
            let counted: Vec<String> = stored.iter().map(|(r, _)| r.clone()).filter(|r| !semijoin.contains(r)).collect();
            let stats = retain::base_stats(store, kept_strata::space(root), binds, &counted)?;
            let table = retain::base_documents(store, kept_strata::space(root), binds)?;
            let counts = retain::base_counts(store, kept_strata::space(root), binds)?;
            let digest = table.as_ref().map(|(_, d)| *d);
            (table.map(|(t, _)| t), (digest, stats), counts)
        }
        _ => (None, (None, HashMap::new()), HashMap::new()),
    };
    let base_compare = |now_of: HashMap<String, (i64, Option<memo::Content>)>| -> Result<OldBases> {
        let (Some(_), Some(memo @ memo::Memo::Stored(_)), true) = (base_binds, memo, looking) else { return Ok(None) };
        let found = (|| -> Result<std::result::Result<(BTreeSet<String>, HashMap<String, OldBase>), String>> {
            let _before = crate::code_rule_dsl::logic::profile::span(|| "the base relations' documents (decision 0458)".into());
            let Some(mut table) = base_table.clone() else { return Ok(Err("no documents' table kept".into())) };
            let mut now_of = now_of;
            // A content written after the table was is found by its digest, as the keeper's are (`Keeper::resolve`);
            // so is a file read in this run (its stat changed, or is too recent to trust), whose source, where the
            // store holds it, is what the table names.
            let unstored: Vec<(Option<usize>, Option<String>, [u8; 32])> = table.iter().enumerate().filter_map(|(i, (_, _, c))| match c { memo::Content::Unstored(d) => Some((Some(i), None, *d)), _ => None })
                .chain(now_of.iter().filter_map(|(p, (_, c))| match c { Some(memo::Content::Unstored(d)) => Some((None, Some(p.clone()), *d)), _ => None }))
                .collect();
            if !unstored.is_empty() {
                let ids = store.sources_by_hashes(&unstored.iter().map(|(_, _, d)| crate::content_store::HashKey::from_sha256_digest(*d)).collect::<Vec<_>>()).map_err(|e| Error::Plan(e.to_string()))?;
                for ((at, path, _), id) in unstored.into_iter().zip(ids) {
                    match (at, path, id) {
                        (Some(i), _, Some(id)) => table[i].2 = memo::Content::Stored(id),
                        (Some(_), _, None) => return Ok(Err("a content of the table is no longer stored".into())),
                        (None, Some(p), Some(id)) => {
                            if let Some(now) = now_of.get_mut(&p) {
                                now.1 = Some(memo::Content::Stored(id));
                            }
                        }
                        (None, _, _) => {}
                    }
                }
            }
            let same = |path: &str, key: i64, was: memo::Content| matches!(now_of.get(path), Some(&(d, Some(c))) if keptstats::same((was, key), (c, d)));
            let olds: Vec<(String, i64, crate::content_store::SourceId)> = table.iter().filter(|(p, k, c)| !same(p, *k, *c)).filter_map(|(p, k, c)| c.stored().map(|id| (p.clone(), *k, id))).collect();
            let in_table: std::collections::HashSet<&str> = table.iter().map(|(p, _, _)| p.as_str()).collect();
            let changed: BTreeSet<String> = olds.iter().map(|(p, _, _)| p.clone()).chain(now_of.keys().filter(|p| !in_table.contains(p.as_str())).cloned()).collect();
            let ids: Vec<crate::content_store::SourceId> = olds.iter().map(|(_, _, id)| *id).collect();
            let texts = store.get_sources_bulk(&ids).map_err(|e| Error::Plan(e.to_string()))?;
            let frames = match (tapes.as_ref().filter(|_| cfg!(feature = "tape-cache")), crate::code_rule_dsl::language::dialect_by_label(label).and_then(crate::code_rule_dsl::language::tape_tag)) {
                (Some(cache), Some(tag)) => cache
                    .tape_frames_for(&ids, i64::from(tag), crate::code_rule_dsl::build_ids::grammar_version(), i64::from(crate::code_rule_dsl::tape::formats::stored_version()))
                    .map_err(|e| Error::Plan(e.to_string()))?,
                _ => vec![None; ids.len()],
            };
            // Each old entry, for its guests' requests (a program with guests only).
            let keys: Vec<(memo::Content, &str)> = olds.iter().map(|(p, _, id)| (memo::Content::Stored(*id), world::store_path(p))).collect();
            let found: Vec<Option<memo::Found>> = if slice.is_some() { memo.found_all(&memo_plan, &keys).map_err(|e| Error::Plan(e.to_string()))? } else { keys.iter().map(|_| None).collect() };
            let mut out = HashMap::new();
            for (((path, key, id), frame), found) in olds.into_iter().zip(frames).zip(found) {
                let Some((bytes, _)) = texts.get(&id) else { return Ok(Err(format!("{path}: its old source is no longer stored"))) };
                let entry = match (slice.is_some(), found) {
                    (false, _) => None,
                    (true, Some(found)) => match found.entry(key).map_err(|e| Error::Plan(e.to_string()))? {
                        Some(entry) => Some(entry),
                        None => return Ok(Err(format!("{path}: its old entry is another document's"))),
                    },
                    (true, None) => return Ok(Err(format!("{path}: its old entry is not in the memo"))),
                };
                out.insert(path, OldBase { key, bytes: Arc::clone(bytes), frame, entry });
            }
            Ok(Ok((changed, out)))
        })()?;
        Ok(match found {
            Ok(old) => Some(old),
            Err(why) => {
                crate::code_rule_dsl::logic::profile::decision("the base relations' old documents (decision 0458)", || format!("none ({why}): what reads a base relation runs again"));
                if std::env::var_os("RERAIL_STATS_KEEP_TRACE").is_some() {
                    eprintln!("rerail: the base relations' old documents: none ({why})");
                }
                None
            }
        })
    };
    // Before the first wave, where the run knows its files then (decision 0104).
    let base_known = !docs.is_empty();
    let mut old_base: OldBases = if base_known {
        let now = before.lock();
        let now_of: HashMap<String, (i64, Option<memo::Content>)> = docs.iter().map(|(d, p)| (p.clone(), (*d, now.get(p).and_then(|b| b.content)))).collect();
        drop(now);
        base_compare(now_of)?
    } else {
        None
    };
    // The entry a file was counted under: as found before the first wave, else (a session's, in memory) looked up.
    let counted_entry = |path: &str, c: memo::Content, key: i64| -> Option<memo::Entry> {
        match counted_before.get(path) {
            Some(Some(found)) => found.entry(key).ok().flatten(),
            Some(None) => None,
            None => memo.and_then(|m| m.lookup(c, world::store_path(path), key, &memo_plan).ok().flatten()),
        }
    };
    // The keeper's end, once every file of the run was seen: the files gone counted out, the statistics as kept (else
    // `None`), and the run's exports the counted ones (else the skipped hits held after all).
    let stats_kept: std::cell::RefCell<Option<std::result::Result<(usize, usize), String>>> = std::cell::RefCell::new(None);
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
            *stats_kept.borrow_mut() = Some(ended.as_ref().map(|_| (counted_in, counted_out)).map_err(Clone::clone));
            ended.ok()
        });
        let skipped = held_by_count.borrow().len();
        let installed = {
            let k = keeper.borrow();
            match k.as_ref().filter(|_| kept.is_some()).and_then(|k| k.counted_exports()) {
                Some(counted) if skipped > 0 => {
                    let _s = crate::code_rule_dsl::logic::profile::span(|| "the counted exports".into());
                    for rel in &split.exports {
                        let width = pf.width(rel);
                        exports.borrow_mut().install(rel, width, counted.source(rel, width));
                    }
                    crate::code_rule_dsl::logic::profile::decision("exports counted by file (0096 step 3)", || format!("{skipped} memo hits neither held again nor counted: {} export rows counted", counted.rows()));
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
    // The cross program's strata identities, read with the cross part's frames (kept as a stored fact, before the
    // first wave): what `Retained::bind_kept` names its frames by.
    let mut cross_idents: Vec<(u64, retain::StrataIdents)> = Vec::new();
    // The run's waves: from its first chunk to the cross-file part's last stratum. SQL a wave's thread runs or waits on
    // meanwhile is a hazard in the profile (decision 0104); before is the run's beginning, after its end.
    {
        // The cone of requests' connection and store, opened before the first wave (decision 0104).
        let (slot, _) = handed.cone();
        connections::Cone::ready(&slot, &path, durable.is_some().then(|| store.database_path()))?;
        // And the kept blocks' rows (decision 0104): the waves bind the kept facts by them, from memory
        // (`preload_kept_state`). The memo's entries are admitted below (decision 0240 §4).
        let preload_at = std::time::Instant::now();
        // The cross program's strata identities and the maintained one's the kept route made, which the part's
        // entries are named by (`Retained::bind_kept_named`), read as stored facts.
        if let Some(maintained) = early_maintained.as_ref() {
            let _s = crate::code_rule_dsl::logic::profile::span(|| "the cross part's strata identities".into());
            let progs = [cross.placed()?, maintained.placed()?];
            let strata = retain::strata_kept(store, &[&progs[0], &progs[1]])?;
            cross_idents = progs.iter().map(super::body::Placed::digest).zip(strata).collect();
        }
        let spaces = {
            let _s = crate::code_rule_dsl::logic::profile::span(|| "the kept blocks' rows, before the first wave (decision 0104)".into());
            store.preload_kept_state().map_err(|e| Error::Plan(e.to_string()))?
        };
        let preload_secs = preload_at.elapsed().as_secs_f64();
        // The run's memo entries its waves may look up (a load's files): each path's newest of its plan's space, but
        // the files looked up already (decision 0104: read before the first wave).
        let (entries, entry_bytes, _) = match memo {
            Some(memo::Memo::Stored(_)) if memoised => {
                let _s = crate::code_rule_dsl::logic::profile::span(|| "the memo entries the waves may look up, admitted".into());
                let looked: std::collections::HashSet<&str> = docs.iter().map(|(_, p)| world::store_path(p)).collect();
                store.admit_memo_entries(&crate::content_store::local_space(&memo::plan_digest(&memo_program)), &looked, &|b| memo::hold(store, b.iter().copied())).map_err(|e| Error::Plan(e.to_string()))?
            }
            _ => (0, 0, None),
        };
        if std::env::var_os("RERAIL_STATS_KEEP_TRACE").is_some() && spaces + entries > 0 {
            eprintln!("rerail: read before the first wave: {spaces} kept spaces' rows, {preload_secs:.2} s; {entries} memo entries its waves may look up, {:.1} MB", entry_bytes as f64 / 1048576.0);
        }
        // The tapes the root's last runs named (Q22), of any lane: a lane is the programs' texts, and a rule's edit
        // starts a new one with no run, where the files are as they were. The last few, for the phases' runs over the
        // root record between. One read, before the first wave (decision 0104), for the waves' builds (a load's files,
        // a file the memo misses). A guest's tape is not stored (Q30).
        if let (Some(cache), Some(tag)) = (
            tapes.as_ref().filter(|_| cfg!(feature = "tape-cache")),
            crate::code_rule_dsl::language::dialect_by_label(label).and_then(crate::code_rule_dsl::language::tape_tag),
        ) {
            let _tapes = crate::code_rule_dsl::logic::profile::span(|| "the root's tapes, by their sources (Q22)".into());
            let wanted = store.root_sources(&canonical_root, TAPE_RUNS).map_err(|e| Error::Plan(e.to_string()))?;
            let frames = cache
                .tape_frames_with_hashes(&wanted, i64::from(tag), crate::code_rule_dsl::build_ids::grammar_version(), i64::from(crate::code_rule_dsl::tape::formats::stored_version()))
                .map_err(|e| Error::Plan(e.to_string()))?;
            if std::env::var_os("RERAIL_STATS_KEEP_TRACE").is_some() {
                eprintln!("rerail: tapes before the first wave: {} sources asked, {} tapes", wanted.len(), frames.len());
            }
            if !frames.is_empty() {
                crate::code_rule_dsl::logic::profile::decision("tapes read before the first wave (Q22)", || format!("{} tapes of {} sources, {:.1} MB", frames.len(), wanted.len(), frames.iter().map(|(_, _, f)| f.len()).sum::<usize>() as f64 / 1048576.0));
            }
            let _ = wave_tapes.set(frames.iter().map(|(s, _, f)| (*s, f.clone())).collect());
            by_text = frames.into_iter().map(|(source, hash, frame)| (hash, (source, frame))).collect();
        }
    }
    let guest_parses = guests::Parses::default();
    // The roots its requests may read, watched before the first wave (`World::watch`): a `World` made inside the
    // fixpoint would otherwise read the observer's store there (decision 0104; Q150).
    if loading.is_some() {
        world::World::watch(root, head);
    }
    let mut waves = Some(crate::code_rule_dsl::logic::profile::waves());
    // A file's memo entry when the run is memoised and the memo has it, else its document, built. An entry keeps
    // no base rows: the cross part reads them off the corpus batch (decision 0458).
    let memo_rows: BTreeSet<String> = split.exports.iter().cloned().collect();
    let memo_row_needed = |r: &str| r == OWN_DOCS || r == memo::COVERED || memo_rows.contains(r)
        || (memo_tree_stats && (r == OWN_KINDS || r == OWN_EDGES || r == OWN_FIELDS || r.starts_with(OWN_POSTINGS)));
    crate::code_rule_dsl::logic::profile::decision("memo consumer projection", || format!(
        "{} demanded relations; tree distributions {}", memo_rows.len(), if memo_tree_stats { "retained" } else { "omitted" }));
    let has_counts = |e: &memo::Entry| e.rows.contains_key(OWN_DOCS);
    // The relations an entry does not hold under their identities now (decision 0225 §2): a rule's edit reached them.
    let missing_of = |e: &memo::Entry| -> BTreeSet<String> {
        let covered = memo::covered(e);
        memo_ids.iter().filter(|(_, id)| !covered.contains(&memo::covered_words(id))).map(|(r, _)| r.clone()).collect()
    };
    // A window's files resolved together: as found before the first wave, or (a load's) looked up now. A hit's
    // entry is decoded here; a miss's file is read here, and is a new content if it changed since it was found.
    /// A file to build: its text, its content as the memo knows it, the text's digest and its tape, when they are
    /// known.
    struct Unbuilt {
        text: String,
        content: Option<memo::Content>,
        digest: Option<[u8; 32]>,
        tape: Option<crate::content_store::TapeFrame>,
        /// Its entry, when it holds some of the relations (`missing_of`): the file is built and its chunk derives the
        /// others (decision 0225 §2).
        partial: Option<memo::Entry>,
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
            if std::env::var_os("RERAIL_STATS_KEEP_TRACE").is_some() {
                eprintln!("statkeep looked up in the waves: {} of {} ({})", asked.len(), window.len(), asked.first().map_or("", |a| a.1.as_str()));
            }
            for (i, b) in late.into_iter().zip(look_up(&asked, true)?) {
                taken[i] = Some(b);
            }
        }
        window
            .iter()
            .zip(taken)
            .map(|((d, path, _), b)| {
                let b = b.expect("every file looked up");
                // An unchanged file on the kept route: its outputs, its rows the kept inputs' (decision 0117 step 1).
                if let (true, true, Some(found), Some(content)) = (memoised, b.outputs, b.found.as_ref(), b.content) {
                    let _decode = crate::code_rule_dsl::logic::profile::span(|| "memo decode: the outputs".into());
                    if let Some(entry) = found.projected(*d, &memo::outputs_alone).map_err(|e| Error::Plan(e.to_string()))?.filter(|e| missing_of(e).is_empty()) {
                        kept_hits.lock().insert(path.clone(), (content, *d, found.clone()));
                        return Ok(Some((b.content, Ok(entry))));
                    }
                }
                let mut partial = None;
                if let (true, false, Some(found)) = (memoised, b.outputs, b.found) {
                    let _decode = crate::code_rule_dsl::logic::profile::span(|| "memo decode".into());
                    let entry = found.projected(*d, &memo_row_needed).map_err(|e| Error::Plan(e.to_string()))?;
                    if std::env::var_os("RERAIL_STATS_KEEP_TRACE").is_some() && entry.as_ref().is_some_and(|e| !has_counts(e)) {
                        eprintln!("rerail: memo entry of {path} unused: it holds no tree counts");
                    }
                    match entry {
                        Some(entry) if has_counts(&entry) && missing_of(&entry).is_empty() => return Ok(Some((b.content, Ok(entry)))),
                        Some(entry) if missing_of(&entry).len() < memo_ids.len() => partial = Some(entry),
                        _ => {}
                    }
                }
                let (text, content, digest, tape) = match b.text {
                    Some(text) => (text, b.content, b.digest, b.tape),
                    None => {
                        let _read = crate::code_rule_dsl::logic::profile::span(|| "read".into());
                        // Held from the run's digests (decision 0300), as of the stat it was read at.
                        let held = source_texts.text(&root.join(path), true);
                        let text = match held.map_or_else(|| std::fs::read_to_string(root.join(path)), Ok) {
                            Ok(text) => text,
                            // A text file that is not UTF-8 (an image, an archive) is no document.
                            Err(e) if e.kind() == std::io::ErrorKind::InvalidData && text_document(path, label) => return Ok(None),
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
                // A partial entry is of the content it was found for: a file changed since is a miss.
                let partial = partial.filter(|_| content == b.content);
                Ok(Some((content, Err(Unbuilt { text, content, digest, tape, partial }))))
            })
            .collect()
    };
    // The matchers a chunk runs (decision 0106). A chunk's documents keep their tapes for them, and for a program that
    // reads W's values, f-string trees or string annotations (`syntax::taped`).
    let chunk_matchers = matchers::compiled(&matchers, &ProgramFields::policies(head)).map_err(Error::Plan)?;
    let keep_tapes = !chunk_matchers.is_empty() || read.iter().any(|rel| syntax::taped(rel));
    if !chunk_matchers.is_empty() {
        crate::code_rule_dsl::logic::profile::decision("ast matchers (decision 0106)", || format!("{} matchers run by the transducer over each chunk's tapes", chunk_matchers.len()));
    }
    // The partial hits' entries (`Unbuilt::partial`), by document key, until their chunk takes them.
    let partials: parking_lot::Mutex<HashMap<i64, memo::Entry>> = parking_lot::Mutex::new(HashMap::new());
    let partial_hits = std::sync::atomic::AtomicUsize::new(0);
    let guests_unseen = std::sync::atomic::AtomicUsize::new(0);
    // The file-local program pruned to each set of relations partial hits miss (`cone_of`), made once a run.
    let cones: parking_lot::Mutex<HashMap<BTreeSet<String>, Arc<lower::Lowered>>> = parking_lot::Mutex::new(HashMap::new());
    let build_doc = |d: &i64, path: &String, module: Option<Arc<str>>, unbuilt: Unbuilt| -> Result<(Option<memo::Content>, Option<[u8; 32]>, syntax::Document)> {
        let Unbuilt { text: source, content: source_id, digest, tape, partial } = unbuilt;
        // The content's digest, which names its memo entry: the writer is given it, not left to read it (decision 0104).
        // A stored source the stat index knew was not hashed to look it up: its text, in hand, is hashed here.
        let digest = match (digest, source_id) {
            (None, Some(memo::Content::Stored(_))) => {
                use sha2::{Digest as _, Sha256};
                Some(Sha256::digest(source.as_bytes()).into())
            }
            (d, _) => d,
        };
        let content_digest = digest;
        if let Some(entry) = partial {
            partials.lock().insert(*d, entry);
            partial_hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        let _build = crate::code_rule_dsl::logic::profile::span(|| "tree-sitter and facts".into());
        // Its own work: the file's input made from its source (the build's parts charge their own).
        crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, source.len() as u64);
        // A file of no grammar is here because `.scope text` reads it (`scoped`): text only, no tree, its lines W's
        // for a file of no language (`core::text`).
        if text_document(path, label) {
            let facts = Arc::new(core::text(Arc::new(source)));
            let digest = core::digest(&facts);
            return Ok((source_id, content_digest, syntax::Document { key: *d as u32, path: path.clone(), core: format!("/c#core:{path}"), digest, facts, guest: None, module, tape: None }));
        }
        let file = core::Source { language: label.to_owned(), source: Arc::new(source) };
        // Tapes kept across runs where the run has the persistent store (`tape_cache`).
        let built = match &tapes {
            // The closure sees `&tapes`, not that it is `None` without `tape-cache`: said here so the store's
            // tape and source frames are not linked.
            // In the waves no SQL is read or written (decision 0104): the tape was found before the first wave, or the
            // file is parsed and its source and tape are the writer's, behind the run; the manifest names the sources
            // it stores (`late_sources`).
            Some(cache) if cfg!(feature = "tape-cache") => {
                // Else by its text and grammar, among the root's last runs' tapes (Q22): a file whose content the run
                // has no source id for (no memo, or one named by digest in a wave). Its source is named in the manifest.
                let (digest, tape) = match tape {
                    Some(t) => (digest, Some(t)),
                    None => {
                        use sha2::Digest as _;
                        let d: [u8; 32] = digest.unwrap_or_else(|| sha2::Sha256::digest(file.source.as_bytes()).into());
                        let held = by_text.get(&crate::content_store::HashKey::from_sha256_digest(d));
                        if let Some((source, _)) = held {
                            late_sources.lock().expect("the late sources").push(*source);
                        }
                        (Some(d), held.map(|(_, f)| f.clone()))
                    }
                };
                if tape.is_some() { &file_tapes.0 } else { &file_tapes.1 }.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                core::build_in_wave_with_tape(&file, cache, digest, tape.as_ref(), true, keep_tapes || references.is_some())
            }
            Some(_) => unreachable!("no tape cache without `tape-cache`"),
            None => core::build_with_tape(&file, keep_tapes || references.is_some()),
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
            // W's reference graph summarizes the file from its tape, read in place (Q51: it parsed every file again
            // with tree-sitter, 39% of internal_dead_code's CPU).
            let _refs = crate::code_rule_dsl::logic::profile::span(|| "W's reference graph".into());
            let doc = tape.as_ref().ok_or_else(|| Error::Plan(format!("{path}: no tape for the reference graph")))?;
            references.add(*d as u32, path, &file.source, &facts, doc)?;
        }
        // A tape kept only for the reference graph goes with it.
        let tape = tape.filter(|_| keep_tapes);
        // The tape, in the file's context: kept only for a program that reads W's values, f-string trees or string
        // annotations off it, or runs matchers over it (decision 0106).
        let tape = tape.map(|doc| (doc, Arc::new(file) as Arc<dyn crate::code_rule_dsl::facts::core::SourceFile>));
        let digest_span = crate::code_rule_dsl::logic::profile::span(|| "digest".into());
        let digest = core::digest(&facts);
        drop(digest_span);
        Ok((source_id, content_digest, syntax::Document { key: *d as u32, path: path.clone(), core: format!("/c#core:{path}"), digest, facts, guest: None, module, tape }))
    };
    // Files resolved a window at a time (`resolve`), so a window's sources are all the builder holds unparsed: one
    // window at a time, in order, each a job of its own, and each file's tree and facts then built as a job.
    const WINDOW: usize = 256;
    let resolver = parking_lot::Mutex::new(resolve);

    // A document: a memo hit, or built from its source; `None` for a file that is no document.
    type Made = Result<Option<(Option<memo::Content>, std::result::Result<memo::Entry, (syntax::Document, Option<[u8; 32]>)>)>>;
    type Sources = Vec<(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)>;
    // A chunk's documents, hits and kept documents while it is projected.
    type Cutting = (usize, Vec<(i64, String, Option<memo::Content>, u64, usize, Option<[u8; 32]>)>, Vec<(String, memo::Entry, Option<memo::Content>, Option<Arc<str>>)>, Vec<syntax::Document>);
    let build = |docs: &Docs, i: usize| -> Made {
        // A window that failed resolves none of its files: its first holds the error, which ends the builder.
        let resolved = docs.resolved.lock().remove(&i).ok_or_else(|| Error::Plan("a file of a window that failed".to_owned()))?;
        let Some((content, outcome)) = resolved else { return Ok(None) };
        let (d, path, module) = &docs.asked[i];
        match outcome {
            Ok(entry) => Ok(Some((content, Ok(entry)))),
            Err(u) => {
                // A memo miss: the document's file-local part is evaluated again (`RERAIL_WARM`).
                if memoised {
                    crate::code_rule_dsl::logic::profile::warm_fault("a document missed the memo", || path.clone());
                }
                build_doc(d, path, module.clone(), u).map(|(c, digest, doc)| Some((c, Err((doc, digest)))))
            }
        }
    };
    // A chunk of partial hits' projection (decision 0225 §2): the relations its cone's rules read, and those the
    // run takes from every chunk (the base rows the cross-file part reads, the tree counts' constants and nodes, the
    // text tests' columns where the cone reads a test); none of the rest is made for its files.
    let cone_projections: parking_lot::Mutex<HashMap<BTreeSet<String>, Arc<syntax::Projection>>> = parking_lot::Mutex::new(HashMap::new());
    let cone_projection = |wanted: &BTreeSet<String>| -> Arc<syntax::Projection> {
        if let Some(p) = cone_projections.lock().get(wanted) {
            return Arc::clone(p);
        }
        let program = Arc::clone(cones.lock().entry(wanted.clone()).or_insert_with(|| Arc::new(wanted_cone(&local, wanted))));
        let v = &program.version;
        let mut reads: BTreeSet<String> = v.rules().flat_map(|r| r.reads().map(|a| v.rel_name(a.rel()).to_owned())).collect();
        // Not the base relations the memo holds for the cross-file part (`stored`): a chunk of partial hits hands on
        // its entries' rows of them, never the batch's (`Finish`), so they are not made for its files.
        reads.extend(constants.keys().map(|(r, _)| r.clone()));
        reads.extend(["node", "child", "doc"].map(str::to_owned));
        // A text test's rows are attributed to the files whose text holds their strings (`text_sources`).
        if reads.iter().any(|r| r.starts_with(crate::code_rule_dsl::logic::strings::RELATION)) {
            reads.extend(crate::code_rule_dsl::logic::strings::TEXT_COLUMNS.iter().map(|(r, _)| (*r).to_owned()));
        }
        let mut narrowed = projection.clone();
        let derived = &narrowed.derived;
        narrowed.reads = Some(reads.into_iter().filter(|r| !derived.contains(r) && projection.reads.as_ref().is_none_or(|all| all.contains(r))).collect());
        let narrowed = Arc::new(narrowed);
        cone_projections.lock().insert(wanted.clone(), Arc::clone(&narrowed));
        narrowed
    };
    let project = |docs: Vec<syntax::Document>| -> (Result<Sources>, (f64, f64)) {
        let from = thread_usage();
        let built = {
            let _batch = crate::code_rule_dsl::logic::profile::span(|| "worker batch".into());
            let coned = {
                let held = partials.lock();
                let keys: Vec<i64> = docs.iter().map(|d| i64::from(d.key)).collect();
                (!keys.is_empty() && keys.iter().all(|k| held.contains_key(k))).then(|| keys.iter().flat_map(|k| missing_of(&held[k])).collect::<BTreeSet<String>>())
            };
            let narrowed = coned.map(|wanted| cone_projection(&wanted));
            syntax::with_matchers(&store, docs, &sets, &chunk_matchers, &vocabulary, binding, narrowed.as_deref().unwrap_or(&projection), &HashMap::new()).map_err(|e| Error::Plan(e.to_string()))
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
        pending_docs: Vec<(i64, String, Option<memo::Content>, u64, usize, Option<[u8; 32]>)>,
        hits: Vec<(String, memo::Entry, Option<memo::Content>, Option<Arc<str>>)>,
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
        /// The contents read that are named by their digests (`memo::named_digests`).
        read_unstored: Vec<[u8; 32]>,
        parse: (f64, f64),
        batch: (f64, f64),
    }
    impl Cut {
        /// A document's content, for the run's manifest: its source, or its digest where the store was not asked.
        fn read(&mut self, content: Option<memo::Content>) {
            match content {
                Some(memo::Content::Stored(id)) => self.read_sources.push(id),
                Some(memo::Content::Unstored(d)) => self.read_unstored.push(d),
                Some(memo::Content::Held(_)) | None => {}
            }
        }
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
        build_jobs: std::sync::atomic::AtomicUsize,
        ready_hits: std::sync::atomic::AtomicUsize,
        /// At most this many documents built ahead of the next taken, and chunks projected at once beside the one
        /// waiting for the main thread.
        ahead: usize,
        /// Chunks projected ahead at most: the chunks allowed at once (Q74), and one more.
        in_flight: &'a std::sync::atomic::AtomicUsize,
        /// Without the memo, two (the one evaluating and the next); with it, the allowed count's.
        fixed_in_flight: Option<usize>,
        resolve: &'a parking_lot::Mutex<R>,
        build: &'a B,
        project: &'a P,
        run_docs: &'a std::sync::Mutex<input_delta::Docs>,
        target: &'a std::sync::atomic::AtomicUsize,
        keep_docs: bool,
        doing: &'a crate::code_rule_dsl::logic::profile::Activity,
        /// Rung when the builder may have handed on a chunk, and when a chunk's evaluation ends: the main thread, which
        /// begins and files the chunks in order, waits on it running no job (`Bell`).
        bell: rerail_accel::exec::Bell,
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
                let mut moved = false;
                while self.dirty.swap(false, SeqCst) {
                    let from = thread_usage();
                    while self.step(&mut c, jobs) {
                        moved = true;
                    }
                    let now = thread_usage();
                    c.parse.0 += now.0 - from.0;
                    c.parse.1 += now.1 - from.1;
                }
                drop(c);
                // Rung only when a step changed something: the main thread's wait polls by `advance`, and a ring for
                // nothing woke it at once, every time (Bell::wait_as sleeps only when nothing rang): a wait of 33 s on
                // the device's first pipeline spun on its core for 17 s of CPU (2026-11-17).
                if moved {
                    self.bell.ring();
                }
            }
        }

        /// The main thread's wait for `poll`, under `site`: on the bell, running no job, while the pool has other
        /// threads to take the jobs; with one thread, running them itself (none else would).
        fn main_wait<T>(&self, site: &'static str, poll: impl FnMut() -> Option<T>) -> T {
            if crate::threads::max() >= 2 {
                self.bell.wait_as(site, poll)
            } else {
                rerail_accel::exec::help_until_as(site, poll)
            }
        }

        /// The main thread's next message: a chunk, a batch's end (`None`) or an error; `Err` once the builder has
        /// ended. Meanwhile the main thread waits on the bell ([`Builder::main_wait`]): the documents' and chunks' jobs
        /// are the pool's other threads'.
        fn next<'s>(&'s self, jobs: &crate::threads::Jobs<'s, '_>) -> std::result::Result<Result<Option<Chunk>>, ()>
        where
            'a: 's,
        {
            self.main_wait("the main thread: its next chunk", || {
                // A stopped builder (a chunk's error) steps no more, so nothing it holds will ever be handed on: it
                // has ended. A load asked after a load's chunk failed waited here for ever, the pool idle (Q224: the
                // cone posts its requests ahead of their answers, and the next was answered before the error was read).
                if self.stopped.load(std::sync::atomic::Ordering::Relaxed) {
                    return Some(Err(()));
                }
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
            // Ready memo rows can be consumed before the resolver publishes its
            // new horizon. They need no job when that horizon becomes visible.
            c.spawned = c.spawned.max(c.next);
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
                        Ok(Ok(r)) => {
                            let (mut resolved, mut built) = (docs.resolved.lock(), docs.built.lock());
                            for (i, row) in (to..end).zip(r) {
                                match row {
                                    Some((content, Ok(entry))) => {
                                        self.ready_hits.fetch_add(1, Relaxed);
                                        built.insert(i, Ok(Some((content, Ok(entry)))));
                                    }
                                    None => { built.insert(i, Ok(None)); }
                                    miss => { resolved.insert(i, miss); }
                                }
                            }
                        }
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
                self.build_jobs.fetch_add(1, Relaxed);
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
            // Projected ahead of the main thread at most as many as evaluate at once (Q74): before a chunk is measured
            // that is one, so the chunks are not all cut to the probe's size, each a whole batch held.
            let in_flight = self.fixed_in_flight.unwrap_or_else(|| self.in_flight.load(std::sync::atomic::Ordering::Relaxed) + 1);
            let held = match c.out.front() {
                None => false,
                Some(Out::Ready(_)) => true,
                Some(Out::Projecting(id, _)) => projecting > in_flight || self.projected.lock().contains_key(id),
            };
            if held {
                self.doing.set(if matches!(c.out.front(), Some(Out::Projecting(..))) && projecting > in_flight { "wait for a chunk's projection" } else { "hand on (a chunk)" });
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
            let (d, path, module) = &docs.asked[i];
            match made {
                Ok(Some((source_id, Ok(entry)))) => {
                    c.read(source_id);
                    if let Some(content) = source_id {
                        self.run_docs.lock().expect("the run's documents").insert(path.clone(), (content, *d));
                    }
                    c.hits.push((path.clone(), entry, source_id, module.clone()));
                }
                Ok(Some((source_id, Err((doc, digest))))) => {
                    c.read(source_id);
                    if let Some(content) = source_id {
                        self.run_docs.lock().expect("the run's documents").insert(path.clone(), (content, *d));
                    }
                    c.pending_nodes += doc.facts.len();
                    c.pending_docs.push((*d, path.clone(), source_id, doc.facts.len() as u64, doc.facts.source.len(), digest));
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
            // A chunk whose rules ask for guests keeps its documents: the guests are made from them (decision 0055). So
            // does a chunk whose documents the corpus batch reads (decision 0458).
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
    // How many chunks evaluate at once: `RERAIL_CHUNKS_AT_ONCE`, else the pool's threads less the main thread's (which
    // files them), at most 8. One is the main thread alone, on its own connection, as before.
    let chunks_at_once = std::env::var("RERAIL_CHUNKS_AT_ONCE").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or_else(|| crate::threads::max().saturating_sub(1).min(8)).max(1);
    // Q88: what chunk `n` plans with is the run's as filed once chunks `0..=n - plan_window` were, whether the chunks
    // evaluate at once or one by one: the same plans either way, and whatever the threads' timing.
    let plan_window = crate::code_rule_dsl::logic::plan::PLAN_WINDOW.max(chunks_at_once);
    // The chunks begun, numbered in the order they are filed.
    let chunk_seq = std::cell::Cell::new(0usize);
    crate::code_rule_dsl::logic::profile::decision("chunks at once (decision 0254)", || format!("up to {chunks_at_once} chunks evaluate at once, each on a connection of its own, as many as the run's headroom holds (Q74); the main thread files them in order"));
    // How many may be in flight now (Q74): one until a chunk's measured growth sizes the rest, then as many as share
    // the run's headroom at `CHUNK_FLOOR_MB` or more each (the chunk-size decision sets it). The chunks in flight
    // together stay within it: 8 chunks each sized to the whole headroom took a cold run at 300 files to 2.3 GB.
    let at_once_allowed = std::sync::atomic::AtomicUsize::new(1);
    // The most a node was measured to add this run: a chunk whose growth the pages freed before it hid (22 bytes a
    // node, beside 9 KB measured next) sizes no chunk.
    let per_node_most = std::cell::Cell::new(0.0f64);
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
            read_unstored: Vec::new(),
            parse: (0.0, 0.0),
            batch: (0.0, 0.0),
        }),
        dirty: false.into(),
        stopped: false.into(),
        projected: Default::default(),
        jobs_cpu: Default::default(),
        build_jobs: 0.into(),
        ready_hits: 0.into(),
        ahead: builders * 8,
        // Beside the one waiting for the main thread, as many projecting as the chunks evaluating at once.
        in_flight: &at_once_allowed,
        fixed_in_flight: (!memoised).then_some(2),
        resolve: &resolver,
        build: &build,
        project: &project,
        run_docs,
        target: &worker_target,
        // And where the cross part reads base relations: they are read off every file's document (decision 0458).
        keep_docs: slice.is_some() || !stored.is_empty(),
        doing: &worker_doing,
        bell: Default::default(),
    };
    let builder = &builder;
    // A chunk's evaluation, from what the main thread hands it (`Task`) to what it hands back (`Evaluated`): its
    // guests' rounds, its sources bound and measured, its file-local program run, and its exports' rows read out of
    // the engine, on the connection it is given. It reads only what the run made before the chunks (unchanged while
    // they run) and its own, and keeps the measurements made once per mix behind their locks: the main thread files
    // what it hands back, in the chunks' order (`process`'s `Finish`), so several chunks evaluate at once, each on a
    // connection of its own (decision 0254).
    struct Task {
        chunk_nodes_built: usize,
        sources: Vec<(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)>,
        documents: Vec<syntax::Document>,
        stats: super::Stats,
        mix: String,
        /// The relations a chunk of partial hits derives (decision 0225 §2), and its guests' requests by round when
        /// its entries hold them (the slice not run).
        cone: Option<BTreeSet<String>>,
        replay: Option<Vec<BTreeMap<String, Arc<output::Rows>>>>,
        /// Its place in the order the chunks are filed, and its view of the run's plans (Q88).
        seq: usize,
        view: Option<Arc<crate::code_rule_dsl::logic::plan::ChunkView>>,
    }
    /// What the main thread files of a chunk once it is evaluated: its run's outcome, its guests' hosts and askers, its
    /// sources the memo attributes, the base totals it measured, and its exports' rows.
    struct Evaluated {
        got: Outcome,
        host_of: HashMap<i64, i64>,
        askers: HashMap<i64, Vec<i64>>,
        totals_add: Vec<(String, usize, Vec<(String, i64, i64, i64, i64)>)>,
        export_rows: Vec<(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)>,
        /// The slice's request lines of each round, for the entries (`memo::GUESTS`).
        slice_rounds: Vec<BTreeMap<String, Arc<output::Rows>>>,
        /// Its documents, its guests' with them, for the corpus batch the cross part reads its base relations off
        /// (decision 0458); none where the cross part reads none.
        documents: Vec<syntax::Document>,
    }
    /// What the main thread keeps of a chunk between its `Begin` and its `Finish`.
    struct Ctx {
        chunk_nodes_built: usize,
        chunk_docs: Vec<(i64, String, Option<memo::Content>, u64, usize, Option<[u8; 32]>)>,
        chunk_own: Option<Own>,
        start_mb: usize,
        start_max_mb: usize,
        /// Chunks evaluating when it began, itself included: the footprint it saw grow was theirs.
        at_once: usize,
        /// Its documents' entries, where every one is a partial hit, and the relations it derives (decision 0225 §2).
        chunk_partials: HashMap<i64, memo::Entry>,
        cone: Option<BTreeSet<String>>,
    }
    enum Step {
        Begin(Chunk, usize),
        /// An export handed off during an evaluation on the main thread (a run without the memo), and its rows.
        Hand(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>),
        Finish(Ctx, Evaluated),
    }
    type Hand<'h> = &'h mut dyn FnMut(&str, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>) -> Result<()>;
    let guest_postings_once = parking_lot::Mutex::new(guest_postings_once);
    let measured_once = parking_lot::Mutex::new(measured_once);
    let evaluate_in = |task: Task, db: &Connection, modules: &Modules, hand: Option<Hand<'_>>| -> Result<Evaluated> {
        let Task { chunk_nodes_built, sources, documents, mut stats, mix, cone, replay, seq, view: _ } = task;
        // A measurement made once per mix (Q88): taken from a chunk filed before this one began (`plan_window` back),
        // else made here, and kept for the chunks after as the earliest chunk's that made it.
        let once_seen = |by: usize| by + plan_window <= seq;
        let chunk_docs_n = documents.len();
        // The slice's request lines of each round, for the entries (`memo::GUESTS`).
        let mut slice_rounds: Vec<BTreeMap<String, Arc<output::Rows>>> = Vec::new();
        // Guests (decision 0055): the embed slice runs over the chunk, the guests it asks for are parsed, and the
        // batch is built again with them, until a round asks for none new. The batch bound below holds them.
        let mut host_of: HashMap<i64, i64> = HashMap::new();
        // A value document's files: those whose rules asked for it (decision 0058 §4, O96).
        let mut askers: HashMap<i64, Vec<i64>> = HashMap::new();
        let (sources, documents) = match &slice {
            Some(slice) if !documents.is_empty() => {
                let _guests = crate::code_rule_dsl::logic::profile::span(|| "guests".into());
                let documents_len = documents.len();
                let mut made = guests::Guests::new(label, projection.shape, Arc::clone(&guest_keys));
                // A chunk of partial hits runs its cone alone: it makes only the guests the cone can see (a rule that
                // names no guest sees no annotation or value, `guests::guests_seen`). Before, it made every guest
                // its entries asked for (on an edit of ep2, 27,373 guests and a second batch, 1.6 s of 12 s).
                if let (Some(_), Some(wanted)) = (&replay, &cone) {
                    let program = Arc::clone(cones.lock().entry(wanted.clone()).or_insert_with(|| Arc::new(wanted_cone(&local, wanted))));
                    made = made.seeing(guests::guests_seen(&program.version));
                }
                let (mut docs, mut sources) = (documents, sources);
                // The slice's planner statistics: what it reads, measured once and scaled to each round's rows, as the
                // run's are (`measured_once`). A relation measured with no rows is measured again once it has some:
                // a guest relation (`parsed`) is empty in the first round, and its rows are all a later round's.
                // Priced at none, `parsed($s, "annotation", $r)` joined every row of a delta before the literal that
                // binds `$s` (7.5 M rows to none, decision 0331).
                let mut measured: BTreeMap<String, (usize, Vec<(String, i64, i64, i64, i64)>)> = BTreeMap::new();
                // The rounds are one maintained program (decision 0331): what a round derived is kept for the next,
                // which runs from its inputs' new rows (`retain::grown_delta`). The round before's requests stand for
                // the outputs of the strata a round does not run; the tables the rounds bound are cleared after.
                let mut slice_kept: Option<retain::Retained> = None;
                let mut previous: BTreeMap<String, Arc<output::Rows>> = BTreeMap::new();
                let mut rounds_bound: BTreeSet<String> = BTreeSet::new();
                // The documents the last round made: a round's inputs gained their rows, and lost none (`owners_delta`).
                let mut round_new: rustc_hash::FxHashSet<i64> = Default::default();
                let slice_placed = slice.placed()?;
                let slice_f = slice_placed.fields()?;
                if replay.is_some() {
                    crate::code_rule_dsl::logic::profile::decision("guests made from the entries (decision 0225 §2)", || format!("a chunk of {chunk_docs_n} partial hits: the slice not run"));
                }
                for round_at in 0..guests::DEPTH {
                    // The round's requests: the entries' (`replay`), else the slice's run over the batch.
                    let asked: BTreeMap<String, Arc<output::Rows>> = match &replay {
                        Some(rounds) => rounds.get(round_at).cloned().unwrap_or_default(),
                        None => {
                            let mut round = stats.clone();
                            // The slice's inputs, each digested by its length: within a chunk's rounds the batch only
                            // gains documents, so a relation's rows changed exactly when there are more of them.
                            let mut inputs: Vec<(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, u64, bool)> = Vec::new();
                            for (rel, source) in &sources {
                                if slice_reads.contains(rel) {
                                    // Measured once per mix and relation (decision 0309), then scaled to each round's
                                    // rows, as the main run's statistics are: measured in every chunk, it was 10.5 s of a
                                    // cold PostHog run's CPU. A relation measured with no rows is measured again once it
                                    // has some, and an empty measurement is never the mix's (decision 0331: `parsed`,
                                    // empty in the first round, was priced at none in every later one).
                                    let rows = source.estimated_len();
                                    let m = match measured.get(rel) {
                                        Some((first, m)) if *first > 0 => scaled_statistics(m, *first, rows),
                                        Some((_, m)) if rows == 0 => m.clone(),
                                        _ => {
                                            let key = (format!("guest slice {mix}"), rel.clone());
                                            let once = measured_once.lock().get(&key).filter(|o| once_seen(o.0)).map(|o| (o.1, o.2.clone()));
                                            let (first, m) = match once {
                                                Some((first, m)) if first > 0 => (first, m),
                                                _ => {
                                                    let _m = crate::code_rule_dsl::logic::profile::span(|| "guest statistics".into());
                                                    let m = sampled_statistics_unanswered(rel, source.as_ref(), &base_shapes, stats.counts());
                                                    if rows > 0 {
                                                        once_keep(&mut measured_once.lock(), key, Once(seq, rows, m.clone()));
                                                    }
                                                    (rows, m)
                                                }
                                            };
                                            measured.insert(rel.clone(), (first, m.clone()));
                                            if first > 0 { scaled_statistics(&m, first, rows) } else { m }
                                        }
                                    };
                                    round.measured(rel, m.into_iter().map(|(c, rows, ndv, sq, sqd)| (c, [rows, ndv, sq, sqd])), []);
                                    inputs.push((rel.clone(), Arc::clone(source), source.len() as u64, true));
                                }
                                modules.bind(db, rel, Arc::clone(source))?;
                            }
                            let _round = crate::code_rule_dsl::logic::profile::span(move || if round_at == 0 { "the slice's first round".into() } else { format!("a later round, maintained (round {})", round_at + 1) });
                            let by_owner = |rel: &str, old: &dyn crate::code_rule_dsl::facts::vtab::Source, new: &dyn crate::code_rule_dsl::facts::vtab::Source| {
                                let node = slice_f.columns(rel).and_then(|c| c.iter().position(|(_, t)| *t == "node"));
                                node.and_then(|g| retain::owners_delta(rel, old, new, g, &|d| round_new.contains(&d))).or_else(|| retain::grown_delta(rel, old, new))
                            };
                            let ran = maintained::run_with(None, db, modules, slice, GUEST_ROUNDS_KEY, &inputs, slice_kept.take(), &mut round, &BTreeSet::new(), modes, &vocabulary, Some(&by_owner))?;
                            rounds_bound.extend(ran.bound);
                            slice_kept = Some(ran.next);
                            let mut out = ran.outputs;
                            for (rel, rows) in &previous {
                                out.entry(rel.clone()).or_insert_with(|| Arc::clone(rows));
                            }
                            out
                        }
                    };
                    previous = asked.clone();
                    if memoised {
                        slice_rounds.push(asked.iter().filter(|(r, _)| guests::LIBRARY.contains(&r.as_str()) || guests::DEMANDS.contains(&r.as_str())).map(|(r, l)| (r.clone(), Arc::clone(l))).collect());
                    }
                    let new = made.make(guests::requests(&asked), &docs, &guest_parses).map_err(Error::Plan)?;
                    if new.is_empty() {
                        break;
                    }
                    round_new = new.iter().map(|d| i64::from(d.key)).collect();
                    docs.extend(new);
                    // Made from the entries, the batch is built once, after the last round: no slice reads it between.
                    if replay.is_none() {
                        let _rebuild = crate::code_rule_dsl::logic::profile::span(|| "batch with guests".into());
                        sources = syntax::with_matchers(&store, docs.clone(), &sets, &chunk_matchers, &vocabulary, binding, &projection, &HashMap::new()).map_err(|e| Error::Plan(e.to_string()))?;
                    }
                }
                if replay.is_some() && docs.len() > documents_len {
                    let _rebuild = crate::code_rule_dsl::logic::profile::span(|| "batch with guests".into());
                    let narrowed = cone.as_ref().map(|wanted| cone_projection(wanted));
                    sources = syntax::with_matchers(&store, docs.clone(), &sets, &chunk_matchers, &vocabulary, binding, narrowed.as_deref().unwrap_or(&projection), &HashMap::new()).map_err(|e| Error::Plan(e.to_string()))?;
                }
                if made.unseen > 0 {
                    let unseen = made.unseen;
                    crate::code_rule_dsl::logic::profile::decision("guests the cone cannot see, not made", || format!("{unseen} in a chunk of {chunk_docs_n} partial hits"));
                }
                guests_unseen.fetch_add(made.unseen, std::sync::atomic::Ordering::Relaxed);
                // The rounds' relations are let go of: the file-local part derives its own, and the engine's are the
                // chunk's to its end otherwise.
                if slice_kept.take().is_some() {
                    for name in &rounds_bound {
                        if let Some(b) = modules.get(name) {
                            b.clear();
                        }
                    }
                    crate::code_rule_dsl::logic::native::component::forget_derived(db);
                }
                host_of = std::mem::take(&mut made.host_of);
                askers = made.askers();
                // The guest relations' constant postings, measured above before any guest was made (none then),
                // again over the batch with them: priced at none, `parsed($s, "annotation", $r)` read after bindings
                // looked like a filter, and each binding read every annotation guest (O143). Once per mix, scaled.
                if own && docs.len() > documents_len {
                    let once = guest_postings_once.lock().get(&mix).filter(|o| once_seen(o.0)).map(|o| (o.1, o.2.clone()));
                    let postings = match once.as_ref() {
                        Some((first, p)) if *first > 0 => p.iter().map(|(k, n)| (k.clone(), (*n as f64 * chunk_nodes_built as f64 / *first as f64).round() as i64)).collect(),
                        _ => {
                            let p = Own::postings(&sources, &constants, |r| guests::READS.contains(&r));
                            once_keep(&mut guest_postings_once.lock(), mix.clone(), Once(seq, chunk_nodes_built, p.clone()));
                            p
                        }
                    };
                    for ((rel, col, v), n) in postings {
                        stats.measured(&rel, [], [(col, v, n)]);
                    }
                    stats.estimated.extend(constants.keys().filter(|(rel, _)| guests::READS.contains(&rel.as_str())).map(|(rel, _)| rel.clone()));
                }
                (sources, docs)
            }
            _ => (sources, documents),
        };
        // Handed on to the corpus batch (decision 0458): the same documents, not their rows.
        let documents = if stored.is_empty() { Vec::new() } else { documents };
        let mut totals_add: Vec<(String, usize, Vec<(String, i64, i64, i64, i64)>)> = Vec::new();
        let mut bound: BTreeSet<String> = BTreeSet::new();
        for (rel, source) in sources {
            // A relation no rule reads is bound unmeasured: measuring reads every row, and a `py.*`
            // family's first read makes them (decision 0029).
            if !read.contains(&rel) {
                modules.bind(db, &rel, source)?;
                bound.insert(rel);
                continue;
            }
            let key = (mix.clone(), rel.clone());
            let once = measured_once.lock().get(&key).filter(|o| once_seen(o.0)).map(|o| (o.1, o.2.clone()));
            let measured = match once {
                Some((first, measured)) if first > 0 => scaled_statistics(&measured, first, source.estimated_len()),
                _ => {
                    let _measure = crate::code_rule_dsl::logic::profile::span(|| format!("measure {rel} ({mix})"));
                    let measured = sampled_statistics_unanswered(&rel, source.as_ref(), &base_shapes, stats.counts());
                    once_keep(&mut measured_once.lock(), key, Once(seq, source.estimated_len(), measured.clone()));
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
                totals_add.push((rel.clone(), source.estimated_len(), measured.clone()));
            }
            stats.measured(&rel, measured.into_iter().map(|(c, rows, ndv, sq, sqd)| (c, [rows, ndv, sq, sqd])), []);
            stats.lookup_costs(&rel, source.as_ref());
            modules.bind(db, &rel, source)?;
            bound.insert(rel);
        }
        let coned: Option<Arc<lower::Lowered>> = cone.as_ref().map(|wanted| {
            Arc::clone(cones.lock().entry(wanted.clone()).or_insert_with(|| Arc::new(wanted_cone(&local, wanted))))
        });
        if let Some(wanted) = &cone {
            crate::code_rule_dsl::logic::profile::decision("partial memo hits (decision 0225 §2)", || {
                format!("a chunk of {chunk_docs_n} documents derives {} of {} memoised relations ({} rules of {})", wanted.len(), memo_ids.len(), coned.as_ref().map_or(0, |c| c.version.size()), local_v.size())
            });
        }
        let local = coned.as_deref().unwrap_or(&local);
        let running = crate::code_rule_dsl::logic::profile::span(|| "run".into());
        // Without the memo (which attributes each row to its file once the run ends), an export the engine holds is
        // handed to the main thread (`Hand`) as soon as no later stratum reads it, and dropped: the chunk's exports
        // are not all held to its end.
        let got = match hand {
            None => texts_run(local, db, stats, split.exports.iter().filter(|r| cone.as_ref().is_none_or(|c| c.contains(*r))).cloned().collect(), modes, &vocabulary)?,
            Some(file) => {
                let mut hand = |rel: &str| -> Result<bool> {
                    if !split.exports.iter().any(|e| e == rel) {
                        return Ok(true);
                    }
                    let _span = crate::code_rule_dsl::logic::profile::span(|| "hand off an export (copy, statistics)".into());
                    let rows = crate::code_rule_dsl::logic::native::component::derived(db, rel).ok_or_else(|| Error::Plan(format!("{rel} is not held")))?;
                    file(rel, rows).map(|()| true)
                };
                super::run_atoms_native_hooked(local, db, "main", stats, split.exports.iter().cloned().collect(), modes, super::Hooks { hand_off: Some(&mut hand), texts: Some(Arc::clone(&vocabulary)), ..Default::default() })?
            }
        };
        drop(running);
        // The rows of its exports not handed off, read out before its relations are forgotten. A relation the engine
        // holds is read from it (an output is not written to a table to be read back); one a SQL stratum derived is in
        // its table.
        let mut export_rows: Vec<(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)> = Vec::new();
        for rel in split.exports.iter().filter(|r| !got.handed.contains(*r) && cone.as_ref().is_none_or(|c| c.contains(*r))) {
            let held = crate::code_rule_dsl::logic::native::component::derived(db, rel);
            let rows: Arc<dyn crate::code_rule_dsl::facts::vtab::Source> = match held {
                Some(rows) => rows,
                None if got.kept.contains(rel) => return Err(Error::Plan(format!("{rel} is not held"))),
                None => Arc::new(Rows::of(db, rel, &format!("temp.{}", crate::code_rule_dsl::logic::sql::ident(rel)))?),
            };
            export_rows.push((rel.clone(), rows));
        }
        // The chunk's native relations are forgotten, and its sources dropped before the connection's next chunk.
        crate::code_rule_dsl::logic::native::component::forget_derived(db);
        for rel in &bound {
            if let Some(b) = modules.get(rel) {
                b.clear();
            }
        }
        crate::code_rule_dsl::logic::native::forget_inputs(db);
        Ok(Evaluated { got, host_of, askers, totals_add, export_rows, slice_rounds, documents })
    };
    // A chunk's relations go in blocks of its own, given back with it (Q58): not in blocks every chunk at once
    // shares, which a few rows of each kept whole and the memory line counted whole.
    // Within its view of the run's plans (Q88): what it planned, learnt and observed comes back with it.
    let evaluate = |mut task: Task, db: &Connection, modules: &Modules, hand: Option<Hand<'_>>| -> (Result<Evaluated>, crate::code_rule_dsl::logic::plan::ChunkPlans) {
        let view = task.view.take();
        crate::code_rule_dsl::logic::native::setfix::in_chunk(|| match view {
            Some(v) => crate::code_rule_dsl::logic::plan::in_view(&v, || evaluate_in(task, db, modules, hand)),
            None => (evaluate_in(task, db, modules, hand), Default::default()),
        })
    };
    // The connections chunks evaluate on beside the main thread's, each opened as the run's is (its atoms, the
    // engine's functions, `x` attached when SQL first runs) and given the run's inputs bound before the chunks.
    let worker_inputs: Vec<(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)> = cross_inputs.clone();
    let attach_worker = format!("{attach_x} PRAGMA x.journal_mode = OFF; PRAGMA x.synchronous = OFF; PRAGMA temp_store = MEMORY;");
    let by_shift = t.is_none();
    let open_worker = || -> Result<(Connection, Modules)> {
        let _span = crate::code_rule_dsl::logic::profile::span(|| "open a chunk connection".into());
        let mut wdb = Connection::open(&path)?;
        crate::content_store::atoms::Atoms::install(&mut wdb)?;
        crate::code_rule_dsl::facts::vtab::defer_setup(&wdb, "PRAGMA synchronous=OFF;");
        super::ready_native(&wdb)?;
        if by_shift {
            super::docs_by_shift(&wdb);
        }
        crate::code_rule_dsl::logic::native::forget_inputs(&wdb);
        crate::code_rule_dsl::facts::vtab::defer_setup_undone(&wdb, &attach_worker, "DETACH x;");
        let wmodules = Modules::default();
        for (rel, source) in &worker_inputs {
            wmodules.bind(&wdb, rel, Arc::clone(source))?;
        }
        Ok((wdb, wmodules))
    };
    let worker_conns: parking_lot::Mutex<Vec<(Connection, Modules)>> = parking_lot::Mutex::new(Vec::new());
    // What the run wrote before its waves is a wave: the combined program, the prepared sources, the plan holds.
    crate::content_store::write_back::end_wave(crate::content_store::write_back::Boundary::Before);
    let worker_cpu = crate::threads::scope(|jobs| -> Result<((f64, f64), (f64, f64), Vec<crate::content_store::SourceId>, Vec<(String, crate::content_store::FileStat, memo::Content)>, Vec<[u8; 32]>)> {
    // The main thread's error or end stops the builder: no job it starts after is left running.
    let _stop = Stopper(&builder.stopped);
    // A chunk's file-local part: its memo hits replayed, its documents evaluated, its exports written to `x`.
    let mut process = |step: Step| -> Result<Option<(Task, Ctx)>> {
        let (ctx, ev) = match step {
            Step::Hand(rel, rows) => {
                let _s = crate::code_rule_dsl::logic::profile::span(|| "an export handed on".into());
                let types: Vec<&str> = pf.types(&rel);
                let inserted = exports.borrow_mut().add(&rel, types.len(), rows.as_ref());
                totals.add_rows(&rel, inserted, rows.as_ref(), &types);
                return Ok(None);
            }
            Step::Finish(ctx, ev) => (ctx, ev),
            Step::Begin(built, at_once) => {
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
                        let values: usize = hits.iter().map(|(_, e, ..)| e.rows.values().map(|(_, v)| v.len()).sum::<usize>()).sum();
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
                    for (path, mut entry, content, module) in hits {
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
                        // Read for its outputs alone: its export and base rows are the kept inputs' (decision 0117 step 1).
                        if kept_route && kept_hits.lock().contains_key(&path) {
                            hits_unmeasured = true;
                            memo_kept.0 += 1;
                            memo_kept.1 += entry.rows.iter().filter(|(r, _)| !r.starts_with("own:")).map(|(_, (_, v))| v.len()).sum::<usize>();
                        }
                        nodes += entry.nodes as usize;
                        // Its tree counts and base rows are the keeper's too, when counted.
                        if !counted {
                            own_totals.add_rows(&entry.rows);
                            if keeper.borrow().is_some() { own_seen.add_rows(&entry.rows); }
                        }
                        // Its document, for the corpus batch (decision 0458): made from its tape once the chunks are done, its
                        // guests from the requests its entry kept, round by round (moved out: nothing else reads them).
                        if !stored.is_empty() {
                            let keys: Vec<String> = entry.outputs.keys().filter(|k| memo::guest_round(k).is_some()).cloned().collect();
                            let rounds = keys.into_iter().filter_map(|k| {
                                let (_, rows) = entry.outputs.remove(&k)?;
                                let (round, rel) = memo::guest_round(&k)?;
                                Some((round, rel.to_owned(), rows))
                            }).collect();
                            corpus_hits.push(CorpusHit { key: entry.key, path: path.clone(), module: module.clone(), rounds });
                        }
                        // Gathered by relation across the batch's hits: one statement per relation below, not one per row.
                        let _gather = crate::code_rule_dsl::logic::profile::span(|| "export rows gathered".into());
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
                        for (rel, (columns, rows)) in entry.outputs.into_iter().filter(|(r, _)| !r.starts_with(memo::GUESTS)) {
                            outputs.entry(rel.clone()).or_default().push(Arc::new(rows));
                            node_columns.insert(rel, columns);
                        }
                    }
                    let _append = crate::code_rule_dsl::logic::profile::span(|| "exports appended".into());
                    for (rel, (width, at, own_cols, cols)) in batch {
                        if let Some(at) = at {
                            exports.borrow_mut().append_own(&rel, width, at, own_cols, own_docs.iter().copied());
                        }
                        if cols.first().is_none_or(Vec::is_empty) {
                            hits_unmeasured |= unchecked;
                            continue;
                        }
                        let types: Vec<&str> = pf.types(&rel);
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
                // A chunk of memo hits alone builds no document: its sources are a projection of nothing, and its local program
                // run over them derives nothing a hit does not already hold (`RERAIL_WARM` found it planned and run, its tree
                // counts measured and written, on every warm run).
                if sources.is_empty() || chunk_docs.is_empty() {
                    phase("local", statements.count());
                    statements.enter("wait");
                    // Its replayed hits' writes are the chunk's wave (decision 0259).
                    crate::content_store::write_back::end_wave(crate::content_store::write_back::Boundary::Chunk);
                    return Ok(None);
                }
                // Documents to evaluate: the prices are read beside them (`costs::begin_read`), before their first
                // wave asks. A chunk of memo hits alone prices nothing (decision 0285).
                crate::code_rule_dsl::logic::costs::begin_read();
                // A warm run's documents are all memo hits: a chunk evaluated is a file-local part computed again (`RERAIL_WARM`).
                crate::code_rule_dsl::logic::profile::warm_fault("a chunk's file-local part evaluated", || {
                    format!("{} documents, {chunk_nodes_built} nodes: {}", chunk_docs.len(), chunk_docs.iter().take(4).map(|(_, p, ..)| p.as_str()).collect::<Vec<_>>().join(", "))
                });
                nodes += chunk_nodes_built;
                chunks += 1;
                let mut stats = t_stats.clone();
                repository_stats(&mut stats);
                let mix = chunk_mix(chunk_docs.iter().map(|(_, path, _, _, bytes, _)| (path.as_str(), *bytes)));
                let mut chunk_own: Option<Own> = None;
                if own {
                    let chunk = match own_once.get(&mix) {
                        Some((first, measured)) if *first > 0 => measured.scaled(chunk_nodes_built as f64 / *first as f64),
                        _ => {
                            let _measure = crate::code_rule_dsl::logic::profile::span(|| format!("measure constants ({mix})"));
                            let measured = Own::measure(&sources, &constants);
                            own_once.insert(mix.clone(), (chunk_nodes_built, measured.clone()));
                            measured
                        }
                    };
                    chunk.apply(&mut stats);
                    // Counted over a sample of the mix's first chunk: a constant missing there is unknown here.
                    stats.estimated.extend(constants.keys().map(|(rel, _)| rel.clone()));
                    own_totals.add(&chunk);
                    chunk_own = Some(chunk);
                }
                // A chunk whose every document is a partial hit derives only what their entries do not hold (decision
                // 0225 §2): the cone of the relations a rule's edit reached. Their other relations are the entries'
                // (`Finish`).
                let chunk_partials: HashMap<i64, memo::Entry> = if memoised {
                    let mut held = partials.lock();
                    chunk_docs.iter().filter_map(|(d, ..)| held.remove(d).map(|e| (*d, e))).collect()
                } else {
                    HashMap::new()
                };
                let cone: Option<BTreeSet<String>> = (!chunk_partials.is_empty() && chunk_partials.len() == chunk_docs.len())
                    .then(|| chunk_partials.values().flat_map(&missing_of).collect());
                // Their guests' requests, by round, when every entry holds them under the slice's identities now: the
                // guests are made from them, and the slice does not run (decision 0225 §2).
                let replay: Option<Vec<BTreeMap<String, Arc<output::Rows>>>> = match (&cone, &slice) {
                    (Some(_), Some(_)) if chunk_partials.values().all(|e| {
                        let held = memo::covered(e);
                        slice_ids.values().all(|id| held.contains(&memo::covered_words(id)))
                    }) => {
                        let mut rounds: Vec<BTreeMap<String, Arc<output::Rows>>> = Vec::new();
                        for e in chunk_docs.iter().filter_map(|(d, ..)| chunk_partials.get(d)) {
                            for (key, (_, rows)) in &e.outputs {
                                let Some((r, rel)) = memo::guest_round(key) else { continue };
                                if rounds.len() <= r {
                                    rounds.resize_with(r + 1, Default::default);
                                }
                                let round = rounds[r].entry(rel.to_owned()).or_insert_with(|| Arc::new(output::Rows::new(rows.types().to_vec())));
                                Arc::make_mut(round).append(rows);
                            }
                        }
                        Some(rounds)
                    }
                    _ => None,
                };
                phase("local", statements.count());
                statements.enter("wait");
                let task = Task { chunk_nodes_built, sources, documents, stats, mix, cone: cone.clone(), replay, seq: 0, view: None };
                return Ok(Some((task, Ctx { chunk_nodes_built, chunk_docs, chunk_own, start_mb, start_max_mb, at_once, chunk_partials, cone })));
            }
        };
        phase("wait", statements.count());
        statements.enter("local");
        let Ctx { chunk_nodes_built, chunk_docs, chunk_own, start_mb, start_max_mb, at_once, mut chunk_partials, cone } = ctx;
        let Evaluated { got, host_of, askers, totals_add, export_rows, slice_rounds, documents } = ev;
        corpus_docs.extend(documents);
        for (rel, rows, measured) in &totals_add {
            totals.add(rel, *rows, measured);
        }
        // The files a row is attributed to: its document's (a guest's is its host file's), or every file that asked
        // for its value document. Rows are sets, so a value's rows kept by each of its files restore as one.
        let owners = |v: i64| -> Vec<i64> {
            let d = memo::document_of(v);
            match askers.get(&d) {
                Some(files) => files.clone(),
                None => vec![host_of.get(&d).copied().unwrap_or(d)],
            }
        };
        if chunk_nodes_built > 0 {
            // What this chunk added at its peak (the lifetime peak, when it rose here; else the footprint now).
            let (now, max) = (crate::code_rule_dsl::prof::footprint_mb(), crate::code_rule_dsl::prof::max_footprint_mb());
            let added = if max > start_max_mb { max } else { now }.saturating_sub(start_mb).max(1);
            let measured = added as f64 * 1048576.0 / (chunk_nodes_built * at_once) as f64;
            // Credible only above the noise of pages reused: until one is, one chunk at a time, of the probe's size.
            let credible = added >= CHUNK_CREDIBLE_MB;
            if credible {
                per_node_most.set(per_node_most.get().max(measured));
            }
            let per_node = per_node_most.get();
            let env = |k: &str, d: f64| std::env::var(k).ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(d);
            // Three quarters of what is left of the run's budget, shared by the chunks in flight together.
            let headroom = (env("RERAIL_RUN_MB", RUN_MB) - start_mb as f64).max(0.0) * 0.75;
            let chunk = env("RERAIL_CHUNK_MB", CHUNK_MB);
            let allowed = if per_node > 0.0 { ((headroom / env("RERAIL_CHUNK_FLOOR_MB", CHUNK_FLOOR_MB)).floor() as usize).clamp(1, chunks_at_once) } else { 1 };
            let budget = chunk.min(headroom / allowed as f64).round();
            let next = if per_node > 0.0 { ((budget * 1048576.0 / per_node) as usize).clamp(MIN_CHUNK_NODES.min(chunk_nodes), chunk_nodes) } else { chunk_nodes.min(PROBE_NODES) };
            // `RERAIL_CHUNK_NODES` (a measurement aid): every chunk after the first of this many nodes (at most
            // `chunk_nodes`), whatever the footprint measured, so two runs cut the corpus alike (Q88).
            let next = std::env::var("RERAIL_CHUNK_NODES").ok().and_then(|v| v.parse::<usize>().ok()).map_or(next, |n| n.clamp(1, chunk_nodes));
            chunk_target.store(next, std::sync::atomic::Ordering::Relaxed);
            at_once_allowed.store(allowed, std::sync::atomic::Ordering::Relaxed);
            // The device pool's share of the footprint the chunk was sized under: its blocks leased and idle.
            #[cfg(target_os = "macos")]
            crate::code_rule_dsl::logic::profile::decision("chunk size: the device pool", || {
                let (live, idle) = rerail_accel::gpu::shared::held_bytes();
                let h = crate::code_rule_dsl::facts::rows::held();
                format!(
                    "{} MB leased, {} MB idle; rows' blocks: a run's {} ({} MB written of {} MB), sources' {} ({} MB of {} MB), kept rows' {} ({} MB of {} MB); {} MB of dead relations' pages given back; {} relations of many parts merged into one ({} MB copied); under a footprint of {start_mb} MB",
                    live >> 20,
                    idle >> 20,
                    h[0].0, h[0].1 >> 20, h[0].2 >> 20,
                    h[1].0, h[1].1 >> 20, h[1].2 >> 20,
                    h[2].0, h[2].1 >> 20, h[2].2 >> 20,
                    crate::code_rule_dsl::facts::rows::given_back() >> 20,
                    crate::code_rule_dsl::facts::rows::merged().0,
                    crate::code_rule_dsl::facts::rows::merged().1 >> 20,
                )
            });
            crate::code_rule_dsl::logic::profile::decision("chunk size", || if credible {
                format!("a chunk of {chunk_nodes_built} nodes added {added} MB ({measured:.0} bytes a node, {per_node:.0} the most this run); the next are about {next} nodes, {allowed} at once ({budget} MB a chunk: the least of {chunk} MB and three quarters of the run's headroom above {start_mb} MB shared by the chunks at once; at most {chunk_nodes})")
            } else {
                format!("a chunk of {chunk_nodes_built} nodes added {added} MB, under {CHUNK_CREDIBLE_MB} MB (pages freed before it reused): it sizes nothing; the next are about {next} nodes, {allowed} at once")
            });
            // The memory line, not the chunk's own size, set it: every stratum and matcher launch runs per chunk, so
            // a cut chunk is a smaller grain for the device and more runs for the same work (the profile's warnings).
            if headroom < chunk / 4.0 {
                crate::code_rule_dsl::logic::profile::decision("chunk cut by the run's memory line", || format!("under a quarter of {chunk} MB left by RERAIL_RUN_MB ({} MB)", env("RERAIL_RUN_MB", RUN_MB)));
            }
        }
        let outputs_of = |rel: &str| head_outputs.iter().any(|o| o == rel);
        // Per document of the chunk, what it hands on (`memo::Entry`), when the run is memoised. A row is its
        // document's by its first location column.
        let mut entries: HashMap<i64, memo::Entry> = if memoised { chunk_docs.iter().map(|(d, _, _, n, _, _)| (*d, memo::Entry { nodes: *n, ..memo::Entry::default() })).collect() } else { HashMap::new() };
        let mut attributable = memoised && chunk_own.is_some();
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
            for (rel, rows) in got.outputs.iter().filter(|(rel, _)| outputs_of(rel)) {
                let at = split.locations.get(rel).and_then(|l| l.first().copied());
                // A memo entry holds a file's output as its rows, typed: each row in the entry of the document its
                // location's node is of.
                let mut cells: Vec<output::Cell<'_>> = Vec::with_capacity(rows.width());
                for i in 0..rows.len() {
                    let files = at.filter(|&c| c < rows.width()).and_then(|c| match rows.cell(i, c) { output::Cell::Int(v) => Some(v), output::Cell::Text(_) => None }).map(owners).unwrap_or_default();
                    if files.is_empty() || !files.iter().all(|d| entries.contains_key(d)) {
                        memo_why.insert(format!("output {rel}: {}", if files.is_empty() { "no location" } else { "another chunk's document" }));
                        attributable = false;
                        continue;
                    }
                    cells.clear();
                    cells.extend((0..rows.width()).map(|c| rows.cell(i, c)));
                    for d in files {
                        let e = entries.get_mut(&d).expect("checked");
                        e.outputs.entry(rel.clone()).or_insert_with(|| (got.nodes[rel].clone(), output::Rows::new(rows.types().to_vec()))).1.push(&cells);
                    }
                }
            }
        }
        // Each file's guests' requests, by the location each asks at (`memo::GUESTS`): the entry holds them under
        // the slice's identities when every line names one of the chunk's files.
        let mut guests_held = memoised && slice.is_some() && !slice_rounds.is_empty();
        if guests_held {
            // Each file's rows of each round's relation, typed as the slice handed them, each once.
            let mut by_file: HashMap<i64, BTreeMap<String, (output::Rows, std::collections::HashSet<Vec<String>>)>> = HashMap::new();
            'rounds: for (r, outputs) in slice_rounds.iter().enumerate() {
                for (rel, rows) in outputs {
                    for i in 0..rows.len() {
                        let files = guests::asked_at(rel, rows, i).map(owners).unwrap_or_default();
                        if files.is_empty() || !files.iter().all(|d| entries.contains_key(d)) {
                            guests_held = false;
                            break 'rounds;
                        }
                        let cells: Vec<output::Cell<'_>> = (0..rows.width()).map(|c| rows.cell(i, c)).collect();
                        let shown: Vec<String> = (0..rows.width()).map(|c| match rows.cell(i, c) { output::Cell::Text(t) => t.to_owned(), output::Cell::Int(v) => v.to_string() }).collect();
                        for d in files {
                            let (held, seen) = by_file.entry(d).or_default().entry(memo::guest_key(r, rel)).or_insert_with(|| (output::Rows::new(rows.types().to_vec()), std::collections::HashSet::new()));
                            if seen.insert(shown.clone()) {
                                held.push(&cells);
                            }
                        }
                    }
                }
            }
            if guests_held {
                for (d, keys) in by_file {
                    let e = entries.get_mut(&d).expect("checked");
                    for (key, (rows, _)) in keys {
                        e.outputs.insert(key, (Vec::new(), rows));
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
        // Exports remain in native memory; this boundary performs no SQL writes. Rows another chunk wrote too (a
        // file-local relation without a location column) are kept once; a kept export is measured in place.
        for (rel, rows) in &export_rows {
            let types: Vec<&str> = pf.types(rel);
            let inserted = { let _s = crate::code_rule_dsl::logic::profile::span(|| "hold the export".into()); exports.borrow_mut().add(rel, types.len(), rows.as_ref()) };
            { let _s = crate::code_rule_dsl::logic::profile::span(|| "totals".into()); totals.add_rows(rel, inserted, rows.as_ref(), &types); }
            { let _s = crate::code_rule_dsl::logic::profile::span(|| "attribute to files".into()); attribute(rel, rows.as_ref()); }
        }
        // A partial hit's relations its chunk did not derive: its entry's rows and lines, handed on as an evaluated
        // file's are, and kept in its new entry (decision 0225 §2).
        if let Some(cone) = &cone {
            let _s = crate::code_rule_dsl::logic::profile::span(|| "the partial hits' rows".into());
            // As a full hit's (`memo replay`): a row of its own document is appended unchecked, the others (a value
            // document's, which each file that asked for it holds) kept once through the export's set.
            let unchecked = keeper.borrow().as_ref().is_some_and(|k| !k.failed());
            // In the chunk's order, as a hit's rows are: the export's rows, and so its digest, as an earlier run's.
            let docs: Vec<i64> = chunk_docs.iter().map(|(d, ..)| *d).filter(|d| chunk_partials.contains_key(d)).collect();
            for rel in split.exports.iter().filter(|r| !cone.contains(*r)) {
                let types: Vec<&str> = pf.types(rel);
                let width = types.len();
                let at = split.locations.get(rel).and_then(|l| l.first().copied()).filter(|a| *a < width && unchecked);
                let (mut own_cols, mut shared_cols): (Vec<Vec<i64>>, Vec<Vec<i64>>) = (vec![Vec::new(); width], vec![Vec::new(); width]);
                // Moved out of the old entry into the new one: only the cone's relations are read of the old entry
                // after this (`standing`).
                for d in &docs {
                    let Some((w, values)) = chunk_partials.get_mut(d).and_then(|e| e.rows.remove(rel)) else { continue };
                    let w = &w;
                    if *w != width {
                        return Err(Error::Plan(format!("{rel}: a memo entry's rows are {w} wide, its declaration {width}")));
                    }
                    for row in values.chunks_exact(width.max(1)) {
                        let cols = if at.is_some_and(|a| memo::document_of(row[a]) == *d) { &mut own_cols } else { &mut shared_cols };
                        for (c, v) in row.iter().enumerate() {
                            cols[c].push(*v);
                        }
                    }
                    if let Some(entry) = entries.get_mut(d) {
                        entry.rows.insert(rel.clone(), (width, values));
                    }
                }
                let names: Vec<String> = (0..width).map(|c| format!("c{c}")).collect();
                if let Some(at) = at {
                    exports.borrow_mut().append_own(rel, width, at, own_cols, docs.iter().copied());
                }
                if shared_cols.first().is_none_or(Vec::is_empty) {
                    hits_unmeasured |= unchecked;
                    continue;
                }
                let rows = crate::code_rule_dsl::logic::native::component::columns_source(names, shared_cols.into_iter().map(Some).collect());
                let inserted = exports.borrow_mut().add(rel, width, rows.as_ref());
                if unchecked {
                    hits_unmeasured = true;
                } else {
                    totals.add_rows(rel, inserted, rows.as_ref(), &types);
                }
            }
            // Their outputs' rows, as a hit's are (`memo replay`): into the run's outputs, and kept in the new entries.
            for d in &docs {
                let Some(e) = chunk_partials.get_mut(d) else { continue };
                let moved: Vec<String> = e.outputs.keys().filter(|r| !cone.contains(*r) && !r.starts_with(memo::GUESTS)).cloned().collect();
                for rel in moved {
                    let (columns, rows) = e.outputs.remove(&rel).expect("listed");
                    let rows = Arc::new(rows);
                    outputs.entry(rel.clone()).or_default().push(Arc::clone(&rows));
                    node_columns.insert(rel.clone(), columns.clone());
                    if let Some(entry) = entries.get_mut(d) {
                        entry.outputs.insert(rel, (columns, Arc::unwrap_or_clone(rows)));
                    }
                }
            }
        }
        if memoised && attributable {
            let _remember = crate::code_rule_dsl::logic::profile::span(|| "memo remember".into());
            // The chunk's entries are written as one wave, not one transaction per document.
            let mut publish: Vec<(memo::Content, String, i64, memo::Entry, bool, Option<[u8; 32]>)> = Vec::with_capacity(chunk_docs.len());
            // A partial hit whose relations the chunk derived came out as its entry held them: its stored rows frame
            // stands, and only the identities they now hold are written (decision 0225 §2).
            let standing = |d: &i64, entry: &memo::Entry| cone.as_ref().zip(chunk_partials.get(d)).is_some_and(|(cone, was)| cone.iter().all(|r| memo::same_relation(entry, was, r)) && memo::same_guests(entry, was));
            for (d, path, source_id, _, _, digest) in &chunk_docs {
                // Counted in before it is remembered: the content it replaces is the memo's still.
                if let (Some(k), Some(entry), Some(_)) = (keeper.borrow_mut().as_mut(), entries.get(d), memo) {
                    let rows: usize = entry.rows.values().map(|(_, v)| v.len()).sum();
                    let _keep = crate::code_rule_dsl::logic::profile::span(|| format!("keep a file's entry ({rows} values)"));
                    own_seen.add(&Own::taken(&entry.rows));
                    k.file(path, *source_id, *d, entry, &|c, key| counted_entry(path, c, key));
                }
                // Read no more here: handed to the store's writer (decision 0102). It holds every relation under its
                // identity now: derived, or (a partial hit's) kept from its entry.
                let (Some(id), Some(mut entry)) = (source_id, entries.remove(d)) else { continue };
                memo::cover(&mut entry, memo_ids.values().chain(slice_ids.values().filter(|_| guests_held)));
                let stands = standing(d, &entry);
                publish.push((*id, world::store_path(path).to_owned(), *d, entry, stands, *digest));
            }
            // Decision 0458 R4: no entry keeps a base relation's rows; the cross part reads them off the corpus batch.
            if let Some((path, rel)) = publish.iter().find_map(|(_, p, _, e, _, _)| e.rows.keys().find(|k| k.starts_with("base:") || stored.iter().any(|(r, _)| r == *k)).map(|k| (p, k))) {
                return Err(Error::Plan(format!("{path}'s memo entry keeps rows of the base relation {rel} (decision 0458 R4)")));
            }
            if let Some(memo) = memo {
                let n = publish.len();
                let _publish = crate::code_rule_dsl::logic::profile::span(|| format!("publish {n} documents' entries"));
                let standing = publish.iter().filter(|e| e.4).count();
                if cone.is_some() {
                    crate::code_rule_dsl::logic::profile::decision("partial memo hits written (decision 0225 §2)", || format!("{standing} of {n} entries' rows stand as stored; the others written again"));
                }
                if let memo::Memo::Stored(store) = memo
                    && !publish.is_empty()
                    && last_unmarked.replace(false)
                {
                    // This run started from what the last run of its program kept (`last_unmarked`): a stop leaves it
                    // valid for the next.
                    memo::set_under_way(store, &memo_program, memo_whole);
                }
                memo.remember_all_but(&memo_plan, publish).map_err(|e| Error::Plan(format!("memo publication of {n} documents: {e}")))?;
                #[cfg(any(test, feature = "test-hooks"))]
                if phases::current().is_none() && STOP_AFTER_ENTRIES.with(|s| s.replace(false)) {
                    // As a run killed after its first entries: they and the marker reach the store, nothing after.
                    if let memo::Memo::Stored(store) = memo {
                        store.written().map_err(|e| Error::Plan(e.to_string()))?;
                    }
                    return Err(Error::Plan("stopped after its first entries (a test's)".into()));
                }
            }
        } else if memoised {
            memo_unattributed += 1;
            if let Some(k) = keeper.borrow_mut().as_mut() {
                k.fail("a chunk's rows not attributed to its files");
            }
        }
        phase("local", statements.count());
        statements.enter("wait");
        // The chunk's writes are one wave, sealed as the main thread files it (decision 0259; filed in order, 0254):
        // its tree counts and memo entries, with what the pool queued since the last boundary (the builders' tapes).
        crate::content_store::write_back::end_wave(crate::content_store::write_back::Boundary::Chunk);
        Ok(None)
    };
    // One batch's chunks, to its end; false when the builder has ended. With `parallel`, up to `chunks_at_once`
    // chunks evaluate at once as jobs, each on a connection of its own, and are filed here in their order; else each
    // evaluates here, on the run's connection.
    let drain = |process: &mut dyn FnMut(Step) -> Result<Option<(Task, Ctx)>>, parallel: bool| -> Result<bool> {
        let k = if parallel && memoised { chunks_at_once } else { 1 };
        type Slot = Arc<parking_lot::Mutex<Option<(Result<Evaluated>, crate::code_rule_dsl::logic::plan::ChunkPlans)>>>;
        let mut flight: std::collections::VecDeque<(Ctx, Slot)> = std::collections::VecDeque::new();
        // The run's plans as the chunks are filed (Q88): each chunk's view, and what it made taken in in their order.
        let mut ordered = crate::code_rule_dsl::logic::plan::OrderedPlans::begin(plan_window);
        // Files the chunk at the front once it is evaluated, waiting for it (running the pool's jobs meanwhile).
        let file_front = |flight: &mut std::collections::VecDeque<(Ctx, Slot)>, ordered: &mut Option<crate::code_rule_dsl::logic::plan::OrderedPlans>, process: &mut dyn FnMut(Step) -> Result<Option<(Task, Ctx)>>| -> Result<()> {
            let (ctx, slot) = flight.pop_front().expect("a chunk in flight");
            main_doing.set("wait for a chunk evaluated");
            let (ev, made) = builder.main_wait("the main thread: a chunk evaluated", || slot.lock().take());
            if let Some(o) = ordered.as_mut() {
                o.file(made);
            }
            process(Step::Finish(ctx, ev?)).map(drop)
        };
        let got = (|| loop {
            // The chunks evaluated at the front are filed as they are ready: their memory goes back.
            while flight.front().is_some_and(|(_, s)| s.lock().is_some()) {
                file_front(&mut flight, &mut ordered, process)?;
            }
            main_doing.set("wait");
            let next = crate::code_rule_dsl::logic::profile::wait("the worker's next chunk", &worker_doing, || builder.next(jobs));
            let chunk = match next {
                Err(()) | Ok(Ok(None)) => {
                    while !flight.is_empty() {
                        file_front(&mut flight, &mut ordered, process)?;
                    }
                    return Ok(next.is_ok());
                }
                Ok(Err(e)) => return Err(e),
                Ok(Ok(Some(chunk))) => chunk,
            };
            let Some((mut task, ctx)) = process(Step::Begin(chunk, flight.len() + 1))? else { continue };
            task.seq = chunk_seq.get();
            chunk_seq.set(task.seq + 1);
            task.view = ordered.as_mut().map(|o| o.next());
            if k == 1 {
                let (ev, made) = if memoised {
                    evaluate(task, &db, &modules, None)
                } else {
                    evaluate(task, &db, &modules, Some(&mut |rel: &str, rows| process(Step::Hand(rel.to_owned(), rows)).map(drop)))
                };
                if let Some(o) = ordered.as_mut() {
                    o.file(made);
                }
                process(Step::Finish(ctx, ev?))?;
                continue;
            }
            let slot: Slot = Arc::default();
            let into = Arc::clone(&slot);
            let (evaluate, open_worker, worker_conns, bell) = (&evaluate, &open_worker, &worker_conns, &builder.bell);
            jobs.spawn("a chunk evaluated", move |_| {
                let taken = worker_conns.lock().pop();
                // A panic is the chunk's error: the main thread waits for its slot.
                let got = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match taken.map_or_else(open_worker, Ok) {
                    Err(e) => (Err(e), Default::default()),
                    Ok((wdb, wmodules)) => {
                        // Beside the other chunks: its joins walk whole, the pool is busy with chunks.
                        let r = crate::code_rule_dsl::logic::native::setfix::walks_whole(|| evaluate(task, &wdb, &wmodules, None));
                        worker_conns.lock().push((wdb, wmodules));
                        r
                    }
                }))
                .unwrap_or_else(|_| (Err(Error::Plan("a chunk's evaluation panicked".to_owned())), Default::default()));
                *into.lock() = Some(got);
                bell.ring();
            });
            flight.push_back((ctx, slot));
            // As many in flight as the run's headroom holds (Q74), at most `k`.
            while flight.len() >= at_once_allowed.load(std::sync::atomic::Ordering::Relaxed).clamp(1, k) {
                file_front(&mut flight, &mut ordered, process)?;
            }
        })();
        if got.is_err() {
            builder.stop();
        }
        got
    };
    drain(&mut process, true)?;
    // The chunks' connections are closed beside the run once the loads' chunks are evaluated too: what follows runs
    // on the run's.
    let close_workers = || for (wdb, wmodules) in std::mem::take(&mut *worker_conns.lock()) {
        drop(wmodules);
        close_beside(wdb);
    };
    // Requests answered inside the fixpoint (decision 0065 §2a). The cone of requests runs on a connection of its
    // own, in a thread of its own, and pauses; this thread answers: a directory's entries and a file's lines from
    // `World`, and a load by sending the document through the chunk path, as any file goes (the memo by content,
    // else parsed), then handing on its rows of the per-file relations the cone reads.
    if let (Some((load, reads)), Some(docs_tx)) = (&loading, &docs_out) {
        let _loading = crate::code_rule_dsl::logic::profile::span(|| "requests answered inside the fixpoint".into());
        let mut world = world::World::new(root, head);
        let corpus: BTreeSet<String> = world.canonical_corpus(docs.iter().map(|(_, p)| p));
        let mut all_paths: Vec<String> = docs.iter().map(|(_, p)| p.clone()).collect();
        // Read from the cone's version: its answers (by request, in order), widths and inputs.
        let load_placed = super::body::Placed::whole(load.clone());
        let load_f = load_placed.fields()?;
        let load_answers: Vec<(&str, &str)> = load_f.answers().iter().map(|&(q, a)| (load_f.rel_name(q), load_f.rel_name(a))).collect();
        let mut per_file: Vec<String> = load_answers.iter().filter(|(q, a)| *q == world::LOAD && split.exports.iter().any(|e| e == a)).map(|(_, a)| (*a).to_owned()).collect();
        // A cone that loads nothing (its requests the file system's, or a language's) still reads the exports its
        // rules read: the corpus's rows of them, bound as a load's would be. Their requests are keyed by them (a C
        // file's directives asked for each document's path).
        if !load_answers.iter().any(|(q, _)| *q == world::LOAD) {
            per_file = load.rules().flat_map(|r| r.reads()).map(|a| load.rel_name(a.rel()).to_owned()).filter(|r| split.exports.contains(r)).collect::<BTreeSet<_>>().into_iter().collect();
        }
        let world_rels: Vec<(String, usize)> = load_answers.iter().map(|(_, a)| *a).filter(|r| !split.exports.iter().any(|e| e == r)).filter_map(|r| load_f.arity(r).map(|w| (r.to_owned(), w))).collect();
        // Those the cone takes as inputs start empty; the others are its own tables.
        let world_inputs: Vec<(String, usize)> = world_rels.iter().filter(|(r, _)| load_f.is_input(r)).cloned().collect();
        // What the rest of the cross-file part reads of the cone; what was answered is kept here, as it is given.
        let kept: BTreeSet<String> = reads.clone();
        let mut answered: BTreeMap<String, Vec<Vec<i64>>> = world_rels.iter().map(|(r, _)| (r.clone(), Vec::new())).collect();
        let lowered_load = part_of(load.clone());
        let load_stats = t_stats.clone();
        let (ask_tx, ask_rx) = std::sync::mpsc::channel::<(u64, String, Vec<Vec<i64>>)>();
        let (reply_tx, reply_rx) = std::sync::mpsc::channel::<(u64, std::result::Result<Vec<(String, Vec<Vec<i64>>)>, String>)>();
        // The rows the cone starts from: the counted exports less those of the files not seen yet (gone, or a load
        // of the last run's, not yet loaded), when the chunks did not hold the hits' rows again; from here on they do.
        let counted_view: Option<Vec<(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)>> = {
            let k = keeper.borrow();
            match (k.as_ref().filter(|k| k.exports_held() && !held_by_count.borrow().is_empty()), memo) {
                (Some(k), Some(_)) => {
                    let mut minus = exports::Counted::default();
                    let whole = k.unseen().into_iter().all(|(path, c, key)| match counted_entry(&path, c, key) {
                        Some(e) => per_file.iter().all(|rel| e.rows.get(rel).is_none_or(|(w, v)| minus.patch(rel, *w, v, 1))),
                        None => false,
                    });
                    let counted = k.counted_exports().expect("held");
                    whole.then(|| per_file.iter().map(|r| (r.clone(), counted.source_without(r, pf.width(r), &minus))).collect())
                }
                _ => None,
            }
        };
        if counted_view.is_none() && !held_by_count.borrow().is_empty() {
            hold_after_all()?;
        }
        skip_counted.set(false);
        let per_file_in: Vec<(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)> =
            counted_view.unwrap_or_else(|| per_file.iter().map(|r| (r.clone(), exports.borrow().snapshot(r, pf.width(r)))).collect());
        // The constants of the run the cone reads (the observer's facts, an earlier phase's exports), bound in its own
        // connection as the main one binds them.
        let constants_in: Vec<(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)> = load
            .rules()
            .flat_map(|r| r.reads())
            .map(|a| load.rel_name(a.rel()).to_owned())
            .collect::<BTreeSet<String>>()
            .into_iter()
            .filter(|r| phased.contains_key(r))
            .map(|r| Ok((r.clone(), atoms_source(&store, head.columns_of(&r).expect("a phase's export the program declares").len(), phased[&r].iter())?)))
            .collect::<Result<_>>()?;
        let (db_path, x_path, kept_in, world_in) = (path.clone(), cross_path.clone(), kept.clone(), world_inputs);
        // The cone's relations are kept like any (t33 K1), in a fact space of their own; its stratum that asks and
        // answers is clean while what it reads and its answers are as they were (K3).
        // The job's own handle on the store: it shares the store's writer.
        let cone_store: Option<std::path::PathBuf> = durable.is_some().then(|| store.database_path().to_path_buf());
        let (cone_slot, cone_kept_conns) = handed.cone();
        let cone_space = kept_strata::part_space(root, "cone");
        // Each of the cone's relations' width, for the job's thread.
        let widths: BTreeMap<String, usize> = load_f.decls().into_iter().map(|d| (load_f.text(d.name()).to_owned(), d.arity())).collect();
        // The cone as rounds of its stratified program, maintained by delta (decision 0145): with a store to keep
        // them in. `RERAIL_CONE_ROUNDS=off` runs the gated fixpoint alone.
        let rounds_wanted = cone_store.is_some()
            && std::env::var("RERAIL_CONE_ROUNDS").map_or(true, |v| v != "off" && v != format!("off-{}", phases::current().as_deref().unwrap_or("none")));
        let round = rounds_wanted.then(|| cone_rounds::RoundProgram::of(load, &per_file, &lowered.domains)).flatten();
        let lowered_round = round.as_ref().map(|r| part_of(r.placed.v.clone()));
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
        // The rounds' key: the cone's round program's digest (`Placed::digest`), made once.
        let rounds_key = match &round {
            Some(r) => Some(retain::key(&[0; 32], r.placed.digest())),
            None => None,
        };
        // A kept manifest names the rounds' key, which keeps their frames (decision 0227); their record names it too,
        // and goes with it (decision 0252, `observe::collect`).
        part_keys.lock().expect("the parts' keys").extend(rounds_key);
        let rounds_record = rounds_key.as_ref().map(|k| format!("cone-rounds:{}", crate::content_store::observe::hex(&rounds_space_of(k))));
        let rounds_journal: Option<(Vec<(std::path::PathBuf, String)>, Vec<std::path::PathBuf>, u64)> = rounds_record.as_ref().and_then(|key| {
            observe(|o| {
                // The record: the cursors and the roots they are of (`reuse::encode_lists`), and the world's digest.
                let (read, world) = o.run_record(key)?;
                // A third list names the plans it goes with: kind, path and digest by turns, as a cutoff's (decision 0252).
                let mut lists = reuse::decode_lists(&read)?.into_iter();
                let (cursors, roots) = (lists.next()?, lists.next()?);
                let roots: Vec<std::path::PathBuf> = roots.into_iter().map(std::path::PathBuf::from).collect();
                let world = u64::from_le_bytes(world.as_slice().try_into().ok()?);
                let named = o.changed_since(key, &roots, &cursors)?;
                Some((roots.into_iter().zip(cursors).collect(), named, world))
            })
        });
        let rounds_world: Arc<std::sync::Mutex<Option<u64>>> = Arc::default();
        let rounds_world_in = Arc::clone(&rounds_world);
        let rounds_withdrawn: Arc<std::sync::Mutex<std::collections::HashSet<i64>>> = Arc::default();
        let rounds_withdrawn_in = Arc::clone(&rounds_withdrawn);
        // The loads answered and taken back, shared with the rounds (decision 0145).
        let load_ledger: Arc<std::sync::Mutex<cone_rounds::LoadLedger>> = Arc::default();
        let load_ledger_in = Arc::clone(&load_ledger);
        let rounds_journal_in = rounds_journal.clone();
        let still_root = root.to_path_buf();
        // Beside the main thread, which answers its requests: a job of the pool's that takes turns with this one.
        let modes = modes.to_owned();
        // Each relation's column types (`retain::types_of`): the cone's kept inputs are digested by content (decision 0212).
        // Read from the run's cross part's version (`run_placed`): no tree is decoded.
        let decl_types: HashMap<String, Vec<u8>> = run_f.decls_by_name().into_iter().map(|(r, _)| (r.to_owned(), retain::types_of(&run_placed, &lowered.domains, r))).collect();
        let vocabulary_cone = Arc::clone(&vocabulary);
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
                load_stats.measured(rel, sampled_statistics(source.as_ref()).into_iter().map(|(c, rows, ndv, sq, sqd)| (c, [rows, ndv, sq, sqd])), []);
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
            let cone_types = cone_inputs.iter().filter_map(|(r, _)| Some((r.clone(), decl_types.get(r)?.clone()))).collect();
            let load_placed = lowered_load.placed()?;
            let mut cone_kept = cone_handle.as_ref().map(|s| kept_strata::Kept::new(s, cone_space, &cone_inputs, cone_types).typed(&load_placed, &lowered_load.domains));
            // Strata kept for the next run keep each relation's whole rows, as a session's cross-file part does: one
            // computed only for the keys its readers reached (the negation split, a corpus run's choice) would miss a
            // key a later run demands (an edited library's new import) whose derivation no delta touches (spike O125,
            // O-cone-kept-plans).
            let _whole = cone_kept.is_some().then(super::demand::whole_relations);
            // Requests go to the main thread with a ticket, and their answers come back with it (decision 0114 §3).
            let asks = std::cell::RefCell::new(Asks { tx: ask_tx, rx: reply_rx, next: 0, early: BTreeMap::new(), posted: HashMap::new(), recorded: None });
            // A request answered in place (the rounds, a stratum needed at once): it waits for its own ticket.
            let mut answer = |rel: &str, rows: &[Vec<i64>]| -> Result<Vec<(String, Vec<Vec<i64>>)>> { asks.borrow_mut().ask(rel, rows) };
            // The rounds from the last run's final round, kept in the store.
            let rounds_space = rounds_key.as_ref().map(&rounds_space_of).unwrap_or_default();
            let rounds_in = cone_rounds::Inputs { per_file: &corpus_rows, corpus: &corpus_docs, constants: &constants_at, ledger: &load_ledger_in };
            // Only what the stored frames hold that is current is known here; rows are read when a round needs them.
            let mut from_rounds = None;
            if let (Some(r), Some(store), Some(key)) = (&round, &cone_handle, rounds_key) {
                let _s = crate::code_rule_dsl::logic::profile::span(|| "the cone's rounds, from the frames".into());
                let k = retain::Retained::bind_kept(store, rounds_space, key, key, &r.placed, r.components(), &rounds_in.constant_digests(&store.atoms_handle(), &r.placed, &r.domains))?;
                let missing = r.missing(&k);
                if missing.is_empty() {
                    from_rounds = Some(k);
                } else {
                    crate::code_rule_dsl::logic::profile::decision("the cone's rounds (decision 0145)", || format!("the frames hold no final round: {} of it not current ({})", missing.len(), missing.iter().take(6).cloned().collect::<Vec<_>>().join(", ")));
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
                    // A relative path is the run root's, as the world reads it (`World::allowed`).
                    let path = still_root.join(&text);
                    let path = path.as_path();
                    roots.iter().any(|(r, _)| path.starts_with(r)) && named.iter().all(|n| !path.starts_with(n) && !(req == "rr_dir_req" && n.parent() == Some(path)))
                }
            });
            let still_ref = still_fn.as_ref().map(|f| f as &dyn Fn(&str, &[i64]) -> bool);
            let on = lowered_round.as_ref().zip(rounds_key).zip(cone_handle.as_ref()).map(|((lowered, key), store)| cone_rounds::On { store, space: rounds_space, db: &db2, modules: &modules, lowered, key, stats: &rounds_stats, keep: &keep_rounds, modes: &modes, still: still_ref, texts: &vocabulary_cone });
            let mut gave_up = None;
            if let (Some(r), Some(on), Some(k)) = (&round, &on, from_rounds) {
                load_ledger_in.lock().unwrap_or_else(|e| e.into_inner()).rounds = true;
                let ran = r.run(on, &rounds_in, cone_rounds::Start::Kept(k), cone_rounds::Answers::default(), &mut answer);
                load_ledger_in.lock().unwrap_or_else(|e| e.into_inner()).rounds = false;
                match ran {
                    Ok(done) => {
                        let mut out = Vec::new();
                        let round_inputs: BTreeSet<&str> = round.as_ref().map(|r| r.placed.v.inputs().map(|a| r.placed.v.text(a)).collect()).unwrap_or_default();
                        for rel in kept_in.iter().filter(|r| !round_inputs.contains(r.as_str())) {
                            let store = cone_handle.as_ref().expect("rounds run with a store");
                            out.push((rel.clone(), done.next.read_derived(store, rel)?.ok_or_else(|| Error::Plan(format!("{rel}: not kept by the cone's rounds")))?));
                        }
                        // The answers' relations as the rounds ended: the kept answers that still stand among them.
                        out.extend(done.produced.iter().filter(|(r, _)| kept_in.contains(r) || world_answer_rels.contains(r)).cloned());
                        crate::code_rule_dsl::logic::profile::decision("the cone's rounds (decision 0145)", || {
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
                let mut ledger = load_ledger_in.lock().unwrap_or_else(|e| e.into_inner());
                ledger.restarted = true;
                ledger.relevel = std::mem::take(&mut ledger.bound);
                drop(ledger);
                // What the rounds derived and held on the cone's connection is not the fixpoint's, nor are the inputs
                // they bound there (the corpus's rows with their loads' rows): the fixpoint's are bound again.
                crate::code_rule_dsl::logic::native::component::forget_derived(&db2);
                for (rel, source) in &cone_inputs {
                    modules.bind(&db2, rel, Arc::clone(source))?;
                }
                crate::code_rule_dsl::logic::profile::decision("the cone's rounds (decision 0145)", || format!("given up ({why}): the gated fixpoint runs"));
            }
            // The gated fixpoint, its answers recorded for a closing round.
            asks.borrow_mut().recorded = Some(cone_rounds::Answers::default());
            let mut recording = |rel: &str, rows: &[Vec<i64>]| -> Result<Vec<(String, Vec<Vec<i64>>)>> { asks.borrow_mut().ask(rel, rows) };
            let mut posting = Posting(&asks);
            let mut fixpoint_keeps = kept_in.clone();
            if let Some(r) = &round {
                fixpoint_keeps.extend(r.asks().cloned());
            }
            load_ledger_in.lock().unwrap_or_else(|e| e.into_inner()).bound = false;
            let mut got = super::run_atoms_native_answering(&lowered_load, &db2, "main", load_stats.clone(), fixpoint_keeps.clone(), &modes, &mut recording, Some(&mut posting), cone_kept.as_mut().map(|k| k as &mut dyn super::KeptStrata), Some(Arc::clone(&vocabulary_cone)))?;
            // Where the budget bound, strata kept from the last run carried its loads, which need not be the nearest
            // ones now: the fixpoint runs again with none kept, its loads in levels (decision 0145).
            let mut cone_strata = cone_kept.as_ref().and_then(|k| k.strata);
            let kept_some = cone_strata.is_some_and(|(ran, all)| ran < all);
            if kept_some && std::mem::take(&mut load_ledger_in.lock().unwrap_or_else(|e| e.into_inner()).bound) {
                crate::code_rule_dsl::logic::profile::decision("the load budget (decision 0145)", || "the budget bound with strata kept from the last run: the cone runs again with none kept".to_owned());
                {
                    let mut ledger = load_ledger_in.lock().unwrap_or_else(|e| e.into_inner());
                    ledger.restarted = true;
                    ledger.relevel = true;
                }
                cone_kept = None;
                asks.borrow_mut().recorded = Some(cone_rounds::Answers::default());
                got = super::run_atoms_native_answering(&lowered_load, &db2, "main", load_stats, fixpoint_keeps, &modes, &mut recording, Some(&mut posting), None, Some(Arc::clone(&vocabulary_cone)))?;
                cone_strata = None;
            }
            let recorded = asks.borrow_mut().recorded.take().unwrap_or_default();
            if let Some(k) = cone_kept {
                k.finish();
            }
            let mut out = Vec::new();
            for rel in &kept_in {
                out.push((rel.clone(), held_rows(&db2, &got, rel, widths[rel.as_str()])?));
            }
            // The closing round over the fixpoint's answers, kept for the next run's rounds; only when the fixpoint
            // ran its strata (a cone kept whole, K3, asked nothing of this run).
            if let (Some(r), Some(on)) = (&round, &on)
                && cone_strata.is_none_or(|(ran, _)| ran > 0)
            {
                let mut asks = BTreeMap::new();
                for ask in r.asks() {
                    asks.insert(ask.clone(), held_rows(&db2, &got, ask, widths[ask.as_str()])?);
                }
                let mut refuse = |rel: &str, _: &[Vec<i64>]| -> Result<Vec<(String, Vec<Vec<i64>>)>> { Err(Error::Plan(format!("the closing round asked {rel}"))) };
                match r.run(on, &rounds_in, cone_rounds::Start::Closing(asks), recorded, &mut refuse) {
                    Ok(done) => {
                        crate::code_rule_dsl::logic::profile::decision("the cone's rounds (decision 0145)", || format!("closing round over the fixpoint's answers kept ({} of {} strata; {} rows written back)", done.strata.0, done.strata.1, done.written));
                        *rounds_world_in.lock().unwrap_or_else(|e| e.into_inner()) = Some(done.world);
                    }
                    Err(cone_rounds::Gave::Up(why)) => crate::code_rule_dsl::logic::profile::decision("the cone's rounds (decision 0145)", || format!("the closing round is not kept: {why}")),
                    Err(cone_rounds::Gave::Err(e)) => crate::code_rule_dsl::logic::profile::decision("the cone's rounds (decision 0145)", || format!("the closing round failed: {e}")),
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
        // The load budget's levels (decision 0145): the loads held back until the cone is quiet, and the levels taken.
        let levels = std::env::var("RERAIL_LOAD_LEVELS").map_or(true, |v| v != "off");
        let mut held: std::collections::HashSet<Vec<i64>> = std::collections::HashSet::new();
        let mut load_levels = 0usize;
        // Where an answer's time goes (decision 0114 §6): reported once the cone ends.
        let times = std::cell::RefCell::new(AnswerTimes::default());
        let mut answer_one = |rel: &str, rows: &[Vec<i64>], process: &mut dyn FnMut(Step) -> Result<Option<(Task, Ctx)>>| -> Result<Vec<(String, Vec<Vec<i64>>)>> {
            pauses += 1;
            let t_call = std::time::Instant::now();
            let _counted = Finally(|| {
                let mut t = times.borrow_mut();
                t.calls += 1;
                t.rows += rows.len();
                t.total += t_call.elapsed().as_secs_f64();
            });
            // A language's answer (`answerers`): the relations of it the program reads, each row led by the request's key.
            if let Some(answerer) = crate::code_rule_dsl::language::answerer(rel) {
                let _s = crate::code_rule_dsl::logic::profile::span(|| format!("answer {rel}"));
                let wanted: Vec<&str> = answerer.produces().iter().copied().filter(|r| world_rels.iter().any(|(w, _)| w == r)).collect();
                use crate::code_rule_dsl::language::Cell;
                struct Files<'w>(&'w mut world::World);
                impl crate::code_rule_dsl::language::Reader for Files<'_> {
                    fn bytes(&mut self, path: &str) -> Option<Vec<u8>> {
                        self.0.bytes(path)
                    }
                }
                let mut cells: Vec<(&'static str, usize, Vec<Cell>)> = Vec::new();
                for (k, row) in rows.iter().enumerate() {
                    let key: Vec<String> = row.iter().map(|a| text(*a)).collect::<Result<_>>()?;
                    let key: Vec<&str> = key.iter().map(String::as_str).collect();
                    answerer.answer(&key, &mut Files(&mut world), &|r| wanted.contains(&r), &mut |r, c| cells.push((r, k, c.to_vec())));
                    language_asked.push((rel.to_owned(), key[0].to_owned()));
                }
                let texts: Vec<&str> = cells.iter().flat_map(|(_, _, c)| c.iter().filter_map(|c| if let Cell::Text(t) = c { Some(t.as_str()) } else { None })).collect();
                let mut atoms = atoms(&texts)?.into_iter();
                let mut out: BTreeMap<&str, Vec<Vec<i64>>> = wanted.iter().map(|r| (*r, Vec::new())).collect();
                for (r, k, c) in &cells {
                    let row = rows[*k].iter().copied().chain(c.iter().map(|c| match c { Cell::Int(i) => *i, Cell::Text(_) => atoms.next().expect("an atom a text") })).collect();
                    out.entry(r).or_default().push(row);
                }
                return Ok(out.into_iter().map(|(r, rows)| (r.to_owned(), rows)).collect());
            }
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
            // The load budget's levels (decision 0145): a fresh cone holds back each load the budget bounds until it
            // is otherwise quiet, then takes the loads held back together. The k-th quiet point's loads are those k
            // loads from the corpus, so where the budget binds the loads taken are the nearest, whatever the order
            // they were asked in. The rounds answer at once, and give up where the budget binds (`LoadLedger::bound`).
            // `RERAIL_LOAD_LEVELS=off` answers every load at once.
            let in_rounds = {
                let mut ledger = load_ledger.lock().unwrap_or_else(|e| e.into_inner());
                if std::mem::take(&mut ledger.relevel) {
                    // The cone runs again in levels: what was loaded from beyond the corpus is taken back, and asked
                    // again level by level.
                    held.clear();
                    let n = world.take_back_libraries();
                    crate::code_rule_dsl::logic::profile::decision("the load budget (decision 0145)", || format!("loaded past the budget out of level order: {n} documents taken back, to be loaded again level by level"));
                }
                ledger.rounds
            };
            let mut hold: Vec<Vec<i64>> = Vec::new();
            let mut now: Vec<Vec<i64>> = Vec::new();
            // Each request's path, resolved once (`World::resolve`), by its path's atom.
            // Resolved together: their `realpath`s at once on the pool (notes/t40).
            let mut resolved: HashMap<i64, Option<std::path::PathBuf>> = if no_loads {
                HashMap::new()
            } else {
                let asked: Vec<String> = rows.iter().map(|r| text(r[0])).collect::<Result<_>>()?;
                rows.iter().map(|r| r[0]).zip(world.resolve_all(&asked)).collect()
            };
            if levels && !in_rounds && !no_loads {
                let mut quiet = false;
                for row in rows {
                    let at = resolved.get(&row[0]).cloned().flatten();
                    let bounded = world.bounded(at.as_deref());
                    if !bounded {
                        now.push(row.clone());
                    } else if held.remove(row) {
                        quiet = true;
                        now.push(row.clone());
                    } else {
                        hold.push(row.clone());
                    }
                }
                if !hold.is_empty() {
                    held.extend(hold.iter().cloned());
                } else if quiet {
                    load_levels += 1;
                }
            } else {
                now.extend(rows.iter().cloned());
            }
            let over_before = world.over_budget;
            let rows: &[Vec<i64>] = &now;
            let mut out = 'load: {
            let mut fresh = Vec::new();
            let mut fresh_files: Vec<i64> = Vec::new();
            let mut again: std::collections::HashSet<i64> = std::collections::HashSet::new();
            let mut loaded_rows = Vec::new();
            // A run of some documents on their own (`phases::maintain`) follows no load: the documents loaded are the
            // last run's, kept.
            // Taken in an order of the requests themselves, not of their atoms: where the load budget binds, a
            // top-level module before a submodule, then by module and path (decision 0145).
            let mut asked_rows: Vec<(String, String, String, &Vec<i64>)> = Vec::new();
            for row in rows.iter().filter(|_| !no_loads) {
                asked_rows.push((text(row[0])?, text(row[1])?, text(row[2])?, row));
            }
            asked_rows.sort_by(|a, b| (a.1.matches('.').count(), &a.1, &a.0).cmp(&(b.1.matches('.').count(), &b.1, &b.0)));
            let t_take = std::time::Instant::now();
            let _taken = Finally(|| times.borrow_mut().take += t_take.elapsed().as_secs_f64());
            // The loads the rounds took back since the last answer free their places in the budget.
            let taken_back = std::mem::take(&mut load_ledger.lock().unwrap_or_else(|e| e.into_inner()).taken_back);
            for file in taken_back {
                world.unload(&text(file)?);
            }
            for (asked, module, grammar, row) in asked_rows {
                let at = resolved.remove(&row[0]).unwrap_or_else(|| world.resolve(&asked));
                // A load asked again (its stratum's requests answered again, t33 K3) is answered alike: the same
                // document, its rows from the exports, not sent again. So is one taken back and asked again.
                if let Some(doc) = world.loaded_as(at.as_deref()).or_else(|| world.reload(at.as_deref())) {
                    if let Some((key, _)) = loaded_docs.iter().find(|(_, p)| *p == doc.path) {
                        let file = atoms(&[doc.path.as_str()])?[0];
                        loaded_rows.push(vec![file, row[1], row[2]]);
                        load_ledger.lock().unwrap_or_else(|e| e.into_inner()).documents.insert(file, *key);
                        again.insert(*key);
                    }
                    continue;
                }
                if let Some(doc) = world.load(&asked, at.as_deref(), &module, &grammar, label, &corpus) {
                    let file = atoms(&[doc.path.as_str()])?[0];
                    loaded_rows.push(vec![file, row[1], row[2]]);
                    fresh_files.push(file);
                    fresh.push(doc);
                }
            }
            let mut out = vec![(world::LOADED.to_owned(), loaded_rows)];
            if fresh.is_empty() && again.is_empty() {
                out.extend(per_file.iter().map(|r| (r.clone(), Vec::new())));
                break 'load out;
            }
            if fresh.is_empty() {
                for rel in &per_file {
                    let at = split.locations.get(rel).and_then(|l| l.first().copied()).ok_or_else(|| Error::Plan(format!("{rel}: no location column")))?;
                    let width = pf.width(rel);
                    out.push((rel.clone(), exports.borrow().rows_of(rel, width, at, &again)));
                }
                break 'load out;
            }
            // A document's key is a function of its path (decision 0052).
            all_paths.extend(fresh.iter().map(|d| d.path.clone()));
            let keys = memo::doc_keys(&all_paths);
            let keys = &keys[keys.len() - fresh.len()..];
            loaded_docs.extend(keys.iter().copied().zip(fresh.iter().map(|d| d.path.clone())));
            load_ledger.lock().unwrap_or_else(|e| e.into_inner()).documents.extend(fresh_files.iter().copied().zip(keys.iter().copied()));
            {
                let mut t = times.borrow_mut();
                t.fresh += fresh.len();
                t.again += again.len();
            }
            drop(_taken);
            docs_tx.send(keys.iter().copied().zip(fresh).map(|(k, d)| (k, d.path, Some(Arc::from(d.module.as_str())))).collect()).map_err(|_| Error::Plan("the chunk builder ended".to_owned()))?;
            // The chunks' file-local evaluation, at once on the pool as the run's chunks are (decision 0292 §2), each
            // begun and filed here (`process`), apart from the wait for them to be built and evaluated.
            let t_drain = std::time::Instant::now();
            let mut evaluate = 0.0;
            let drained = drain(&mut |s: Step| {
                let t = std::time::Instant::now();
                let r = process(s);
                evaluate += t.elapsed().as_secs_f64();
                r
            }, true);
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
                let width = pf.width(rel);
                out.push((rel.clone(), exports.borrow().rows_of(rel, width, at, &new)));
            }
            out
            };
            if world.over_budget > over_before {
                load_ledger.lock().unwrap_or_else(|e| e.into_inner()).bound = true;
            }
            if !hold.is_empty() {
                out.push((format!("{}{rel}", crate::code_rule_dsl::logic::answers::HELD_BACK), hold));
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
            // The rounds gave up: the fresh cone's answers are the run's, not those given to the rounds.
            if std::mem::take(&mut load_ledger.lock().unwrap_or_else(|e| e.into_inner()).restarted) {
                for rows in answered.values_mut() {
                    rows.clear();
                }
            }
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
        if load_levels > 0 {
            crate::code_rule_dsl::logic::profile::decision("the load budget (decision 0145)", || format!("loads taken in {load_levels} levels, each once the cone was otherwise quiet; {} past the budget", world.over_budget));
        }
        {
            let mut t = times.into_inner();
            t.idle = idle;
            t.report();
        }
        close_workers();
        (loading_rows, cone_kept, cone_rounds) = cone.join().map_err(|_| Error::Plan("the cone of requests panicked".to_owned()))??;
        // The documents the rounds loaded and took back (decision 0145): neither the run's documents nor rows of its
        // exports.
        let mut gone = std::mem::take(&mut *rounds_withdrawn.lock().unwrap_or_else(|e| e.into_inner()));
        // And any document taken back and not loaded again (the budget's levels after the rounds gave up).
        let taken: std::collections::HashSet<&String> = world.taken_paths().collect();
        gone.extend(loaded_docs.iter().filter(|(_, p)| taken.contains(p)).map(|(d, _)| *d));
        if !gone.is_empty() {
            loaded_docs.retain(|(d, _)| !gone.contains(d));
            for rel in &per_file {
                let width = load_f.arity(rel).expect("a declared relation");
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
            crate::code_rule_dsl::logic::profile::decision("the cone's rounds (decision 0145)", || format!("{} documents loaded and taken back: no load the run's own rows ask for names them", gone.len()));
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
                    let cursors: Vec<String> = at.iter().map(|(_, c)| c.clone()).collect();
                    let roots: Vec<String> = at.iter().map(|(r, _)| r.to_string_lossy().into_owned()).collect();
                    // With the key its frames are kept under, which the run's manifest names: collection drops the
                    // record once no kept manifest does (decision 0252, `observe::collect`).
                    let plans: Vec<String> = rounds_key.iter().flat_map(|k| ["plan".to_owned(), String::new(), crate::content_store::observe::hex(k)]).collect();
                    o.put_run_record(key, &reuse::encode_lists(&[&cursors[..], &roots[..], &plans[..]]), &world.to_le_bytes());
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
        cone_inputs = cone_digests(&exports.borrow(), &per_file, &split.locations, &pf, docs.iter().chain(&loaded_docs))?;
        let summary = world.summary(pauses);
        consulted.extend(world.consulted());
        crate::code_rule_dsl::logic::profile::decision("file system and loaded documents (t21)", || summary);
    } else {
        // No requests: no cone, so each document reads into it nothing.
        cone_inputs = docs.iter().map(|(_, p)| (p.clone(), String::new())).collect();
    }
    close_workers();
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
    crate::code_rule_dsl::logic::profile::decision("input scheduling", || serde_json::json!({
        "root":root,"buildJobs":builder.build_jobs.load(std::sync::atomic::Ordering::Relaxed),
        "cachedDocuments":builder.ready_hits.load(std::sync::atomic::Ordering::Relaxed)
    }).to_string());
    Ok(((c.parse.0 + jobs_cpu.0, c.parse.1 + jobs_cpu.1), c.batch, std::mem::take(&mut c.read_sources), std::mem::take(&mut *stats_read.lock()), std::mem::take(&mut c.read_unstored)))
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
    run_sources.extend(program_sources.iter().copied());
    // The contents named by their digests: their sources found behind the run's writes, before the manifest's.
    if recording {
        memo::named_digests(store, worker_cpu.4.clone());
    }
    // A run that read no file anew (every one known by its stat) has no stat to record.
    if recording && !worker_cpu.3.is_empty() {
        let _stats = crate::code_rule_dsl::logic::profile::span(|| "record stats".into());
        // Behind the run (decision 0104): a new content's source is written by then, by the tape's write queued before.
        // In the run's record (decision 0306): a content is its source as the store holds it then.
        fn apply(cx: &crate::content_store::record::Apply<'_>, body: crate::content_store::record::codec::Body) -> crate::content_store::StoreResult<()> {
            let (root, read, started_ns): (String, Vec<(String, crate::content_store::FileStat, memo::Content)>, i64) = crate::content_store::record::codec::from_body("the files' stats", body)?;
            let mut ids = Vec::with_capacity(read.len());
            for (path, stat, content) in read {
                if let Some(id) = content.resolve(cx.store)? {
                    ids.push((path, stat, id));
                }
            }
            cx.store.record_corpus_stats(&root, &ids, started_ns)
        }
        store.record("the files' stats", vec![stats_key(&canonical_root)], &(&canonical_root, &worker_cpu.3, started_ns), None, apply);
    }
    // Publish all source references before releasing the lease and starting collection (decision 0053).
    // The memo's relations, named and typed, so a SQL session can read the run's frames (docs/sql-access).
    // An export's entry holds rows (its columns, as JSON); a file-local output's holds lines (`{"lines": columns}`).
    let mut relations: Vec<(String, String)> = split
        .exports
        .iter()
        .filter_map(|rel| pf.columns(rel).map(|cols| (rel.clone(), serde_json::to_string(&cols).unwrap_or_default())))
        .collect();
    for rel in &local_outputs {
        if let Some(cols) = pf.columns(rel).filter(|_| !relations.iter().any(|(r, _)| r == rel)) {
            relations.push((rel.clone(), serde_json::json!({ "lines": cols }).to_string()));
        }
    }
    // With the cross-file memo, the manifest also names the components' stored rows (decision 0076), known only
    // after the cross-file part: published then, the lease held until. So with a durable store: the cross part's
    // key (`part_keys`) is known only once it runs, and the manifest is what keeps its frames (decision 0227). Named
    // before it, a phase's or phase 0's cross part was named by no manifest, and collected after every cold run.
    // The guests' keys the run claimed, in its record (decision 0391), before the manifest's item.
    if recording && let Some(cache) = tapes.as_ref() {
        cache.record_guest_keys();
    }
    let mut publish_later = None;
    let mut collecting = if recording && cross_plan.is_none() && durable.is_none() {
        record_run(store, root, &run_sources, &memo_program, &program_plans.iter().chain(&nested_plans).chain(part_keys.lock().expect("the parts' keys").iter()).chain(kept_runs.lock().expect("the kept runs").iter()).copied().collect::<Vec<_>>(), Some(lane), policy, &relations, &late_sources, reading, !phases::in_phase() && !phase0::active() && options.get(constants::OWN).is_none())?
    } else if recording {
        publish_later = Some((relations, reading));
        None
    } else {
        drop(reading);
        None
    };
    statements.enter("stats");
    // Decision 0458: the corpus batch, every file's documents, which the cross part reads its base relations off as a
    // chunk reads its own. The evaluated chunks handed theirs on; a memo hit's is made from its stored tape (parsed
    // again, in memory, where the store has none), its guests from the requests its entry kept. Nothing is read from
    // SQL here (decision 0104): the tapes were read with the misses'.
    // A document of `source` made from its tape `frame` (parsed in memory where there is none); `None` for a text file
    // that is not UTF-8, which is no document.
    let doc_of = |key: i64, path: &str, module: Option<Arc<str>>, source: String, frame: Option<&crate::content_store::TapeFrame>| -> Result<Option<syntax::Document>> {
        let _doc = crate::code_rule_dsl::logic::profile::span(|| "a document from its tape (decision 0458)".into());
        let document = |facts: Arc<core::Facts>, tape| {
            let digest = core::digest(&facts);
            syntax::Document { key: key as u32, path: path.to_owned(), core: format!("/c#core:{path}"), digest, facts, guest: None, module, tape }
        };
        if text_document(path, label) {
            return Ok(Some(document(Arc::new(core::text(Arc::new(source))), None)));
        }
        let file = core::Source { language: label.to_owned(), source: Arc::new(source) };
        let built = match (&tapes, frame) {
            (Some(cache), Some(frame)) if cfg!(feature = "tape-cache") => core::build_in_wave_with_tape(&file, cache, None, Some(frame), true, keep_tapes),
            _ => core::build_with_tape(&file, keep_tapes),
        };
        let (built, tape) = built.map_err(|e| Error::Plan(format!("{path}: {e:?}")))?;
        let facts = match built {
            Built::Facts(f) => Arc::new(f),
            Built::Unparsed => return Err(Error::Plan(format!("{path} did not parse"))),
        };
        let tape = tape.filter(|_| keep_tapes).map(|doc| (doc, Arc::new(file) as Arc<dyn crate::code_rule_dsl::facts::core::SourceFile>));
        Ok(Some(document(facts, tape)))
    };
    // `docs`' guests, round by round, from the requests their entries kept (`memo::GUESTS`): as a chunk of partial hits
    // makes its own (decision 0225 §2), under the store's keys (decision 0391), so their nodes are those the entries
    // name.
    let with_guests = |docs: &mut Vec<syntax::Document>, kept: Vec<(usize, String, output::Rows)>| -> Result<()> {
        if slice.is_none() {
            return Ok(());
        }
        let mut rounds: Vec<BTreeMap<String, Arc<output::Rows>>> = Vec::new();
        for (r, rel, rows) in kept {
            if rounds.len() <= r {
                rounds.resize_with(r + 1, Default::default);
            }
            let round = rounds[r].entry(rel).or_insert_with(|| Arc::new(output::Rows::new(rows.types().to_vec())));
            Arc::make_mut(round).append(&rows);
        }
        let mut made = guests::Guests::new(label, projection.shape, Arc::clone(&guest_keys));
        for round in &rounds {
            let new = made.make(guests::requests(round), docs, &guest_parses).map_err(Error::Plan)?;
            if new.is_empty() {
                break;
            }
            docs.extend(new);
        }
        Ok(())
    };
    // A run that listed its files as it went compares them with the table now, from the contents its chunks found:
    // unless it runs inside another run's waves (a phase's), where the store is not read (decision 0104).
    if !base_known && base_binds.is_some() && crate::code_rule_dsl::logic::profile::inside_waves() {
        crate::code_rule_dsl::logic::profile::decision("the base relations' old documents (decision 0458)", || "none: the run's files were listed inside the waves, where their old sources are not read (decision 0104); what reads a base relation runs again".into());
    } else if !base_known {
        let now_of: HashMap<String, (i64, Option<memo::Content>)> = run_docs_held.lock().expect("the run's documents").iter().map(|(p, (c, k))| (p.clone(), (*k, Some(*c)))).collect();
        old_base = base_compare(now_of)?;
    }
    // The changed files' documents now, guests with them (decision 0458): a base relation's new version.
    let mut changed_docs: Vec<syntax::Document> = Vec::new();
    let mut corpus_digest = 0u64;
    let mut corpus_batch: Option<Arc<syntax::Batch>> = None;
    // The corpus batch makes the chunks' projection (decision 0458): a span the cross part reads (a line's `line`,
    // a comment's `col`) is made only where its relation is, and the file-local part names them. Not what makes no
    // span and the cross part does not read: the tree digests (every node's hash), the text tests and the `ast`
    // matchers. The old and new batches of an edit are made the same way, so their columns are the batch's.
    let cross_reads: BTreeSet<String> = stored.iter().map(|(r, _)| r.clone()).collect();
    let cross_projection = {
        let mut p = projection.clone();
        if let Some(reads) = &mut p.reads {
            reads.retain(|r| !matches!(r.as_str(), "tree_digest" | "def_digest") || cross_reads.contains(r));
        }
        p
    };
    let cross_vocabulary = if vocabulary.tests.iter().any(|(t, _)| cross_reads.contains(t)) {
        Arc::clone(&vocabulary)
    } else {
        Arc::new(crate::code_rule_dsl::logic::strings::Vocabulary::from_parts(Vec::new(), Vec::new(), vocabulary.matches.clone(), vocabulary.markers.clone()))
    };
    let cross_matchers = if cross_reads.iter().any(|r| r.starts_with(matchers::PREFIX)) { chunk_matchers.clone() } else { syntax::Matchers::default() };
    let corpus_sources: HashMap<String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>> = if stored.is_empty() || docs.is_empty() {
        HashMap::new()
    } else {
        use rayon::prelude::*;
        let _s = crate::code_rule_dsl::logic::profile::span(|| "the corpus batch (decision 0458)".into());
        let hits = std::mem::take(&mut corpus_hits);
        let tapes_of = std::mem::take(&mut *hit_tapes.lock());
        let hit_doc = |h: &CorpusHit| -> Result<Option<syntax::Document>> {
            let source = match std::fs::read_to_string(root.join(&h.path)) {
                Ok(source) => source,
                Err(e) if e.kind() == std::io::ErrorKind::InvalidData && text_document(&h.path, label) => return Ok(None),
                Err(e) => return Err(Error::Plan(format!("{}: {e}", h.path))),
            };
            doc_of(h.key, &h.path, h.module.clone(), source, tapes_of.get(&h.path))
        };
        let mut made: Vec<syntax::Document> = hits.par_iter().map(hit_doc).collect::<Result<Vec<_>>>()?.into_iter().flatten().collect();
        with_guests(&mut made, hits.into_iter().flat_map(|h| h.rounds).collect())?;
        let mut all = std::mem::take(&mut corpus_docs);
        all.extend(made);
        // A guest two files asked for (a value's askers) is one document, made alike by each.
        all.sort_by_key(|d| d.key);
        all.dedup_by_key(|d| d.key);
        if let Some((changed, _)) = &old_base {
            changed_docs = all.iter().filter(|d| changed.contains(&d.path)).cloned().collect();
        }
        // What the base relations are a function of, under the run's plan: their documents (decision 0458). A base
        // relation's digest is theirs, taken here, never its rows' (a scan of every row).
        let words = |d: &syntax::Document| -> [i64; 5] {
            let at = |i: usize| i64::from_le_bytes(d.digest[i * 8..i * 8 + 8].try_into().expect("eight bytes"));
            [i64::from(d.key), at(0), at(1), at(2), at(3)]
        };
        corpus_digest = all.iter().map(|d| super::row_digest(&words(d))).fold(0u64, u64::wrapping_add);
        // Each document's views' counts as kept, where its contents are those they were counted of (decision 0464).
        let given: HashMap<u32, syntax::made::ViewCounts> = all.iter().filter_map(|d| base_counts.get(&d.key).filter(|(digest, _)| *digest == d.digest).map(|(_, c)| (d.key, *c))).collect();
        crate::code_rule_dsl::logic::profile::decision("the corpus batch's row space (decision 0464)", || format!("{} of {} documents' views' counts kept", given.len(), all.len()));
        let sources = syntax::with_matchers(&store, all, &sets, &cross_matchers, &cross_vocabulary, binding, &cross_projection, &given).map_err(|e| Error::Plan(e.to_string()))?;
        corpus_batch = sources.iter().find_map(|(_, s)| s.python_batch());
        sources.into_iter().filter(|(r, _)| stored.iter().any(|(s, _)| s == r)).collect()
    };
    // A base relation's digest: its documents', under its name (decision 0458).
    let base_digest = |rel: &str| corpus_digest ^ super::row_digest(&rel.bytes().map(i64::from).collect::<Vec<_>>());
    // Decision 0458 step 3: a base relation's change is the changed files' documents, compared one document at a
    // time: its `Minus` the rows of their old documents (made from the old sources and tapes read before the first
    // wave, `old_base`) that their new ones do not hold, its `Plus` the reverse, each picked where it lies in its version
    // (0364 R2). None without the old documents: what reads a base relation then runs again from its reads.
    type Versions = HashMap<String, (Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)>;
    // And its version before the edit (`input_delta::Before`): the corpus batch's rows but the changed documents',
    // then their old documents' rows, read where they lie.
    let mut base_olds: HashMap<String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>> = HashMap::new();
    // And its changed documents' whole rows, old and new: its kept statistics' patch (counted out, counted in).
    type Whole = (Option<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>>, Option<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>>);
    let mut base_wholes: HashMap<String, Whole> = HashMap::new();
    // The statistics counted or patched this run, with the digest of their rows: kept beside the table.
    let mut base_stats_out: Vec<(String, u64, super::statkeep::RelStats)> = Vec::new();
    let base_versions: Versions = match &old_base {
        Some((changed, olds)) if !corpus_sources.is_empty() && !changed.is_empty() => {
            let _s = crate::code_rule_dsl::logic::profile::span(|| "the base relations' versions (decision 0458)".into());
            let mut old_docs: Vec<syntax::Document> = Vec::new();
            let mut old_rounds: Vec<(usize, String, output::Rows)> = Vec::new();
            for (path, old) in olds {
                let source = String::from_utf8(old.bytes.to_vec()).map_err(|e| Error::Plan(format!("{path}: {e}")))?;
                old_docs.extend(doc_of(old.key, path, given_modules.get(path).cloned(), source, old.frame.as_ref())?);
                for (k, (_, rows)) in old.entry.iter().flat_map(|e| &e.outputs) {
                    if let Some((round, rel)) = memo::guest_round(k) {
                        old_rounds.push((round, rel.to_owned(), rows.clone()));
                    }
                }
            }
            with_guests(&mut old_docs, old_rounds)?;
            let batch_of = |mut docs: Vec<syntax::Document>| -> Result<HashMap<String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>>> {
                if docs.is_empty() {
                    return Ok(HashMap::new());
                }
                docs.sort_by_key(|d| d.key);
                docs.dedup_by_key(|d| d.key);
                Ok(syntax::with_matchers(&store, docs, &sets, &cross_matchers, &cross_vocabulary, binding, &cross_projection, &HashMap::new()).map_err(|e| Error::Plan(e.to_string()))?.into_iter().collect())
            };
            let changed_keys: BTreeSet<i64> = old_docs.iter().chain(changed_docs.iter()).map(|d| i64::from(d.key)).collect();
            let (old_batch, new_batch) = (batch_of(old_docs)?, batch_of(std::mem::take(&mut changed_docs))?);
            let mut versions = Versions::new();
            for (rel, _) in &stored {
                let Some(source) = corpus_sources.get(rel) else { continue };
                // A text test is a test of a string (decision 0464): nothing of it changes with an edit.
                if cross_vocabulary.tests.iter().any(|(t, _)| t == rel) {
                    crate::code_rule_dsl::logic::profile::decision("the base relations' versions (decision 0458)", || format!("{rel}: a text test, no version (decision 0464)"));
                    continue;
                }
                let columns = source.columns();
                let width = columns.len();
                // A version: its source, in the corpus batch's columns (the same projection's), and its rows.
                let version = |batch: &HashMap<String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>>| -> Result<Option<(Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, Vec<Vec<i64>>)>> {
                    let Some(s) = batch.get(rel) else { return Ok(None) };
                    if s.columns() != columns {
                        return Err(Error::Plan(format!("{rel}: a version's columns {:?}, the corpus batch's {columns:?}", s.columns())));
                    }
                    Ok(Some((Arc::clone(s), (0..s.len()).map(|r| (0..width).map(|c| s.value(r, c)).collect()).collect())))
                };
                let (old, new) = (version(&old_batch)?, version(&new_batch)?);
                base_wholes.insert(rel.clone(), (old.as_ref().map(|(s, _)| Arc::clone(s)), new.as_ref().map(|(s, _)| Arc::clone(s))));
                let set = |v: &Option<(_, Vec<Vec<i64>>)>| -> BTreeSet<Vec<i64>> { v.as_ref().map_or_else(BTreeSet::new, |(_, rows)| rows.iter().cloned().collect()) };
                let (old_set, new_set) = (set(&old), set(&new));
                let names = |suffix: &str| { let name = format!("{rel}__{suffix}"); (0..width).map(|c| super::sql_col(&name, c)).collect::<Vec<_>>() };
                // Read where they lie (0364 R2): the rows of one version the other does not hold, picked.
                let picked = |v: Option<(Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, Vec<Vec<i64>>)>, other: &BTreeSet<Vec<i64>>, suffix: &str| match v {
                    Some((s, rows)) => crate::code_rule_dsl::logic::native::component::InRel::picked(s, (0..rows.len() as u32).filter(|&r| !other.contains(&rows[r as usize])).collect(), names(suffix)),
                    None => crate::code_rule_dsl::logic::native::component::columns_source(names(suffix), vec![Some(Vec::new()); width]),
                };
                // The changed documents' rows now, through the batch's index of their documents; a relation with none
                // is read through for them.
                let skip: Vec<u32> = match input_delta::rows_of_documents(source.as_ref(), &changed_keys) {
                    Some(rows) => rows,
                    None => {
                        crate::code_rule_dsl::logic::profile::decision("the base relations' versions (decision 0458)", || format!("{rel}: no index of its documents, read through for the changed ones"));
                        let by_document = columns.first().is_some_and(|c| c == "d" || c == "n" || c == "p");
                        (0..source.len() as u32).filter(|&r| if by_document { changed_keys.contains(&memo::document_of(source.value(r as usize, 0))) } else { new_set.contains(&(0..width).map(|c| source.value(r as usize, c)).collect::<Vec<_>>()) }).collect()
                    }
                };
                base_olds.insert(rel.clone(), Arc::new(input_delta::Before::new(Arc::clone(source), skip, old.as_ref().map(|(s, _)| Arc::clone(s)))));
                versions.insert(rel.clone(), (picked(new, &old_set, "plus"), picked(old, &new_set, "minus")));
            }
            versions
        }
        _ => Versions::new(),
    };
    // Each base relation's rows, as its kept statistics count them (decision 0464): kept of these documents, or of
    // the table's and patched by the changed documents' old and new rows. The corpus batch answers its `len` by them,
    // and fails a relation whose made rows are not them.
    if let Some(batch) = &corpus_batch {
        let (table_digest, kept) = &base_stats_kept;
        let tag = |rel: &str| super::row_digest(&rel.bytes().map(i64::from).collect::<Vec<_>>());
        let totals: HashMap<String, usize> = stored
            .iter()
            .filter_map(|(rel, _)| {
                let (d, k) = kept.get(rel)?;
                let rows = if *d == base_digest(rel) {
                    k.rows()
                } else if Some(*d) == table_digest.map(|t| t ^ tag(rel)) {
                    let (old, new) = base_wholes.get(rel)?;
                    k.rows() - old.as_ref().map_or(0, |s| s.len() as i64) + new.as_ref().map_or(0, |s| s.len() as i64)
                } else {
                    return None;
                };
                usize::try_from(rows).ok().map(|r| (rel.clone(), r))
            })
            .collect();
        crate::code_rule_dsl::logic::profile::decision("the corpus batch's row space (decision 0464)", || format!("{} of {} base relations' rows counted by their kept statistics", totals.len(), stored.len()));
        batch.give_totals(&totals);
    }
    if std::env::var_os("RERAIL_STATS_KEEP_TRACE").is_some() {
        eprintln!("rerail: base versions: old {:?}, corpus {}, versions {:?}", old_base.as_ref().map(|(c, o)| (c.len(), o.len())), corpus_sources.len(), base_versions.iter().map(|(r, (p, m))| (r.clone(), p.len(), m.len())).collect::<Vec<_>>());
    }
    given_deltas.extend(base_versions.iter().map(|(rel, v)| (rel.clone(), v.clone())));
    // Each base relation the cross part reads: its columns as the corpus batch has them, and which of them it names
    // (`stored`). The rows are the batch's; none is gathered.
    for (rel, names) in stored.iter().filter(|_| !corpus_sources.is_empty()) {
        let have: Vec<String> = match corpus_sources.get(rel) {
            Some(source) => source.columns(),
            None => projection.columns(rel).unwrap_or_default().iter().map(|c| (*c).to_owned()).collect(),
        };
        let values = have.iter().map(|c| names.contains(c).then(Vec::new)).collect();
        base_rows.insert(rel.clone(), (have, values));
    }
    // An export no evaluated chunk measured (every file read from the memo, or none with rows): measured as empty.
    for rel in &split.exports {
        if !totals.rels.contains_key(rel) {
            let width = pf.width(rel);
            let types: Vec<&str> = pf.types(rel);
            let empty = crate::code_rule_dsl::logic::native::component::columns_source((0..width).map(|c| format!("c{c}")).collect(), vec![Some(Vec::new()); width]);
            totals.add_rows(rel, 0, empty.as_ref(), &types);
        }
    }
    // A memoised run's base relations, as the cross-file part reads them: every file's rows, read from the memo or
    // evaluated, measured once (a file read from the memo has no chunk to measure them in).
    if memoised {
        let _s = crate::code_rule_dsl::logic::profile::span(|| "measure the base rows held".into());
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
        // Decision 0117 step 1: the hits read for their outputs alone take their export and base rows from the kept
        // inputs: each input its kept rows, less the changed documents', and this run's rows of them. Else (no kept
        // inputs, or a row not attributable to its document) the hits are decoded whole, as before.
        let kept_hits = std::mem::take(&mut *kept_hits.lock());
        if !kept_hits.is_empty() {
            let _s = crate::code_rule_dsl::logic::profile::span(|| "the kept inputs".into());
            let installed: std::result::Result<(usize, usize), String> = (|| {
                let maintained = match early_maintained.take() {
                    Some(m) => m,
                    None => maintained_cross_of(shared_maintained, &cross).map_err(|e| e.to_string())?,
                };
                let (_, binds) = retain::keys(&prepared_key, maintained.digest().map_err(|e| e.to_string())?);
                // The run reads the program's version, not its tree (t36 item 7).
                let placed = maintained.placed().map_err(|e| e.to_string())?;
                let prog = &placed;
                let fields = prog.fields().map_err(|e| e.to_string())?;
                // The exports alone: the base relations are read off the corpus batch (decision 0458, below).
                let rels: Vec<String> = split.exports.clone();
                let kept = retain::kept_inputs(store, kept_strata::space(root), binds, prog, &rels, &semijoin).map_err(|e| e.to_string())?.map_err(|why| format!("no kept inputs of this cross part: {why}"))?;
                let _s1 = crate::code_rule_dsl::logic::profile::span(|| "the changed files' old and new rows".into());
                // The changed files' entries as the kept inputs hold them: the keeper's, found before the first wave.
                let mut old_entries = Vec::new();
                for (path, found) in &counted_before {
                    let Some((_, key)) = counted_now.get(path) else { continue };
                    let entry = found.as_ref().ok_or_else(|| format!("{path}: its counted entry is not in the memo"))?.entry(*key).map_err(|e| e.to_string())?.ok_or_else(|| format!("{path}: its counted entry is another document's"))?;
                    old_entries.push((path.clone(), entry));
                }
                let no_holders = rustc_hash::FxHashMap::default();
                let mut changed = 0usize;
                let mut results: Vec<(String, usize, Vec<String>, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, OwnerPatch, Option<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>>)> = Vec::new();
                // Each input's old rows (the changed files' entries) and this run's, before any is patched.
                #[allow(clippy::type_complexity)]
                let mut staged: Vec<(&String, usize, Vec<Vec<Vec<i64>>>, Vec<Vec<i64>>)> = Vec::new();
                for rel in &rels {
                    let k = &kept[rel];
                    let width = fields.width(rel);
                    if k.source.columns().len() != width {
                        return Err(format!("{rel}: the kept input has {} columns, the run binds {width}", k.source.columns().len()));
                    }
                    // An export's holders are its owners' (`group_col`): the column the memo attributes its rows by.
                    let g = retain::group_col(prog, &split.locations, rel);
                    if width > 0 && split.locations.get(rel).and_then(|l| l.first()) != Some(&g) {
                        return Err(format!("{rel}: its rows are kept by column {g}, the memo attributes them by {:?} (declared {:?})", split.locations.get(rel), fields.columns(rel)));
                    }
                    // Each changed file's old rows, read as the kept input's columns.
                    let mut old: Vec<Vec<Vec<i64>>> = Vec::new();
                    for (_, e) in &old_entries {
                        let mut rows: Vec<Vec<i64>> = Vec::new();
                        if let Some((w, flat)) = e.rows.get(rel) {
                            if *w != width {
                                return Err(format!("{rel}: an entry's rows are {w} wide, the input {width}"));
                            }
                            rows.extend(flat.chunks_exact(width.max(1)).map(<[i64]>::to_vec));
                        }
                        old.push(rows);
                    }
                    // This run's rows of the changed files: the sum of their new entries.
                    let new: Vec<Vec<i64>> = {
                        let s = exports.borrow().snapshot(rel, width);
                        let all: Vec<usize> = (0..width).collect();
                        let mut row = vec![0i64; width];
                        (0..s.len()).map(|r| { s.row_at(r, &all, &mut row); row.clone() }).collect()
                    };
                    staged.push((rel, g, old, new));
                }
                // The documents this run evaluated: every real document a changed file's old entry or this run's rows
                // name. A chunk evaluates its unchanged files with its changed ones, so this is more than the files
                // the keeper counted as changed. Each such document's rows are this run's (`patch_owners`).
                drop(_s1);
                let _s2 = crate::code_rule_dsl::logic::profile::span(|| "the evaluated owners, patched".into());
                let values = crate::code_rule_dsl::facts::vtab::syntax::HIDDEN_KEYS as i64;
                // Of the relations owned by a node: a string's owner (`memo::Owner::Text`) is no document.
                // One owner a row, deduplicated as read: a set of every row's owner sorted millions (Q20).
                let mut owners: rustc_hash::FxHashSet<i64> = rustc_hash::FxHashSet::default();
                for (_, g, old, new) in staged.iter().filter(|(rel, ..)| matches!(memo::owner(rel), memo::Owner::Node(_))) {
                    owners.extend(old.iter().flatten().chain(new.iter()).map(|r| retain::owner_of(r, *g)).filter(|o| *o < values));
                }
                let evaluated: BTreeSet<i64> = owners.into_iter().collect();
                // `RERAIL_STATS_KEEP_TRACE`: what the patches read, by input.
                let mut read_trace: Option<Vec<(usize, usize, usize, String)>> = std::env::var_os("RERAIL_STATS_KEEP_TRACE").is_some().then(Vec::new);
                for (rel, g, old, new) in staged {
                    let k = &kept[rel];
                    let width = fields.width(rel);
                    let holders = keeper.as_ref().and_then(|k| k.holders(rel)).unwrap_or(&no_holders);
                    // `RERAIL_KEPT_INPUTS_TRACE`: each input's old and new rows of the changed files, and its patch.
                    let trace_rows = std::env::var_os("RERAIL_KEPT_INPUTS_TRACE").is_some().then(|| (old.clone(), new.clone()));
                    let atoms = store.atoms_handle();
                    let cells = retain::cells_of(&atoms, prog, &maintained.domains, rel, width);
                    let mut patch = patch_owners(rel, k, g, holders, &evaluated, &old, new, &cells)?;
                    if let Some(t) = read_trace.as_mut() {
                        t.push((patch.read.1, patch.read.0, patch.read.2, rel.clone()));
                    }
                    if let Some((o, n)) = trace_rows.filter(|(o, n)| !o.iter().all(Vec::is_empty) || !n.is_empty() || !patch.changed.is_empty()) {
                        eprintln!("rerail: kept input {rel} (owner column {g}, {} kept rows): old {o:?}; new {n:?}; owners changed {:?}; minus {:?}; plus {:?}", k.source.len(), patch.changed.iter().map(|(o, r)| (*o, r.len())).collect::<Vec<_>>(), patch.minus, patch.plus);
                    }
                    let bound: Vec<String> = (0..width).map(|c| format!("c{c}")).collect();
                    let (source, added): (Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, _) = if patch.changed.is_empty() {
                        (retain::renamed(Arc::clone(&k.source), bound.clone()), None)
                    } else {
                        changed += 1;
                        let layered = retain::Layered::new(bound.clone(), Arc::clone(&k.source), Arc::clone(k.ranges()?), std::mem::take(&mut patch.changed));
                        let added = Arc::clone(layered.added());
                        (Arc::new(layered), Some(added))
                    };
                    results.push((rel.clone(), width, bound, source, patch, added));
                }
                if let Some(mut t) = read_trace {
                    let real = evaluated.len();
                    t.sort_unstable_by(|a, b| b.cmp(a));
                    let (rows, owners, same) = t.iter().fold((0, 0, 0), |(r, o, s), x| (r + x.0, o + x.1, s + x.2));
                    eprintln!("rerail: kept inputs patched: {real} documents evaluated; {owners} owners read ({same} unchanged), {rows} kept rows read; most: {:?}", &t[..t.len().min(6)]);
                }
                // Every input made: only now are they the run's, so a failure above leaves the run as it was.
                drop(_s2);
                let _s3 = crate::code_rule_dsl::logic::profile::span(|| "the inputs adopted".into());
                let mut adopted: HashMap<String, retain::KeptInput> = HashMap::new();
                for (rel, width, bound, source, patch, added) in results {
                    let k = &kept[&rel];
                    input_digests.insert(rel.clone(), patch.digest);
                    if let Some(st) = &k.stats {
                        kept_input_stats.insert(rel.clone(), (k.digest, st.clone()));
                    }
                    adopted.insert(rel.clone(), retain::KeptInput { source: retain::renamed(Arc::clone(&k.source), bound.clone()), layout: Arc::clone(&k.layout), digest: k.digest, stats: k.stats.clone() });
                    if let Some(docs) = patch.docs {
                        // Its rows' digests by owner, for the frames written back and the next run.
                        kept_input_docs.insert(rel.clone(), Arc::new(docs));
                        // Read where they lie (0364 R2): the rows gained among the appended ones, those lost among the kept
                        // ones. Named as the planner reads `r__plus`'s and `r__minus`'s statistics (`sql_col`).
                        let names = |suffix: &str| { let name = format!("{rel}__{suffix}"); (0..width).map(|c| super::sql_col(&name, c)).collect::<Vec<_>>() };
                        let none = || crate::code_rule_dsl::logic::native::component::columns_source(names("plus"), vec![Some(Vec::new()); width]);
                        let plus = added.map_or_else(none, |a| crate::code_rule_dsl::logic::native::component::InRel::picked(a, patch.plus, names("plus")));
                        let minus = crate::code_rule_dsl::logic::native::component::InRel::picked(Arc::clone(&k.source), patch.minus, names("minus"));
                        given_deltas.insert(rel.clone(), (plus, minus));
                    }
                    exports.borrow_mut().install(&rel, width, source);
                }
                // Decision 0458: each base relation is the corpus batch's, read in place; its versions were made from
                // the changed files' documents (`base_versions`), none where no file changed. Without the documents'
                // table, or with a changed file's versions missing, the kept route is not taken.
                for (rel, _) in &stored {
                    let source = corpus_sources.get(rel).ok_or_else(|| format!("{rel} is not in the corpus batch"))?;
                    if old_base.as_ref().is_none_or(|(changed, _)| !changed.is_empty() && !base_versions.contains_key(rel)) {
                        return Err(format!("{rel}: no versions of its changed files' documents"));
                    }
                    base_layered.insert(rel.clone(), Arc::clone(source));
                }
                drop(kept);
                kept_loaded = Some(adopted);
                *maintained_early.borrow_mut() = Some(maintained);
                Ok((changed, rels.len()))
            })();
            match installed {
                Ok((changed, all)) => crate::code_rule_dsl::logic::profile::decision("the kept inputs (decision 0117 steps 1, 2)", || format!("{} memo hits read for their outputs alone; {changed} of {all} inputs changed, from {} changed files, each its kept rows with its changed owners' rows", kept_hits.len(), counted_before.len())),
                Err(why) => {
                    crate::code_rule_dsl::logic::profile::decision("the kept inputs (decision 0117 steps 1, 2)", || format!("not taken ({why}): {} memo hits decoded whole", kept_hits.len()));
                    if std::env::var_os("RERAIL_STATS_KEEP_TRACE").is_some() {
                        eprintln!("rerail: the kept inputs not taken: {why}");
                    }
                    let memo = memo.ok_or_else(|| Error::Plan("kept hits without a memo".to_owned()))?;
                    let memo::Memo::Stored(memo_store) = memo else { return Err(Error::Plan("kept hits of a session's memo".to_owned())) };
                    for (path, (_, key, found)) in &kept_hits {
                        let entry = found
                            .whole(memo_store, &memo_plan)
                            .map_err(|e| Error::Plan(e.to_string()))?
                            .and_then(|w| w.projected(*key, &memo_row_needed).transpose())
                            .transpose()
                            .map_err(|e| Error::Plan(e.to_string()))?
                            .ok_or_else(|| Error::Plan(format!("{path}: its memo entry is gone")))?;
                        for (rel, (width, values)) in &entry.rows {
                            if values.is_empty() || !split.exports.contains(rel) {
                                continue;
                            }
                            let cols: Vec<Option<Vec<i64>>> = (0..*width).map(|c| Some(values.iter().skip(c).step_by(*width).copied().collect())).collect();
                            let rows = crate::code_rule_dsl::logic::native::component::columns_source((0..*width).map(|c| format!("c{c}")).collect(), cols);
                            exports.borrow_mut().add(rel, *width, rows.as_ref());
                        }
                    }
                }
            }
        }
        // The tree counts of the hits not held again (0096 step 3): the kept tree counts beyond those of the files it
        // saw. Given up, the skipped hits' are taken from the memo, as the replay would have. Their base rows are the
        // corpus batch's (decision 0458).
        match keeper.as_ref().and_then(|k| k.counted_own()).filter(|_| installed.get()) {
            Some(own) => {
                let mut skipped = own.clone();
                skipped.sub(&own_seen);
                own_totals.add(&skipped);
            }
            None => {
                let memo = memo.filter(|_| !given_up.borrow().is_empty());
                for (path, content, key) in std::mem::take(&mut *given_up.borrow_mut()).into_iter().filter(|_| memo.is_some()) {
                    let entry = memo.expect("checked").lookup(content, world::store_path(&path), key, &memo_plan).map_err(|e| Error::Plan(e.to_string()))?.ok_or_else(|| Error::Plan(format!("{path}: its memo entry is gone")))?;
                    own_totals.add(&Own::taken(&entry.rows));
                }
            }
        }
        // The exports' statistics as kept by file: a memo hit's rows are not measured again. An export whose kept rows
        // are not the rows held (a row handed to two files is held once) is measured from the rows held, as is every
        // export when the keeper was given up after the hits went unmeasured.
        // An export no chunk measured and the keeper gave none of (it gave up after its hits, as in a phase whose
        // files are all loads) is measured from its rows held too: the planner prices every input it reads.
        let unmeasured: BTreeSet<&String> = split.exports.iter().filter(|rel| !totals.rels.contains_key(rel.as_str())).collect();
        // `RERAIL_KEPT_DOCS` (Q20): the listed exports' rows this process counted, by document, against those held.
        if let Some(k) = keeper.as_ref() {
            for rel in split.exports.iter().filter(|r| keptstats::kept_docs_listed(r)) {
                for line in k.compare_documents(rel, &exports.borrow(), pf.types(rel).len()) {
                    eprintln!("RERAIL_KEPT_DOCS {rel}: {line}");
                }
            }
        }
        if hits_unmeasured || kept.is_some() || !unmeasured.is_empty() {
            let _s = crate::code_rule_dsl::logic::profile::span(|| "the exports' statistics, kept or measured".into());
            let tested = constants_tested_in(&prepared_placed.v);
            let (mut from_kept, mut measured_held) = (0usize, Vec::<String>::new());
            for rel in &split.exports {
                let types: Vec<&str> = pf.types(rel);
                let held = exports.borrow().len(rel);
                match kept.as_ref().and_then(|m| m.get(rel)).filter(|r| r.rows() == held as i64) {
                    Some(r) => {
                        totals.forget(rel);
                        totals.add(rel, held, &r.statistics(&|_| true));
                        // The planner prices a constant by its count (`const_freq`): a value column's counts as kept
                        // while few-valued, else those of the constants the rules test, counted in the rows held.
                        let mut count: Vec<(usize, i64)> = Vec::new();
                        for c in (0..types.len()).filter(|c| types[*c] != "node") {
                            match r.exact(c) {
                                Some(vals) => totals.consts.extend(vals.map(|(v, n)| (rel.clone(), format!("c{c}"), v, n))),
                                None => count.extend(tested.get(rel.as_str()).into_iter().flatten().filter(|(tc, _)| *tc == c)),
                            }
                        }
                        if !count.is_empty() {
                            let src = exports.borrow_mut().source(rel, types.len());
                            for (c, v) in count {
                                let n = (0..src.len()).filter(|&i| src.value(i, c) == v).count();
                                totals.consts.push((rel.clone(), format!("c{c}"), v, n as i64));
                            }
                        }
                        from_kept += 1;
                    }
                    None if hits_unmeasured || unmeasured.contains(rel) => {
                        let _m = crate::code_rule_dsl::logic::profile::span(|| format!("measure export {rel}"));
                        totals.forget(rel);
                        let src = exports.borrow_mut().source(rel, types.len());
                        totals.add_rows(rel, src.len(), src.as_ref(), &types);
                        // Why: the kept rows (or none kept) against those held.
                        let why = kept.as_ref().and_then(|m| m.get(rel)).map_or_else(|| "none kept".to_owned(), |r| format!("{} kept", r.rows()));
                        measured_held.push(format!("{rel} ({why}, {held} held)"));
                    }
                    None => {}
                }
            }
            crate::code_rule_dsl::logic::profile::decision("exports' statistics kept by file", || format!("{from_kept} kept; {} measured from the rows held: {}", measured_held.len(), measured_held.join(", ")));
        }
        match (session.as_ref(), keeper.take(), memo) {
            (Some(s), Some(k), _) => {
                s.stats.lock().expect("the session's statistics").insert(memo_program, (memo_whole, k));
            }
            // Written as one blob with the run, whose memo entries it names by content (0096 §2): a later process
            // patches it. A run that could not keep them writes it empty, so the next counts every file in.
            (None, Some(k), Some(memo::Memo::Stored(store))) if kept.is_none() || k.counted != (0, 0) => {
                let _s = crate::code_rule_dsl::logic::profile::span(|| "write the kept statistics, queued".into());
                // Its constants as the store numbers them, in the writer's transaction (decision 0170): a record of its
                // own (decision 0306), its relations' symbol columns asked here.
                let columns = k.symbol_columns(&|key, names| memo_plan.syms.columns(key, names.len()));
                fn apply(cx: &crate::content_store::record::Apply<'_>, body: crate::content_store::record::codec::Body) -> crate::content_store::StoreResult<()> {
                    let (plan, columns): ([u8; 32], std::collections::BTreeMap<String, Option<Vec<bool>>>) = crate::content_store::record::codec::from_body("the kept statistics, mapped", body)?;
                    if !cx.store.atoms_handle().forked() {
                        return cx.store.put_stat_blob(plan, cx.bytes);
                    }
                    let mut k = keptstats::Keeper::decode(cx.bytes).ok_or_else(|| crate::content_store::StoreError::Invalid("the kept statistics do not read back".into()))?;
                    k.map_symbols(&|key, _| columns.get(key).cloned().flatten(), true, |v| cx.store.symbols_to_store(crate::content_store::ProcessIds::of_process(v)).map(drop))?;
                    cx.store.put_stat_blob(plan, &k.encode())
                }
                store.queue_record(crate::content_store::write_back::Syms::Mapped, "the kept statistics, mapped", vec![crate::content_store::stat_blob_key(&memo_whole)], &(memo_whole, columns), Some(k.encode()), apply, false);
            }
            _ => {}
        }
        // The space's entries are this program's now, the statistics just kept with them.
        if let Some(memo::Memo::Stored(store)) = memo.filter(|_| memoised && !last_unmarked.get()) {
            memo::set_last_program(store, &memo_program, Some(memo_whole));
        }
        // A base relation's statistics (decision 0458): kept beside its documents' table, of the rows its digest
        // names. Kept of these rows, they stand; kept of the table's documents, the changed documents' old rows are
        // counted out and their new ones in (0096 §3's patch, of the documents rather than memo entries); else counted
        // off the corpus batch, read where its rows lie. Counted at the columns its rules read (`held`).
        let (table_digest, kept_stats) = &mut base_stats_kept;
        let tag = |rel: &str| super::row_digest(&rel.bytes().map(i64::from).collect::<Vec<_>>());
        let mut base_statistics = |rel: &str, source: &dyn crate::code_rule_dsl::facts::vtab::Source, held: &BTreeSet<&str>| -> Vec<(String, i64, i64, i64, i64)> {
            let columns = source.columns();
            let width = columns.len();
            let from: Vec<usize> = (0..width).filter(|&c| held.contains(columns[c].as_str())).collect();
            let names: Vec<String> = from.iter().map(|&c| columns[c].clone()).collect();
            let rows = source.len() as i64;
            let now = base_digest(rel);
            let counted = match kept_stats.remove(rel).filter(|(_, k)| k.columns() == names.as_slice()) {
                Some((d, k)) if d == now && k.rows() == rows => Some((k, "kept")),
                Some((d, mut k)) if Some(d) == table_digest.map(|t| t ^ tag(rel)) => base_wholes.get(rel).and_then(|(old, new)| {
                    for (v, sign) in [(old, -1), (new, 1)] {
                        if let Some(v) = v {
                            k.documents_read(v.len(), width, &|r, c| v.value(r, c), &from, sign);
                        }
                    }
                    (k.rows() == rows).then(|| {
                        if base_binds.is_some() {
                            base_stats_out.push((rel.to_owned(), now, k.clone()));
                        }
                        (k, "patched by the changed documents")
                    })
                }),
                _ => None,
            };
            let counted = counted.unwrap_or_else(|| {
                let _s = crate::code_rule_dsl::logic::profile::span(|| format!("count {rel}'s statistics off the corpus batch"));
                let mut k = super::statkeep::RelStats::new(names.clone());
                k.documents_read(source.len(), width, &|r, c| source.value(r, c), &from, 1);
                if base_binds.is_some() {
                    base_stats_out.push((rel.to_owned(), now, k.clone()));
                }
                (k, "counted")
            });
            let (counted, how) = counted;
            crate::code_rule_dsl::logic::profile::decision("the base relations' statistics (decision 0458)", || format!("{rel}: {how}"));
            let by_name: HashMap<String, (String, i64, i64, i64, i64)> = counted.statistics(&|c| held.contains(c)).into_iter().map(|m| (m.0.clone(), m)).collect();
            columns.iter().map(|c| by_name.get(c).cloned().unwrap_or((c.clone(), rows, rows, rows, rows))).collect()
        };
        for (rel, (columns, values)) in std::mem::take(&mut base_rows) {
            // A kept base relation with its delta (decision 0117 step 2): bound as it is, its digest taken from its
            // owners', its statistics the keeper's.
            if let Some(source) = base_layered.remove(&rel) {
                let rows = source.len();
                let held: BTreeSet<&str> = columns.iter().zip(&values).filter(|(_, v)| v.is_some()).map(|(c, _)| c.as_str()).collect();
                let statistics = if semijoin.contains(&rel) {
                    // A semi-join's, kept at these rows (`kept_input_stats`), else measured and kept for the next run.
                    match kept_input_stats.get(&rel).filter(|(d, _)| input_digests.get(&rel) == Some(d)) {
                        Some((_, st)) => st.clone(),
                        None => {
                            let st = syntax::statistics(source.as_ref());
                            semijoin_stats.insert(rel.clone(), st.clone());
                            st
                        }
                    }
                } else {
                    base_statistics(&rel, source.as_ref(), &held)
                };
                totals.add(&rel, rows, &statistics);
                input_digests.insert(rel.clone(), base_digest(&rel));
                base_sources.insert(rel, source);
                continue;
            }
            // Read off the corpus batch's documents (decision 0458), not the rows the chunks gathered.
            let source = Arc::clone(corpus_sources.get(&rel).ok_or_else(|| Error::Plan(format!("{rel} is not in the corpus batch")))?);
            let rows = source.len();
            let columns = source.columns();
            let held: BTreeSet<&str> = columns.iter().zip(&values).filter(|(_, v)| v.is_some()).map(|(c, _)| c.as_str()).collect();
            let statistics = (!semijoin.contains(&rel)).then(|| base_statistics(&rel, source.as_ref(), &held));
            let source = retain::kept_source_digested(parts.as_deref(), &rel, Arc::clone(&source), base_digest(&rel), &mut input_digests);
            let statistics = statistics.unwrap_or_else(|| {
                // A semi-join's, kept beside its frame for the next run's kept route (`kept_input_stats`).
                let st = syntax::statistics(source.as_ref());
                semijoin_stats.insert(rel.clone(), st.clone());
                st
            });
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
        for rel in std::mem::take(&mut base_rows).into_keys() {
            // Read off the corpus batch's documents (decision 0458), not the rows the chunks gathered.
            let source = Arc::clone(corpus_sources.get(&rel).ok_or_else(|| Error::Plan(format!("{rel} is not in the corpus batch")))?);
            base_sources.insert(rel.clone(), retain::kept_source_digested(parts.as_deref(), &rel, source, base_digest(&rel), &mut input_digests));
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
                format!("{}: {:.1} MB of values over {nodes} nodes: {}", phases::current().as_deref().unwrap_or("the program"), total as f64 * 8.0 / 1e6, named.join("; "))
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
    let one_fixpoint = run_placed.v.rules().next().is_some() && session.is_none() && !k4;
    // The import components (decision 0076) run before the cross part, with no hooks: what they read is bound now.
    let eager: BTreeSet<&str> = cross_plan.iter().flat_map(|p| p.components.rules().flat_map(|r| r.reads().map(|a| p.components.rel_name(a.rel())))).collect();
    // A new process that maintains its cross-file part from the store (t32 4.1) runs it as a session does: its
    // inputs bound before the run, not loaded as their first reader runs. Not a part whose requests the run answers
    // as it goes (0065 §2a, phase 0's loads): their answers are the loader's, which no kept row stands for.
    let kept_part_on = session.is_none()
        && durable.is_some()
        && one_fixpoint
        && kept_strata::part_wanted()
        // Read from the part's version: a part with no answers has no relation they produce.
        && cross.placed().ok().is_some_and(|p| p.fields().is_ok_and(|f| f.answers().is_empty()))
        && !phase0::active();
    for rel in &split.exports {
        if one_fixpoint && !kept_part_on && !eager.contains(rel.as_str()) {
            let width = pf.width(rel);
            let empty = crate::code_rule_dsl::logic::native::component::columns_source((0..width).map(|c| format!("c{c}")).collect(), vec![Some(Vec::new()); width]);
            modules.bind(&db, rel, empty)?;
            continue;
        }
        let source = exports.borrow_mut().source(rel, pf.width(rel));
        // As a base relation's (`retain::kept_source`): the last run's copy if its rows are the same, else this one with
        // the last run's sorted columns carried into it (decision 0101 §5).
        let source = {
            let parts = session.as_ref().map(|s| s.cross.lock().expect("the session's cross-file parts"));
            let cells = retain::cells_of(&input_atoms, &run_placed, &lowered.domains, rel, source.columns().len());
            retain::kept_source(parts.as_deref(), rel, source, &cells, &mut input_digests)
        };
        cross_inputs.push((rel.clone(), Arc::clone(&source)));
        modules.bind(&db, rel, source)?;
    }
    // The cone of requests' rows and the answers, as the rest of the cross-file part reads them (decision 0065 §2a).
    if let Some((load, ..)) = &loading {
        for (rel, rows) in std::mem::take(&mut loading_rows) {
            let width = load.decl_of(&rel).expect("a relation of the cone").arity();
            let names: Vec<String> = (0..width).map(|c| format!("c{c}")).collect();
            let columns = (0..width).map(|c| Some(rows.iter().map(|r| r[c]).collect())).collect();
            let source = crate::code_rule_dsl::logic::native::component::columns_source(names, columns);
            // Loading answers are inputs to maintenance as well as to execution. Omitting them here makes a
            // retained output look unchanged after an import edit, although its bound input has new rows.
            let source = {
                let parts = session.as_ref().map(|s| s.cross.lock().expect("the session's cross-file parts"));
                let cells = retain::cells_of(&input_atoms, &run_placed, &lowered.domains, &rel, source.columns().len());
                retain::kept_source(parts.as_deref(), &rel, source, &cells, &mut input_digests)
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
    // The source texts no wave took (decision 0300), and the chunks' freed pages, go back before the cross part
    // builds its own.
    {
        let (files, bytes) = own_sources.release();
        crate::code_rule_dsl::logic::profile::decision("source texts read once (decision 0300)", || format!("{files} kept texts left untaken as the waves ended, {:.1} MB", bytes as f64 / 1e6));
    }
    { let _s = crate::code_rule_dsl::logic::profile::span(|| "release the chunks' pages".into()); crate::code_rule_dsl::prof::release(); }
    phase("stats", statements.count());
    // What the file-local part left queued (the files' stats) goes to the writer now, not with the cross part's
    // writes at its end (decision 0296).
    crate::content_store::write_back::end_wave(crate::content_store::write_back::Boundary::Local);
    statements.enter("cross");
    // Strata a session's run evaluated, of all (decision 0077).
    let mut cross_strata: Option<(usize, usize)> = None;
    let mut cross_kept: Option<(usize, usize)> = None;
    let mut cross_maintained = 0;
    let mut cross_handed = 0usize;
    let mut cross_unloaded = 0usize;
    let mut cross_derived_again = 0;
    let mut cross_quiet = (0, 0);
    let (mut cross_changed, mut cross_again) = (Vec::new(), BTreeMap::new());
    let cross_delta_counts = [0usize; 6];
    let mut cross_recursive = 0;
    // `returns`, held for the typing outputs (`corpus::typing`), when the cross-file part derives it: a program that
    // declares it and derives it per file (`python/typing.rr` in a rule's program) has no table of it here.
    let mut wanted: BTreeSet<String> = run_f.declared("returns").then(|| "returns".to_owned()).into_iter().filter(|r| !head_outputs.contains(r)).collect();
    // The import components (decision 0076): derived first, from the exports, and each one's stored rows looked up.
    // The hits' rows are bound as `<rel>__cross` with `rr_cross_doc`; the misses are derived and stored after.
    let mut cross_state: Option<(cross_memo::Components, cross_memo::Found)> = None;
    if let Some(plan) = &cross_plan {
        let components_span = crate::code_rule_dsl::logic::profile::span(|| "import components".into());
        let held: BTreeSet<String> = cross_memo::HELD.iter().map(|r| (*r).to_owned()).collect();
        let got = texts_run(&part_of(plan.components.clone()), &db, cross_stats.clone(), held, modes, &vocabulary)?;
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
        let outputs_of = |rel: &str| head_outputs.iter().any(|o| o == rel);
        native.extend(gather(got, &mut outputs, &mut node_columns, &outputs_of).into_iter().map(|k| format!("components:{k}")));
        let components = cross_memo::Components::new(rows, &edges);
        let found = cross_memo::lookup(&store, plan, &components, &paths)?;
        // Every component found: a memoised relation's own rules derive nothing (each is guarded to the documents not
        // found), so only its copy from `<rel>__cross` runs, and only the copies something reads (decision 0076).
        let all_found = cross_mode == cross_memo::Mode::On && found.misses.is_empty() && !plan.memo.is_empty();
        if all_found {
            let before = cross.version.size();
            cross_memo::keep_copies(&mut cross, plan, &wanted)?;
            let after = cross.version.size();
            crate::code_rule_dsl::logic::profile::decision("import components (decision 0076)", || {
                format!("every component found: {} rules of memoised relations not run, their copies only", before - after)
            });
        }
        // No component found (a new store): `rr_cross_doc` is empty and every guard passes, so none is run.
        if found.hits.is_empty() && cross_mode != cross_memo::Mode::Verify {
            let dropped = cross_memo::drop_guards(&mut cross)?;
            crate::code_rule_dsl::logic::profile::decision("import components (decision 0076)", || format!("no component found: {dropped} guards on `rr_cross_doc` not run"));
        }
        let cross_now = cross.placed()?;
        let cross_f = cross_now.fields()?;
        let read: BTreeSet<&str> = cross_now.v.rules().flat_map(|r| r.reads().map(|a| cross_now.v.rel_name(a.rel()))).collect();
        let mut replayed = Totals::new(tested_values.clone());
        let inputs = found.inputs(&store, plan, &components, cross_mode, |r| !all_found || read.contains(format!("{r}{}", cross_memo::SUFFIX).as_str()))?;
        let replayed_rows: usize = inputs.iter().skip(1).map(|(_, _, cols)| cols.first().map_or(0, Vec::len)).sum();
        for (rel, width, cols) in inputs {
            let types: Vec<String> = cross_f.types(&rel).into_iter().map(String::from).collect();
            let columns = (0..width).map(|i| format!("c{i}")).collect();
            let source = crate::code_rule_dsl::logic::native::component::columns_source(columns, cols.into_iter().map(Some).collect());
            replayed.add_rows(&rel, source.len(), source.as_ref(), &types.iter().map(String::as_str).collect::<Vec<_>>());
            modules.bind(&db, &rel, source)?;
        }
        replayed.apply(&mut cross_stats);
        // Held for storing, unless an output (written to its table, read from there): only when a component missed or
        // the run verifies. Every component found is replayed, and nothing is stored.
        if !found.misses.is_empty() || cross_mode == cross_memo::Mode::Verify {
            wanted.extend(plan.memo.keys().filter(|r| !head_outputs.contains(*r)).cloned());
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
    // `derived`: the relations the part derives, each with its width (`derived_widths_in`).
    let mut collect = |db: &Connection, got: &Outcome, derived: BTreeMap<&str, usize>| -> Result<()> {
        for (rel, width) in derived.into_iter().filter(|(r, _)| memo_rels.contains(*r)) {
            let rows = held_rows(db, got, rel, width)?;
            memo_rows.entry(rel.to_owned()).or_default().extend(rows);
        }
        Ok(())
    };
    let mut returns = Vec::new();
    // A session's outputs whose strata were clean, written by its last run of this program to this directory: their
    // files are as they were, and their row counts. And that program's key, to record what this run writes.
    let mut unchanged_outputs: HashMap<String, usize> = HashMap::new();
    let mut written_key: Option<[u8; 32]> = None;
    if run_placed.v.rules().next().is_some() {
        // RERAIL_DUMP_CROSS=<file>: the cross-file part's rules, one per line (a debugging aid).
        if let Ok(path) = std::env::var("RERAIL_DUMP_CROSS") {
            let _ = std::fs::write(path, run_placed.v.rules().map(|r| format!("{}\n", run_placed.v.show_rule(r))).collect::<String>());
        }
        let outputs_of = |rel: &str| head_outputs.iter().any(|o| o == rel);
        {
            // A new process keeps its cross-file part in the store, and maintains it as a session does (t32 4.1):
            // the session's branch, its kept part read from the store's frames and written back after the run.
            let kept_part: Option<std::sync::Mutex<HashMap<[u8; 32], retain::Retained>>> =
                kept_part_on.then(Default::default);
            // A run with no session keeps its part in the frames (K4): held here for the run, read from them and
            // written back to them. (`kept_part` is t32 4.1's form of it, taken only with `RERAIL_CROSS_K4=off`.)
            let kept_cells: Option<std::sync::Mutex<HashMap<[u8; 32], retain::Retained>>> = (session.is_none() && k4).then(Default::default);
            match session.as_ref().map(|s| &s.cross).or(kept_part.as_ref()).or(kept_cells.as_ref()) {
                // A session's run evaluates the strata whose inputs changed, the others' rows kept (decision 0077).
                Some(cell) => {
                    // A session maintains its cross-file part from the rows the last run kept: each relation is
                    // stored, not recomputed in its readers (decision 0099), so the next run has its old rows. An
                    // inlined one left none, and every stratum over it was derived again (about 100 a step at O125,
                    // against none stored). A relation the program declares `inline` stays so;
                    // `RERAIL_SESSION_INLINE=on` inlines as a full run does, to compare.
                    // The relations the program declares `inline`: inlined in a session as in a full run. The session
                    // keeps no rows of theirs (the run's `Outcome::inlined` names them), and the strata over them are
                    // derived again each run: the cost the author chose by writing `inline`.
                    let declared_inline: BTreeSet<String> = run_f.decls_by_name().into_iter().map(|(r, _)| r).filter(|r| run_f.qualified(r, "inline")).map(String::from).collect();
                    let maintained_part = match maintained_early.borrow_mut().take() {
                        Some(m) => m,
                        None => maintained_cross_of(shared_maintained, &cross)?,
                    };
                    let cross = &maintained_part;
                    // The run reads its version, not its tree (t36 item 7).
                    let cross_placed = cross.placed()?;
                    let cross_f = cross_placed.fields()?;
                    // Its strata, once for every pass below (the kept part's load, cut, frames and derived relations).
                    let cross_sccs = cross_placed.components()?;
                    // Each linear cost of a session's run is its own span (decision 0090, step 1).
                    let span = |name: &'static str| crate::code_rule_dsl::logic::profile::span(move || format!("session/{name}"));
                    let s = span("digest inputs");
                    // The base relations are read off the corpus batch and kept as their documents' digest alone, no
                    // rows (decision 0458).
                    let digest_only: BTreeSet<String> = stored.iter().map(|(r, _)| r.clone()).collect();
                    // A base relation's digest was taken when its source was made: its documents' (decision 0458).
                    let digests: HashMap<String, u64> = cross_inputs
                        .iter()
                        .map(|(rel, source)| (rel.clone(), input_digests.get(rel).copied().unwrap_or_else(|| retain::digest(source.as_ref(), &retain::cells_of(&store.atoms_handle(), &cross_placed, &cross.domains, rel, source.columns().len())))))
                        .collect();
                    drop(s);
                    let (ckey, binds) = retain::keys(&prepared_key, cross.digest()?);
                    part_keys.lock().expect("the parts' keys").push(ckey);
                    let mut cells = cell.lock().expect("the session's cross-file part");
                    // Taken out while the run uses it; a run that fails leaves no kept part for this program.
                    // Kept by the cross program alone (`binds`): a file-local rule's edit changes `ckey`, not the part's
                    // program, whose inputs `cut` compares by digest. The part is taken under this run's key.
                    let mut cell = cells.remove(&binds).map(|c| c.rekeyed(ckey));
                    if let Some(c) = cell.as_mut() {
                        c.rebind(store)?;
                    }
                    let mut kept_frames = None;
                    if kept_part.is_some() {
                        let (loaded, p) = kept_strata::load(store, root, ckey, binds, &cross_placed, &cross.domains, &cross_sccs, &cross_inputs, &digests)?;
                        (cell, kept_frames) = (loaded, Some(p));
                    }
                    // The phase's space (`kept_strata::space`): a relation two programs of one phase derive alike (its
                    // rules' identity) is one frame, kept once; phase 1's inputs and the pack's are in spaces apart.
                    let frames_space = kept_strata::space(root);
                    if cell.is_none() && kept_cells.is_some() {
                        let _s = span("the kept part, from the frames");
                        let strata = cross_idents.iter().find(|(d, _)| *d == cross_placed.digest()).map(|(_, s)| s);
                        cell = Some(retain::Retained::bind_kept_named(store, frames_space, ckey, binds, &cross_placed, &cross_sccs, &digests, strata)?);
                        if let (Some(c), Some(k)) = (cell.as_mut(), kept_loaded.as_ref()) {
                            c.adopt_inputs(k);
                        }
                        // The base relations' old versions (decision 0458), where the frames keep them as changed.
                        if let Some(c) = cell.as_mut() {
                            c.adopt_old_inputs(&base_olds);
                        }
                    }
                    let mut cut = retain::cut(cell.as_ref(), ckey, &cross_placed, &cross_sccs, &digests)?;
                    if kept_frames.is_some() {
                        cross_kept = Some(cut.strata);
                    }
                    // Decision 0117 step 4: a maintained run reads the kept rows where a stratum it reaches reads them.
                    let drive = cross_f.gates().is_empty() && std::env::var_os("RERAIL_MAINTAIN_WHOLE").is_none();
                    if let Some(kept) = cell.as_mut().filter(|_| kept_cells.is_some()) {
                        if let Some((rels, rows)) = kept.load(store, &cut, &cross_placed, drive)? {
                            crate::code_rule_dsl::logic::profile::decision("the kept part, from the frames (t33 K4)", || format!("{rels} relations' rows read ({rows} rows) for the strata that run"));
                        } else {
                            cut = retain::cut(None, ckey, &cross_placed, &cross_sccs, &digests)?;
                        }
                    }
                    let was_kept = cell.as_ref().filter(|_| kept_cells.is_some()).and_then(|k| k.in_kept().cloned());
                    // An input with the rows it had is read from the last run's source, with its indexes and statistics.
                    let s = span("bind unchanged inputs");
                    let same = cell.as_ref().map(|kept| kept.same_inputs(ckey, &digests)).unwrap_or_default();
                    let at: HashMap<String, usize> = cross_inputs.iter().enumerate().map(|(i, (r, _))| (r.clone(), i)).collect();
                    for (rel, source) in same {
                        if let Some(&i) = at.get(rel.as_str()) {
                            modules.bind(&db, &rel, Arc::clone(&source))?;
                            cross_inputs[i].1 = source;
                        }
                    }
                    drop(s);
                    written_key = Some(ckey);
                    // Only from a stack as the session's last run here left it: a stat of its file, not a read.
                    if let Some(s) = session.as_ref()
                        && s.stacks.lock().expect("the session's stacks").get(out).is_some_and(|stack| stack.stands(out))
                    {
                        let written = s.written.lock().expect("the session's written outputs");
                        // Not the findings nor typing's outputs, which this run reads as well as writes, nor the
                        // findings' relations, whose messages the writer composes from the templates (decision 0225
                        // §3: a message's edit leaves their rows as they were).
                        for rel in cross_f.outputs().filter(|r| cut.clean.contains(*r) && !matches!(*r, "finding" | "type_of") && !crate::code_rule_dsl::logic::rr::is_diagnostic(r)) {
                            if let Some((_, n)) = written.get(&out.join(rel)).filter(|(k, _)| *k == ckey) {
                                unchanged_outputs.insert(rel.to_owned(), *n);
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
                    if cut.strata.0 > 0 {
                        crate::code_rule_dsl::logic::costs::begin_read();
                    }
                    if std::env::var_os("RERAIL_CROSS_VERIFY").is_some() {
                        let read: BTreeSet<&str> = cross_placed.v.rules().flat_map(|r| r.reads()).map(|a| cross_placed.v.rel_name(a.rel())).collect();
                        let undigested: Vec<&str> = cross_f.inputs().filter(|r| !digests.contains_key(*r) && read.contains(r)).collect();
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
                            let was = kept.docs(&memo_whole);
                            if was.is_none() && std::env::var_os("RERAIL_STRATA_LIVE").is_some() {
                                eprintln!("RERAIL_STRATA_LIVE changed documents: the kept run holds no documents of this program");
                            }
                            was.and_then(|was| input_delta::changed(was, &run_docs_held.lock().expect("the run's documents"), &held.memo, &memo_whole))
                        }
                        _ => None,
                    };
                    let (mut from_entries, mut from_entries_rows, mut hashed) = (0usize, 0usize, Vec::<String>::new());
                    // The kept inputs' digests by owner after the edit (decision 0117 step 2), for the frames written back.
                    let mut input_docs: HashMap<String, Arc<retain::DocDigests>> = std::mem::take(&mut kept_input_docs);
                    let mut from_owners = 0usize;
                    let mut answer = |rel: &str| -> Option<(Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)> {
                        if by_rules_only {
                            return None;
                        }
                        // A kept input's delta is beside it: the rows its changed owners gained and lost.
                        if let Some((plus, minus)) = given_deltas.get(rel) {
                            from_owners += 1;
                            return Some((Arc::clone(plus), Arc::clone(minus)));
                        }
                        let kept = cell.as_ref()?;
                        let Some((_, new)) = cross_inputs.iter().find(|(r, _)| r == rel) else {
                            if std::env::var_os("RERAIL_PROFILE").is_some() { eprintln!("rerail: input delta of {rel}: not among the cross part's inputs, diffed by rules"); }
                            return None;
                        };
                        if let Some(changes) = &changes {
                            let _s = crate::code_rule_dsl::logic::profile::span(|| "session/input delta from the changed documents".into());
                            // An export's, from its entries (a base relation's rows are in no entry, decision 0458).
                            let read: Option<(String, Vec<Option<usize>>)> = if split.exports.iter().any(|e| e == rel) {
                                Some((rel.to_owned(), (0..new.columns().len()).map(Some).collect()))
                            } else {
                                None
                            };
                            // Its rows add up: the old less those gone, with those new, are its rows now.
                            let adds_up = |d: &input_delta::Delta| kept.input_source(rel).is_some_and(|old| old.len() + d.new.len() == new.len() + d.gone.len());
                            let live = std::env::var_os("RERAIL_STRATA_LIVE").is_some();
                            let read_some = read.is_some();
                            let delta = read.and_then(|(key, cols)| input_delta::delta(changes, &key, &cols));
                            if live {
                                let why = match &delta {
                                    _ if !read_some => "not an export".to_owned(),
                                    None => "a row not its document's alone".to_owned(),
                                    Some(d) if !adds_up(d) => format!(
                                        "rows do not add up: old {:?} + new {} != now {} + gone {}",
                                        kept.input_source(rel).map(|o| o.len()), d.new.len(), new.len(), d.gone.len()
                                    ),
                                    Some(d) => format!("from the changed documents' entries: {} gone, {} new, of {} rows", d.gone.len(), d.new.len(), new.len()),
                                };
                                eprintln!("RERAIL_STRATA_LIVE input delta {rel}: {why}");
                            }
                            if let Some(d) = delta.filter(adds_up) {
                                from_entries += 1;
                                from_entries_rows += d.gone.len() + d.new.len();
                                let width = new.columns().len();
                                // Read from the entries where the rows lie (0364 R2). Named as the planner reads
                                // `r__plus`'s and `r__minus`'s statistics (`sql_col`).
                                let names = |suffix: &str| { let name = format!("{rel}__{suffix}"); (0..width).map(|c| super::sql_col(&name, c)).collect::<Vec<_>>() };
                                let d = Arc::new(d);
                                return Some((d.source(false, names("plus")), d.source(true, names("minus"))));
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
                        let Some((delta, docs)) = retain::document_delta(rel, old.as_ref(), kept.input_docs(rel).map(|d| d.as_ref()), new.as_ref(), retain::group_col(&cross_placed, &split.locations, rel), &retain::cells_of(&store.atoms_handle(), &cross_placed, &cross.domains, rel, new.columns().len())) else {
                            if trace { eprintln!("rerail: input delta of {rel}: not by document (old columns {:?}, new {:?}, {} and {} rows), diffed by rules", old.columns(), new.columns(), old.len(), new.len()); }
                            if live {
                                eprintln!("RERAIL_STRATA_LIVE input delta {rel}: by the rules (no document delta)");
                            }
                            return None;
                        };
                        input_docs.insert(rel.to_owned(), docs);
                        hashed.push(rel.to_owned());
                        if std::env::var_os("RERAIL_SPIKE_DELTA").is_some() {
                            eprintln!("O125 input delta {rel}: +{} -{} of {} rows", delta.0.len(), delta.1.len(), new.len());
                        }
                        Some(delta)
                    };
                    // Decision 0117 step 3: with a kept part, the run walks the base program and makes a stratum's
                    // maintained form when a changed read reaches it (`Driver`); the inputs' deltas are taken here.
                    // `RERAIL_MAINTAIN_WHOLE` rewrites the whole program first, as before.
                    let driven = cell.is_some() && drive;
                    let mut seeds: Vec<String> = Vec::new();
                    let mut given_rels: BTreeSet<String> = BTreeSet::new();
                    let mut maintained = if driven {
                        let (m, s, g) = driven_part(&cut, cross, &mut answer)?;
                        seeds = s;
                        given_rels = g;
                        m
                    } else {
                        // No kept part, or a part with gates (which the level driver does not walk): the strata the cut
                        // runs are derived again from their reads, the clean ones bound as kept (decision 0364: no
                        // maintained forms).
                        maintain::Maintained::again(cut.program(cross)?, cut.strata.0)
                    };
                    if maintain::levels_wanted() && !driven {
                        qualify(&mut maintained.program, |heads, qualifiers| {
                            for rel in heads {
                                qualifiers.entry(rel.clone()).or_default().insert("levels".to_owned());
                            }
                        })?;
                    }
                    crate::code_rule_dsl::logic::profile::decision("input deltas", || match &changes {
                        Some(c) => format!("{} documents changed; {from_owners} inputs' deltas kept beside them; {from_entries} read from their memo entries ({from_entries_rows} rows), {} found by hashing their documents: {}", c.len(), hashed.len(), hashed.join(", ")),
                        None => format!("{from_owners} inputs' deltas kept beside them (decision 0117 step 2); {} found by hashing their documents", hashed.len()),
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
                        let mut stored: BTreeSet<String> = cut.derived(&cross_sccs)?.into_iter().chain(wanted.iter().cloned()).collect();
                        if aids {
                            let placed = maintained.program.placed()?;
                            stored.extend(placed.fields()?.decls_by_name().into_iter().map(|(r, _)| r).filter(|r| ["__del", "__ins", "__minus", "__plus"].iter().any(|x| r.ends_with(x))).map(str::to_owned));
                        }
                        qualify(&mut maintained.program, |_, qualifiers| {
                            for rel in stored.into_iter().filter(|r| !declared_inline.contains(r)) {
                                let q = qualifiers.entry(rel).or_default();
                                q.remove("inline");
                                q.insert("no_inline".to_owned());
                            }
                        })?;
                    }
                    // RERAIL_DUMP_MAINTAINED=<file>: the maintained program's rules, appended each run (a debugging aid).
                    if let Ok(path) = std::env::var("RERAIL_DUMP_MAINTAINED") {
                        use std::io::Write as _;
                        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                            let _ = writeln!(f, "=== run");
                            let v = &maintained.program.version;
                            for r in v.rules() {
                                let _ = writeln!(f, "{}", v.show_rule(r));
                            }
                        }
                    }
                    let part = &maintained.program;
                    // The part's fields, read from its version: no reader below decodes its tree.
                    let placed = part.placed()?;
                    let fields = placed.fields()?;
                    step(&format!("rewrite: {} maintained ({} recursive), {} derived again, {} rules", maintained.maintained, maintained.recursive, maintained.derived_again, placed.v.rules().count()));
                    let mut stats = cross_stats;
                    let s = span("bind and measure kept");
                    let clean = cell.as_ref().map(|kept| cut.sources(kept, &|r| fields.is_input(r))).unwrap_or_default();
                    // The clean relations the part reads that are left stored: bound where a stratum reads them.
                    let clean_reads: BTreeSet<String> = fields.inputs().filter(|r| cut.clean.contains(*r) && !clean.iter().any(|(c, _, _)| c == r)).map(str::to_owned).collect();
                    // Each kept relation bound as `r__old`, and its statistics kept by delta if a run kept them: this
                    // run's `r`, maintained from it, has them patched by its delta (decision 0096 step 4). The kept
                    // relations whose statistics are made when first planned with (decision 0101 §4), and
                    // `RERAIL_EAGER_KEPT_STATS` to make them all at binding, as before.
                    let mut binder = Binder {
                        constants: constants_tested_in(&placed.v).into_iter().map(|(r, c)| (r.to_owned(), c)).collect(),
                        cell: cell.as_ref(),
                        measured_kept: cell.as_ref().map(retain::Retained::measured_all).unwrap_or_default(),
                        bound_old: HashMap::new(),
                        lazy_kept: std::env::var_os("RERAIL_EAGER_KEPT_STATS").is_none(),
                        lazies: Vec::new(),
                        copies: {
                            let inputs: HashMap<&str, &Arc<dyn crate::code_rule_dsl::facts::vtab::Source>> = cross_inputs.iter().map(|(i, s)| (i.as_str(), s)).collect();
                            let is_input = |r: &str| inputs.contains_key(r);
                            let heads = rule_by_head(&cross_placed.v);
                            let mut copies: HashMap<String, (String, Vec<String>, Vec<usize>)> = cross_inputs.iter().map(|(i, src)| (i.clone(), (i.clone(), src.columns(), (0..src.columns().len()).collect()))).collect();
                            for rel in fields.decls_by_name().into_iter().filter_map(|(r, _)| r.strip_suffix("__old")).filter(|r| !is_input(r)) {
                                let Some((src, map)) = projection_of(&cross_placed.v, &heads, &is_input, rel) else { continue };
                                let Some(columns) = inputs.get(src.as_str()).map(|s| s.columns()) else { continue };
                                copies.insert(rel.to_owned(), (src, columns, map));
                            }
                            copies
                        },
                    };
                    // The clean relations, and the changed inputs' deltas.
                    for (rel, source, held) in clean {
                        bind_kept(&mut binder, &modules, &db, &mut stats, rel, source, held, false)?;
                    }
                    drop(s);
                    // Each input's statistics as this run plans with them: next run's `__old`'s (`retain::InputStats`).
                    let mut input_stats: HashMap<String, retain::InputStats> = cross_inputs.iter().filter_map(|(r, src)| Some((r.clone(), stats.of_relation(r, &src.columns())?))).collect();
                    // A copy of an input's columns has the input's statistics of those columns (`projection_of`): kept
                    // as an input's are, and its `__old` bound with them next run, never counted, when it holds as
                    // many rows as the input did (none collapsed).
                    let inputs: HashMap<&str, &Arc<dyn crate::code_rule_dsl::facts::vtab::Source>> = cross_inputs.iter().map(|(i, s)| (i.as_str(), s)).collect();
                    let is_input = |r: &str| inputs.contains_key(r);
                    let heads = rule_by_head(&cross_placed.v);
                    for rel in cross_f.decls_by_name().into_iter().map(|(r, _)| r).filter(|r| !is_input(r)) {
                        let Some((src, map)) = projection_of(&cross_placed.v, &heads, &is_input, rel) else { continue };
                        let Some(columns) = inputs.get(src.as_str()).map(|s| s.columns()) else { continue };
                        let Some((cols, vals)) = stats.of_columns(&src, &columns, &map) else { continue };
                        input_stats.insert(rel.to_string(), (cols, vals));
                    }
                    let derived = cut.derived(&cross_sccs)?;
                    let mut keep = wanted.clone();
                    // An output is written to its table, where the write phase reads it, and read back from there.
                    keep.extend(derived.iter().filter(|rel| !fields.is_output(rel)).cloned());
                    // The O125 spike (`RERAIL_SPIKE_DELTA`): each maintained relation's overdeleted and inserted rows.
                    let spike = std::env::var_os("RERAIL_SPIKE_DELTA").is_some();
                    step("bound; running");
                    if std::env::var_os("RERAIL_CROSS_DELTA_COUNTS").is_some() || std::env::var_os("RERAIL_CROSS_VERIFY").is_some() {
                        keep.extend(fields.decls_by_name().into_iter().map(|(r, _)| r).filter(|r| ["__del", "__ins", "__minus", "__plus", "__cand", "__chk", "__gone", "__held", "__redo"].iter().any(|x| r.ends_with(x))).map(str::to_owned));
                    }
                    // `RERAIL_CROSS_DEMAND_SPLIT`: keep the demand split in a session (to measure what dropping it costs).
                    let _whole = std::env::var_os("RERAIL_CROSS_DEMAND_SPLIT").is_none().then(super::demand::whole_relations);
                    let verify_stats = std::env::var_os("RERAIL_CROSS_VERIFY").map(|_| stats.clone());
                    let s = span("plan and evaluate");
                    // A derived relation derived again (no delta: every one of a cold run's, a new build's first) is handed
                    // off once no later stratum reads it, its frame made then, and the engine drops its rows (Q74): only
                    // where the frames are written, and not one the caller reads after the run or a check reads. One
                    // maintained by a delta is held, and written as its delta at the end. A run that keeps nothing (no
                    // clean relation: a cold run, a new build's first) lets its inputs go too: their frames written
                    // before the run, each input (its rows and the indexes built in them) let go after its last reader
                    // (`Hooks::unload`). Its part is not held for the next run (it reads the frames), so neither are
                    // its inputs; a run that keeps some (a session's edit) keeps its inputs for the next (0094 §4).
                    // Never with the kept strata (`kept_strata::keep`, which writes each input as held): those need
                    // one fixpoint, the frames need K4.
                    let hand_off = was_kept.is_some()
                        && !spike
                        && std::env::var_os("RERAIL_CROSS_VERIFY").is_none()
                        && std::env::var_os("RERAIL_CROSS_DELTA_COUNTS").is_none()
                        && std::env::var("RERAIL_CROSS_HAND_OFF").map_or(true, |v| v != "off");
                    let handed: std::cell::RefCell<HashMap<String, retain::Handed>> = std::cell::RefCell::new(HashMap::new());
                    let mut unloaded: Vec<String> = Vec::new();
                    let let_go_inputs = hand_off && cut.clean.is_empty();
                    // The relations the driver derives again (`Maintain::no_delta`), as it reaches them.
                    let derived_again: std::cell::RefCell<BTreeSet<String>> = std::cell::RefCell::new(BTreeSet::new());
                    // The inputs' frames, before the run: they depend on the inputs alone.
                    let inputs_done = match (&was_kept, let_go_inputs) {
                        (Some(was), true) => Some(retain::write_input_kept(store, was, &cross_placed, &cross.domains, &split.locations, &cross_inputs, &digests, &|rel: &str| input_docs.get(rel).cloned(), &semijoin_stats, &digest_only)),
                        _ => None,
                    };
                    // `false`: not taken, so the run holds it to its end as before.
                    let mut hand = |rel: &str| -> Result<bool> {
                        if wanted.contains(rel) || fields.is_output(rel) || (driven && !derived_again.borrow().contains(rel)) {
                            return Ok(false);
                        }
                        let Some(held) = crate::code_rule_dsl::logic::native::component::held(&db, rel) else { return Ok(false) };
                        let Some(h) = retain::Handed::of(store, &cross_placed, &cross.domains, rel, &held, was_kept.as_ref()) else { return Ok(false) };
                        handed.borrow_mut().insert(rel.to_owned(), h);
                        Ok(true)
                    };
                    // An input no later stratum reads: its binding, its place among the run's inputs and its
                    // export's rows let go, so its rows and the indexes built in them go (the engine has dropped
                    // its own copy, `forget_inputs_of`). Not one read after the run (`wanted`, typing's `fn_q`).
                    let mut unload = |gone: &[String]| -> Result<()> {
                        for rel in gone.iter().filter(|r| !wanted.contains(*r) && r.as_str() != "fn_q") {
                            if let Some(b) = modules.get(rel) {
                                b.clear();
                            }
                            if let Some((_, held)) = cross_inputs.iter_mut().find(|(r, _)| r == rel) {
                                let columns = held.columns();
                                let width = columns.len();
                                *held = crate::code_rule_dsl::logic::native::component::columns_source(columns, vec![Some(Vec::new()); width]);
                            }
                            exports.borrow_mut().release(rel);
                            unloaded.push(rel.clone());
                        }
                        Ok(())
                    };
                    let got = match driven.then_some(()) {
                        // Decision 0117 step 3: the base program, each stratum maintained by its own form when reached.
                        Some(()) => {
                            let kept = cell.as_ref().expect("a driven run has a kept part");
                            // The clean reads left stored: known by their rows' count until a stratum reads them.
                            let counts = kept.stored_rows(store, &clean_reads.iter().cloned().collect::<Vec<_>>());
                            for rel in &clean_reads {
                                let width = fields.arity(rel).unwrap_or(0);
                                let rows = counts.get(rel).copied().unwrap_or(1);
                                stats.lazily(rel, rows, Arc::new(crate::code_rule_dsl::logic::plan::Unbound { rel: rel.clone(), width, rows }));
                            }
                            let mut driver = Driver {
                                cross_placed: &cross_placed,
                                kept,
                                store: Some(store),
                                clean_reads,
                                at_end: wanted.clone(),
                                binder: &mut binder,
                                modules: &modules,
                                db: &db,
                                has_delta: given_rels,
                                no_delta: BTreeSet::new(),
                                derived_again: &derived_again,
                                maintains: BTreeSet::new(),
                                seeds: std::mem::take(&mut seeds),
                                given: maintained.given.clone(),
                                counts: (0, 0, 0),
                                again: Vec::new(),
                                walked: (0, 0, 0),
                            };
                            let hooks = crate::code_rule_dsl::logic::plan::Hooks { maintain: Some(&mut driver), hand_off: hand_off.then_some(&mut hand as &mut dyn FnMut(&str) -> Result<bool>), unload: let_go_inputs.then_some(&mut unload as &mut dyn FnMut(&[String]) -> Result<()>), texts: Some(Arc::clone(&vocabulary)), ..Default::default() };
                            let got = super::run_atoms_native_hooked(part, &db, "main", stats, keep, modes, hooks)?;
                            let (rows_read, rels_read) = kept.admitted();
                            if std::env::var_os("RERAIL_STATS_KEEP_TRACE").is_some() {
                                eprintln!("rerail: kept rows read where reached: {rels_read} relations ({rows_read} rows); {} relations left stored, unread", got.unread.len());
                            }
                            crate::code_rule_dsl::logic::profile::decision("the kept part, from the frames, where reached (decision 0117 step 4)", || {
                                format!("{rels_read} relations' rows read ({rows_read} rows) for the strata reached; {} relations left stored, unread", got.unread.len())
                            });
                            let (reached, quiet, of) = driver.walked;
                            (maintained.maintained, maintained.derived_again, maintained.recursive) = driver.counts;
                            maintained.again = std::mem::take(&mut driver.again);
                            crate::code_rule_dsl::logic::profile::decision("maintenance driven by deltas (decision 0117 step 3)", || {
                                format!("{reached} of {of} strata reached by a changed read: {} maintained ({} recursive), {} derived again; {quiet} not reached, held as kept", driver.counts.0, driver.counts.2, driver.counts.1)
                            });
                            step(&format!("driven: {reached} of {of} strata reached, {} maintained, {} derived again, {quiet} quiet", driver.counts.0, driver.counts.1));
                            got
                        }
                        None if hand_off => {
                            let hooks = crate::code_rule_dsl::logic::plan::Hooks { hand_off: Some(&mut hand), unload: let_go_inputs.then_some(&mut unload as &mut dyn FnMut(&[String]) -> Result<()>), texts: Some(Arc::clone(&vocabulary)), ..Default::default() };
                            super::run_atoms_native_hooked(part, &db, "main", stats, keep, modes, hooks)?
                        }
                        None => texts_run(part, &db, stats, keep, modes, &vocabulary)?,
                    };
                    drop(s);
                    let handed = handed.into_inner();
                    let handed_any = !handed.is_empty() || !unloaded.is_empty();
                    cross_handed += handed.len();
                    cross_unloaded += unloaded.len();
                    if hand_off {
                        crate::code_rule_dsl::logic::profile::decision("derived relations handed off (Q74)", || format!("{} relations' entries put once no later stratum read them, their rows let go", handed.len()));
                        crate::code_rule_dsl::logic::profile::decision("inputs let go (Q74)", || format!("{} inputs' rows and indexes let go after their last reader, their entries put before the run: {}", unloaded.len(), unloaded.join(", ")));
                    }
                    cross_atoms_ahead(store);
                    let Binder { constants, mut measured_kept, bound_old, lazies, .. } = binder;
                    let mut got = got;
                    // The relations whose rows are as they were (decision 0117 step 4): their frames are not written.
                    let unchanged_rels = std::mem::take(&mut got.unchanged);
                    let run_deltas = std::mem::take(&mut got.deltas);
                    // The reached strata the maintain mode did not take, by reason (decision 0364, R3).
                    let run_again = std::mem::take(&mut got.again);
                    let run_quiet = got.quiet;
                    step("ran");
                    collect(&db, &got, derived_widths_in(&fields))?;
                    step("collected");
                    if spike {
                        for (rel, why) in &maintained.again {
                            eprintln!("O125 derived-again {rel}: {why}");
                        }
                        // Each maintained relation's rows as its stratum's form left them (`Outcome::spike_forms`; the forms are
                        // made per stratum when reached, decision 0117 step 3, so the part's program has none of their
                        // deltas). A relation whose form changed nothing (no row gone, none new) moves no reader: an
                        // early cutoff's.
                        let forms = std::mem::take(&mut got.spike_forms);
                        let unchanged = forms.iter().filter(|(_, c)| c[3] + c[4] == 0).count();
                        let (mut t, mut lines) = ([0usize; 6], Vec::new());
                        for (rel, c) in &forms {
                            for k in 0..6 {
                                t[k] += c[k];
                            }
                            lines.push((c[1], format!("{rel}: old {} new {} overdeleted {} (gone {}, so {} put back) inserted {} (new {})", c[0], c[5], c[1], c[3], c[1].saturating_sub(c[3]), c[2], c[4])));
                        }
                        lines.sort_by(|a, b| b.0.cmp(&a.0));
                        for (_, l) in lines.iter().take(25) {
                            eprintln!("O125 {l}");
                        }
                        let reached = std::mem::take(&mut got.spike_reached);
                        let mut by_read: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
                        let mut quiet_in = 0usize;
                        for (_, changed, ins) in &reached {
                            if ins.iter().all(|(_, n)| *n == 0) {
                                quiet_in += 1;
                            }
                            for (r, n) in ins {
                                let e = by_read.entry(r.clone()).or_default();
                                e.0 += 1;
                                if !changed { e.1 += 1; }
                                if *n == usize::MAX { e.2 += 1; }
                            }
                        }
                        let mut top: Vec<_> = by_read.into_iter().collect();
                        top.sort_by(|a, b| b.1.1.cmp(&a.1.1));
                        eprintln!("O125 reached: {} strata, {} changed nothing, {quiet_in} with every changed read's delta empty", reached.len(), reached.iter().filter(|r| !r.1).count());
                        for (r, (n, quiet, unheld)) in top.iter().take(15) {
                            eprintln!("O125 reached by {r}: {n} strata, {quiet} of them changed nothing, {unheld} with its delta not held");
                        }
                        for (k, _, ins) in reached.iter().filter(|r| !r.1).take(12) {
                            eprintln!("O125 changed nothing: {} <- {ins:?}", if k.len() > 60 { &k[..60] } else { k });
                        }
                        eprintln!("O125 cutoff: {} relations maintained, {unchanged} changed nothing (no row gone, none new)", forms.len());
                        eprintln!("O125 total: overdeleted {}, gone {}, put back {}, inserted {}, new {}; strata {} of {} run, {} maintained ({} recursive), {} derived again", t[1], t[3], t[1].saturating_sub(t[3]), t[2], t[4], cut.strata.0, cut.strata.1, maintained.maintained, maintained.recursive, maintained.derived_again);
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
                        .filter(|rel| !got.inlined.contains(rel.as_str()) && !got.unread.contains(rel.as_str()) && !handed.contains_key(rel.as_str()))
                        .map(|rel| (rel, fields.width(rel)))
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
                                    let types = retain::types_of(&placed, &part.domains, rel);
                                    crate::code_rule_dsl::logic::native::component::InRel::flat(&held_rows(&db, &got, rel, width)?, types)
                                }
                            };
                            Ok((rel.clone(), width, held))
                        })
                        .collect::<Result<Vec<_>>>()?;
                    // A warm run's deltas are empty (`RERAIL_WARM`): an input's or stratum's rows gone or new, a maintained
                    // relation's overdeleted or inserted rows. Read from memory alone, so the check changes nothing a run
                    // does: the outcome's net deltas, and the delta relations the engine holds. A delta relation a SQL
                    // stratum left in a table is not read (a statement inside the waves, which the strict mode refuses).
                    if crate::code_rule_dsl::logic::profile::warm() {
                        for (rel, d) in &got.deltas {
                            let (gone, added) = d.counts(1);
                            if gone + added > 0 {
                                crate::code_rule_dsl::logic::profile::warm_fault("a non-empty delta", || format!("{rel}: {gone} gone, {added} added"));
                            }
                        }
                        let derived_here: BTreeSet<&str> = placed.v.rules().map(|r| placed.v.rel_name(r.head().rel())).collect();
                        for (rel, cols) in fields.decls_by_name() {
                            if cols.is_empty() || !derived_here.contains(rel) || !["__minus", "__plus", "__del", "__ins", "__gone"].iter().any(|x| rel.ends_with(x)) {
                                continue;
                            }
                            let n = crate::code_rule_dsl::logic::native::component::derived(&db, rel).map_or(0, |h| h.len());
                            if n > 0 {
                                crate::code_rule_dsl::logic::profile::warm_fault("a non-empty delta", || format!("{rel}: {n} rows"));
                            }
                        }
                    }
                    let derived_outputs: BTreeMap<String, (Arc<output::Rows>, Vec<usize>)> = got.outputs.iter().map(|(rel, rows)| (rel.clone(), (Arc::clone(rows), got.nodes[rel].clone()))).collect();
                    // An output whose file stands is not read back (`unchanged_outputs`).
                    let read_back: Vec<String> = cross_f.outputs().filter(|o| !unchanged_outputs.contains_key(*o)).map(str::to_owned).collect();
                    for rel in unchanged_outputs.keys() {
                        outputs.entry(rel.clone()).or_default();
                    }
                    let mut clean_outputs = cell.as_ref().map(|kept| cut.outputs(kept, &read_back)).unwrap_or_default();
                    // A clean output read back from the store has its rows, not its lines: decoded from the rows.
                    if let Some(kept) = cell.as_ref() {
                        let rows: Vec<(String, Arc<crate::code_rule_dsl::logic::native::component::InRel>)> = read_back
                            .iter()
                            .filter(|o| cut.clean.contains(*o) && !clean_outputs.iter().any(|(r, _, _)| r == *o))
                            .filter_map(|o| kept.kept(o).map(|(h, _)| (o.clone(), h)))
                            .collect();
                        if !rows.is_empty() {
                            let (decoded, nodes) = super::decode_kept(&db, &cross, rows)?;
                            clean_outputs.extend(decoded.into_iter().map(|(rel, r)| {
                                let n = nodes.get(&rel).cloned().unwrap_or_default();
                                (rel, r, n)
                            }));
                        }
                    }
                    native.extend(gather(got, &mut outputs, &mut node_columns, &outputs_of).into_iter().map(|k| format!("cross:{k}")));
                    for (rel, rows, nodes) in clean_outputs {
                        if outputs_of(&rel) {
                            outputs.entry(rel.clone()).or_default().push(rows);
                            node_columns.insert(rel, nodes);
                        }
                    }
                    // `RERAIL_CROSS_VERIFY`: derive the run's strata again (the cut's program, no maintenance) and name the
                    // first relations, in stratum order, whose maintained rows differ (spike O125).
                    if let Some(vstats) = verify_stats.filter(|_| maintained.maintained > 0) {
                        let plain = cut.program(&cross)?;
                        // `O125_VERIFY_RULES=<rel>`: the rules for `rel` the verifier derives with, and the maintained ones.
                        if let Ok(r) = std::env::var("O125_VERIFY_RULES") {
                            for (name, v) in [("plain", &plain.version), ("maintained", &placed.v)] {
                                for x in std::iter::once(v).flat_map(|v| v.rules().filter(|x| v.rel_name(x.head().rel()) == r).map(|x| v.show_rule(x))) {
                                    eprintln!("O125_VERIFY_RULES {name} {x}");
                                }
                            }
                        }
                        let mut vkeep = wanted.clone();
                        let plain_placed = plain.placed()?;
                        let plain_f = plain_placed.fields()?;
                        vkeep.extend(derived.iter().filter(|rel| !plain_f.is_output(rel)).cloned());
                        let again = texts_run(&plain, &db, vstats, vkeep, modes, &vocabulary)?;
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
                                let (a, b): (Vec<_>, Vec<_>) = (kept_rows.iter().filter(|r| hit(r)).collect(), fresh.iter().filter(|r| hit(r)).collect());
                                if !a.is_empty() || !b.is_empty() {
                                    eprintln!("O125_TRACE {rel}: maintained {:?} fresh {:?}", &a[..a.len().min(4)], &b[..b.len().min(4)]);
                                }
                            }
                            for (rel, src) in &cross_inputs {
                                let w = src.columns().len();
                                let now: Vec<Vec<i64>> = (0..src.len()).map(|i| (0..w).map(|c| src.value(i, c)).collect::<Vec<i64>>()).filter(|r| hit(r)).take(4).collect();
                                if !now.is_empty() {
                                    eprintln!("O125_TRACE input {rel}: now {now:?}");
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
                    let (patched, made, lazy_made) = kept_statistics(lazies, bound_old, &mut measured_kept, &rows, &input_stats, &constants);
                    step(&format!("kept statistics: {made} of {lazy_made} lazy relations planned with"));
                    drop(s);
                    let s = span("retain");
                    let mut next = retain::after(cell.take(), ckey, digests.clone(), &cut, rows, &derived_outputs, &cross_inputs, measured_kept, input_stats, input_docs, (run_docs_held.lock().expect("the run's documents").clone(), memo_whole), patched, retain::budget(), false);
                    if let Some(was) = &was_kept {
                        let written = next.write_kept(store, was, &cross_placed, &cross.domains, &split.locations, &cross_sccs, &cut, &cross_inputs, &digests, &unchanged_rels, &run_deltas, handed, inputs_done, &semijoin_stats, &digest_only)?;
                        // The base relations' documents, as this run read them (decision 0458): the next run's change.
                        // Under the key the next run's compare reads them by (`base_binds`): none where it reads none.
                        if let (false, Some(binds)) = (digest_only.is_empty(), base_binds) {
                            let mut table: Vec<retain::BaseDocument> = run_docs_held.lock().expect("the run's documents").iter().map(|(p, (c, k))| (p.clone(), *k, *c)).collect();
                            table.sort_by(|a, b| a.0.cmp(&b.0));
                            // Written with the digests it is the documents of: when one changed (or none was kept).
                            let unchanged = digest_only.iter().all(|r| was.input_digest(r).is_some() && was.input_digest(r) == digests.get(r).copied());
                            if !unchanged && !retain::put_base_documents(store, was, binds, &table, corpus_digest) {
                                crate::code_rule_dsl::logic::profile::decision("the base relations' documents (decision 0458)", || "not kept: a content held in memory alone, or a path with no atom; the next run reads its base relations' readers again".into());
                            }
                        }
                        // Their statistics, as this run counted or patched them.
                        if let Some(binds) = base_binds {
                            for (rel, digest, stats) in &base_stats_out {
                                retain::put_base_stats(store, was, binds, rel, *digest, stats);
                            }
                        }
                        // The corpus batch's documents' views' counts (decision 0464), when one was not kept.
                        if let (Some(binds), Some(batch)) = (base_binds, corpus_batch.as_ref().filter(|b| !b.lazy())) {
                            retain::put_base_counts(store, was, binds, &batch.kept_counts(), corpus_digest);
                        }
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
                    if std::env::var_os("RERAIL_COPIES").is_some() {
                        for (copy, rows, guests, shared, original) in next.copies() {
                            eprintln!("COPY {copy}: {rows} rows, {guests} with a guest's value, {shared} also in the original ({original} rows)");
                        }
                    }
                    if std::env::var_os("RERAIL_KEPT_BYTES").is_some() {
                        // Decision 0364's required shape, so far in this process (`maintain::shape`).
                        eprintln!("SHAPE {:?}", maintain::shape::now());
                        for (kind, rows, values, levels, index) in next.held() {
                            eprintln!("KEPT {kind}: {rows} rows, values {:.2} MB, levels {:.2} MB, sets and indexes {:.2} MB", values as f64 / 1e6, levels as f64 / 1e6, index as f64 / 1e6);
                        }
                        if crate::code_rule_dsl::logic::native::component::probed::on() {
                            eprintln!("{}", next.index_report(&crate::code_rule_dsl::logic::native::component::probed::take()));
                        }
                    }
                    // `RERAIL_SPIKE_PRESORTED`: the run's sorts, and the rows of them already in order (decision 0187).
                    if std::env::var_os("RERAIL_SPIKE_PRESORTED").is_some() {
                        eprintln!("presorted: {}", crate::code_rule_dsl::logic::native::setfix::runs::presorted::report());
                    }
                    // A session's run reports its strata here; a new process's, as `cross_kept` (above, or here when kept
                    // in the frames).
                    if kept_cells.is_some() {
                        cross_kept = Some(cut.strata);
                    } else if kept_part.is_none() {
                        cross_strata = Some(cut.strata);
                    }
                    cross_maintained = maintained.maintained;
                    cross_recursive = maintained.recursive;
                    cross_derived_again = maintained.derived_again;
                    cross_quiet = run_quiet;
                    cross_changed = cut.changed.clone();
                    cross_again = maintained.again.iter().fold(BTreeMap::new(), |mut m, (_, why)| {
                        *m.entry(*why).or_insert(0) += 1;
                        m
                    });
                    for (why, n) in run_again {
                        *cross_again.entry(why).or_insert(0) += n;
                    }
                    // Every program's kept part shares the one budget: past it, only this program's is kept.
                    if next.bytes() + cells.values().map(retain::Retained::bytes).sum::<usize>() > retain::budget() {
                        cells.clear();
                    }
                    if let Some(p) = kept_frames.take() {
                        kept_strata::keep(store, p, &next);
                    }
                    // Written: what is past the budget is not held for this process's next run. A part whose relations
                    // were handed off is not held at all: the next run reads it from the frames, as a new process does.
                    next.let_go_past();
                    if !handed_any {
                        cells.insert(binds, next);
                    }
                }
                None => {
                    let exported: BTreeSet<&String> = split.exports.iter().collect();
                    // Without a session, the strata are kept in the tape cache from run to run (decision 0077 §4).
                    let typed_by = cross.placed()?;
                    let mut kept_strata = (durable.is_some() && one_fixpoint).then(|| kept_strata::Kept::new(store, kept_strata::space(root), &cross_inputs, cross_inputs.iter().map(|(r, _)| (r.clone(), retain::types_of(&typed_by, &cross.domains, r))).collect()).typed(&typed_by, &cross.domains));
                    let loaded = kept_strata.as_ref().map(|k| std::rc::Rc::clone(&k.loaded));
                    let mut load = |rels: &[String]| -> Result<()> {
                        for rel in rels.iter().filter(|r| exported.contains(r)) {
                            let src = exports.borrow_mut().source(rel, pf.width(rel));
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
                    let hooks = if one_fixpoint { super::Hooks { load: Some(&mut load), unload: Some(&mut unload), strata, texts: Some(Arc::clone(&vocabulary)), ..Default::default() } } else { super::Hooks { texts: Some(Arc::clone(&vocabulary)), ..Default::default() } };
                    let got = super::run_atoms_native_hooked(&cross, &db, "main", cross_stats, wanted.clone(), modes, hooks)?;
                    cross_atoms_ahead(store);
                    if let Some(k) = kept_strata {
                        let written = k.written_rows;
                        cross_kept = k.strata;
                        k.finish();
                        crate::code_rule_dsl::logic::profile::decision("cross-file strata kept across runs (decision 0077 §4)", || format!("{written} rows of the relations whose rows changed written to the store"));
                    }
                    {
                        let placed = cross.placed()?;
                        collect(&db, &got, derived_widths_in(&placed.fields()?))?;
                    }
                    if wanted.contains("returns") && got.kept.contains("returns") {
                        returns = held_rows(&db, &got, "returns", 2)?;
                    }
                    native.extend(gather(got, &mut outputs, &mut node_columns, &outputs_of).into_iter().map(|k| format!("cross:{k}")));
                }
            }
        }
    }
    phase("cross", statements.count());
    drop(waves.take());
    let mut cross_counts = (0, 0);
    if let (Some(plan), Some((components, found))) = (&cross_plan, &cross_state) {
        cross_counts = (found.hits.len(), found.misses.len());
        let _storing = crate::code_rule_dsl::logic::profile::span(|| "store import components' rows".into());
        let cross_placed = cross.placed()?;
        let cross_f = cross_placed.fields()?;
        let nodes: BTreeMap<String, Vec<usize>> = plan
            .memo
            .keys()
            .map(|rel| (rel.clone(), cross_f.types(rel).iter().enumerate().filter(|(_, t)| **t == "node").map(|(i, _)| i).collect()))
            .collect();
        let recorded = cross_memo::record(&store, plan, components, found, cross_mode, &memo_rows, &nodes, &paths)?;
        crate::code_rule_dsl::logic::profile::decision("import components stored (decision 0076)", || {
            format!("{} components' rows stored ({} rows); {} found components verified", recorded.stored, recorded.rows, recorded.verified)
        });
        run_sources.extend(found.sources.iter().copied().chain(recorded.sources.iter().copied()));
    }
    if let Some((relations, reading)) = publish_later {
        let _s = crate::code_rule_dsl::logic::profile::span(|| "record the run in the store".into());
        collecting = record_run(store, root, &run_sources, &memo_program, &program_plans.iter().chain(&nested_plans).chain(part_keys.lock().expect("the parts' keys").iter()).chain(kept_runs.lock().expect("the kept runs").iter()).copied().collect::<Vec<_>>(), Some(lane), policy, &relations, &late_sources, reading, !phases::in_phase() && !phase0::active() && options.get(constants::OWN).is_none())?;
    }
    // The cross part's writes are one wave: its kept strata's frames, retained rows, rounds and statistics.
    crate::content_store::write_back::end_wave(crate::content_store::write_back::Boundary::Cross);
    statements.enter("write");
    let writing = crate::code_rule_dsl::logic::profile::span(|| "write the outputs".into());
    // Every output has its file, empty when nothing made a row: a run read whole from the memo hands no rows of a
    // file-local output no file made (`rr_embed_req`), which an evaluated run writes empty (O109).
    for rel in &head_outputs {
        if !outputs.contains_key(rel) && let Some(cols) = head.columns_of(rel) {
            outputs.insert(rel.clone(), Vec::new());
            node_columns.entry(rel.clone()).or_insert_with(|| cols.iter().enumerate().filter(|(_, (_, t))| t == "node").map(|(i, _)| i).collect());
        }
    }
    let mut rows = BTreeMap::new();
    // An output with no rows is written empty: a run whose files all came from the memo replays only the lines
    // there are.
    for rel in &head_outputs {
        outputs.entry(rel.clone()).or_default();
        node_columns.entry(rel.clone()).or_default();
    }
    let templates: std::cell::OnceCell<HashMap<(String, String), String>> = std::cell::OnceCell::new();
    // A node column's values (none where a part holds the column as text: rows a phase handed of its texts).
    let node_values = |part: &output::Rows, c: usize| -> Vec<i64> {
        (0..part.len())
            .filter_map(|i| match part.cell(i, c) {
                output::Cell::Int(v) if v != output::NULL => Some(v),
                _ => None,
            })
            .collect()
    };
    // A node column's value that is no node: `unset` (-1) is written as it is, and a `@place` (`rr::lower`'s
    // `PLACE`, `-2 - (path << 32 | line)`) as `@place("path", line)`.
    let mut places: HashMap<i64, (String, i64)> = HashMap::new();
    for (rel, parts) in &outputs {
        for part in parts {
            for &c in node_columns[rel].iter().filter(|&&c| c < part.width()) {
                for v in node_values(part, c) {
                    if v < -1 && !places.contains_key(&v) {
                        let n = -2 - v;
                        let path: String = db.query_row("SELECT sym_text(?1)", [n >> 32], |r| r.get(0))?;
                        places.insert(v, (path, n & 0xffff_ffff));
                    }
                }
            }
        }
    }
    // The outputs as one flat image (decision 0239), the file every reader reads (`output::lines`, `output::rows`).
    // The outputs this run made are an image in memory, from the runs' rows as they are. Over the session's stack,
    // only what changed is written: an overlay of each output's rows gone and added, an output whole where most of
    // it changed, nothing where none did. Otherwise, or at the stack's depth, a base of them all.
    {
        let flags = if own { output::flags::OWN } else { 0 } | if syntax::definition_ids() { output::flags::DEFINITION_IDS } else { 0 };
        // The session's stack here as its last run left it, if it stands (a stat, not a read).
        let held: Option<session::Stack> = session.as_ref().and_then(|s| s.stacks.lock().expect("the session's stacks").get(out).filter(|k| k.stands(out)).cloned());
        let mut image = output::Builder::new(flags, false);
        let mut made: Vec<&str> = Vec::new();
        for (rel, parts) in &outputs {
            if let Some(n) = unchanged_outputs.get(rel) {
                rows.insert(rel.clone(), *n);
                continue;
            }
            let node_columns = &node_columns[rel];
            // A row about a loaded document (its first node is one) is not the corpus's; a corpus row may name a
            // loaded document's node (a call's target in a stub).
            let filtered: Vec<Arc<output::Rows>>;
            let parts: &[Arc<output::Rows>] = match node_columns.first() {
                Some(&c) if !loaded_keys.is_empty() => {
                    let loaded = |v: i64| v >= 0 && loaded_keys.contains(&(v >> 32));
                    filtered = parts
                        .iter()
                        .map(|p| {
                            if c >= p.width() || p.types()[c] == output::col::TEXT {
                                return Arc::clone(p);
                            }
                            let mut kept = (**p).clone();
                            kept.retain(|_, row| !loaded(row[c]));
                            Arc::new(kept)
                        })
                        .collect();
                    &filtered
                }
                _ => parts,
            };
            // A finding's message from its template and its holes' values (decision 0225 §3, `findings::HOLE`): the
            // rule leaves `message` empty and its holes last; the row is written with its message and without them.
            let messaged: Vec<Arc<output::Rows>>;
            let parts: &[Arc<output::Rows>] = if crate::code_rule_dsl::logic::rr::is_diagnostic(rel) {
                let templates = templates.get_or_init(|| message_templates(programs, &head_settings));
                let holes = head.columns_of(rel).map_or(0, |d| d.iter().filter(|(c, _)| c.starts_with(crate::code_rule_dsl::logic::rr::lower::findings::HOLE)).count());
                messaged = parts.iter().map(|p| Arc::new(compose_messages(p, holes, templates))).collect();
                &messaged
            } else {
                parts
            };
            // The documents and places its node columns name.
            for part in parts {
                for &c in node_columns.iter().filter(|&&c| c < part.width()) {
                    for v in node_values(part, c) {
                        if v < -1 {
                            if let Some((path, line)) = places.get(&v) {
                                image.place(v, path, *line);
                            }
                        } else if v >= 0 {
                            let d = v >> 32;
                            if paths.contains_key(&d) || first.contains_key(&d) {
                                image.doc(d, &output::Doc { path: paths.get(&d).cloned(), first: first.get(&d).copied() });
                            }
                        }
                    }
                }
            }
            let parts: Vec<&output::Rows> = parts.iter().map(|p| &**p).collect();
            image.put(rel, &parts, node_columns);
            made.push(rel);
        }
        let bytes = image.finish();
        let made_image = output::Image::one(&bytes).map_err(Error::Plan)?;
        let made_stack = std::slice::from_ref(&made_image);
        for rel in &made {
            rows.insert((*rel).to_owned(), made_image.rel(rel).expect("made").rows);
        }
        // Each made output's rows' hashes, per row and as a set (a session's runs only).
        let hashed: BTreeMap<&str, (Vec<u128>, Vec<u128>)> = if session.is_some() {
            made.iter()
                .map(|&rel| {
                    let r = made_image.rel(rel).expect("made");
                    let each = r.each_hash();
                    let mut set = each.clone();
                    set.sort_unstable();
                    set.dedup();
                    (rel, (each, set))
                })
                .collect()
        } else {
            BTreeMap::new()
        };
        let total: usize = hashed.values().map(|(_, set)| set.len()).sum::<usize>()
            + held.as_ref().map_or(0, |h| unchanged_outputs.keys().filter_map(|r| h.rels.get(r)).map(|set| set.len()).sum());
        let (mut overlay, mut changed) = (output::Builder::new(flags, true), 0usize);
        let (mut whole, mut partly) = (0usize, 0usize);
        let over = held.as_ref().filter(|h| h.depth < output::MAX_DEPTH).filter(|h| h.deltas * 8 <= total.max(8)).map(|h| {
            for &rel in &made {
                let (each, set) = &hashed[rel];
                let r = made_image.rel(rel).expect("made");
                match h.rels.get(rel) {
                    Some(old) if **old == *set => {}
                    Some(old) => {
                        let gone: Vec<u128> = old.iter().copied().filter(|x| set.binary_search(x).is_err()).collect();
                        let added: Vec<usize> = (0..r.rows).filter(|&i| old.binary_search(&each[i]).is_err()).collect();
                        if (gone.len() + added.len()) * 2 >= set.len().max(1) {
                            overlay.take(rel, &output::find(made_stack, rel).expect("made"));
                            changed += set.len();
                            whole += 1;
                        } else {
                            overlay.added(rel, r, &added);
                            overlay.gone(rel, &gone);
                            changed += gone.len() + added.len();
                            partly += 1;
                        }
                    }
                    None => {
                        overlay.take(rel, &output::find(made_stack, rel).expect("made"));
                        changed += set.len();
                        whole += 1;
                    }
                }
            }
            h.clone()
        });
        let mut wrote = 0usize;
        let (depth, deltas, base) = match &over {
            Some(h) if overlay.is_empty() => (h.depth, h.deltas, false),
            Some(h) => {
                let image = overlay.finish();
                wrote = image.len();
                output::write(out, &image).map_err(|e| Error::Plan(e.to_string()))?;
                (h.depth + 1, h.deltas + changed, false)
            }
            None if unchanged_outputs.is_empty() => {
                output::write(out, &bytes).map_err(|e| Error::Plan(e.to_string()))?;
                (1, 0, true)
            }
            None => {
                // The stack folded into one base: the made outputs, and the unchanged ones as the stack reads them.
                let old = output::read(out).map_err(Error::Plan)?;
                let mut folded = output::Builder::new(flags, false);
                for &rel in &made {
                    folded.take(rel, &output::find(made_stack, rel).expect("made"));
                }
                for rel in unchanged_outputs.keys() {
                    let view = output::find(&old, rel).ok_or_else(|| Error::Plan(format!("{rel}: unchanged, but not in {}", out.display())))?;
                    folded.take(rel, &view);
                }
                output::write(out, &folded.finish()).map_err(|e| Error::Plan(e.to_string()))?;
                (1, 0, true)
            }
        };
        if over.is_some() {
            crate::code_rule_dsl::logic::profile::decision("the outputs' overlay (0239 stage 4)", || {
                format!("{} of {} outputs unchanged, {whole} written whole, {partly} by their rows gone and added ({changed} rows); {wrote} bytes written, where the outputs whole are {}; depth {depth}", made.len() - whole - partly, made.len(), bytes.len())
            });
        }
        // What a session's run of this program wrote, for its next (`unchanged_outputs`); any other run's writes
        // forget it. A base holds no output an earlier run wrote but this one did not make or keep.
        if let Some(s) = session.as_ref() {
            let mut written = s.written.lock().expect("the session's written outputs");
            if base {
                written.retain(|p, _| p.parent() != Some(out));
            }
            for (rel, n) in &rows {
                match written_key {
                    Some(key) => written.insert(out.join(rel), (key, *n)),
                    None => written.remove(&out.join(rel)),
                };
            }
            let mut stacks = s.stacks.lock().expect("the session's stacks");
            match session::Stack::stat(out) {
                Some(stat) => {
                    // A base holds the made outputs and the unchanged ones; an overlay leaves the rest as they read.
                    let mut rels: HashMap<String, Arc<Vec<u128>>> = match held {
                        Some(h) if base => unchanged_outputs.keys().filter_map(|r| h.rels.get(r).map(|set| (r.clone(), Arc::clone(set)))).collect(),
                        Some(h) => h.rels,
                        None => HashMap::new(),
                    };
                    for (rel, (_, set)) in hashed {
                        rels.insert(rel.to_owned(), Arc::new(set));
                    }
                    stacks.insert(out.to_path_buf(), session::Stack { depth, deltas, rels, ..stat })
                }
                None => stacks.remove(out),
            };
        }
    }
    if head.declares("declared_ret") {
        let fn_q = split.exports.iter().any(|e| e == "fn_q").then(|| exports.borrow_mut().source("fn_q", 2));
        // `type_of`'s rows, its parts as the runs handed them.
        let type_of: Option<&[Arc<output::Rows>]> = outputs.get("type_of").map(Vec::as_slice);
        typing::outputs(typing, &db, &store, root, type_of, &returns, &declared, fn_q.as_deref(), &mut typed)?;
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
    // None pending is nothing to write: no job (a warm run makes no atom, decision 0211).
    if store.atoms_handle().pending() {
        store.queue_atoms("the run's atoms");
    }
    let ((segments, atoms, bytes), pending) = store.atom_writes();
    crate::code_rule_dsl::logic::profile::decision("atoms", || {
        format!("{atoms} atoms appended to the store's blocks in {segments} commits ({:.1} MB of strings), {pending} pending at the end (decisions 0049, 0401)", bytes as f64 / 1e6)
    });
    if memoised {
        crate::code_rule_dsl::logic::profile::decision("per-file memo use", || format!("{}: {memo_hits} of {} files read from the memo; {memo_unattributed} chunks' rows not memoised (a row naming no document of its chunk){}", phases::current().as_deref().unwrap_or("the program"), docs.len(), if memo_why.is_empty() { String::new() } else { format!(": {}", memo_why.iter().take(12).cloned().collect::<Vec<_>>().join("; ")) }));
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
    // (decision 0102). A phase's run (phase 1's, inside the run) leaves its writes queued: the store's one writer
    // writes them behind the outer run, whose end waits for them and reports the first that failed (decision 0240).
    // Nothing the outer run reads waits for them: a queued write is read from memory until it is written.
    if durable.is_some() {
        let waited = crate::code_rule_dsl::logic::profile::span(|| "the store's writer, the run's last writes".into());
        // The run's atoms still pending, appended by the writer with its last writes: none pending, no job (a warm
        // run makes no atom, decision 0211). Forked, none is wanted (`flush_wanted`).
        if store.atoms_handle().flush_wanted() {
            store.queue_atoms_flush();
        }
        let drain = std::time::Instant::now();
        let outcome = if phases::in_phase() { Ok(()) } else { store.written() };
        if std::env::var_os("RERAIL_WRITES").is_some() {
            eprintln!("RERAIL_WRITES drain {:.3} s ({})", drain.elapsed().as_secs_f64(), phases::current().unwrap_or_else(|| "main".to_owned()));
        }
        drop(waited);
        let w = store.written_so_far();
        let (by_what, peak, held) = store.written_by_what();
        let dropped = store.dropped();
        crate::code_rule_dsl::logic::profile::decision("write-back by what (decision 0102)", || {
            // Held: what a job keeps queued until written (the queue's cap); encoded: the frames it made.
            let rows: Vec<String> = by_what.iter().map(|(what, jobs, bytes, cpu, encoded)| format!("{what}: {jobs} jobs, {:.1} MB held, {:.1} MB encoded, {cpu:.3} s", *bytes as f64 / 1048576.0, *encoded as f64 / 1048576.0)).collect();
            format!(
                "queue peak {:.1} MB; back-pressure held producers {} times, {:.3} s; cache writes dropped for a full queue {} ({:.1} MB: the next run parses them again); {}",
                peak as f64 / 1048576.0, held.0, held.1, dropped.0, dropped.1 as f64 / 1048576.0, rows.join("; ")
            )
        });
        let encoded: usize = by_what.iter().map(|r| r.4).sum();
        crate::code_rule_dsl::logic::profile::decision("write-back (decision 0102)", || format!("{} writes queued behind the run in {} transactions, {:.1} MB held, {:.1} MB of frames encoded, {:.3} s of the writer's CPU", w.jobs, w.batches, w.bytes as f64 / 1048576.0, encoded as f64 / 1048576.0, w.cpu));
        // By kind, over the process so far.
        let kinds = crate::content_store::write_back::written_by_kind();
        let text = || kinds.iter().map(|(k, n, b, c, e)| format!("{k}: {n} jobs, {:.1} MB held, {:.1} MB encoded, {c:.3} s", *b as f64 / 1048576.0, *e as f64 / 1048576.0)).collect::<Vec<_>>().join("; ");
        crate::code_rule_dsl::logic::profile::decision("write-back by kind, this process so far (decision 0102)", text);
        if std::env::var_os("RERAIL_WRITES").is_some() {
            eprintln!("RERAIL_WRITES queue peak {:.1} MB, producers held {} times ({:.3} s); {}", peak as f64 / 1048576.0, held.0, held.1, text());
        }
        outcome.map_err(|e| Error::Plan(e.to_string()))?;
    }
    store.atoms_handle().run_over();
    crate::code_rule_dsl::logic::profile::decision("chunks", || format!("{chunks} chunks of about {chunk_nodes} nodes; the next chunk is parsed and batched on a worker while this one runs"));
    process_phases.push(("finalize", process_cpu() - process_last));
    crate::code_rule_dsl::logic::profile::decision("run process CPU by phase", || {
        let mut totals: BTreeMap<&str, f64> = BTreeMap::new();
        for &(name, cpu) in &process_phases { *totals.entry(name).or_default() += cpu; }
        serde_json::json!({"root":root,"phases":totals}).to_string()
    });
    if profiling && let Some(dir) = crate::code_rule_dsl::logic::profile::directory().map(|d| if phase0::active() { d.join("phase-0") } else { d }) {
        let mut header = format!(
            "Profile of a corpus run: {} files, {nodes} nodes, {chunks} chunks; text tests {} relations, {tested} strings, {text_cpu:.3} s CPU\n\n== Phases (CPU = user + system; worker phases run beside the main thread)\n",
            docs.len(),
            vocabulary.tests.len()
        );
        for (name, user, sys, rss, n) in &phases {
            header.push_str(&format!("{:>9.2} s  {name:<16} rss {rss:.0} MB  {n} statements\n", user + sys));
        }
        header.push_str(&crate::code_rule_dsl::logic::profile::run_thread_report(&phase_clock));
        let lines: usize = programs.iter().map(|p| p.lines().filter(|l| !l.trim().is_empty()).count()).sum();
        header.push_str(&format!("\nProgram: {} lines in {} programs; rules are numbered as in the `rule #k` notes at the end.\n", lines, programs.len()));
        crate::code_rule_dsl::logic::profile::report::write(&dir, &header).map_err(|e| Error::Plan(format!("profile: {e}")))?;
    }
    // The index, when `@git_tracked` read it: it moves between runs (`strings::git`).
    if t.is_none() {
        let _s = crate::code_rule_dsl::logic::profile::span(|| "the Git index consulted".into());
        consulted.extend(crate::code_rule_dsl::logic::strings::git::consulted(root));
    }
    crate::code_rule_dsl::logic::profile::decision("files' tapes found and parsed in the waves (Q22)", || format!("{} found, {} parsed", file_tapes.0.load(std::sync::atomic::Ordering::Relaxed), file_tapes.1.load(std::sync::atomic::Ordering::Relaxed)));
    if tracking.get() {
        crate::code_rule_dsl::logic::profile::untrack_run_thread();
    }
    if !nested && !phase0::active() {
        crate::code_rule_dsl::logic::profile::run_ends(crate::code_rule_dsl::logic::profile::snapshot());
    }
    let report = Report { phases, phase_instructions, phase_clock, files: docs.len(), nodes, native: native.len(), rows, chunks, typing: typed, text: (vocabulary.tests.len(), tested, text_cpu), phase_runs, memo: (memo_hits, memo_unattributed), memo_partial: partial_hits.load(std::sync::atomic::Ordering::Relaxed), memo_kept, stats_kept: stats_kept.into_inner(), cross: cross_counts, prepared: origin, cross_strata, cross_kept, cone_kept, guests_unseen: guests_unseen.load(std::sync::atomic::Ordering::Relaxed), file_tapes: (file_tapes.0.load(std::sync::atomic::Ordering::Relaxed), file_tapes.1.load(std::sync::atomic::Ordering::Relaxed)), cone_rounds, cross_maintained: (cross_maintained, cross_recursive, cross_derived_again), cross_quiet, language_asked, cross_handed, cross_unloaded, cross_changed, cross_again, cross_delta_counts, consulted, cone_inputs, not_per_document, files_observed: Default::default(), plans: std::iter::once(memo::plan_digest(&memo_program)).chain(program_plans.iter().copied()).chain(part_keys.into_inner().expect("the parts' keys")).chain(nested_plans).collect() };
    handed.succeeded();
    Ok(report)
}



/// What binding a session's kept relations and sources keeps for after its run: each kept relation bound as
/// `r__old` with the statistics it is planned with (decision 0094 §4, 0096 step 4, 0101 §4).
pub(in crate::code_rule_dsl) struct Binder<'k> {
    constants: HashMap<String, BTreeSet<(usize, i64)>>,
    cell: Option<&'k retain::Retained>,
    measured_kept: HashMap<(String, usize), retain::Measured>,
    bound_old: HashMap<String, (Arc<crate::code_rule_dsl::logic::native::component::InRel>, Option<retain::Patched>)>,
    lazy_kept: bool,
    lazies: Vec<(String, Arc<crate::code_rule_dsl::logic::native::component::InRel>, Arc<crate::code_rule_dsl::logic::plan::LazyStats<LazyKept>>)>,
    /// Each input of the cross part, and each relation that copies some of an input's columns (`projection_of`): the
    /// input, its columns, and the input's column each of the relation's copies. A new process's `r__old` of one has
    /// this run's statistics of those columns, never counted for it.
    copies: HashMap<String, (String, Vec<String>, Vec<usize>)>,
}

/// The driver's side of a session's maintained run (decision 0117 step 3, `plan::Maintain`): the kept rows a
/// reached stratum reads, read and bound here with their statistics, and what each stratum's run left of its delta
/// (decision 0364: the maintain mode, or derived again).
struct Driver<'d, 'k> {
    /// `cross`'s version: what the run reads of it.
    cross_placed: &'d super::body::Placed,
    kept: &'k retain::Retained,
    /// Where the kept part's frames are read from, when it was read from them (a session's); none when the caller
    /// holds the whole kept part in memory, its clean reads bound before the run (`maintained::run`).
    store: Option<&'d ContentStore>,
    /// The clean relations the part reads that are still stored (`need` binds them).
    clean_reads: BTreeSet<String>,
    /// What the caller reads after the run.
    at_end: BTreeSet<String>,
    binder: &'d mut Binder<'k>,
    modules: &'d Modules,
    db: &'d rusqlite::Connection,
    /// Relations whose deltas are relations of the run: given, or made by their stratum's run.
    has_delta: BTreeSet<String>,
    /// Relations that changed with no delta: derived again, their old rows not kept.
    no_delta: BTreeSet<String>,
    /// The same, shared with the run's hand-off (Q74): a relation derived again is written whole, so it can be
    /// handed off once no later stratum reads it.
    derived_again: &'d std::cell::RefCell<BTreeSet<String>>,
    /// Relations of the strata maintained so far.
    maintains: BTreeSet<String>,
    seeds: Vec<String>,
    /// The changed inputs' rows gone and added (`driven_part`).
    given: maintain::Given,
    /// Strata maintained, derived again, and of those maintained the recursive.
    counts: (usize, usize, usize),
    again: Vec<(String, &'static str)>,
    walked: (usize, usize, usize),
}


impl crate::code_rule_dsl::logic::plan::Maintain for Driver<'_, '_> {
    fn kept(&mut self, rel: &str) -> Option<Arc<crate::code_rule_dsl::logic::native::component::InRel>> {
        self.kept.kept(rel).map(|(h, _)| h)
    }

    fn keeps(&self, rel: &str) -> bool {
        self.kept.keeps(rel)
    }

    fn need(&mut self, rels: &[String], model: &mut crate::code_rule_dsl::logic::plan::Model) -> Result<()> {
        if let Some(store) = self.store
            && !self.kept.admit(store, self.cross_placed, rels)?
        {
            return Err(Error::Plan("the kept part's frames changed during the run".into()));
        }
        // A clean relation the part reads, bound as an input is, with its statistics.
        for rel in rels {
            if !self.clean_reads.remove(rel) {
                continue;
            }
            let Some((held, width)) = self.kept.kept(rel) else { continue };
            let source = held.source((0..width).map(|c| super::sql_col(rel, c)).collect());
            model.stats.lazy.0.remove(rel);
            bind_kept(self.binder, self.modules, self.db, &mut model.stats, rel.clone(), source, Some(held), false)?;
            if !model.stats.lazy.0.contains_key(rel) {
                let _ = super::input_stats(&mut model.stats, &mut model.ndv, self.cross_placed.fields()?.arity(rel), rel);
            }
        }
        Ok(())
    }

    fn at_end(&self, rel: &str) -> bool {
        self.at_end.contains(rel)
    }


    fn has_delta(&self, rel: &str) -> bool {
        self.has_delta.contains(rel)
    }

    fn no_delta(&mut self, rel: &str) {
        self.no_delta.insert(rel.to_owned());
        self.derived_again.borrow_mut().insert(rel.to_owned());
    }

    fn delta_made(&mut self, rel: &str) {
        self.has_delta.insert(rel.to_owned());
    }

    fn walked(&mut self, reached: usize, quiet: usize, of: usize) {
        self.walked = (reached, quiet, of);
    }

    fn seeds(&mut self) -> Vec<String> {
        std::mem::take(&mut self.seeds)
    }

    fn given(&self, rel: &str) -> Option<[Arc<dyn crate::code_rule_dsl::facts::vtab::Source>; 2]> {
        self.given.get(rel).cloned()
    }

    fn old_input(&self, rel: &str) -> Option<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>> {
        self.kept.old_source(rel, rel).filter(|(_, held)| held.is_none()).map(|(s, _)| s)
    }

    fn mode_ran(&mut self, scc: &BTreeSet<String>, recursive: bool) {
        self.has_delta.extend(scc.iter().cloned());
        self.maintains.extend(scc.iter().cloned());
        self.counts.0 += 1;
        if recursive {
            self.counts.2 += 1;
            maintain::by_levels_ran();
        }
    }
}

/// The statistics a maintained run kept of the relations it bound, as the next run binds them (`retain::after`'s
/// `measured` and `patched`): the lazy statistics a plan made, kept by delta or as measured, and each maintained
/// relation's patched by its delta (decision 0096 step 4); `rows` the run's derived relations. Returns how many of the
/// lazy statistics were made, too. Shared by the session's run and `maintained::run`.
#[allow(clippy::type_complexity)]
pub(in crate::code_rule_dsl) fn kept_statistics(
    lazies: Vec<(String, Arc<crate::code_rule_dsl::logic::native::component::InRel>, Arc<super::LazyStats<LazyKept>>)>,
    mut bound_old: HashMap<String, (Arc<crate::code_rule_dsl::logic::native::component::InRel>, Option<retain::Patched>)>,
    measured_kept: &mut HashMap<(String, usize), retain::Measured>,
    rows: &[(String, usize, Arc<crate::code_rule_dsl::logic::native::component::InRel>)],
    input_stats: &HashMap<String, retain::InputStats>,
    constants: &HashMap<String, BTreeSet<(usize, i64)>>,
) -> (HashMap<String, retain::Patched>, usize, usize) {
    let mut unmade: BTreeSet<String> = BTreeSet::new();
    let lazy_made = lazies.len();
    let mut made = 0;
    for (rel, h, lazy) in lazies {
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
    let mut patched: HashMap<String, retain::Patched> = HashMap::new();
    for (rel, width, new) in rows {
        let Some((old, kept)) = bound_old.get(rel) else { continue };
        // A copy of an input's columns is bound next run with the input's statistics (`projection_of`).
        if unmade.contains(rel) || input_stats.contains_key(rel) {
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
                let tested = tested_of(constants, &name).cloned().unwrap_or_default();
                let counts = kept_counts(&stats, &tested, new.source(stats.columns().to_vec()).as_ref());
                Some(retain::Patched { at: Arc::as_ptr(new) as *const u8 as usize, stats, counts, tested })
            }
            // Not counted here (decision 0123): the next run counts it from its postings when a round
            // requests it, and patches it by delta from then on. Before 0123 it was counted here, the
            // first run it was maintained, as a lazy statistic was measured (not counted) at every step
            // it was planned with (O125 steps 06-08: 2.5, 2.5 and 3.0 s so, against 5.6, 0.7 and 0.2 s
            // counted here).
            None => None,
        };
        if let Some(p) = next {
            patched.insert(name, p);
        }
    }
    (patched, made, lazy_made)
}

/// A driven maintained run's program (decision 0117 step 3): the cut's base program, each changed input's delta that
/// `answer` gives declared as its inputs `r__plus` and `r__minus` (bound from `sources`). The seeds are the inputs that
/// changed with a delta not empty, or with none; `given` those with one. Each stratum's form is made when the run
/// reaches it (`Driver::form`). Shared by the session's run and `maintained::run`.
pub(in crate::code_rule_dsl) fn driven_part(cut: &retain::Cut, cross: &lower::Lowered, answer: &mut dyn FnMut(&str) -> Option<(Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)>) -> Result<(maintain::Maintained, Vec<String>, BTreeSet<String>)> {
    let mut seeds: Vec<String> = Vec::new();
    let mut given_rels: BTreeSet<String> = BTreeSet::new();
    let part = cut.program(cross)?;
    // Each changed input's rows gone and added, handed to the maintain mode as its versions (0364 R1): not declared
    // as relations of the program, nor bound under names of their own.
    let mut given = maintain::Given::new();
    for rel in &cut.changed {
        let Some((plus, minus)) = answer(rel) else {
            seeds.push(rel.clone());
            continue;
        };
        if plus.len() + minus.len() > 0 {
            seeds.push(rel.clone());
        }
        given.insert(rel.clone(), [minus, plus]);
        given_rels.insert(rel.clone());
    }
    Ok((maintain::Maintained { program: part, given, maintained: 0, derived_again: 0, recursive: 0, again: Vec::new() }, seeds, given_rels))
}

/// The constants the rules test `rel` by: a relation's own, or for its old rows (`r__old`) its relation's.
fn tested_of<'c>(constants: &'c HashMap<String, BTreeSet<(usize, i64)>>, rel: &str) -> Option<&'c BTreeSet<(usize, i64)>> {
    constants.get(rel).or_else(|| constants.get(rel.strip_suffix("__old")?))
}

/// Binds `rel` (a kept relation, adopted when `held`, or a source) for a session's run, with its statistics in
/// `stats`; `maintains`: this run maintains the relation whose old rows `rel` is.
fn bind_kept(b: &mut Binder, modules: &Modules, db: &rusqlite::Connection, stats: &mut super::Stats, rel: String, source: Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, held: Option<Arc<crate::code_rule_dsl::logic::native::component::InRel>>, maintains: bool) -> Result<()> {
    maintain::shape::held(&rel);
        let _bind = crate::code_rule_dsl::logic::profile::span(|| format!("bind {}", if held.is_some() { "kept" } else { "source" }));
        // A kept relation's statistics, measured when it was first bound so, while it is kept as it is.
        let tested = tested_of(&b.constants, &rel).cloned().unwrap_or_default();
        // An input's old rows are its rows of the last run: their statistics are those that run planned with,
        // when they cover every constant the rules test it by (decision 0094 §4).
        // With none kept (a new process, which reads the kept part from the frames), an input's old rows, or a copy of
        // an input's columns (`projection_of`), have this run's statistics of the same columns: the two differ by the
        // documents that changed, not counted again for it.
        let carried: Option<retain::InputStats> = rel.strip_suffix("__old").and_then(|of| {
            if let Some(kept) = b.cell.and_then(|c| c.input_stats(of)) {
                return Some(kept.clone());
            }
            let (src, columns, map) = b.copies.get(of)?;
            stats.of_columns(src, columns, map)
        });
        // As many rows as the statistics say, within 1%: a copy whose rows collapsed is measured.
        let carried = carried.filter(|(cols, _)| cols.len() == source.columns().len() && cols.first().is_some_and(|c| (c[0] - source.len() as i64).abs() as f64 <= 0.01 * c[0].max(1) as f64));
        // A derived relation kept is read in place, with its indexes (decision 0090 step 3).
        if let Some(held) = &held {
            crate::code_rule_dsl::logic::native::component::adopt(db, &rel, Arc::clone(held));
        }
        if let Some((cols, vals)) = carried {
            // By the bound source's own column names, as the planner reads them. A constant the rules test that the run
            // did not count is counted here, by its column alone, not the relation measured.
            let names: Vec<String> = source.columns();
            let missing: BTreeSet<(usize, i64)> = tested.iter().filter(|(c, v)| !vals.iter().any(|(vc, vv, _)| vc == c && vv == v)).copied().collect();
            let extra = constant_counts(Some(&missing), source.as_ref());
            stats.measured(&rel, names.iter().cloned().zip(cols.iter().copied()), vals.iter().map(|(c, v, n)| (names[*c].clone(), *v, *n)).chain(extra));
        } else {
            let patched = held.as_ref().and_then(|h| b.cell.and_then(|k| k.patched(&rel, h)).filter(|p| p.tested == tested));
            // Kept, and known neither by delta nor as measured for these constants: its statistics are
            // made when a stratum first plans with it (decision 0101 §4), as the eager path below would
            // make them. Most strata a step reaches are quiet or join an empty delta and never do.
            let measured_before = held.as_ref().and_then(|h| b.cell.and_then(|k| k.measured(&rel, h)).filter(|m| m.2 == tested));
            // Decision 0123: made only when a planning round requests them, measured before or not.
            if patched.is_none() && b.lazy_kept
                && let Some(h) = &held
            {
                let maintains = maintains && rel.ends_with("__old");
                let (h2, source2, tested2, rel2) = (Arc::clone(h), Arc::clone(&source), tested.clone(), rel.clone());
                let wanted = tested_of(&b.constants, &rel).cloned();
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
                    // Counted in the form patched by delta: from its postings where every column has them, else
                    // from its rows (its indexes dropped once no later stratum read it), as the run that first
                    // maintained it counted it before decision 0123.
                    if maintains {
                        let names = source2.columns();
                        let stats = if h2.posts_all() {
                            let _c = crate::code_rule_dsl::logic::profile::span(|| format!("count {rel2}"));
                            super::statkeep::RelStats::from_postings(names.clone(), h2.rows() as i64, &mut |c, visit| h2.each_posting(c, visit))
                        } else {
                            let _c = crate::code_rule_dsl::logic::profile::span(|| format!("count {rel2} from its rows"));
                            h2.live_rows().map(|live| {
                                let mut stats = super::statkeep::RelStats::paired(names.clone());
                                stats.documents(&live, names.len(), 1);
                                stats
                            })
                        };
                        if let Some(stats) = stats {
                            let counts = constant_counts(Some(&tested2), source2.as_ref());
                            let p = retain::Patched { at: Arc::as_ptr(&h2) as *const u8 as usize, stats, counts: counts.clone(), tested: tested2.clone() };
                            return (as_cols(&p.stats.statistics(&|_| true)), counts, LazyKept::Patched(p));
                        }
                    }
                    // Measured by an earlier run of the same relation: as it was.
                    if let Some((measured, counts, tested)) = measured_before {
                        return (as_cols(&measured), counts.clone(), LazyKept::Measured(measured, counts, tested));
                    }
                    let _m = crate::code_rule_dsl::logic::profile::span(|| format!("measure {rel2}"));
                    let measured = syntax::statistics(source2.as_ref());
                    let counts = constant_counts(wanted.as_ref(), source2.as_ref());
                    (as_cols(&measured), counts.clone(), LazyKept::Measured(measured, counts, tested2))
                });
                stats.lazily(&rel, h.rows(), lazy.clone());
                if let Some(of) = rel.strip_suffix("__old") {
                    b.bound_old.insert(of.to_owned(), (Arc::clone(h), None));
                }
                b.lazies.push((rel.clone(), Arc::clone(h), lazy));
                modules.bind(db, &rel, source)?;
                return Ok(());
            }
            // Not kept by delta yet: counted from the engine's postings of it, in place of measuring it,
            // and patched by this run's delta (decision 0096 step 4): a relation's first count costs
            // about what a measure did, once.
            let patched = patched.or_else(|| {
                // Only a relation this run maintains (it has an overdelete): one derived again is measured.
                rel.strip_suffix("__old")?;
                let h = held.as_ref().filter(|_| maintains)?;
                let _c = crate::code_rule_dsl::logic::profile::span(|| format!("count {rel}"));
                let names = source.columns();
                // One the engine kept no index on (no later stratum read it) is measured: its count is
                // taken from this run's relation, made with its indexes.
                let stats = super::statkeep::RelStats::from_postings(names, h.rows() as i64, &mut |c, visit| h.each_posting(c, visit))?;
                let counts = constant_counts(Some(&tested), source.as_ref());
                Some(retain::Patched { at: Arc::as_ptr(h) as *const u8 as usize, stats, counts, tested: tested.clone() })
            });
            if let (Some(h), Some(of)) = (&held, rel.strip_suffix("__old")) {
                b.bound_old.insert(of.to_owned(), (Arc::clone(h), patched.clone()));
            }
            let known = match patched {
                Some(p) => Some((p.stats.statistics(&|_| true), p.counts, tested.clone())),
                None => held.as_ref().and_then(|h| b.cell.and_then(|k| k.measured(&rel, h)).filter(|m| m.2 == tested)),
            };
            let (measured, counts) = match known {
                Some((measured, counts, _)) => (measured, counts),
                None => {
                    let _m = crate::code_rule_dsl::logic::profile::span(|| format!("measure {rel}"));
                    let measured = syntax::statistics(source.as_ref());
                    // The constants the rules test it by are counted too: a column measured without them
                    // reads as holding none, so `kind__old(r, identifier)` was planned as one row and drove
                    // a delta rule.
                    let counts = constant_counts(tested_of(&b.constants, &rel), source.as_ref());
                    if let Some(h) = &held {
                        b.measured_kept.insert((rel.clone(), Arc::as_ptr(h) as *const u8 as usize), (measured.clone(), counts.clone(), tested));
                    }
                    (measured, counts)
                }
            };
            stats.measured(&rel, measured.iter().map(|(c, rows, ndv, sq, sqd)| (c.clone(), [*rows, *ndv, *sq, *sqd])), counts);
        }
        modules.bind(db, &rel, source)?;
    
    Ok(())
}


/// Per relation, the integer constants its atoms test, by column (`constant_counts`'s), read from version `v` by its
/// rules' handles: one pass over the rules, not one per relation. A relation that copies another's columns (a rule
/// of one positive atom) passes its tests on to the columns it copies (`tested_through`).
pub fn constants_tested_in(v: &crate::code_rule_dsl::logic::program_image::Version) -> HashMap<&str, BTreeSet<(usize, i64)>> {
    use crate::code_rule_dsl::logic::program_image::{Lit, TermRef};
    let mut by: HashMap<&str, BTreeSet<(usize, i64)>> = HashMap::new();
    let mut projections: HashMap<(&str, usize), Vec<(&str, usize)>> = HashMap::new();
    for r in v.rules() {
        for a in r.reads() {
            for (c, t) in a.args().enumerate() {
                if let Some(n) = match t { TermRef::Int(n) => Some(n), TermRef::Branded(b) => Some(b.value()), _ => None } {
                    by.entry(v.rel_name(a.rel())).or_default().insert((c, n));
                }
            }
        }
        let mut body = r.body();
        let (Some(only), None) = (body.next(), body.next()) else { continue };
        let Lit::Atom(a) = only.view() else { continue };
        if a.neg() { continue; }
        let head = r.head();
        for (hc, term) in head.args().enumerate() {
            let TermRef::Var(s) = term else { continue };
            for (bc, arg) in a.args().enumerate() {
                if arg.var() == Some(s) {
                    projections.entry((v.rel_name(head.rel()), hc)).or_default().push((v.rel_name(a.rel()), bc));
                }
            }
        }
    }
    tested_through(by, &projections)
}

/// The constants tested `by` relation, each also tested of the column a single-atom projection copies it from
/// (`projections`: a head column to the body columns it copies): a consumer may test a projection, while
/// statistics are collected on its source. A worklist also handles alias chains and cycles.
fn tested_through<'a>(mut by: HashMap<&'a str, BTreeSet<(usize, i64)>>, projections: &HashMap<(&'a str, usize), Vec<(&'a str, usize)>>) -> HashMap<&'a str, BTreeSet<(usize, i64)>> {
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

/// A relation the cross-file part makes as a copy of some columns of one input, or of such a copy
/// (`kind(n, k) :- node(_, n, k, _, _)`): that input, and
/// for each of the relation's columns the input's column it copies. When it holds as many rows as the input, its
/// rows are the input's one for one, so its statistics are the input's by column: kept with the input's, never
/// counted (a full count of `kind` and `docof` was 0.32 s of a one-file edit's 0.38 s of statistics).
/// `heads`: each relation's one rule (`None` when it has more), `rule_by_head`.
fn projection_of(v: &crate::code_rule_dsl::logic::program_image::Version, heads: &HashMap<&str, Option<crate::code_rule_dsl::logic::program_image::RuleRef<'_>>>, is_input: &dyn Fn(&str) -> bool, rel: &str) -> Option<(String, Vec<usize>)> {
    use crate::code_rule_dsl::logic::program_image::{Lit, TermRef};
    let (mut at, mut map): (String, Option<Vec<usize>>) = (rel.to_owned(), None);
    for _ in 0..8 {
        if is_input(&at) {
            return map.map(|m| (at, m));
        }
        let Some(Some(r)) = heads.get(at.as_str()) else { return None };
        let mut body = r.body();
        let (Some(l), None) = (body.next(), body.next()) else { return None };
        let Lit::Atom(a) = l.view() else { return None };
        if a.neg() {
            return None;
        }
        // Each variable once in the body (no equality between columns), and nothing tested.
        let mut col: HashMap<u32, usize> = HashMap::new();
        for (i, t) in a.args().enumerate() {
            match t {
                TermRef::Var(s) => {
                    if col.insert(s.0, i).is_some() {
                        return None;
                    }
                }
                TermRef::Wild => {}
                _ => return None,
            }
        }
        let step: Vec<usize> = r.head().args().map(|t| t.var().and_then(|s| col.get(&s.0).copied())).collect::<Option<_>>()?;
        if step.iter().collect::<BTreeSet<_>>().len() != step.len() {
            return None;
        }
        map = Some(match map { None => step, Some(m) => m.iter().map(|&c| step[c]).collect() });
        at = v.rel_name(a.rel()).to_owned();
    }
    None
}

/// Each relation of `prog` that heads one rule, with it; `None` for one that heads more (`projection_of`).
fn rule_by_head(v: &crate::code_rule_dsl::logic::program_image::Version) -> HashMap<&str, Option<crate::code_rule_dsl::logic::program_image::RuleRef<'_>>> {
    let mut heads: HashMap<&str, Option<crate::code_rule_dsl::logic::program_image::RuleRef<'_>>> = HashMap::new();
    for r in v.rules() {
        heads.entry(v.rel_name(r.head().rel())).and_modify(|h| *h = None).or_insert(Some(r));
    }
    heads
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

/// The relations a version's program derives, each with its declared width (`collect`'s), read from its fields.
fn derived_widths_in<'v>(f: &super::fields::Fields<'v>) -> BTreeMap<&'v str, usize> {
    f.v.rules().map(|r| f.v.rel_name(r.head().rel())).map(|rel| (rel, f.width(rel))).collect()
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
/// `run_atoms_native_over` with the run's text tests (`Hooks::texts`).
fn texts_run(lowered: &lower::Lowered, db: &Connection, given: super::Stats, kept: BTreeSet<String>, modes: &str, texts: &Arc<crate::code_rule_dsl::logic::strings::Vocabulary>) -> Result<Outcome> {
    super::run_atoms_native_hooked(lowered, db, "main", given, kept, modes, super::Hooks { texts: Some(Arc::clone(texts)), ..Default::default() })
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
/// per-document sums (`sqd`) add exactly, and node columns' values never cross documents, so their `ndv` and `sq`
/// add exactly too. A value column's values cross chunks: its `ndv` and `sq` are of each value's total, kept as
/// `ValTotals`: exact while it has `SMALL_NDV` values or fewer, else sketched (HyperLogLog and AMS, Q89). Its
/// tested values are always counted exactly: the planner reads a constant missing from `stat_val` as matching no
/// row (`const_freq`).
#[derive(Default)]
struct Totals {
    /// The values the program tests, of any relation and column (`constants_tested_in`).
    tested: Arc<rustc_hash::FxHashSet<i64>>,
    rels: BTreeMap<String, (usize, BTreeMap<String, [i64; 4]>)>,
    /// Per (relation, value column).
    vals: BTreeMap<(String, String), ValTotals>,
    /// Constants' counts of columns whose statistics are given whole (`add`): (relation, column, value, count).
    consts: Vec<(String, String, i64, i64)>,
}
/// A value column's totals over the chunks.
#[derive(Clone, Default)]
struct ValTotals {
    /// The sum over documents of the square of each value's rows in it.
    sqd: i64,
    /// Each value's rows while the column has at most `SMALL_NDV` values; `None` once it has more.
    few: Option<HashMap<i64, i64>>,
    /// The tested values' rows.
    tested: HashMap<i64, i64>,
    /// Its values sketched, for `ndv` and `sq` once `few` is dropped.
    moments: super::sketch::Moments,
    /// Whether `few` was dropped (a column starts with every value counted).
    many: bool,
}

impl ValTotals {
    fn add(&mut self, tested: &rustc_hash::FxHashSet<i64>, v: i64, n: i64) {
        self.moments.add(v, n);
        if tested.contains(&v) {
            *self.tested.entry(v).or_default() += n;
        }
        if !self.many {
            let few = self.few.get_or_insert_with(HashMap::new);
            *few.entry(v).or_default() += n;
            if few.len() as i64 > super::SMALL_NDV {
                self.few = None;
                self.many = true;
            }
        }
    }
    /// (ndv, sq): exact while the column has few values, else the sketch's.
    fn moments(&self) -> (i64, i64) {
        match &self.few {
            Some(few) => (few.len() as i64, few.values().map(|n| n * n).sum()),
            None => (self.moments.ndv().round() as i64, self.moments.sq().round() as i64),
        }
    }
}

impl Totals {
    /// Totals that count `tested`'s values exactly (`ValTotals`).
    fn new(tested: rustc_hash::FxHashSet<i64>) -> Self {
        Totals { tested: Arc::new(tested), ..Default::default() }
    }
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
    pub fn add_rows(&mut self, rel: &str, inserted: usize, rows: &dyn crate::code_rule_dsl::facts::vtab::Source, types: &[&str]) {
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
            // Per value of this chunk: its rows, and the sum of squares of its rows per document.
            let vals = self.vals.entry((rel.to_owned(), name.clone())).or_default();
            value_runs(rows, c, &doc, |v, count, docs| {
                vals.add(&self.tested, v, count);
                vals.sqd += docs.iter().map(|k| k * k).sum::<i64>();
            });
        }
    }
    /// The totals as the planner's statistics (T's `stats.py` definitions), over `stats`. Per value, only what the
    /// planner reads (Q89): the values the program tests (`tested`), and every value of a column of `SMALL_NDV` or
    /// fewer (`small_values`). Every distinct value of every export column, kept, was 118 MB of a cold PostHog run's
    /// cross part that nothing read.
    fn apply(&self, stats: &mut super::Stats) {
        for (rel, (rows, measured)) in &self.rels {
            stats.measured(rel, measured.iter().map(|(col, [_, ndv, sq, sqd])| (col.clone(), [*rows as i64, (*ndv).max(1), *sq, *sqd])), []);
        }
        for ((rel, col), per) in &self.vals {
            let rows = self.rels.get(rel).map_or(0, |r| r.0 as i64);
            let (ndv, sq) = per.moments();
            let values = per.few.as_ref().unwrap_or(&per.tested);
            stats.measured(rel, [(col.clone(), [rows, ndv.max(1), sq, per.sqd])], values.iter().map(|(v, n)| (col.clone(), *v, *n)));
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


/// Whether the corpus runner reads the file at `path` as text only in a run of `label`'s grammar: it is not of that
/// grammar, so it is in the run only because a `.scope text` glob reads it (`scoped`), and it has `doc` and the
/// source text's relations and no tree.
fn text_document(path: &str, label: &str) -> bool {
    crate::code_rule_dsl::language::label_of_path(path) != Some(label)
}

/// A corpus's files by language: Python's, Rust's and `.rr` rule files, and the text-only documents.
#[derive(Clone)]
pub struct CorpusFiles {
    /// Each dialect's files, by its label (the language registry's).
    pub by_label: BTreeMap<&'static str, Vec<String>>,
    /// Files of no grammar: after `scoped`, those a `.scope text` glob reads as text (`ProgramFields::text_globs`),
    /// with the files of a grammar it matches (a `.md` file is Markdown's, and text to a run of another grammar).
    pub text: Vec<String>,
}

impl CorpusFiles {
    /// The run's documents: the language's files, then the text ones, in path order.
    /// A file of `label`'s grammar a `.scope text` glob also matches is parsed, not read as text as well.
    fn of(mut self, label: &str) -> Vec<String> {
        let mut files = self.by_label.remove(label).unwrap_or_default();
        files.extend(self.text.into_iter().filter(|p| crate::code_rule_dsl::language::label_of_path(p) != Some(label)));
        files.sort();
        files.dedup();
        files
    }

    /// How many files of `label` there are.
    pub fn count(&self, label: &str) -> usize {
        self.by_label.get(label).map_or(0, Vec::len)
    }

    /// How many files of any grammar there are.
    pub fn parsed(&self) -> usize {
        self.by_label.values().map(Vec::len).sum()
    }
}

/// The language a corpus run reads: the one its program names (`.language`: any dialect of the registry this build
/// parses), else Rust or Python by `Typing::rust`, or by whichever has more files.
/// The grammar a run of `prog` reads (`corpus_language`, as `run_observed` picks it): its `.language`'s, else the
/// tree's own, of the files its scope selects. A batch's policies that read one grammar run as one pack (decision 0168).
pub fn run_language(root: &Path, prog: &(impl ProgramFields + ?Sized), options: &serde_json::Value) -> Result<&'static str> {
    let language = prog.setting_values("language").into_iter().next().map(|v| v.trim_matches('"').to_owned());
    if let Some(label) = language.as_deref().and_then(crate::code_rule_dsl::language::label_named) {
        return Ok(label);
    }
    Ok(corpus_language(language.as_deref(), &Typing::default(), &scoped(corpus_files(root)?, prog, options)))
}

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
pub fn run_dialects(root: &Path, prog: &(impl ProgramFields + ?Sized), options: &serde_json::Value) -> Result<Vec<&'static str>> {
    run_dialects_in(&|| corpus_files(root), prog, options)
}

/// [`run_dialects`] over a corpus listing `files` makes (once, for many programs).
pub fn run_dialects_in(files: &dyn Fn() -> Result<CorpusFiles>, prog: &(impl ProgramFields + ?Sized), options: &serde_json::Value) -> Result<Vec<&'static str>> {
    use crate::code_rule_dsl::language::{CodeRuleLanguage, label};
    let Some(name) = prog.setting_values("language").into_iter().next().map(|v| v.trim_matches('"').to_owned()) else {
        return Ok(Vec::new());
    };
    let Some(language) = CodeRuleLanguage::ALL.into_iter().find(|l| l.name() == name) else { return Ok(Vec::new()) };
    if language.dialects().len() < 2 {
        return Ok(Vec::new());
    }
    let found = scoped(files()?, prog, options);
    let mut labels: Vec<&'static str> = language.dialects().iter().filter_map(|d| label(*d)).filter(|l| found.count(l) > 0).collect();
    labels.sort_by_key(|l| *l != name);
    Ok(labels)
}

/// The files the program's policies select (`ProgramFields::doc_scopes`, their `.scope`): a document outside
/// every policy's scope is not parsed, and gives no facts.
/// A scope entry taken from a policy option (`.scope dirs option("…")`) reads the option's values, from the run's
/// `options`. A file of no grammar is read, as text only, when a `.scope text` glob matches it and a scope selects it.
fn scoped(mut found: CorpusFiles, prog: &(impl ProgramFields + ?Sized), options: &serde_json::Value) -> CorpusFiles {
    let globs = prog.text_globs();
    let text_glob = |p: &String| globs.iter().any(|g| crate::code_rule_dsl::logic::strings::test("glob", p, g));
    found.text.retain(text_glob);
    // A glob names files of a grammar too (`"**/*.md"`): a run of another grammar reads them as text (`CorpusFiles::of`).
    // A hidden file (`.hidden.md`) is no text document, as `corpus_files` has it.
    let hidden = |p: &String| p.rsplit('/').next().is_some_and(|name| name.starts_with('.'));
    let of_grammars: Vec<String> = found.by_label.values().flatten().filter(|p| text_glob(p) && !hidden(p)).cloned().collect();
    found.text.extend(of_grammars);
    found.text.sort();
    // A program of JSON policies (`.policy {…}`) and no `.scope`: their `scope`s (decision 0107).
    let doc_scopes = prog.doc_scopes().or_else(|| json_scopes(&prog.policies()));
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
    // What the record read (`reuse::encode_lists`): the journal's cursors before the status, the roots they are of,
    // and the paths read; and what it found: the overlays' count (eight bytes), then the files.
    let kept = observe(|o| {
        let (read, files) = o.run_record(&key)?;
        let [cursors, watched, read]: [Vec<String>; 3] = reuse::decode_lists(&read)?.try_into().ok()?;
        let watched: Vec<PathBuf> = watched.into_iter().map(PathBuf::from).collect();
        let read: Vec<PathBuf> = read.into_iter().map(PathBuf::from).collect();
        let named = o.changed_since(&key, &watched, &cursors)?;
        // The repository's own directories, of which only what `read` names counts (not its objects).
        let git_dirs: Vec<PathBuf> = watched.iter().skip(2).cloned().collect();
        let overlays = u64::from_le_bytes(files.get(..8)?.try_into().ok()?) as usize;
        let [under]: [Vec<String>; 1] = reuse::decode_lists(&files[8..])?.try_into().ok()?;
        Some((read, git_dirs, named, overlays, under))
    });
    if let Some((read, git_dirs, named, overlays, under)) = kept {
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
    // Kept when Git sees no file under the root too (a corpus in an ignored directory): unkept, every run read the
    // work tree's whole status again to find none (decision 0285).
    let found = observe(|o| o.git(root)).map(|g| {
        let prefix = real.strip_prefix(&g.worktree_root).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        let under: Vec<String> = g
            .worktree_files()
            .into_iter()
            .filter_map(|file| if prefix.is_empty() { Some(file) } else { file.strip_prefix(prefix.as_str()).and_then(|f| f.strip_prefix('/')).map(str::to_owned) })
            .collect();
        (under, g.overlays.len())
    });
    if let (Some((work, own, common)), Some(cursors), Some((under, overlays))) = (&places, cursors, &found) {
        let mut read: Vec<PathBuf> = vec![real.clone(), work.join(".git"), own.join("HEAD"), own.join("index")];
        read.extend(real.ancestors().filter(|d| d.starts_with(work) && *d != real.as_path()).map(|d| d.join(".gitignore")));
        read.extend(["HEAD", "refs", "packed-refs", "config", "info"].iter().map(|f| common.join(f)));
        let watched: Vec<String> = [&real, work, own, common].iter().map(|p| p.to_string_lossy().into_owned()).collect();
        let read: Vec<String> = read.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        let record = reuse::encode_lists(&[&cursors[..], &watched[..], &read[..]]);
        let mut files = (*overlays as u64).to_le_bytes().to_vec();
        files.extend(reuse::encode_lists(&[&under[..]]));
        observe(|o| o.put_run_record(&key, &record, &files));
    }
    found.filter(|(under, _)| !under.is_empty())
}

/// The Python files among `paths` (relative to `root`) whose module, as Python imports it, is not their path's name
/// (`syntax::module`), each with that module. Python names a module from its package root: the highest folder
/// reached by climbing from the file's own while each holds an `__init__.py` (`model::python_module_in`, the name the
/// hosts send). Below a source root the two differ: `src/shop/base.py` is `shop.base`, which `from shop.base import
/// Base` names, not `src.shop.base`. A file in no package (a script, or a folder of the root without `__init__.py`)
/// keeps its path's name, as does one whose names keep a dot.
pub(in crate::code_rule_dsl) fn package_root_modules(root: &Path, paths: &[String]) -> BTreeMap<String, String> {
    let mut packages: HashMap<String, bool> = HashMap::new();
    let mut is_package = |dir: &str| *packages.entry(dir.to_owned()).or_insert_with(|| root.join(dir).join("__init__.py").is_file());
    let mut out = BTreeMap::new();
    for p in paths.iter().filter(|p| p.ends_with(".py") || p.ends_with(".pyi")) {
        let Some((dir, _)) = p.rsplit_once('/') else { continue };
        // Climbs while each folder holds an `__init__.py`; `top` is the package root's first segment's index.
        let segments: Vec<&str> = dir.split('/').collect();
        let mut top = segments.len();
        while top > 0 && is_package(&segments[..top].join("/")) {
            top -= 1;
        }
        if top == segments.len() || top == 0 {
            // In no package, or the root's own folders are packages up to the tree's root: the path's name.
            continue;
        }
        let m = syntax::module(&segments[top..].iter().copied().chain(p.rsplit_once('/').map(|(_, f)| f)).collect::<Vec<_>>().join("/"));
        if m != syntax::module(p) {
            out.insert(p.clone(), m);
        }
    }
    out
}

/// The corpus's files under `root`, relative and in path order: Python's (`.py`), Rust's (`.rs`) and rule files
/// (`.rr`), skipping hidden directories and `target`.
pub fn corpus_files(root: &Path) -> Result<CorpusFiles> {
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
    // The run reads its files by their canonical paths too (`Observer::canonical`): a root reached through a symlink
    // (macOS's `/var` is `/private/var`) has its real part read before the waves as well, where no store is asked.
    observe(|o| {
        o.preload(root);
        if let Ok(real) = std::fs::canonicalize(root)
            && real != root
        {
            o.preload(&real);
        }
    });
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

// A statistics sample has its own row budget, independent of execution chunks.
#[doc(hidden)]
pub const STAT_SAMPLE_ROWS: usize = 8192;
#[doc(hidden)]
pub fn sampled_statistics(source: &dyn crate::code_rule_dsl::facts::vtab::Source) -> Vec<(String, i64, i64, i64, i64)> {
    if let Some(known) = source.statistics() { return known; }
    // Sampling must not force a lookup-driven producer to enumerate its domain.
    if source.made_on_lookup() { return Vec::new(); }
    let rows = source.len();
    let count = rows.min(STAT_SAMPLE_ROWS);
    let names = source.columns();
    let columns: Vec<usize> = (0..names.len()).collect();
    let mut values = vec![Vec::with_capacity(count); names.len()];
    let mut row = vec![0; names.len()];
    for i in 0..count {
        // Deterministic strata cover the whole relation without a full scan.
        let at = (i as u128 * rows as u128 / count as u128) as usize;
        source.row_at(at, &columns, &mut row);
        for (column, value) in values.iter_mut().zip(&row) { column.push(*value); }
    }
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Value, (count * names.len()) as u64);
    let sample = Rows { columns: names, values };
    let measured = syntax::measured(&sample);
    crate::code_rule_dsl::logic::profile::decision("statistics sample", || format!("{count} of {rows} rows (limit {STAT_SAMPLE_ROWS})"));
    if count == rows { measured } else { scaled_statistics(&measured, count, rows) }
}

/// `sampled_statistics` of `rel`'s columns its layout does not answer (decision 0361 item 5): an answered column's
/// statistic is its facts' estimate (`Shapes::statistic`), not counted, and a relation they answer whole reads none
/// of its rows.
#[doc(hidden)]
pub fn sampled_statistics_unanswered(rel: &str, source: &dyn crate::code_rule_dsl::facts::vtab::Source, shapes: &super::shapes::Shapes, counts: super::shapes::Counts) -> Vec<(String, i64, i64, i64, i64)> {
    if let Some(known) = source.statistics() { return known; }
    if source.made_on_lookup() { return Vec::new(); }
    let rows = source.len();
    let names = source.columns();
    let answered: Vec<Option<(i64, i64, i64, i64)>> = names.iter().map(|n| shapes.statistic(rel, n, rows as i64, counts)).collect();
    let open: Vec<usize> = (0..names.len()).filter(|&c| answered[c].is_none()).collect();
    let mut counted: HashMap<usize, (String, i64, i64, i64, i64)> = HashMap::new();
    if !open.is_empty() {
        let count = rows.min(STAT_SAMPLE_ROWS);
        let mut values = vec![Vec::with_capacity(count); open.len()];
        let mut row = vec![0; open.len()];
        for i in 0..count {
            let at = (i as u128 * rows as u128 / count as u128) as usize;
            source.row_at(at, &open, &mut row);
            for (column, value) in values.iter_mut().zip(&row) { column.push(*value); }
        }
        crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Value, (count * open.len()) as u64);
        let sample = Rows { columns: open.iter().map(|&c| names[c].clone()).collect(), values };
        let measured = syntax::measured(&sample);
        let measured = if count == rows { measured } else { scaled_statistics(&measured, count, rows) };
        counted = open.iter().copied().zip(measured).collect();
    }
    names
        .iter()
        .enumerate()
        .map(|(c, n)| match answered[c] {
            Some((r, ndv, sq, sqd)) => (n.clone(), r, ndv, sq, sqd),
            None => counted.remove(&c).expect("counted"),
        })
        .collect()
}

/// Rows held as columns that keep no statistics of their own (`Rows`), for a test of a measurement.
#[doc(hidden)]
pub fn unmeasured_rows(columns: Vec<String>, values: Vec<Vec<i64>>) -> impl crate::code_rule_dsl::facts::vtab::Source {
    Rows { columns, values }
}

/// A digest as lowercase hex.
fn hex(digest: &[u8]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// A memo entry's rows holding its file's tree statistics (`Own::put`).
const OWN_KINDS: &str = "own:kinds";
const OWN_EDGES: &str = "own:edges";
const OWN_FIELDS: &str = "own:fields";
pub const OWN_DOCS: &str = "own:docs";
const OWN_POSTINGS: &str = "own:postings:";

/// The base relations' statistics a chunk measures as it binds them, beyond each column's moments
/// (`syntax::statistics`), which T's `stats.py` made at ingestion: per kind atom its nodes and summed subtree sizes
/// (`stat_kind`), per (parent kind, child kind) its edges (`stat_edge`), the documents, and per (relation, column,
/// constant) of the program's constants its rows (`stat_val`'s `n`). Summed over the chunks for the cross-file part:
/// a document is in one chunk, so every count adds exactly.
#[derive(Clone, Default)]
pub struct Own {
    pub kinds: BTreeMap<i64, (i64, i64)>,
    pub edges: BTreeMap<(i64, i64), i64>,
    /// Per (parent kind, child kind, field) its fielded edges (`field`'s rows): which kinds have a field, and which
    /// kinds a field's children are (Q44: `kind($n, k)` after `field($n, f, _)` passes nearly every row).
    pub fields: BTreeMap<(i64, i64, i64), i64>,
    pub docs: i64,
    pub postings: BTreeMap<(String, String, i64), i64>,
}

impl Own {
    /// The program's integer constants by (input relation, column name) they are compared at, read from the
    /// program's version by its rules' handles. A kind, field or name is a branded atom (decision 0270), compared as
    /// its atom's value, as `Body::constant` reads it (Q36: unread, `field($l, "left", $t)` had no postings, and was
    /// priced at one row a line). A constant tested of a relation that copies an input's columns (`kind(n, k) :-
    /// node(_, n, k, _, _)`, Q37) is counted at the input's column it copies (`constants_tested_in`): the input is
    /// what has statistics, and `kind`'s tests had none since `kind` became a view.
    pub fn constants_of(v: &crate::code_rule_dsl::logic::program_image::Version) -> BTreeMap<(String, String), BTreeSet<i64>> {
        let inputs: BTreeSet<&str> = v.inputs().map(|a| v.text(a)).collect();
        let mut out: BTreeMap<(String, String), BTreeSet<i64>> = BTreeMap::new();
        for (rel, tested) in constants_tested_in(v) {
            if !inputs.contains(rel) {
                continue;
            }
            for (col, n) in tested {
                out.entry((rel.to_owned(), super::sql_col(rel, col))).or_default().insert(n);
            }
        }
        out
    }

    pub fn measure(sources: &[(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)], constants: &BTreeMap<(String, String), BTreeSet<i64>>) -> Self {
        use rustc_hash::{FxHashMap, FxHashSet};
        let mut own = Own::default();
        let of = |rel: &str| sources.iter().find(|(r, _)| r == rel).map(|(_, s)| s.as_ref());
        let column = |source: &dyn crate::code_rule_dsl::facts::vtab::Source, name: &str| source.columns().iter().position(|c| c == name);
        // Kinds by node id, then the tape's sizes and parents against them. Counted in hash maps (a chunk is a
        // bounded prefix: an ordered map per node was most of this measure's time) and ordered once.
        let mut kind_of = FxHashMap::default();
        let mut kinds: FxHashMap<i64, (i64, i64)> = FxHashMap::default();
        let mut edges: FxHashMap<(i64, i64), i64> = FxHashMap::default();
        if let Some(node) = of("node")
            && let (Some(n), Some(k), Some(d)) = (column(node, "n"), column(node, "kind"), column(node, "d"))
        {
            kind_of.reserve(node.len().min(STAT_SAMPLE_ROWS));
            let mut docs = FxHashSet::default();
            for r in 0..node.len().min(STAT_SAMPLE_ROWS) {
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
            for r in 0..tape.len().min(STAT_SAMPLE_ROWS) {
                let Some(&kind) = kind_of.get(&tape.value(r, n)) else { continue };
                kinds.entry(kind).or_default().1 += tape.value(r, size);
                if let Some(&parent) = kind_of.get(&tape.value(r, par)) {
                    *edges.entry((parent, kind)).or_default() += 1;
                }
            }
        }
        own.kinds = kinds.into_iter().collect();
        own.edges = edges.into_iter().collect();
        if let Some(node) = of("node") {
            let sampled = node.len().min(STAT_SAMPLE_ROWS);
            if sampled > 0 { own = own.scaled(node.len() as f64 / sampled as f64); }
        }
        // The fielded edges of the sampled nodes (`field`'s rows in node order, as the tape's above): each with its
        // parent's and child's kind, scaled as the edges are. A view's rows past the sampled nodes have no kind here.
        if let Some(field) = of("field")
            && let (Some(p), Some(f), Some(c)) = (column(field, "p"), column(field, "f"), column(field, "c"))
            && let Some(node) = of("node")
        {
            let sampled = node.len().min(STAT_SAMPLE_ROWS);
            let scale = if sampled > 0 { node.len() as f64 / sampled as f64 } else { 1.0 };
            let mut fields: FxHashMap<(i64, i64, i64), i64> = FxHashMap::default();
            for r in 0..field.len().min(2 * STAT_SAMPLE_ROWS) {
                if let (Some(&pk), Some(&ck)) = (kind_of.get(&field.value(r, p)), kind_of.get(&field.value(r, c))) {
                    *fields.entry((pk, ck, field.value(r, f))).or_default() += 1;
                }
            }
            own.fields = fields.into_iter().map(|(k, n)| (k, (n as f64 * scale).round().max(1.0) as i64)).collect();
        }
        if let Some(doc) = of("doc") { own.docs = doc.estimated_len() as i64; }
        own.postings = Self::postings(sources, constants, |_| true);
        own
    }

    /// The program's constants' postings in the relations `of_rel` keeps: rows per (relation, column, value).
    pub fn postings(
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
            let asked: Vec<_> = values.iter().map(|&v| (c,v)).collect();
            if let Some(counts) = source.value_counts(&asked) {
                out.extend(asked.into_iter().zip(counts).map(|((_,v),n)| ((rel.clone(),col.clone(),v),n)));
                continue;
            }
            let wanted: FxHashSet<i64> = values.iter().copied().collect();
            let mut counts: FxHashMap<i64, i64> = FxHashMap::default();
            if source.made_on_lookup() { continue; }
            let rows = source.len();
            let sampled = rows.min(STAT_SAMPLE_ROWS);
            for r in 0..sampled {
                let at = (r as u128 * rows as u128 / sampled as u128) as usize;
                let v = source.value(at, c);
                if wanted.contains(&v) {
                    *counts.entry(v).or_default() += 1;
                }
            }
            out.extend(counts.into_iter().map(|(v, n)| ((rel.clone(), col.clone(), v), (n as f64 * rows as f64 / sampled.max(1) as f64).round() as i64)));
        }
        out
    }

    /// These counts for a chunk `r` times this one's size: every count is of nodes, edges, documents or rows, so
    /// each scales by `r`.
    pub fn scaled(&self, r: f64) -> Own {
        let s = |n: i64| (n as f64 * r).round() as i64;
        Own {
            kinds: self.kinds.iter().map(|(k, (n, size))| (*k, (s(*n), s(*size)))).collect(),
            edges: self.edges.iter().map(|(e, n)| (*e, s(*n))).collect(),
            fields: self.fields.iter().map(|(e, n)| (*e, s(*n))).collect(),
            docs: s(self.docs).max(1),
            postings: self.postings.iter().map(|(k, n)| (k.clone(), s(*n))).collect(),
        }
    }


    pub fn add(&mut self, other: &Own) {
        for (k, (n, size)) in &other.kinds {
            let e = self.kinds.entry(*k).or_default();
            e.0 += n;
            e.1 += size;
        }
        for (e, n) in &other.edges {
            *self.edges.entry(*e).or_default() += n;
        }
        for (e, n) in &other.fields {
            *self.fields.entry(*e).or_default() += n;
        }
        self.docs += other.docs;
        for (key, n) in &other.postings {
            *self.postings.entry(key.clone()).or_default() += n;
        }
    }

    /// `other` taken away (a file's share counted out): counts that reach 0 are dropped, as a sum without it holds.
    pub fn sub(&mut self, other: &Own) {
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
        for (e, n) in &other.fields {
            let c = self.fields.entry(*e).or_default();
            *c -= n;
            if *c <= 0 {
                self.fields.remove(e);
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
    pub fn put(&self, rows: &mut BTreeMap<String, (usize, Vec<i64>)>) {
        rows.insert(OWN_KINDS.into(), (3, self.kinds.iter().flat_map(|(k, (n, size))| [*k, *n, *size]).collect()));
        rows.insert(OWN_EDGES.into(), (3, self.edges.iter().flat_map(|((p, k), n)| [*p, *k, *n]).collect()));
        rows.insert(OWN_FIELDS.into(), (4, self.fields.iter().flat_map(|((p, k, f), n)| [*p, *k, *f, *n]).collect()));
        rows.insert(OWN_DOCS.into(), (1, vec![self.docs]));
        for ((rel, col, v), n) in &self.postings {
            rows.entry(format!("{OWN_POSTINGS}{rel}:{col}")).or_insert_with(|| (2, Vec::new())).1.extend([*v, *n]);
        }
    }

    /// Add a frame directly to the run's totals. No per-file maps are reconstructed.
    pub fn add_rows(&mut self, rows: &BTreeMap<String, (usize, Vec<i64>)>) {
        let flat = |key: &str| rows.get(key).map(|(_, v)| v.as_slice()).unwrap_or_default();
        for r in flat(OWN_KINDS).chunks_exact(3) {
            let k = self.kinds.entry(r[0]).or_default(); k.0 += r[1]; k.1 += r[2];
        }
        for r in flat(OWN_EDGES).chunks_exact(3) { *self.edges.entry((r[0], r[1])).or_default() += r[2]; }
        for r in flat(OWN_FIELDS).chunks_exact(4) { *self.fields.entry((r[0], r[1], r[2])).or_default() += r[3]; }
        self.docs += flat(OWN_DOCS).first().copied().unwrap_or(0);
        for (key, (_, values)) in rows.range(OWN_POSTINGS.to_owned()..).take_while(|(k, _)| k.starts_with(OWN_POSTINGS)) {
            let Some((rel, col)) = key[OWN_POSTINGS.len()..].split_once(':') else { continue };
            for r in values.chunks_exact(2) { *self.postings.entry((rel.into(), col.into(), r[0])).or_default() += r[1]; }
        }
    }

    /// From a memo entry's rows (`put`).
    pub fn taken(rows: &BTreeMap<String, (usize, Vec<i64>)>) -> Own {
        let mut own = Own::default();
        let flat = |key: &str| rows.get(key).map(|(_, v)| v.as_slice()).unwrap_or_default();
        own.kinds = flat(OWN_KINDS).chunks(3).map(|r| (r[0], (r[1], r[2]))).collect();
        own.edges = flat(OWN_EDGES).chunks(3).map(|r| ((r[0], r[1]), r[2])).collect();
        own.fields = flat(OWN_FIELDS).chunks_exact(4).map(|r| ((r[0], r[1], r[2]), r[3])).collect();
        own.docs = flat(OWN_DOCS).first().copied().unwrap_or(0);
        for (key, (_, values)) in rows.range(OWN_POSTINGS.to_owned()..).take_while(|(k, _)| k.starts_with(OWN_POSTINGS)) {
            let Some((rel, col)) = key[OWN_POSTINGS.len()..].split_once(':') else { continue };
            own.postings.extend(values.chunks(2).map(|r| ((rel.to_owned(), col.to_owned(), r[0]), r[1])));
        }
        own
    }

    pub fn apply(&self, stats: &mut super::Stats) {
        stats.tree(&self.kinds, &self.edges, self.docs);
        stats.fielded(self.fields.iter().map(|(&(p, c, f), &n)| (p.to_string(), c.to_string(), f.to_string(), n as f64)));
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
    /// The chunks' totals give the planner every column's moments, but per value only what it reads (Q89): the
    /// values the program tests (`binding_domains`' lookups by constant), and every value of a column of few
    /// (`small_values`). Held whole, every distinct value of every export column was 118 MB of the cross part's
    /// statistics, which nothing read.
    #[test]
    fn the_totals_keep_per_value_counts_only_for_what_the_planner_reads() {
        use crate::code_rule_dsl::logic::plan::{Constant, Stats};
        let p = crate::code_rule_dsl::logic::rr::parse(".decl e(a: number, b: number)\n.output e\n").unwrap();
        let many: Vec<i64> = (0..1000).map(|i| 100 + i % 500).collect();
        let few: Vec<i64> = (0..1000).map(|i| i % 3).collect();
        let source = crate::code_rule_dsl::logic::native::component::columns_source(vec!["c0".into(), "c1".into()], vec![Some(many), Some(few)]);
        let mut totals = super::Totals::new([107].into_iter().collect());
        // Two chunks holding the same values: each value's rows are its total over both.
        totals.add_rows("e", 1000, source.as_ref(), &["number", "number"]);
        totals.add_rows("e", 1000, source.as_ref(), &["number", "number"]);
        let mut stats = Stats::empty(&p);
        totals.apply(&mut stats);
        let memo = &stats.memo["e"];
        let of = |col: &str| memo.keys().filter(|(c, _)| c == col).count();
        assert_eq!(of("c0"), 1, "a column of many values keeps the tested one alone");
        assert_eq!(memo.get(&("c0".to_owned(), Constant::Int(107))), Some(&4));
        assert_eq!(of("c1"), 3, "a column of few keeps them all");
        let col = |c: &str| *stats.col.get(&("e".to_owned(), c.to_owned())).unwrap();
        let [rows, ndv, sq, _] = col("c0");
        assert_eq!(rows, 2000);
        assert!((ndv - 500).abs() <= 25, "a column of many: its distinct values sketched, {ndv} for 500");
        assert!(sq * 2 > 500 * 16 && sq < 500 * 16 * 2, "its squared postings sketched, {sq} for {}", 500 * 16);
        assert_eq!(col("c1")[..3], [2000, 3, 668 * 668 + 2 * 666 * 666], "a column of few, exact: (2·334)² + 2·(2·333)²");
    }

    /// A corpus in a directory Git ignores has no file Git sees, and that answer is kept too (`git_files`'s record):
    /// unkept, every run read the work tree's whole status again to find none. In a process of its own, whose store
    /// is named (`RERAIL_TAPE_CACHE`), so the observer asks the journal, as a run's does.
    #[test]
    fn git_seeing_no_file_under_the_root_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join("work");
        std::fs::create_dir_all(work.join("ignored/pkg")).unwrap();
        // Git as the test states it, not as the host's configuration does (a farm host's signing or hooks failed the
        // commit): no global or system configuration, no signing.
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .args(["-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", "-C"])
                .arg(&work)
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        git(&["init", "-q"]);
        std::fs::write(work.join(".gitignore"), "/ignored/\n").unwrap();
        std::fs::write(work.join("ignored/pkg/a.py"), "x = 1\n").unwrap();
        git(&["add", ".gitignore"]);
        git(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "a work tree with a head"]);
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "code_rule_dsl::logic::plan::corpus::tests::git_seeing_no_file_under_the_root_is_kept_child", "--ignored", "--test-threads", "1", "--nocapture"])
            .env("RERAIL_TAPE_CACHE", dir.path().join("store.sqlite"))
            .env("RERAIL_GIT_FILES_CHILD", work.join("ignored"))
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success() && stdout.contains("1 passed"), "{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
    }

    #[test]
    #[ignore = "run by git_seeing_no_file_under_the_root_is_kept"]
    fn git_seeing_no_file_under_the_root_is_kept_child() {
        let Some(root) = std::env::var_os("RERAIL_GIT_FILES_CHILD").map(std::path::PathBuf::from) else { return };
        // Each in a run of its own (`ObservedBatch`): the observer asks the journal for a run under way.
        {
            let _run = super::ObservedBatch::begin();
            assert_eq!(super::git_files(&root), None, "Git sees no file under an ignored directory");
        }
        let real = std::fs::canonicalize(&root).unwrap();
        let record = super::observe(|o| o.run_record(&format!("git-files:{}", real.display())));
        // As `git_files` writes it: the overlays' count (eight bytes), then the files' list.
        let mut empty = 0u64.to_le_bytes().to_vec();
        // Typed apart: Relax's rustc does not infer `&[][..] as &[String]` (E0283), as cargo's does.
        let no_files: &[String] = &[];
        empty.extend(super::reuse::encode_lists(&[no_files]));
        assert_eq!(record.map(|(_, files)| files), Some(empty), "the empty answer is kept, with no overlay and no file");
        let _run = super::ObservedBatch::begin();
        assert_eq!(super::git_files(&root), None, "and answered from the record");
    }

    /// Q20: an evaluated document no old entry names, whose rows this run are its kept digest's, is unchanged without
    /// its kept rows read (`no-reads-of-unchanged`): here the kept source holds other rows than the digest says, so a
    /// read of them is an error. A document whose rows changed is patched from its kept rows as before.
    #[test]
    fn an_unchanged_evaluated_owners_kept_rows_are_not_read() {
        use super::*;
        let doc = 7i64;
        let rows = vec![vec![doc, 1], vec![doc, 2]];
        let sum = rows.iter().map(|r| super::super::row_digest(r)).fold(0u64, u64::wrapping_add);
        let docs: retain::DocDigests = [(doc, (sum, 2))].into_iter().collect();
        let ranges: retain::Ranges = [(doc, (0u32, 2u32))].into_iter().collect();
        let kept_of = |c1: Vec<i64>| retain::KeptInput::laid(crate::code_rule_dsl::logic::native::component::columns_source(vec!["c0".into(), "c1".into()], vec![Some(vec![doc; 2]), Some(c1)]), docs.clone(), ranges.clone());
        let evaluated: BTreeSet<i64> = [doc].into_iter().collect();
        let cells = super::super::Cells::plain(2);
        let other = kept_of(vec![9, 9]);
        let holders = rustc_hash::FxHashMap::default();
        let patch = patch_owners("defs", &other, 0, &holders, &evaluated, &[], rows.clone(), &cells).unwrap();
        assert!(patch.changed.is_empty() && patch.docs.is_none(), "its rows are its kept digest's: {:?}", patch.changed);
        // Held once: this run's rows, a row twice among them, are its kept set.
        let twice = vec![vec![doc, 2], vec![doc, 1], vec![doc, 2]];
        let patch = patch_owners("defs", &other, 0, &holders, &evaluated, &[], twice, &cells).unwrap();
        assert!(patch.changed.is_empty() && patch.docs.is_none(), "an export's rows are its kept set: {:?}", patch.changed);
        let kept = kept_of(vec![1, 2]);
        let patch = patch_owners("defs", &kept, 0, &holders, &evaluated, &[], vec![vec![doc, 1], vec![doc, 3]], &cells).unwrap();
        assert_eq!(patch.changed, vec![(doc, vec![vec![doc, 1], vec![doc, 3]])]);
        // Named by place: the second of the appended rows, the second kept one.
        assert_eq!((patch.plus, patch.minus), (vec![1], vec![1]));
    }

    #[test]
    fn constant_demands_reach_projected_input_columns() {
        let v=crate::code_rule_dsl::logic::rr::parse_version(".decl raw(d:number,n:number,k:number)\n.input raw\n.decl tags(n:number,k:number)\ntags(n,k) :- raw(_,n,k).\n.decl alias(n:number,k:number)\nalias(n,k) :- tags(n,k).\n.decl out(n:number)\nout(n) :- alias(n,9).\n").unwrap();
        let demands=super::constants_tested_in(&v);
        assert!(demands["alias"].contains(&(1,9)));
        assert!(demands["tags"].contains(&(1,9)));
        assert!(demands["raw"].contains(&(2,9)),"measure the actual source column before planning its consumers");
    }

    /// A kind's or a field's name in a rule is a branded atom (decision 0270): its postings are counted as an
    /// integer constant's are (Q36: unread, `field($l, "left", $t)` was priced at one row, 1,159 read). The names are
    /// branded as lowering brands them (`plan::lower`, which needs a language's tables): by hand here. A kind is no
    /// input since 0329 (`kind(n, k)` is a view of `node`, counted by the view's statistics, `node_column_counts`),
    /// so its constant has no posting of its own.
    #[test]
    fn branded_constants_have_their_postings_counted() {
        use super::*;
        let p = super::preparation::pruned(crate::code_rule_dsl::logic::rr::parse_version(".decl left_name(l: node)\n.output left_name\nleft_name(l) :- field(l, \"left\", t), kind(t, \"identifier\").\n").unwrap()).unwrap();
        // Lowered for its dialect, as the run's prepared program is (`Own::constants_of(&prepared_placed.v)`): the
        // names are branded by lowering (0281 step 6b), not by the parse.
        let python = crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
        let mut atoms: HashMap<String, u32> = HashMap::new();
        let lowered = crate::code_rule_dsl::logic::plan::lower::lowered(p, python, &mut |s| {
            Ok(s.iter().map(|s| { let n = atoms.len() as u32 + 1; *atoms.entry((*s).to_owned()).or_insert(n) }).collect())
        })
        .unwrap();
        let placed = lowered.placed().unwrap();
        let constants = Own::constants_of(&placed.v);
        let of = |rel: &str| constants.iter().filter(|((r, _), _)| r == rel).map(|(_, v)| v.len()).sum::<usize>();
        // `kind` copies `node`'s columns (Q37): its test is counted at `node`'s kind column.
        assert_eq!((of("field"), of("node")), (1, 1), "{constants:?}");
    }

    #[test]
    fn memo_tree_statistics_accumulate_once_without_per_file_maps() {
        use super::*;
        let mut first = Own::default();
        first.kinds.insert(1,(10,20)); first.edges.insert((1,2),5); first.fields.insert((1,2,3),4); first.docs=1;
        first.postings.insert(("kind".into(),"k".into(),1),10);
        let mut rows=BTreeMap::new(); first.put(&mut rows);
        let mut totals=Own::default(); totals.add_rows(&rows); totals.add_rows(&rows);
        assert_eq!(totals.kinds[&1],(20,40)); assert_eq!(totals.edges[&(1,2)],10);
        assert_eq!(totals.docs,2); assert_eq!(totals.postings.values().sum::<i64>(),20);
        assert_eq!(totals.fields[&(1,2,3)],8, "fielded edges sum as the edges do");
        assert_eq!(Own::taken(&rows).fields, first.fields);
        totals.sub(&first); totals.sub(&first); assert!(totals.fields.is_empty());
        let projected=BTreeMap::from([(OWN_DOCS.into(),(1,vec![1]))]);
        let mut narrow=Own::default(); narrow.add_rows(&projected);
        assert_eq!(narrow.docs,1); assert!(narrow.kinds.is_empty()); assert!(narrow.edges.is_empty());
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
        super::record_run(&store, &root, &[source], &program, &[], None, None, &[], &Default::default(), None, true).unwrap().unwrap().join().unwrap().unwrap().1.unwrap();
        assert!(store.corpus_collection_due().unwrap());
        // Same files and plan: the manifest is unchanged, but pending collection must still run.
        super::record_run(&store, &root, &[source], &program, &[], None, None, &[], &Default::default(), Some(reader), true).unwrap().unwrap().join().unwrap().unwrap().1.unwrap();
        assert!(!store.corpus_collection_due().unwrap());
        assert_eq!(store.source_by_hash(orphan_hash).unwrap(), None);
    }

    #[test]
    fn a_chunks_mix_is_the_file_types_holding_a_tenth_of_its_source_bytes() {
        assert_eq!(super::chunk_mix([("a/b.py", 900), ("c.ts", 50), ("d.ts", 60)].into_iter()), "py,ts/4");
        assert_eq!(super::chunk_mix([("a/b.py", 950), ("c.ts", 50)].into_iter()), "py/4");
        assert_eq!(super::chunk_mix([("Makefile", 10), ("x.v1/README", 10)].into_iter()), "/1");
        // Many small files and a few large ones of one language are two mixes.
        assert_ne!(super::chunk_mix([("a.py", 20), ("b.py", 30)].into_iter()), super::chunk_mix([("c.py", 2000)].into_iter()));
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
/// answers them: requests taken (resolved and ordered), chunks waited for (built and evaluated on workers), chunks
/// begun and filed here, answer rows read back, and the time this thread waited for the cone's next request.
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
                "{} requests ({} rows), {} documents loaded, {} answered again: {:.3} s answering (taking {:.3} s, waiting for chunks built and evaluated on workers {:.3} s, begun and filed on this thread {:.3} s, answer rows {:.3} s); {:.3} s waiting for the cone's next request",
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

/// An edit's version of a part (decision 0244): made wherever the part had one. In a debug build (every test's), an
/// edit that made none is a fault, never a quiet fall back to an image of the tree.
pub(super) fn made_from<T>(had: bool, made: Option<T>, edit: &str) -> Option<T> {
    debug_assert!(!had || made.is_some(), "{edit}: no version was made from the part's");
    made
}

/// The cone of requests of the cross-file part `cross`, answered inside the fixpoint (decision 0065 §2a): a load's
/// answer is the loaded document's rows of every per-file relation the cone reads (an export, `exports`; a base
/// relation, `base`, is refused), made by the chunk path as any file's are; the rest of the cross-file part reads the
/// cone's rows as inputs. A derivation of the part's version alone (`answers::gate`, `answers::split`), reading no
/// tree: made once for a prepared split (`Prepared::cone`).
fn cone_of(cross: &super::body::Placed, exports: &[String], base: &[String], prog: &dyn ProgramFields) -> Result<Cone> {
    let f = cross.fields()?;
    let mut answers = world::answers(&f);
    if answers.is_empty() {
        return Err(Error::Plan("a program that reads the file system makes no request in its cross-file part".to_owned()));
    }
    // A constant of the run (an earlier phase's export, the observer's facts) is one set of rows for the run, not
    // a document's: the cone reads it as it is bound, and a load does not answer it.
    let cone = crate::code_rule_dsl::logic::answers::cone(cross, answers.keys().map(String::as_str));
    let v: &crate::code_rule_dsl::logic::program_image::Version = &cross.v;
    let per_file: BTreeSet<String> = cross
        .rules()
        .filter(|r| cone.contains(v.rel_name(r.head().rel())))
        .flat_map(|r| r.reads())
        .map(|a| v.rel_name(a.rel()).to_owned())
        .filter(|r| (exports.contains(r) || base.contains(r)) && !crate::code_rule_dsl::typing::file_local::run_constant(prog, r))
        .collect();
    if let Some(base) = per_file.iter().find(|r| base.contains(*r)) {
        let rule = cross.rules().find(|r| cone.contains(v.rel_name(r.head().rel())) && r.reads().any(|a| v.rel_name(a.rel()) == base.as_str())).map(|r| v.show_rule(r)).unwrap_or_default();
        return Err(Error::Plan(format!("the cone of requests reads base relation {base}: a load answers exports only (decision 0065 §2a); in `{rule}`")));
    }
    if let Some(produced) = answers.get_mut(world::LOAD) {
        produced.extend(per_file);
    }
    let gated = crate::code_rule_dsl::logic::answers::gate(cross, &answers).map_err(Error::Plan)?;
    let whole = super::body::Placed::whole(gated);
    let mut parts = crate::code_rule_dsl::logic::answers::split(&whole).expect("the program has answers");
    // The rest reads the answers as inputs: the runner binds what it gave.
    for rel in answers.values().flatten().filter(|r| !exports.contains(r)) {
        if !parts.rest_inputs.contains(rel) {
            parts.rest_inputs.push(rel.clone());
        }
    }
    let (load, rest) = parts.parts(&whole.v).map_err(Error::Plan)?;
    crate::code_rule_dsl::logic::profile::decision("requests (decision 0065 §2a)", || {
        let gates = super::body::Placed::whole(load.clone()).fields().map_or(0, |f| f.gates().len());
        format!(
            "{} rules make and answer requests inside one fixpoint ({gates} gated negations), before the rest of the cross-file part; answers: {}",
            load.size(),
            answers.iter().map(|(r, p)| format!("{r} → {}", p.join(", "))).collect::<Vec<_>>().join("; ")
        )
    });
    Ok(Cone { load, reads: parts.reads, rest })
}

/// `part` with its qualifiers edited by `edit` (given its rules' heads): on its version (an overlay of that field,
/// decision 0244).
fn qualify(part: &mut lower::Lowered, edit: impl FnOnce(&BTreeSet<String>, &mut BTreeMap<String, BTreeSet<String>>)) -> Result<()> {
    {
        let v = part.version.clone();
        let about = crate::code_rule_dsl::logic::program_image::manager::analysis::<crate::code_rule_dsl::logic::program_image::About>(&v).expect("a version's about rows are read");
        let mut qualifiers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (r, w) in &about.qualifiers {
            qualifiers.entry(v.text(*r).to_owned()).or_default().insert(v.text(*w).to_owned());
        }
        let heads: BTreeSet<String> = v.rules().map(|r| v.rel_name(r.head().rel()).to_owned()).collect();
        edit(&heads, &mut qualifiers);
        let words: Vec<(&str, &str)> = qualifiers.iter().flat_map(|(r, ws)| ws.iter().map(move |w| (r.as_str(), w.as_str()))).collect();
        let made = set_qualifiers(&v, &words).map_err(Error::Plan)?;
        part.version = made;
        Ok(())
    }
}

/// `v` with its qualifiers `words` (`passes::SetQualifiers`): an overlay of that field, never folded.
fn set_qualifiers(v: &Arc<crate::code_rule_dsl::logic::program_image::Version>, words: &[(&str, &str)]) -> std::result::Result<Arc<crate::code_rule_dsl::logic::program_image::Version>, String> {
    use crate::code_rule_dsl::logic::program_image::{manager::Pipeline, passes::SetQualifiers};
    let mut p = Pipeline::new("qualify", v.clone());
    p.program(&mut SetQualifiers(words));
    Ok(p.finish()?.0)
}

/// `maintained_cross(cross)`, made once for a prepared program (`Prepared::maintained`) and shared: made again only
/// for another part than the one kept (another digest) or other settings, and then not kept.
fn maintained_cross_of(slot: &std::sync::OnceLock<(MaintainedOf, lower::Lowered)>, cross: &lower::Lowered) -> Result<lower::Lowered> {
    let settings = [std::env::var("RERAIL_SESSION_INLINE").is_ok_and(|v| v == "on"), maintain::levels_wanted()];
    let digest = cross.digest()?;
    if let Some((of, made)) = slot.get() {
        if *of == (digest, settings) {
            return Ok(made.clone());
        }
        return maintained_cross(cross);
    }
    let made = maintained_cross(cross)?;
    Ok(slot.get_or_init(|| ((digest, settings), made)).1.clone())
}

/// The cross-file part as a run that keeps it maintains it (decision 0077): each derived relation stored, not
/// inlined (decision 0099), so the next run has its old rows, unless the program declares it `inline`
/// (`RERAIL_SESSION_INLINE=on` inlines as a full run does); its recursive relations with their levels where wanted
/// (decision 0090). No rule's derivations are kept (decision 0364: counting and its support relations are gone).
/// Made where a run first needs it: the kept inputs' key at the waves' end (decision 0117 step 1), else the cross
/// part.
///
/// The qualifiers are set on the cross part's own heads (`qualify`), on its version, copying no tree.
fn maintained_cross(cross: &lower::Lowered) -> Result<lower::Lowered> {
    let inline_too = std::env::var("RERAIL_SESSION_INLINE").is_ok_and(|v| v == "on");
    // Read from the part's version, not its tree.
    let (declared_inline, heads) = {
        let p = cross.placed()?;
        let f = p.fields()?;
        let declared_inline: BTreeSet<String> = f.decls_by_name().into_iter().map(|(r, _)| r).filter(|r| f.qualified(r, "inline")).map(String::from).collect();
        (declared_inline, p.v.rules().map(|r| p.v.rel_name(r.head().rel()).to_owned()).collect::<BTreeSet<String>>())
    };
    let levels = maintain::levels_wanted();
    let mut stored = cross.clone();
    qualify(&mut stored, |_, qualifiers| {
        for rel in heads {
            if levels {
                qualifiers.entry(rel.clone()).or_default().insert("levels".to_owned());
            }
            if !inline_too && !declared_inline.contains(rel.as_str()) {
                qualifiers.entry(rel).or_default().insert("no_inline".to_owned());
            }
        }
    })?;
    if crate::code_rule_dsl::logic::version_trace::on() {
        crate::code_rule_dsl::logic::version_trace::note_versions("session.maintained", &cross.version, &stored.version);
    }
    Ok(stored)
}

/// An input's change by owner (decision 0117 step 2, `patch_owners`): each changed owner's rows now, the rows the
/// input gained and lost as a set (`r__plus`, `r__minus`), and its digests by owner and digest after it (`None`, the
/// kept ones, when no owner's rows changed). The rows gained and lost are named where they lie (0364 R2): `plus`
/// by their places in `changed`'s rows, one after another (the layered input's appended rows), `minus` by their
/// places in the kept rows. Neither is copied.
struct OwnerPatch {
    changed: Vec<(i64, Vec<Vec<i64>>)>,
    plus: Vec<u32>,
    minus: Vec<u32>,
    docs: Option<retain::DocDigests>,
    digest: u64,
    /// Measured (`RERAIL_STATS_KEEP_TRACE`): the owners read, the kept rows read, the owners found unchanged.
    read: (usize, usize, usize),
}

/// Decision 0117 step 2: input `rel` after an edit, from its kept rows `kept` (each owner's a range, owner by
/// column `g`), the changed files' old entries' rows (`old`, one list a file) and this run's rows of them (`new`).
/// Only the owners those rows name are read. An input is an export (the base relations are read off the corpus
/// batch, decision 0458), which holds each row once: an owner's rows are this run's if a changed file holds it now;
/// else its kept rows while another file holds it (a value document: the keeper's `holders`), else none. A value
/// document's rows are the same in every file that holds it, which is checked.
/// `Err` when the kept rows do not hold what the old entries say (the run then decodes its hits whole).
fn patch_owners(rel: &str, kept: &retain::KeptInput, g: usize, holders: &rustc_hash::FxHashMap<i64, i64>, evaluated: &BTreeSet<i64>, old: &[Vec<Vec<i64>>], new: Vec<Vec<i64>>, cells: &super::Cells) -> std::result::Result<OwnerPatch, String> {
    use rustc_hash::FxHashMap;
    let width = kept.source.columns().len();
    let values = crate::code_rule_dsl::facts::vtab::syntax::HIDDEN_KEYS as i64;
    // A string's rows (`memo::Owner::Text`): its owner is the string, held once by each file whose text holds it, never
    // a document evaluated.
    let text = matches!(memo::owner(rel), memo::Owner::Text(_));
    if width == 0 {
        // A relation of no columns is its rows' count (`write_kept`): none of its rows names an owner.
        return if old.iter().all(Vec::is_empty) && new.is_empty() { Ok(OwnerPatch { changed: Vec::new(), plus: Vec::new(), minus: Vec::new(), docs: None, digest: kept.digest, read: (0, 0, 0) }) } else { Err(format!("{rel}: a relation of no columns changed")) };
    }
    let mut by_old: BTreeMap<i64, Vec<Vec<Vec<i64>>>> = BTreeMap::new();
    for rows in old {
        let mut per: FxHashMap<i64, Vec<Vec<i64>>> = FxHashMap::default();
        for r in rows {
            per.entry(retain::owner_of(r, g)).or_default().push(r.clone());
        }
        for (o, rows) in per {
            by_old.entry(o).or_default().push(rows);
        }
    }
    let mut by_new: BTreeMap<i64, Vec<Vec<i64>>> = BTreeMap::new();
    for r in new {
        by_new.entry(retain::owner_of(&r, g)).or_default().push(r);
    }
    // Every owner a changed file's old rows or this run's name, and every evaluated document the kept rows hold: an
    // evaluated file that has no rows of this input now has its kept ones removed.
    // The evaluated documents the kept rows hold: from the smaller side (an input keeps a few owners, or many).
    let mut touched: BTreeSet<i64> = by_old.keys().chain(by_new.keys()).copied().collect();
    // The kept layout is read only where an owner may be touched: a rule edit evaluates none (Q91).
    if !evaluated.is_empty() {
        let ranges = kept.ranges()?;
        if ranges.len() < evaluated.len() {
            touched.extend(ranges.keys().copied().filter(|o| evaluated.contains(o)));
        } else {
            touched.extend(evaluated.iter().copied().filter(|o| ranges.contains_key(o)));
        }
    }
    if touched.is_empty() {
        return Ok(OwnerPatch { changed: Vec::new(), plus: Vec::new(), minus: Vec::new(), docs: None, digest: kept.digest, read: (0, 0, 0) });
    }
    let (kept_docs, kept_ranges) = (kept.docs()?, kept.ranges()?);
    let all: Vec<usize> = (0..width).collect();
    let mut docs: Option<retain::DocDigests> = None;
    let (mut changed, mut plus, mut minus) = (Vec::new(), Vec::new(), Vec::new());
    let sorted = |mut rows: Vec<Vec<i64>>| {
        rows.sort_unstable();
        rows
    };
    let set = |rows: &[Vec<i64>]| rows.iter().cloned().collect::<BTreeSet<Vec<i64>>>();
    let mut read = (0usize, 0usize, 0usize);
    // The rows of `changed` so far: where the next changed owner's rows start among the appended ones.
    let mut appended = 0u32;
    for o in touched {
        // A real document evaluated this run, which no changed file's old entry names: its rows are this run's
        // (below), so when they are its kept digest's it is unchanged, and its kept rows are not read (Q20: read,
        // copied and sorted for every evaluated owner of every input, 84 G of an edit on ep2).
        // An export's real document is held once: its rows are the set of this run's, so the same test reads their
        // distinct rows.
        let real = o < values && !text && !by_old.contains_key(&o);
        if real {
            let rows = by_new.get(&o).map_or(&[][..], Vec::as_slice);
            let distinct: rustc_hash::FxHashSet<&[i64]> = rows.iter().map(Vec::as_slice).collect();
            let (sum, n) = (distinct.iter().map(|r| cells.row(r)).fold(0u64, u64::wrapping_add), distinct.len());
            if kept_docs.get(&o).copied() == (n > 0).then_some((sum, n as u64)) {
                by_new.remove(&o);
                continue;
            }
        }
        // Read (`RERAIL_STATS_KEEP_TRACE`): an owner found unchanged above reads no kept rows and is not counted.
        read.0 += 1;
        read.1 += kept_ranges.get(&o).map_or(0, |r| r.1 as usize);
        // The owner's kept rows, checked against its digest: the only kept rows the edit reads.
        let was_at = kept_ranges.get(&o).map_or(0, |r| r.0);
        let was: Vec<Vec<i64>> = match kept_ranges.get(&o) {
            Some(&(start, n)) => {
                let mut row = vec![0i64; width];
                let rows: Vec<Vec<i64>> = (start..start + n).map(|r| { kept.source.row_at(r as usize, &all, &mut row); row.clone() }).collect();
                let sum = cells.rows(&rows.concat()).into_iter().fold(0u64, u64::wrapping_add);
                if kept_docs.get(&o) != Some(&(sum, n as u64)) {
                    return Err(format!("{rel}: owner {o}'s kept rows are not its digest's"));
                }
                rows
            }
            None => Vec::new(),
        };
        let old_rows = by_old.remove(&o).unwrap_or_default();
        let new_rows = by_new.remove(&o);
        let now: Vec<Vec<i64>> = {
            let kept_set = set(&was);
            if old_rows.iter().any(|rows| set(rows) != kept_set) {
                return Err(format!("{rel}: owner {o}'s kept rows are not a changed file's old ones"));
            }
            let held = if o >= values { holders.get(&o).copied().unwrap_or(0) } else { i64::from(new_rows.is_some()) };
            match (held > 0, new_rows) {
                (false, _) => Vec::new(),
                (true, Some(rows)) => {
                    let rows: Vec<Vec<i64>> = set(&rows).into_iter().collect();
                    // A value document's rows are its key's (its text, parsed): the same in every file that
                    // holds it, so rows kept of it that differ are another run's mistake.
                    if o >= values && !kept_set.is_empty() && rows.iter().ne(kept_set.iter()) {
                        return Err(format!("{rel}: value document {o}'s rows differ between the files that hold it"));
                    }
                    rows
                }
                (true, None) if !was.is_empty() => sorted(was.clone()),
                (true, None) => return Err(format!("{rel}: value document {o} is held, but no rows of it are")),
            }
        };
        if now == sorted(was.clone()) {
            read.2 += 1;
            continue;
        }
        let (a, b) = (set(&was), set(&now));
        // Each row once, at its first place.
        let mut once: rustc_hash::FxHashSet<&[i64]> = Default::default();
        minus.extend(was.iter().enumerate().filter(|(_, r)| !b.contains(*r) && once.insert(r.as_slice())).map(|(i, _)| was_at + i as u32));
        once.clear();
        plus.extend(now.iter().enumerate().filter(|(_, r)| !a.contains(*r) && once.insert(r.as_slice())).map(|(i, _)| appended + i as u32));
        appended += now.len() as u32;
        let d = docs.get_or_insert_with(|| (**kept_docs).clone());
        if now.is_empty() {
            d.remove(&o);
        } else {
            d.insert(o, (cells.rows(&now.concat()).into_iter().fold(0u64, u64::wrapping_add), now.len() as u64));
        }
        changed.push((o, now));
    }
    let digest = docs.as_ref().map_or(kept.digest, retain::digest_of_docs);
    Ok(OwnerPatch { changed, plus, minus, docs, digest, read })
}

/// A measurement made once per mix (Q88): the chunk that made it (its place in the filing order), the rows it was
/// made over, and the measurement.
#[derive(Clone)]
struct Once<T>(usize, usize, T);

/// Keeps `once` for `key` where no earlier chunk's is kept: the earliest chunk's stands, whichever finished first.
fn once_keep<K: std::hash::Hash + Eq, T>(map: &mut HashMap<K, Once<T>>, key: K, once: Once<T>) {
    match map.get(&key) {
        Some(o) if o.0 <= once.0 => {}
        _ => {
            map.insert(key, once);
        }
    }
}
