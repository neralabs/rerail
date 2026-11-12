//! Phase 0's cutoff for a whole run (t23 §4): a run whose every read is as it was writes the outputs it wrote.
//!
//! A run consults a known set of things, and nothing else:
//! - its programs' text, with the root's settings;
//! - the library files that text includes;
//! - the workspace's documents;
//! - what its resolution rules listed, read, loaded, or found missing (`world::World::consulted`).
//!
//! After a run, its record keeps all of these, each with its digest, under a key of the text, the options, the
//! root, the modes and the engine, together with the outputs it wrote. The next run under the same key asks phase
//! 0's observer about each consulted thing. That costs a `stat` each, and a read only for what changed. When all
//! are as they were, the run writes the kept outputs and evaluates nothing: the empty delta stops before phase 1.
//!
//! The cutoff is on with `options.reuse`: the command line's runs, and phase 0's. A run that reads something it cannot name does not record. That covers typing inputs,
//! T facts, a program reading another phase's exports, or a run that consulted no file.

use super::{Report, Result, Typing, observe, unchanged};
use super::Error;
use sha2::{Digest as _, Sha256};
use std::path::Path;

/// Whether the cutoff applies to a run with these options and typing inputs.
pub(super) fn wanted(options: &serde_json::Value, typing: &Typing, t_facts: Option<&Path>) -> bool {
    let asked = options.get("reuse").and_then(serde_json::Value::as_bool).unwrap_or(false);
    // A run asked for what only running writes (a profile, a debugging dump) runs: kept outputs carry neither.
    let side_output = crate::code_rule_dsl::logic::profile::on() || std::env::vars_os().any(|(k, _)| k.to_str().is_some_and(|k| k.starts_with("RERAIL_DUMP_")));
    asked && !side_output && t_facts.is_none() && typing.declarations.is_empty() && typing.site.is_none() && typing.summarize.is_none() && super::tape_cache_path().is_some()
}

/// The run's key: everything it is a function of besides what it consults.
fn key(programs: &[String], root: &Path, options: &serde_json::Value, modes: &str, typing: &Typing) -> String {
    let mut h = Sha256::new();
    h.update(b"rerail.run-reuse/v1\0");
    for p in programs {
        h.update((p.len() as u64).to_le_bytes());
        h.update(p.as_bytes());
    }
    h.update(options.to_string().as_bytes());
    h.update([0]);
    h.update(std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf()).to_string_lossy().as_bytes());
    h.update([0]);
    h.update(modes.as_bytes());
    h.update(format!("\0{typing:?}\0").as_bytes());
    h.update(crate::code_rule_dsl::build_ids::engine().as_bytes());
    h.update([u8::from(crate::code_rule_dsl::facts::vtab::syntax::definition_ids())]);
    // What can change the outputs from outside the text: every engine switch (a sample of the corpus, decision 0091's
    // guards, the matcher and planner switches). Listing only some reused a run made under others: a run with
    // `RERAIL_AST_PARTS=off` answered with the outputs of one with it on. The cache's own path names the store.
    let mut switches: Vec<(String, String)> = std::env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.to_string_lossy().into_owned())))
        .filter(|(k, _)| k.starts_with("RERAIL_") && k != "RERAIL_TAPE_CACHE")
        .collect();
    switches.sort();
    for (k, v) in switches {
        h.update(k.as_bytes());
        h.update([0]);
        h.update(v.as_bytes());
        h.update([0]);
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The programs with the root's settings (`<root>/rerail.env`, the `env` option), as the run reads them.
fn with_settings(programs: &[&str], root: &Path, options: &serde_json::Value) -> Result<Vec<String>> {
    let flags: Vec<String> = options.get("env").and_then(|v| v.as_array()).into_iter().flatten().filter_map(|v| v.as_str().map(str::to_owned)).collect();
    let settings = crate::code_rule_dsl::logic::settings::settings(root, &flags).map_err(Error::Plan)?;
    Ok(programs.iter().map(|p| crate::code_rule_dsl::logic::settings::with_settings(p, &settings)).collect())
}

/// The library files the programs include, each with its digest now: the record's `rule` entries.
fn rule_files(programs: &[String]) -> Result<Vec<(String, String, String)>> {
    let mut out = Vec::new();
    // Phase 1's program (`sources.rr`), which every run's documents come from: its includes are core's, as the
    // programs' are.
    let sources = super::phases::sources_path();
    let digest = observe(|o| o.digest(&sources)).map(|d| crate::content_store::observe::hex(&d)).unwrap_or_default();
    out.push(("rule".to_owned(), sources.to_string_lossy().into_owned(), digest));
    for p in programs {
        for path in crate::code_rule_dsl::logic::rr::included_files(p)? {
            let digest = observe(|o| o.digest(&path)).map(|d| crate::content_store::observe::hex(&d)).unwrap_or_default();
            out.push(("rule".to_owned(), path.to_string_lossy().into_owned(), digest));
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

fn encode_consulted(consulted: &[(String, String, String)]) -> String {
    consulted.iter().map(|(w, p, d)| format!("{w}\t{p}\t{d}")).collect::<Vec<_>>().join("\n")
}

fn decode_consulted(text: &str) -> Vec<(String, String, String)> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.splitn(3, '\t');
            Some((f.next()?.to_owned(), f.next()?.to_owned(), f.next()?.to_owned()))
        })
        .collect()
}

/// The outputs a run wrote: each `<rel>.csv` in `out`, as (name, bytes).
fn outputs_of<'a>(out: &Path, names: impl Iterator<Item=&'a str>) -> Vec<(String,Vec<u8>)> {
    let mut files=names.filter_map(|name| {
        let name=format!("{name}.csv");
        std::fs::read(out.join(&name)).ok().map(|bytes|(name,bytes))
    }).collect::<Vec<_>>();
    files.sort(); files
}

fn encode_outputs(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, bytes) in files {
        out.extend((name.len() as u32).to_le_bytes());
        out.extend(name.as_bytes());
        out.extend((bytes.len() as u64).to_le_bytes());
        out.extend(bytes);
    }
    out
}

fn decode_outputs(mut b: &[u8]) -> Option<Vec<(String, Vec<u8>)>> {
    let mut files = Vec::new();
    while !b.is_empty() {
        let n = u32::from_le_bytes(b.get(..4)?.try_into().ok()?) as usize;
        let name = String::from_utf8(b.get(4..4 + n)?.to_vec()).ok()?;
        b = &b[4 + n..];
        let m = u64::from_le_bytes(b.get(..8)?.try_into().ok()?) as usize;
        files.push((name, b.get(8..8 + m)?.to_vec()));
        b = &b[8 + m..];
    }
    Some(files)
}

/// The kept outputs written into `out`, when every read of the last run under this key is as it was.
#[allow(clippy::too_many_arguments)]
pub(super) fn reused(programs: &[&str], root: &Path, out: &Path, modes: &str, typing: &Typing, options: &serde_json::Value) -> Result<Option<Report>> {
    let checking = super::thread_usage();
    let texts = with_settings(programs, root, options)?;
    let key = key(&texts, root, options, modes, typing);
    let Some((consulted, outputs)) = observe(|o| o.run_record(&key)) else { return Ok(None) };
    let consulted = decode_consulted(&consulted);
    let Some(outputs) = decode_outputs(&outputs) else { return Ok(None) };
    // The library files first (a `stat` each), then the documents and the resolution rules' reads.
    let (rules, rest): (Vec<_>, Vec<_>) = consulted.into_iter().partition(|c| c.0 == "rule");
    if rule_files(&texts)? != rules || !unchanged(programs, root, typing, options, &rest)? {
        return Ok(None);
    }
    std::fs::create_dir_all(out).map_err(|e| Error::Plan(format!("{}: {e}", out.display())))?;
    let mut report = Report::default();
    for (name, bytes) in &outputs {
        std::fs::write(out.join(name), bytes).map_err(|e| Error::Plan(format!("{name}: {e}")))?;
        let rel = name.trim_end_matches(".csv").to_owned();
        report.rows.insert(rel, bytes.iter().filter(|b| **b == b'\n').count());
    }
    let now = super::thread_usage();
    report.phases.push(("unchanged: phase 0 checked every read, outputs kept", now.0 - checking.0, now.1 - checking.1, super::usage().2, 0));
    // The documents the run read: the workspace's and those its requests loaded.
    report.files = rest.iter().filter(|c| (c.0 == "doc" && !c.2.is_empty()) || c.0 == "load").count();
    report.consulted = rest;
    Ok(Some(report))
}

/// Keeps the record of a run that ended: what it consulted (and the library files its text includes), and its
/// outputs.
pub(super) fn record(programs: &[&str], root: &Path, out: &Path, modes: &str, typing: &Typing, options: &serde_json::Value, report: &Report) -> Result<()> {
    // A phase's program and exports are its own record's (`phases`), not this one's: such a run is not kept. Phase 1's
    // program is among the rule files, and what it consulted (the source files, and the directories, files and
    // library documents its imports read) is the record's too: a run of the same files would load the same.
    if report.consulted.is_empty() || report.phase_runs.iter().any(|r| r.name != "sources") {
        return Ok(());
    }
    let texts = with_settings(programs, root, options)?;
    let key = key(&texts, root, options, modes, typing);
    let mut consulted = rule_files(&texts)?;
    consulted.extend(report.consulted.iter().cloned());
    consulted.extend(report.phase_runs.iter().filter(|r| r.name == super::phases::SOURCES).flat_map(|r| r.report.consulted.iter().cloned()));
    let outputs = encode_outputs(&outputs_of(out,report.rows.keys().map(String::as_str)));
    observe(|o| o.put_run_record(&key, &encode_consulted(&consulted), &outputs));
    Ok(())
}

#[cfg(test)]
mod tests;
