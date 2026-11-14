//! Read-only SQL work: fuse dependencies, export only cut edges, and dispatch ready groups.
//! Connections belong to workers. Native callbacks expose storage; SQLite evaluates every operator.
use super::*;
use rerail_accel::kernel::{Kernel, Program, ProgramKernel, ProgramOutput};
use rerail_accel::{Batch, Lane, Launch};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

pub mod planner;
// tests: a test the engine hosts (it reads above this crate; tools/cratify/test-hosts.tsv)
pub mod worker;

#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub name: String,
    pub sql: String,
    /// Output columns are c0..cN; cells are nullable integers, including symbol atoms.
    pub width: usize,
    pub reads: Vec<String>,
    #[serde(default = "materialized")]
    pub materialized: bool,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub nodes: Vec<Node>,
    pub outputs: Vec<String>,
    #[serde(default = "allowance")]
    pub max_submissions: usize,
}
fn materialized() -> bool {
    true
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
    reservation: Option<Arc<super::super::sql_admission::Reservation>>,
}
#[derive(Default)]
pub struct Accounting(pub [AtomicU64; 6]);
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
            reservation: None,
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
pub struct ResultSet {
    pub rows: BTreeMap<String, Arc<worker::Rows>>,
    pub explain: Value,
    pub submissions: usize,
    pub vm_steps: u64,
}

#[cfg(any(test, feature = "test-hooks"))]
pub fn execute(
    request: Request,
    sources: BTreeMap<String, Arc<dyn Source>>,
    limits: &runtime::Limits,
    profiler: Option<&super::super::sql_profile::Profiler>,
) -> Result<ResultSet> {
    execute_accounted(request, sources, limits, profiler, None)
}
pub fn execute_accounted(
    mut request: Request,
    sources: BTreeMap<String, Arc<dyn Source>>,
    limits: &runtime::Limits,
    profiler: Option<&super::super::sql_profile::Profiler>,
    accounting: Option<Arc<Accounting>>,
) -> Result<ResultSet> {
    let ledger = super::super::sql_admission::current();
    if let Some(ledger) = &ledger {
        request.max_submissions = request.max_submissions.min(ledger.available());
    }
    let plan =
        planner::Plan::new(&request, &sources, rerail_accel::exec::threads()).map_err(error)?;
    execute_planned(request, sources, limits, profiler, accounting, plan)
}

pub fn execute_planned(
    request: Request,
    sources: BTreeMap<String, Arc<dyn Source>>,
    limits: &runtime::Limits,
    profiler: Option<&super::super::sql_profile::Profiler>,
    accounting: Option<Arc<Accounting>>,
    plan: planner::Plan,
) -> Result<ResultSet> {
    let reservation = super::super::sql_admission::current()
        .map(|l| l.reserve(plan.groups.len()))
        .transpose()
        .map_err(error)?;
    let mut shared = Shared::new(limits, request.max_submissions, accounting);
    Arc::get_mut(&mut shared).unwrap().reservation = reservation;
    let explain = plan.explain(&request);
    let graph = NEXT_GRAPH.fetch_add(1, Ordering::Relaxed);
    let _context = profiler.map(|p| p.scope(json!({"sqlWorkGraph":graph,"sqlWorkPlan":explain})));
    let requested = request.outputs.iter().cloned().collect();
    let pending = (0..plan.groups.len()).collect();
    let scheduler = Arc::new(Scheduler {
        request,
        plan,
        graph,
        explain: explain.clone(),
        shared: shared.clone(),
        requested,
        state: parking_lot::Mutex::new(Scheduled {
            values: sources,
            exported: BTreeMap::new(),
            pending,
            completed: BTreeSet::new(),
            running: BTreeMap::new(),
            failure: None,
        }),
    });
    let _guard = Flights(scheduler.clone());
    scheduler
        .enqueue(&mut scheduler.state.lock())
        .map_err(error)?;
    loop {
        let (finished, done) = {
            let mut state = scheduler.state.lock();
            let finished = state
                .running
                .iter_mut()
                .filter_map(|(&id, job)| job.ready().then_some(id))
                .collect::<Vec<_>>();
            let finished = finished
                .into_iter()
                .map(|id| state.running.remove(&id).unwrap())
                .collect::<Vec<_>>();
            let done =
                state.running.is_empty() && (state.pending.is_empty() || state.failure.is_some());
            (finished, done)
        };
        let advanced = !finished.is_empty();
        for job in finished {
            if let Err(message) = job.wait() {
                scheduler.failed(message);
            }
        }
        if done {
            break;
        }
        if shared.deadline.is_some_and(|d| Instant::now() >= d) {
            scheduler.failed("SQL work graph timed out".into());
        }
        if !advanced && (!rerail_accel::exec::on_pool() || !rerail_accel::exec::help_once()) {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    let mut state = scheduler.state.lock();
    if let Some(failure) = state.failure.take() {
        return Err(error(failure));
    }
    let rows = std::mem::take(&mut state.exported);
    drop(state);
    Ok(ResultSet {
        rows,
        explain,
        submissions: shared.submitted.load(Ordering::Acquire),
        vm_steps: shared.steps.load(Ordering::Relaxed),
    })
}

/// Worker completions release successors even while the controller helps a long SQL job.
struct Scheduler {
    request: Request,
    plan: planner::Plan,
    graph: u64,
    explain: Value,
    shared: Arc<Shared>,
    requested: BTreeSet<String>,
    state: parking_lot::Mutex<Scheduled>,
}
struct Scheduled {
    values: BTreeMap<String, Arc<dyn Source>>,
    exported: BTreeMap<String, Arc<worker::Rows>>,
    pending: BTreeSet<usize>,
    completed: BTreeSet<usize>,
    running: BTreeMap<usize, rerail_accel::exec::Running>,
    failure: Option<String>,
}
impl Scheduler {
    fn enqueue(self: &Arc<Self>, state: &mut Scheduled) -> std::result::Result<(), String> {
        if self.shared.cancelled.load(Ordering::Acquire) {
            return Ok(());
        }
        let mut ready = state
            .pending
            .iter()
            .copied()
            .filter(|&id| {
                self.plan.groups[id]
                    .dependencies
                    .is_subset(&state.completed)
            })
            .collect::<Vec<_>>();
        ready.sort_by_key(|&id| std::cmp::Reverse(self.plan.groups[id].cost));
        for id in ready {
            let group = &self.plan.groups[id];
            let bindings = group
                .inputs
                .iter()
                .map(|name| {
                    state
                        .values
                        .get(name)
                        .cloned()
                        .map(|source| (name.clone(), source))
                        .ok_or_else(|| format!("SQL group {id} is missing input {name}"))
                })
                .collect::<std::result::Result<BTreeMap<_, _>, String>>()?;
            let launch = Launch::on(
                Kernel::Program(ProgramKernel(Arc::new(worker::Job {
                    id,
                    sql: group.sql(&self.request),
                    outputs: group.outputs.clone(),
                    bindings,
                    shared: self.shared.clone(),
                    graph: self.graph,
                    explain: self.explain["groups"][id].clone(),
                    units: group.cost.max(1) as f64,
                    scheduler: self.clone(),
                }))),
                Lane::Sql,
            );
            let running = super::super::logic::lanes::sql_executor()
                .begin_as("SQL read-only graph", Batch(vec![launch]))?;
            state.running.insert(id, running);
            state.pending.remove(&id);
        }
        // Running jobs own their inputs. Keep cut roots only for undispatched readers.
        state.values.retain(|name, _| {
            state
                .pending
                .iter()
                .any(|&id| self.plan.groups[id].inputs.contains(name))
        });
        Ok(())
    }
    fn finished(
        self: &Arc<Self>,
        id: usize,
        output: &worker::Results,
    ) -> std::result::Result<(), String> {
        let mut state = self.state.lock();
        for (name, rows) in &output.0 {
            state.values.insert(name.clone(), rows.clone());
            if self.requested.contains(name) {
                state.exported.insert(name.clone(), rows.clone());
            }
        }
        state.completed.insert(id);
        self.enqueue(&mut state)
    }
    fn failed(&self, message: String) {
        self.shared.cancelled.store(true, Ordering::Release);
        self.state.lock().failure.get_or_insert(message);
    }
}

/// Failure and unwinding still join every worker before the owner can change inputs.
struct Flights(Arc<Scheduler>);
impl Drop for Flights {
    fn drop(&mut self) {
        self.0.shared.cancelled.store(true, Ordering::Release);
        loop {
            let job = self.0.state.lock().running.pop_first().map(|(_, job)| job);
            let Some(job) = job else {
                break;
            };
            let _ = job.wait();
        }
    }
}
