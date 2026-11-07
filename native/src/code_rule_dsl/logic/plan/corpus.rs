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
use crate::code_rule_dsl::model::FileInput;
use crate::code_rule_dsl::typing::cross_rounds;
pub(in crate::code_rule_dsl) mod guests;
mod hiding;
pub(in crate::code_rule_dsl) use hiding::qualified;
pub(in crate::code_rule_dsl) mod memo;
mod bridge;
mod inputs;
pub(in crate::code_rule_dsl) use inputs::parameters as input_parameters;
mod world;
mod library;
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
    /// The per-file memo: files read from it, and chunks whose rows it could not keep (a row naming no document).
    pub(crate) memo: (usize, usize),
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
    let parse = |text: &str| crate::code_rule_dsl::logic::rr::parse(&if extra.is_empty() { text.to_owned() } else { format!("{text}\n{extra}") });
    let mut parsed = programs.iter().map(|text| parse(text)).collect::<std::result::Result<Vec<_>, _>>()?;
    // A run that makes guests bounds each program's rules to the documents it shows (decision 0058 §1); a run that
    // makes none is left as written.
    if parsed.iter().any(|p| p.rules.iter().any(|r| guests::REQUESTS.contains(&r.head.rel.as_str()))) {
        for p in &mut parsed {
            let bound = hiding::bound(p);
            hiding::hide(p, bound);
        }
    }
    if parsed.len() == 1 {
        return Ok(parsed.pop().expect("one program"));
    }
    let axis = crate::code_rule_dsl::logic::front::AXIS;
    let mut all = Program::default();
    let mut sets: BTreeMap<Vec<String>, String> = BTreeMap::new();
    let core = crate::code_rule_dsl::logic::rr::core_names();
    for (i, mut prog) in parsed.into_iter().enumerate() {
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
            // `core.rr`'s relations are the engine's, one per run however many programs include it (decision 0047);
            // a repository file a policy reads (`.input r(IO=…)`) is its own.
            .filter(|r| (!prog.inputs.contains(r) || file_inputs.contains(*r)) && !shared_outputs.contains(&r.as_str()) && !core.contains(*r) && !bridge::is_bridged(r) && !guests::SHARED.contains(&r.as_str()))
            .cloned()
            .collect();
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
        for mut setting in prog.settings {
            if matches!(setting.0.as_str(), "input" | "output") && let Some((rel, params)) = setting.1.split_once('(') {
                setting.1 = format!("{}({params}", rename(rel));
            }
            if !all.settings.contains(&setting) {
                all.settings.push(setting);
            }
        }
    }
    Ok(all)
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

    fn new(store: &'a ContentStore, plan: [u8; 32]) -> std::result::Result<Self, crate::content_store::StoreError> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let owner = format!("corpus-run:{}:{}:{}", std::process::id(), now.as_nanos(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        let lease = now.as_secs() as i64 + Self::LEASE_SECS;
        store.hold_corpus_plan(plan, &owner, lease)?;
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

fn record_run(store: &ContentStore, root: &Path, sources: &[crate::content_store::SourceId], program: &[u8; 32], relations: &[(String, String)], reading: Option<crate::content_store::CollectionLease>) -> Result<Option<std::thread::JoinHandle<(usize, std::result::Result<String, String>)>>> {
    memo::off();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    store
        .record_corpus_run(&root.to_string_lossy(), sources, crate::code_rule_dsl::workspace::grammar_version(), i64::from(crate::code_rule_dsl::tape::TAPE_FORMAT_VERSION), memo::plan_digest(program), now)
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

/// The store the corpus runner keeps tapes in: `RERAIL_TAPE_CACHE`, else one in the temporary
/// directory; `RERAIL_TAPE_CACHE=off` keeps none.
fn tape_cache() -> std::result::Result<Option<ContentStore>, crate::content_store::StoreError> {
    let path = match std::env::var("RERAIL_TAPE_CACHE") {
        Ok(v) if v == "off" => return Ok(None),
        Ok(v) => std::path::PathBuf::from(v),
        Err(_) => std::env::temp_dir().join("rerail-tape-cache.sqlite"),
    };
    ContentStore::open(&path).map(Some)
}

/// `run` with typing inputs and outputs (`typing`: declarations, dependency summaries, summarizing).
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_typed(programs: &[&str], root: &Path, t_facts: Option<&Path>, out: &Path, modes: &str, binding: Binding, chunk_nodes: usize, typing: &Typing) -> Result<Report> {
    run_with_options(programs, root, t_facts, out, modes, binding, chunk_nodes, typing, &serde_json::Value::Null)
}

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
    // A program that reads the file system or loads documents runs in rounds of its requests first (`world`).
    let world_round = options.get(world::OPTION).and_then(|v| v.as_str()).map(str::to_owned);
    // The run's settings, as `env` facts (t21 E4): `<root>/rerail.env`, then the `env` option's `key=value`s (the
    // command line's `--env`). A round of `discover` has them already.
    let with_settings: Vec<String>;
    let programs: &[&str] = if world_round.is_none() {
        let flags: Vec<String> = options.get("env").and_then(|v| v.as_array()).into_iter().flatten().filter_map(|v| v.as_str().map(str::to_owned)).collect();
        let settings = crate::code_rule_dsl::tools::settings(root, &flags).map_err(Error::Plan)?;
        with_settings = programs.iter().map(|p| crate::code_rule_dsl::tools::with_settings(p, &settings)).collect();
        &with_settings.iter().map(String::as_str).collect::<Vec<_>>()
    } else {
        programs
    };
    if world_round.is_none() {
        let prog = combine(programs, "")?;
        if world::wanted(&prog) {
            return discover(programs, &prog, root, t_facts, out, modes, binding, chunk_nodes, typing, options);
        }
    }
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
    world::add_facts(&mut prog, options.get("worldFacts"));
    // The library stage (decision 0059): the loaded documents alone, the program told so if it asks.
    let library_stage = world_round.as_deref() == Some("library");
    if library_stage && prog.decls.contains_key(library::STAGE) {
        prog.facts.entry(library::STAGE.to_owned()).or_default().push(vec![front::Value::Str("library".to_owned())]);
    }
    if world_round.as_deref() == Some("discover") {
        prog.outputs = world::requests(&prog);
    }
    if let Some(summary) = options.get("worldSummary").and_then(|v| v.as_str()) {
        crate::code_rule_dsl::logic::profile::decision("file system and loaded documents (t21)", || summary.to_owned());
    }
    // Typing programs' answers the rule reads (`bridge`): each program runs first, and its rows are the rule's facts.
    let (bridged, typing_runs) = bridge::facts(&prog, root, out, modes, binding, chunk_nodes, typing)?;
    let add_bridged = |prog: &mut front::Program| {
        for (rel, rows) in &bridged {
            prog.facts.entry(rel.clone()).or_default().extend(rows.iter().cloned());
        }
    };
    add_bridged(&mut prog);
    // Plans are kept across this run's chunks only (decision 0038).
    let _plans = super::KeepPlans::start();
    // And the values derived from them (`logic::derived`): each chunk's join specs.
    let _derived = crate::code_rule_dsl::logic::derived::Keep::start();
    let t = t_facts.map(|t_facts| Connection::open_with_flags(t_facts, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)).transpose()?;
    let (docs, first, rust): (Vec<(i64, String)>, HashMap<i64, i64>, bool) = match &t {
        Some(t) => {
            let docs: Vec<(i64, String)> = t.prepare("SELECT d, path FROM doc ORDER BY d")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
            let first = t.prepare("SELECT d, min(n) FROM node GROUP BY d")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
            let rust = docs.first().is_some_and(|(_, p)| p.ends_with(".rs"));
            (docs, first, rust)
        }
        None => {
            let (files, rust) = if let Some(supplied) = options.get("inputFiles") {
                let supplied = supplied.as_array().ok_or_else(|| Error::Plan("inputFiles must be a list of relative document paths".to_owned()))?;
                let files = supplied.iter().map(|value| {
                    let path = value.as_str().ok_or_else(|| Error::Plan("inputFiles must contain strings".to_owned()))?;
                    if path.is_empty() || Path::new(path).components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
                        return Err(Error::Plan(format!("inputFiles: invalid relative path {path:?}")));
                    }
                    Ok(path.to_owned())
                }).collect::<Result<Vec<_>>>()?;
                let language = prog.settings.iter().find(|(key, _)| key == "language").map(|(_, value)| value.trim().trim_matches('"'));
                let rust = match language {
                    Some("python") => false,
                    Some("rust") => true,
                    Some(other) => return Err(Error::Plan(format!("explicit input batch: unsupported corpus language {other:?}"))),
                    None => files.iter().filter(|p| p.ends_with(".rs")).count() > files.len() / 2,
                };
                (files, rust)
            } else {
                let (python, rs) = corpus_files(root)?;
                let rust = typing.rust.unwrap_or(rs.len() > python.len());
                (if rust { rs } else { python }, rust)
            };
            // Documents a rule loaded (`world`), after the corpus's own; a library stage's are only those.
            let mut files = if library_stage { Vec::new() } else { files };
            for (path, _) in world::library_files(options) {
                if !files.contains(&path) {
                    files.push(path);
                }
            }
            // A document's key is a function of its path (decision 0052), so a memoised row's node ids stay valid.
            let keys = memo::doc_keys(&files);
            (keys.into_iter().zip(files).collect(), HashMap::new(), rust)
        }
    };
    let paths: HashMap<i64, String> = docs.iter().cloned().collect();
    // A loaded document's module, from its load; and the loaded documents' keys, whose rows are not written.
    let named: HashMap<String, Arc<str>> = world::library_files(options).into_iter().map(|(p, m)| (p, Arc::from(m))).collect();
    let library_keys: std::collections::HashSet<i64> = docs.iter().filter(|(_, p)| named.contains_key(p)).map(|(d, _)| *d).collect();
    // Declarations and dependency summaries, as `declared_ret` facts (`corpus::typing`).
    let mut typed = TypingReport::default();
    let mut declared = BTreeMap::new();
    if prog.decls.contains_key("declared_ret") {
        let files: Vec<String> = docs.iter().map(|(_, p)| p.clone()).collect();
        let (facts, d) = typing::facts(typing, root, &files, &mut typed)?;
        if !facts.is_empty() {
            prog = combine(programs, &facts)?;
            texts = texts_digest(programs, &facts);
            world::add_facts(&mut prog, options.get("worldFacts"));
            if library_stage && prog.decls.contains_key(library::STAGE) {
                prog.facts.entry(library::STAGE.to_owned()).or_default().push(vec![front::Value::Str("library".to_owned())]);
            }
            add_bridged(&mut prog);
        }
        declared = d;
    }
    // Typing may rebuild the combined program. Publish external facts after that rebuild.
    external_inputs(&mut prog, root)?;
    // Guest requests (decision 0055) are outputs, so the file-local part keeps the rules that make them; a round of
    // `world` derives only its own requests.
    let discovering = world_round.as_deref() == Some("discover");
    for rel in guests::LIBRARY.iter().copied().chain(guests::DEMANDS).filter(|_| !discovering) {
        if prog.rules.iter().any(|r| r.head.rel == rel) && !prog.outputs.iter().any(|o| o == rel) {
            prog.outputs.push(rel.to_owned());
        }
    }
    let (label, dialect) = if rust { ("rust", "rust") } else { ("python", "python") };
    let dialect = crate::code_rule_dsl::language::dialect_by_label(dialect).ok_or_else(|| Error::Plan(format!("no dialect {label}")))?;
    if !dialect.built() {
        return Err(Error::Plan(format!("{label} is not in this build (its `lang-{label}` feature is off)")));
    }
    std::fs::create_dir_all(out).map_err(|e| Error::Plan(e.to_string()))?;
    let path = out.join("store.sqlite");
    let cross_path = out.join("cross.sqlite");
    for p in [&path, &cross_path] {
        if p.exists() {
            std::fs::remove_file(p).map_err(|e| Error::Plan(e.to_string()))?;
        }
    }
    // The persistent store: tapes kept across runs, keyed by content (`RERAIL_TAPE_CACHE` names it, `off` parses
    // every file), and the run's atoms and each file's memoised rows (decision 0052): atoms are never given out again,
    // so a memoised row's symbols mean the same in every run. The run's SQL works in `store.sqlite`, on the same
    // atoms. Without the store, or reproducing T, the atoms are the run's own.
    // Without the `tape-cache` feature (the browser build) there is no store to keep: `tapes` is `None` where the
    // compiler can see it, so the memo, the run record and their zstd frames are not linked.
    let tapes = if cfg!(feature = "tape-cache") { tape_cache().map_err(|e| Error::Plan(e.to_string()))? } else { None };
    // Hold before any source/stat lookup, through parsing, memo publication and the run manifest.
    let reading = tapes.as_ref().map(|cache| cache.hold_collection()).transpose().map_err(|e| Error::Plan(e.to_string()))?;
    let own_store;
    let store: &ContentStore = match &tapes {
        Some(cache) if t.is_none() => {
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
    let prepared_key: [u8; 32] = {
        use sha2::Digest as _;
        let mut hash = sha2::Sha256::new();
        hash.update(texts);
        hash.update(format!("{label}\0{library_stage}\0{:?}\0{:?}\0{:?}", prog.facts, prog.inputs, prog.outputs).as_bytes());
        hash.finalize().into()
    };
    let kept = prepared_cache().lock().expect("the prepared programs").get(&prepared_key).cloned();
    let kept = kept.filter(|p| {
        // The lowered program's atoms are the store's: the same strings must still be the same atoms.
        p.interned.iter().all(|(strings, atoms)| {
            let strings: Vec<&str> = strings.iter().map(String::as_str).collect();
            store.intern_atoms(&strings).is_ok_and(|got| got.iter().map(|a| u32::from(*a)).eq(atoms.iter().copied()))
        })
    });
    let reused = kept.is_some();
    let prepared = match kept {
        Some(p) => {
            crate::code_rule_dsl::prof::count("corpus.prepared_reused", 1);
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
            // Not in a library stage: its rows are replayed for whatever a workspace's rules ask (decision 0059).
            let (mut prog, demanded) = if library_stage { (prog, Vec::new()) } else { crate::code_rule_dsl::logic::magic::demand_negated_tests(prog) };
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
            let sets = lowered.transduce_nearest();
            // The file-local part of each cross-file rule runs in the chunks, which export its rows, not their base relations.
            let pushed = crate::code_rule_dsl::typing::file_local::push_local_joins(&mut lowered.program);
            // Every relation the chunks hand on names its document per row, so a file's rows can be memoised (decision 0052).
            let located = crate::code_rule_dsl::typing::file_local::locate(&mut lowered.program);
            // The joins pushed are made per rule, so two programs' copies of one library rule made two: merged again, and so
            // are the relations over them, before the program splits.
            let merged = crate::code_rule_dsl::logic::simplify::merge_identical(&mut lowered.program);
            let split = crate::code_rule_dsl::typing::file_local::programs(&lowered.program);
            let prepared = Prepared { prog, vocabulary: Arc::new(vocabulary), lowered, sets, demanded, pushed, located, merged, split, interned };
            let mut kept = prepared_cache().lock().expect("the prepared programs");
            if kept.len() >= PREPARED_KEPT {
                kept.clear();
            }
            kept.insert(prepared_key, prepared.clone());
            prepared
        }
    };
    crate::code_rule_dsl::logic::profile::decision("program prepared", || if reused { "reused: this process prepared the same program and additions before".to_owned() } else { "prepared".to_owned() });
    let Prepared { prog, vocabulary, lowered, sets, demanded, pushed, located, merged: (remerged, reshared), mut split, interned: _ } = prepared;
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
    // The library's rows (decision 0059): bound for the cross-file part, which then derives no row of theirs about
    // a library node. Only under the document keys they were made with.
    let mut library_inputs: Vec<(String, usize, Vec<i64>)> = Vec::new();
    // The stored rows' digest: the run's manifest names them, so collection keeps them (decision 0053).
    let mut library_digest: Option<[u8; 32]> = None;
    if !library_stage && let Some(file) = options.get(library::OPTION).and_then(|v| v.as_str()) {
        let bytes = std::fs::read(file).ok();
        library_digest = bytes.as_ref().map(|b| <sha2::Sha256 as sha2::Digest>::digest(b).into());
        let rows = bytes.and_then(|b| library::Rows::decode(&b));
        let keys: HashMap<&str, i64> = docs.iter().map(|(d, p)| (p.as_str(), *d)).collect();
        match rows {
            Some(rows) if rows.keys.iter().all(|(p, k)| keys.get(p.as_str()) == Some(k)) => {
                library_inputs = library::replay(&mut split.cross, &rows, &library::open_relations(&prog));
                let n: usize = library_inputs.iter().map(|(_, w, v)| v.len() / (*w).max(1)).sum();
                crate::code_rule_dsl::logic::profile::decision("library rows (decision 0059)", || {
                    format!("{} relations' rows about {} library documents replayed ({n} rows); their rules derive no row about a library node", library_inputs.len() - 1, rows.keys.len())
                });
            }
            Some(_) => crate::code_rule_dsl::logic::profile::decision("library rows (decision 0059)", || "not replayed: a library document's key differs from the stage's".to_owned()),
            None => crate::code_rule_dsl::logic::profile::decision("library rows (decision 0059)", || format!("not replayed: {file} is not a library stage's rows")),
        }
    }
    let unlocated: Vec<&String> = split.exports.iter().chain(split.local.outputs.iter()).filter(|r| split.locations.get(*r).is_none_or(Vec::is_empty)).collect();
    let memo_program = memo::program_digest(&split.local);
    // `declared_ret` rows come from outside the memo's key (`corpus::typing`): a program whose rules still read them
    // after pruning is not memoised (a round of `world` does not read them).
    let reads_declared = prog.rules.iter().flat_map(|r| &r.body).filter_map(|l| l.atom()).any(|a| a.rel == "declared_ret");
    let memoised = tapes.is_some() && t.is_none() && unlocated.is_empty() && !reads_declared && std::env::var("RERAIL_CORPUS_MEMO").map_or(true, |v| v != "off");
    crate::code_rule_dsl::logic::profile::decision("per-file memo", || match (&located, memoised) {
        (_, true) => format!("each file's file-local rows are memoised under its content, path and the run's plan; {} relations given a document column", located.as_ref().map_or(0, Vec::len)),
        (Err(blocked), _) => format!("off: rows of {blocked:?} name no document"),
        _ => format!("off ({})", if tapes.is_none() { "no persistent store" } else if t.is_some() { "T's facts" } else if unlocated.is_empty() { "typing, or RERAIL_CORPUS_MEMO=off" } else { "a relation without a location column" }),
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
    crate::code_rule_dsl::logic::profile::decision("nearest ancestors", || {
        let names: Vec<&str> = sets.iter().map(|(n, _)| n.as_str()).collect();
        format!("{} sets made by the transducer's file-local FIX, bound {binding:?}: {}", sets.len(), names.join(", "))
    });
    let (local, cross) = (part(split.local), part(split.cross));
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
    let mut measured_once: HashMap<(String, String), (usize, Vec<(String, i64, i64, i64, i64)>)> = HashMap::new();
    db.execute("ATTACH ?1 AS x", [cross_path.to_string_lossy()])?;
    // Only this connection opens `cross.sqlite`: its journal can stay in memory.
    db.execute_batch("PRAGMA x.journal_mode=MEMORY; PRAGMA x.synchronous=OFF")?;
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
    // A cross-file part that reads base relations takes their rows from the chunks it evaluates: a memoised file has
    // none to give, so such a run is not memoised (all base reads are pushed into the chunks when they can be).
    let memoised = memoised && stored.is_empty();
    // The run holds its memo plan and the sources it reads until its manifest names them (O95): a collector running
    // meanwhile would otherwise take the rows this run writes, or the sources they and its tapes are of.
    let _held = if memoised { Some(HeldPlan::new(store, memo::plan_digest(&memo_program)).map_err(|e| Error::Plan(e.to_string()))?) } else { None };
    if !stored.is_empty() && tapes.is_some() && t.is_none() {
        crate::code_rule_dsl::logic::profile::decision("per-file memo held back", || format!("the cross-file part reads base relations {:?}: files are evaluated", stored.iter().map(|(r, _)| r).collect::<Vec<_>>()));
    }
    // One transaction for every export's table, not a commit each.
    let mut tables = String::from("BEGIN;");
    for rel in &split.exports {
        let width = lowered.program.decls[rel].len();
        let cols: Vec<String> = (0..width).map(|i| format!("c{i}")).collect();
        tables.push_str(&format!("CREATE TABLE x.{rel}({}, PRIMARY KEY ({})) WITHOUT ROWID;", cols.iter().map(|c| format!("{c} INTEGER")).collect::<Vec<_>>().join(", "), cols.join(", ")));
    }
    tables.push_str("COMMIT;");
    db.execute_batch(&tables)?;
    phase("stats", statements.count());
    statements.enter("parse");
    let modules = Modules::default();
    for (rel, source, _) in &repository {
        modules.bind(&db, rel, Arc::clone(source))?;
    }
    let repository_stats = |stats: &mut super::Stats| {
        for (rel, _, measured) in &repository {
            stats.measured(rel, measured.iter().map(|(c, rows, ndv, sq, sqd)| (c.clone(), [*rows, *ndv, *sq, *sqd])), []);
        }
    };
    let mut base_rows: BTreeMap<String, (Vec<String>, Vec<Option<Vec<i64>>>)> = BTreeMap::new();
    let mut bound = BTreeSet::new();
    let mut totals = Totals::default();
    let (mut nodes, mut native, mut chunks) = (0, BTreeSet::new(), 0);
    let (mut memo_hits, mut memo_unattributed) = (0usize, 0usize);
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
    type Chunk = (usize, Vec<(String, Arc<dyn crate::code_rule_dsl::facts::vtab::Source>)>, Vec<(i64, String, Option<crate::content_store::SourceId>, u64)>, Vec<memo::Entry>, Vec<syntax::Document>);
    #[cfg(not(target_family = "wasm"))]
    let (send, chunks_in) = std::sync::mpsc::sync_channel::<Result<Chunk>>(1);
    // wasm32 has no threads: the builder runs to its end first (`spawn_chunks`), so every chunk waits here.
    #[cfg(target_family = "wasm")]
    let (send, chunks_in) = std::sync::mpsc::channel::<Result<Chunk>>();
    // What each side of the channel is doing, for the other's waits (`profile::wait`).
    let (worker_doing, main_doing) = (crate::code_rule_dsl::logic::profile::Activity::default(), crate::code_rule_dsl::logic::profile::Activity::default());
    worker_doing.set("parse");
    main_doing.set("setup");
    let (mut parse_cpu, mut batch_cpu) = ((0.0, 0.0), (0.0, 0.0));
    // A run over the persistent store records which sources it read, its manifest (decision 0053).
    let recording = tapes.is_some() && t.is_none();
    // The stats the run records are as of its start: an entry is trusted only for a file last changed before then.
    let started_ns = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as i64);
    let canonical_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf()).to_string_lossy().into_owned();
    let stat_index = if memoised || recording {
        store.corpus_stats(&canonical_root).map_err(|e| Error::Plan(e.to_string()))?
    } else {
        crate::content_store::StatIndex::default()
    };
    let stat_index = &stat_index;
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
        let mut at = docs.iter().peekable();
        let mut t = thread_usage();
        // A file's memo entry when the run is memoised and the memo has it, else its document, built.
        let mut stats_read = Vec::new();
        let mut build = |d: &i64, path: &String| -> Result<(Option<crate::content_store::SourceId>, std::result::Result<memo::Entry, syntax::Document>)> {
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
            if let (true, Some(id)) = (memoised, known) {
                let _lookup = crate::code_rule_dsl::logic::profile::span(|| "memo lookup".into());
                if let Some(entry) = memo::lookup(store, id, world::store_path(path), *d, &memo_program).map_err(|e| Error::Plan(e.to_string()))? {
                    return Ok((known, Ok(entry)));
                }
            }
            let read = crate::code_rule_dsl::logic::profile::span(|| "read".into());
            let source = std::fs::read_to_string(root.join(path)).map_err(|e| Error::Plan(format!("{path}: {e}")))?;
            crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, source.len() as u64);
            drop(read);
            let mut source_id = None;
            if memoised || recording {
                // Read, the file is hashed whatever the index said: it may have changed since its stat.
                let _lookup = crate::code_rule_dsl::logic::profile::span(|| "memo lookup".into());
                crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, source.len() as u64);
                let id = memo::source(store, &source).map_err(|e| Error::Plan(e.to_string()))?;
                source_id = Some(id);
                if let Some(stat) = stat.filter(|_| known != Some(id)) {
                    stats_read.push((world::store_path(path).to_owned(), stat, id));
                }
            }
            if let (true, Some(id), false) = (memoised, source_id, known == source_id) {
                let _lookup = crate::code_rule_dsl::logic::profile::span(|| "memo lookup".into());
                if let Some(entry) = memo::lookup(store, id, world::store_path(path), *d, &memo_program).map_err(|e| Error::Plan(e.to_string()))? {
                    return Ok((source_id, Ok(entry)));
                }
            }
            let _build = crate::code_rule_dsl::logic::profile::span(|| "tree-sitter and facts".into());
            // Its own work: the file's input made from its source (the build's parts charge their own).
            crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, source.len() as u64);
            let file = FileInput::new(path.clone(), label.to_owned(), Arc::new(source), None, None, "core").map_err(|e| Error::Plan(format!("{path}: {e:?}")))?;
            let built = match &tapes {
                // The closure sees `&tapes`, not that it is `None` without `tape-cache`: said here so the store's
                // tape and source frames are not linked.
                // A source collected since it was read is stored again with its tape (O95), under the id it has now.
                Some(cache) if cfg!(feature = "tape-cache") => core::build_cached_source(&file, cache, source_id).map(|(built, id)| {
                    if source_id.is_some() {
                        source_id = id;
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
            Ok((source_id, Err(syntax::Document { key: *d as u32, path: path.clone(), core: format!("/c#core:{path}"), digest, facts, guest: None, module: named.get(path).cloned() })))
        };
        while let Some((d, path)) = at.next() {
            match build(d, path) {
                Ok((source_id, Ok(entry))) => {
                    read_sources.extend(source_id);
                    hits.push(entry);
                }
                Ok((source_id, Err(doc))) => {
                    read_sources.extend(source_id);
                    pending_nodes += doc.facts.len();
                    pending_docs.push((*d, path.clone(), source_id, doc.facts.len() as u64));
                    pending.push(doc);
                }
                Err(e) => {
                    let _ = send.send(Err(e));
                    break;
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
                    if crate::code_rule_dsl::logic::profile::wait("the main thread (a channel of one chunk)", &main_doing, || send.send(hand)).is_err() {
                        break;
                    }
                }
                continue;
            }
            if pending_nodes < chunk_nodes && at.peek().is_some() {
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
            if crate::code_rule_dsl::logic::profile::wait("the main thread (a channel of one chunk)", &main_doing, || send.send(hand)).is_err() {
                break;
            }
            t = thread_usage();
        }
        drop(send);
        worker_doing.set("done");
        (parse, batch, read_sources, stats_read)
    });
    loop {
        main_doing.set("wait");
        let Ok(built) = crate::code_rule_dsl::logic::profile::wait("the worker's next chunk", &worker_doing, || chunks_in.recv()) else { break };
        let (chunk_nodes_built, sources, chunk_docs, hits, documents) = built?;
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
        // Memo hits: their rows go to `x` and their lines to the outputs as an evaluated chunk's would.
        if !hits.is_empty() {
            let _replay = crate::code_rule_dsl::logic::profile::span(|| "memo replay".into());
            memo_hits += hits.len();
            db.execute_batch("BEGIN")?;
            for entry in hits {
                nodes += entry.nodes as usize;
                for (rel, (width, values)) in entry.rows {
                    if values.is_empty() || !split.exports.contains(&rel) {
                        continue;
                    }
                    let types: Vec<&str> = lowered.program.decls[&rel].iter().map(|(_, t)| t.as_str()).collect();
                    let marks = vec!["?"; width].join(", ");
                    let mut insert = db.prepare_cached(&format!("INSERT OR IGNORE INTO x.{rel} VALUES ({marks})"))?;
                    let mut inserted = 0;
                    for row in values.chunks(width) {
                        inserted += insert.execute(rusqlite::params_from_iter(row.iter()))?;
                    }
                    let columns = (0..width).map(|c| Some(values.iter().skip(c).step_by(width).copied().collect())).collect();
                    let rows = crate::code_rule_dsl::logic::native::component::columns_source((0..width).map(|c| format!("c{c}")).collect(), columns);
                    totals.add_rows(&rel, inserted, rows.as_ref(), &types);
                }
                for (rel, (columns, lines)) in entry.lines {
                    outputs.entry(rel.clone()).or_default().extend(lines);
                    node_columns.insert(rel, columns);
                }
            }
            db.execute_batch("COMMIT")?;
        }
        if sources.is_empty() {
            phase("local", statements.count());
            statements.enter("wait");
            continue;
        }
        nodes += chunk_nodes_built;
        chunks += 1;
        let mut stats = t_stats.clone();
        repository_stats(&mut stats);
        let mix = chunk_mix(chunk_docs.iter().map(|(_, path, _, n)| (path.as_str(), *n as usize)));
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
            own_totals.add(&chunk);
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
            if split.base.contains(&rel) && (own || !t_stats.knows(&rel) || crate::code_rule_dsl::logic::front::is_transduced(&rel) || rel == syntax::WITHIN || rel == syntax::SPAN_WITHIN || rel.starts_with(crate::code_rule_dsl::logic::strings::RELATION)) {
                totals.add(&rel, source.estimated_len(), &measured);
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
        // Per document of the chunk, what it hands on (`memo::Entry`), when the run is memoised. A row is its
        // document's by its first location column.
        let mut entries: HashMap<i64, memo::Entry> = if memoised { chunk_docs.iter().map(|(d, _, _, n)| (*d, memo::Entry { nodes: *n, ..memo::Entry::default() })).collect() } else { HashMap::new() };
        let mut attributable = memoised;
        if memoised {
            for (rel, lines) in got.outputs.iter().filter(|(rel, _)| outputs_of(rel)) {
                let at = split.locations.get(rel).and_then(|l| l.first().copied());
                for line in lines {
                    let files = at.and_then(|c| line.split('\t').nth(c)).and_then(|v| v.parse::<i64>().ok()).map(owners).unwrap_or_default();
                    if files.is_empty() || !files.iter().all(|d| entries.contains_key(d)) {
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
                attributable = false;
                return;
            };
            let width = rows.columns().len();
            for r in 0..rows.len() {
                let files = owners(rows.value(r, at));
                if !files.iter().all(|d| entries.contains_key(d)) {
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
        for rel in &split.exports {
            // Rows another chunk wrote too (a file-local relation without a location column) are kept once. A kept
            // export goes from the engine to `x` in one statement and is measured in place; one a SQL stratum
            // derived is in its table.
            let types: Vec<&str> = lowered.program.decls[rel].iter().map(|(_, t)| t.as_str()).collect();
            if kept.contains(rel) {
                let cols = (0..types.len()).map(|i| format!("c{i}")).collect::<Vec<_>>().join(", ");
                let inserted = db.execute(&format!("INSERT OR IGNORE INTO x.{rel} SELECT {cols} FROM set_derived WHERE rel = ?1"), [rel])?;
                let rows = crate::code_rule_dsl::logic::native::component::derived(&db, rel).ok_or_else(|| Error::Plan(format!("{rel} is not held")))?;
                totals.add_rows(rel, inserted, rows.as_ref(), &types);
                attribute(rel, rows.as_ref());
            } else {
                let inserted = db.execute(&format!("INSERT OR IGNORE INTO x.{rel} SELECT * FROM temp.{rel}"), [])?;
                let rows = Rows::read(&db, &format!("temp.{rel}"))?;
                totals.add_rows(rel, inserted, &rows, &types);
                attribute(rel, &rows);
            }
        }
        db.execute_batch("COMMIT")?;
        if memoised && attributable {
            let _remember = crate::code_rule_dsl::logic::profile::span(|| "memo remember".into());
            for (d, path, source_id, _) in &chunk_docs {
                let (Some(id), Some(entry)) = (source_id, entries.get(d)) else { continue };
                memo::remember(store, *id, world::store_path(path), *d, &memo_program, entry).map_err(|e| Error::Plan(format!("{path}: memo publication for source {id:?}: {e}")))?;
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
    }
    main_doing.set("done");
    crate::code_rule_dsl::logic::profile::wait("the worker's end", &worker_doing, || worker.join()).map_err(|_| Error::Plan("the chunk builder panicked".to_owned()))
    })?;
    (parse_cpu.0, parse_cpu.1, batch_cpu.0, batch_cpu.1) = (worker_cpu.0.0, worker_cpu.0.1, worker_cpu.1.0, worker_cpu.1.1);
    // The run's sources, with the library stage's rows (decision 0059), for its manifest (decision 0053).
    let mut run_sources = worker_cpu.2.clone();
    if let Some(digest) = library_digest
        && let Ok(Some(id)) = store.source_by_hash(crate::content_store::HashKey::from_sha256_digest(digest))
    {
        run_sources.push(id);
    }
    if recording {
        let _stats = crate::code_rule_dsl::logic::profile::span(|| "record stats".into());
        store.record_corpus_stats(&canonical_root, &worker_cpu.3, started_ns).map_err(|e| Error::Plan(e.to_string()))?;
    }
    // Publish all source references before releasing the lease and starting collection (decision 0053).
    // The memo's relations, named and typed, so a SQL session can read the run's frames (docs/sql-access).
    let relations: Vec<(String, String)> = split
        .exports
        .iter()
        .filter_map(|rel| lowered.program.decls.get(rel).map(|cols| (rel.clone(), serde_json::to_string(cols).unwrap_or_default())))
        .collect();
    let collecting = if recording { record_run(store, root, &run_sources, &memo_program, &relations, reading)? } else { drop(reading); None };
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
    // The cross-file part's statistics: T's for the base relations, the chunks' totals for the rest.
    let mut cross_stats = t_stats;
    if own {
        own_totals.apply(&mut cross_stats);
    }
    totals.apply(&mut cross_stats);
    repository_stats(&mut cross_stats);
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
        modules.bind(&db, &rel, crate::code_rule_dsl::logic::native::component::columns_source(columns, values))?;
    }
    for rel in &split.exports {
        let source = crate::code_rule_dsl::logic::native::component::table_source(&db, "x", rel).ok_or_else(|| Error::Plan(format!("x.{rel} is not a table of integers")))?;
        modules.bind(&db, rel, source)?;
    }
    // The library's rows (decision 0059), measured as inputs.
    let mut library_totals = Totals::default();
    for (rel, width, values) in std::mem::take(&mut library_inputs) {
        let types: Vec<String> = cross.program.decls[&rel].iter().map(|(_, t)| t.clone()).collect();
        let columns = (0..width).map(|i| format!("c{i}")).collect();
        let values = (0..width).map(|c| Some(values.iter().skip(c).step_by(width).copied().collect())).collect();
        let source = crate::code_rule_dsl::logic::native::component::columns_source(columns, values);
        library_totals.add_rows(&rel, source.len(), source.as_ref(), &types.iter().map(String::as_str).collect::<Vec<_>>());
        modules.bind(&db, &rel, source)?;
    }
    library_totals.apply(&mut cross_stats);
    phase("stats", statements.count());
    statements.enter("cross");
    let mut rounds = cross_rounds::Rounds::default();
    // `returns`, held for the typing outputs (`corpus::typing`).
    let mut wanted: BTreeSet<String> = cross.program.decls.contains_key("returns").then(|| "returns".to_owned()).into_iter().filter(|r| !prog.outputs.contains(r)).collect();
    // A library stage holds every relation its cross-file part derives, its output (decision 0059).
    let mut library_rows = library::Rows::default();
    if library_stage {
        wanted.extend(cross.program.rules.iter().map(|r| r.head.rel.clone()).filter(|r| !prog.outputs.contains(r)));
        library_rows.keys = docs.iter().map(|(d, p)| (p.clone(), *d)).collect();
    }
    let mut collect = |db: &Connection, got: &Outcome, part: &front::Program| -> Result<()> {
        if !library_stage {
            return Ok(());
        }
        let derived: BTreeSet<&str> = part.rules.iter().map(|r| r.head.rel.as_str()).collect();
        for rel in derived.into_iter().filter(|r| !r.ends_with(cross_rounds::PREV) && !r.ends_with(cross_rounds::DONE)) {
            let width = part.decls[rel].len();
            let rows = held_rows(db, got, rel, width)?;
            library_rows.rels.insert(rel.to_owned(), (width, rows.into_iter().flatten().collect()));
        }
        Ok(())
    };
    let mut returns = Vec::new();
    if !cross.program.rules.is_empty() {
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
            let got = super::run_atoms_native_over(&cross, &db, "main", cross_stats, wanted.clone(), modes)?;
            collect(&db, &got, &cross.program)?;
            if wanted.contains("returns") {
                returns = held_rows(&db, &got, "returns", 2)?;
            }
            native.extend(gather(got, &mut outputs, &mut node_columns, &outputs_of).into_iter().map(|k| format!("cross:{k}")));
        } else if let Some(cone) = cross_rounds::cone_split(&bounded.program, &bounded.hops) {
            // The rounds run the hops' cone; what reads it and no hop reads runs once, after the last round.
            // Held: what the rest reads, unless an output (written to its table, read from there).
            let mut held = wanted.clone();
            held.extend(cone.reads.iter().filter(|r| !outputs_of(r)).cloned());
            let cone_rules = if library_stage { cone.cone.clone() } else { front::Program::default() };
            let got = run_rounds(&db, &modules, part(cone.cone), &bounded.hops, &held, &cross_stats, modes, k.expect("declared"), &mut rounds)?;
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
                let rows = held_rows(&db, &got, rel, types.len())?;
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
            if rest_wanted.contains("returns") {
                returns = held_rows(&db, &got, "returns", 2)?;
            }
            native.extend(gather(got, &mut outputs, &mut node_columns, &outputs_of).into_iter().map(|k| format!("cross:{k}")));
        } else {
            let all_rules = if library_stage { bounded.program.clone() } else { front::Program::default() };
            let got = run_rounds(&db, &modules, part(bounded.program), &bounded.hops, &wanted, &cross_stats, modes, k.expect("declared"), &mut rounds)?;
            collect(&db, &got, &all_rules)?;
            if wanted.contains("returns") {
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
    if library_stage {
        library::write(out, &library_rows)?;
    }
    statements.enter("write");
    let mut rows = BTreeMap::new();
    for (rel, lines) in &outputs {
        let node_columns = &node_columns[rel];
        let mut decoded: Vec<String> = lines
            .iter()
            // A row about a loaded document (its first node is one) is not the corpus's; a corpus row may name a
            // loaded document's node (a call's target in a stub).
            .filter(|row| library_keys.is_empty() || node_columns.first().is_none_or(|c| !row.split('\t').nth(*c).and_then(|v| v.parse::<i64>().ok()).is_some_and(|v| library_keys.contains(&(v >> 32)))))
            .map(|row| {
                row.split('\t')
                    .enumerate()
                    .map(|(i, v)| match v.parse::<i64>() {
                        // A guest's node (decision 0055): its document is no file of the run.
                        Ok(v) if node_columns.contains(&i) && !paths.contains_key(&(v >> 32)) && !first.contains_key(&(v >> 32)) => format!("guest{}:{}", v >> 32, v & 0xffff_ffff),
                        Ok(v) if node_columns.contains(&i) && own => format!("{}:{}", paths[&(v >> 32)], v & 0xffff_ffff),
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
        crate::code_rule_dsl::logic::profile::decision("per-file memo use", || format!("{memo_hits} of {} files read from the memo; {memo_unattributed} chunks' rows not memoised (a row naming no document of its chunk)", docs.len()));
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
    Ok(Report { phases, files: docs.len(), nodes, native: native.len(), rows, chunks, rounds, typing: typed, text: (vocabulary.tests.len(), tested, text_cpu), typing_runs, memo: (memo_hits, memo_unattributed) })
}


/// A program that reads the file system or loads documents (t21 E1, E2; `world`): rounds of its requests over the
/// corpus and what it has loaded, until a round asks for nothing new, then the run proper over both.
#[allow(clippy::too_many_arguments)]
fn discover(programs: &[&str], prog: &front::Program, root: &Path, t_facts: Option<&Path>, out: &Path, modes: &str, binding: Binding, chunk_nodes: usize, typing: &Typing, options: &serde_json::Value) -> Result<Report> {
    let (python, rs) = corpus_files(root)?;
    let rust = typing.rust.unwrap_or(rs.len() > python.len());
    let (label, corpus) = if rust { ("rust", rs) } else { ("python", python) };
    let corpus: BTreeSet<String> = corpus.into_iter().collect();
    let mut world = world::World::new(root, prog);
    let scratch = out.join("world");
    let start = thread_usage();
    let mut rounds = 0;
    loop {
        rounds += 1;
        let _round = crate::code_rule_dsl::logic::profile::span(|| format!("world round {rounds}"));
        run_with_options(programs, root, None, &scratch, modes, binding, chunk_nodes, typing, &world.options(options, prog, "discover"))?;
        if !world.answer(&scratch, label, &corpus)? || rounds >= world::ROUNDS {
            break;
        }
    }
    let now = thread_usage();
    let mut final_options = world.options(options, prog, "final");
    let mut summary = format!("{}; {:.2} s CPU on the main thread", world.summary(rounds), now.0 - start.0 + now.1 - start.1);
    // The library's cross-file rows (decision 0059): the store's, or a library stage's, recorded for the next run.
    let loaded: Vec<String> = world::library_files(&final_options).into_iter().map(|(p, _)| p).collect();
    let off = prog.facts.get("env").into_iter().flatten().any(|r| matches!(r.as_slice(), [front::Value::Str(k), front::Value::Str(v)] if k == library::SETTING && v == "off"));
    if !loaded.is_empty() && !off && tape_cache().map_err(|e| Error::Plan(e.to_string()))?.is_some() {
        let key = library::key(programs, &loaded)?;
        let dir = out.join("library");
        std::fs::create_dir_all(&dir).map_err(|e| Error::Plan(format!("{}: {e}", dir.display())))?;
        let file = dir.join(library::FILE);
        let stored = tape_cache().map_err(|e| Error::Plan(e.to_string()))?.and_then(|store| library::lookup(&store, &key));
        let start = thread_usage();
        let how = match stored {
            Some(bytes) => {
                std::fs::write(&file, bytes).map_err(|e| Error::Plan(format!("{}: {e}", file.display())))?;
                "read from the store"
            }
            None => {
                run_with_options(programs, root, None, &dir, modes, binding, chunk_nodes, typing, &world.options(options, prog, "library"))?;
                let bytes = std::fs::read(&file).map_err(|e| Error::Plan(format!("{}: {e}", file.display())))?;
                if let Some(store) = tape_cache().map_err(|e| Error::Plan(e.to_string()))? {
                    library::record(&store, &key, &bytes, &loaded)?;
                }
                "made by a library stage and stored"
            }
        };
        let now = thread_usage();
        summary.push_str(&format!("; the library's cross-file rows {how} ({:.2} s CPU)", now.0 - start.0 + now.1 - start.1));
        final_options[library::OPTION] = file.to_string_lossy().into_owned().into();
    }
    final_options["worldSummary"] = summary.into();
    run_with_options(programs, root, t_facts, out, modes, binding, chunk_nodes, typing, &final_options)
}

/// The cross-file part's bounded rounds (`typing::cross_rounds`): `round` is the part with its hops reading
/// `<rel>__prev`, the previous round's rows held in memory (none in round 0). Each round's rows of the
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
                Arc::new(Rows::read(db, rel)?)
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


/// The corpus's files under `root`, relative and in path order: Python's (`.py`) and Rust's (`.rs`), skipping
/// hidden directories and `target`.
fn corpus_files(root: &Path) -> Result<(Vec<String>, Vec<String>)> {
    let (mut python, mut rust) = (Vec::new(), Vec::new());
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
            let relative = path.strip_prefix(root).map_err(|e| Error::Plan(e.to_string()))?.to_string_lossy().into_owned();
            // The language registry's suffixes (`languages/<id>.json`): Python's include stubs (`.pyi`).
            match crate::code_rule_dsl::language::label_of_path(&relative) {
                Some("python") => python.push(relative),
                Some("rust") => rust.push(relative),
                _ => {}
            }
        }
    }
    python.sort();
    rust.sort();
    // `RERAIL_SAMPLE_EVERY=n`: every n-th file only, to iterate on a rule's cost quickly (its findings are a sample's).
    if let Some(n) = std::env::var("RERAIL_SAMPLE_EVERY").ok().and_then(|n| n.parse::<usize>().ok()).filter(|n| *n > 1) {
        for files in [&mut python, &mut rust] {
            *files = files.iter().step_by(n).cloned().collect();
        }
    }
    Ok((python, rust))
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

/// The base relations' statistics a chunk measures as it binds them, beyond each column's moments
/// (`syntax::statistics`), which T's `stats.py` made at ingestion: per kind atom its nodes and summed subtree sizes
/// (`stat_kind`), per (parent kind, child kind) its edges (`stat_edge`), the documents, and per (relation, column,
/// constant) of the program's constants its rows (`stat_val`'s `n`). Summed over the chunks for the cross-file part:
/// a document is in one chunk, so every count adds exactly.
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

#[cfg(test)]
mod tests {
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
        super::record_run(&store, &root, &[source], &program, &[], None).unwrap().unwrap().join().unwrap().1.unwrap();
        assert!(store.corpus_collection_due().unwrap());
        // Same files and plan: the manifest is unchanged, but pending collection must still run.
        super::record_run(&store, &root, &[source], &program, &[], Some(reader)).unwrap().unwrap().join().unwrap().1.unwrap();
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
