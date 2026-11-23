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
//! rule after it, runs in the cross-file part over every document's base rows: `struct.rerail`'s definitions were all
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

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::super::Result;
use super::{Report, Typing};
use crate::code_rule_dsl::logic::front::Value;
use crate::code_rule_dsl::logic::program_image::Version;
use crate::code_rule_dsl::typing::file_local::option_input;

/// The option that marks a run as the constants' own (it folds nothing).
pub const OWN: &str = "constants_run";

/// The relations made from no document that a rule outside them reads, or that the program outputs: from facts,
/// settings and file-system reads, through other such relations and negations of them. Those the split already
/// treats as constants (`constant_relations`: no request among them) are left to it.
pub(super) fn foldable_of(v: &std::sync::Arc<Version>) -> Result<BTreeSet<String>> {
    use crate::code_rule_dsl::logic::program_image::{Lit, RuleRef};
    let input_names = v.input_names();
    let inputs: std::collections::HashSet<&str> = input_names.iter().map(String::as_str).collect();
    let (natives, lattice) = (v.native_map().map_err(super::super::Error::Plan)?, v.lattice_decls().map_err(super::super::Error::Plan)?);
    let outputs = v.output_names();
    let world = |rel: &str| crate::code_rule_dsl::logic::rr::FILE_SYSTEM.iter().any(|(r, _)| *r == rel);
    // A language's answer is a constant where its request is (as `foldable`'s, 0327 mode (b)); not in the split's
    // constants (`constant_relations` reads no requests). Not the engine's other answers (Git's, the references'):
    // their requests can be derived from documents and other answers.
    // Found once a name: each was a search of the world's relations and an answerer's lookup, per atom read.
    let requests: std::collections::HashMap<&str, &str> = crate::code_rule_dsl::logic::rr::WORLD.iter().rev().filter(|(r, q)| !world(r) && crate::code_rule_dsl::language::answerer(q).is_some()).map(|&(r, q)| (r, q)).collect();
    let request_of = |rel: &str| requests.get(rel).copied();
    let mut heads: std::collections::HashMap<&str, Vec<RuleRef<'_>>> = std::collections::HashMap::new();
    for r in v.rules() {
        heads.entry(v.rel_name(r.head().rel())).or_default().push(r);
    }
    let candidates: std::collections::HashSet<&str> = heads.keys().copied().filter(|r| !natives.contains_key(*r) && !lattice.contains_key(*r) && !inputs.contains(r)).collect();
    // The greatest fixpoint of the candidates whose every rule reads only those left, relations `given` holds, and
    // comparisons: by a worklist, each candidate checked once and again only when a relation it reads (or the
    // request of one) leaves (each round of every candidate's rules was 5 ms of a warm run).
    // Each candidate's rules as the relations their bodies read, decoded once for both fixpoints: `None` a literal
    // that is neither an atom, an aggregate nor a comparison.
    let bodies: rustc_hash::FxHashMap<&str, Vec<Vec<Option<&str>>>> = candidates
        .iter()
        .map(|&head| {
            let rules = heads[head]
                .iter()
                .map(|r| {
                    r.body()
                        .filter_map(|l| match l.view() {
                            Lit::Atom(a) => Some(Some(v.rel_name(a.rel()))),
                            Lit::Agg { atom, .. } => Some(Some(v.rel_name(atom.rel()))),
                            Lit::Cmp { .. } => None,
                            _ => Some(None),
                        })
                        .collect()
                })
                .collect();
            (head, rules)
        })
        .collect();
    let fixpoint = |given: &dyn Fn(&str) -> bool, requests: bool| -> BTreeSet<String> {
        let mut fold = candidates.clone();
        // The candidates reading each relation, and those reading a relation whose request it is.
        let mut readers: rustc_hash::FxHashMap<&str, Vec<&str>> = rustc_hash::FxHashMap::default();
        for (&head, rules) in &bodies {
            for read in rules.iter().flatten().flatten().copied() {
                readers.entry(read).or_default().push(head);
                if requests && let Some(q) = request_of(read) {
                    readers.entry(q).or_default().push(head);
                }
            }
        }
        // `given` asked once a relation read (or request), not at each check of each rule.
        let given: rustc_hash::FxHashSet<&str> = readers.keys().copied().filter(|r| given(r)).collect();
        let reads = |fold: &std::collections::HashSet<&str>, rel: &str| fold.contains(rel) || given.contains(rel) || (requests && request_of(rel).is_some_and(|q| fold.contains(q) || given.contains(q)));
        let holds = |fold: &std::collections::HashSet<&str>, rel: &str| bodies[rel].iter().all(|body| body.iter().all(|read| read.is_some_and(|r| reads(fold, r))));
        let mut queue: Vec<&str> = candidates.iter().copied().collect();
        while let Some(rel) = queue.pop() {
            if fold.contains(rel) && !holds(&fold, rel) {
                fold.remove(rel);
                queue.extend(readers.get(rel).into_iter().flatten().copied().filter(|r| fold.contains(r)));
            }
        }
        fold.into_iter().map(str::to_owned).collect()
    };
    let fact = |rel: &str| !heads.contains_key(rel) && v.find(rel).is_some_and(|a| v.facts(a).is_some()) && !inputs.contains(rel);
    let mut fold = fixpoint(&|rel| !heads.contains_key(rel) && (option_input(rel) || world(rel) || fact(rel)), true);
    // Only those that read the world: the split's constants (`file_local::FileLocal::constants`: facts, the run's
    // options and phases' exports, no request) are derived in both parts already.
    let constant = fixpoint(&|rel| option_input(rel) || (super::phases::is_phased(rel) && inputs.contains(rel)) || fact(rel), false);
    fold.retain(|r| !constant.contains(r));
    let shape = crate::code_rule_dsl::logic::program_image::manager::analysis::<super::shape::Shape>(v).unwrap_or_else(|_| std::sync::Arc::new(super::shape::Shape::of(v)));
    let read_outside: BTreeSet<&str> =
        shape.reads_of.iter().filter(|(h, _)| !fold.contains(h.as_str())).flat_map(|(_, reads)| reads.iter().map(String::as_str)).chain(outputs.iter().map(String::as_str)).collect();
    Ok(fold.into_iter().filter(|r| read_outside.contains(r.as_str())).collect())
}

/// The constants' run's program: its outputs are a copy of each wanted relation, made by a rule of its own. The copy
/// reads the relation as any rule outside the requests' cone does, so the cone hands it on wherever it is made.
pub fn as_outputs_of(v: &std::sync::Arc<Version>, wanted: &[String]) -> Result<std::sync::Arc<Version>> {
    use crate::code_rule_dsl::logic::program_image::{Builder, build::T};
    let mut b = Builder::on(v);
    let mut outputs = Vec::new();
    for rel in wanted {
        let Some(cols) = v.columns_of(rel) else { continue };
        let names: Vec<String> = (0..cols.len()).map(|i| format!("c{i}")).collect();
        let args: Vec<T<'_>> = names.iter().map(|n| T::Var(n.as_str())).collect();
        let pairs: Vec<(&str, &str)> = cols.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
        b.fields().declare(&copy(rel), &pairs);
        b.rule().head(copy(rel).as_str(), &args).atom(rel.as_str(), &args).finish().map_err(super::super::Error::Plan)?;
        outputs.push(copy(rel));
    }
    b.fields().set_outputs(&outputs);
    // An included core library declares Git inputs for the whole policy. Only inputs used by this derivation
    // (including its world requests) can make its result depend on the revision.
    use crate::code_rule_dsl::logic::program_image::{About, RelId, analyses};
    let about = About::of(v);
    let reached = |roots: &BTreeSet<String>| -> BTreeSet<String> {
        let need = analyses::needed(v, &about, roots.iter().filter_map(|r| v.rel(r)));
        (0..v.rel_end()).filter(|&i| need[i as usize]).map(|i| v.rel_name(RelId(i)).to_owned()).collect()
    };
    let mut roots: BTreeSet<String> = wanted.iter().cloned().collect();
    let needed = reached(&roots);
    roots.extend(crate::code_rule_dsl::logic::rr::WORLD.iter().filter(|(answer, _)| needed.contains(*answer)).map(|(_, request)| request.to_string()));
    let needed = reached(&roots);
    b.fields().retain_inputs(|name| !super::git::INPUTS.contains(&name) || needed.contains(name));
    b.finish().map(|(v, _)| v).map_err(super::super::Error::Plan)
}

/// The relation the constants' run writes `rel` as.
fn copy(rel: &str) -> String {
    format!("rr_const_{rel}")
}

/// Derives the version's foldable relations (`foldable_of`) by a run over no documents, and puts their rows in as facts,
/// their rules (and those of the relations only they read) dropped. The run's consulted reads, for the record.
#[allow(clippy::too_many_arguments)]
pub fn fold(
    v: &mut std::sync::Arc<Version>,
    programs: &[&str],
    read: Option<&std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>>,
    added: &super::added::Added,
    root: &Path,
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
    let revision = crate::git_workspace::snapshot::contains(root);
    let was_kept = if revision { kept_revision(&key, root, v) } else { kept(&key, root, v)? };
    let reused = was_kept.is_some();
    // The plans the constants' run used, for the outer run's record (`Report::plans`): kept with the constants, so a
    // run that reuses them names the plans the run that derived them did, and records the same manifest (0252).
    let ((image, consulted), plans) = match was_kept {
        Some(kept) => kept,
        None => {
            let (mut derived, mut plans) = derive(programs, read, root, modes, binding, chunk_nodes, typing, options, &wanted)?;
            if revision {
                if let Some(owner) = keep_revision(&key, root, &mut derived, &plans) { plans.push(owner); }
            } else {
                keep(&key, &derived.0, &derived.1, &plans)?;
            }
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
        b.fields().set_facts(rel, rows);
    }
    b.retain(|r| !wanted.contains(v.rel_name(r.head().rel())));
    *v = b.finish().map(|(v, _)| v).map_err(super::super::Error::Plan)?;
    crate::code_rule_dsl::logic::profile::decision("the run's constants (decision 0094 §6)", || {
        format!("{} relations made from no document, {} and given as facts: {}", wanted.len(), if reused { "kept from an earlier run (what they read is as it was)" } else { "derived first" }, wanted.iter().cloned().collect::<Vec<_>>().join(", "))
    });
    Ok((consulted, plans, wanted))
}

/// The constants' outputs (one base image of the wanted relations' copies) and what their run
/// consulted.
type Derived = (Vec<u8>, Vec<(String, String, String)>);

/// The constants' run: the same texts (or the same program read, `read`), its outputs the wanted relations, over no
/// documents. Each relation's rows as the run wrote them, and what it consulted.
#[allow(clippy::too_many_arguments)]
fn derive(
    programs: &[&str],
    read: Option<&std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>>,
    root: &Path,
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
    let mut captured = Vec::new();
    let report: Report = super::run_handed(programs, read, None, root, None, Path::new(""), modes, binding,
        chunk_nodes, typing, &own, Some(&mut captured), Default::default())?;
    let copies: Vec<String> = wanted.iter().map(|rel| copy(rel)).collect();
    let stack = super::output::Image::stack(&captured).map_err(super::Error::Plan)?;
    let image = super::output::selected_base(&stack, copies.iter().map(String::as_str));
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
    if let Some(stable) = crate::git_workspace::snapshot::fact_root(root) {
        h.update(b"git-constants-v1\0");
        h.update(stable.as_os_str().as_encoded_bytes());
    } else {
        h.update(crate::git_workspace::snapshot::canonicalize(root).unwrap_or_else(|_| root.to_path_buf()).to_string_lossy().as_bytes());
    }
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
    let mut leaves = super::leaves::Leaves::new(root, prog);
    if !consulted.iter().all(|c| leaves.still(c)) {
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

type Observation = (String, String, String);

// The same program can read different paths, and each set of paths can have different values.
// Keep both in the record name. Returning to an earlier revision can then find its result again.
fn revision_record(base: &str, reads: &[Observation]) -> (String, [u8; 32]) {
    use sha2::{Digest, Sha256};
    let field = |hash: &mut Sha256, value: &str| {
        hash.update((value.len() as u64).to_le_bytes());
        hash.update(value.as_bytes());
    };
    let (mut shape, mut values) = (Sha256::new(), Sha256::new());
    for (kind, path, value) in reads {
        field(&mut shape, kind);
        field(&mut shape, path);
        // Repacking changes the observation of storage, while the blob remains the same.
        field(&mut values, if kind == "git_file_storage" { value.split(':').next().unwrap_or(value) } else { value });
    }
    let hash = |h: Sha256| crate::content_store::observe::hex(&h.finalize().into());
    let name = format!("{}:{}:{}", record_key(base), hash(shape), hash(values));
    let owner = Sha256::digest(name.as_bytes()).into();
    (name, owner)
}

fn kept_revision(base: &str, root: &Path, program: &Version) -> Option<(Derived, Vec<[u8; 32]>)> {
    let mut world = super::world::World::new(root, program);
    let samples = super::observe(|o| o.run_record_samples(&record_key(base)));
    'sample: for sample in samples {
        let Some(record) = super::reuse::decode_consulted(&sample) else { continue };
        let reads: Option<Vec<_>> = record.into_iter().filter(|(k, _, _)| k != "plan" && k != "constants-owner")
            .map(|(k, p, _)| world.observation(&k, &p).map(|now| (k, p, now))).collect();
        let Some(reads) = reads else { continue };
        let (name, owner) = revision_record(base, &reads);
        let Some((record, image)) = super::observe(|o| o.stored_run_record(&name)) else { continue };
        let Some(mut recorded) = super::reuse::decode_consulted(&record) else { continue };
        let digests: BTreeMap<_, _> = reads.iter().filter(|(k, _, _)| k == "file" || k == "digest")
            .map(|(_, path, digest)| (path, digest)).collect();
        let mut updated = false;
        let mut previous = recorded.iter_mut().filter(|(k, _, _)| k != "plan" && k != "constants-owner");
        for (kind, path, value) in &reads {
            let Some(old) = previous.next() else { continue 'sample };
            if &old.0 != kind || &old.1 != path { continue 'sample; }
            if &old.2 == value { continue; }
            if kind != "git_file_storage" || !digests.get(path).is_some_and(|digest| verified_file(root, path, value, digest)) { continue 'sample; }
            old.2.clone_from(value);
            updated = true;
        }
        if previous.next().is_some() { continue; }
        if updated {
            let encoded = super::reuse::encode_consulted(&recorded);
            super::observe(|o| o.store_run_record(&name, &encoded, &image));
        }
        let mut plans = recorded.iter().filter(|(k, _, _)| k == "plan")
            .map(|(_, _, s)| crate::content_store::observe::unhex(s)).collect::<Option<Vec<_>>>()?;
        plans.push(owner);
        return Some(((image, reads), plans));
    }
    None
}

fn verified_file(root: &Path, path: &str, stamp: &str, digest: &str) -> bool {
    use crate::git_workspace::snapshot;
    use sha2::{Digest, Sha256};
    let file = root.join(path);
    let Ok(bytes) = snapshot::read(&file) else { return false };
    let sha = crate::content_store::observe::hex(&Sha256::digest(bytes).into());
    sha == digest && snapshot::file_stamp(&file).as_deref() == Some(stamp)
}

fn keep_revision(base: &str, root: &Path, (image, reads): &mut Derived, plans: &[[u8; 32]]) -> Option<[u8; 32]> {
    use crate::git_workspace::snapshot;
    let verified: BTreeSet<_> = reads.iter().filter(|(k, _, s)| k == "git_file_storage" && !s.is_empty())
        .map(|(_, path, _)| path).collect();
    for (kind, path, digest) in reads.iter() {
        if kind == "git_file_storage" && digest.is_empty() { return None; }
        if kind == "refused" && digest == "file" { return None; }
        if (kind == "file" || kind == "digest") && !verified.contains(path) && snapshot::contains(&root.join(path)) { return None; }
    }
    reads.sort();
    reads.dedup();
    let (name, owner) = revision_record(base, reads);
    let mut record = reads.clone();
    record.extend(plans.iter().map(|p| ("plan".into(), String::new(), crate::content_store::observe::hex(p))));
    record.push(("constants-owner".into(), String::new(), crate::content_store::observe::hex(&owner)));
    let encoded = super::reuse::encode_consulted(&record);
    super::observe(|o| o.store_run_record(&name, &encoded, image));
    Some(owner)
}
