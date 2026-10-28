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
    // Until `end_push`, SQLite may hold any prefix of this push's writes; an error leaves the
    // push marked unended, and `recover` finishes it.
    store.begin_push()?;
    let (changed, trees_read) = observe_root(store, scope, new)?;
    let rebound = changed.len();
    let work = propagate(store, changed, max_waves, recompute)?;
    store.end_push()?;
    Ok(Push { trees_read, rebound, work })
}

/// Finishes a push that did not end (open-items O1): called on open, before any push. Observed
/// names are never recomputed: the observer re-diffs from its bound root, and a write-back prefix
/// binds a root only with the names it rebound. Every derivation an unwritten write would have
/// recomputed is invalid by the validity join, so the invalid derivations are the first wave and
/// `propagate` carries their changes as on any push. None: the store was clean.
pub(crate) fn recover(
    store: &(impl Graph + ?Sized),
    max_waves: u32,
    mut recompute: impl FnMut(&[Derivation]) -> StoreResult<Vec<DerivationRecord>>,
) -> StoreResult<Option<Work>> {
    if store.store().push_clean()? {
        return Ok(None);
    }
    let frontier = store.store().invalid_derivations()?;
    let mut work = Work::default();
    if !frontier.is_empty() {
        let records = recompute(&frontier)?;
        work.rerun = records.len() as u64;
        let changed = store.record_derivations(&records)?;
        work.changed = changed.len() as u64;
        let rest = propagate(store, changed, max_waves.saturating_sub(1), recompute)?;
        work = Work { waves: 1 + rest.waves, rerun: work.rerun + rest.rerun, changed: work.changed + rest.changed };
    }
    store.end_push()?;
    Ok(Some(work))
}

#[cfg(test)]
mod tests;
