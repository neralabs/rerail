//! The solver's device interface (`SolverDevice`): setfix's seed joins and recursive fixpoints over its store, and
//! columns' statistics, which the planner reads. A solver device is a device (`Device`, the run's solvers'), and
//! registering it registers both (`register`).
use rerail_blocks::vector::Vector;
use std::any::Any;
use std::sync::Arc;

use super::{Device, Refused};
use crate::code_rule_dsl::logic::native::setfix::{Fresh, InRel, Interner, JoinRule, Store};

/// A join rule's plan on the device, as the device made it (`SolverDevice::compile_join`).
#[derive(Clone)]
pub struct JoinPlan(pub Arc<dyn Any + Send + Sync>);

/// What setfix runs on a device, besides what every device runs (`Device`).
pub trait SolverDevice: Device {
    /// Whether the device's join stages can run here (a device is present and has their pipelines).
    fn join_available(&self) -> bool;
    /// A seed join's plan on the device, or why it takes none (`spec`: the join's text, which keys its history).
    /// `own_texts`: the component's own (relation, column) pairs whose values are texts its rounds make into one arena
    /// (decision 0307: a `shortlex_min` relation's value; decision 0659: a recursive set head's column of made texts),
    /// `head` the join's head relation among them.
    fn compile_join(&self, join: &JoinRule, spec: &str, own_texts: &[(usize, usize)], head: usize) -> Result<JoinPlan, String>;
    /// The head columns `plan` makes texts into (its arena's), for a component's `own_texts`.
    fn made_columns(&self, plan: &JoinPlan) -> Vec<usize>;
    /// Notes that the device refused the stratum being opened or run: it runs on the CPU, and the profile names why.
    fn refuse(&self, why: String);
    /// The seed joins `rules` (plan, rule, head relation) in one batch, their rows into `fresh`: per rule, the rows
    /// made. `names`: the run's symbols, for texts the device makes into heads.
    fn run_joins(&self, rules: &[(&JoinPlan, &JoinRule, usize)], store: &Store, names: &Interner, fresh: &mut [Fresh]) -> Result<Vector<u64>, String>;
    /// Encodes the seed joins `rules` into this thread's device wave (`lanes::wave`), to be finished by `resume`
    /// once the wave ran: the ticket, or `None` when they cannot wait in it (they then run by `run_joins`). `store`:
    /// the relations, as `resume` will find them (its heads' rows placed by the same program).
    fn submit(&self, rules: &[(&JoinPlan, &JoinRule, usize)], store: &Store, names: &Interner) -> Result<Option<u64>, String>;
    /// Finishes the joins `submit` sent with ticket `ticket`: their rows into `fresh`, per rule the rows made.
    fn resume(&self, ticket: u64, rules: &[(&JoinPlan, &JoinRule, usize)], store: &Store, names: &Interner, fresh: &mut [Fresh]) -> Result<Vector<u64>, String>;
    /// Whether the plan reads its own component's relations (a delta join of a recursive component).
    fn reads_own(&self, plan: &JoinPlan) -> bool;
    /// A recursive component's fixpoint on the device: the `seeds` once, then the `deltas` round by round until no
    /// head in `heads` grows, every row into `fresh`. Per rule (seeds, then deltas), the rows made, and whether the
    /// device reached the fixpoint: where a round was too narrow for it, it hands the component back (decision 0657),
    /// its rows so far in `fresh` for the CPU's waves to go on from.
    #[allow(clippy::too_many_arguments)]
    fn fixpoint(
        &self,
        seeds: &[(&JoinPlan, &JoinRule, usize)],
        deltas: &[(&JoinPlan, &JoinRule, usize)],
        heads: &[usize],
        store: &Store,
        names: &Interner,
        fresh: &mut [Fresh],
    ) -> Result<(Vector<u64>, bool), String>;
    /// Columns' exact statistics and `k` most common values (decision 0209 §6), in one batch. A relation's column the
    /// device holds is read where it is (decision 0261); the others are gathered. Also how many were read in place.
    fn column_stats(&self, cols: &[StatColumn<'_>], k: usize) -> Result<(Vec<ColumnStats>, usize), Refused>;
    /// Whether its kernels run on the CPU (`RERAIL_GPU=cpu`): work a launch pays for on a GPU (`table::gpu_pays`) is
    /// the CPU's own there, done faster by the CPU's path.
    fn kernels_on_cpu(&self) -> bool {
        false
    }
}

/// A column for `SolverDevice::column_stats`: values on the host, or column `.1` of an engine relation.
pub enum StatColumn<'a> {
    Values(&'a [i64]),
    Rel(&'a Arc<InRel>, usize),
}

/// A column's statistics: its rows, distinct values, sums of its values' counts squared and cubed, and its most
/// common values with their counts, most common first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColumnStats {
    pub rows: u64,
    pub ndv: u64,
    pub sq: u64,
    pub cube: u64,
    pub top: Vec<(i64, u64)>,
}

/// The logic's solver device, as the program's composition registers it (`register`).
pub static SOLVER: rerail_registry::Slot<&'static dyn SolverDevice> = rerail_registry::Slot::new("the logic's solver device (logic::device::solver::SOLVER)");

/// Registers the logic's solver device, by `provider`, as the logic's device too (`register_device`).
pub fn register(provider: &'static str, device: &'static dyn SolverDevice) {
    SOLVER.register(provider, device);
    super::register_device(provider, device);
}

/// The registered solver device, if any.
pub fn solver() -> Option<&'static dyn SolverDevice> {
    SOLVER.try_get()
}
