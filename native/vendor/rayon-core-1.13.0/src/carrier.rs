//! What a job carries from the thread that made it to the thread that runs it (Rerail's patch, PATCHES.md).
//!
//! A program installs a carrier once, before its first job: `capture` runs where a job is made (a `join`'s second
//! half, an `install`ed closure, a `spawn`ed or scoped job), and `enter` where another thread runs it, its guard held
//! while the job runs and dropped before the job says it is done. A job its maker runs itself (a `join` half popped
//! back) enters nothing. Without a carrier, jobs carry nothing, as upstream's do.

use std::any::Any;
use std::sync::OnceLock;

/// What a job carries: the maker's state, as the carrier captured it.
pub type Carried = Box<dyn Any + Send>;
/// Captures what a job made on this thread carries; `None` for nothing.
pub type CaptureFn = fn() -> Option<Carried>;
/// Enters what a job carried on the thread that runs it, until the guard drops.
pub type EnterFn = fn(Carried) -> Box<dyn Any>;

// Code registered at load, as a panic hook is: the program's one carrier, set before its first job.
static CARRIER: OnceLock<(CaptureFn, EnterFn)> = OnceLock::new();

/// Installs the carrier, once for the process; later calls are ignored. Whether this call installed it.
pub fn set_job_carrier(capture: CaptureFn, enter: EnterFn) -> bool {
    CARRIER.set((capture, enter)).is_ok()
}

/// What a job made here carries.
#[inline]
pub(super) fn capture() -> Option<Carried> {
    CARRIER.get().and_then(|(capture, _)| capture())
}

/// Enters `carried` on this thread until the guard drops.
#[inline]
pub(super) fn enter(carried: Option<Carried>) -> Option<Box<dyn Any>> {
    let carried = carried?;
    CARRIER.get().map(|(_, enter)| enter(carried))
}
