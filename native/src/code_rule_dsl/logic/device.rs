//! The logic's device, behind an interface: the Datalog executor solves on it when one is registered, and on the
//! CPU otherwise. The engine registers its devices as it loads (`crate::install_devices`); on macOS that is Metal
//! (`rerail-device-metal`, `code_rule_dsl::device_metal`). Nothing here names a device.
//!
//! What crosses the interface is data: a stratum's report, a lattice system and its solution, the nearest
//! ancestors' targets. This is the device as the run's own solvers use it (`Device`); setfix's joins and fixpoints
//! over its store, and the column statistics the planner reads, are the solver's interface over it
//! (`SolverDevice`, `device::solver`, setfix's), and registering a solver device registers both.
use std::sync::OnceLock;

/// The run's binding: the CPU only, or the device too.
pub mod binding;
pub mod caps;
pub mod lattice;
pub mod nearest;
pub use crate::code_rule_dsl::logic::profile::{Report};

// cratify: `solver` is rerail-setfix's (it mounts it: its joins and fixpoints name setfix's store)

/// Why the device did not solve: there is none for this work (the caller solves on the CPU), or it failed.
#[derive(Debug)]
pub enum Refused {
    NoDevice,
    Failed(String),
}

/// What the logic's own solvers run on a device (and what every solver device does: `SolverDevice`).
pub trait Device: Send + Sync {
    /// The report of the last stratum the device ran or refused on this thread (taken).
    fn take_report(&self) -> Option<Report>;
    /// A flat-lattice system's least fixpoint on the device's work queue.
    fn lattice(&self, system: &lattice::System<'_>) -> Result<lattice::Solved, Refused>;
    /// Nearest ancestors in a set by pointer jumping (`nodes` per set; `target` and `done` set-major).
    fn nearest(&self, nodes: usize, target: &[u32], done: &[u8], how: nearest::Buffers) -> Result<nearest::Solved, Refused>;
    /// Checks every submission the relational executor makes against the engine's launch budget (decision 0100 §2).
    fn install_launch_guard(&self);
}

static DEVICE: OnceLock<&'static dyn Device> = OnceLock::new();

/// Registers the logic's device; the first registration holds. A solver device registers here through
/// `device::solver::register`.
pub fn register_device(device: &'static dyn Device) {
    let _ = DEVICE.set(device);
}

/// The registered device, if any.
pub fn device() -> Option<&'static dyn Device> {
    DEVICE.get().copied()
}

/// The report of the last stratum the device ran or refused on this thread, when there is a device.
pub fn take_report() -> Option<Report> {
    device().and_then(|d| d.take_report())
}
