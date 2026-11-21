//! The engine's Metal runtime, in a crate of its own so that the GPU code that needs no engine
//! types rebuilds without the engine: the device, the buffer budget and its reservations,
//! mapped no-copy buffers (`MappedBuffer`), the execution lock, compiled pipelines, and the
//! launch layer's per-evaluation budget (`launch`).
//!
//! `test-hooks` gives each test thread its own buffer ledger (`test_pool`) and the buffer
//! observations, for the engine's tests as for this crate's.
#![cfg(target_os = "macos")]
#![allow(clippy::missing_safety_doc)]

/// Block programs on Metal (decision 0066): the interpreter and generated MSL, over a batch of
/// files read in place from source slabs.
pub mod block_program;
pub mod buffer_observation;
/// The engine's commands on the device layer (decision 0440 step C).
pub mod lane;
pub mod launch;
pub mod launch_budget;
/// The preprocessor's evaluator over tapes (decision 0436): plain MSL passes.
/// The building blocks of the preprocessor's grid passes (decision 0448).
pub mod pp_blocks;
pub mod pp_eval;
/// Macro expansion on the device (decision 0448).
pub mod pp_expand;
/// The device work queue (decision 0027) in MSL, for the preprocessor's expansion (0448) until it
/// is written in UKL; the engine's kernels include `<ukl/workqueue.h>` (0440 step C).
pub mod work_queue {
    pub const SOURCE: &str = include_str!("work_queue.metal");
}
/// The preprocessing words on the device (decision 0348, stage 5).
pub mod pp_on;
/// Pre-pinned blocks (decision 0039): the device's scratch buffers, made once and reused.
pub mod pool;
pub mod runtime;
/// The slab residency's arena and sources (decision 0032).
pub mod slabs;
#[cfg(any(test, feature = "test-hooks"))]
pub mod test_pool;
/// Tier 2 on the buffers tier 1 left on the device (`image_on`).
pub mod tier2_on;

pub use rerail_compiler::Error;

/// The pool's hooks into this crate, installed before the process's device is first used through it (by
/// `METAL_DEVICE` or the device layer's `lane::device`): whatever its backend, the pool counts the ledger's
/// reservations and evicts the residency's slabs.
static POOL_HOOKS: std::sync::LazyLock<()> = std::sync::LazyLock::new(|| {
    // Relational work's bounded leases count the pool's reservations against the device's budget (decision 0126).
    rerail_accel::gpu::shared::set_other_reserved(runtime::ledger);
    // The pool's one eviction order: idle buffers, then the residency's least recently used slabs (0266 §16).
    rerail_accel::gpu::shared::set_evictor(slabs::evict);
});

/// The process's one device as Metal's (`rerail_accel::gpu::shared`, decision 0100 §2), for what is Metal's own:
/// its compiler's probes, its residency's measures.
pub static METAL_DEVICE: std::sync::LazyLock<Option<metal::Device>> = std::sync::LazyLock::new(|| {
    std::sync::LazyLock::force(&POOL_HOOKS);
    rerail_accel::gpu::shared::get().map(|s| s.device.clone())
});

/// Installs the pool's hooks (`POOL_HOOKS`).
pub(crate) fn pool_hooks() {
    std::sync::LazyLock::force(&POOL_HOOKS);
}

/// The process's one device (`METAL_DEVICE`), or `UnsupportedBackend` without one.
pub fn device() -> Result<metal::Device, Error> {
    METAL_DEVICE.clone().ok_or(Error::UnsupportedBackend)
}

/// The process's one command queue (`rerail_accel::gpu::shared`, decision 0100 §2). Every command buffer the engine
/// makes is made on it, so the engine's programs, its relational work and the executor's lanes are submitted in one
/// order; no module makes a queue of its own (`executors_are_shared`, the engine's source guard).
pub fn queue() -> Result<metal::CommandQueue, Error> {
    rerail_accel::gpu::shared::get().map(|s| s.queue.clone()).ok_or(Error::UnsupportedBackend)
}
