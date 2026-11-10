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
//! run of the same engine, in its own fact space (its program, its rules, its scratch store), to its fixed point.
//! The reader sees only the exports, and only once that run has ended. A relation the phase does not export cannot
//! be read: it is not in the reader's space.
//!
//! **Exports name symbols, never nodes.** A node id names a position in one version of a file, so an edit renumbers
//! it; an export with a `node` column is refused. Each export row is about its first column, a symbol: its *granule*.
//! A phase that also exports `unit(s, h)`, each definition it read with its own-code digest (`def_digest`), keeps
//! freshness:
//! - each definition is a name in the store's derivation graph (`content_store::derivation`), bound to its digest;
//!   an edit rebinds the definitions whose digest changed, and only those;
//! - each granule is a derivation that consulted its definition (`unit` row of the same symbol), and its output is
//!   bound to the digest of its rows. A granule whose rows did not change rebinds nothing: the early cutoff;
//! - a granule that changed although its definition did not is *unattributed*: it consulted something its record does
//!   not name. Each run counts them, so the record is checked rather than trusted.
//!
//! The exports are kept by granule in the phase store, `<content store>.phases`, and each run's rows are compared
//! with the last run's: the delta the next phase takes.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};

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
    /// While a phase runs, what it reads from earlier phases: worked out once to key it (`facts`), and taken as it is
    /// by the phase's own run, which would otherwise check those phases again.
    static HANDED_DOWN: std::cell::RefCell<Vec<(String, Facts, Deltas)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Hands phase `name`'s upstream facts to its run until dropped.
struct HandedDown;
impl HandedDown {
    fn enter(name: &str, facts: Facts, deltas: Deltas) -> HandedDown {
        HANDED_DOWN.with(|h| h.borrow_mut().push((name.to_owned(), facts, deltas)));
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

/// The export a phase names its definitions in, with their digests.
const UNITS: &str = "unit";

/// What a program says of itself as a phase: its name and its exports.
pub(crate) struct Declared {
    pub(crate) name: String,
    pub(crate) exports: Vec<String>,
    /// Exports that say where things are (`.positions at`): handed on, but no part of a granule's digest, since an
    /// edit moves every later line.
    pub(crate) positions: Vec<String>,
    /// The export that names each granule's document (`.owner at`: its first column the granule, its second the
    /// document's path): a document's rows can then be replaced on their own (`maintain`).
    pub(crate) owner: Option<String>,
}

/// The phase `prog` declares (`.phase NAME`, `.export a, b`), if any.
pub(crate) fn declared(prog: &Program) -> Option<Declared> {
    let setting = |key: &str| prog.settings.iter().filter(|(k, _)| k == key).map(|(_, v)| v.trim().to_owned()).collect::<Vec<_>>();
    let name = setting("phase").into_iter().next()?;
    let list = |key: &str| setting(key).iter().flat_map(|v| v.split(',').map(|e| e.trim().trim_end_matches('.').to_owned()).collect::<Vec<_>>()).filter(|e| !e.is_empty()).collect::<Vec<_>>();
    Some(Declared { name, exports: list("export"), positions: list("positions"), owner: list("owner").into_iter().next() })
}

/// The phase program `name`, with its `#include`s read in: `<phase_dir>/<name>.rr` when the run's options name a
/// `phase_dir`, else `typing/programs/<name>.rr`.
fn source(name: &str, dir: Option<&Path>) -> Result<Option<String>> {
    let builtin = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/code_rule_dsl/typing/programs");
    let Some(path) = dir.into_iter().chain([builtin.as_path()]).map(|d| d.join(format!("{name}.rr"))).find(|p| p.exists()) else {
        return Ok(None);
    };
    crate::code_rule_dsl::tools::program_text(&path).map(Some).map_err(Error::Plan)
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
pub(crate) struct PhaseRun {
    pub(crate) name: String,
    pub(crate) report: Report,
    /// Each export's rows, and of them the rows not in the last run's and the last run's rows gone.
    pub(crate) exports: Vec<(String, usize, usize, usize)>,
    /// Freshness: definitions, those whose digest changed (new ones included), and those gone.
    pub(crate) units: (usize, usize, usize),
    /// Granules, those whose rows changed, those the changed definitions reach (the frontier), and changed granules
    /// outside it (unattributed).
    pub(crate) granules: (usize, usize, usize, usize),
    /// The CPU the freshness bookkeeping took (store, digests, deltas), seconds on this thread.
    pub(crate) bookkeeping: f64,
    /// Why the phase ran in full, when its fixed point could not be maintained: by the phase store (`maintain`), or
    /// by a session's run from its inputs' deltas.
    pub(crate) full_because: Option<String>,
    /// Each export's version, and its delta when this run made one.
    pub(crate) versions: Deltas,
}

pub(crate) type Facts = BTreeMap<String, Vec<Vec<Value>>>;

/// The store a phase keeps its exports in, beside the content store: `<content store>.phases`.
pub(super) fn store_path() -> Option<PathBuf> {
    super::tape_cache_path().map(|p| PathBuf::from(format!("{}.phases", p.display())))
}

/// The phase store, open, in its current form. Its facts are kept a granule at a time, never a row per fact: memory
/// holds what is fast, and SQLite takes each run's changes as one wave of few statements.
///
/// - `phase_granules`: one row per granule, its rows of every export as one text (`granule_text`), and the document
///   its `.owner` row names. Written, read and deleted by granule lists (`json_each`), one statement a wave.
/// - `phase_cones`: one row per phase, each document's cone digest (`Report::cone_inputs`) as one JSON text.
///
/// A store in the earlier form (a row per export row, per definition, per document) is emptied once: its phases
/// run in full again.
fn open_store(path: &Path) -> Result<Connection> {
    let db = Connection::open(path)?;
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=OFF;")?;
    let old: i64 = db.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name='export_rows'", [], |r| r.get(0))?;
    if old > 0 {
        db.execute_batch(
            "DROP TABLE IF EXISTS export_rows; DROP TABLE IF EXISTS phase_units; DROP TABLE IF EXISTS phase_documents;
             DROP TABLE IF EXISTS phase_record; DROP TABLE IF EXISTS export_versions; DROP TABLE IF EXISTS store_epoch;",
        )?;
    }
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS phase_granules(phase TEXT, granule TEXT, doc TEXT, rows TEXT NOT NULL, PRIMARY KEY(phase, granule)) WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS phase_granules_doc ON phase_granules(phase, doc);
         CREATE TABLE IF NOT EXISTS phase_cones(phase TEXT PRIMARY KEY, cones TEXT NOT NULL) WITHOUT ROWID;",
    )?;
    records(&db)?;
    versions_table(&db)?;
    Ok(db)
}

/// One granule's rows, by export: what `phase_granules.rows` holds.
type Granule = BTreeMap<String, Vec<Vec<String>>>;

/// A granule's rows as stored: JSON, exports in order and each export's rows sorted, so equal rows are equal text.
fn granule_text(g: &Granule) -> String {
    let sorted: BTreeMap<&String, Vec<&Vec<String>>> = g.iter().map(|(rel, rs)| {
        let mut rs: Vec<&Vec<String>> = rs.iter().collect();
        rs.sort();
        rs.dedup();
        (rel, rs)
    }).collect();
    serde_json::to_string(&sorted).expect("rows of text")
}

fn granule_of(text: &str) -> Result<Granule> {
    serde_json::from_str(text).map_err(|e| Error::Plan(format!("phase store: a granule's rows: {e}")))
}

/// The stored granules of `phase`: those named (`Some`), or all of them; each with its rows.
fn read_granules(db: &Connection, phase: &str, named: Option<&BTreeSet<String>>) -> Result<BTreeMap<String, (String, Granule)>> {
    let mut out = BTreeMap::new();
    let mut take = |r: &rusqlite::Row| -> rusqlite::Result<(String, String)> { Ok((r.get(0)?, r.get(1)?)) };
    let got: Vec<(String, String)> = match named {
        Some(names) => {
            let list = serde_json::to_string(names).expect("names");
            db.prepare("SELECT granule, rows FROM phase_granules WHERE phase=?1 AND granule IN (SELECT value FROM json_each(?2))")?.query_map(params![phase, list], &mut take)?.collect::<rusqlite::Result<_>>()?
        }
        None => db.prepare("SELECT granule, rows FROM phase_granules WHERE phase=?1")?.query_map([phase], &mut take)?.collect::<rusqlite::Result<_>>()?,
    };
    for (g, text) in got {
        let rows = granule_of(&text)?;
        out.insert(g, (text, rows));
    }
    Ok(out)
}

fn thread_cpu() -> f64 {
    let (u, s) = super::thread_usage();
    u + s
}

/// The facts `prog` reads from earlier phases, by relation, and each phase's run. `scratch` holds the phases' runs
/// (`<scratch>/phase-<name>`).
///
/// `held` is the version of a reader's relation the reader holds already (a session's, bound): when that is this
/// version, or the one its delta is from, no row of it is read or handed on, only its version and delta.
#[allow(clippy::too_many_arguments)]
pub(crate) fn facts(prog: &Program, root: &Path, scratch: &Path, modes: &str, binding: Binding, chunk_nodes: usize, typing: &Typing, options: &serde_json::Value, held: &dyn Fn(&str) -> Option<u64>) -> Result<(Facts, Vec<PhaseRun>, Deltas)> {
    // A phase's own run: its upstream facts were worked out to key it, and its upstream runs are reported there.
    if let Some(me) = declared(prog)
        && let Some((f, d)) = HANDED_DOWN.with(|h| h.borrow().last().filter(|(n, _, _)| *n == me.name).map(|(_, f, d)| (f.clone(), d.clone())))
    {
        return Ok((f, Vec::new(), d));
    }
    let (mut facts, mut runs, mut handed) = (Facts::new(), Vec::new(), Deltas::new());
    // The constants' own run (`constants::derive`) reads no phase: its relations are made from no document, and a
    // phase's exports are the run's inputs, never a constant. Run under its options, a phase would be a constants
    // run itself, writing none of its exports.
    if options.get(super::constants::OWN).is_some() {
        return Ok((facts, runs, handed));
    }
    let dir = options.get("phase_dir").and_then(|v| v.as_str()).map(PathBuf::from);
    for (name, reads) in wanted(prog, dir.as_deref())? {
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
        if let Some(owner) = &declared.owner {
            let cols = declared.exports.contains(owner).then(|| &program.decls[owner]).ok_or_else(|| Error::Plan(format!("{name}.rr: `.owner {owner}`, which it does not export")))?;
            if cols.get(1).is_none_or(|(_, t)| t != "symbol" && t != "sym") {
                return Err(Error::Plan(format!("{name}.rr: `.owner {owner}`: its second column, the granule's document, must be a symbol")));
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
        // The phase's run: its own program, its own scratch, its exports written as its outputs.
        let out = scratch.join(format!("phase-{name}"));
        let text = format!("{src}\n.output {}\n", declared.exports.join(", "));
        // The earlier phases this one reads, run (or kept) first: their exports are part of what it consulted, so an
        // upstream change is a new key (types over struct: a changed struct export runs types again).
        // Their runs are listed with the reader's, in the order they ran (struct, types, …), each once.
        let (upstream, upstream_runs, upstream_handed) = self::facts(&program, root, scratch, modes, binding, chunk_nodes, typing, options, &|_| None)?;
        let key = run_key(&format!("{text}\n{}", digest_facts(&upstream, &upstream_handed)), root, options);
        for r in upstream_runs {
            if !runs.iter().any(|x: &PhaseRun| x.name == r.name) {
                runs.push(r);
            }
        }
        let _down = HandedDown::enter(&name, upstream, upstream_handed);
        // Unchanged since its last run (everything it consulted as it was): its exports are the last run's, kept.
        let checking = super::thread_usage();
        if let Some(documents) = reuse(&name, &key, &text, root, typing, options)? {
            let versions = current_versions(&name, &declared.exports)?;
            let rows = { let _s = crate::code_rule_dsl::logic::profile::span(|| format!("phase {name}: its kept exports read")); stored_rows(&name, &wanted_rows(&reads, &versions, held))? };
            { let _s = crate::code_rule_dsl::logic::profile::span(|| format!("phase {name}: handed on")); hand_on(&name, &program, &reads, &rows, &mut facts)?; }
            hand_versions(&reads, &versions, &mut handed);
            let now = super::thread_usage();
            let mut report = Report::default();
            report.phases.push(("unchanged: consulted inputs checked, exports kept", now.0 - checking.0, now.1 - checking.1, super::usage().2, 0));
            report.files = documents;
            // The counts the store keeps: no row is read for them. Granules are the definitions (`unit`'s rows).
            let units = versions.get(UNITS).map_or(0, |v| v.rows);
            let (exports, units, granules) = (versions.iter().map(|(rel, v)| (rel.clone(), v.rows, 0, 0)).collect(), (units, 0, 0), (units, 0, 0, 0));
            runs.push(PhaseRun { name, report, exports, units, granules, bookkeeping: 0.0, full_because: None, versions });
            continue;
        }
        let producer: [u8; 32] = Sha256::digest(src.as_bytes()).into();
        // Some workspace documents changed, and nothing else: their rows replaced, when the rest stands.
        let job = Job { name: &name, key: &key, text: &text, root, scratch, modes, binding, chunk_nodes, typing, options, program: &program, declared: &declared, producer };
        let maintained = { let _s = crate::code_rule_dsl::logic::profile::span(|| format!("phase {name}: maintained")); maintain(&job)? };
        let full_because = match maintained {
            Ok((report, exports, units, granules, began, versions)) => {
                let rows = { let _s = crate::code_rule_dsl::logic::profile::span(|| format!("phase {name}: its exports read")); stored_rows(&name, &wanted_rows(&reads, &versions, held))? };
                { let _s = crate::code_rule_dsl::logic::profile::span(|| format!("phase {name}: handed on")); hand_on(&name, &program, &reads, &rows, &mut facts)?; }
                hand_versions(&reads, &versions, &mut handed);
                runs.push(PhaseRun { name, report, exports, units, granules, bookkeeping: thread_cpu() - began, full_because: None, versions });
                continue;
            }
            Err(why) => why,
        };
        let report = {
            // The phase's own run, its workers' spans under it (`profile::span_at`).
            let _s = crate::code_rule_dsl::logic::profile::span(|| format!("phase {name}: run"));
            super::run_with_options(&[&text], root, None, &out, modes, binding, chunk_nodes, typing, options)?
        };
        let began = thread_cpu();
        let rows = { let _s = crate::code_rule_dsl::logic::profile::span(|| format!("phase {name}: its exports read")); read_exports(&name, &program, &declared, &out)? };
        let ((exports, units, granules), versions) = { let _s = crate::code_rule_dsl::logic::profile::span(|| format!("phase {name}: its exports kept")); keep_in_session(&name, rows.clone(), &declared, producer)? };
        // Once its exports are kept: what it consulted, so an unchanged next run takes them as they are, and each
        // document's cone inputs, so a run of changed documents can tell whether the rest stands.
        {
            let _s = crate::code_rule_dsl::logic::profile::span(|| format!("phase {name}: what it consulted recorded"));
            record(&name, &key, &report.consulted)?;
            record_documents(&name, &report.cone_inputs, true)?;
        }
        let wanted = wanted_rows(&reads, &versions, held);
        let rows: BTreeMap<String, Vec<Vec<String>>> = rows.into_iter().filter(|(e, _)| wanted.contains(e)).collect();
        { let _s = crate::code_rule_dsl::logic::profile::span(|| format!("phase {name}: handed on")); hand_on(&name, &program, &reads, &rows, &mut facts)?; }
        hand_versions(&reads, &versions, &mut handed);
        // A session's run maintains the phase's fixed point from its inputs' deltas (an earlier phase's exports): no
        // stratum derived again, it ran in full only as far as the phase store is concerned.
        let full_because = match report.cross_strata {
            Some((ran, all)) if report.cross_maintained.2 == 0 && report.cross_maintained.0 > 0 => {
                crate::code_rule_dsl::logic::profile::decision("phase maintained (decision 0094 §4)", || format!("phase {name}: maintained by the session from its inputs' deltas: {ran} of {all} strata run, {} maintained", report.cross_maintained.0));
                None
            }
            Some((0, _)) => None,
            _ => Some(full_because),
        };
        runs.push(PhaseRun { name, report, exports, units, granules, bookkeeping: thread_cpu() - began, full_because, versions });
    }
    Ok((facts, runs, handed))
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

/// The exports whose rows the reader needs: those it reads and holds neither at this version nor at the one its delta
/// is from.
fn wanted_rows(reads: &[(String, String)], versions: &Deltas, held: &dyn Fn(&str) -> Option<u64>) -> Vec<String> {
    let mut wanted: Vec<String> = reads
        .iter()
        .filter(|(rel, export)| {
            let Some(v) = versions.get(export) else { return true };
            !held(rel).is_some_and(|h| h == v.now || v.delta.as_ref().is_some_and(|(before, _, _)| *before == h))
        })
        .map(|(_, e)| e.clone())
        .collect();
    wanted.sort();
    wanted.dedup();
    wanted
}

/// The versions of the exports `reads` names, by the reader's relation.
fn hand_versions(reads: &[(String, String)], versions: &Deltas, handed: &mut Deltas) {
    for (rel, export) in reads {
        if let Some(v) = versions.get(export) {
            handed.insert(rel.clone(), v.clone());
        }
    }
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

/// What a phase's run is given, for `maintain`.
struct Job<'a> {
    name: &'a str,
    key: &'a str,
    text: &'a str,
    root: &'a Path,
    scratch: &'a Path,
    modes: &'a str,
    binding: Binding,
    chunk_nodes: usize,
    typing: &'a Typing,
    options: &'a serde_json::Value,
    program: &'a Program,
    declared: &'a Declared,
    producer: [u8; 32],
}

type Maintained = (Report, Vec<(String, usize, usize, usize)>, (usize, usize, usize), (usize, usize, usize, usize), f64, Deltas);

/// The phase's fixed point kept in the phase store, when only workspace documents changed since its last run: the
/// last run's key, every request answered alike, the same documents (`corpus::changes`). The facts are in SQLite,
/// by granule, and each granule names its document (`.owner`), so:
/// - the changed documents are run on their own (`inputFiles`, no loads), the same program;
/// - each output a union of file-local rows (`Report::not_per_document`), a document's rows are its own, whatever the
///   other documents hold;
/// - the cutoff: each changed document's rows of what the requests' cone reads are as they were (node ids by rank,
///   `Report::cone_inputs`), so the cone loads the same documents, and the other documents' rows stand;
/// - the changed documents' granules are replaced in the store, their freshness kept within them, the record updated.
///
/// Anything else (a document added or gone, an import changed, a granule with no document or two) is a full run.
fn maintain(job: &Job) -> Result<std::result::Result<Maintained, String>> {
    let started = super::thread_usage();
    let Some(owner) = &job.declared.owner else { return Ok(Err("no `.owner` export".to_owned())) };
    let Some(path) = store_path().filter(|p| p.exists()) else { return Ok(Err("no phase store".to_owned())) };
    let db = open_store(&path)?;
    records(&db)?;
    let Some((last, consulted)): Option<(String, String)> = db.query_row("SELECT key, consulted FROM phase_record WHERE phase=?1", [job.name], |r| Ok((r.get(0)?, r.get(1)?))).ok() else { return Ok(Err("no last run recorded".to_owned())) };
    if last != job.key {
        return Ok(Err("the program, options, root or engine changed".to_owned()));
    }
    let consulted = parse_consulted(&consulted);
    let Some(changed) = super::changes(&[job.text], job.root, job.typing, job.options, &consulted)? else { return Ok(Err("a document added or gone, or a request answered otherwise".to_owned())) };
    if changed.is_empty() {
        return Ok(Err("nothing changed, yet the record was not taken".to_owned()));
    }
    let stored = cones(&db, job.name)?;
    if let Some(p) = changed.iter().find(|p| !stored.contains_key(*p)) {
        return Ok(Err(format!("{p}: no cone inputs recorded")));
    }
    // The changed documents on their own.
    let mut own = if job.options.is_object() { job.options.clone() } else { serde_json::json!({}) };
    own["inputFiles"] = serde_json::Value::Array(changed.iter().map(|p| serde_json::Value::String(p.clone())).collect());
    own["phase_no_loads"] = serde_json::Value::Bool(true);
    let out = job.scratch.join(format!("phase-{}-changed", job.name));
    let mut report = super::run_with_options(&[job.text], job.root, None, &out, job.modes, job.binding, job.chunk_nodes, job.typing, &own)?;
    let ran = super::thread_usage();
    let constants = report.phases.iter().find(|p| p.0 == "constants").map_or(0.0, |p| p.1 + p.2);
    let began = thread_cpu();
    let rows = read_exports(job.name, job.program, job.declared, &out)?;
    let refuse = |why: String| {
        crate::code_rule_dsl::logic::profile::decision("phase maintained (decision 0094 §4)", || format!("phase {}: run in full: {why}", job.name));
        Ok(Err(why))
    };
    if let Some(o) = &report.not_per_document {
        return refuse(format!("output {o} is not a union of file-local rows"));
    }
    if let Some(p) = changed.iter().find(|p| report.cone_inputs.get(*p) != stored.get(*p)) {
        return refuse(format!("{p}: what the requests' cone reads of it changed"));
    }
    // Each new granule's document, one of the changed ones.
    let changed_set: BTreeSet<&str> = changed.iter().map(String::as_str).collect();
    let mut owned: BTreeMap<&str, &str> = BTreeMap::new();
    for r in rows.get(owner).into_iter().flatten() {
        if owned.insert(r[0].as_str(), r[1].as_str()).is_some_and(|p| p != r[1]) || !changed_set.contains(r[1].as_str()) {
            return refuse(format!("granule {}: in two documents, or in one not changed", r[0]));
        }
    }
    if let Some(r) = rows.values().flatten().find(|r| !owned.contains_key(r[0].as_str())) {
        return refuse(format!("granule {}: no `.owner` row names its document", r[0]));
    }
    // The changed documents' granules before, by the stored `.owner` rows.
    let mut only: BTreeSet<String> = owned.keys().map(|g| (*g).to_owned()).collect();
    {
        // By the document each stored granule's `.owner` row names, kept with it: one statement.
        let list = serde_json::to_string(&changed_set).expect("paths");
        let mut read = db.prepare("SELECT granule FROM phase_granules WHERE phase=?1 AND doc IN (SELECT value FROM json_each(?2))")?;
        for g in read.query_map(params![job.name, list], |r| r.get::<_, String>(0))? {
            only.insert(g?);
        }
    }
    drop(db);
    let ((exports, units, granules), versions) = keep(job.name, &rows, &job.declared.positions, job.declared.owner.as_deref(), job.producer, Some(&only))?;
    let kept = super::thread_usage();
    // The record: the changed documents' bytes as now, the rest as consulted.
    let now: BTreeMap<&str, &str> = report.consulted.iter().filter(|c| c.0 == "doc").map(|c| (c.1.as_str(), c.2.as_str())).collect();
    let consulted: Vec<(String, String, String)> = consulted.into_iter().map(|(w, p, d)| if w == "doc" && let Some(n) = now.get(p.as_str()) { (w, p, (*n).to_owned()) } else { (w, p, d) }).collect();
    record(job.name, job.key, &consulted)?;
    record_documents(job.name, &report.cone_inputs, false)?;
    report.files = documents(&consulted);
    // Its parts: the changed documents' run (of it, the run's constants), the granules replaced, the reader's rows read.
    let at = super::thread_usage();
    let rss = super::usage().2;
    let cpu = |a: (f64, f64), b: (f64, f64)| (b.0 - a.0, b.1 - a.1);
    let (run, keeping, reading) = (cpu(started, ran), cpu(ran, kept), cpu(kept, at));
    report.phases = vec![
        ("maintained: changed documents run alone, their granules replaced", run.0 - constants, run.1, rss, changed.len()),
        ("maintained: the run's constants", constants, 0.0, rss, 0),
        ("maintained: granules replaced", keeping.0, keeping.1, rss, 0),
        ("maintained: the reader's rows read", reading.0, reading.1, rss, 0),
    ];
    crate::code_rule_dsl::logic::profile::decision("phase maintained (decision 0094 §4)", || format!("phase {}: {} documents run alone, {} granules replaced", job.name, changed.len(), only.len()));
    Ok(Ok((report, exports, units, granules, began, versions)))
}

/// The documents a run read: the workspace's and those it loaded, each once.
fn documents(consulted: &[(String, String, String)]) -> usize {
    // A workspace document listed without a digest was not read (`run_observed`: none built).
    consulted.iter().filter(|c| (c.0 == "doc" && !c.2.is_empty()) || c.0 == "load").map(|c| c.1.as_str()).collect::<BTreeSet<_>>().len()
}

fn parse_consulted(text: &str) -> Vec<(String, String, String)> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.splitn(3, '\t');
            Some((f.next()?.to_owned(), f.next()?.to_owned(), f.next()?.to_owned()))
        })
        .collect()
}

/// Every row the phase store holds of `exports`, by export.
fn stored_rows(phase: &str, exports: &[String]) -> Result<BTreeMap<String, Vec<Vec<String>>>> {
    let mut rows: BTreeMap<String, Vec<Vec<String>>> = exports.iter().map(|e| (e.clone(), Vec::new())).collect();
    let Some(path) = store_path().filter(|_| !exports.is_empty()) else { return Ok(rows) };
    let db = open_store(&path)?;
    for (_, (_, granule)) in read_granules(&db, phase, None)? {
        for (rel, rs) in granule {
            if let Some(out) = rows.get_mut(&rel) {
                out.extend(rs);
            }
        }
    }
    Ok(rows)
}

/// Each document's cone digest (`Report::cone_inputs`), all of them anew (`all`) or those given: one row per phase.
fn record_documents(phase: &str, cone: &BTreeMap<String, String>, all: bool) -> Result<()> {
    let Some(path) = store_path() else { return Ok(()) };
    let db = open_store(&path)?;
    let mut cones = if all { BTreeMap::new() } else { cones(&db, phase)? };
    cones.extend(cone.iter().map(|(p, d)| (p.clone(), d.clone())));
    db.execute("INSERT OR REPLACE INTO phase_cones VALUES (?1, ?2)", params![phase, serde_json::to_string(&cones).expect("text")])?;
    Ok(())
}

/// Each document's cone digest as the store holds it for `phase`.
fn cones(db: &Connection, phase: &str) -> Result<BTreeMap<String, String>> {
    let text: Option<String> = db.query_row("SELECT cones FROM phase_cones WHERE phase=?1", [phase], |r| r.get(0)).ok();
    text.map_or(Ok(BTreeMap::new()), |t| serde_json::from_str(&t).map_err(|e| Error::Plan(format!("phase store: cones: {e}"))))
}

/// The exports `reads` names, as the reader's facts.
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

/// A digest of what earlier phases hand on: each export's version, which the phase store moves only when its rows
/// change, or else its rows, in order (a full run hands them on as written, a kept one as stored).
fn digest_facts(facts: &Facts, versions: &Deltas) -> String {
    let mut h = Sha256::new();
    for (rel, rows) in facts {
        h.update(rel.as_bytes());
        h.update([0]);
        if let Some(v) = versions.get(rel) {
            h.update(v.now.to_le_bytes());
            continue;
        }
        let mut rows: Vec<String> = rows.iter().map(|r| format!("{r:?}")).collect();
        rows.sort();
        for row in rows {
            h.update(row.as_bytes());
            h.update([1]);
        }
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// What a phase's run is keyed by, beside what it consults: its program, the run's options, the root, the engine and
/// the node-id scheme.
fn run_key(text: &str, root: &Path, options: &serde_json::Value) -> String {
    let mut h = Sha256::new();
    h.update(b"rerail.phase-run/v1\0");
    h.update(text.as_bytes());
    h.update([0]);
    h.update(options.to_string().as_bytes());
    h.update([0]);
    h.update(std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf()).to_string_lossy().as_bytes());
    h.update(env!("RERAIL_ENGINE_BUILD_ID").as_bytes());
    h.update([u8::from(crate::code_rule_dsl::facts::vtab::syntax::definition_ids())]);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn records(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS phase_record(phase TEXT PRIMARY KEY, key TEXT, consulted TEXT) WITHOUT ROWID;")?;
    Ok(())
}

/// Records what a phase's run consulted, under its key (`reuse` checks it). A run that consulted nothing it can name
/// (no workspace files read) is not recorded.
fn record(phase: &str, key: &str, consulted: &[(String, String, String)]) -> Result<()> {
    let Some(path) = store_path() else { return Ok(()) };
    let db = open_store(&path)?;
    records(&db)?;
    // A run that consulted nothing (a phase over earlier phases' exports reads no document) is recorded too: its
    // key, which holds those exports' versions, is all it depends on.
    let text = consulted.iter().map(|(w, p, d)| format!("{w}\t{p}\t{d}")).collect::<Vec<_>>().join("\n");
    db.execute("INSERT OR REPLACE INTO phase_record VALUES (?1, ?2, ?3)", params![phase, key, text])?;
    Ok(())
}

/// The last run's exports, by export, and the documents it read, when the phase is unchanged since: the same key,
/// and everything it consulted as it was (`corpus::unchanged`). Its freshness then has nothing to record.
#[allow(clippy::too_many_arguments)]
fn reuse(phase: &str, key: &str, text: &str, root: &Path, typing: &Typing, options: &serde_json::Value) -> Result<Option<usize>> {
    // `RERAIL_PHASE_WHY`: why a phase was not kept, on stderr.
    let why = |what: &str| {
        if std::env::var_os("RERAIL_PHASE_WHY").is_some() {
            eprintln!("phase {phase}: not kept: {what}");
        }
    };
    let Some(path) = store_path().filter(|p| p.exists()) else {
        why("no phase store");
        return Ok(None);
    };
    let db = open_store(&path)?;
    records(&db)?;
    let Some((last, consulted)): Option<(String, String)> = db.query_row("SELECT key, consulted FROM phase_record WHERE phase=?1", [phase], |r| Ok((r.get(0)?, r.get(1)?))).ok() else {
        why("no last run recorded");
        return Ok(None);
    };
    if last != key {
        why("its key changed (its program, options, root or an earlier phase's exports)");
        return Ok(None);
    }
    let consulted = parse_consulted(&consulted);
    if consulted.is_empty() {
        return Ok(Some(0));
    }
    if !super::unchanged_phase(&[text], root, typing, options, &consulted)? {
        why("an input it consulted changed");
        return Ok(None);
    }
    Ok(Some(documents(&consulted)))
}

/// An export as handed on: its version (a digest of the phase store's epoch, the export and its version number), and
/// when this run made it, its delta from the version before: that version, the rows new and the rows gone. A reader
/// that holds the version before takes the delta as its input's (`maintain`), and reads no other row.
#[derive(Clone)]
pub(crate) struct Versioned {
    pub(crate) now: u64,
    /// Its rows at this version.
    pub(crate) rows: usize,
    pub(crate) delta: Option<(u64, Vec<Vec<String>>, Vec<Vec<String>>)>,
}

pub(crate) type Deltas = BTreeMap<String, Versioned>;

fn versions_table(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS export_versions(phase TEXT, rel TEXT, v INTEGER, n INTEGER, PRIMARY KEY(phase, rel)) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS store_epoch(one INTEGER PRIMARY KEY CHECK (one = 1), epoch INTEGER);",
    )?;
    // A store made afresh numbers its versions from 0 again: its epoch tells its versions from an earlier store's.
    let epoch = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as i64) ^ i64::from(std::process::id());
    db.execute("INSERT OR IGNORE INTO store_epoch VALUES (1, ?1)", [epoch])?;
    Ok(())
}

/// The store's epoch, an export's version number (0 before its first rows) and its rows.
fn version(db: &Connection, phase: &str, rel: &str) -> Result<(i64, u64, usize)> {
    let epoch: i64 = db.query_row("SELECT epoch FROM store_epoch", [], |r| r.get(0))?;
    let v: Option<(i64, i64)> = db.query_row("SELECT v, n FROM export_versions WHERE phase=?1 AND rel=?2", params![phase, rel], |r| Ok((r.get(0)?, r.get(1)?))).ok();
    let (v, n) = v.unwrap_or((0, 0));
    Ok((epoch, v as u64, n as usize))
}

fn version_digest(epoch: i64, rel: &str, v: u64) -> u64 {
    let h = Sha256::digest(format!("rerail.export-version/v1\0{epoch}\0{rel}\0{v}").as_bytes());
    u64::from_le_bytes(h[..8].try_into().expect("eight bytes")) | (1 << 63)
}

/// Each export's version as the store holds it, with no delta (a run that took the kept rows).
fn current_versions(phase: &str, exports: &[String]) -> Result<Deltas> {
    let Some(path) = store_path() else { return Ok(Deltas::new()) };
    let db = open_store(&path)?;
    versions_table(&db)?;
    exports.iter().map(|rel| Ok((rel.clone(), { let (e, v, n) = version(&db, phase, rel)?; Versioned { now: version_digest(e, rel, v), rows: n, delta: Some((version_digest(e, rel, v), Vec::new(), Vec::new())) } }))).collect()
}

type Kept = (Vec<(String, usize, usize, usize)>, (usize, usize, usize), (usize, usize, usize, usize));

/// A phase's exports' rows, by export.
pub(super) type Rows = BTreeMap<String, Vec<Vec<String>>>;

/// `keep` after a full run of the phase, in a session that holds its last run's rows at the store's current versions:
/// the rows are diffed in memory, and only the granules whose rows changed are read, compared and written (as
/// `maintain` keeps a changed document's). Outside a session, or holding other versions, every granule is.
fn keep_in_session(phase: &str, rows: Rows, declared: &Declared, producer: [u8; 32]) -> Result<(Kept, Deltas)> {
    // The session's lock is not held across `keep`, which opens the stores.
    let held = {
        let session = super::session::current();
        let Some(s) = session.as_ref() else { return keep(phase, &rows, &declared.positions, declared.owner.as_deref(), producer, None) };
        s.phase_rows.lock().ok().and_then(|m| m.get(phase).cloned())
    };
    let at = |v: &Deltas| -> BTreeMap<String, u64> { v.iter().map(|(rel, v)| (rel.clone(), v.now)).collect() };
    let rows = std::sync::Arc::new(rows);
    let (kept, versions) = match held {
        Some((versions, old)) if versions == at(&current_versions(phase, &declared.exports)?) && old.keys().eq(rows.keys()) => {
            // The granules whose rows changed: a row's first column, of a row new or gone.
            let mut touched: BTreeSet<String> = BTreeSet::new();
            for (rel, now) in rows.iter() {
                let was: std::collections::HashSet<&Vec<String>> = old[rel].iter().collect();
                let is: std::collections::HashSet<&Vec<String>> = now.iter().collect();
                touched.extend(now.iter().filter(|r| !was.contains(r)).chain(old[rel].iter().filter(|r| !is.contains(r))).map(|r| r[0].clone()));
            }
            let within: Rows = rows.iter().map(|(rel, rs)| (rel.clone(), rs.iter().filter(|r| touched.contains(&r[0])).cloned().collect())).collect();
            let ((mut exports, units, granules), versions) = keep(phase, &within, &declared.positions, declared.owner.as_deref(), producer, Some(&touched))?;
            for e in &mut exports {
                e.1 = rows[&e.0].len();
            }
            ((exports, units, granules), versions)
        }
        _ => keep(phase, &rows, &declared.positions, declared.owner.as_deref(), producer, None)?,
    };
    if let Some(s) = super::session::current().as_ref()
        && let Ok(mut m) = s.phase_rows.lock()
    {
        m.insert(phase.to_owned(), (at(&versions), rows));
    }
    Ok((kept, versions))
}

/// Keeps a phase's exports in the phase store and its freshness in the derivation graph; the deltas against the last
/// run. With `only`, `rows` are those of these granules, and the rest of the store stands.
fn keep(phase: &str, rows: &BTreeMap<String, Vec<Vec<String>>>, positions: &[String], owner: Option<&str>, producer: [u8; 32], only: Option<&BTreeSet<String>>) -> Result<(Kept, Deltas)> {
    use crate::content_store::{ABSENT, Binding as Bound, DerivationInput, DerivationRecord};
    let store_err = |e: crate::content_store::StoreError| Error::Plan(e.to_string());
    let Some(path) = store_path() else {
        let counts = rows.iter().map(|(rel, r)| (rel.clone(), r.len(), r.len(), 0)).collect();
        return Ok(((counts, (0, 0, 0), (0, 0, 0, 0)), Deltas::new()));
    };
    // The exports by granule, and the delta against the last run's: worked out in memory from the stored granules in
    // scope (`only`'s, or all of them), and written back as one wave.
    let db = open_store(&path)?;
    let tx = db.unchecked_transaction()?;
    let old = read_granules(&tx, phase, only)?;
    // The last run's granules, so a granule whose rows are all gone is a change too.
    let granules_before: BTreeSet<String> = old.iter().filter(|(_, (_, g))| g.keys().any(|rel| rel != UNITS && !positions.contains(rel))).map(|(g, _)| g.clone()).collect();
    // This run's, by granule.
    let mut now: BTreeMap<&str, Granule> = BTreeMap::new();
    for (rel, rs) in rows {
        for r in rs {
            now.entry(r[0].as_str()).or_default().entry(rel.clone()).or_default().push(r.clone());
        }
    }
    let mut exports = Vec::new();
    let mut deltas = Deltas::new();
    for (rel, rs) in rows {
        let was: std::collections::HashSet<&Vec<String>> = old.values().flat_map(|(_, g)| g.get(rel).into_iter().flatten()).collect();
        let is: std::collections::HashSet<&Vec<String>> = rs.iter().collect();
        let mut seen = std::collections::HashSet::new();
        let plus: Vec<Vec<String>> = rs.iter().filter(|r| !was.contains(r) && seen.insert(*r)).cloned().collect();
        let minus: Vec<Vec<String>> = old.values().flat_map(|(_, g)| g.get(rel).into_iter().flatten()).filter(|r| !is.contains(r)).cloned().collect();
        let (added, gone) = (plus.len(), minus.len());
        // The export's version: the next when its rows changed, and the delta from the one before.
        let before = version(&tx, phase, rel)?;
        // Its rows: all of them here, or within `only` the last run's and the delta.
        let n = if only.is_some() { (before.2 + added).saturating_sub(gone) } else { rs.len() };
        let v = if added + gone > 0 { before.1 + 1 } else { before.1 };
        tx.execute("INSERT OR REPLACE INTO export_versions VALUES (?1, ?2, ?3, ?4)", params![phase, rel, v as i64, n as i64])?;
        deltas.insert(rel.clone(), Versioned { now: version_digest(before.0, rel, v), rows: n, delta: Some((version_digest(before.0, rel, before.1), plus, minus)) });
        exports.push((rel.clone(), rs.len(), added, gone));
    }
    // The granules written: those whose rows changed; gone: those in scope with no rows now. One statement each.
    let owner_of = |g: &Granule| owner.and_then(|o| g.get(o)).and_then(|rs| rs.first()).and_then(|r| r.get(1)).cloned();
    let written: Vec<serde_json::Value> = now
        .iter()
        .filter_map(|(g, rows)| {
            let text = granule_text(rows);
            (old.get(*g).map(|(t, _)| t) != Some(&text)).then(|| serde_json::json!([g, owner_of(rows), text]))
        })
        .collect();
    let gone: Vec<&String> = old.keys().filter(|g| !now.contains_key(g.as_str())).collect();
    if !gone.is_empty() {
        tx.execute("DELETE FROM phase_granules WHERE phase=?1 AND granule IN (SELECT value FROM json_each(?2))", params![phase, serde_json::to_string(&gone).expect("names")])?;
    }
    if !written.is_empty() {
        tx.execute(
            "INSERT OR REPLACE INTO phase_granules(phase, granule, doc, rows) SELECT ?1, json_extract(value, '$[0]'), json_extract(value, '$[1]'), json_extract(value, '$[2]') FROM json_each(?2)",
            params![phase, serde_json::Value::Array(written).to_string()],
        )?;
    }
    // The definitions: each bound to its digest; the changed ones are the freshness delta.
    let units: BTreeMap<&str, &str> = rows.get(UNITS).into_iter().flatten().filter(|r| r.len() >= 2).map(|r| (r[0].as_str(), r[1].as_str())).collect();
    let before: BTreeSet<String> = old.values().flat_map(|(_, g)| g.get(UNITS).into_iter().flatten()).map(|r| r[0].clone()).collect();
    let gone_units = before.iter().filter(|u| !units.contains_key(u.as_str())).count();
    tx.commit()?;
    let store = super::tape_cache().map_err(store_err)?.expect("a content store beside the phase store");
    let unit_name = |u: &str| format!("unit:{phase}:{u}");
    let digest = |s: &str| -> [u8; 32] { Sha256::digest(s.as_bytes()).into() };
    let mut changed_units = store.bind_names(&units.iter().map(|(u, d)| Bound { name: unit_name(u), digest: digest(d) }).collect::<Vec<_>>()).map_err(store_err)?;
    // A definition gone reads as absent from now on.
    let gone: Vec<Bound> = before.iter().filter(|u| !units.contains_key(u.as_str())).map(|u| Bound { name: unit_name(u), digest: ABSENT }).collect();
    changed_units.extend(store.bind_names(&gone).map_err(store_err)?);
    // The granules: each export row's first column; the digest of its rows, over every export but `unit`.
    let mut by_granule: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for (rel, rs) in rows.iter().filter(|(rel, _)| rel.as_str() != UNITS && !positions.contains(rel)) {
        for r in rs {
            by_granule.entry(r[0].as_str()).or_default().push(format!("{rel}\t{}", r.join("\t")));
        }
    }
    // The frontier: the granules recorded as consulting a changed definition.
    let frontier: BTreeSet<String> = store.derivations_reading(&changed_units).map_err(store_err)?.into_iter().map(|d| d.output).collect();
    let gone_granules: Vec<String> = granules_before.iter().filter(|g| !by_granule.contains_key(g.as_str())).cloned().collect();
    let mut records: Vec<DerivationRecord> = by_granule
        .into_iter()
        .map(|(g, mut lines)| {
            lines.sort();
            let mut h = Sha256::new();
            for l in &lines {
                h.update(l.as_bytes());
                h.update([0]);
            }
            (g, h)
        })
        .map(|(g, h)| DerivationRecord {
            output: format!("granule:{phase}:{g}"),
            producer,
            inputs: vec![DerivationInput { name: Some(unit_name(g)), digest: units.get(g).map_or(ABSENT, |d| digest(d)) }],
            digest: h.finalize().into(),
            status: 0,
            cost: 0,
        })
        .collect();
    let n = records.len();
    // A granule gone: its output is bound to the absent digest, as a name nobody binds reads.
    records.extend(gone_granules.iter().map(|g| DerivationRecord {
        output: format!("granule:{phase}:{g}"),
        producer,
        inputs: vec![DerivationInput { name: Some(unit_name(g)), digest: units.get(g.as_str()).map_or(ABSENT, |d| digest(d)) }],
        digest: ABSENT,
        status: 0,
        cost: 0,
    }));
    let changed: Vec<String> = store.record_derivations(&records).map_err(store_err)?;
    // A changed granule is attributed when its definition changed too, or it is new (its definition first bound).
    let fresh_units: BTreeSet<&str> = changed_units.iter().map(String::as_str).collect();
    let unattributed = changed.iter().filter(|g| !frontier.contains(*g) && !fresh_units.contains(unit_name(g.trim_start_matches(&format!("granule:{phase}:"))).as_str())).count();
    Ok(((exports, (units.len(), changed_units.len() - gone.len(), gone_units), (n, changed.len(), frontier.len(), unattributed)), deltas))
}

#[cfg(test)]
mod tests {
    use crate::code_rule_dsl::typing::file_local::Binding;

    /// A phase runs in its own space: a reader sees its exports and nothing else, an export naming a node is refused,
    /// and freshness follows definitions. A comment changes no definition and no granule; a body edit changes its
    /// definition and, through it, its granule only; a new function adds its definition and its granule, and changes
    /// its module's; no change is unattributed.
    #[test]
    fn a_phase_hands_on_its_exports_and_keeps_freshness_by_definition() {
        let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("phases");
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
        assert_eq!(std::fs::read_to_string(out.join("r.csv")).unwrap(), "m\nm.f\nm.g\n");
        let p = &first.phase_runs[0];
        assert_eq!((p.units, p.granules.0, p.granules.3), ((3, 3, 0), 3, 0), "every definition new, three granules");
        assert_eq!(first.files, 0, "the reader reads only the phase's exports, so no file");
        let fresh = |text: &str| {
            let r = run(text).unwrap();
            let p = &r.phase_runs[0];
            (p.units.1, p.units.2, p.granules.1, p.granules.3)
        };
        // Unchanged: everything the phase consulted is as it was, so it is not run; its exports are the kept ones.
        let again = run(&text).unwrap();
        assert!(again.phase_runs[0].report.phases.iter().any(|p| p.0.starts_with("unchanged")), "an unchanged phase is not run again");
        assert_eq!(std::fs::read_to_string(out.join("r.csv")).unwrap(), "m\nm.f\nm.g\n");
        assert_eq!(fresh(&text), (0, 0, 0, 0), "unchanged: nothing");
        write("a.py", "# note\ndef f():\n    return 1\n\n\ndef g():  # g\n    return 2\n");
        assert_eq!(fresh(&text), (0, 0, 0, 0), "a comment: no definition changed");
        write("a.py", "# note\ndef f():\n    x = 1\n    return x\n\n\ndef g():  # g\n    return 2\n");
        assert_eq!(fresh(&text), (1, 0, 0, 0), "a body: f's definition, and no granule (def has no body)");
        write("a.py", "# note\ndef f():\n    x = 1\n    return x\n\n\ndef g():  # g\n    return 2\n\ndef h():\n    pass\n");
        assert_eq!(fresh(&text), (2, 0, 1, 0), "a new function: its definition and the module's; its granule");
        assert_eq!(std::fs::read_to_string(out.join("r.csv")).unwrap(), "m\nm.f\nm.g\nm.h\n");
        write("a.py", "def f():\n    x = 1\n    return x\n\ndef h():\n    pass\n");
        assert_eq!(fresh(&text), (1, 1, 1, 0), "g removed: the module changed, g gone, g's granule gone");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Phases chain: a phase may read an earlier phase's exports (0083: struct, then types, then the analysis), and
    /// the reader sees only the last phase's. Phases that read each other are refused, not run forever.
    #[test]
    fn a_phase_reads_an_earlier_phase_and_a_cycle_is_refused() {
        let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("phases-chain");
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
        // Both phases are listed flat, in the order they ran; p2's own run checked p1 once, not again.
        let names: Vec<&str> = report.phase_runs.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["p1", "p2"]);
        assert!(report.phase_runs[1].report.phase_runs.is_empty(), "p1 is not checked again inside p2's run");
        // Nothing changed: p2 is kept, although p1's rows come back from the store in another order than its run wrote.
        let again = run(&reader).unwrap();
        assert!(again.phase_runs.iter().all(|r| r.report.phases.first().is_some_and(|p| p.0.starts_with("unchanged"))), "both kept: {:?}", again.phase_runs.iter().map(|r| r.report.phases.first().map(|p| p.0)).collect::<Vec<_>>());
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
        let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("phases-constants");
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
        assert_eq!(report.phase_runs.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["p1"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A phase whose granules name their documents (`.owner`) is maintained in its store: an edit runs the changed
    /// document alone and replaces its granules; the other documents' rows stand; the reader sees what a full run
    /// gives. A document added is a full run.
    #[test]
    fn a_phase_with_owned_granules_replaces_a_changed_documents_rows() {
        let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("phases-owned");
        let _ = std::fs::remove_dir_all(&dir);
        let (src, phases) = (dir.join("src"), dir.join("phases"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&phases).unwrap();
        let write = |name: &str, text: &str| std::fs::write(src.join(name), text).unwrap();
        write("a.py", "def f():\n    return 1\n");
        write("b.py", "def g():\n    return 2\n");
        std::fs::write(phases.join("p1.rr"), r#"
#include "core.rr"
.phase p1
.export unit, def, own
.owner own
.decl fn(n: node, s: symbol, p: symbol)
fn(n, cat(cat(p, ":"), x), p) :- kind(n, "function_definition"), field(n, "name", i), text(i, x), docof(n, d), doc(d, p, _).
.decl unit(s: symbol, h: symbol)
unit(s, h) :- fn(n, s, _), def_digest(n, h).
.decl def(s: symbol, k: symbol)
def(s, "function") :- fn(_, s, _).
.decl own(s: symbol, p: symbol)
own(s, p) :- fn(_, s, p).
"#).unwrap();
        let out = dir.join("out");
        let _no_session = crate::code_rule_dsl::logic::plan::corpus::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        crate::code_rule_dsl::logic::plan::corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
        let options = serde_json::json!({ "phase_dir": phases.to_string_lossy() });
        let text = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse("#include \"core.rr\"\n.decl p1__def(s: symbol, k: symbol)\n.decl p1__own(s: symbol, p: symbol)\n.decl r(s: symbol)\nr(s) :- p1__def(s, _).\n.decl q(x: symbol, s: symbol)\nq(x, s) :- kind(n, \"function_definition\"), field(n, \"name\", i), text(i, x), x = \"f\", p1__own(s, \"b.py\").\n.output r, q\n").unwrap()).unwrap();
        let run = || crate::code_rule_dsl::logic::plan::corpus::run_with_options(&[&text], &src, None, &out, "program", Binding::Cpu, 1 << 18, &Default::default(), &options).unwrap();
        let r = || std::fs::read_to_string(out.join("r.csv")).unwrap();
        let how = |report: &crate::code_rule_dsl::logic::plan::corpus::Report| {
            let p = &report.phase_runs[0];
            (p.full_because.is_none(), p.report.phases.last().map_or("", |x| x.0).split(':').next().unwrap_or("").to_owned(), p.units)
        };
        assert!(!how(&run()).0, "the first run is a full one");
        assert_eq!(r(), "a.py:f\nb.py:g\n");
        write("b.py", "def g():\n    x = 2\n    return x\n\ndef h():\n    pass\n");
        let (maintained, what, units) = how(&run());
        assert!(maintained && what == "maintained", "an edit to b.py is maintained: {what}");
        assert_eq!(units, (2, 2, 0), "b.py's definitions only: g changed, h new");
        assert_eq!(r(), "a.py:f\nb.py:g\nb.py:h\n", "a.py's rows stand, b.py's replaced");
        // a.py's file-local rows read b.py's exports: its bytes are as they were, yet its memo entry is not taken.
        assert_eq!(std::fs::read_to_string(out.join("q.csv")).unwrap(), "f\tb.py:g\nf\tb.py:h\n", "a.py's rows over b.py's exports, as they are now");
        write("b.py", "def h():\n    pass\n");
        let (maintained, _, units) = how(&run());
        assert!(maintained, "a definition removed is maintained");
        assert_eq!(units, (1, 0, 1), "g gone");
        assert_eq!(r(), "a.py:f\nb.py:h\n");
        write("c.py", "def k():\n    pass\n");
        let report = run();
        assert!(report.phase_runs[0].full_because.as_deref().is_some_and(|w| w.contains("added or gone")), "a document added: a full run");
        assert_eq!(r(), "a.py:f\nb.py:h\nc.py:k\n");
        crate::code_rule_dsl::logic::plan::corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
