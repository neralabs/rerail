//! The committed scope records that analysis.rr projects into public relations.

use super::{front::{Program, Value}, memo, Result};
use std::collections::BTreeMap;

type FactRows = BTreeMap<String, Vec<Vec<Value>>>;

pub(super) const INPUT: &str = "analysis_run_record";

const SCOPE: &str = "analysis_scope_record";

pub(super) fn is_input(name: &str) -> bool {
    matches!(name, INPUT | SCOPE)
}

#[derive(Clone, Copy)]
pub(super) enum Read { Runs, Scopes, Both }

impl Read {
    pub(super) fn needed(program: &Program) -> Option<Self> {
        if !program.inputs.iter().any(|name| is_input(name)) { return None; }
        let needed = super::front::needed(program, program.outputs.clone());
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
            let mut rows = Vec::new();
            for record in records.runs {
                let kind = if record.scope.starts_with("cwd:") { "directory" } else { "named" };
                let values = [
                    format!("scope:{id}:{}", record.scope), kind.to_owned(), record.value,
                    format!("run:{id}:{}", record.acquisition), record.scan_id, record.directory.unwrap_or_default(),
                    record.repository.unwrap_or_default(), record.tree.unwrap_or_default(), record.commit.unwrap_or_default(),
                    record.snapshot.map(|key| format!("snapshot:{id}:{key}")).unwrap_or_default(),
                ];
                rows.push(values.into_iter().map(|v| Value::Str(v.into())).collect());
            }
            inputs.insert(INPUT.to_owned(), rows);
        }
        let digest = memo::with_rows([0; 32], inputs.iter().map(|(name, rows)| (name.as_str(), rows)));
        Ok((inputs, crate::content_store::observe::hex(&digest)))
    }
}
