//! The GPU lane's part below the engine (crate split 10d-3): the pool every relational submission leases from
//! (`shared`, decision 0100 §2), the launch ledger (`launch`), the lane's batches' counts (`programs`), and the slots
//! its kernels register into as the engine boots. The lane itself (its kernels, the device joins, the tiles) is
//! rerail-accel-gpu's `gpu`, over the GPU's compiler (rerail-gpu, UKL): the executor and every crate that leases or
//! counts build without it.

use crate::exec::Backend;
use std::sync::Arc;

pub mod launch;
pub mod shared;

/// `f` in a pool of its own for the objects its work autoreleases: Metal's, on macOS; elsewhere `f` as it is.
pub fn released<T>(f: impl FnOnce() -> T) -> T {
    #[cfg(target_os = "macos")]
    {
        objc::rc::autoreleasepool(f)
    }
    #[cfg(not(target_os = "macos"))]
    {
        f()
    }
}

/// What makes the GPU lane's backend: none where this machine has no device or its kernels do not compile.
pub type MakeBackend = fn() -> Option<Arc<dyn Backend>>;

/// The GPU lane's backend, as the executor takes it (`exec`): rerail-accel-gpu's `gpu::backend`, registered as the
/// engine boots. None registered (a binary that does not boot): no GPU lane.
pub static BACKEND: rerail_registry::Slot<MakeBackend> = rerail_registry::Slot::new("the GPU lane's backend (accel::gpu::BACKEND)");

/// The GPU lane's backend, when one is registered and this machine has a device and its kernels.
pub fn backend() -> Option<Arc<dyn Backend>> {
    BACKEND.try_get().and_then(|make| make())
}

/// Upkeep's job for the lane's kernels (rerail-accel-gpu's `gpu::kernels::Maintenance`, decision 0395): compiles
/// what runs went without when the machine is idle. Registered with the lane.
pub static MAINTENANCE: rerail_registry::Slot<fn() -> Box<dyn crate::upkeep::Job>> = rerail_registry::Slot::new("the GPU kernels' upkeep (accel::gpu::MAINTENANCE)");

/// The GPU lane's batches of programs on this machine (decision 0442): running now, begun, and begun while another
/// still ran (decision 0344: one's host part beside another's command buffers).
#[derive(Default)]
pub struct Programs {
    pub running: std::sync::atomic::AtomicUsize,
    pub begun: std::sync::atomic::AtomicUsize,
    pub beside: std::sync::atomic::AtomicUsize,
}

/// This machine's counts of the lane's batches of programs.
pub fn programs() -> Arc<Programs> {
    rerail_context::machine::<Programs>()
}

/// The GPU lane's batches of programs begun on this machine, and how many of them began while another still ran.
pub fn programs_begun() -> (usize, usize) {
    let p = programs();
    (p.begun.load(std::sync::atomic::Ordering::Relaxed), p.beside.load(std::sync::atomic::Ordering::Relaxed))
}
