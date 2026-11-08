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
//! The cutoff is on with `options.reuse` (the command line's runs) or `RERAIL_RUN_REUSE=on`, and off with
//! `RERAIL_RUN_REUSE=off`. A run that reads something it cannot name does not record. That covers typing inputs,
//! T facts, a program reading another phase's exports, or a run that consulted no file.

use super::{Report, Result, Typing, observe, unchanged};
use super::Error;
use sha2::{Digest as _, Sha256};
use std::path::Path;

/// Whether the cutoff applies to a run with these options and typing inputs.
pub(super) fn wanted(options: &serde_json::Value, typing: &Typing, t_facts: Option<&Path>) -> bool {
    let asked = match std::env::var("RERAIL_RUN_REUSE").as_deref() {
        Ok("off") => return false,
        Ok("on") => true,
        _ => options.get("reuse").and_then(serde_json::Value::as_bool).unwrap_or(false),
    };
    asked && t_facts.is_none() && typing.declarations.is_empty() && typing.site.is_none() && typing.summarize.is_none() && super::tape_cache_path().is_some()
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
    h.update(env!("RERAIL_ENGINE_BUILD_ID").as_bytes());
    h.update([u8::from(crate::code_rule_dsl::facts::vtab::syntax::definition_ids())]);
    // What changes the outputs from outside the text: a sample of the corpus, decision 0091's guards.
    for var in ["RERAIL_SAMPLE_EVERY", "RERAIL_SCOPE_GUARDS"] {
        h.update(std::env::var(var).unwrap_or_default().as_bytes());
        h.update([0]);
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The programs with the root's settings (`<root>/rerail.env`, the `env` option), as the run reads them.
fn with_settings(programs: &[&str], root: &Path, options: &serde_json::Value) -> Result<Vec<String>> {
    let flags: Vec<String> = options.get("env").and_then(|v| v.as_array()).into_iter().flatten().filter_map(|v| v.as_str().map(str::to_owned)).collect();
    let settings = crate::code_rule_dsl::tools::settings(root, &flags).map_err(Error::Plan)?;
    Ok(programs.iter().map(|p| crate::code_rule_dsl::tools::with_settings(p, &settings)).collect())
}

/// The library files the programs include, each with its digest now: the record's `rule` entries.
fn rule_files(programs: &[String]) -> Result<Vec<(String, String, String)>> {
    let mut out = Vec::new();
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
fn outputs_of(out: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(out)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.ends_with(".csv").then(|| std::fs::read(e.path()).ok().map(|b| (name, b))).flatten()
        })
        .collect();
    files.sort();
    files
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
    report.files = rest.iter().filter(|c| c.0 == "doc").count();
    report.consulted = rest;
    Ok(Some(report))
}

/// Keeps the record of a run that ended: what it consulted (and the library files its text includes), and its
/// outputs.
pub(super) fn record(programs: &[&str], root: &Path, out: &Path, modes: &str, typing: &Typing, options: &serde_json::Value, report: &Report) -> Result<()> {
    // A phase's program and exports are its own record's (`phases`), not this one's: such a run is not kept.
    if report.consulted.is_empty() || !report.phase_runs.is_empty() {
        return Ok(());
    }
    let texts = with_settings(programs, root, options)?;
    let key = key(&texts, root, options, modes, typing);
    let mut consulted = rule_files(&texts)?;
    consulted.extend(report.consulted.iter().cloned());
    let outputs = encode_outputs(&outputs_of(out));
    observe(|o| o.put_run_record(&key, &encode_consulted(&consulted), &outputs));
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::code_rule_dsl::typing::file_local::Binding;

    /// An unchanged run is kept; a document's edit, a new document, the root's settings (read by `env`) and the text
    /// each run it again.
    #[test]
    fn an_unchanged_run_writes_its_kept_outputs() {
        let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("reuse");
        let _ = std::fs::remove_dir_all(&dir);
        let src = dir.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("a.py"), "def f():\n    return 1\n").unwrap();
        let out = dir.join("out");
        let _no_session = super::super::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        super::super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
        let program = |name: &str| {
            let text = format!("#include \"core.rr\"\n.decl {name}(s: symbol)\n{name}(x) :- kind(n, \"function_definition\"), field(n, \"name\", i), text(i, x).\n.output {name}\n");
            crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse(&text).unwrap()).unwrap()
        };
        let options = serde_json::json!({ "reuse": true });
        let run = |text: &str| super::super::run_with_options(&[text], &src, None, &out, "program", Binding::Cpu, 1 << 18, &Default::default(), &options).unwrap();
        let kept = |r: &super::Report| r.phases.iter().any(|p| p.0.starts_with("unchanged"));
        let defs = || std::fs::read_to_string(out.join("defs.csv")).unwrap();
        let text = program("defs");
        assert!(!kept(&run(&text)), "the first run derives");
        assert_eq!(defs(), "f\n");
        std::fs::remove_file(out.join("defs.csv")).unwrap();
        let again = run(&text);
        assert!(kept(&again), "unchanged: kept");
        assert_eq!((defs().as_str(), again.rows.get("defs").copied()), ("f\n", Some(1)), "the kept outputs written again");
        // A document's edit runs it again, and that run is kept in turn.
        std::fs::write(src.join("a.py"), "def g():\n    return 1\n").unwrap();
        assert!(!kept(&run(&text)), "an edited document");
        assert_eq!(defs(), "g\n");
        assert!(kept(&run(&text)));
        std::fs::write(src.join("b.py"), "def h():\n    pass\n").unwrap();
        assert!(!kept(&run(&text)), "a new document");
        assert_eq!(defs(), "g\nh\n");
        std::fs::write(src.join("rerail.env"), "mode=strict\n").unwrap();
        run(&text);
        assert!(kept(&run(&text)), "settings reach only a program that declares `env`");
        let with_env = |name: &str| program(name).replace(".output", ".decl env(k: symbol, v: symbol)\n.output");
        assert!(!kept(&run(&with_env("defs"))));
        assert!(kept(&run(&with_env("defs"))));
        std::fs::write(src.join("rerail.env"), "mode=lax\n").unwrap();
        assert!(!kept(&run(&with_env("defs"))), "the root's settings, for a program that declares `env`");
        assert!(!kept(&run(&program("names"))), "another text");
        // Each library file a text includes is in the record, with its digest (a flattened text includes none).
        let rules = super::rule_files(&["#include \"core.rr\"\n.decl x(s: symbol)\n".to_owned()]).unwrap();
        assert!(rules.iter().any(|r| r.1.ends_with("core.rr") && !r.2.is_empty()) && rules.iter().any(|r| r.1.ends_with("components.rr")), "{rules:?}");
        // Off: derived every time.
        let off = serde_json::json!({});
        let r = super::super::run_with_options(&[&text], &src, None, &out, "program", Binding::Cpu, 1 << 18, &Default::default(), &off).unwrap();
        assert!(!kept(&r), "not asked");
        super::super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn outputs_and_records_round_trip() {
        let files = vec![("a.csv".to_owned(), b"1\t2\n".to_vec()), ("b.csv".to_owned(), Vec::new())];
        assert_eq!(super::decode_outputs(&super::encode_outputs(&files)).unwrap(), files);
        let c = vec![("doc".to_owned(), "a.py".to_owned(), "ab".to_owned()), ("rule".to_owned(), "/x/core.rr".to_owned(), "cd".to_owned())];
        assert_eq!(super::decode_consulted(&super::encode_consulted(&c)), c);
    }
}
