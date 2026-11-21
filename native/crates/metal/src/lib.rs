//! The engine's GPU runtime, in a crate of its own so that the GPU code that needs no engine types
//! rebuilds without the engine: the device (the device layer's, on every backend: decision 0440),
//! the buffer budget and its reservations, mapped no-copy buffers (`HostMapped`), the execution
//! lock, compiled pipelines, and the
//! launch layer's per-evaluation budget (`launch`).
//!
//! `test-hooks` gives each test thread its own buffer ledger (`test_pool`) and the buffer
//! observations, for the engine's tests as for this crate's.
// Where the device layer is (decision 0440 step E).
#![cfg(any(target_os = "macos", target_os = "linux"))]
#![allow(clippy::missing_safety_doc)]

/// Block programs on Metal (decision 0066): the interpreter and generated MSL, over a batch of
/// files read in place from source slabs.
pub mod block_program;
pub mod buffer_observation;
/// The engine's commands on the device layer (decision 0440 step C).
pub mod lane;
pub mod launch;
pub mod launch_budget;
/// The preprocessor's evaluation arena and its kernels' one UKL source (decisions 0448, 0468).
pub mod pp_arena;
/// The preprocessor's evaluator over tapes (decision 0436).
pub mod pp_eval;
/// Macro expansion on the device (decision 0448).
pub mod pp_expand;
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
/// A machine object (decision 0442), as the pool it hooks into is.
#[derive(Default)]
struct PoolHooks(std::sync::OnceLock<()>);

/// The process's one device as Metal's (`rerail_accel::gpu::shared`, decision 0100 §2), for what is Metal's own:
/// its compiler's probes, its residency's measures.
#[cfg(target_os = "macos")]
pub static METAL_DEVICE: std::sync::LazyLock<Option<metal::Device>> = std::sync::LazyLock::new(|| {
    pool_hooks();
    rerail_accel::gpu::shared::get().map(|s| s.device.clone())
});

/// Installs the pool's hooks (`PoolHooks`), once a machine.
pub(crate) fn pool_hooks() {
    rerail_context::machine::<PoolHooks>().0.get_or_init(|| {
        // Relational work's bounded leases count the pool's reservations against the device's budget (decision 0126).
        rerail_accel::gpu::shared::set_other_reserved(runtime::reserved);
        // The pool's one eviction order: idle buffers, then the residency's least recently used slabs (0266 §16).
        rerail_accel::gpu::shared::set_evictor(slabs::evict);
    });
}

/// The process's one device (`METAL_DEVICE`), or `UnsupportedBackend` without one.
#[cfg(target_os = "macos")]
pub fn device() -> Result<metal::Device, Error> {
    METAL_DEVICE.clone().ok_or(Error::UnsupportedBackend)
}

