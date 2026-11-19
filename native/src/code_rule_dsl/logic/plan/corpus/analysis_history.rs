//! The committed scope records that analysis.rr projects into public relations.

use super::{front::{Program, Value}, memo, Result};
use std::collections::BTreeMap;

type FactRows = BTreeMap<String, Vec<Vec<Value>>>;

pub(super) const INPUT: &str = "analysis_run_record";
pub(super) const REQUEST: &str = "analysis_history_lookup";
pub(super) const ANSWER: &str = "analysis_history_of";

const SCOPE: &str = "analysis_scope_record";

fn run_fields(id: i64, record: crate::content_store::analysis_scope::CompletedRun) -> [String; 10] {
    let kind = if record.scope.starts_with("cwd:") { "directory" } else { "named" };
    [
        format!("scope:{id}:{}", record.scope), kind.to_owned(), record.value,
        format!("run:{id}:{}", record.acquisition), record.scan_id, record.directory.unwrap_or_default(),
        record.repository.unwrap_or_default(), record.tree.unwrap_or_default(), record.commit.unwrap_or_default(),
        record.snapshot.map(|key| format!("snapshot:{id}:{key}")).unwrap_or_default(),
    ]
}

pub(super) fn requested(kind: &str, key: &str) -> Result<(Vec<[String; 10]>, String)> {
    use sha2::{Digest, Sha256};
    let store = super::tape_cache()?.ok_or_else(|| super::Error::Plan("analysis run history requires a readable content store".into()))?;
    let id = store.identity();
    let scope = match kind {
        "named" => crate::content_store::analysis_scope::named(key),
        "scope" => key.strip_prefix(&format!("scope:{id}:")).map(str::to_owned),
        _ => return Err(super::Error::Plan(format!("analysis_history_request expects named or scope, got {kind:?}"))),
    };
    let mut rows = Vec::new();
    if let Some(scope) = scope {
        for record in store.completed_runs_in_scope(&scope)? {
            let [scope, _, _, run, scan, dir, repo, tree, commit, snapshot] = run_fields(id, record);
            rows.push([kind.to_owned(), key.to_owned(), scope, run, scan, dir, repo, tree, commit, snapshot]);
        }
    }
    crate::code_rule_dsl::logic::profile::decision("scope history lookup", || format!("{kind} {key:?}: {} run records", rows.len()));
    // The store returns rows in acquisition and snapshot order.
    let mut hash = Sha256::new();
    for field in rows.iter().flatten() {
        hash.update((field.len() as u64).to_le_bytes());
        hash.update(field.as_bytes());
    }
    Ok((rows, crate::content_store::observe::hex(&hash.finalize().into())))
}

/// A retained answer may have a different row order. Compare each stored row with its current fields.
pub(super) fn kept(kind: &str, key: &str, previous: &[&Vec<i64>], text: impl Fn(i64) -> Option<String>) -> Option<String> {
    let (rows, digest) = requested(kind, key).ok()?;
    if rows.len() != previous.len() { return None; }
    let mut current: std::collections::HashSet<_> = rows.into_iter().collect();
    for row in previous {
        let fields: [String; 10] = row.iter().map(|v| text(*v)).collect::<Option<Vec<_>>>()?.try_into().ok()?;
        if !current.remove(&fields) { return None; }
    }
    crate::code_rule_dsl::logic::profile::decision("scope history reuse", || format!("{kind} {key:?}: retained {} run records", previous.len()));
    Some(digest)
}

pub(super) fn is_input(name: &str) -> bool {
    matches!(name, INPUT | SCOPE)
}

#[derive(Clone, Copy)]
pub(super) enum Read { Runs, Scopes, Both }

impl Read {
    pub(super) fn needed(program: &Program) -> Option<Self> {
        if !program.inputs.iter().any(|name| is_input(name)) { return None; }
        let needed = super::world::needed(program);
        let reads = |relation: &str| needed.contains(relation) && program.inputs.iter().any(|name| name == relation);
        match (reads(SCOPE), reads(INPUT)) {
            (true, true) => Some(Self::Both),
            (true, false) => Some(Self::Scopes),
            (false, true) => Some(Self::Runs),
            (false, false) => None,
        }
    }

    pub(super) fn key(self) -> &'static str {
        match self { Self::Runs => "", Self::Scopes => "scopes", Self::Both => "scopes+runs" }
    }

    pub(super) fn from_key(key: &str) -> Option<Self> {
        match key { "" => Some(Self::Runs), "scopes" => Some(Self::Scopes), "scopes+runs" => Some(Self::Both), _ => None }
    }

    pub(super) fn records(self) -> Result<(FactRows, String)> {
        let scopes = matches!(self, Self::Scopes | Self::Both);
        let runs = matches!(self, Self::Runs | Self::Both);
        let store = super::tape_cache()?.ok_or_else(|| super::Error::Plan("analysis run history requires a readable content store".to_owned()))?;
        let id = store.identity();
        let records = store.analysis_records(scopes, runs)?;
        let mut inputs = BTreeMap::new();
        if scopes {
            let rows = records.scopes.into_iter().map(|(scope, value)| {
                let kind = if scope.starts_with("cwd:") { "directory" } else { "named" };
                [format!("scope:{id}:{scope}"), kind.to_owned(), value].into_iter().map(|v| Value::Str(v.into())).collect()
            }).collect();
            inputs.insert(SCOPE.to_owned(), rows);
        }
        if runs {
            let rows = records.runs.into_iter().map(|record| run_fields(id, record).into_iter().map(|v| Value::Str(v.into())).collect()).collect();
            inputs.insert(INPUT.to_owned(), rows);
        }
        let digest = memo::with_rows([0; 32], inputs.iter().map(|(name, rows)| (name.as_str(), rows)));
        Ok((inputs, crate::content_store::observe::hex(&digest)))
    }
}
