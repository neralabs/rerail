//! Execute a wide wave schedule as one read-only SQL statement. The recursive CTE
//! visits each active rule and relation fold in order. Callbacks only advance the
//! schedule, expose pinned inputs, and store the rows produced by SQL.
use super::*;
use crate::code_rule_dsl::facts::vtab::Modules;
use rusqlite::functions::{Aggregate, Context, FunctionFlags};

pub mod compile;
pub mod folds;
pub mod incremental;
pub mod runtime;
pub mod scalars;
pub mod state;
pub use runtime::Runtime;

use state::{Arena, Wave};

const ROWS: &str = "__rr_batch_rows";
const FRONTIER: &str = "__rr_batch_frontier";
const CANDIDATES: &str = "__rr_batch_candidates";
const CHANGES: &str = "__rr_batch_changes";
const REUSE: &str = "__rr_batch_reuse";
const NEXT: &str = "__rr_batch_next";
const COLLECT: &str = "__rr_batch_collect";
const PRIOR: &str = "__rr_batch_prior";
const FOLD_CHANGES: &str = "__rr_batch_fold_changes";

pub fn wide(trace_width: usize) -> bool {
    trace_width > 32
}

pub fn selected(trace_width: usize, w: &Work<'_>) -> bool {
    // A broad retained graph can use several submissions even in a single wave.
    // Keep the sparse context path when it has room to complete.
    wide(trace_width)
        && super::super::super::sql_admission::current()
            .is_some_and(|ledger| ledger.available() < trace_width.saturating_mul(2) + 40)
        && w.versions.is_some()
}

#[derive(Clone)]
pub struct History {
    plan: Arc<Plan>,
    waves: Arc<Vec<Wave>>,
    explanation: Value,
}

struct Rule {
    relation: usize,
    reads: Vec<usize>,
    external: BTreeSet<String>,
    reusable: bool,
    candidate: versions::Version,
    incremental: Option<incremental::Rule>,
    incremental_analysis: crate::code_rule_dsl::logic::analysis::Decision,
}
struct Output {
    name: String,
    spec: Relation,
    readers: Vec<usize>,
    empty: state::Roots,
}
struct Plan {
    rules: Vec<Rule>,
    outputs: Vec<Output>,
    after: BTreeSet<(u32, u32)>,
    strata: Vec<u32>,
    width: usize,
    sql: String,
    reuse_id: usize,
    fact_probe_anchors: usize,
    changes: Vec<(incremental::Input, incremental::VersionKind)>,
    fold_branches: Vec<Option<usize>>,
}

impl History {
    pub fn planning_statistics(&self) -> Value {
        let explain = &self.explanation;
        json!({
            "signed_rule_results":explain["signed_rule_results"],
            "full_rule_results":explain["evaluated_rule_results"],
            "reused_rule_results":explain["reused_rule_results"],
            "affected_heads":explain["affected_heads"],
            "version_difference_nodes":explain["version_difference_nodes"],
            "rule_decisions":explain["latest_rule_decisions"],
            "reused_folds":explain["reused_folds"],
            "signed_folds":explain["signed_folds"],
            "signed_scalar_results":explain["signed_scalar_results"],
            "fold_affected_keys":explain["fold_affected_keys"],
            "fold_decisions":explain["fold_decisions"],
            "preparation_cpu_ns":explain["preparation_cpu_ns"],
            "automatic_reprepares":explain["automatic_reprepares"],
        })
    }
    pub fn planning_strategy(&self) -> planning::Strategy {
        if self.plan.rules.iter().any(|r| r.incremental.is_some()) {
            planning::Strategy::SignedWaveBatch
        } else {
            planning::Strategy::FullWaveBatch
        }
    }
    pub fn explain(&self) -> &Value {
        &self.explanation
    }

    pub fn len(&self) -> usize {
        self.waves.len()
    }

    pub fn versions(&self) -> frame::Versions {
        let roots = self.waves.last().map(|w| w.roots.as_slice());
        let output = |get: fn(&state::Roots) -> &versions::Version| {
            self.plan
                .outputs
                .iter()
                .enumerate()
                .map(|(i, out)| {
                    let v = roots.map_or(&out.empty, |roots| &roots[i]);
                    (out.name.clone(), get(v).clone())
                })
                .collect()
        };
        frame::Versions {
            published: output(|r| &r.rows),
            frontiers: output(|r| &r.frontier),
            counters: self
                .plan
                .outputs
                .iter()
                .enumerate()
                .filter(|(_, o)| o.spec.widening())
                .map(|(i, out)| {
                    let v = roots.map_or(&out.empty, |roots| &roots[i]);
                    (out.name.clone(), v.counts.clone())
                })
                .collect(),
        }
    }

    pub fn prepare(trace: &Trace, db: &rusqlite::Connection) -> Result<Option<Self>> {
        let Some(plan) = compile::plan(trace, db)? else {
            return Ok(None);
        };
        Ok(Some(Self {
            plan: Arc::new(plan),
            waves: Arc::new(vec![]),
            explanation: json!({}),
        }))
    }

    pub fn replay(
        &mut self,
        w: &mut Work<'_>,
        changes: &BTreeMap<String, Delta>,
        limit: u32,
        preparing: bool,
    ) -> Result<u32> {
        let external = incremental::external(w, &self.plan, changes)?;
        let arena = Arc::new(parking_lot::Mutex::new(Arena::new(
            self.plan.clone(),
            self.waves.clone(),
            changes
                .iter()
                .filter(|(_, d)| d.inserted + d.deleted > 0)
                .map(|(n, _)| n.clone())
                .collect(),
            limit,
            external,
        )));
        let guard = {
            let mut runtime = w.pool.batch_runtime.lock();
            if runtime.is_none() {
                *runtime = Some(Runtime::new(w.db)?);
            }
            runtime.as_ref().unwrap().enter(arena.clone())?
        };
        let _context = w.profile_scope(|| {
            json!({
                "stage":"SQL wave batch",
                "rules":self.plan.rules.len(),
                "relations":self.plan.outputs.len(),
                "sqlBytes":self.plan.sql.len(),
            })
        });
        let preparation_cpu = crate::code_rule_dsl::sql_profile::thread_cpu_ns();
        let mut preparation_ns = None;
        let mut reprepares = 0;
        let execution = crate::code_rule_dsl::logic::lanes::sql_executor().local(
            rerail_accel::Lane::Sql,
            "SQL wave batch",
            0.0,
            || {
                let mut statement = w.db.prepare_cached(&self.plan.sql).map_err(error)?;
                preparation_ns = preparation_cpu
                    .zip(crate::code_rule_dsl::sql_profile::thread_cpu_ns())
                    .map(|(start, end)| end.saturating_sub(start) as u64);
                let before = statement.get_status(rusqlite::StatementStatus::VmStep);
                let reprepare_before = statement.get_status(rusqlite::StatementStatus::RePrepare);
                let _: i64 = statement.query_row([], |row| row.get(0)).map_err(error)?;
                reprepares = statement
                    .get_status(rusqlite::StatementStatus::RePrepare)
                    .saturating_sub(reprepare_before);
                Ok::<_, napi::Error>(
                    statement
                        .get_status(rusqlite::StatementStatus::VmStep)
                        .saturating_sub(before),
                )
            },
        );
        if let Some(profiler) = w.profiler {
            profiler.explain_pending(w.db);
        }
        drop(guard);
        let vm_steps = execution?;
        let mut arena = arena.lock();
        if let Some(failure) = arena.failure.take() {
            return Err(error(failure));
        }
        let waves = arena.waves.len() as u32;
        if preparing {
            // Preserve the wave diagnostics using one bulk publication after the statement.
            let mut diagnostic = vec![];
            for (at, wave) in arena.waves.iter().enumerate() {
                for &id in &wave.outputs {
                    diagnostic.push(vec![
                        SqlValue::Integer(at as i64 + 1),
                        SqlValue::Text(self.plan.outputs[id].name.clone()),
                        SqlValue::Integer(wave.roots[id].frontier.len() as i64),
                    ]);
                }
            }
            plan::insert_many(
                w.db,
                "INSERT INTO temp.wave_delta(wave,rel,rows)",
                diagnostic,
            )
            .map_err(error)?;
        }
        self.explanation = json!({
            "strategy":"SQL wave batch",
            "executor":"sql",
            "planned_submissions":1,
            "scope":if preparing {"cold"} else {"warm"},
            "waves":waves,
            "rule_stages":self.plan.rules.len(),
            "relation_folds":self.plan.outputs.len(),
            "fact_probe_anchors":self.plan.fact_probe_anchors,
            "sql_bytes":self.plan.sql.len(),
            "vm_steps":vm_steps,
            "preparation_cpu_ns":preparation_ns,
            "automatic_reprepares":reprepares,
            "runtime_registration":"Connection lifetime with cleared operation bindings",
            "reused_rule_results":arena.reused,
            "evaluated_rule_results":arena.evaluated,
            "signed_rule_results":arena.reconciled,
            "affected_heads":arena.affected_heads,
            "reused_folds":arena.reused_folds,
            "signed_folds":arena.signed_folds,
            "signed_scalar_results":arena.signed_scalars,
            "fold_affected_keys":arena.fold_keys,
            "fold_decisions":arena.fold_decisions,
            "fold_analysis":self.plan.outputs.iter().map(|out|json!({
                "relation":out.name,"key_columns":out.spec.keys,
                "analysis":crate::code_rule_dsl::logic::analysis::deltas::reconcile_fold(true,true),
            })).collect::<Vec<_>>(),
            "version_difference_nodes":arena.version_nodes,
            "latest_rule_decisions":arena.decisions,
            "signed_projection_analysis":self.plan.rules.iter().enumerate().map(|(id,rule)|
                json!({"rule":id,"analysis":rule.incremental_analysis})).collect::<Vec<_>>(),
            "reuse_proof":"Equal preceding-wave roots and unchanged external inputs",
        });
        w.version_nodes += arena.version_nodes;
        self.waves = Arc::new(std::mem::take(&mut arena.waves));
        Ok(waves)
    }
}

fn sql_error(message: impl ToString) -> rusqlite::Error {
    rusqlite::Error::UserFunctionError(message.to_string().into())
}

struct Collect(runtime::Active);
#[derive(Default)]
struct ResultRows {
    state: i64,
    stage: usize,
    rows: Vec<Vec<Option<i64>>>,
    keys: Vec<Vec<Option<i64>>>,
    counts: Vec<Vec<Option<i64>>>,
}

impl Aggregate<ResultRows, i64> for Collect {
    fn init(&self, _: &mut Context<'_>) -> rusqlite::Result<ResultRows> {
        Ok(ResultRows::default())
    }
    fn step(&self, c: &mut Context<'_>, out: &mut ResultRows) -> rusqlite::Result<()> {
        out.state = c.get(0)?;
        out.stage =
            usize::try_from(c.get::<i64>(1)?).map_err(|_| sql_error("invalid SQL wave stage"))?;
        let tag = c.get::<i64>(2)?;
        if tag != 0 {
            let row = (3..c.len())
                .map(|i| c.get(i))
                .collect::<rusqlite::Result<_>>()?;
            match tag {
                1 => out.rows.push(row),
                2 => out.keys.push(row),
                3 => out.counts.push(row),
                _ => return Err(sql_error("invalid signed projection row tag")),
            }
        }
        Ok(())
    }
    fn finalize(&self, _: &mut Context<'_>, out: Option<ResultRows>) -> rusqlite::Result<i64> {
        let out = out.ok_or_else(|| sql_error("missing SQL wave sentinel"))?;
        self.0
            .arena()?
            .lock()
            .accept(out.state, out.stage, out.rows, out.keys, out.counts)
    }
}

#[cfg(test)]
pub mod tests;
