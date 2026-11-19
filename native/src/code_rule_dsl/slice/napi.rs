//! The slice's napi entry point (06 S6, after S7's gate): a host opens a store, lowers a pack's
//! policies once, and audits or applies a scope's directory snapshot, receiving the finding keys
//! and coverage as JSON.

use super::{Rules, Run, apply, audit};
use crate::code_rule_dsl::CodeRuleDslRuntime;
use crate::code_rule_dsl::facts::observe::roots::{buffered_root, git_root};
use crate::code_rule_dsl::findings::contract::Coverage;
use crate::code_rule_dsl::transducer::packed::ExecutionBackend;
use crate::content_store::{ContentStore, HashKey};
use napi::{Error, Result};
use napi_derive::napi;
use serde_json::json;
use std::path::Path;

/// A policy document handed to the slice: its name (the file stem), source and format.
#[napi(object)]
pub struct SlicePolicy {
    pub name: String,
    pub source: String,
    pub format: String,
}

/// An editor buffer: the unsaved source of the file at `path` (relative to the directory, `/`
/// separators), which may not exist on disk yet.
#[napi(object)]
pub struct SliceBuffer {
    pub path: String,
    pub source: String,
}

/// The first slice over one store: the lowered rules and the SQL owner live between runs, so a
/// run after the first is warm.
#[napi]
pub struct CodeRuleSlice {
    store: ContentStore,
    rules: Rules,
}

fn failed(error: impl std::fmt::Debug) -> Error {
    Error::from_reason(format!("{error:?}"))
}

#[napi]
impl CodeRuleSlice {
    /// Opens the store at `store_path` and lowers `policies` for `backend` ("cpu" or "metal").
    #[napi(constructor)]
    pub fn new(store_path: String, policies: Vec<SlicePolicy>, capabilities_json: String, backend: String) -> Result<Self> {
        let backend = match backend.as_str() {
            // Interpreted first, the compiled binding swapped in once built (list step 16).
            "cpu" => ExecutionBackend::CpuBackground,
            #[cfg(target_os = "macos")]
            "metal" => ExecutionBackend::MetalBackground,
            other => return Err(Error::from_reason(format!("unknown slice backend `{other}`"))),
        };
        let compiled = policies
            .into_iter()
            .map(|policy| Ok((policy.name, CodeRuleDslRuntime::compile(policy.source, policy.format, capabilities_json.clone())?)))
            .collect::<Result<Vec<_>>>()?;
        let store = ContentStore::open(Path::new(&store_path)).map_err(failed)?;
        let rules = Rules::lower(&compiled, backend).map_err(failed)?;
        Ok(Self { store, rules })
    }

    /// How many clauses lowered.
    #[napi]
    pub fn clauses(&self) -> u32 {
        self.rules.clauses() as u32
    }

    /// The lowered rules as `rir` (list step 18): programs and label tables, with `fused` the
    /// packs and groups, with `setplan` the set plan.
    #[napi]
    pub fn dump(&self, fused: Option<bool>, setplan: Option<bool>) -> Result<String> {
        self.rules.dump(fused.unwrap_or(false), setplan.unwrap_or(false)).map_err(failed)
    }

    /// A cold audit of `scope` at `directory`, as `observer` sees it: `"directory"` (the default,
    /// the working tree's snapshot, with `buffers` over it) or `"head"` (the `HEAD` tree of its
    /// Git checkout).
    #[napi]
    pub fn audit(&mut self, scope: String, directory: String, observer: Option<String>, buffers: Option<Vec<SliceBuffer>>) -> Result<String> {
        let root = self.root(&directory, observer.as_deref(), buffers)?;
        let run = audit(&self.store, &scope, root, &mut self.rules).map_err(failed)?;
        Ok(report(&run))
    }

    /// `scope` after edits: `directory` as `observer` sees it (as `audit`'s), evaluating only what
    /// changed since the last run.
    #[napi]
    pub fn apply(&mut self, scope: String, directory: String, observer: Option<String>, buffers: Option<Vec<SliceBuffer>>) -> Result<String> {
        let root = self.root(&directory, observer.as_deref(), buffers)?;
        let run = apply(&self.store, &scope, root, &mut self.rules).map_err(failed)?;
        Ok(report(&run))
    }
}

/// `policies` lowered and dumped as `rir` (list step 18, `rerail engine dump --rule <file>
/// [--clause <id>] [--fused] [--setplan]`), without a store.
#[napi]
#[cfg_attr(test, allow(dead_code))]
pub fn dump_rir(policies: Vec<SlicePolicy>, capabilities_json: String, clause: Option<String>, fused: Option<bool>, setplan: Option<bool>) -> Result<String> {
    let policies: Vec<_> = policies.into_iter().map(|policy| (policy.name, policy.source, policy.format)).collect();
    crate::code_rule_dsl::tools::dump(&policies, &capabilities_json, clause.as_deref(), fused.unwrap_or(false), setplan.unwrap_or(false))
        .map_err(Error::from_reason)
}

/// `rir` assembled (`rerail-engine asm`): every `program` block of `text` through the verifier,
/// returned as its dump, or the first refusal.
#[napi]
#[cfg_attr(test, allow(dead_code))]
pub fn assemble_rir(text: String) -> Result<String> {
    crate::code_rule_dsl::tools::assemble(&text).map_err(Error::from_reason)
}

impl CodeRuleSlice {
    /// The root tree `observer` sees at `directory`, stored with its sources.
    fn root(&self, directory: &str, observer: Option<&str>, buffers: Option<Vec<SliceBuffer>>) -> Result<HashKey> {
        let directory = Path::new(directory);
        let buffers = buffers.unwrap_or_default().into_iter().map(|buffer| (buffer.path, buffer.source.into_bytes())).collect::<std::collections::BTreeMap<_, _>>();
        match observer.unwrap_or("directory") {
            "directory" => buffered_root(&self.store, directory, &buffers),
            "head" if buffers.is_empty() => git_root(&self.store, directory),
            "head" => return Err(Error::from_reason("editor buffers lie over a directory snapshot, not over `HEAD`")),
            other => return Err(Error::from_reason(format!("unknown slice observer `{other}`"))),
        }
        .map_err(failed)
    }
}

/// A run as JSON: finding keys and coverage by path, the documents evaluated and those whose rows
/// came from the content memo, the derivations re-run, and the wave's and layer's families that changed.
pub(super) fn report(run: &Run) -> String {
    let path = |doc: &u32| run.paths.get(doc).cloned().unwrap_or_default();
    let findings = run
        .findings
        .keys
        .iter()
        .map(|key| json!({"rule": key.rule, "code": key.code, "path": path(&key.doc), "line": key.line}))
        .collect::<Vec<_>>();
    let coverage = run
        .findings
        .coverage
        .iter()
        .map(|((rule, doc), coverage)| match coverage {
            Coverage::Complete => json!({"rule": rule, "path": path(doc), "coverage": "complete"}),
            Coverage::Unknown(reason) => json!({"rule": rule, "path": path(doc), "coverage": "unknown", "reason": reason}),
        })
        .collect::<Vec<_>>();
    json!({
        "findings": findings,
        "coverage": coverage,
        "evaluated": run.evaluated,
        "reused": run.reused,
        "rerun": run.ingested.push.work.rerun,
        "changed": {"matches": run.matches, "findings": run.changed},
    })
    .to_string()
}
