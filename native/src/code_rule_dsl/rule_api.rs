//! The compiled code rule and its editor as the host holds them (napi classes): what W evaluates for a snapshot or
//! an editor's edits.

use super::eval::{DerivedPackages, evaluate_code_rule, evaluation_json, parse_snapshot, violation_ids};
use super::json::{invalid, json_error, required_str, required_u64};
use super::model::{CompiledCodeRule, Snapshot};
use napi::Result;
use napi_derive::napi;
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::Arc;

/// What audits the candidate compiler over a compiled rule (`prefilter::candidate_frontier_audit`, the engine's),
/// registered at load.
pub static FRONTIER_AUDIT: std::sync::OnceLock<fn(&CompiledCodeRule) -> Value> = std::sync::OnceLock::new();

/// The capabilities this build grants a rule (`capabilities.json`): what the CLI and the engine's glue compile with.
pub const CAPABILITIES: &str = include_str!("capabilities.json");

/// A rule's document compiled against `capabilities_json`, for the host and the engine's glue (`compile_rule`).
pub fn compile(source: &str, format: &str, capabilities_json: &str) -> Result<CompiledCodeRuleDsl> {
    compile_rule(source, format, capabilities_json).map(|rule| CompiledCodeRuleDsl { inner: Arc::new(rule) })
}

/// A rule's document compiled against `capabilities_json`: parsed as `format`, canonicalized, digested and compiled.
/// The host's entries (`CodeRuleDslRuntime::compile` on its thread, `compile_async` on a pool thread) compile
/// through here.
pub fn compile_rule(source: &str, format: &str, capabilities_json: &str) -> Result<CompiledCodeRule> {
    use super::compile::{compile_document, parse_capabilities, parse_document};
    use super::json::{canonicalize_value, hex_digest};
    let capabilities = parse_capabilities(capabilities_json)?;
    let document = parse_document(source, format)?;
    let canonical_value = canonicalize_value(document);
    let canonical = serde_json::to_string(&canonical_value).map_err(json_error)?;
    let digest = format!("sha256:{}", hex_digest(canonical.as_bytes()));
    // The regexes only validation took, or a rejected document's, go as this ends (`regexes`).
    let _compiling = super::regexes::compiling();
    let (id, rule_mode, scope, clauses, required, report_count_gt) = compile_document(&canonical_value, &capabilities)?;
    Ok(CompiledCodeRule {
        id,
        report_count_gt,
        rule_mode,
        canonical,
        digest,
        required_capabilities: serde_json::to_string(&required).map_err(json_error)?,
        clauses,
        scope,
        capabilities,
    })
}

#[napi]
pub struct CompiledCodeRuleDsl {
    #[napi(skip)] pub inner: Arc<CompiledCodeRule>,
}

#[napi]
impl CompiledCodeRuleDsl {
    /// Audit the normal candidate compiler over fixed permutations. This executes no candidate SQL.
    #[napi]
    pub fn candidate_frontier_audit(&self) -> String {
        FRONTIER_AUDIT.get().map_or_else(|| "null".to_owned(), |audit| audit(&self.inner).to_string())
    }

    #[napi]
    pub fn canonical_json(&self) -> String {
        self.inner.canonical.clone()
    }

    /// Whether a repository-relative path is inside this code rule's scope: the same
    /// matcher evaluation uses, so a host can hand the code rule only the files it will
    /// read and be sure the two never disagree.
    #[napi(catch_unwind)]
    pub fn in_scope(&self, path: String) -> bool {
        self.inner.scope.matches(&path)
    }

    /// Whether the code rule reads text: a clause holds a regex or file query, so files no
    /// code-rule language lists are read for it, as `text`.
    #[napi]
    pub fn reads_text(&self) -> bool {
        self.inner.reads_text()
    }

    /// Whether a finding of this code rule in one file can depend on another file's text: a host
    /// may then check a workspace's edits in the order they came only, where it may check those of
    /// a code rule that reads none in any order (`CompiledCodeRule::reads_other_files`).
    #[napi]
    pub fn reads_other_files(&self) -> bool {
        self.inner.reads_other_files()
    }

    #[napi]
    pub fn digest(&self) -> String {
        self.inner.digest.clone()
    }

    #[napi]
    pub fn required_capabilities_json(&self) -> String {
        self.inner.required_capabilities.clone()
    }

    /// Whether the code rule reads `file.package.serverGraph`: a host then sends each file of a
    /// package naming `next` its membership (`serverGraph`).
    #[napi]
    pub fn reads_server_graph(&self) -> bool {
        self.inner.reads_server_graph()
    }

    #[napi(catch_unwind)]
    pub fn evaluate(&self, snapshot_json: String, mode: String) -> Result<String> {
        let (snapshot, _) = parse_snapshot(&snapshot_json, self.inner.reads_server_graph())?;
        let evaluation = evaluate_code_rule(&self.inner, &snapshot, &mode)?;
        serde_json::to_string(&evaluation_json(evaluation, None, &[], &[])).map_err(json_error)
    }

    #[napi(catch_unwind)]
    pub fn open_editor(&self, snapshot_json: String) -> Result<CodeRuleDslEditor> {
        let (snapshot, derived) = parse_snapshot(&snapshot_json, self.inner.reads_server_graph())?;
        Ok(CodeRuleDslEditor {
            code_rule: Arc::clone(&self.inner),
            snapshot,
            derived,
            previous_ids: BTreeSet::new(),
        })
    }
}

#[napi]
pub struct CodeRuleDslEditor {
    pub(crate) code_rule: Arc<CompiledCodeRule>,
    #[napi(skip)] pub snapshot: Snapshot,
    /// The `package` the snapshot's `package.json` entries give the files the host sent none for:
    /// an edit gives them theirs again where it changes what they read (`DerivedPackages::edited`).
    derived: DerivedPackages,
    previous_ids: BTreeSet<String>,
}

#[napi]
impl CodeRuleDslEditor {
    #[napi(catch_unwind)]
    pub fn evaluate(&mut self, mode: String) -> Result<String> {
        let evaluation = evaluate_code_rule(&self.code_rule, &self.snapshot, &mode)?;
        let current = violation_ids(&evaluation.violations);
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

    #[napi(catch_unwind)]
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
        let index = self
            .snapshot
            .files
            .iter()
            .position(|file| file.path == path)
            .ok_or_else(|| invalid(format!("edit path `{path}` is not in the snapshot")))?;
        let file = &mut self.snapshot.files[index];
        if start > end
            || end > file.source.len()
            || !file.source.is_char_boundary(start)
            || !file.source.is_char_boundary(end)
        {
            return Err(invalid("edit byte range is invalid"));
        }
        // The edited document's text, made anew: a source is never written where it lies.
        let mut text = file.source.as_str().to_owned();
        text.replace_range(start..end, new_text);
        file.source = Arc::new(text.into());
        self.derived.edited(&mut self.snapshot, index);
        self.snapshot.revision = revision;
        self.evaluate(mode)
    }
}
