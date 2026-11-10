//! The run's constants, folded first (decision 0094 §6, phase 0's stored answers).
//!
//! Some relations are made from no document at all: from the program's facts, the run's settings (`env`), and what
//! the file system says (`dir_entry`, `file_line`):
//!
//! ```text
//! py_version_text(v) :- env("python.version", v).
//! py_version_text(v) :- !env("python.version", _), env("python.venv", e), file_line(cat(e, "/pyvenv.cfg"), _, l), …
//! py_version(a, b)   :- py_version_text(v), …
//! py_t(c)            :- py_vtest(c, ">=", a, b), py_version(x, y), …      // per document, but for py_version
//! ```
//!
//! Such a relation has one value for the whole run. Left in the program, a rule that reads it reads a relation the
//! file-local part cannot make (a file read is a request, answered in the cross-file part), so the rule, and every
//! rule after it, runs in the cross-file part over every document's base rows: `struct.rr`'s definitions were all
//! cross-file for `py_version` alone, and a warm run measured 2.3 M base rows to plan them.
//!
//! So these relations are derived first, by a run of the same engine over no documents with the requests answered
//! (phases, not a new evaluator), and given to the program as facts. Their rules go. A rule that read them now reads
//! facts, so the split places it by what else it reads. The per-file memo keys a file-local part by its facts, so a
//! changed constant (another Python version) misses the memo, as it must. What the constants' run read is part of
//! the run's consulted record (`Report::consulted`).
//!
//! The constants' rows are kept in the phase store (`constant_rows`), under the programs, the relations, the run's
//! options (but which documents it runs over: the constants read none), the root and the engine, with what their run
//! consulted. A later run whose consulted things are as they were (`World::still`) takes them as they are: an edit to
//! a workspace file derives no constant again.

use std::collections::BTreeSet;
use std::path::Path;

use super::super::Result;
use super::{Report, Typing};
use crate::code_rule_dsl::logic::front::{Literal, Program, Value};
use crate::code_rule_dsl::typing::file_local::{constant_relations, option_input};

/// The option that marks a run as the constants' own (it folds nothing).
pub(super) const OWN: &str = "constants_run";

/// The relations made from no document that a rule outside them reads, or that the program outputs: from facts,
/// settings and file-system reads, through other such relations and negations of them. Those the split already
/// treats as constants (`constant_relations`: no request among them) are left to it.
pub(super) fn foldable(prog: &Program) -> BTreeSet<String> {
    let derived: BTreeSet<&str> = prog.rules.iter().map(|r| r.head.rel.as_str()).collect();
    let world = |rel: &str| crate::code_rule_dsl::logic::rr::WORLD.iter().any(|(r, _)| *r == rel);
    let given = |rel: &str| !derived.contains(rel) && (option_input(rel) || world(rel) || (prog.facts.contains_key(rel) && !prog.inputs.iter().any(|i| i == rel)));
    let mut fold: BTreeSet<String> =
        derived.iter().filter(|r| !prog.natives.contains_key(**r) && !prog.lattice.contains_key(**r) && !prog.inputs.iter().any(|i| i == **r)).map(|r| (*r).to_owned()).collect();
    loop {
        let kept: BTreeSet<String> = fold
            .iter()
            .filter(|rel| {
                prog.rules.iter().filter(|r| &r.head.rel == *rel).all(|r| {
                    r.body.iter().all(|l| match l {
                        Literal::Atom(a) | Literal::Agg { atom: a, .. } => fold.contains(&a.rel) || given(&a.rel),
                        Literal::Cmp { .. } => true,
                        _ => false,
                    })
                })
            })
            .cloned()
            .collect();
        if kept == fold {
            break;
        }
        fold = kept;
    }
    // Only those that read the world (else the split derives them in both parts already).
    let constant = constant_relations(prog);
    fold.retain(|r| !constant.contains(r));
    // Of them, those something else reads, or the program outputs.
    let read_outside: BTreeSet<&str> = prog
        .rules
        .iter()
        .filter(|r| !fold.contains(&r.head.rel))
        .flat_map(|r| r.body.iter().filter_map(Literal::atom).map(|a| a.rel.as_str()))
        .chain(prog.outputs.iter().map(String::as_str))
        .collect();
    fold.into_iter().filter(|r| read_outside.contains(r.as_str())).collect()
}

/// The relation the constants' run writes `rel` as.
fn copy(rel: &str) -> String {
    format!("rr_const_{rel}")
}

/// The constants' run's program: its outputs are a copy of each wanted relation, made by a rule of its own. The copy
/// reads the relation as any rule outside the requests' cone does, so the cone hands it on wherever it is made.
pub(super) fn as_outputs(prog: &mut Program, wanted: &[String]) {
    use crate::code_rule_dsl::logic::front::{Atom, Rule, Term};
    prog.outputs.clear();
    for rel in wanted {
        let Some(cols) = prog.decls.get(rel).cloned() else { continue };
        let args: Vec<Term> = (0..cols.len()).map(|i| Term::Var(format!("c{i}"))).collect();
        let atom = |r: String| Atom { rel: r, args: args.clone(), neg: false, gated: false };
        prog.decls.insert(copy(rel), cols);
        prog.rules.push(Rule { head: atom(copy(rel)), body: vec![Literal::Atom(atom(rel.clone()))] });
        prog.outputs.push(copy(rel));
    }
}

/// Derives `prog`'s foldable relations (`foldable`) by a run over no documents, and puts their rows in as facts,
/// their rules (and those of the relations only they read) dropped. The run's consulted reads, for the record.
#[allow(clippy::too_many_arguments)]
pub(super) fn fold(
    prog: &mut Program,
    programs: &[&str],
    root: &Path,
    scratch: &Path,
    modes: &str,
    binding: crate::code_rule_dsl::typing::file_local::Binding,
    chunk_nodes: usize,
    typing: &Typing,
    options: &serde_json::Value,
) -> Result<Vec<(String, String, String)>> {
    if options.get(OWN).is_some() {
        return Ok(Vec::new());
    }
    let wanted = foldable(prog);
    if wanted.is_empty() {
        return Ok(Vec::new());
    }
    let _span = crate::code_rule_dsl::logic::profile::span(|| "fold the run's constants".into());
    let key = key(programs, root, options, &wanted);
    let was_kept = kept(&key, root, prog)?;
    let reused = was_kept.is_some();
    let (texts, consulted) = match was_kept {
        Some(kept) => kept,
        None => {
            let derived = derive(programs, root, scratch, modes, binding, chunk_nodes, typing, options, &wanted)?;
            keep(&key, &derived.0, &derived.1)?;
            derived
        }
    };
    for rel in &wanted {
        let text = texts.get(rel).map_or("", String::as_str);
        let types: Vec<String> = prog.decls.get(rel).map(|d| d.iter().map(|(_, t)| t.clone()).collect()).unwrap_or_default();
        let mut rows = Vec::new();
        for line in text.lines() {
            let vals: Vec<&str> = line.split('\t').collect();
            if vals.len() != types.len() {
                return Err(super::super::Error::Plan(format!("the run's constant {rel}: a row of {} values, not {}: {line:?}", vals.len(), types.len())));
            }
            rows.push(vals.iter().zip(&types).map(|(v, t)| if t == "number" || t == "int" { v.parse::<i64>().map_or_else(|_| Value::Str((*v).to_owned()), Value::Int) } else { Value::Str((*v).to_owned()) }).collect());
        }
        prog.facts.insert(rel.clone(), rows);
    }
    prog.rules.retain(|r| !wanted.contains(&r.head.rel));
    crate::code_rule_dsl::logic::profile::decision("the run's constants (decision 0094 §6)", || {
        format!("{} relations made from no document, {} and given as facts: {}", wanted.len(), if reused { "kept from an earlier run (what they read is as it was)" } else { "derived first" }, wanted.iter().cloned().collect::<Vec<_>>().join(", "))
    });
    Ok(consulted)
}

type Derived = (std::collections::BTreeMap<String, String>, Vec<(String, String, String)>);

/// The constants' run: the same texts, its outputs the wanted relations, over no documents. Each relation's rows as
/// the run wrote them, and what it consulted.
#[allow(clippy::too_many_arguments)]
fn derive(
    programs: &[&str],
    root: &Path,
    scratch: &Path,
    modes: &str,
    binding: crate::code_rule_dsl::typing::file_local::Binding,
    chunk_nodes: usize,
    typing: &Typing,
    options: &serde_json::Value,
    wanted: &BTreeSet<String>,
) -> Result<Derived> {
    let mut own = if options.is_object() { options.clone() } else { serde_json::json!({}) };
    own[OWN] = serde_json::Value::Array(wanted.iter().map(|r| serde_json::Value::String(r.clone())).collect());
    own["inputFiles"] = serde_json::json!([]);
    let out = scratch.join("constants");
    let report: Report = super::run_with_options(programs, root, None, &out, modes, binding, chunk_nodes, typing, &own)?;
    let texts = wanted.iter().map(|rel| (rel.clone(), std::fs::read_to_string(out.join(format!("{}.csv", copy(rel)))).unwrap_or_default())).collect();
    let _ = std::fs::remove_dir_all(&out);
    Ok((texts, report.consulted))
}

/// What the constants are keyed by, beside what their run consulted.
fn key(programs: &[&str], root: &Path, options: &serde_json::Value, wanted: &BTreeSet<String>) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"rerail.constants/v1\0");
    for p in programs {
        h.update(p.as_bytes());
        h.update([0]);
    }
    for w in wanted {
        h.update(w.as_bytes());
        h.update([0]);
    }
    // Which documents the run is over is no part of them (they read none).
    let mut options = options.clone();
    if let Some(o) = options.as_object_mut() {
        o.remove("inputFiles");
        o.remove("phase_no_loads");
    }
    h.update(options.to_string().as_bytes());
    h.update([0]);
    h.update(std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf()).to_string_lossy().as_bytes());
    h.update(env!("RERAIL_ENGINE_BUILD_ID").as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn table(db: &rusqlite::Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS constant_rows(key TEXT PRIMARY KEY, rows TEXT, consulted TEXT) WITHOUT ROWID;")?;
    Ok(())
}

/// The constants kept under `key`, when what their run consulted is as it was.
fn kept(key: &str, root: &Path, prog: &Program) -> Result<Option<Derived>> {
    let Some(path) = super::phases::store_path().filter(|p| p.exists()) else { return Ok(None) };
    let db = rusqlite::Connection::open(&path)?;
    table(&db)?;
    let Some((rows, consulted)): Option<(String, String)> = db.query_row("SELECT rows, consulted FROM constant_rows WHERE key=?1", [key], |r| Ok((r.get(0)?, r.get(1)?))).ok() else { return Ok(None) };
    let consulted: Vec<(String, String, String)> = serde_json::from_str(&consulted).map_err(|e| super::super::Error::Plan(e.to_string()))?;
    if !super::world::World::new(root, prog).still(&consulted) {
        return Ok(None);
    }
    let rows = serde_json::from_str(&rows).map_err(|e| super::super::Error::Plan(e.to_string()))?;
    Ok(Some((rows, consulted)))
}

fn keep(key: &str, rows: &std::collections::BTreeMap<String, String>, consulted: &[(String, String, String)]) -> Result<()> {
    let Some(path) = super::phases::store_path() else { return Ok(()) };
    let db = rusqlite::Connection::open(&path)?;
    table(&db)?;
    db.execute("INSERT OR REPLACE INTO constant_rows VALUES (?1, ?2, ?3)", rusqlite::params![key, serde_json::to_string(rows).unwrap_or_default(), serde_json::to_string(consulted).unwrap_or_default()])?;
    Ok(())
}
