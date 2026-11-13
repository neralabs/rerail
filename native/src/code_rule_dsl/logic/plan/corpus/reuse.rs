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
    // A run asked for a debugging dump runs: kept outputs carry none. A profile does not turn the cutoff off: a
    // profiled warm run is measured as it runs, and its profile says the cutoff applied.
    let side_output = std::env::vars_os().any(|(k, _)| k.to_str().is_some_and(|k| k.starts_with("RERAIL_DUMP_")));
    let wanted = asked && !side_output && t_facts.is_none() && typing.declarations.is_empty() && typing.site.is_none() && typing.summarize.is_none() && super::tape_cache_path().is_some();
    if !wanted {
        miss(&format!("not wanted (asked {asked}, dump {side_output}, t facts {}, typing {}, tape cache {})", t_facts.is_some(), !typing.declarations.is_empty() || typing.site.is_some() || typing.summarize.is_some(), super::tape_cache_path().is_some()));
    }
    wanted
}

/// The run's key: everything it is a function of besides what it consults.
/// Switches that change how a run is measured or held, never what it outputs: left out of the key. A profile's path
/// differs every run, so with it in the key no profiled run ever found the last run's record.
const NOT_OUTPUTS: &[&str] = &["RERAIL_TAPE_CACHE", "RERAIL_PROFILE", "RERAIL_REUSE_TRACE", "RERAIL_DEADLINE", "RERAIL_SQL_STRICT", "RERAIL_WRITE_QUEUE_MB", "RERAIL_WRITES", "RERAIL_STATS_KEEP_TRACE", "RERAIL_JOURNAL_TRACE"];

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
    // A phase's run is a function of the facts handed down to it (its files, for phase 1), which no consulted file
    // names: the same program over other files is another run.
    h.update(super::phases::handed_down_text().as_bytes());
    h.update([0]);
    h.update(crate::code_rule_dsl::build_ids::engine().as_bytes());
    h.update([u8::from(crate::code_rule_dsl::facts::vtab::syntax::definition_ids())]);
    // What can change the outputs from outside the text: every engine switch (a sample of the corpus, decision 0091's
    // guards, the matcher and planner switches). Listing only some reused a run made under others: a run with
    // `RERAIL_AST_PARTS=off` answered with the outputs of one with it on. The cache's own path names the store.
    let mut switches: Vec<(String, String)> = std::env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.to_string_lossy().into_owned())))
        .filter(|(k, _)| k.starts_with("RERAIL_") && !NOT_OUTPUTS.contains(&k.as_str()))
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
pub fn rule_files(programs: &[String], options: &serde_json::Value) -> Result<Vec<(String, String, String)>> {
    let mut out = Vec::new();
    // Phase 1's program (`sources.rr`), which every run's documents come from: its includes are core's, as the
    // programs' are.
    let sources = super::phases::sources_path();
    let digest = observe(|o| o.digest(&sources)).map(|d| crate::content_store::observe::hex(&d)).unwrap_or_default();
    out.push(("rule".to_owned(), sources.to_string_lossy().into_owned(), digest));
    // The programs' includes, and every phase program they read (with its includes).
    let dir = options.get("phase_dir").and_then(|v| v.as_str()).map(std::path::PathBuf::from);
    let mut paths = super::phases::phase_files(programs, dir.as_deref())?;
    for p in programs {
        paths.extend(crate::code_rule_dsl::logic::rr::included_files(p)?);
    }
    for path in paths {
        let digest = observe(|o| o.digest(&path)).map(|d| crate::content_store::observe::hex(&d)).unwrap_or_default();
        out.push(("rule".to_owned(), path.to_string_lossy().into_owned(), digest));
    }
    out.sort();
    out.dedup();
    Ok(out)
}

pub fn encode_consulted(consulted: &[(String, String, String)]) -> String {
    consulted.iter().map(|(w, p, d)| format!("{w}\t{p}\t{d}")).collect::<Vec<_>>().join("\n")
}

pub fn decode_consulted(text: &str) -> Vec<(String, String, String)> {
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

pub fn encode_outputs(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, bytes) in files {
        out.extend((name.len() as u32).to_le_bytes());
        out.extend(name.as_bytes());
        out.extend((bytes.len() as u64).to_le_bytes());
        out.extend(bytes);
    }
    out
}

pub fn decode_outputs(mut b: &[u8]) -> Option<Vec<(String, Vec<u8>)>> {
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
pub(super) fn reused(programs: &[&str], graphs: &[Option<[u8; 32]>], root: &Path, out: &Path, modes: &str, typing: &Typing, options: &serde_json::Value) -> Result<Option<Report>> {
    let checking = super::thread_usage();
    let texts = with_settings(programs, root, options)?;
    let key = key(&texts, root, options, modes, typing);
    let Some((consulted, outputs)) = observe(|o| o.run_record(&key)) else {
        miss(&format!("no record under the run's key {} (programs {})", &key[..12], texts.len()));
        return Ok(None);
    };
    let consulted = decode_consulted(&consulted);
    let Some(outputs) = decode_outputs(&outputs) else {
        miss("the record's outputs do not decode");
        return Ok(None);
    };
    // The library files first (a `stat` each), then the documents and the resolution rules' reads.
    let (rules, rest): (Vec<_>, Vec<_>) = consulted.into_iter().partition(|c| c.0 == "rule");
    // What the record names, by its digest, first: an edit's document or rule file differs here, before the rule
    // files are listed again (each phase program parsed) or the programs combined. Each is the observer's answer,
    // which the journal trusts unread where nothing changed.
    if let Some(c) = rules.iter().chain(rest.iter().filter(|c| c.0 == "doc" && !c.2.is_empty())).find(|c| super::world::bytes_digest(&root.join(&c.1)) != c.2) {
        miss(&format!("{} {} changed", c.0, c.1));
        return Ok(None);
    }
    if rule_files(&texts, options)? != rules {
        miss("the rule files differ");
        return Ok(None);
    }
    if !unchanged(programs, graphs, root, typing, options, &rest)? {
        miss("a consulted file or read changed");
        return Ok(None);
    }
    crate::code_rule_dsl::logic::profile::decision("whole-run cutoff (t23 §4)", || "every read as it was: the kept outputs written, nothing evaluated".into());
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
/// Why the cutoff did not apply: a profile line, and stderr under `RERAIL_REUSE_TRACE`.
fn miss(why: &str) {
    crate::code_rule_dsl::logic::profile::decision("whole-run cutoff (t23 §4)", || format!("not applied: {why}"));
    if std::env::var_os("RERAIL_REUSE_TRACE").is_some() {
        eprintln!("rerail: whole-run cutoff not applied: {why}");
    }
}

pub(super) fn record(programs: &[&str], root: &Path, out: &Path, modes: &str, typing: &Typing, options: &serde_json::Value, report: &Report) -> Result<()> {
    // Every phase's program is among the rule files (`rule_files`), and what each phase consulted is the record's
    // too: a phase's exports are a function of its program and what it consulted, so a run whose every read, its
    // phases' included, is as it was derives the same (O-cutoff-phases). Phase 1's are the source files, and the
    // directories, files and library documents its imports read.
    if report.consulted.is_empty() {
        miss("not recorded: nothing consulted");
        return Ok(());
    }
    let texts = with_settings(programs, root, options)?;
    let key = key(&texts, root, options, modes, typing);
    let mut consulted = rule_files(&texts, options)?;
    let mut read: Vec<(String, String, String)> = report.consulted.iter().cloned().chain(report.phase_runs.iter().flat_map(|r| r.report.consulted.iter().cloned())).collect();
    read.sort();
    read.dedup();
    // One entry a document: a phase that read its bytes names its digest, where another listed it unread.
    let read_docs: std::collections::BTreeSet<String> = read.iter().filter(|c| c.0 == "doc" && !c.2.is_empty()).map(|c| c.1.clone()).collect();
    read.retain(|c| !(c.0 == "doc" && c.2.is_empty() && read_docs.contains(&c.1)));
    consulted.extend(read);
    let outputs = encode_outputs(&outputs_of(out,report.rows.keys().map(String::as_str)));
    if std::env::var_os("RERAIL_REUSE_TRACE").is_some() {
        eprintln!("rerail: whole-run cutoff: recorded under {} ({} consulted)", &key[..12], consulted.len());
    }
    observe(|o| o.put_run_record(&key, &encode_consulted(&consulted), &outputs));
    Ok(())
}

// Its tests run in the engine's test binary (its facade of this module mounts `tests.rs`): they read what the
// engine registers as it loads (the languages, their `.rr` libraries).
