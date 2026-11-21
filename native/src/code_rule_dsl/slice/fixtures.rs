//! The first slice's tests' fixtures (06 S6): the Python pack's policies, a cold audit to compare a warm run with,
//! and a run's keys by path. Boot's `slice` tests and the engine's napi test read them.

use super::*;
use crate::code_rule_dsl::facts::observe::roots::directory_root;
use crate::code_rule_dsl::findings::contract::Coverage;
use crate::code_rule_dsl::rule_api::CAPABILITIES;
use std::path::{Path, PathBuf};

pub const WS: &str = "/slice";

pub fn pack() -> PathBuf {
    crate::source_tree::native().join("../packs/python")
}

pub fn temp(what: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!("rerail-s6-{what}-{}-{nonce}", std::process::id()))
}

pub fn files(directory: &Path, extension: &str, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files(&path, extension, found);
        } else if path.extension().is_some_and(|found| found == extension) {
            found.push(path);
        }
    }
}

/// The Python pack's compiled policies, by file stem.
pub fn policies() -> Vec<(String, CompiledCodeRuleDsl)> {
    let mut documents = Vec::new();
    files(&pack().join("rules"), "json", &mut documents);
    files(&pack().join("candidates/rules"), "json", &mut documents);
    documents.sort();
    documents
        .into_iter()
        .filter_map(|path| {
            let text = std::fs::read_to_string(&path).ok()?;
            // A policy that does not compile is left out, and named.
            let compiled = crate::code_rule_dsl::rule_api::compile(&text, "json", CAPABILITIES)
                .map_err(|error| eprintln!("SKIP {}: {}", path.display(), error.reason))
                .ok()?;
            Some((path.file_stem().unwrap().to_string_lossy().into_owned(), compiled))
        })
        .collect()
}

/// The keys of a run, by path rather than document key, and its coverage likewise.
pub type Keys = BTreeSet<(String, String, String, usize)>;
pub fn keys(run: &Run) -> Keys {
    run.findings.keys.iter().map(|k| (k.rule.clone(), k.code.clone(), run.paths[&k.doc].clone(), k.line)).collect()
}
pub fn coverage(run: &Run) -> BTreeMap<(String, String), Coverage> {
    run.findings.coverage.iter().map(|((rule, doc), c)| ((rule.clone(), run.paths[doc].clone()), c.clone())).collect()
}

/// A cold audit of `dir` in a fresh store: what a warm run over the same snapshot must find.
pub fn cold(dir: &Path, policies: &[(String, CompiledCodeRuleDsl)]) -> Run {
    let store = ContentStore::open(&temp("cold").with_extension("sqlite")).unwrap();
    let mut rules = Rules::lower(policies, ExecutionBackend::CpuJit).unwrap();
    let root = directory_root(&store, dir).unwrap();
    audit(&store, WS, root, &mut rules).unwrap()
}

pub fn write(dir: &Path, path: &str, source: &str) {
    std::fs::write(dir.join(path), source).unwrap();
}
