//! Phases over separate fact spaces (decision 0083 §1).
//!
//! A phase is a `.rr` program that names itself and the relations it hands on:
//!
//! ```text
//! // typing/programs/struct.rr                    // a later phase, or any rule
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

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::super::{Error, Result};
use super::{Report, Typing};
use crate::code_rule_dsl::logic::front::{Literal, Program, Value};
use crate::code_rule_dsl::typing::file_local::Binding;

const SEP: &str = "__";

thread_local! {
    /// The phases whose runs are under way on this thread, outermost first: a phase reading a later phase's export
    /// runs that phase inside its own run, so a phase met again is a cycle.
    static RUNNING: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

thread_local! {
    /// While a phase runs, what it reads from earlier phases: worked out once (`facts`), and taken as it is by the
    /// phase's own run, which would otherwise run those phases again.
    static HANDED_DOWN: std::cell::RefCell<Vec<(String, Facts)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Phase 1's name (`sources.rr`): run first, once for a run, by the runner (`corpus::run_observed`), not as a phase a
/// program's reads start.
pub(super) const SOURCES: &str = "sources";

/// What phase 1 handed on, for the run that ran it and every phase run inside that run: its documents are theirs.
pub(super) struct PhaseOne {
    /// The tree it read, and its grammar: a run of another tree or grammar (phase 0's) does not take its documents.
    pub(super) root: PathBuf,
    pub(super) label: String,
    /// The source files it made documents of, `(path, grammar, content)`, in the order given.
    pub(super) documents: Vec<(String, String, String)>,
    /// The library documents the imports reached, `(canonical path, module, grammar, content)`.
    pub(super) libraries: Vec<(String, String, String, String)>,
    /// Its exports, as a reader of `sources__<export>` takes them.
    rows: BTreeMap<String, Vec<Vec<String>>>,
}

thread_local! {
    static PHASE_ONE: std::cell::RefCell<Option<std::rc::Rc<PhaseOne>>> = const { std::cell::RefCell::new(None) };
}

/// Holds phase 1's result for the run until dropped.
pub(super) struct PhaseOneHeld(Option<std::rc::Rc<PhaseOne>>);
impl PhaseOneHeld {
    pub(super) fn enter(one: PhaseOne) -> PhaseOneHeld {
        PhaseOneHeld(PHASE_ONE.with(|p| p.borrow_mut().replace(std::rc::Rc::new(one))))
    }
}
impl Drop for PhaseOneHeld {
    fn drop(&mut self) {
        PHASE_ONE.with(|p| *p.borrow_mut() = self.0.take());
    }
}

/// Phase 1's result, while a run that ran it is under way on this thread, if it read `root` in `label`.
pub(super) fn phase_one(root: &Path, label: &str) -> Option<std::rc::Rc<PhaseOne>> {
    PHASE_ONE.with(|p| p.borrow().clone()).filter(|o| o.root == root && o.label == label)
}

/// Hands phase `name`'s upstream facts to its run until dropped.
struct HandedDown;
impl HandedDown {
    fn enter(name: &str, facts: Facts) -> HandedDown {
        HANDED_DOWN.with(|h| h.borrow_mut().push((name.to_owned(), facts)));
        HandedDown
    }
}
impl Drop for HandedDown {
    fn drop(&mut self) {
        HANDED_DOWN.with(|h| h.borrow_mut().pop());
    }
}

/// Marks `name`'s run under way until dropped.
struct Running;
impl Running {
    fn enter(name: &str) -> Result<Running> {
        RUNNING.with(|r| {
            let mut r = r.borrow_mut();
            if r.iter().any(|n| n == name) {
                return Err(Error::Plan(format!("phases read each other: {} -> {name}", r.join(" -> "))));
            }
            r.push(name.to_owned());
            Ok(Running)
        })
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        RUNNING.with(|r| {
            r.borrow_mut().pop();
        });
    }
}

/// What a program says of itself as a phase: its name and its exports.
pub struct Declared {
    pub(crate) name: String,
    pub exports: Vec<String>,
}

/// The phase `prog` declares (`.phase NAME`, `.export a, b`), if any.
pub fn declared(prog: &Program) -> Option<Declared> {
    let setting = |key: &str| prog.settings.iter().filter(|(k, _)| k == key).map(|(_, v)| v.trim().to_owned()).collect::<Vec<_>>();
    let name = setting("phase").into_iter().next()?;
    let list = |key: &str| setting(key).iter().flat_map(|v| v.split(',').map(|e| e.trim().trim_end_matches('.').to_owned()).collect::<Vec<_>>()).filter(|e| !e.is_empty()).collect::<Vec<_>>();
    Some(Declared { name, exports: list("export") })
}

/// The phase program `name`, with its `#include`s read in: `<phase_dir>/<name>.rr` when the run's options name a
/// `phase_dir`, else `typing/programs/<name>.rr`.
fn source(name: &str, dir: Option<&Path>) -> Result<Option<String>> {
    let Some(path) = locate(name, dir) else {
        return Ok(None);
    };
    crate::code_rule_dsl::logic::settings::program_text(&path).map(Some).map_err(Error::Plan)
}

/// The file of phase program `name` (`source`).
fn locate(name: &str, dir: Option<&Path>) -> Option<PathBuf> {
    let builtin = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).join("src/code_rule_dsl/typing/programs");
    dir.into_iter().chain([builtin.as_path()]).map(|d| d.join(format!("{name}.rr"))).find(|p| p.exists())
}

/// The files of every phase program `programs` read, and of the phases those read: each phase's `.rr` and its
/// `#include`s. A run of a phased program is a function of them too, so the whole-run cutoff names them
/// (O-cutoff-phases).
pub(super) fn phase_files(programs: &[String], dir: Option<&Path>) -> Result<Vec<PathBuf>> {
    let (mut seen, mut out) = (BTreeSet::new(), Vec::new());
    let mut todo: Vec<String> = programs.to_vec();
    while let Some(text) = todo.pop() {
        let prog = crate::code_rule_dsl::logic::rr::parse(&text)?;
        for name in wanted(&prog, dir)?.into_keys() {
            if !seen.insert(name.clone()) {
                continue;
            }
            let Some(path) = locate(&name, dir) else { continue };
            let raw = std::fs::read_to_string(&path).map_err(|e| Error::Plan(format!("{}: {e}", path.display())))?;
            out.extend(crate::code_rule_dsl::logic::rr::included_files(&raw)?);
            out.push(path);
            if let Some(src) = source(&name, dir)? {
                todo.push(src);
            }
        }
    }
    Ok(out)
}

/// The relations `prog` reads from earlier phases, by phase: `(the reader's relation, the export)`. A relation named
/// `<phase>__<x>` that no rule of `prog` makes, where `<phase>` is a phase program.
pub(super) fn wanted(prog: &Program, dir: Option<&Path>) -> Result<BTreeMap<String, Vec<(String, String)>>> {
    let made: BTreeSet<&str> = prog.rules.iter().map(|r| r.head.rel.as_str()).collect();
    let read: BTreeSet<&str> = prog.rules.iter().flat_map(|r| r.body.iter().filter_map(Literal::atom).map(|a| a.rel.as_str())).collect();
    let own = declared(prog).map(|d| d.name);
    let mut out: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for rel in read.into_iter().filter(|r| !made.contains(r) && prog.decls.contains_key(*r)) {
        let Some((phase, export)) = rel.split_once(SEP) else { continue };
        if phase.is_empty() || export.is_empty() || own.as_deref() == Some(phase) || phase.starts_with("typing_") || phase.starts_with("rr_") {
            continue;
        }
        if source(phase, dir)?.is_some() {
            out.entry(phase.to_owned()).or_default().push((rel.to_owned(), export.to_owned()));
        }
    }
    Ok(out)
}

/// Whether `rel` names an earlier phase's export (`<phase>__<export>`): shared by every program of a run, as bridged
/// relations are (`corpus::combine` does not prefix it).
pub(crate) fn is_phased(rel: &str) -> bool {
    rel.split_once(SEP).is_some_and(|(p, e)| !p.is_empty() && !e.is_empty() && !p.starts_with("typing_") && !p.starts_with("rr_") && !e.contains(SEP))
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
pub(crate) fn facts(prog: &Program, root: &Path, scratch: &Path, modes: &str, binding: Binding, chunk_nodes: usize, typing: &Typing, options: &serde_json::Value) -> Result<(Facts, Vec<PhaseRun>)> {
    // A phase's own run: its upstream facts were worked out by the run that started it.
    if let Some(me) = declared(prog)
        && let Some(f) = HANDED_DOWN.with(|h| h.borrow().last().filter(|(n, _)| *n == me.name).map(|(_, f)| f.clone()))
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
    for (name, reads) in wanted(prog, dir.as_deref())? {
        // Phase 1 ran before any phase, for the run (`PhaseOne`): its exports are handed on as it left them. A run
        // that did not run it (given its files, or with `RERAIL_SOURCES_PHASE=off`) reads none.
        if name == SOURCES {
            let src = source(&name, dir.as_deref())?.expect("sources.rr, a builtin phase program");
            let program = crate::code_rule_dsl::logic::rr::parse(&src)?;
            let rows = PHASE_ONE.with(|p| p.borrow().as_ref().map(|o| o.rows.clone())).unwrap_or_default();
            hand_on(&name, &program, &reads, &rows, &mut facts)?;
            continue;
        }
        // An earlier phase that reads a phase of its own runs that one first, inside its run (struct, then types).
        let _running = Running::enter(&name)?;
        let src = source(&name, dir.as_deref())?.expect("a phase program");
        let program = crate::code_rule_dsl::logic::rr::parse(&src)?;
        // A later phase sees the libraries only through what earlier ones hand on (0083 §1: P3 loads nothing).
        if derives_load(prog) && loads_documents(&program, dir.as_deref(), &mut BTreeSet::new())? {
            return Err(Error::Plan(format!("a program reading phase {name}, whose fixed point loaded documents, loads documents itself: it reads the libraries through {name}'s exports")));
        }
        let declared = declared(&program).ok_or_else(|| Error::Plan(format!("{name}.rr: no `.phase {name}`")))?;
        if declared.name != name {
            return Err(Error::Plan(format!("{name}.rr says `.phase {}`", declared.name)));
        }
        for export in &declared.exports {
            let cols = program.decls.get(export).ok_or_else(|| Error::Plan(format!("{name}.rr exports {export}, which it does not declare")))?;
            if let Some((c, _)) = cols.iter().find(|(_, t)| t == "node") {
                return Err(Error::Plan(format!("{name}.rr exports {export}, whose column {c} is a node: an export names symbols, which an edit does not renumber")));
            }
            if cols.first().is_none_or(|(_, t)| t != "symbol" && t != "sym") {
                return Err(Error::Plan(format!("{name}.rr exports {export}: its first column, the row's granule, must be a symbol")));
            }
        }
        for (rel, export) in &reads {
            if !declared.exports.contains(export) {
                return Err(Error::Plan(format!("{rel}: phase {name} does not export {export} (a phase's other relations are its own)")));
            }
            let (want, have) = (&program.decls[export], &prog.decls[rel]);
            let ty = |t: &str| if t == "sym" { "symbol".to_owned() } else if t == "int" { "number".to_owned() } else { t.to_owned() };
            if want.len() != have.len() || want.iter().zip(have).any(|((_, a), (_, b))| ty(a) != ty(b)) {
                return Err(Error::Plan(format!("{rel}: {} columns, where phase {name}'s {export} has {}", have.len(), want.len())));
            }
        }
        // The phase's run: its own program, its own scratch, its exports written as its outputs. The earlier phases it
        // reads run first (struct, then types), and are listed with the reader's, in the order they ran, each once.
        let out = scratch.join(format!("phase-{name}"));
        let text = format!("{src}\n.output {}\n", declared.exports.join(", "));
        let (upstream, upstream_runs) = self::facts(&program, root, scratch, modes, binding, chunk_nodes, typing, options)?;
        for r in upstream_runs {
            if !runs.iter().any(|x: &PhaseRun| x.name == r.name) {
                runs.push(r);
            }
        }
        let _down = HandedDown::enter(&name, upstream);
        let report = {
            // The phase's own run, its jobs' spans under it (`profile::span_at`).
            let _s = crate::code_rule_dsl::logic::profile::span(|| format!("phase {name}: run"));
            super::run_with_options(&[&text], root, None, &out, modes, binding, chunk_nodes, typing, options)?
        };
        let rows = { let _s = crate::code_rule_dsl::logic::profile::span(|| format!("phase {name}: its exports read")); read_exports(&name, &program, &declared, &out)? };
        { let _s = crate::code_rule_dsl::logic::profile::span(|| format!("phase {name}: handed on")); hand_on(&name, &program, &reads, &rows, &mut facts)?; }
        let exports = declared.exports.iter().map(|e| (e.clone(), rows.get(e).map_or(0, Vec::len))).collect();
        runs.push(PhaseRun { name, report, exports });
    }
    Ok((facts, runs))
}

/// The facts handed down to the innermost phase under way on this thread (phase 1's `observer__source_file`), as text:
/// a phase's run is a function of them as of its program, so the whole-run cutoff keys its record by them too.
pub(super) fn handed_down_text() -> String {
    HANDED_DOWN.with(|h| h.borrow().last().map(|(n, f)| format!("{n}\0{f:?}")).unwrap_or_default())
}

/// Whether a phase's run is under way on this thread (its documents are its own: phase 1 does not run inside it).
pub(super) fn in_phase() -> bool {
    RUNNING.with(|r| !r.borrow().is_empty())
}

/// The innermost phase whose run is under way on this thread, if any: its facts are its own fact space's.
pub(super) fn current() -> Option<String> {
    RUNNING.with(|r| r.borrow().last().cloned())
}

/// Phase 1's program, the builtin one.
pub(super) fn sources_path() -> PathBuf {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).join("src/code_rule_dsl/typing/programs/sources.rr")
}

/// Phase 1, `sources` (decision 0104): the run's source files, each `(path, module, grammar, content digest)`, loaded
/// as documents by `sources.rr`, its own run of the same engine, with the libraries their imports reach. What it hands
/// on is each file that became a document, `(path, grammar, content)`, in the order given, and each library document:
/// the later phases' documents.
pub(super) fn sources(
    files: &[(String, String, String, String)],
    label: &str,
    root: &Path,
    scratch: &Path,
    modes: &str,
    binding: Binding,
    chunk_nodes: usize,
    typing: &Typing,
    options: &serde_json::Value,
) -> Result<(PhaseOne, PhaseRun)> {
    const NAME: &str = SOURCES;
    let _running = Running::enter(NAME)?;
    let dir = options.get("phase_dir").and_then(|v| v.as_str()).map(PathBuf::from);
    let src = source(NAME, dir.as_deref())?.expect("sources.rr, a builtin phase program");
    let program = crate::code_rule_dsl::logic::rr::parse(&src)?;
    let declared = declared(&program).ok_or_else(|| Error::Plan(format!("{NAME}.rr: no `.phase {NAME}`")))?;
    // Its documents are those it loads: it is given no file of its own, and its language is the run's.
    let text = format!("{src}
.language {label}
.output {}
", declared.exports.join(", "));
    let mut options = options.clone();
    if let Some(o) = options.as_object_mut() {
        o.insert("inputFiles".to_owned(), serde_json::Value::Array(Vec::new()));
    } else {
        options = serde_json::json!({ "inputFiles": [] });
    }
    // The observer's facts: stated, handed down as a phase's exports are (they are its inputs, not its program).
    let stated: Vec<Vec<Value>> = files.iter().map(|(p, m, g, c)| vec![Value::Str(p.clone()), Value::Str(m.clone()), Value::Str(g.clone()), Value::Str(c.clone())]).collect();
    let _down = HandedDown::enter(NAME, Facts::from([("observer__source_file".to_owned(), stated)]));
    let out = scratch.join(format!("phase-{NAME}"));
    let report = {
        let _s = crate::code_rule_dsl::logic::profile::span(|| format!("phase {NAME}: run"));
        super::run_with_options(&[&text], root, None, &out, modes, binding, chunk_nodes, typing, &options)?
    };
    let rows = { let _s = crate::code_rule_dsl::logic::profile::span(|| format!("phase {NAME}: its exports read")); read_exports(NAME, &program, &declared, &out)? };
    let made: std::collections::HashSet<&str> = rows.get("document").into_iter().flatten().filter_map(|r| r.first().map(String::as_str)).collect();
    let documents: Vec<(String, String, String)> = files.iter().filter(|(p, _, _, _)| made.contains(p.as_str())).map(|(p, _, g, c)| (p.clone(), g.clone(), c.clone())).collect();
    let mut libraries: Vec<(String, String, String, String)> = rows
        .get("library")
        .into_iter()
        .flatten()
        .filter_map(|r| match r.as_slice() {
            [p, m, g, c] => Some((p.clone(), m.clone(), g.clone(), c.clone())),
            _ => None,
        })
        .collect();
    libraries.sort();
    libraries.dedup_by(|a, b| a.0 == b.0);
    let exports = declared.exports.iter().map(|e| (e.clone(), rows.get(e).map_or(0, Vec::len))).collect();
    let one = PhaseOne { root: root.to_path_buf(), label: label.to_owned(), documents, libraries, rows };
    Ok((one, PhaseRun { name: NAME.to_owned(), report, exports }))
}

/// Whether a program loads documents into its fixed point (a `load` rule, decision 0065 §2a).
fn derives_load(prog: &Program) -> bool {
    prog.rules.iter().any(|r| r.head.rel == "load")
}

/// Whether phase program `program`'s fixed point, or an earlier phase's it reads, loads documents (`seen`: the
/// phases looked at, so a cycle, refused when the phases run, ends here).
fn loads_documents(program: &Program, dir: Option<&Path>, seen: &mut BTreeSet<String>) -> Result<bool> {
    if derives_load(program) {
        return Ok(true);
    }
    for (name, _) in wanted(program, dir)? {
        if !seen.insert(name.clone()) {
            continue;
        }
        let src = source(&name, dir)?.expect("a phase program");
        if loads_documents(&crate::code_rule_dsl::logic::rr::parse(&src)?, dir, seen)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// A phase's exports, as its run wrote them in `out` (which goes).
fn read_exports(name: &str, program: &Program, declared: &Declared, out: &Path) -> Result<BTreeMap<String, Vec<Vec<String>>>> {
    let mut rows: BTreeMap<String, Vec<Vec<String>>> = BTreeMap::new();
    for export in &declared.exports {
        let text = std::fs::read_to_string(out.join(format!("{export}.csv"))).map_err(|e| {
            let held: Vec<String> = std::fs::read_dir(out).map(|d| d.filter_map(|f| f.ok().map(|f| f.file_name().to_string_lossy().into_owned())).collect()).unwrap_or_default();
            Error::Plan(format!("phase {name}: {export}: {e} (in {}, which holds {held:?})", out.display()))
        })?;
        let width = program.decls[export].len();
        let read: Vec<Vec<String>> = text.lines().map(|l| l.split('\t').map(str::to_owned).collect()).collect();
        // Outputs are lines of tab-separated values, unescaped: a value holding a newline or a tab would come back
        // as other rows, and be handed on as facts no rule made.
        if let Some(bad) = read.iter().find(|r| r.len() != width) {
            return Err(Error::Plan(format!("phase {name}: {export}: a row of {} values, not {width} (a value holds a newline or a tab): {:?}", bad.len(), bad.join("\t"))));
        }
        rows.insert(export.clone(), read);
    }
    let _ = std::fs::remove_dir_all(out);
    Ok(rows)
}

/// The rows of the exports `reads` names, as the reader's relations' values.
fn hand_on(name: &str, program: &Program, reads: &[(String, String)], rows: &BTreeMap<String, Vec<Vec<String>>>, facts: &mut Facts) -> Result<()> {
    for (rel, export) in reads {
        let types: Vec<&str> = program.decls[export].iter().map(|(_, t)| t.as_str()).collect();
        let held = facts.entry(rel.clone()).or_default();
        for r in rows.get(export).into_iter().flatten() {
            let row = r
                .iter()
                .zip(&types)
                .map(|(v, t)| if *t == "symbol" || *t == "sym" { Ok(Value::Str(v.clone())) } else { v.parse::<i64>().map(Value::Int).map_err(|_| Error::Plan(format!("phase {name}: {export}: not a number: {v}"))) })
                .collect::<Result<Vec<_>>>()?;
            held.push(row);
        }
    }
    Ok(())
}

// Its tests run in the engine's test binary (its facade of this module mounts `tests.rs`): they read what the
// engine registers as it loads (the languages, their `.rr` libraries).
