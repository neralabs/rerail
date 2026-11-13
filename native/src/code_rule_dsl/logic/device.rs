//! The logic's device, behind an interface: the Datalog executor solves on it when one is registered, and on the
//! CPU otherwise. The engine registers its devices as it loads (`crate::install_devices`); on macOS that is Metal
//! (`rerail-device-metal`, `code_rule_dsl::device_metal`). Nothing here names a device.
//!
//! What crosses the interface is data: a stratum's report, a lattice system and its solution, the nearest
//! ancestors' targets. A join's device plan is the device's own, held here as an opaque value.
use std::any::Any;
use std::sync::{Arc, OnceLock};

use super::native::setfix::{Fresh, Interner, JoinRule, Store};

pub mod lattice;
pub mod nearest;
pub use crate::code_rule_dsl::logic::profile::{Report};

/// A join rule's plan on the device, as the device made it (`SolverDevice::compile_join`).
#[derive(Clone)]
pub struct JoinPlan(pub Arc<dyn Any + Send + Sync>);

/// Why the device did not solve: there is none for this work (the caller solves on the CPU), or it failed.
#[derive(Debug)]
pub enum Refused {
    NoDevice,
    Failed(String),
}

/// What the logic runs on a device.
pub trait SolverDevice: Send + Sync {
    /// Whether the device's join stages can run here (a device is present and has their pipelines).
    fn join_available(&self) -> bool;
    /// A seed join's plan on the device, or why it takes none (`spec`: the join's text, which keys its history).
    fn compile_join(&self, join: &JoinRule, spec: &str) -> Result<JoinPlan, String>;
    /// Notes that the device refused the stratum being opened or run: it runs on the CPU, and the profile names why.
    fn refuse(&self, why: String);
    /// The seed joins `rules` (plan, rule, head relation) in one batch, their rows into `fresh`: per rule, the rows
    /// made. `names`: the run's symbols, for texts the device makes into heads.
    fn run_joins(&self, rules: &[(&JoinPlan, &JoinRule, usize)], store: &Store, names: &Interner, fresh: &mut [Fresh]) -> Result<Vec<u64>, String>;
    /// Encodes the seed joins `rules` into this thread's device wave (`lanes::wave`), to be finished by `resume`
    /// once the wave ran: the ticket, or `None` when they cannot wait in it (they then run by `run_joins`).
    fn submit(&self, rules: &[(&JoinPlan, &JoinRule, usize)], names: &Interner) -> Result<Option<u64>, String>;
    /// Finishes the joins `submit` sent with ticket `ticket`: their rows into `fresh`, per rule the rows made.
    fn resume(&self, ticket: u64, rules: &[(&JoinPlan, &JoinRule, usize)], store: &Store, names: &Interner, fresh: &mut [Fresh]) -> Result<Vec<u64>, String>;
    /// Whether the plan reads its own component's relations (a delta join of a recursive component).
    fn reads_own(&self, plan: &JoinPlan) -> bool;
    /// A recursive component's fixpoint on the device: the `seeds` once, then the `deltas` round by round until no
    /// head in `heads` grows, every row into `fresh`. Per rule (seeds, then deltas), the rows made.
    #[allow(clippy::too_many_arguments)]
    fn fixpoint(
        &self,
        seeds: &[(&JoinPlan, &JoinRule, usize)],
        deltas: &[(&JoinPlan, &JoinRule, usize)],
        heads: &[usize],
        store: &Store,
        names: &Interner,
        fresh: &mut [Fresh],
    ) -> Result<Vec<u64>, String>;
    /// The report of the last stratum the device ran or refused on this thread (taken).
    fn take_report(&self) -> Option<Report>;
    /// A flat-lattice system's least fixpoint on the device's work queue.
    fn lattice(&self, system: &lattice::System<'_>) -> Result<lattice::Solved, Refused>;
    /// Nearest ancestors in a set by pointer jumping (`nodes` per set; `target` and `done` set-major).
    fn nearest(&self, nodes: usize, target: &[u32], done: &[u8], how: nearest::Buffers) -> Result<nearest::Solved, Refused>;
    /// Checks every submission the relational executor makes against the engine's launch budget (decision 0100 §2).
    fn install_launch_guard(&self);
}

static SOLVER: OnceLock<&'static dyn SolverDevice> = OnceLock::new();

/// Registers the logic's device; the first registration holds.
pub fn register(device: &'static dyn SolverDevice) {
    let _ = SOLVER.set(device);
}

/// The registered device, if any.
pub(in crate::code_rule_dsl) fn solver() -> Option<&'static dyn SolverDevice> {
    SOLVER.get().copied()
}

/// The report of the last stratum the device ran or refused on this thread, when there is a device.
pub fn take_report() -> Option<Report> {
    solver().and_then(|d| d.take_report())
}

