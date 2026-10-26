//! Per-file facts as derivations (docs/final-shape/03-architecture.md §2, §14; 05 Phase 1, the
//! facts memo row): one derivation per (scope, path, plan set, fact schema). It reads the path's
//! name, so a blob edit dirties exactly the files it touched, and its output digest is the memo's
//! run digest, so an edit whose facts are unchanged rebinds nothing and its readers never run.
//! The memo stays keyed by content: a content seen before (any path, any scope) is a lookup.

use super::observe::path_name;
use crate::content_store::{
    ABSENT, ContentStore, DerivationInput, DerivationRecord, FactsCompletion, FactsMemoKey, FactsPlanSet,
    FactsResult, SourceId, StoreError, StoreResult,
};

/// One file's facts under one plan set.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FileFacts<'a> {
    pub scope: &'a str,
    pub path: &'a str,
    pub set: &'a FactsPlanSet,
    pub fact_schema: u32,
}

impl FileFacts<'_> {
    /// The derivation's output name.
    pub(crate) fn output(&self) -> String {
        let set = self.set.digest.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
        format!("{}#facts:{}:{set}:{}", self.scope, self.fact_schema, self.path)
    }
}

/// What evaluating one file did: whether the memo answered, and the derivation as run.
#[derive(Clone, Debug)]
pub(crate) struct Evaluated {
    pub hit: bool,
    pub record: DerivationRecord,
}

/// The facts of `file`, whose content is `source`: from the memo when it knows every clause of the
/// set, otherwise from `run` (the clauses' results and empty completions), which is memoised.
pub(crate) fn evaluate(
    store: &ContentStore,
    file: FileFacts<'_>,
    source: SourceId,
    run: impl FnOnce() -> StoreResult<(Vec<FactsResult>, Vec<FactsCompletion>)>,
) -> StoreResult<Evaluated> {
    let key = FactsMemoKey { source, fact_schema: file.fact_schema, path: file.path };
    let (hit, digest) = match store.facts_memo(key)?.run_digest(file.set) {
        Some(digest) => (true, digest),
        None => {
            let (results, completions) = run()?;
            let digest = store.put_facts_memo(key, file.set, &results, &completions)?.ok_or_else(|| {
                StoreError::Invalid("a facts run did not cover its plan set".to_owned())
            })?;
            (false, digest)
        }
    };
    let name = path_name(file.scope, file.path);
    let read = store.binding(&name)?.unwrap_or(ABSENT);
    let record = DerivationRecord {
        output: file.output(),
        producer: file.set.digest,
        inputs: vec![DerivationInput { name: Some(name), digest: read }],
        digest,
        status: 0,
        cost: i64::from(!hit),
    };
    Ok(Evaluated { hit, record })
}

#[cfg(test)]
mod tests;
