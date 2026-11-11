//! The nearest ancestor of each node of a chunk's trees whose kind is in a set (`nearest`), which the containment
//! relations read for a whole batch: the trees and where the solve runs are facts', and the solve is the file-local
//! FIX's (`typing::file_local::nearest`), registered at load (`install_nearest`).

use crate::content_store::{StoreError, StoreResult};

/// No node: a root's parent, or a node with no ancestor in the set.
pub(crate) const NONE: u32 = u32::MAX;

/// Where the file-local FIX runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Binding {
    Cpu,
    /// The device work queue; the CPU where there is no device.
    Metal,
}

/// A chunk's trees: per node (a cell of the chunk), its parent's cell or `NONE`, and its kind atom.
pub(crate) struct Trees<'a> {
    pub(crate) parent: &'a [u32],
    pub(crate) kind: &'a [i64],
}

/// What solves `nearest`: per set, each node's nearest strict ancestor of a kind in the set, or `NONE`.
pub(crate) type Nearest = fn(&Trees<'_>, &[Vec<i64>], Binding) -> StoreResult<Vec<Vec<u32>>>;

static NEAREST: std::sync::OnceLock<Nearest> = std::sync::OnceLock::new();

/// Registers the solve (once, at load).
pub(crate) fn install_nearest(solve: Nearest) {
    let _ = NEAREST.set(solve);
}

/// The registered solve of `nearest` over `trees`.
pub(crate) fn nearest(trees: &Trees<'_>, sets: &[Vec<i64>], binding: Binding) -> StoreResult<Vec<Vec<u32>>> {
    match NEAREST.get() {
        Some(solve) => solve(trees, sets, binding),
        None => Err(StoreError::Invalid("no nearest solve is registered".to_owned())),
    }
}
