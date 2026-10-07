mod blade;
mod budget;
mod calls;
mod compile;
mod eval;
mod findings;
mod json;
mod language;
mod lines;
mod literal;
mod matcher;
mod model;
mod module_path;
#[cfg(feature = "policy-profile")]
mod profile;
mod query;
mod scope;
mod soundness;
mod suppress;
mod tape;
#[cfg(test)]
mod test_support;
mod transducer;
mod value;
mod workspace;

/// The conformance fixtures checkout `native/build.rs` found: `RERAIL_FIXTURES_DIR`, or the
/// repository's `fixtures/` submodule.
#[cfg(test)]
macro_rules! fixtures_dir {
    () => {
        env!(
            "RERAIL_FIXTURES_DIR",
            "Rerail fixtures not found: run `git submodule update --init`, or set RERAIL_FIXTURES_DIR to a rerail-fixtures checkout"
        )
    };
}

#[cfg(test)]
pub(crate) use fixtures_dir;

#[cfg(test)]
macro_rules! oracle {
    ($path:literal) => {
        include_str!(concat!(crate::policy_dsl::fixtures_dir!(), "/", $path))
    };
}

#[cfg(test)]
pub(crate) use oracle;

use compile::{compile_document, parse_capabilities, parse_document};
use eval::{evaluate_policy, evaluation_json, finding_ids, parse_snapshot};
use json::{canonicalize_value, hex_digest, invalid, json_error, required_str, required_u64};
use model::{CompiledPolicy, Snapshot};
use napi::Result;
use napi_derive::napi;
use query::PolicyStatements;
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::Arc;

#[napi]
pub struct PolicyDslRuntime;

#[napi]
impl PolicyDslRuntime {
    #[napi]
    pub fn compile(
        source: String,
        format: String,
        capabilities_json: String,
    ) -> Result<CompiledPolicyDsl> {
        let capabilities = parse_capabilities(&capabilities_json)?;
        let document = parse_document(&source, &format)?;
        let canonical_value = canonicalize_value(document);
        let canonical = serde_json::to_string(&canonical_value).map_err(json_error)?;
        let digest = format!("sha256:{}", hex_digest(canonical.as_bytes()));
        let (id, scope, checks, required, statements) =
            compile_document(&canonical_value, &capabilities)?;
        Ok(CompiledPolicyDsl {
            inner: Arc::new(CompiledPolicy {
                id,
                canonical,
                digest,
                required_capabilities: serde_json::to_string(&required).map_err(json_error)?,
                checks,
                scope,
                capabilities,
            }),
            statements: Arc::new(statements),
        })
    }
}

/// The module path a Python file's `module` carries, from the package markers on disk: a dotted
/// string, a list of names when one keeps a dot, or null outside a package. Hosts and the native
/// repository ingestion paths share this one derivation.
#[napi]
pub fn python_module_path(absolute: String) -> Option<napi::Either<String, Vec<String>>> {
    match model::python_module_path_on_disk(std::path::Path::new(&absolute))? {
        Value::String(module) => Some(napi::Either::A(module)),
        Value::Array(names) => Some(napi::Either::B(
            names
                .into_iter()
                .filter_map(|name| name.as_str().map(str::to_owned))
                .collect(),
        )),
        _ => None,
    }
}

#[napi]
pub struct CompiledPolicyDsl {
    inner: Arc<CompiledPolicy>,
    /// The statements `inner`'s grammar matchers run as (`GrammarMatcher::statement`).
    statements: Arc<PolicyStatements>,
}

#[napi]
impl CompiledPolicyDsl {
    #[napi]
    pub fn canonical_json(&self) -> String {
        self.inner.canonical.clone()
    }

    /// Whether a repository-relative path is inside this policy's scope: the same
    /// matcher evaluation uses, so a host can hand the policy only the files it will
    /// read and be sure the two never disagree.
    #[napi]
    pub fn in_scope(&self, path: String) -> bool {
        self.inner.scope.matches(&path)
    }

    #[napi]
    pub fn digest(&self) -> String {
        self.inner.digest.clone()
    }

    #[napi]
    pub fn required_capabilities_json(&self) -> String {
        self.inner.required_capabilities.clone()
    }

    #[napi]
    pub fn evaluate(&self, snapshot_json: String, mode: String) -> Result<String> {
        let snapshot = parse_snapshot(&snapshot_json)?;
        let evaluation = evaluate_policy(&self.inner, &self.statements, &snapshot, &mode)?;
        serde_json::to_string(&evaluation_json(evaluation, None, &[], &[])).map_err(json_error)
    }

    #[napi]
    pub fn open_session(&self, snapshot_json: String) -> Result<PolicyDslSession> {
        Ok(PolicyDslSession {
            policy: Arc::clone(&self.inner),
            statements: Arc::clone(&self.statements),
            snapshot: parse_snapshot(&snapshot_json)?,
            previous_ids: BTreeSet::new(),
        })
    }
}

#[napi]
pub struct PolicyDslSession {
    policy: Arc<CompiledPolicy>,
    statements: Arc<PolicyStatements>,
    snapshot: Snapshot,
    previous_ids: BTreeSet<String>,
}

#[napi]
impl PolicyDslSession {
    #[napi]
    pub fn evaluate(&mut self, mode: String) -> Result<String> {
        let evaluation = evaluate_policy(&self.policy, &self.statements, &self.snapshot, &mode)?;
        let current = finding_ids(&evaluation.findings);
        let added = current
            .difference(&self.previous_ids)
            .cloned()
            .collect::<Vec<_>>();
        let retracted = self
            .previous_ids
            .difference(&current)
            .cloned()
            .collect::<Vec<_>>();
        self.previous_ids = current;
        serde_json::to_string(&evaluation_json(
            evaluation,
            Some(self.snapshot.revision),
            &added,
            &retracted,
        ))
        .map_err(json_error)
    }

    #[napi]
    pub fn apply_edit(&mut self, edit_json: String, mode: String) -> Result<String> {
        let edit: Value = serde_json::from_str(&edit_json).map_err(json_error)?;
        let revision = required_u64(&edit, "revision", "edit")? as u32;
        let expected = self.snapshot.revision + 1;
        if revision != expected {
            return Err(invalid(format!(
                "out-of-order edit revision {revision}; expected {expected}"
            )));
        }
        let path = required_str(&edit, "path", "edit")?;
        let start = required_u64(&edit, "startByte", "edit")? as usize;
        let end = required_u64(&edit, "oldEndByte", "edit")? as usize;
        let new_text = required_str(&edit, "newText", "edit")?;
        let file = self
            .snapshot
            .files
            .iter_mut()
            .find(|file| file.path == path)
            .ok_or_else(|| invalid(format!("edit path `{path}` is not in the snapshot")))?;
        if start > end
            || end > file.source.len()
            || !file.source.is_char_boundary(start)
            || !file.source.is_char_boundary(end)
        {
            return Err(invalid("edit byte range is invalid"));
        }
        Arc::make_mut(&mut file.source).replace_range(start..end, new_text);
        self.snapshot.revision = revision;
        self.evaluate(mode)
    }
}
