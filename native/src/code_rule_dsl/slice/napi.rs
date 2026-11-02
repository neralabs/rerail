//! The slice's napi entry point (06 S6, after S7's gate): a host opens a store, lowers a pack's
//! policies once, and audits or applies a scope's directory snapshot, receiving the finding keys
//! and coverage as JSON.

use super::{Rules, Run, apply, audit};
use crate::code_rule_dsl::CodeRuleDslRuntime;
use crate::code_rule_dsl::facts::observe::roots::{directory_root, git_root};
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
            "cpu" => ExecutionBackend::CpuJit,
            #[cfg(target_os = "macos")]
            "metal" => ExecutionBackend::MetalJit,
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

    /// A cold audit of `scope` at `directory`, as `observer` sees it: `"directory"` (the default,
    /// the working tree's snapshot) or `"head"` (the `HEAD` tree of its Git checkout).
    #[napi]
    pub fn audit(&mut self, scope: String, directory: String, observer: Option<String>) -> Result<String> {
        let root = self.root(&directory, observer.as_deref())?;
        let run = audit(&self.store, &scope, root, &mut self.rules).map_err(failed)?;
        Ok(report(&run))
    }

    /// `scope` after edits: `directory` as `observer` sees it (as `audit`'s), evaluating only what
    /// changed since the last run.
    #[napi]
    pub fn apply(&mut self, scope: String, directory: String, observer: Option<String>) -> Result<String> {
        let root = self.root(&directory, observer.as_deref())?;
        let run = apply(&self.store, &scope, root, &mut self.rules).map_err(failed)?;
        Ok(report(&run))
    }
}

impl CodeRuleSlice {
    /// The root tree `observer` sees at `directory`, stored with its sources.
    fn root(&self, directory: &str, observer: Option<&str>) -> Result<HashKey> {
        let directory = Path::new(directory);
        match observer.unwrap_or("directory") {
            "directory" => directory_root(&self.store, directory),
            "head" => git_root(&self.store, directory),
            other => return Err(Error::from_reason(format!("unknown slice observer `{other}`"))),
        }
        .map_err(failed)
    }
}

/// A run as JSON: finding keys and coverage by path, the documents evaluated, the derivations
/// re-run, and the wave's and layer's families that changed.
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
        "rerun": run.ingested.push.work.rerun,
        "changed": {"matches": run.matches, "findings": run.changed},
    })
    .to_string()
}
