//! Development comparison of component scheduling and one recursive SQL statement.
//! Both strategies use the same SQL transition and immutable native storage.
//! This does not replace retained incremental maintenance.
use super::*;
use crate::code_rule_dsl::facts::vtab::Modules;
use crate::code_rule_dsl::logic::analysis::stages::{Components, loops::Analysis};
use crate::code_rule_dsl::logic::sql::select::Query;
use rusqlite::functions::FunctionFlags;
pub mod arena;

struct Plan {
    analysis: Arc<Analysis>,
    members: Vec<String>,
    widths: Vec<usize>,
    steps: Vec<(schedule::Step, Query)>,
}
fn eligibility(e: &plan::emit::Emitted, members: &BTreeSet<String>) -> (Analysis, Option<Plan>) {
    let steps = e
        .program
        .steps
        .iter()
        .cloned()
        .zip(e.queries.iter().cloned())
        .collect::<Vec<_>>();
    let lattices = e
        .program
        .lattices
        .iter()
        .map(|(r, _, _)| r.clone())
        .collect();
    let analysis = plan::acyclic::version_loop(&steps, members, &lattices);
    let plan = analysis.decision.proven.then(|| Plan {
        analysis: Arc::new(analysis.clone()),
        members: members.iter().cloned().collect(),
        widths: members
            .iter()
            .map(|r| {
                e.program
                    .relations
                    .iter()
                    .find(|(name, _)| name == r)
                    .unwrap()
                    .1
            })
            .collect(),
        steps: steps
            .into_iter()
            .filter(|(s, _)| members.contains(&s.output))
            .map(|(s, q)| (s, q.unwrap()))
            .collect(),
    });
    (analysis, plan)
}
const ROWS: &str = "__rr_component_rows";
const COLLECT: &str = "__rr_component_collect";
const ACTIVE: &str = "__rr_component_active";
fn columns(alias: &str, width: usize) -> String {
    (0..width)
        .map(|c| {
            if alias.is_empty() {
                format!("c{c}")
            } else {
                format!("{alias}.c{c}")
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}
fn source(state: &str, relation: usize, width: usize, frontier: bool) -> String {
    format!(
        "(SELECT {} FROM {ROWS} WHERE rr_state={state} AND rr_relation={relation} AND rr_frontier={})",
        columns("", width),
        i32::from(frontier)
    )
}
impl Plan {
    fn transition(&self) -> String {
        let positions = self
            .members
            .iter()
            .enumerate()
            .map(|(id, r)| (r.clone(), id))
            .collect::<BTreeMap<_, _>>();
        let mut candidates = vec![vec![]; self.members.len()];
        for (step, original) in &self.steps {
            let mut query = original.clone();
            let deltas = query
                .reads()
                .iter()
                .filter_map(|r| match r {
                    select::Relation::Derived { rel, delta: true } => positions.get(rel).copied(),
                    _ => None,
                })
                .collect::<BTreeSet<_>>();
            for block in query
                .ctes
                .iter_mut()
                .flat_map(|c| &mut c.arms)
                .chain([&mut query.body])
            {
                block.relations_mut(&mut |r| {
                    if let select::Relation::Derived { rel, delta } = r
                        && let Some(&id) = positions.get(rel)
                    {
                        *r = select::Relation::Bound {
                            rel: rel.clone(),
                            table: source("__rr_parent.state", id, self.widths[id], *delta),
                        };
                    }
                });
            }
            let enabled = if deltas.is_empty() {
                "__rr_parent.iteration=0".to_owned()
            } else {
                deltas
                    .iter()
                    .map(|&id| {
                        format!(
                            "EXISTS(SELECT 1 FROM {})",
                            source("__rr_parent.state", id, self.widths[id], true)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" OR ")
            };
            candidates[positions[&step.output]].push(format!(
                "SELECT {} FROM ({}) WHERE ({enabled})",
                columns("", self.widths[positions[&step.output]]),
                query.render()
            ));
        }
        let mut definitions = vec![];
        let width = *self.widths.iter().max().unwrap();
        let mut tagged = vec![format!(
            "SELECT -1 AS rr_tag,{}",
            (0..width)
                .map(|c| format!("NULL AS c{c}"))
                .collect::<Vec<_>>()
                .join(",")
        )];
        for (id, arms) in candidates.iter().enumerate() {
            let width = self.widths[id];
            definitions.push(format!(
                "rr_candidates_{id} AS MATERIALIZED ({})",
                arms.join(" UNION ")
            ));
            let same = (0..width)
                .map(|c| format!("old.c{c}=candidate.c{c}"))
                .collect::<Vec<_>>()
                .join(" AND ");
            let present = (0..width)
                .map(|c| format!("candidate.c{c} IS NOT NULL"))
                .collect::<Vec<_>>()
                .join(" AND ");
            definitions.push(format!("rr_delta_{id} AS MATERIALIZED (SELECT DISTINCT {} FROM rr_candidates_{id} candidate WHERE {present} AND NOT EXISTS(SELECT 1 FROM {} old WHERE {same}))",columns("candidate",width),source("__rr_parent.state",id,width,false)));
            let values = (0..*self.widths.iter().max().unwrap())
                .map(|c| {
                    if c < width {
                        format!("c{c}")
                    } else {
                        "NULL".into()
                    }
                })
                .collect::<Vec<_>>()
                .join(",");
            tagged.push(format!("SELECT {id},{values} FROM rr_delta_{id}"));
        }
        format!(
            "(WITH {} SELECT {COLLECT}(__rr_parent.state,rr_tag,{}) FROM ({}))",
            definitions.join(","),
            columns("", width),
            tagged.join(" UNION ALL ")
        )
    }
}
fn cpu() -> Option<u128> {
    crate::code_rule_dsl::sql_profile::thread_cpu_ns()
}
fn elapsed(start: Option<u128>) -> Option<u128> {
    start
        .zip(cpu())
        .map(|(start, end)| end.saturating_sub(start))
}
struct Outcome {
    roots: Vec<versions::Version>,
    metrics: Value,
}
fn execute(db: &Database, plan: &Plan, sql_loop: bool, limit: u32) -> Result<Outcome> {
    if limit == 0 {
        return Err(error("component comparison: no fixpoint in 0 waves"));
    }
    let arena = Arc::new(parking_lot::Mutex::new(arena::Arena::new(
        plan.widths.clone(),
    )));
    let modules = Modules::default();
    let width = *plan.widths.iter().max().unwrap();
    let flags = FunctionFlags::SQLITE_UTF8;
    let start = cpu();
    let result = (|| {
        modules
            .bind(
                db,
                ROWS,
                Arc::new(arena::Rows {
                    arena: arena.clone(),
                    width,
                }),
            )
            .map_err(error)?;
        db.create_aggregate_function(
            COLLECT,
            width as i32 + 2,
            flags,
            arena::Collect(arena.clone()),
        )
        .map_err(error)?;
        let active = arena.clone();
        db.create_scalar_function(ACTIVE, 1, flags, move |context| {
            active.lock().active(context.get(0)?)
        })
        .map_err(error)?;
        let transition = plan.transition();
        let sql = if sql_loop {
            format!(
                "WITH RECURSIVE rr_states(iteration,state) AS (VALUES(0,?1) UNION ALL SELECT __rr_parent.iteration+1,{transition} FROM rr_states __rr_parent WHERE __rr_parent.iteration<?2 AND (__rr_parent.iteration=0 OR {ACTIVE}(__rr_parent.state))) SELECT iteration,state,{ACTIVE}(state) FROM rr_states ORDER BY iteration DESC LIMIT 1"
            )
        } else {
            format!(
                "WITH rr_next AS MATERIALIZED (SELECT {transition} AS state FROM (SELECT ?1 AS state,?2 AS iteration) __rr_parent) SELECT state,{ACTIVE}(state) FROM rr_next"
            )
        };
        let _context=db.profile_scope(||json!({"stage":"recursive component comparison","strategy":if sql_loop{"SQL component loop"}else{"host component loop"},"component":plan.members,"logicalAnalysis":plan.analysis.as_ref()}));
        let prepare = cpu();
        let before = db.profiler().map(|p| p.totals());
        let mut statement = db.prepare(&sql).map_err(error)?;
        let preparation_cpu_ns = elapsed(prepare);
        let (mut state, mut iterations, mut submissions) = (0i64, 0u32, 0u32);
        if sql_loop {
            let (count, handle, active) = statement
                .query_row(rusqlite::params![state, limit], |row| {
                    Ok((
                        row.get::<_, u32>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, bool>(2)?,
                    ))
                })
                .map_err(error)?;
            if active {
                return Err(error(format!(
                    "component comparison: no fixpoint in {limit} waves"
                )));
            }
            state = handle;
            iterations = count;
            submissions = 1;
        } else {
            loop {
                let (handle, active) = statement
                    .query_row(rusqlite::params![state, iterations], |row| {
                        Ok((row.get::<_, i64>(0)?, row.get::<_, bool>(1)?))
                    })
                    .map_err(error)?;
                state = handle;
                iterations += 1;
                submissions += 1;
                if !active {
                    break;
                }
                if iterations >= limit {
                    return Err(error(format!(
                        "component comparison: no fixpoint in {limit} waves"
                    )));
                }
            }
        }
        let vm_steps = before
            .map(|before| db.profiler().unwrap().totals()[2].saturating_sub(before[2]))
            .or_else(|| {
                u64::try_from(statement.get_status(rusqlite::StatementStatus::VmStep))
                    .ok()
                    .filter(|&steps| steps > 0)
            });
        drop(statement);
        let data = arena.lock();
        let roots = data.state(state).map_err(error)?.full.clone();
        let produced = data
            .produced
            .iter()
            .map(|rows| rows.iter().sum::<usize>())
            .collect::<Vec<_>>();
        Ok(Outcome {
            roots,
            metrics: json!({"strategy":if sql_loop{"SQL component loop"}else{"host component loop"},"submissions":submissions,"iterations_including_empty_detection":iterations,"new_rows_per_iteration":produced,"storage_cpu_ns":data.storage_cpu_ns,"preparation_cpu_ns":preparation_cpu_ns,"total_cpu_ns":elapsed(start),"vm_steps":vm_steps,"sql_bytes":sql.len(),"retained_states":data.states.len()}),
        })
    })();
    // Deferred explanations need the native schema and functions while they still exist.
    if let Some(profiler) = db.profiler() {
        profiler.explain_pending(db);
    }
    // No SQL statement can remain prepared while its native functions are removed.
    let cleanup = [
        modules.unregister(db, ROWS),
        db.remove_function(COLLECT, width as i32 + 2),
        db.remove_function(ACTIVE, 1),
    ]
    .into_iter()
    .collect::<rusqlite::Result<Vec<_>>>()
    .map(|_| ())
    .map_err(error);
    match result {
        Ok(outcome) => cleanup.map(|_| outcome),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
pub mod tests;

pub fn compare(
    db: &Database,
    run: &session::Execution,
    limit: u32,
    sql_first: bool,
) -> Result<Value> {
    let e = &run.emitted;
    let mut edges = BTreeMap::<String, BTreeSet<String>>::new();
    for (step, query) in e.program.steps.iter().zip(&e.queries) {
        edges
            .entry(step.output.clone())
            .or_default()
            .extend(plan::acyclic::reads(step, query.as_ref()));
    }
    let components = Components::analyze(&edges);
    let groups = components
        .recursive
        .iter()
        .map(|r| components.members[r].clone())
        .collect::<BTreeSet<_>>();
    let mut results = vec![];
    for members in groups {
        let (proof, plan) = eligibility(e, &members);
        let Some(plan) = plan else {
            results.push(
                json!({"component":members,"eligibility":proof.decision,"logical_analysis":proof}),
            );
            continue;
        };
        let (host, sql) = if sql_first {
            let sql = execute(db, &plan, true, limit)?;
            let host = execute(db, &plan, false, limit)?;
            (host, sql)
        } else {
            (
                execute(db, &plan, false, limit)?,
                execute(db, &plan, true, limit)?,
            )
        };
        let mut counts = BTreeMap::new();
        for (id, relation) in plan.members.iter().enumerate() {
            let query = format!(
                "SELECT * FROM {} ORDER BY {}",
                quote(relation),
                (1..=plan.widths[id])
                    .map(|c| c.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            );
            let mut statement = db.prepare(&query).map_err(error)?;
            let expected = statement
                .query_map([], |row| {
                    (0..plan.widths[id])
                        .map(|c| row.get(c))
                        .collect::<rusqlite::Result<Vec<Option<i64>>>>()
                })
                .map_err(error)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(error)?;
            if host.roots[id].rows() != expected || sql.roots[id].rows() != expected {
                return Err(error(format!(
                    "{relation}: component strategies differ from the completed session"
                )));
            }
            counts.insert(relation, expected.len());
        }
        results.push(json!({"component":members,"eligibility":proof.decision,"logical_analysis":proof,"parity":true,"rows":counts,"validation_submissions":plan.members.len(),"host":host.metrics,"sql":sql.metrics}));
    }
    Ok(
        json!({"scope":"Component-only comparison after a complete session; external inputs remain fixed at completed versions","production_incremental_replacement":false,"strategy_order":if sql_first{"SQL then host"}else{"host then SQL"},"components":results}),
    )
}
