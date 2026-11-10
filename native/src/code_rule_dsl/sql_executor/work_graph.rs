//! Read-only SQL work: fuse dependencies, export only cut edges, and dispatch ready groups.
//! Connections belong to workers. Native callbacks expose storage; SQLite evaluates every operator.
use super::*;
use rerail_accel::kernel::{Kernel, Output, Program, ProgramKernel, ProgramOutput};
use rerail_accel::{Batch, Lane, Launch};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

mod planner;
#[cfg(test)]
mod tests;
mod worker;

#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Node {
    pub name: String,
    pub sql: String,
    /// Output columns are c0..cN; cells are nullable integers, including symbol atoms.
    pub width: usize,
    pub reads: Vec<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub nodes: Vec<Node>,
    pub outputs: Vec<String>,
    #[serde(default = "allowance")]
    pub max_submissions: usize,
}
fn allowance() -> usize {
    100
}
static NEXT_GRAPH: AtomicU64 = AtomicU64::new(1);

struct Shared {
    submitted: AtomicUsize,
    limit: usize,
    steps: AtomicU64,
    max_steps: Option<u64>,
    deadline: Option<Instant>,
    cancelled: AtomicBool,
    accounting: Option<Arc<Accounting>>,
}
#[derive(Default)]
pub(super) struct Accounting(pub [AtomicU64; 6]);
impl Accounting {
    pub fn totals(&self) -> [u64; 6] {
        std::array::from_fn(|n| self.0[n].load(Ordering::Relaxed))
    }
}
impl Shared {
    fn new(
        limits: &runtime::Limits,
        limit: usize,
        accounting: Option<Arc<Accounting>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            submitted: AtomicUsize::new(0),
            limit,
            steps: AtomicU64::new(0),
            max_steps: limits.max_steps,
            deadline: limits
                .timeout_ms
                .and_then(|ms| Instant::now().checked_add(Duration::from_millis(ms))),
            cancelled: AtomicBool::new(false),
            accounting,
        })
    }
    fn interrupted(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed) || self.deadline.is_some_and(|d| Instant::now() >= d)
    }
    fn submit(&self) -> std::result::Result<(), String> {
        if self.interrupted() {
            return Err("SQL work graph cancelled or timed out".into());
        }
        self.submitted
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < self.limit).then(|| n + 1)
            })
            .map(|_| ())
            .map_err(|_| "SQL work graph submission allowance exhausted".into())
    }
}

#[derive(Debug)]
pub(super) struct ResultSet {
    pub rows: BTreeMap<String, Arc<worker::Rows>>,
    pub explain: Value,
    pub submissions: usize,
    pub vm_steps: u64,
}

pub(super) fn execute(
    request: Request,
    sources: BTreeMap<String, Arc<dyn Source>>,
    limits: &runtime::Limits,
    profiler: Option<&super::super::sql_profile::Profiler>,
) -> Result<ResultSet> {
    execute_accounted(request, sources, limits, profiler, None)
}
pub(super) fn execute_accounted(
    request: Request,
    sources: BTreeMap<String, Arc<dyn Source>>,
    limits: &runtime::Limits,
    profiler: Option<&super::super::sql_profile::Profiler>,
    accounting: Option<Arc<Accounting>>,
) -> Result<ResultSet> {
    let plan =
        planner::Plan::new(&request, &sources, rerail_accel::exec::threads()).map_err(error)?;
    let shared = Shared::new(limits, request.max_submissions, accounting);
    let explain = plan.explain(&request);
    let graph = NEXT_GRAPH.fetch_add(1, Ordering::Relaxed);
    let _context = profiler.map(|p| p.scope(json!({"sqlWorkGraph":graph,"sqlWorkPlan":explain})));
    let mut values = sources;
    let mut exported = BTreeMap::new();
    let requested = request.outputs.iter().collect::<BTreeSet<_>>();
    let mut completed = BTreeSet::new();
    let mut pending = (0..plan.groups.len()).collect::<BTreeSet<_>>();
    while !pending.is_empty() {
        let ready = pending
            .iter()
            .copied()
            .filter(|&id| plan.groups[id].dependencies.is_subset(&completed))
            .collect::<Vec<_>>();
        if ready.is_empty() {
            return Err(error("SQL work graph has a dependency cycle"));
        }
        let jobs = ready
            .iter()
            .map(|&id| {
                let group = &plan.groups[id];
                let bindings = group
                    .inputs
                    .iter()
                    .map(|name| {
                        values
                            .get(name)
                            .cloned()
                            .map(|source| (name.clone(), source))
                            .ok_or_else(|| error(format!("SQL group {id} is missing input {name}")))
                    })
                    .collect::<Result<BTreeMap<_, _>>>()?;
                Ok(Launch::on(
                    Kernel::Program(ProgramKernel(Arc::new(worker::Job {
                        id,
                        sql: group.sql(&request),
                        outputs: group.outputs.clone(),
                        bindings,
                        shared: shared.clone(),
                        graph,
                        explain: explain["groups"][id].clone(),
                        units: group.cost.max(1) as f64,
                    }))),
                    Lane::Sql,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        // Executor::run joins every submitted job, including its failures, before returning.
        // No worker can outlive these pinned sources or publish a partial result.
        let done = super::super::logic::lanes::sql_executor()
            .run_as("SQL read-only graph", Batch(jobs))
            .map_err(error)?;
        for (&id, result) in ready.iter().zip(done.0) {
            let Output::Program(output) = result.out else {
                return Err(error("invalid SQL work output"));
            };
            let output = output
                .0
                .downcast_ref::<worker::Results>()
                .ok_or_else(|| error("invalid SQL work result type"))?;
            for (name, rows) in &output.0 {
                values.insert(name.clone(), rows.clone());
                if requested.contains(name) {
                    exported.insert(name.clone(), rows.clone());
                }
            }
            pending.remove(&id);
            completed.insert(id);
        }
        // A cut root lives until its last reader completes, not until the whole job ends.
        values.retain(|name, _| {
            pending
                .iter()
                .any(|&id| plan.groups[id].inputs.contains(name))
        });
    }
    let rows = request
        .outputs
        .iter()
        .map(|name| {
            let rows = exported
                .remove(name)
                .ok_or_else(|| error("missing SQL work graph output"))?;
            Ok((name.clone(), rows))
        })
        .collect::<Result<_>>()?;
    Ok(ResultSet {
        rows,
        explain,
        submissions: shared.submitted.load(Ordering::Acquire),
        vm_steps: shared.steps.load(Ordering::Relaxed),
    })
}
