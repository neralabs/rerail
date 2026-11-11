//! The logic's device, behind an interface: the Datalog executor solves on it when one is registered, and on the
//! CPU otherwise. The engine registers its devices as it loads (`crate::install_devices`); on macOS that is Metal
//! (`rerail-device-metal`, `code_rule_dsl::device_metal`). Nothing here names a device.
//!
//! What crosses the interface is data: a stratum's report, a lattice system and its solution, the nearest
//! ancestors' targets. A join's device plan is the device's own, held here as an opaque value.
use std::any::Any;
use std::sync::{Arc, OnceLock};

use super::native::setfix::{Fresh, JoinRule, Store};

pub(in crate::code_rule_dsl) mod lattice;
pub(in crate::code_rule_dsl) mod nearest;

/// A join rule's plan on the device, as the device made it (`SolverDevice::compile_join`).
#[derive(Clone)]
pub(in crate::code_rule_dsl) struct JoinPlan(pub(in crate::code_rule_dsl) Arc<dyn Any + Send + Sync>);

/// Why the device did not solve: there is none for this work (the caller solves on the CPU), or it failed.
#[derive(Debug)]
pub(in crate::code_rule_dsl) enum Refused {
    NoDevice,
    Failed(String),
}

/// What the logic runs on a device.
pub(in crate::code_rule_dsl) trait SolverDevice: Send + Sync {
    /// Whether the device's join stages can run here (a device is present and has their pipelines).
    fn join_available(&self) -> bool;
    /// A seed join's plan on the device, or why it takes none (`spec`: the join's text, which keys its history).
    fn compile_join(&self, join: &JoinRule, spec: &str) -> Result<JoinPlan, String>;
    /// Notes that the device refused the stratum being opened or run: it runs on the CPU, and the profile names why.
    fn refuse(&self, why: String);
    /// The seed joins `rules` (plan, rule, head relation) in one batch, their rows into `fresh`: per rule, the rows
    /// made.
    fn run_joins(&self, rules: &[(&JoinPlan, &JoinRule, usize)], store: &Store, fresh: &mut [Fresh]) -> Result<Vec<u64>, String>;
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
pub(in crate::code_rule_dsl) fn register(device: &'static dyn SolverDevice) {
    let _ = SOLVER.set(device);
}

/// The registered device, if any.
pub(in crate::code_rule_dsl) fn solver() -> Option<&'static dyn SolverDevice> {
    SOLVER.get().copied()
}

/// The report of the last stratum the device ran or refused on this thread, when there is a device.
pub(in crate::code_rule_dsl) fn take_report() -> Option<Report> {
    solver().and_then(|d| d.take_report())
}

/// What a stratum's device run did, for the profile.
#[derive(Clone, Debug, Default)]
pub(in crate::code_rule_dsl) struct Report {
    pub(in crate::code_rule_dsl) rules: u32,
    pub(in crate::code_rule_dsl) stages: u32,
    pub(in crate::code_rule_dsl) submissions: u32,
    pub(in crate::code_rule_dsl) dispatches: u32,
    /// Runs again from an overflowing stage.
    pub(in crate::code_rule_dsl) reruns: u32,
    pub(in crate::code_rule_dsl) gpu_seconds: f64,
    /// This thread's CPU: encoding (uploads and sorts' encoding included), waiting, and reading the rows back into
    /// the store.
    pub(in crate::code_rule_dsl) encode_cpu: f64,
    pub(in crate::code_rule_dsl) wait_cpu: f64,
    pub(in crate::code_rule_dsl) wait_wall: f64,
    pub(in crate::code_rule_dsl) decode_cpu: f64,
    /// Head rows left once made distinct on the device (of `rows`).
    pub(in crate::code_rule_dsl) distinct: u64,
    pub(in crate::code_rule_dsl) upload_cpu: f64,
    /// Relations uploaded (and their bytes), found resident; sorts made, found resident.
    pub(in crate::code_rule_dsl) uploads: u32,
    pub(in crate::code_rule_dsl) upload_bytes: u64,
    pub(in crate::code_rule_dsl) resident_hits: u32,
    pub(in crate::code_rule_dsl) sorts: u32,
    pub(in crate::code_rule_dsl) sort_hits: u32,
    /// Rows the heads made (before the store's set), and bindings the stages made.
    pub(in crate::code_rule_dsl) rows: u64,
    pub(in crate::code_rule_dsl) bindings: u64,
    /// Why the device refused the stratum (at open, or at run time: a table past its budget), which then ran on the
    /// CPU; `None` when it ran here.
    pub(in crate::code_rule_dsl) refused: Option<String>,
}
