//! Capture SQL planning inputs and outcomes at the operation and evaluation boundaries.
use super::*;
use crate::code_rule_dsl::logic::costs::sql_plans::{
    self, Alternative, Conditions, Estimate, Inputs, Metrics, Outcome, Report, Stage,
};
use crate::code_rule_dsl::sql_admission::Ledger;
use crate::code_rule_dsl::sql_profile::thread_cpu_ns;
use std::time::Instant;

use sql_plans::Operation as Kind;
pub use sql_plans::Strategy;

const STAGE_LIMIT: usize = 128;

struct Meter {
    wall: Instant,
    cpu: Option<u128>,
    submissions: usize,
    vm_steps: u64,
}

impl Meter {
    fn start(ledger: &Ledger) -> Self {
        let (submissions, vm_steps) = ledger.counters();
        Self {
            wall: Instant::now(),
            cpu: thread_cpu_ns(),
            submissions,
            vm_steps,
        }
    }

    fn finish(self, ledger: &Ledger) -> Metrics {
        let (submissions, vm_steps) = ledger.counters();
        Metrics {
            submissions: submissions.saturating_sub(self.submissions),
            vm_steps: vm_steps.saturating_sub(self.vm_steps),
            owner_cpu_ns: self
                .cpu
                .zip(thread_cpu_ns())
                .map(|(before, after)| after.saturating_sub(before) as u64),
            wall_ns: self.wall.elapsed().as_nanos() as u64,
        }
    }
}

pub struct Operation {
    kind: Kind,
    conditions: Conditions,
    ledger: Arc<Ledger>,
    meter: Meter,
    stages: Vec<Stage>,
    omitted_stages: usize,
}

pub struct Evaluation {
    operation: Kind,
    inputs: Inputs,
    selection: Option<Selection>,
    meter: Meter,
    gate_statistics: Vec<Value>,
}

struct Selection {
    strategy: Strategy,
    forecast: Estimate,
    alternate_forecast: Option<Alternative>,
}

impl Operation {
    pub fn start(name: &str, db: &Database, limits: &runtime::Limits) -> Option<Self> {
        if !sql_plans::enabled() {
            return None;
        }
        let kind = match name {
            "run" => Kind::Run,
            "run_file" => Kind::RunFile,
            "apply_changes" => Kind::ApplyChanges,
            "apply_snapshot" => Kind::ApplySnapshot,
            "apply_file_changes" => Kind::ApplyFileChanges,
            _ => return None,
        };
        let ledger = db.admission();
        Some(Self {
            kind,
            conditions: Conditions {
                vm_limit: limits.max_steps.is_some(),
                time_limit: limits.timeout_ms.is_some(),
                detailed_profile: db.profiler().is_some(),
            },
            meter: Meter::start(&ledger),
            ledger,
            stages: Vec::new(),
            omitted_stages: 0,
        })
    }

    pub fn evaluation(
        &mut self,
        run: &session::Execution,
        wave_limit: u32,
        mut rows: impl FnMut(&str) -> u64,
        mut delta: impl FnMut(&str) -> (u64, u64),
    ) -> Option<Evaluation> {
        if self.stages.len() == STAGE_LIMIT {
            self.omitted_stages += 1;
            return None;
        }
        let program = run.planning_digest.clone()?;
        let mut inputs = Inputs {
            program,
            rule_steps: run.emitted.program.steps.len(),
            relations: run.emitted.program.relations.len(),
            input_rows: 0,
            largest_input_rows: 0,
            changed_relations: 0,
            inserted_rows: 0,
            deleted_rows: 0,
            wave_limit,
            available_submissions: self.ledger.credits(),
            conditions: self.conditions,
        };
        for relation in &run.inputs {
            let n = rows(relation);
            let (inserted, deleted) = delta(relation);
            inputs.input_rows = inputs.input_rows.saturating_add(n);
            inputs.largest_input_rows = inputs.largest_input_rows.max(n);
            inputs.changed_relations += usize::from(inserted > 0 || deleted > 0);
            inputs.inserted_rows = inputs.inserted_rows.saturating_add(inserted);
            inputs.deleted_rows = inputs.deleted_rows.saturating_add(deleted);
        }
        Some(Evaluation {
            operation: self.kind,
            inputs,
            selection: None,
            meter: Meter::start(&self.ledger),
            gate_statistics: Vec::new(),
        })
    }

    pub fn record(&mut self, eval: Evaluation) {
        if let Some(selection) = eval.selection {
            self.stages.push(Stage {
                inputs: eval.inputs,
                strategy: selection.strategy,
                forecast: selection.forecast,
                alternate_forecast: selection.alternate_forecast,
                metrics: eval.meter.finish(&self.ledger),
                gate_statistics: eval.gate_statistics,
            });
        }
    }

    pub fn finish(self, ok: bool, exhausted: bool) -> Report {
        let outcome = match (ok, exhausted) {
            (true, _) => Outcome::Completed,
            (false, true) => Outcome::SubmissionLimit,
            (false, false) => Outcome::Failed,
        };
        Report {
            operation: self.kind,
            outcome,
            metrics: self.meter.finish(&self.ledger),
            stages: self.stages,
            omitted_stages: self.omitted_stages,
        }
    }
}

impl Evaluation {
    pub fn capture(&mut self, run: &session::Execution) {
        self.gate_statistics = run
            .temporal
            .values()
            .filter_map(temporal::Trace::planning_statistics)
            .collect();
    }
    pub fn choose(&mut self, strategy: Strategy, compare_retained: bool) {
        let alternate = match strategy {
            Strategy::SparseRetained if compare_retained => Some(Strategy::FullWaveBatch),
            Strategy::FullWaveBatch if compare_retained => Some(Strategy::SparseRetained),
            Strategy::SignedWaveBatch if compare_retained => Some(Strategy::FullWaveBatch),
            _ => None,
        };
        self.selection = Some(Selection {
            strategy,
            forecast: sql_plans::estimate(self.operation, &self.inputs, strategy),
            alternate_forecast: alternate.map(|strategy| Alternative {
                strategy,
                forecast: sql_plans::estimate(self.operation, &self.inputs, strategy),
            }),
        });
    }

    pub fn choose_cold_schedule(&mut self) {
        let strategy = if self.selection.is_some() {
            Strategy::ColdScheduleAfterRetainedAttempt
        } else {
            Strategy::ColdSchedule
        };
        self.choose(strategy, false);
    }
}

pub fn digest(
    program: &front::Program,
    dialect: &str,
    lattice_extensions: &BTreeMap<String, schedule::LatticeOperations>,
) -> String {
    // Hash the parsed program before SQL lowering assigns database-local atom IDs.
    let extensions = serde_json::to_string(lattice_extensions)
        .expect("registered lattice operations serialize to JSON");
    super::super::json::hex_digest(
        format!(
            "{dialect}\0{}\0{program:?}\0{extensions}",
            rusqlite::version_number()
        )
        .as_bytes(),
    )
}
