//! Phases over separate fact spaces (decision 0083 §1).
//!
//! A phase is a `.rerail` program that names itself and the relations it hands on:
//!
//! ```text
//! // typing/programs/struct.rerail                    // a later phase, or any rule
//! .phase struct                                   .decl struct__binds(m: symbol, x: symbol, t: symbol)
//! .export unit, def, binds                        r(m, x) :- struct__binds(m, x, _).
//! ```
//!
//! A program reads an earlier phase's export as `<phase>__<export>`. The corpus runner runs the phase first: its own
//! run of the same engine, in its own fact space (its program, its rules, its scratch), to its fixed point. The
//! reader sees only the exports, and only once that run has ended. A relation the phase does not export cannot be
//! read: it is not in the reader's space.
//!
//! **Exports name symbols, never nodes.** A node id names a position in one version of a file, so an edit renumbers
//! it; an export with a `node` column is refused.
//!
//! **A phase's state is its run's relations** (t32 stage 3, decision 0104). The phase's run keeps its relations as
//! every run does: the per-file memo, the kept strata, a session's resident rows. An unchanged phase is a run whose
//! parts are all kept; an edit is maintained by that run as any input change is. Its exports are its run's outputs,
//! handed to the reader as input rows, and the reader's run takes their delta as it takes any input's. There is no
//! store, record or version of a phase's own.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use super::super::{Error, Result};
use super::{Report, Typing};
use crate::code_rule_dsl::logic::front::Value;
use crate::code_rule_dsl::logic::program_image::Version;
use crate::code_rule_dsl::logic::program_image::fields::ProgramFields;
use crate::code_rule_dsl::typing::file_local::Binding;

use crate::code_rule_dsl::logic::plan::phased::SEP;

/// The phases whose runs are under way, outermost first, in the run's context: a phase reading a later phase's export
/// runs that phase inside its own run, so a phase met again is a cycle. A run in a job is inside the same phases.
#[derive(Clone, Default)]
struct Phases(std::sync::Arc<Vec<String>>);

/// While a phase runs, what it reads from earlier phases, the innermost last, in the run's context: worked out once
/// (`facts`), and taken as it is by the phase's own run, which would otherwise run those phases again.
#[derive(Clone, Default)]
struct HandedDown(std::sync::Arc<Vec<(String, std::sync::Arc<Facts>)>>);

/// Phase 1's name (`sources.rerail`): run first, once for a run, by the runner (`corpus::run_observed`), not as a phase a
/// program's reads start.
pub(super) const SOURCES: &str = "sources";

/// What phase 1 handed on, for the run that ran it and every phase run inside that run: its documents are theirs.
#[derive(Clone)]
pub struct PhaseOne {
    /// The tree it read, and its grammar: a run of another tree or grammar (phase 0's) does not take its documents.
    pub(super) root: PathBuf,
    pub(super) label: String,
    /// The source files it made documents of, `(path, grammar, content)`, in the order given.
    pub documents: Vec<(String, String, String)>,
    /// The library documents the imports reached, `(canonical path, module, grammar, content)`.
    pub libraries: Vec<(String, String, String, String)>,
    /// Its exports, as a reader of `sources__<export>` takes them.
    rows: BTreeMap<String, Vec<Vec<Value>>>,
}

/// Phase 1's result, for the run that ran it and every phase and job inside that run (the run's context).
#[derive(Clone, Default)]
struct HeldPhaseOne(Option<std::sync::Arc<PhaseOne>>);

/// Holds phase 1's result for the run until dropped.
pub struct PhaseOneHeld {
    _one: crate::code_rule_dsl::logic::profile::Bound,
}
impl PhaseOneHeld {
    pub fn enter(one: PhaseOne) -> PhaseOneHeld {
        PhaseOneHeld { _one: crate::code_rule_dsl::logic::profile::bind(HeldPhaseOne(Some(std::sync::Arc::new(one)))) }
    }
}

/// Phase 1's result, while a run that ran it is under way here, if it read `root` in `label`.
pub fn phase_one(root: &Path, label: &str) -> Option<std::sync::Arc<PhaseOne>> {
    crate::code_rule_dsl::logic::profile::bound::<HeldPhaseOne>().0.filter(|o| o.root == root && o.label == label)
}

/// Hands phase `name`'s upstream facts to its run until dropped.
struct HandingDown {
    _handed: crate::code_rule_dsl::logic::profile::Bound,
}
impl HandingDown {
    fn enter(name: &str, facts: Facts) -> HandingDown {
        let mut handed = Vec::clone(&crate::code_rule_dsl::logic::profile::bound::<HandedDown>().0);
        handed.push((name.to_owned(), std::sync::Arc::new(facts)));
        HandingDown { _handed: crate::code_rule_dsl::logic::profile::bind(HandedDown(std::sync::Arc::new(handed))) }
    }
}

/// Marks `name`'s run under way until dropped.
struct Running {
    _phases: crate::code_rule_dsl::logic::profile::Bound,
}
impl Running {
    fn enter(name: &str) -> Result<Running> {
        let mut r = Vec::clone(&crate::code_rule_dsl::logic::profile::bound::<Phases>().0);
        if r.iter().any(|n| n == name) {
            return Err(Error::Plan(format!("phases read each other: {} -> {name}", r.join(" -> "))));
        }
        r.push(name.to_owned());
        Ok(Running { _phases: crate::code_rule_dsl::logic::profile::bind(Phases(std::sync::Arc::new(r))) })
    }
}

/// What a program says of itself as a phase: its name and its exports.
pub struct Declared {
    pub(crate) name: String,
    pub exports: Vec<String>,
}

/// The phase `prog` declares (`.phase NAME`, `.export a, b`), if any.
pub fn declared(prog: &(impl ProgramFields + ?Sized)) -> Option<Declared> {
    let setting = |key: &str| prog.setting_values(key);
    let name = setting("phase").into_iter().next()?;
    let list = |key: &str| setting(key).iter().flat_map(|v| v.split(',').map(|e| e.trim().trim_end_matches('.').to_owned()).collect::<Vec<_>>()).filter(|e| !e.is_empty()).collect::<Vec<_>>();
    Some(Declared { name, exports: list("export") })
}

/// The phase program `name`, with its `#include`s read in: `<phase_dir>/<name>.rerail` when the run's options name a
/// `phase_dir`, else `typing/programs/<name>.rerail`.
fn source(name: &str, dir: Option<&Path>) -> Result<Option<String>> {
    let Some(path) = locate(name, dir) else {
        return Ok(None);
    };
    crate::code_rule_dsl::logic::settings::program_text(&path).map(Some).map_err(Error::Plan)
}

/// The phase programs lowered, kept for the process (decision 0442: the shared object's) by their text and include
/// graph (`rr::graph_digest`, as `combine` keys a text) and the switches set: a phase's program is the same in every run, and each run
/// lowered it again (phase 1's `sources.rerail`, 0.05-0.07 s of a run at the tests' opt-level).
#[derive(Default)]
pub struct PhasePrograms(std::sync::Mutex<HashMap<[u8; 32], std::sync::Arc<Version>>>);

/// Phase program `src` lowered (`rr::parse_version`), or as an earlier run of the process lowered it (`PhasePrograms`).
fn lowered(src: &str) -> Result<std::sync::Arc<Version>> {
    use sha2::Digest as _;
    const KEPT: usize = 16;
    let mut hash = sha2::Sha256::new();
    hash.update(src.as_bytes());
    // Under another value of a switch, the same text may lower to another program: each one set that can change what a
    // run outputs (`reuse::measures_only` the rest), as a run's key holds them. A test sets them while others run.
    for (k, v) in crate::code_rule_dsl::logic::profile::vars() {
        if !super::reuse::measures_only(&k) {
            hash.update(format!("\0{k}={v}").as_bytes());
        }
    }
    if let Some(graph) = crate::code_rule_dsl::logic::rr::graph_digest(src)? {
        hash.update(graph);
    }
    let key: [u8; 32] = hash.finalize().into();
    let held = crate::code_rule_dsl::logic::profile::shared::<PhasePrograms>();
    if let Some(v) = held.0.lock().expect("the phase programs").get(&key) {
        return Ok(std::sync::Arc::clone(v));
    }
    let v = crate::code_rule_dsl::logic::rr::parse_version(src)?;
    let mut kept = held.0.lock().expect("the phase programs");
    if kept.len() >= KEPT {
        kept.clear();
    }
    kept.insert(key, std::sync::Arc::clone(&v));
    Ok(v)
}

/// The engine's own phase programs' directory: where a phase is found when no `phase_dir` is given (`locate`), and
/// where phase 0 finds the phases a program names (`phase0::phase_lib`).
pub(super) fn builtin_dir() -> PathBuf {
    rerail_store::source_tree::native().join("src/code_rule_dsl/typing/programs")
}

/// The file of phase program `name` (`source`).
fn locate(name: &str, dir: Option<&Path>) -> Option<PathBuf> {
    candidates(name, dir).into_iter().find(|p| p.exists())
}

/// Where a phase program `name` is looked for, in order: `dir`, then the engine's own programs.
fn candidates(name: &str, dir: Option<&Path>) -> Vec<PathBuf> {
    let builtin = builtin_dir();
    dir.into_iter().chain([builtin.as_path()]).map(|d| d.join(format!("{name}.rerail"))).collect()
}

/// The files of every phase program `programs` may read, and of the phases those may read: each phase's `.rerail` and
/// its `#include`s. A run of a phased program is a function of them too, so the whole-run cutoff names them
/// (O-cutoff-phases).
///
/// Read off the texts' declarations (`rr::declared_names`), nothing lowered: lowering the main program to find
/// them was 2.7 s of a one-file edit. A relation declared `<phase>__<x>`, for a phase program, names that phase:
/// a superset of what `wanted` finds (it also counts one a rule of the reader makes), so the record names as many
/// files or more, and the record and its check list them the same way.
/// `read`: the programs' texts, read once by the run (`Texts::names`); a phase program found here is read here.
/// With the places looked in before each one found (or every place, for a phase found nowhere): a phase program
/// appearing there is another run (decision 0275: the whole-run cutoff checks them as files that must still be
/// missing).
pub(super) fn phase_files(read: &[crate::code_rule_dsl::logic::rr::TextNames], dir: Option<&Path>) -> Result<(Vec<PathBuf>, Vec<PathBuf>)> {
    let (mut seen, mut out, mut missed) = (BTreeSet::new(), Vec::new(), Vec::new());
    let mut todo: Vec<(Vec<String>, Option<String>)> = read.iter().map(|t| (t.names.clone(), t.phase.clone())).collect();
    while let Some((names, own)) = todo.pop() {
        let phases: BTreeSet<&str> = names
            .iter()
            .filter_map(|rel| rel.split_once(SEP))
            .filter(|(phase, export)| !phase.is_empty() && !export.is_empty() && own.as_deref() != Some(*phase) && !phase.starts_with("rr_"))
            .map(|(phase, _)| phase)
            .collect();
        for name in phases {
            if !seen.insert(name.to_owned()) {
                continue;
            }
            let looked = candidates(name, dir);
            let found = looked.iter().position(|p| p.exists());
            missed.extend(looked[..found.unwrap_or(looked.len())].iter().cloned());
            let Some(path) = found.map(|k| looked[k].clone()) else { continue };
            let raw = std::fs::read_to_string(&path).map_err(|e| Error::Plan(format!("{}: {e}", path.display())))?;
            out.extend(crate::code_rule_dsl::logic::rr::included_files(&raw)?);
            out.push(path);
            if let Some(src) = source(name, dir)? {
                todo.push(crate::code_rule_dsl::logic::rr::declared_names(&src)?);
            }
        }
    }
    Ok((out, missed))
}

/// The relations `prog` reads from earlier phases, by phase: `(the reader's relation, the export)`. A relation named
/// `<phase>__<x>` that no rule of `prog` makes (by its rules' `shape`), where `<phase>` is a phase program.
pub(super) fn wanted_in(prog: &(impl ProgramFields + ?Sized), shape: &super::shape::Shape, dir: Option<&Path>) -> Result<BTreeMap<String, Vec<(String, String)>>> {
    let declared_here = declared(prog);
    // Only the reads an output (or, in a phase, an export) reaches: a library a program includes for one rule may hold
    // rules that read a phase no output needs (python/calls.rerail's points-to, for a policy that takes `call_arg`),
    // and that phase would run at every run and every edit for nothing. A program that names no output reads all.
    let roots: Vec<String> = prog.output_names().into_iter().chain(declared_here.iter().flat_map(|d| d.exports.iter().cloned())).collect();
    let reached: Option<BTreeSet<&str>> = (!roots.is_empty()).then(|| {
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        let mut todo: Vec<&str> = roots.iter().map(String::as_str).collect();
        while let Some(r) = todo.pop() {
            if seen.insert(r)
                && let Some(reads) = shape.reads_of.get(r)
            {
                todo.extend(reads.iter().map(String::as_str));
            }
        }
        seen
    });
    let own = declared_here.map(|d| d.name);
    let mut out: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for rel in shape.read.iter().map(String::as_str).filter(|r| !shape.made.contains(*r) && prog.declares(r) && reached.as_ref().is_none_or(|s| s.contains(r))) {
        let Some((phase, export)) = rel.split_once(SEP) else { continue };
        if phase.is_empty() || export.is_empty() || own.as_deref() == Some(phase) || phase.starts_with("rr_") {
            continue;
        }
        if source(phase, dir)?.is_some() {
            out.entry(phase.to_owned()).or_default().push((rel.to_owned(), export.to_owned()));
        }
    }
    Ok(out)
}

/// Whether `rel` is an input shared by every program of a run (the planner's, `plan::phased`).
pub use crate::code_rule_dsl::logic::plan::phased::is_phased;

/// A batch of policies run one after another over one tree (`tools::run_policies`): while it is held, phase 1's runs
/// are kept by their inputs and taken again by a policy whose run has the same ones. Nothing is kept past it, so an
/// edit between batches is never missed.
///
/// The batch is one run of the frames' spaces too (O-store-single-program): its policies share one run token, as a
/// run's nested programs do. Each policy counted as its own run of the root's cross-file space, so a policy's kept
/// inputs, used once a batch, went unused for 43 of the space's runs and were collected (by recency, before decision 0227) by another
/// policy's write: the next batch found its kept inputs and its memo entries gone.
///
/// The batch is its caller's (decision 0442): held by the thread that entered it and carried to its runs' jobs
/// (`carried`), so two batches at once, on two threads, never see each other's runs.
///
/// Its parts drop in the reverse of their binding (fields drop in order): the reading, the run's count, the batch.
pub struct Batch {
    _reading: Reading,
    _under_way: super::UnderWay,
    _batch: crate::code_rule_dsl::logic::profile::Bound,
}

type BatchRuns = std::collections::HashMap<[u8; 32], (PhaseOne, Vec<(String, usize)>)>;

/// A batch's own: its phase runs by their inputs, and its runs' memo plans by root (`batch_plans`).
#[derive(Default)]
pub(in crate::code_rule_dsl) struct BatchState {
    runs: std::sync::Mutex<BatchRuns>,
    plans: std::sync::Mutex<std::collections::HashMap<String, Vec<[u8; 32]>>>,
}

/// The batch under way (`Batch`), in the run's context: its caller's, and its runs' jobs'.
#[derive(Clone, Default)]
struct BatchUnderWay(Option<std::sync::Arc<BatchState>>);

impl Batch {
    pub fn enter() -> Batch {
        let batch = crate::code_rule_dsl::logic::profile::bind(BatchUnderWay(Some(Default::default())));
        let under_way = super::UnderWay::enter();
        Batch { _reading: Reading::enter(), _under_way: under_way, _batch: batch }
    }

    fn state() -> Option<std::sync::Arc<BatchState>> {
        crate::code_rule_dsl::logic::profile::bound::<BatchUnderWay>().0
    }

    fn active() -> bool {
        Self::state().is_some()
    }

    fn get(key: &[u8; 32]) -> Option<(PhaseOne, Vec<(String, usize)>)> {
        Self::state()?.runs.lock().expect("the batch's phase runs").get(key).cloned()
    }

    fn put(key: [u8; 32], run: (PhaseOne, Vec<(String, usize)>)) {
        if let Some(state) = Self::state() {
            state.runs.lock().expect("the batch's phase runs").insert(key, run);
        }
    }
}



/// While one is held, the reads made outside a run (a combined program, `combine_stored_version`; a rule file's tape was one
/// too until the lean parser read it, decision 0275) share one handle on the tape cache (`reading_store`): each had opened the store and closed it,
/// and the close of the store's last handle closes every thread's connection to it (`ContentStore`'s drop). A warm
/// batch spent 514 of 7,850 samples so, a file at a time. A batch holds one; so does a program's reading.
///
/// The reading is its caller's (decision 0442): bound by the holder that entered it first (a nested holder joins it), in
/// the run's context, so its runs' jobs share it too.
pub struct Reading {
    _outermost: Option<crate::code_rule_dsl::logic::profile::Bound>,
}

/// A reading's handle: the tape cache's path, and the store there (`None`: none), opened at the first read.
#[derive(Default)]
pub(in crate::code_rule_dsl) struct ReadingState(std::sync::Mutex<Option<(std::path::PathBuf, Option<std::sync::Arc<crate::content_store::ContentStore>>)>>);

/// The reading under way (`Reading`), in the run's context.
#[derive(Clone, Default)]
struct ReadingUnderWay(Option<std::sync::Arc<ReadingState>>);

impl Reading {
    pub fn enter() -> Reading {
        // The outermost holder's binding: the shared handle closes as it drops (and the jobs that took it end).
        let outer = crate::code_rule_dsl::logic::profile::bound::<ReadingUnderWay>().0.is_none();
        Reading { _outermost: outer.then(|| crate::code_rule_dsl::logic::profile::bind(ReadingUnderWay(Some(Default::default())))) }
    }
}

/// The tape cache, for a read outside a run: the handle the holders share (`Reading`), opened at the first read; with
/// none under way, or another tape cache this thread's (a test's own), one opened for the read.
pub fn reading_store() -> Option<std::sync::Arc<crate::content_store::ContentStore>> {
    let path = super::tape_cache_path();
    if let Some(reading) = crate::code_rule_dsl::logic::profile::bound::<ReadingUnderWay>().0 {
        let mut r = reading.0.lock().expect("the reading store");
        match (&*r, &path) {
            (Some((held, store)), Some(path)) if held == path => return store.clone(),
            (None, Some(path)) => {
                let store = crate::content_store::ContentStore::open(path).ok().map(std::sync::Arc::new);
                *r = Some((path.clone(), store.clone()));
                return store;
            }
            _ => {}
        }
    }
    path.and_then(|p| crate::content_store::ContentStore::open(&p).ok()).map(std::sync::Arc::new)
}

// The memo plans of the batch's runs so far, by root (O-store-single-program): `BatchState::plans`.

/// The plans a run's manifest over `root` names besides its own (`plan`) and `also`: under a batch, its earlier runs'
/// over the root, and this run's are added. Each policy recorded its own manifest, and collection keeps the plans of a
/// root's last `protected_runs` (2): of a 44-policy batch, 42 plans were dead at the next decision, their memo spaces
/// dropped, and the next batch found a hit's memo entry gone. The batch is one run of the root. `None` outside a
/// batch.
pub fn batch_plans(root: &str, plan: [u8; 32], also: &[[u8; 32]]) -> Option<Vec<[u8; 32]>> {
    let state = Batch::state()?;
    let mut all = state.plans.lock().expect("the batch's plans");
    let plans = all.entry(root.to_owned()).or_default();
    let earlier: Vec<[u8; 32]> = plans.iter().filter(|p| **p != plan && !also.contains(p)).copied().collect();
    for p in std::iter::once(&plan).chain(also) {
        if !plans.contains(p) {
            plans.push(*p);
        }
    }
    Some(earlier)
}

/// One phase's run, as the reader's run reports it.
pub struct PhaseRun {
    pub name: String,
    pub report: Report,
    /// Each export's rows.
    pub exports: Vec<(String, usize)>,
}

pub(crate) type Facts = BTreeMap<String, Vec<Vec<Value>>>;

/// The facts `prog` reads from earlier phases (`<phase>__<export>`), each phase run first in its own space, and the
/// phases' runs, in the order they ran.
pub fn facts(prog: &(impl ProgramFields + ?Sized), shape: &super::shape::Shape, root: &Path, modes: &str, binding: Binding, chunk_nodes: usize, typing: &Typing, options: &serde_json::Value) -> Result<(Facts, Vec<PhaseRun>)> {
    // A phase's own run: its upstream facts were worked out by the run that started it.
    if let Some(me) = declared(prog)
        && let Some(f) = crate::code_rule_dsl::logic::profile::with_bound(|h: &HandedDown| h.0.last().filter(|(n, _)| *n == me.name).map(|(_, f)| Facts::clone(f)))
    {
        return Ok((f, Vec::new()));
    }
    let (mut facts, mut runs) = (Facts::new(), Vec::new());
    // The constants' own run (`constants::derive`) reads no phase: its relations are made from no document, and a
    // phase's exports are the run's inputs, never a constant. Run under its options, a phase would be a constants
    // run itself, writing none of its exports.
    if options.get(super::constants::OWN).is_some() {
        return Ok((facts, runs));
    }
    let dir = options.get("phase_dir").and_then(|v| v.as_str()).map(PathBuf::from);
    for (name, reads) in wanted_in(prog, shape, dir.as_deref())? {
        // Phase 1 ran before any phase, for the run (`PhaseOne`): its exports are handed on as it left them. A run
        // that did not run it (given its files, or with `RERAIL_SOURCES_PHASE=off`) reads none.
        if name == SOURCES {
            // Its program is there (a builtin), though phase 1 ran it already.
            let _ = source(&name, dir.as_deref())?.expect("sources.rerail, a builtin phase program");
            let rows = crate::code_rule_dsl::logic::profile::with_bound(|p: &HeldPhaseOne| p.0.as_ref().map(|o| o.rows.clone())).unwrap_or_default();
            hand_on(&reads, &rows, &mut facts);
            continue;
        }
        // An earlier phase that reads a phase of its own runs that one first, inside its run (struct, then types).
        let _running = Running::enter(&name)?;
        let src = source(&name, dir.as_deref())?.expect("a phase program");
        let program = lowered(&src)?;
        // A later phase sees the libraries only through what earlier ones hand on (0083 §1: P3 loads nothing).
        if shape.made.contains("load") && loads_documents(&program, dir.as_deref(), &mut BTreeSet::new())? {
            return Err(Error::Plan(format!("a program reading phase {name}, whose fixed point loaded documents, loads documents itself: it reads the libraries through {name}'s exports")));
        }
        let declared = declared(&*program).ok_or_else(|| Error::Plan(format!("{name}.rerail: no `.phase {name}`")))?;
        if declared.name != name {
            return Err(Error::Plan(format!("{name}.rerail says `.phase {}`", declared.name)));
        }
        for export in &declared.exports {
            let cols = program.columns_of(export).ok_or_else(|| Error::Plan(format!("{name}.rerail exports {export}, which it does not declare")))?;
            if let Some((c, _)) = cols.iter().find(|(_, t)| t == "node") {
                return Err(Error::Plan(format!("{name}.rerail exports {export}, whose column {c} is a node: an export names symbols, which an edit does not renumber")));
            }
            if cols.first().is_none_or(|(_, t)| t != "symbol" && t != "sym") {
                return Err(Error::Plan(format!("{name}.rerail exports {export}: its first column, the row's granule, must be a symbol")));
            }
        }
        for (rel, export) in &reads {
            if !declared.exports.contains(export) {
                return Err(Error::Plan(format!("{rel}: phase {name} does not export {export} (a phase's other relations are its own)")));
            }
            let (want, have) = (program.columns_of(export).unwrap_or_default(), prog.columns_of(rel).unwrap_or_default());
            let ty = |t: &str| if t == "sym" { "symbol".to_owned() } else if t == "int" { "number".to_owned() } else { t.to_owned() };
            if want.len() != have.len() || want.iter().zip(&have).any(|((_, a), (_, b))| ty(a) != ty(b)) {
                return Err(Error::Plan(format!("{rel}: {} columns, where phase {name}'s {export} has {}", have.len(), want.len())));
            }
        }
        // The phase's run: its own program, its own scratch, its exports written as its outputs. The earlier phases it
        // reads run first (struct, then types), and are listed with the reader's, in the order they ran, each once.
        let mut image = Vec::new();
        // Its exports are its outputs: the program read, given them (decision 0249), not its text with an `.output`
        // after it read again.
        let handed = {
            use crate::code_rule_dsl::logic::program_image::fields::FieldsMut;
            let mut b = crate::code_rule_dsl::logic::program_image::Builder::on(&program);
            for e in &declared.exports {
                b.fields().add_output(e);
            }
            b.finish().map_err(Error::Plan)?.0
        };
        let (upstream, upstream_runs) = self::facts(&*program, &super::shape::Shape::of(&program), root, modes, binding, chunk_nodes, typing, options)?;
        for r in upstream_runs {
            if !runs.iter().any(|x: &PhaseRun| x.name == r.name) {
                runs.push(r);
            }
        }
        let _down = HandingDown::enter(&name, upstream);
        let report = {
            // The phase's own run, its jobs' spans under it (`profile::span_at`).
            let _s = crate::code_rule_dsl::logic::profile::span(|| format!("phase {name}: run"));
            super::run_read_handing(&handed, None, root, Path::new(""), modes, binding, chunk_nodes, typing, options, Some(&mut image))?
        };
        let rows = { let _s = crate::code_rule_dsl::logic::profile::span(|| format!("phase {name}: its exports read")); read_exports(&name, &*program, &declared, &image)? };
        { let _s = crate::code_rule_dsl::logic::profile::span(|| format!("phase {name}: handed on")); hand_on(&reads, &rows, &mut facts); }
        let exports = declared.exports.iter().map(|e| (e.clone(), rows.get(e).map_or(0, Vec::len))).collect();
        runs.push(PhaseRun { name, report, exports });
    }
    Ok((facts, runs))
}

/// The facts handed down to the innermost phase under way on this thread (phase 1's `observer__source_file`), as text:
/// a phase's run is a function of them as of its program, so the whole-run cutoff keys its record by them too.
pub(super) fn handed_down_text() -> String {
    crate::code_rule_dsl::logic::profile::with_bound(|h: &HandedDown| h.0.last().map(|(n, f)| format!("{n}\0{f:?}")).unwrap_or_default())
}

/// Content already observed for source requests, available before the request waves start.
pub fn observed_source_digests() -> Vec<[u8; 32]> {
    crate::code_rule_dsl::logic::profile::with_bound(|handed: &HandedDown| {
        handed.0.last().and_then(|(_, facts)| facts.get("observer__source_file"))
            .into_iter().flatten()
            .filter_map(|row| match row.get(3) {
                Some(Value::Str(digest)) => crate::content_store::observe::unhex(digest),
                _ => None,
            })
            .collect::<std::collections::BTreeSet<_>>().into_iter().collect()
    })
}

/// The files handed down to the innermost phase under way on this thread (phase 1's `observer__source_file`, with
/// `observer__source_path`), known before its first wave: each one's paths (as given, and as loaded where that is
/// another), its module and its content's digest.
pub(super) fn handed_files() -> Vec<(Vec<String>, String, [u8; 32])> {
    crate::code_rule_dsl::logic::profile::with_bound(|handed: &HandedDown| {
        let Some((_, facts)) = handed.0.last() else { return Vec::new() };
        let text = |v: Option<&Value>| match v {
            Some(Value::Str(s)) => Some(s.to_string()),
            _ => None,
        };
        let loaded: HashMap<String, String> = facts.get("observer__source_path").into_iter().flatten().filter_map(|r| Some((text(r.first())?, text(r.get(1))?))).collect();
        facts
            .get("observer__source_file")
            .into_iter()
            .flatten()
            .filter_map(|r| {
                let path = text(r.first())?;
                let digest = crate::content_store::observe::unhex(&text(r.get(3))?)?;
                let mut paths = vec![path.clone()];
                paths.extend(loaded.get(&path).filter(|f| **f != path).cloned());
                Some((paths, text(r.get(1))?, digest))
            })
            .collect()
    })
}

/// Whether a phase's run is under way on this thread (its documents are its own: phase 1 does not run inside it).
pub fn in_phase() -> bool {
    crate::code_rule_dsl::logic::profile::with_bound(|r: &Phases| !r.0.is_empty())
}

/// The innermost phase whose run is under way on this thread, if any: its facts are its own fact space's.
pub fn current() -> Option<String> {
    crate::code_rule_dsl::logic::profile::with_bound(|r: &Phases| r.0.last().cloned())
}

/// Phase 1's program, the builtin one.
pub(super) fn sources_path() -> PathBuf {
    rerail_store::source_tree::native().join("src/code_rule_dsl/typing/programs/sources.rerail")
}

/// Phase 1, `sources` (decision 0104): the run's source files, each `(path, module, grammar, content digest)`, loaded
/// as documents by `sources.rerail`, its own run of the same engine, with the libraries their imports reach. What it hands
/// on is each file that became a document, `(path, grammar, content)`, in the order given, and each library document:
/// the later phases' documents.
pub fn sources(
    files: &[(String, String, String, String)],
    // The texts the run read for `files`' digests, which this phase's builder borrows (decision 0300).
    texts: &super::texts::Sources,
    label: &str,
    root: &Path,
    modes: &str,
    binding: Binding,
    chunk_nodes: usize,
    typing: &Typing,
    options: &serde_json::Value,
) -> Result<(PhaseOne, PhaseRun)> {
    const NAME: &str = SOURCES;
    let _running = Running::enter(NAME)?;
    let dir = options.get("phase_dir").and_then(|v| v.as_str()).map(PathBuf::from);
    let src = source(NAME, dir.as_deref())?.expect("sources.rerail, a builtin phase program");
    // Within a batch of policies (`Batch`), a run over the same files, tree, language and options as an earlier one
    // is that one: phase 1 is a function of them (O-batch-phase-sources).
    let key = Batch::active().then(|| {
        use sha2::Digest as _;
        let mut hash = sha2::Sha256::new();
        for part in [src.as_str(), label, &root.display().to_string(), modes, &format!("{binding:?}"), &chunk_nodes.to_string(), &format!("{typing:?}"), &options.to_string()] {
            hash.update(part.as_bytes());
            hash.update(b"\0");
        }
        for (p, m, g, c) in files {
            for part in [p, m, g, c] {
                hash.update(part.as_bytes());
                hash.update(b"\0");
            }
        }
        <[u8; 32]>::from(hash.finalize())
    });
    if let Some(found) = key.and_then(|k| Batch::get(&k)) {
        crate::code_rule_dsl::logic::profile::decision("phase 1 (sources.rerail)", || "the batch's run of the same files, taken again".into());
        let (one, exports) = found;
        return Ok((one, PhaseRun { name: NAME.to_owned(), report: Report::default(), exports }));
    }
    let program = lowered(&src)?;
    let declared = declared(&*program).ok_or_else(|| Error::Plan(format!("{NAME}.rerail: no `.phase {NAME}`")))?;
    // Its documents are those it loads: it is given no file of its own, and its language is the run's. The program
    // read, given that language and its exports as its outputs (decision 0249), not its text with directives after it.
    let handed = {
        use crate::code_rule_dsl::logic::program_image::fields::FieldsMut;
        let mut b = crate::code_rule_dsl::logic::program_image::Builder::on(&program);
        let mut settings = ProgramFields::settings(&*program);
        settings.push(("language".to_owned(), label.to_owned()));
        b.fields().set_field(crate::code_rule_dsl::logic::program_facts::Field::Settings(&settings));
        for e in &declared.exports {
            b.fields().add_output(e);
        }
        b.finish().map_err(Error::Plan)?.0
    };
    let mut options = options.clone();
    if let Some(o) = options.as_object_mut() {
        o.insert("inputFiles".to_owned(), serde_json::Value::Array(Vec::new()));
    } else {
        options = serde_json::json!({ "inputFiles": [] });
    }
    // The observer's facts: stated, handed down as a phase's exports are (they are its inputs, not its program).
    let stated: Vec<Vec<Value>> = files.iter().map(|(p, m, g, c)| vec![Value::Str(p.clone().into()), Value::Str(m.clone().into()), Value::Str(g.clone().into()), Value::Str(c.clone().into())]).collect();
    let paths = files.iter().filter_map(|(p, _, _, _)| {
        let file = if crate::git_workspace::snapshot::contains(root) {
            p.clone()
        } else {
            // Spelled with `/` on every OS, as the rules join and split paths (`corpus::slashed`).
            super::slashed(&crate::git_workspace::snapshot::canonicalize(root.join(p)).ok()?)
        };
        Some(vec![Value::Str(p.clone().into()), Value::Str(file.into())])
    }).collect();
    let _down = HandingDown::enter(NAME, Facts::from([
        ("observer__source_file".to_owned(), stated),
        ("observer__source_path".to_owned(), paths),
    ]));
    let mut image = Vec::new();
    let report = {
        let _s = crate::code_rule_dsl::logic::profile::span(|| format!("phase {NAME}: run"));
        super::run_read_handing(&handed, Some(texts), root, Path::new(""), modes, binding, chunk_nodes, typing, &options, Some(&mut image))?
    };
    let rows = { let _s = crate::code_rule_dsl::logic::profile::span(|| format!("phase {NAME}: its exports read")); read_exports(NAME, &*program, &declared, &image)? };
    let text = |v: &Value| match v {
        Value::Str(s) => Some(s.to_string()),
        // A path is text: an atom (decision 0270) is none.
        Value::Int(_) | Value::Branded(_) => None,
    };
    let made: std::collections::HashSet<String> = rows.get("document").into_iter().flatten().filter_map(|r| r.first().and_then(text)).collect();
    let documents: Vec<(String, String, String)> = files.iter().filter(|(p, _, _, _)| made.contains(p)).map(|(p, _, g, c)| (p.clone(), g.clone(), c.clone())).collect();
    let mut libraries: Vec<(String, String, String, String)> = rows
        .get("library")
        .into_iter()
        .flatten()
        .filter_map(|r| match r.as_slice() {
            [p, m, g, c] => Some((text(p)?, text(m)?, text(g)?, text(c)?)),
            _ => None,
        })
        .collect();
    libraries.sort();
    libraries.dedup_by(|a, b| a.0 == b.0);
    let exports: Vec<(String, usize)> = declared.exports.iter().map(|e| (e.clone(), rows.get(e).map_or(0, Vec::len))).collect();
    let one = PhaseOne { root: root.to_path_buf(), label: label.to_owned(), documents, libraries, rows };
    if let Some(k) = key {
        Batch::put(k, (one.clone(), exports.clone()));
    }
    Ok((one, PhaseRun { name: NAME.to_owned(), report, exports }))
}

/// Whether phase program `program`'s fixed point, or an earlier phase's it reads, loads documents (a `load` rule,
/// decision 0065 §2a; `seen`: the phases looked at, so a cycle, refused when the phases run, ends here).
fn loads_documents(program: &Version, dir: Option<&Path>, seen: &mut BTreeSet<String>) -> Result<bool> {
    let shape = super::shape::Shape::of(program);
    if shape.made.contains("load") {
        return Ok(true);
    }
    for (name, _) in wanted_in(program, &shape, dir)? {
        if !seen.insert(name.clone()) {
            continue;
        }
        let src = source(&name, dir)?.expect("a phase program");
        if loads_documents(&*lowered(&src)?, dir, seen)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// A phase's exports, as its run wrote them in `out` (which goes): each row's values as the export's columns are
/// declared, read from the outputs' image (`output::Outputs`), nothing parsed.
fn read_exports(name: &str, program: &(impl ProgramFields + ?Sized), declared: &Declared, bytes: &[u8]) -> Result<BTreeMap<String, Vec<Vec<Value>>>> {
    use crate::code_rule_dsl::logic::plan::output;
    let outputs = output::Outputs::of(bytes).map_err(|e| Error::Plan(format!("phase {name}: {e}")))?;
    let mut rows: BTreeMap<String, Vec<Vec<Value>>> = BTreeMap::new();
    for export in &declared.exports {
        let rel = outputs.get(export).ok_or_else(|| Error::Plan(format!("phase {name}: {export}: not in its outputs ({:?})", outputs.names())))?;
        let declared_types = program.columns_of(export).ok_or_else(|| Error::Plan(format!("phase {name}: {export}: not declared")))?;
        let types: Vec<&str> = declared_types.iter().map(|(_, t)| t.as_str()).collect();
        if rel.width() != types.len() {
            return Err(Error::Plan(format!("phase {name}: {export}: {} columns, not {}", rel.width(), types.len())));
        }
        let mut read = Vec::with_capacity(rel.len());
        for row in rel.rows() {
            let values = row
                .values()
                .zip(&types)
                .map(|(v, t)| match (v, *t) {
                    (output::Value::Text(s), _) => Ok(Value::Str(s.to_owned().into())),
                    (v, "symbol" | "sym") => Err(Error::Plan(format!("phase {name}: {export}: a symbol column holds {v:?}"))),
                    (v, _) => v.int().map(Value::Int).ok_or_else(|| Error::Plan(format!("phase {name}: {export}: not a number: {v:?}"))),
                })
                .collect::<Result<Vec<_>>>()?;
            read.push(values);
        }
        rows.insert(export.clone(), read);
    }

    Ok(rows)
}

/// The rows of the exports `reads` names, as the reader's relations' values.
fn hand_on(reads: &[(String, String)], rows: &BTreeMap<String, Vec<Vec<Value>>>, facts: &mut Facts) {
    for (rel, export) in reads {
        facts.entry(rel.clone()).or_default().extend(rows.get(export).into_iter().flatten().cloned());
    }
}

// Its tests run in the engine's test binary (its facade of this module mounts `tests.rs`): they read what the
// engine registers as it loads (the languages, their `.rerail` libraries).
