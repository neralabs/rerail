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
//! The constants' rows are kept among phase 0's run records (`constants:<key>`), under the programs, the relations, the run's
//! options (but which documents it runs over: the constants read none), the root and the engine, with what their run
//! consulted. A later run whose consulted things are as they were (`World::still`) takes them as they are: an edit to
//! a workspace file derives no constant again.

use std::collections::BTreeSet;
use std::path::Path;

use super::super::Result;
use super::{Report, Typing};
use crate::code_rule_dsl::logic::front::Value;
use crate::code_rule_dsl::logic::program_image::Version;
use crate::code_rule_dsl::typing::file_local::option_input;

/// The option that marks a run as the constants' own (it folds nothing).
pub(super) const OWN: &str = "constants_run";

/// The relations made from no document that a rule outside them reads, or that the program outputs: from facts,
/// settings and file-system reads, through other such relations and negations of them. Those the split already
/// treats as constants (`constant_relations`: no request among them) are left to it.
pub(super) fn foldable_of(v: &std::sync::Arc<Version>) -> Result<BTreeSet<String>> {
    use crate::code_rule_dsl::logic::program_image::{Lit, RuleRef};
    let input_names = v.input_names();
    let inputs: std::collections::HashSet<&str> = input_names.iter().map(String::as_str).collect();
    let (natives, lattice) = (v.native_map().map_err(super::super::Error::Plan)?, v.lattice_decls().map_err(super::super::Error::Plan)?);
    let outputs = v.output_names();
    let world = |rel: &str| crate::code_rule_dsl::logic::rr::WORLD.iter().any(|(r, _)| *r == rel);
    let mut heads: std::collections::HashMap<&str, Vec<RuleRef<'_>>> = std::collections::HashMap::new();
    for r in v.rules() {
        heads.entry(v.rel_name(r.head().rel())).or_default().push(r);
    }
    let candidates: std::collections::HashSet<&str> = heads.keys().copied().filter(|r| !natives.contains_key(*r) && !lattice.contains_key(*r) && !inputs.contains(r)).collect();
    // The greatest fixpoint of the candidates whose every rule reads only those left, relations `given` holds, and
    // comparisons.
    let fixpoint = |given: &dyn Fn(&str) -> bool| -> BTreeSet<String> {
        let mut fold = candidates.clone();
        loop {
            let out: Vec<&str> = fold
                .iter()
                .copied()
                .filter(|rel| {
                    !heads[rel].iter().all(|r| {
                        r.body().all(|l| match l.view() {
                            Lit::Atom(a) => fold.contains(v.rel_name(a.rel())) || given(v.rel_name(a.rel())),
                            Lit::Agg { atom, .. } => fold.contains(v.rel_name(atom.rel())) || given(v.rel_name(atom.rel())),
                            Lit::Cmp { .. } => true,
                            _ => false,
                        })
                    })
                })
                .collect();
            if out.is_empty() {
                return fold.into_iter().map(str::to_owned).collect();
            }
            for rel in out {
                fold.remove(rel);
            }
        }
    };
    let fact = |rel: &str| !heads.contains_key(rel) && v.find(rel).is_some_and(|a| v.facts(a).is_some()) && !inputs.contains(rel);
    let mut fold = fixpoint(&|rel| !heads.contains_key(rel) && (option_input(rel) || world(rel) || fact(rel)));
    // Only those that read the world: the split's constants (`file_local::FileLocal::constants`: facts, the run's
    // options and phases' exports, no request) are derived in both parts already.
    let constant = fixpoint(&|rel| option_input(rel) || (super::phases::is_phased(rel) && inputs.contains(rel)) || fact(rel));
    fold.retain(|r| !constant.contains(r));
    let shape = super::shape::Shape::of(v);
    let read_outside: BTreeSet<&str> =
        shape.reads_of.iter().filter(|(h, _)| !fold.contains(h.as_str())).flat_map(|(_, reads)| reads.iter().map(String::as_str)).chain(outputs.iter().map(String::as_str)).collect();
    Ok(fold.into_iter().filter(|r| read_outside.contains(r.as_str())).collect())
}

/// The constants' run's program: its outputs are a copy of each wanted relation, made by a rule of its own. The copy
/// reads the relation as any rule outside the requests' cone does, so the cone hands it on wherever it is made.
pub(super) fn as_outputs_of(v: &std::sync::Arc<Version>, wanted: &[String]) -> Result<std::sync::Arc<Version>> {
    use crate::code_rule_dsl::logic::program_image::{Builder, build::T};
    let mut b = Builder::on(v);
    let mut outputs = Vec::new();
    for rel in wanted {
        let Some(cols) = v.columns_of(rel) else { continue };
        let names: Vec<String> = (0..cols.len()).map(|i| format!("c{i}")).collect();
        let args: Vec<T<'_>> = names.iter().map(|n| T::Var(n.as_str())).collect();
        let pairs: Vec<(&str, &str)> = cols.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
        b.declare(&copy(rel), &pairs);
        b.rule().head(copy(rel).as_str(), &args).atom(rel.as_str(), &args).finish().map_err(super::super::Error::Plan)?;
        outputs.push(copy(rel));
    }
    b.set_outputs(&outputs);
    b.finish().map(|(v, _)| v).map_err(super::super::Error::Plan)
}

/// The relation the constants' run writes `rel` as.
fn copy(rel: &str) -> String {
    format!("rr_const_{rel}")
}

/// Derives the version's foldable relations (`foldable_of`) by a run over no documents, and puts their rows in as facts,
/// their rules (and those of the relations only they read) dropped. The run's consulted reads, for the record.
#[allow(clippy::too_many_arguments)]
pub(super) fn fold(
    v: &mut std::sync::Arc<Version>,
    programs: &[&str],
    read: Option<&std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>>,
    added: &super::added::Added,
    root: &Path,
    scratch: &Path,
    modes: &str,
    binding: crate::code_rule_dsl::typing::file_local::Binding,
    chunk_nodes: usize,
    typing: &Typing,
    options: &serde_json::Value,
) -> Result<(Vec<(String, String, String)>, Vec<[u8; 32]>, BTreeSet<String>)> {
    if options.get(OWN).is_some() {
        return Ok(Default::default());
    }
    let wanted = foldable_of(v)?;
    if wanted.is_empty() {
        return Ok(Default::default());
    }
    let _span = crate::code_rule_dsl::logic::profile::span(|| "fold the run's constants".into());
    let key = key(programs, added, root, options, &wanted);
    let was_kept = kept(&key, root, &**v)?;
    let reused = was_kept.is_some();
    // The plans the constants' run used, for the outer run's record (`Report::plans`): kept with the constants, so a
    // run that reuses them names the plans the run that derived them did, and records the same manifest (0252).
    let ((image, consulted), plans) = match was_kept {
        Some(kept) => kept,
        None => {
            let (derived, plans) = derive(programs, read, root, scratch, modes, binding, chunk_nodes, typing, options, &wanted)?;
            keep(&key, &derived.0, &derived.1, &plans)?;
            (derived, plans)
        }
    };
    // Each relation's rows from the image, typed as its columns are declared: nothing parsed.
    let outputs = crate::code_rule_dsl::logic::plan::output::Outputs::of(&image).map_err(|e| super::super::Error::Plan(format!("the run's constants: {e}")))?;
    let mut folded: Vec<(String, Vec<crate::code_rule_dsl::logic::front::List<Value>>)> = Vec::new();
    for rel in &wanted {
        let types: Vec<String> = v.columns_of(rel).map(|d| d.into_iter().map(|(_, t)| t).collect()).unwrap_or_default();
        let mut rows = Vec::new();
        for row in outputs.rows(&copy(rel)) {
            if row.len() != types.len() {
                return Err(super::super::Error::Plan(format!("the run's constant {rel}: a row of {} values, not {}: {row:?}", row.len(), types.len())));
            }
            let values = row
                .values()
                .zip(&types)
                .map(|(v, t)| match (v.text(), v.int()) {
                    (Some(text), _) => Ok(Value::Str(text.to_owned().into())),
                    (None, Some(n)) if t == "number" || t == "int" => Ok(Value::Int(n)),
                    _ => Err(super::super::Error::Plan(format!("the run's constant {rel}: {v:?} in a column of type {t}"))),
                })
                .collect::<Result<Vec<_>>>()?;
            rows.push(values.into());
        }
        folded.push((rel.clone(), rows));
    }
    // The version: the rows as facts, the relations' rules dropped.
    let mut b = crate::code_rule_dsl::logic::program_image::Builder::on(v);
    for (rel, rows) in &folded {
        b.set_facts(rel, rows);
    }
    b.retain(|r| !wanted.contains(v.rel_name(r.head().rel())));
    *v = b.finish().map(|(v, _)| v).map_err(super::super::Error::Plan)?;
    crate::code_rule_dsl::logic::profile::decision("the run's constants (decision 0094 §6)", || {
        format!("{} relations made from no document, {} and given as facts: {}", wanted.len(), if reused { "kept from an earlier run (what they read is as it was)" } else { "derived first" }, wanted.iter().cloned().collect::<Vec<_>>().join(", "))
    });
    Ok((consulted, plans, wanted))
}

/// The constants' outputs (one base image of the wanted relations' copies, `output::base_of`) and what their run
/// consulted.
type Derived = (Vec<u8>, Vec<(String, String, String)>);

/// The constants' run: the same texts (or the same program read, `read`), its outputs the wanted relations, over no
/// documents. Each relation's rows as the run wrote them, and what it consulted.
#[allow(clippy::too_many_arguments)]
fn derive(
    programs: &[&str],
    read: Option<&std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>>,
    root: &Path,
    scratch: &Path,
    modes: &str,
    binding: crate::code_rule_dsl::typing::file_local::Binding,
    chunk_nodes: usize,
    typing: &Typing,
    options: &serde_json::Value,
    wanted: &BTreeSet<String>,
) -> Result<(Derived, Vec<[u8; 32]>)> {
    let mut own = if options.is_object() { options.clone() } else { serde_json::json!({}) };
    own[OWN] = serde_json::Value::Array(wanted.iter().map(|r| serde_json::Value::String(r.clone())).collect());
    own["inputFiles"] = serde_json::json!([]);
    let out = scratch.join("constants");
    let report: Report = super::run_handed(programs, read, None, root, None, &out, modes, binding, chunk_nodes, typing, &own)?;
    let copies: Vec<String> = wanted.iter().map(|rel| copy(rel)).collect();
    let image = crate::code_rule_dsl::logic::plan::output::base_of(&out, copies.iter().map(String::as_str)).map_err(|e| super::super::Error::Plan(format!("the run's constants: {e}")))?;
    let _ = std::fs::remove_dir_all(&out);
    Ok(((image, report.consulted), report.plans))
}

/// What the constants are keyed by, beside what their run consulted.
fn key(programs: &[&str], added: &super::added::Added, root: &Path, options: &serde_json::Value, wanted: &BTreeSet<String>) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    // v5: the rows the run adds (its settings, decision 0249) are keyed as rows, the texts as the files'; the record
    // names its run's plans (decision 0252).
    h.update(b"rerail.constants/v5\0");
    for p in programs {
        h.update(p.as_bytes());
        h.update([0]);
    }
    added.hash(&mut h);
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
    h.update(crate::code_rule_dsl::build_ids::engine().as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The record the constants of `key` are kept under, among phase 0's run records (`content_store::observe`): read
/// with the observer's other records before the run, written behind it by the observer's writer (decision 0104).
fn record_key(key: &str) -> String {
    format!("constants:{key}")
}

/// The constants kept under `key`, when what their run consulted is as it was, with the plans their run used: the
/// record's `plan` lines, for collection (`observe::collect` drops the record once none is live) and for the outer
/// run's manifest.
fn kept(key: &str, root: &Path, prog: &Version) -> Result<Option<(Derived, Vec<[u8; 32]>)>> {
    let Some((consulted, rows)) = super::observe(|o| o.run_record(&record_key(key))) else { return Ok(None) };
    let Some(consulted) = super::reuse::decode_consulted(&consulted) else { return Ok(None) };
    let (plans, consulted): (Vec<_>, Vec<_>) = consulted.into_iter().partition(|c| c.0 == "plan");
    let plans: Vec<[u8; 32]> = plans.iter().filter_map(|c| crate::content_store::observe::unhex(&c.2)).collect();
    if !super::world::World::new(root, prog).still(&consulted) {
        return Ok(None);
    }
    Ok(Some(((rows, consulted), plans)))
}

fn keep(key: &str, image: &[u8], consulted: &[(String, String, String)], plans: &[[u8; 32]]) -> Result<()> {
    let mut record = consulted.to_vec();
    record.extend(plans.iter().map(|p| ("plan".to_owned(), String::new(), crate::content_store::observe::hex(p))));
    let consulted = super::reuse::encode_consulted(&record);
    super::observe(|o| o.put_run_record(&record_key(key), &consulted, image));
    Ok(())
}
