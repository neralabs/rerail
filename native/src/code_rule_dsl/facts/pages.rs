//! Per-directory result pages as derivations (C16, C17; docs/final-shape/03-architecture.md §15):
//! one derivation per (scope, prefix, plan). It reads the directory's tree name, so only an edit
//! under the directory dirties it, and outputs the digest of the directory's pages. The pages are
//! keyed by tree hash, not by scope (`content_store::tree_pages`), so a subtree that another
//! workspace already evaluated is a lookup: Tempo's reuse across workspaces, as a cutoff.

use super::Graph;
use super::memo::Evaluated;
use super::observe::tree_name;
use crate::content_store::{ABSENT, DerivationInput, DerivationRecord, StoreError, StoreResult, TreePage, TreePageKey};

/// One directory's results under one plan.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DirectoryPages<'a> {
    pub scope: &'a str,
    pub prefix: &'a str,
    pub plan: [u8; 32],
    /// The context chain the results read (package markers above the directory), as one digest.
    pub context: [u8; 32],
}

impl DirectoryPages<'_> {
    pub(crate) fn output(&self) -> String {
        let plan = self.plan.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
        format!("{}#page:{plan}:{}", self.scope, self.prefix)
    }
}

/// The pages of `directory` as its tree is bound now: stored ones when any scope evaluated the
/// same subtree under the same context and plan, otherwise `compute`'s, which are stored.
pub(crate) fn evaluate_pages(
    graph: &(impl Graph + ?Sized),
    directory: DirectoryPages<'_>,
    compute: impl FnOnce() -> StoreResult<Vec<TreePage>>,
) -> StoreResult<Evaluated> {
    let name = tree_name(directory.scope, directory.prefix);
    let tree = graph
        .binding(&name)?
        .filter(|tree| *tree != ABSENT)
        .ok_or_else(|| StoreError::Invalid(format!("no tree is bound for {name}")))?;
    let key = TreePageKey { tree, prefix: directory.prefix, context: directory.context, plan: directory.plan };
    let store = graph.store();
    let (hit, digest) = match store.tree_pages(&key)? {
        Some((digest, _)) => (true, digest),
        None => (false, store.put_tree_pages(&key, &compute()?)?),
    };
    let record = DerivationRecord {
        output: directory.output(),
        producer: directory.plan,
        inputs: vec![
            DerivationInput { name: Some(name), digest: tree },
            DerivationInput { name: None, digest: directory.context },
        ],
        digest,
        status: 0,
        cost: i64::from(!hit),
    };
    Ok(Evaluated { hit, record })
}

#[cfg(test)]
mod tests;
