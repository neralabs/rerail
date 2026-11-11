//! Execute a wide wave schedule as one read-only SQL statement. The recursive CTE
//! visits each active rule and relation fold in order. Callbacks only advance the
//! schedule, expose pinned inputs, and store the rows produced by SQL.
use super::*;
use crate::code_rule_dsl::facts::vtab::Modules;
use rusqlite::functions::{Aggregate, Context, FunctionFlags};

mod compile;
mod state;

use state::{Arena, Wave};

const ROWS: &str = "__rr_batch_rows";
const FRONTIER: &str = "__rr_batch_frontier";
const CANDIDATES: &str = "__rr_batch_candidates";
const REUSE: &str = "__rr_batch_reuse";
const NEXT: &str = "__rr_batch_next";
const COLLECT: &str = "__rr_batch_collect";

pub(super) fn wide(trace_width: usize) -> bool {
    trace_width > 32
}

pub(super) fn selected(trace_width: usize, w: &Work<'_>) -> bool {
    // A broad retained graph can use several submissions even in a single wave.
    // Keep the sparse context path when it has room to complete.
    wide(trace_width)
        && super::super::super::sql_admission::current()
            .is_some_and(|ledger| ledger.available() < trace_width.saturating_mul(2) + 40)
        && w.versions.is_some()
}

#[derive(Clone)]
pub(super) struct History {
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
}

impl History {
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
        let arena = Arc::new(parking_lot::Mutex::new(Arena::new(
            self.plan.clone(),
            self.waves.clone(),
            changes
                .iter()
                .filter(|(_, d)| d.inserted + d.deleted > 0)
                .map(|(n, _)| n.clone())
                .collect(),
            limit,
        )));
        let modules = Modules::default();
        let _context = w.profile_scope(|| {
            json!({
                "stage":"SQL wave batch",
                "rules":self.plan.rules.len(),
                "relations":self.plan.outputs.len(),
                "sqlBytes":self.plan.sql.len(),
            })
        });
        let execution = (|| {
            for (name, estimate) in [(ROWS, 4096), (FRONTIER, 16), (CANDIDATES, 128)] {
                modules
                    .bind(
                        w.db,
                        name,
                        Arc::new(super::cold_loop::rows::Rows(
                            arena.clone(),
                            self.plan.width,
                            Some(estimate),
                        )),
                    )
                    .map_err(error)?;
            }
            let next = arena.clone();
            w.db.create_scalar_function(NEXT, 1, FunctionFlags::SQLITE_UTF8, move |c| {
                next.lock().next(c.get(0)?)
            })
            .map_err(error)?;
            let reuse = arena.clone();
            w.db.create_scalar_function(REUSE, 1, FunctionFlags::SQLITE_UTF8, move |c| {
                reuse.lock().reuse(c.get(0)?)
            })
            .map_err(error)?;
            w.db.create_aggregate_function(
                COLLECT,
                -1,
                FunctionFlags::SQLITE_UTF8,
                Collect(arena.clone()),
            )
            .map_err(error)?;
            crate::code_rule_dsl::logic::lanes::sql_executor().local(
                rerail_accel::Lane::Sql,
                "SQL wave batch",
                0.0,
                || {
                    let mut statement = w.db.prepare(&self.plan.sql).map_err(error)?;
                    let _: i64 = statement.query_row([], |row| row.get(0)).map_err(error)?;
                    Ok::<_, napi::Error>(statement.get_status(rusqlite::StatementStatus::VmStep))
                },
            )
        })();
        if let Some(profiler) = w.profiler {
            profiler.explain_pending(w.db);
        }
        let cleanup = [
            modules.unregister(w.db, ROWS),
            modules.unregister(w.db, FRONTIER),
            modules.unregister(w.db, CANDIDATES),
            w.db.remove_function(NEXT, 1),
            w.db.remove_function(REUSE, 1),
            w.db.remove_function(COLLECT, -1),
        ];
        let vm_steps = execution?;
        for result in cleanup {
            result.map_err(error)?;
        }
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
            "reused_rule_results":arena.reused,
            "evaluated_rule_results":arena.evaluated,
            "reuse_proof":"Equal preceding-wave roots and unchanged external inputs",
        });
        self.waves = Arc::new(std::mem::take(&mut arena.waves));
        Ok(waves)
    }
}

fn sql_error(message: impl ToString) -> rusqlite::Error {
    rusqlite::Error::UserFunctionError(message.to_string().into())
}

struct Collect(Arc<parking_lot::Mutex<Arena>>);
#[derive(Default)]
struct ResultRows {
    state: i64,
    stage: usize,
    rows: Vec<Vec<Option<i64>>>,
}

impl Aggregate<ResultRows, i64> for Collect {
    fn init(&self, _: &mut Context<'_>) -> rusqlite::Result<ResultRows> {
        Ok(ResultRows::default())
    }
    fn step(&self, c: &mut Context<'_>, out: &mut ResultRows) -> rusqlite::Result<()> {
        out.state = c.get(0)?;
        out.stage =
            usize::try_from(c.get::<i64>(1)?).map_err(|_| sql_error("invalid SQL wave stage"))?;
        if c.get::<bool>(2)? {
            out.rows.push(
                (3..c.len())
                    .map(|i| c.get(i))
                    .collect::<rusqlite::Result<_>>()?,
            );
        }
        Ok(())
    }
    fn finalize(&self, _: &mut Context<'_>, out: Option<ResultRows>) -> rusqlite::Result<i64> {
        let out = out.ok_or_else(|| sql_error("missing SQL wave sentinel"))?;
        self.0.lock().accept(out.state, out.stage, out.rows)
    }
}

#[cfg(test)]
mod tests;
