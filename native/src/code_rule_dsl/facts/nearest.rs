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
pub(crate) fn nearest(trees: &Trees<'_>, sets: &[Vec<i64>], binding: Binding) -> StoreResult<Vec<Vec<u32>>> {
    match NEAREST.get() {
        Some(solve) => solve(trees, sets, binding),
        None => Err(StoreError::Invalid("no nearest solve is registered".to_owned())),
    }
}

/// A kind-set axis's batch solve made on the device from the batch's chunk block (decision 0289 §2): per node row of
/// the batch's `node`, the first of its rows (`nodes + 1` values, `u32`, in the chunk), and its rows, each the ids
/// of `n` and `a`, written once into a relation of their own (decision 0353 step 5), which the host and the device
/// read where it lies.
#[cfg(target_os = "macos")]
pub struct InChunk {
    pub start: usize,
    pub rows: super::rows::Part,
}

/// What makes an axis's solve in a chunk: `members` per dialect tag (256) and kind code (1,024) a bit, `None` for
/// every kind (containment); `inside` for every ancestor in the set, else the nearest.
#[cfg(target_os = "macos")]
/// Several chunks' rows go in one relation, each chunk's past those before (a chunk's batch and its tape values,
/// decision 0431 §8); the starts are the first's.
pub type ChunkAxis = fn(&[&crate::code_rule_dsl::facts::vtab::syntax::device_chunk::Chunk], Option<&[u32]>, bool) -> Result<InChunk, String>;

#[cfg(target_os = "macos")]
static CHUNK_AXIS: std::sync::OnceLock<ChunkAxis> = std::sync::OnceLock::new();

/// Registers the device's solve of an axis in a chunk (once, at load: the Metal device's).
#[cfg(target_os = "macos")]
pub fn install_chunk_axis(solve: ChunkAxis) {
    let _ = CHUNK_AXIS.set(solve);
}

/// The device's solve of an axis in a chunk, when one is registered.
#[cfg(target_os = "macos")]
pub(crate) fn chunk_axis() -> Option<ChunkAxis> {
    CHUNK_AXIS.get().copied()
}
