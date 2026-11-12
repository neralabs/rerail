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
    let builtin = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).join("src/code_rule_dsl/typing/programs");
    let Some(path) = dir.into_iter().chain([builtin.as_path()]).map(|d| d.join(format!("{name}.rr"))).find(|p| p.exists()) else {
        return Ok(None);
    };
    crate::code_rule_dsl::logic::settings::program_text(&path).map(Some).map_err(Error::Plan)
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

#[cfg(test)]
pub mod tests {
    use crate::code_rule_dsl::typing::file_local::Binding;

    /// Phase 1 (`sources.rr`, decision 0104): a run's documents are those phase 1 loads, one per source file's key.
    /// After an edit, only the edited file is built again, in either phase: the others are their memos' entries.
    #[test]
    fn a_runs_documents_come_from_phase_1_and_an_edit_builds_only_its_file() {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension("sources");
        let _ = std::fs::remove_dir_all(&dir);
        let src = dir.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("a.py"), "def f():\n    return 1\n").unwrap();
        std::fs::write(src.join("b.py"), "def g():\n    return 2\n").unwrap();
        let out = dir.join("out");
        let _no_session = crate::code_rule_dsl::logic::plan::corpus::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        crate::code_rule_dsl::logic::plan::corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
        let text = crate::code_rule_dsl::logic::rr::print(
            &crate::code_rule_dsl::logic::rr::parse("#include \"core.rr\"\n.decl defs(s: symbol)\ndefs(x) :- kind(n, \"function_definition\"), field(n, \"name\", i), text(i, x).\n.output defs\n").unwrap(),
        )
        .unwrap();
        let run = || crate::code_rule_dsl::logic::plan::corpus::run_with_options(&[&text], &src, None, &out, "program", Binding::Cpu, 1 << 18, &Default::default(), &serde_json::Value::Null).unwrap();
        let defs = || std::fs::read_to_string(out.join("defs.csv")).unwrap();
        let first = run();
        let sources = first.phase_runs.iter().find(|r| r.name == "sources").expect("phase 1 ran");
        assert_eq!(sources.exports, vec![("document".to_owned(), 2), ("library".to_owned(), 0)], "each source file is a document of phase 1's, and no library is set");
        assert_eq!(defs(), "f\ng\n");
        std::fs::write(src.join("a.py"), "def h():\n    return 1\n").unwrap();
        let second = run();
        assert_eq!(defs(), "g\nh\n", "the findings follow the edit");
        let sources = second.phase_runs.iter().find(|r| r.name == "sources").expect("phase 1 ran");
        assert_eq!(sources.report.memo.0, 1, "phase 1 builds only the edited file");
        assert_eq!(second.memo.0, 1, "so does the run");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Phase 1 resolves the imports to their fixed point: a library an import reaches is loaded, and its own imports in
    /// turn. A program that reads `sources__library` takes those documents as its own loaded documents, and loads
    /// nothing. An edit of a library file is followed; the tree's file is not built again.
    #[test]
    fn phase_1_loads_the_libraries_the_imports_reach_and_a_reader_takes_them() {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension("sources-libraries");
        let _ = std::fs::remove_dir_all(&dir);
        let (src, lib) = (dir.join("src"), dir.join("ts/stdlib"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(src.join("rerail.env"), "python.typeshed = ../ts\n").unwrap();
        std::fs::write(src.join("m.py"), "import a\n\ndef own():\n    return 1\n").unwrap();
        std::fs::write(lib.join("VERSIONS"), "builtins: 3.0-\na: 3.0-\nb: 3.0-\nc: 3.0-\n").unwrap();
        std::fs::write(lib.join("builtins.pyi"), "class object: ...\n").unwrap();
        std::fs::write(lib.join("a.pyi"), "import b\ndef fa() -> None: ...\n").unwrap();
        std::fs::write(lib.join("b.pyi"), "def fb() -> None: ...\n").unwrap();
        std::fs::write(lib.join("c.pyi"), "def fc() -> None: ...\n").unwrap();
        let out = dir.join("out");
        let _no_session = crate::code_rule_dsl::logic::plan::corpus::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        crate::code_rule_dsl::logic::plan::corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
        let reader = "#include \"core.rr\"\n.decl sources__library(p: symbol, m: symbol, g: symbol, c: symbol)\n.decl lib(m: symbol)\nlib(m) :- sources__library(_, m, _, _).\n.decl defs(s: symbol)\ndefs(x) :- kind(n, \"function_definition\"), field(n, \"name\", i), text(i, x).\n.output lib, defs\n";
        let text = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse(reader).unwrap()).unwrap();
        let run = || crate::code_rule_dsl::logic::plan::corpus::run_with_options(&[&text], &src, None, &out, "program", Binding::Cpu, 1 << 18, &Default::default(), &serde_json::Value::Null).unwrap();
        let rows = |rel: &str| {
            let mut v: Vec<String> = std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap().lines().map(str::to_owned).collect();
            v.sort();
            v
        };
        let first = run();
        assert_eq!(first.phase_runs.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["sources"]);
        let sources = first.phase_runs.iter().find(|r| r.name == "sources").expect("phase 1 ran");
        assert_eq!(sources.exports, vec![("document".to_owned(), 1), ("library".to_owned(), 3)], "m, and builtins, a and b: c is imported by none");
        assert_eq!(rows("lib"), ["a", "b", "builtins"]);
        assert_eq!(rows("defs"), ["fa", "fb", "own"], "the libraries are the reader's documents");
        std::fs::write(lib.join("b.pyi"), "import c\ndef fb2() -> None: ...\n").unwrap();
        let second = run();
        assert_eq!(rows("lib"), ["a", "b", "builtins", "c"], "the edited library's import is followed");
        assert_eq!(rows("defs"), ["fa", "fb2", "fc", "own"]);
        let sources = second.phase_runs.iter().find(|r| r.name == "sources").expect("phase 1 ran");
        assert_eq!(sources.report.memo.0, 3, "phase 1 reads m, builtins and a from the memo, and builds only the edited library and the new one");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A phase runs in its own space: a reader sees its exports and nothing else, and an export naming a node is
    /// refused. The phase keeps nothing of its own: across runs, its run keeps its relations as any run does, and the
    /// reader's rows follow every edit.
    #[test]
    fn a_phase_hands_on_its_exports_and_its_reader_follows_each_edit() {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension("phases");
        let _ = std::fs::remove_dir_all(&dir);
        let (src, phases) = (dir.join("src"), dir.join("phases"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&phases).unwrap();
        let write = |name: &str, text: &str| std::fs::write(src.join(name), text).unwrap();
        write("a.py", "def f():\n    return 1\n\ndef g():\n    return 2\n");
        let phase = |exports: &str, extra: &str| {
            std::fs::write(phases.join("p1.rr"), format!(r#"
#include "core.rr"
.phase p1
.export {exports}
.decl unit(s: symbol, h: symbol)
unit("m", h) :- def_digest(n, h), kind(n, "module").
unit(@cat("m", x), h) :- def_digest(n, h), field(n, "name", i), text(i, x).
.decl def(s: symbol, k: symbol)
def(@cat("m", x), k) :- kind(n, k), k = "function_definition", field(n, "name", i), text(i, x).
def("m", "module") :- kind(_, "module").
.decl private(s: symbol)
private(s) :- def(s, _).
.decl at(n: node, s: symbol)
at(n, s) :- kind(n, "function_definition"), field(n, "name", i), text(i, s).
{extra}
"#)).unwrap();
        };
        let reader = |reads: &str| format!("{reads}\n.decl r(s: symbol)\nr(s) :- p1__def(s, _).\n.output r\n");
        let out = dir.join("out");
        let _no_session = crate::code_rule_dsl::logic::plan::corpus::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        crate::code_rule_dsl::logic::plan::corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
        let options = serde_json::json!({ "phase_dir": phases.to_string_lossy() });
        let run = |text: &str| crate::code_rule_dsl::logic::plan::corpus::run_with_options(&[text], &src, None, &out, "program", Binding::Cpu, 1 << 18, &Default::default(), &options);
        let print = |text: &str| crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse(text).unwrap()).unwrap();
        // A private relation is not the reader's to read.
        phase("unit, def", "");
        let err = run(&print(&reader(".decl p1__def(s: symbol, k: symbol)\n.decl p1__private(s: symbol)\n.decl q(s: symbol)\nq(s) :- p1__private(s).\n.output q"))).err().expect("a private relation refused").to_string();
        assert!(err.contains("does not export private"), "{err}");
        // An export naming a node is refused.
        phase("unit, def, at", "");
        let err = run(&print(&reader(".decl p1__def(s: symbol, k: symbol)"))).err().expect("a node export refused").to_string();
        assert!(err.contains("is a node"), "{err}");
        phase("unit, def", "");
        let text = print(&reader(".decl p1__def(s: symbol, k: symbol)"));
        let first = run(&text).unwrap();
        let r = || std::fs::read_to_string(out.join("r.csv")).unwrap();
        assert_eq!(r(), "m\nm.f\nm.g\n");
        assert_eq!(first.phase_runs.iter().find(|r| r.name == "p1").expect("p1 ran").exports, [("unit".to_owned(), 3), ("def".to_owned(), 3)]);
        assert_eq!(first.files, 0, "the reader reads only the phase's exports, so no file");
        // Unchanged, a comment, a body: the exports the reader reads are as they were.
        run(&text).unwrap();
        assert_eq!(r(), "m\nm.f\nm.g\n");
        write("a.py", "# note\ndef f():\n    x = 1\n    return x\n\n\ndef g():  # g\n    return 2\n");
        run(&text).unwrap();
        assert_eq!(r(), "m\nm.f\nm.g\n");
        // A function added, then one removed: the phase's run follows, and so does the reader.
        write("a.py", "def f():\n    return 1\n\ndef g():\n    return 2\n\ndef h():\n    pass\n");
        run(&text).unwrap();
        assert_eq!(r(), "m\nm.f\nm.g\nm.h\n");
        write("a.py", "def f():\n    return 1\n\ndef h():\n    pass\n");
        run(&text).unwrap();
        assert_eq!(r(), "m\nm.f\nm.h\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Phases chain: a phase may read an earlier phase's exports (0083: struct, then types, then the analysis), and
    /// the reader sees only the last phase's. Phases that read each other are refused, not run forever.
    #[test]
    fn a_phase_reads_an_earlier_phase_and_a_cycle_is_refused() {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension("phases-chain");
        let _ = std::fs::remove_dir_all(&dir);
        let (src, phases) = (dir.join("src"), dir.join("phases"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&phases).unwrap();
        std::fs::write(src.join("a.py"), "class A:\n    pass\n\nclass B(A):\n    pass\n").unwrap();
        let p1 = r#"
#include "core.rr"
.phase p1
.export def, base
.decl def(s: symbol, k: symbol)
def(x, "class") :- kind(n, "class_definition"), field(n, "name", i), text(i, x).
.decl base(s: symbol, b: symbol)
base(x, b) :- kind(n, "class_definition"), field(n, "name", i), text(i, x), field(n, "superclasses", l), child(l, e, _), text(e, b).
"#;
        // The typing phase reads only p1's exports: the classes' ancestors, as a later phase would read them.
        let p2 = r#"
.phase p2
.export anc
.decl p1__def(s: symbol, k: symbol)
.decl p1__base(s: symbol, b: symbol)
.decl anc(s: symbol, a: symbol)
anc(s, a) :- p1__base(s, a), p1__def(a, _).
anc(s, a) :- anc(s, b), p1__base(b, a).
"#;
        std::fs::write(phases.join("p1.rr"), p1).unwrap();
        std::fs::write(phases.join("p2.rr"), p2).unwrap();
        let out = dir.join("out");
        let _no_session = crate::code_rule_dsl::logic::plan::corpus::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        crate::code_rule_dsl::logic::plan::corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
        let options = serde_json::json!({ "phase_dir": phases.to_string_lossy() });
        let run = |text: &str| crate::code_rule_dsl::logic::plan::corpus::run_with_options(&[text], &src, None, &out, "program", Binding::Cpu, 1 << 18, &Default::default(), &options);
        let reader = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse("#include \"core.rr\"\n.decl p2__anc(s: symbol, a: symbol)\n.decl r(s: symbol, a: symbol)\nr(s, a) :- p2__anc(s, a).\n.output r\n").unwrap()).unwrap();
        let report = run(&reader).unwrap();
        assert_eq!(std::fs::read_to_string(out.join("r.csv")).unwrap(), "B\tA\n");
        // Phase 1 (`sources`) first, which gives every run its documents, then both phases, listed flat, in the order
        // they ran; p2's own run checked p1 once, not again.
        let names: Vec<&str> = report.phase_runs.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["sources", "p1", "p2"]);
        assert!(report.phase_runs[2].report.phase_runs.is_empty(), "p1 is not checked again inside p2's run");
        // Nothing changed: the same rows.
        run(&reader).unwrap();
        assert_eq!(std::fs::read_to_string(out.join("r.csv")).unwrap(), "B\tA\n");
        // p1 changed (its program: no class has a base now): p2, which reads it, is not kept.
        std::fs::write(phases.join("p1.rr"), p1.replace("text(e, b).", "text(e, b), b = \"none\".")).unwrap();
        run(&reader).unwrap();
        assert_eq!(std::fs::read_to_string(out.join("r.csv")).unwrap(), "", "p2 run again over p1's new exports");
        // A reader of p2 that loads documents itself is refused: p1 loaded none, so first give p1 a load.
        std::fs::write(phases.join("p1.rr"), format!("{p1}\nload(\"x.pyi\", \"x\", \"python\") :- def(\"A\", _).\n")).unwrap();
        let loading = reader.replace(".output r", "load(\"y.pyi\", \"y\", \"python\") :- r(_, _).\n.output r");
        let err = run(&loading).err().expect("a later phase's reader that loads").to_string();
        assert!(err.contains("loads documents itself"), "{err}");
        // p1 reading p2's export: each runs the other first.
        std::fs::write(phases.join("p1.rr"), format!("{p1}\n.decl p2__anc(s: symbol, a: symbol)\n.decl seen(s: symbol)\nseen(s) :- p2__anc(s, _).\n.output seen\n")).unwrap();
        let err = run(&reader).err().expect("a cycle refused").to_string();
        assert!(err.contains("phases read each other"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A reader with a constant of its own (made from the run's settings alone, folded first by `constants`) reads a
    /// phase: the constants' run starts no phase, so the phase runs once, as itself, and writes its exports.
    #[test]
    fn a_reader_with_constants_runs_its_phases_once_as_themselves() {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension("phases-constants");
        let _ = std::fs::remove_dir_all(&dir);
        let (src, phases) = (dir.join("src"), dir.join("phases"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&phases).unwrap();
        std::fs::write(src.join("a.py"), "class A:\n    pass\n").unwrap();
        std::fs::write(phases.join("p1.rr"), "#include \"core.rr\"\n.phase p1\n.export def\n.decl def(s: symbol, k: symbol)\ndef(x, \"class\") :- kind(n, \"class_definition\"), field(n, \"name\", i), text(i, x).\n").unwrap();
        let out = dir.join("out");
        let _no_session = crate::code_rule_dsl::logic::plan::corpus::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        crate::code_rule_dsl::logic::plan::corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
        let options = serde_json::json!({ "phase_dir": phases.to_string_lossy(), "env": ["mode=strict"] });
        let reader = "#include \"core.rr\"\n.decl mode(m: symbol)\nmode(m) :- env(\"mode\", m).\n.decl p1__def(s: symbol, k: symbol)\n.decl r(s: symbol, m: symbol)\nr(s, m) :- p1__def(s, _), mode(m), kind(_, \"module\").\n.output r\n";
        let reader = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse(reader).unwrap()).unwrap();
        let report = crate::code_rule_dsl::logic::plan::corpus::run_with_options(&[&reader], &src, None, &out, "program", Binding::Cpu, 1 << 18, &Default::default(), &options).unwrap();
        assert_eq!(std::fs::read_to_string(out.join("r.csv")).unwrap(), "A\tstrict\n");
        assert_eq!(report.phase_runs.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["sources", "p1"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
