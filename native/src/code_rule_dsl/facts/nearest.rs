//! The nearest ancestor of each node of a chunk's trees whose kind is in a set (`nearest`), which the containment
//! relations read for a whole batch: the trees and where the solve runs are facts', and the solve is the file-local
//! FIX's (`typing::file_local::nearest`), registered at load (`install_nearest`).

use crate::content_store::{StoreError, StoreResult};

/// No node: a root's parent, or a node with no ancestor in the set.
pub const NONE: u32 = u32::MAX;

/// Where the file-local FIX runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Binding {
    Cpu,
    /// The device work queue; the CPU where there is no device.
    Metal,
}

/// A chunk's trees: per node (a cell of the chunk), its parent's cell or `NONE`, and its kind atom.
pub struct Trees<'a> {
    pub parent: &'a [u32],
    pub kind: &'a [i64],
}

/// What solves `nearest`: per set, each node's nearest strict ancestor of a kind in the set, or `NONE`.
pub(crate) type Nearest = fn(&Trees<'_>, &[Vec<i64>], Binding) -> StoreResult<Vec<Vec<u32>>>;

static NEAREST: std::sync::OnceLock<Nearest> = std::sync::OnceLock::new();

/// Registers the solve (once, at load).
pub fn install_nearest(solve: Nearest) {
    let _ = NEAREST.set(solve);
}

/// The registered solve of `nearest` over `trees`.
pub fn nearest(trees: &Trees<'_>, sets: &[Vec<i64>], binding: Binding) -> StoreResult<Vec<Vec<u32>>> {
    match NEAREST.get() {
        Some(solve) => solve(trees, sets, binding),
        None => Err(StoreError::Invalid("no nearest solve is registered".to_owned())),
    }
}

/// A kind-set axis's batch solve made on the device off the tape (decision 0353 step 4): per node row of the batch's
/// `node`, the first of its rows (`nodes + 1` values, `u32`, at `start` in the batch's block), and its rows, each the
/// ids of `n` and `a`, written once into a relation of their own (decision 0353 step 5), which the host and the device
/// read where it lies.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
pub struct SolvedOnDevice {
    pub start: usize,
    pub rows: super::rows::Part,
}

/// What makes an axis's solve on the device: a views' directory read in place (a batch's, or a layered source's of
/// a batch and its tape values, decision 0431 §8), its `node` rows, the block its starts go in (the batch's),
/// `members` per kind atom (256 tags of 1,024 slots) a bit, `None` for every kind (containment); `inside` for every
/// ancestor in the set, else the nearest.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
pub type AxisSolve = fn(&crate::code_rule_dsl::facts::vtab::ViewInPlace, usize, &rerail_blocks::Block, usize, Option<&[u32]>, bool) -> Result<SolvedOnDevice, String>;

/// The device's solve of an axis (registered once, at load: the device crate's).
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
pub static AXIS_SOLVE: rerail_registry::Slot<AxisSolve> = rerail_registry::Slot::new("the device's solve of an axis, its views read in place (facts::nearest::AXIS_SOLVE)");

/// The device's solve of an axis, when one is registered.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
pub fn axis_solve() -> Option<AxisSolve> {
    AXIS_SOLVE.try_get()
}
