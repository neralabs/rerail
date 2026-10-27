//! Observers: the start of the push path (docs/final-shape/03-architecture.md §2, §15). An observer
//! sees a new Git root tree for a scope (a workspace), compares it with the root it saw last, and
//! rebinds the names of exactly the directories and files whose object changed. The rebound names
//! are the batch `propagate` starts from. The root it saw last is its own binding of the root's
//! name, so an observer keeps no state of its own.

use super::Graph;
use super::propagate::{Work, propagate};
use crate::content_store::{Binding, Derivation, DerivationRecord, HashKey, StoreResult};

/// The name a file's object is bound under in `scope`.
pub(crate) fn path_name(scope: &str, path: &str) -> String {
    format!("{scope}#path:{path}")
}

/// The name a directory's tree hash is bound under in `scope` ("" is the root).
pub(crate) fn tree_name(scope: &str, prefix: &str) -> String {
    format!("{scope}#tree:{prefix}")
}

/// What one push did: trees the comparison read, names it rebound, and the propagation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Push {
    pub trees_read: usize,
    pub rebound: usize,
    pub work: Work,
}

/// Rebinds the names of `scope` that changed between the root tree last observed for it and
/// `new`; returns the names whose digest changed, and how many trees the comparison read.
pub(crate) fn observe_root(store: &(impl Graph + ?Sized), scope: &str, new: HashKey) -> StoreResult<(Vec<String>, usize)> {
    let old = store.binding(&tree_name(scope, ""))?;
    let diff = store.store().tree_changes_since(old, new)?;
    let bindings = diff
        .changes
        .into_iter()
        .map(|change| Binding {
            name: if change.tree { tree_name(scope, &change.path) } else { path_name(scope, &change.path) },
            digest: change.digest,
        })
        .collect::<Vec<_>>();
    Ok((store.bind_names(&bindings)?, diff.trees_read))
}

/// One push: observe the new root, then recompute every derivation the rebound names dirty.
pub(crate) fn push_root(
    store: &(impl Graph + ?Sized),
    scope: &str,
    new: HashKey,
    max_waves: u32,
    recompute: impl FnMut(&[Derivation]) -> StoreResult<Vec<DerivationRecord>>,
) -> StoreResult<Push> {
    let (changed, trees_read) = observe_root(store, scope, new)?;
    let rebound = changed.len();
    let work = propagate(store, changed, max_waves, recompute)?;
    Ok(Push { trees_read, rebound, work })
}

#[cfg(test)]
mod tests;
