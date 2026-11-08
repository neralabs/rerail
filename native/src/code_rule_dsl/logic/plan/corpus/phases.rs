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

/// The export a phase names its definitions in, with their digests.
const UNITS: &str = "unit";

/// What a program says of itself as a phase: its name and its exports.
pub(crate) struct Declared {
    pub(crate) name: String,
    pub(crate) exports: Vec<String>,
    /// Exports that say where things are (`.positions at`): handed on, but no part of a granule's digest, since an
    /// edit moves every later line.
    pub(crate) positions: Vec<String>,
}

/// The phase `prog` declares (`.phase NAME`, `.export a, b`), if any.
pub(crate) fn declared(prog: &Program) -> Option<Declared> {
    let setting = |key: &str| prog.settings.iter().filter(|(k, _)| k == key).map(|(_, v)| v.trim().to_owned()).collect::<Vec<_>>();
    let name = setting("phase").into_iter().next()?;
    let list = |key: &str| setting(key).iter().flat_map(|v| v.split(',').map(|e| e.trim().trim_end_matches('.').to_owned()).collect::<Vec<_>>()).filter(|e| !e.is_empty()).collect::<Vec<_>>();
    Some(Declared { name, exports: list("export"), positions: list("positions") })
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
fn wanted(prog: &Program, dir: Option<&Path>) -> Result<BTreeMap<String, Vec<(String, String)>>> {
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
}

pub(crate) type Facts = BTreeMap<String, Vec<Vec<Value>>>;

/// The store a phase keeps its exports in, beside the content store: `<content store>.phases`.
fn store_path() -> Option<PathBuf> {
    super::tape_cache_path().map(|p| PathBuf::from(format!("{}.phases", p.display())))
}

fn thread_cpu() -> f64 {
    let (u, s) = super::thread_usage();
    u + s
}

/// The facts `prog` reads from earlier phases, by relation, and each phase's run. `scratch` holds the phases' runs
/// (`<scratch>/phase-<name>`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn facts(prog: &Program, root: &Path, scratch: &Path, modes: &str, binding: Binding, chunk_nodes: usize, typing: &Typing, options: &serde_json::Value) -> Result<(Facts, Vec<PhaseRun>)> {
    let (mut facts, mut runs) = (Facts::new(), Vec::new());
    let dir = options.get("phase_dir").and_then(|v| v.as_str()).map(PathBuf::from);
    for (name, reads) in wanted(prog, dir.as_deref())? {
        let src = source(&name, dir.as_deref())?.expect("a phase program");
        let program = crate::code_rule_dsl::logic::rr::parse(&src)?;
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
        // The phase's run: its own program, its own scratch, its exports written as its outputs.
        let out = scratch.join(format!("phase-{name}"));
        let text = format!("{src}\n.output {}\n", declared.exports.join(", "));
        let report = super::run_with_options(&[&text], root, None, &out, modes, binding, chunk_nodes, typing, options)?;
        let began = thread_cpu();
        let mut rows: BTreeMap<String, Vec<Vec<String>>> = BTreeMap::new();
        for export in &declared.exports {
            let text = std::fs::read_to_string(out.join(format!("{export}.csv"))).map_err(|e| Error::Plan(format!("phase {name}: {export}: {e}")))?;
            let width = program.decls[export].len();
            let read: Vec<Vec<String>> = text.lines().map(|l| l.split('\t').map(str::to_owned).collect()).collect();
            // Outputs are lines of tab-separated values, unescaped: a value holding a newline or a tab would come back
            // as other rows, and be handed on as facts no rule made.
            if let Some(bad) = read.iter().find(|r| r.len() != width) {
                return Err(Error::Plan(format!("phase {name}: {export}: a row of {} values, not {width} (a value holds a newline or a tab): {:?}", bad.len(), bad.join("\t"))));
            }
            rows.insert(export.clone(), read);
        }
        let _ = std::fs::remove_dir_all(&out);
        let producer: [u8; 32] = Sha256::digest(src.as_bytes()).into();
        let (exports, units, granules) = keep(&name, &rows, &declared.positions, producer)?;
        for (rel, export) in &reads {
            let types: Vec<&str> = program.decls[export].iter().map(|(_, t)| t.as_str()).collect();
            let held = facts.entry(rel.clone()).or_default();
            for r in &rows[export] {
                let row = r
                    .iter()
                    .zip(&types)
                    .map(|(v, t)| if *t == "symbol" || *t == "sym" { Ok(Value::Str(v.clone())) } else { v.parse::<i64>().map(Value::Int).map_err(|_| Error::Plan(format!("phase {name}: {export}: not a number: {v}"))) })
                    .collect::<Result<Vec<_>>>()?;
                held.push(row);
            }
        }
        runs.push(PhaseRun { name, report, exports, units, granules, bookkeeping: thread_cpu() - began });
    }
    Ok((facts, runs))
}

type Kept = (Vec<(String, usize, usize, usize)>, (usize, usize, usize), (usize, usize, usize, usize));

/// Keeps a phase's exports in the phase store and its freshness in the derivation graph; the deltas against the last
/// run.
fn keep(phase: &str, rows: &BTreeMap<String, Vec<Vec<String>>>, positions: &[String], producer: [u8; 32]) -> Result<Kept> {
    use crate::content_store::{ABSENT, Binding as Bound, DerivationInput, DerivationRecord};
    let store_err = |e: crate::content_store::StoreError| Error::Plan(e.to_string());
    let Some(path) = store_path() else {
        let counts = rows.iter().map(|(rel, r)| (rel.clone(), r.len(), r.len(), 0)).collect();
        return Ok((counts, (0, 0, 0), (0, 0, 0, 0)));
    };
    // The exports by granule, and the delta against the last run's.
    let db = Connection::open(&path)?;
    db.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA synchronous=OFF;
         CREATE TABLE IF NOT EXISTS export_rows(phase TEXT, rel TEXT, granule TEXT, row TEXT, PRIMARY KEY(phase, rel, granule, row)) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS phase_units(phase TEXT, unit TEXT, digest TEXT, PRIMARY KEY(phase, unit)) WITHOUT ROWID;",
    )?;
    let tx = db.unchecked_transaction()?;
    // The last run's granules, so a granule whose rows are all gone is a change too.
    let granules_before: BTreeSet<String> = tx
        .prepare("SELECT DISTINCT rel, granule FROM export_rows WHERE phase=?1 AND rel<>?2")?
        .query_map(params![phase, UNITS], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .filter(|(rel, _)| !positions.contains(rel))
        .map(|(_, g)| g)
        .collect();
    let mut exports = Vec::new();
    for (rel, rs) in rows {
        tx.execute_batch("DROP TABLE IF EXISTS temp.now; CREATE TEMP TABLE now(granule TEXT, row TEXT, PRIMARY KEY(granule, row)) WITHOUT ROWID;")?;
        {
            let mut put = tx.prepare_cached("INSERT OR IGNORE INTO temp.now VALUES (?1, ?2)")?;
            for r in rs {
                put.execute(params![r[0], r.join("\t")])?;
            }
        }
        let added: i64 = tx.query_row("SELECT count(*) FROM temp.now n WHERE NOT EXISTS (SELECT 1 FROM export_rows e WHERE e.phase=?1 AND e.rel=?2 AND e.granule=n.granule AND e.row=n.row)", params![phase, rel], |r| r.get(0))?;
        let gone: i64 = tx.query_row("SELECT count(*) FROM export_rows e WHERE e.phase=?1 AND e.rel=?2 AND NOT EXISTS (SELECT 1 FROM temp.now n WHERE n.granule=e.granule AND n.row=e.row)", params![phase, rel], |r| r.get(0))?;
        tx.execute("DELETE FROM export_rows WHERE phase=?1 AND rel=?2 AND NOT EXISTS (SELECT 1 FROM temp.now n WHERE n.granule=export_rows.granule AND n.row=export_rows.row)", params![phase, rel])?;
        tx.execute("INSERT OR IGNORE INTO export_rows SELECT ?1, ?2, granule, row FROM temp.now", params![phase, rel])?;
        exports.push((rel.clone(), rs.len(), added as usize, gone as usize));
    }
    tx.execute_batch("DROP TABLE IF EXISTS temp.now")?;
    // The definitions: each bound to its digest; the changed ones are the freshness delta.
    let units: BTreeMap<&str, &str> = rows.get(UNITS).into_iter().flatten().filter(|r| r.len() >= 2).map(|r| (r[0].as_str(), r[1].as_str())).collect();
    let before: BTreeSet<String> = tx.prepare("SELECT unit FROM phase_units WHERE phase=?1")?.query_map([phase], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    let gone_units = before.iter().filter(|u| !units.contains_key(u.as_str())).count();
    tx.execute("DELETE FROM phase_units WHERE phase=?1", [phase])?;
    {
        let mut put = tx.prepare_cached("INSERT INTO phase_units VALUES (?1, ?2, ?3)")?;
        for (u, d) in &units {
            put.execute(params![phase, u, d])?;
        }
    }
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
    Ok((exports, (units.len(), changed_units.len() - gone.len(), gone_units), (n, changed.len(), frontier.len(), unattributed)))
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
        let fresh = |text: &str| {
            let r = run(text).unwrap();
            let p = &r.phase_runs[0];
            (p.units.1, p.units.2, p.granules.1, p.granules.3)
        };
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
}
