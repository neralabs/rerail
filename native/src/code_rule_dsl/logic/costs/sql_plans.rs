//! SQL plan feedback in the shared costs database. The execution thread only uses memory.
use parking_lot::{Mutex, RwLock};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};

const VERSION: i64 = 3;
const PROFILE: &str = if cfg!(debug_assertions) {
    "debug"
} else {
    "optimized"
};
const QUEUE_LIMIT: usize = 256;
const WINDOW: usize = 32;
const COHORT_LIMIT: usize = 1024;
const MIN_SAMPLES: usize = 5;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Run,
    RunFile,
    ApplyChanges,
    ApplySnapshot,
    ApplyFileChanges,
}

impl Operation {
    fn name(self) -> &'static str {
        match self {
            Self::Run => "run",
            Self::RunFile => "run_file",
            Self::ApplyChanges => "apply_changes",
            Self::ApplySnapshot => "apply_snapshot",
            Self::ApplyFileChanges => "apply_file_changes",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Strategy {
    SparseRetained,
    SparseRetainedAfterBatchAttempt,
    FullWaveBatch,
    SignedWaveBatch,
    GroupedMaintenance,
    ColdSchedule,
    ColdScheduleAfterRetainedAttempt,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Completed,
    Failed,
    SubmissionLimit,
}

impl Outcome {
    fn name(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::SubmissionLimit => "submission_limit",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Inputs {
    pub program: String,
    pub rule_steps: usize,
    pub relations: usize,
    pub input_rows: u64,
    pub largest_input_rows: u64,
    pub changed_relations: usize,
    pub inserted_rows: u64,
    pub deleted_rows: u64,
    pub wave_limit: u32,
    pub available_submissions: Option<usize>,
    pub conditions: Conditions,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct Conditions {
    pub vm_limit: bool,
    pub time_limit: bool,
    pub detailed_profile: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct Metrics {
    pub submissions: usize,
    pub vm_steps: u64,
    pub owner_cpu_ns: Option<u64>,
    pub wall_ns: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Estimate {
    pub samples: usize,
    pub max_observed_submissions: Option<usize>,
    pub median_vm_steps: Option<u64>,
    pub median_owner_cpu_ns: Option<u64>,
    pub median_wall_ns: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Stage {
    pub inputs: Inputs,
    pub strategy: Strategy,
    pub forecast: Estimate,
    pub alternate_forecast: Option<Alternative>,
    pub metrics: Metrics,
    #[serde(default)]
    pub gate_statistics: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Alternative {
    pub strategy: Strategy,
    pub forecast: Estimate,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Report {
    pub operation: Operation,
    pub outcome: Outcome,
    pub metrics: Metrics,
    pub stages: Vec<Stage>,
    pub omitted_stages: usize,
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct Cohort {
    cold: bool,
    program: String,
    strategy: Strategy,
    rows: u32,
    largest_input: u32,
    changed_relations: usize,
    inserted: u32,
    deleted: u32,
    wave_limit: u32,
    conditions: Conditions,
}

impl Cohort {
    fn new(operation: Operation, inputs: &Inputs, strategy: Strategy) -> Self {
        Self {
            cold: matches!(operation, Operation::Run | Operation::RunFile),
            program: inputs.program.clone(),
            strategy,
            rows: u64::BITS - inputs.input_rows.leading_zeros(),
            largest_input: u64::BITS - inputs.largest_input_rows.leading_zeros(),
            changed_relations: inputs.changed_relations,
            inserted: u64::BITS - inputs.inserted_rows.leading_zeros(),
            deleted: u64::BITS - inputs.deleted_rows.leading_zeros(),
            wave_limit: inputs.wave_limit,
            conditions: inputs.conditions,
        }
    }
}

#[derive(Default)]
struct Model(HashMap<Cohort, Vec<((i64, usize), Metrics)>>);

impl Model {
    fn learn(&mut self, id: i64, report: &Report) {
        if report.outcome != Outcome::Completed {
            return;
        }
        for (index, stage) in report.stages.iter().enumerate() {
            let cohort = Cohort::new(report.operation, &stage.inputs, stage.strategy);
            if self.0.len() >= COHORT_LIMIT && !self.0.contains_key(&cohort) {
                let oldest = self
                    .0
                    .iter()
                    .min_by_key(|(_, samples)| samples.last().unwrap().0);
                if let Some((key, samples)) = oldest {
                    if samples.last().unwrap().0.0 >= id {
                        continue;
                    }
                    let key = key.clone();
                    self.0.remove(&key);
                }
            }
            let samples = self.0.entry(cohort).or_default();
            // One operation can contain several independent samples in the same cohort.
            if let Err(at) = samples.binary_search_by_key(&(id, index), |(seen, _)| *seen) {
                samples.insert(at, ((id, index), stage.metrics));
                if samples.len() > WINDOW {
                    samples.remove(0);
                }
            }
        }
    }
}

fn median<T: Ord + Copy>(values: impl Iterator<Item = T>) -> Option<T> {
    let mut values = values.collect::<Vec<_>>();
    let mid = values.len() / 2;
    (values.len() >= MIN_SAMPLES).then(|| *values.select_nth_unstable(mid).1)
}

pub fn estimate(operation: Operation, inputs: &Inputs, strategy: Strategy) -> Estimate {
    let model = MODEL.read();
    let samples = model
        .0
        .get(&Cohort::new(operation, inputs, strategy))
        .map_or(&[][..], Vec::as_slice);
    Estimate {
        samples: samples.len(),
        max_observed_submissions: (samples.len() >= MIN_SAMPLES)
            .then(|| samples.iter().map(|(_, m)| m.submissions).max().unwrap()),
        median_vm_steps: median(samples.iter().map(|(_, m)| m.vm_steps)),
        median_owner_cpu_ns: median(samples.iter().filter_map(|(_, m)| m.owner_cpu_ns)),
        median_wall_ns: median(samples.iter().map(|(_, m)| m.wall_ns)),
    }
}

static MODEL: LazyLock<RwLock<Model>> = LazyLock::new(RwLock::default);
static ENABLED: AtomicBool = AtomicBool::new(false);
static QUEUE: Mutex<VecDeque<Report>> = Mutex::new(VecDeque::new());
static DROPPED: AtomicU64 = AtomicU64::new(0);
static WRITE_ERRORS: AtomicU64 = AtomicU64::new(0);

pub fn enable() {
    ENABLED.store(true, Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Relaxed)
}

pub fn status() -> serde_json::Value {
    serde_json::json!({
        "enabled": enabled(),
        "dropped_reports": DROPPED.load(Relaxed),
        "write_errors": WRITE_ERRORS.load(Relaxed),
    })
}

pub fn submit(report: Report) {
    let mut queue = QUEUE.lock();
    if queue.len() == QUEUE_LIMIT {
        queue.pop_front();
        DROPPED.fetch_add(1, Relaxed);
    }
    queue.push_back(report);
}

pub fn take() -> VecDeque<Report> {
    std::mem::take(&mut *QUEUE.lock())
}

pub fn retry(mut reports: VecDeque<Report>) {
    if reports.is_empty() {
        return;
    }
    WRITE_ERRORS.fetch_add(1, Relaxed);
    let mut queue = QUEUE.lock();
    reports.append(&mut queue);
    let excess = reports.len().saturating_sub(QUEUE_LIMIT);
    reports.drain(..excess);
    DROPPED.fetch_add(excess as u64, Relaxed);
    *queue = reports;
}

pub fn create(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS sql_plan_observations(
           id INTEGER PRIMARY KEY, at REAL NOT NULL, machine TEXT NOT NULL,
           threads INTEGER NOT NULL, profile TEXT NOT NULL, version INTEGER NOT NULL,
           operation TEXT NOT NULL, outcome TEXT NOT NULL, report TEXT NOT NULL);
         CREATE INDEX IF NOT EXISTS sql_plan_observations_by_machine
           ON sql_plan_observations(machine, threads, profile, version, id);",
    )
}

pub fn insert(
    tx: &Transaction<'_>,
    machine: &str,
    threads: i64,
    at: f64,
    reports: &VecDeque<Report>,
) -> rusqlite::Result<Vec<i64>> {
    let mut ids = Vec::with_capacity(reports.len());
    let mut stmt = tx.prepare_cached(
        "INSERT INTO sql_plan_observations(at,machine,threads,profile,version,operation,outcome,report)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
    )?;
    for report in reports {
        let json = serde_json::to_string(report)
            .map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))?;
        ids.push(stmt.insert(params![
            at,
            machine,
            threads,
            PROFILE,
            VERSION,
            report.operation.name(),
            report.outcome.name(),
            json
        ])?);
    }
    if let (Some(first), Some(last)) = (ids.first(), ids.last())
        && (first - 1) / QUEUE_LIMIT as i64 != last / QUEUE_LIMIT as i64 {
            tx.execute(
                "DELETE FROM sql_plan_observations
                 WHERE machine=?1 AND threads=?2 AND profile=?3 AND version=?4 AND id < (
                   SELECT id FROM sql_plan_observations
                   WHERE machine=?1 AND threads=?2 AND profile=?3 AND version=?4
                   ORDER BY id DESC LIMIT 1 OFFSET 19999
                 )",
                params![machine, threads, PROFILE, VERSION],
            )?;
        }
    Ok(ids)
}

pub fn learn(ids: Vec<i64>, reports: &VecDeque<Report>) {
    let mut model = MODEL.write();
    for (id, report) in ids.into_iter().zip(reports) {
        model.learn(id, report);
    }
}

pub fn read(db: &Connection, machine: &str, threads: i64) -> rusqlite::Result<()> {
    let exists = db
        .query_row(
            "SELECT 1 FROM sqlite_schema WHERE type='table' AND name='sql_plan_observations'",
            [],
            |_| Ok(()),
        )
        .optional()?;
    if exists.is_none() {
        return Ok(());
    }
    let mut stmt = db.prepare(
        "SELECT id,report FROM sql_plan_observations
         WHERE machine=?1 AND threads=?2 AND profile=?3 AND version=?4
         ORDER BY id DESC LIMIT 2048",
    )?;
    let rows = stmt.query_map(params![machine, threads, PROFILE, VERSION], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut reports = Vec::new();
    for row in rows {
        let (id, json) = row?;
        if let Ok(report) = serde_json::from_str::<Report>(&json) {
            reports.push((id, report));
        }
    }
    let mut model = MODEL.write();
    for (id, report) in reports.into_iter().rev() {
        model.learn(id, &report);
    }
    Ok(())
}
