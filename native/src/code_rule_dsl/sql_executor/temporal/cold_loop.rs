//! A cold positive graph constructs its complete retained trajectory in one SQL submission.
//! SQLite evaluates signed context transitions and folds. Callbacks only build immutable
//! storage and schedule the next gate from frontier lengths; they never execute SQL.
use super::*;
mod compile;
mod execute;
mod rows;
mod storage;

const ROWS: &str = "__rr_retained_loop_rows";
const COLLECT: &str = "__rr_retained_loop_collect";
const GATE: &str = "__rr_retained_loop_gate";
const EMPTY: &str = "__rr_retained_loop_empty";
const CLEAR: &str = "__rr_retained_loop_clear";
const MERGE: &str = "__rr_retained_loop_merge";
const LIVE: &str = "__rr_retained_loop_live";

pub(super) fn candidate(e: &plan::emit::Emitted) -> bool {
    let recursive = e.program.steps.iter().any(|s| {
        s.stratum > 0
            && s.reads.iter().any(|r| {
                e.program
                    .steps
                    .iter()
                    .any(|p| p.output == *r && p.stratum == s.stratum)
            })
    });
    let shape = recursive
        && !e.program.steps.is_empty()
        && e.program.steps.len() <= 32
        && e.program.lattices.is_empty()
        && e.queries.iter().all(|q| {
            q.as_ref().is_some_and(|q| {
                q.ctes.is_empty() && !q.body.from.is_empty() && plan::acyclic::total(&q.body)
            })
        });
    if !shape {
        return false;
    }
    let steps = e
        .program
        .steps
        .iter()
        .cloned()
        .zip(e.queries.iter().cloned())
        .collect::<Vec<_>>();
    // The ordinary single-relation closure and walk kernels already avoid retained
    // context expansion. Preserve their placement until a cost proof selects this path.
    let dependency = components(&steps);
    if !dependency
        .recursive
        .iter()
        .any(|r| dependency.members[r].len() > 1)
    {
        return false;
    }
    let members = e.program.relations.iter().map(|(r, _)| r.clone()).collect();
    plan::acyclic::version_loop(&steps, &members, &BTreeSet::new())
        .decision
        .proven
}
fn sql_error(message: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::UserFunctionError(message.into().into())
}
#[derive(Clone)]
struct State {
    roots: Vec<versions::Version>,
    changed: Vec<usize>,
    initialized: Vec<bool>,
    empty_body: Vec<bool>,
    pending: BTreeSet<usize>,
    active: BTreeSet<usize>,
    ran: BTreeSet<usize>,
    waves: u32,
    publication: Value,
}
struct Operator {
    seen: Vec<(String, usize, usize)>, // logical input, previous cursor, current source
    comparison: (usize, usize),
}
struct Stage {
    template: phase::Template,
    snapshots: Vec<usize>,
    outputs: Vec<usize>,
    expansion: usize,
    arms: usize,
}
struct Plan {
    stages: Vec<Arc<Stage>>,
    fold: Arc<Stage>,
    differences: BTreeMap<usize, (usize, usize, bool)>,
    targets: BTreeMap<usize, String>,
    inputs: Vec<(usize, usize)>,
    operators: Vec<Operator>,
    contexts: Vec<reductions::Retained>,
    frontiers: BTreeMap<String, usize>,
    steps: Arc<Vec<(schedule::Step, Option<Query>)>>,
    after: BTreeSet<(u32, u32)>,
    limit: u32,
    sql: String,
    explain: Value,
}
impl Plan {
    fn stage(&self, id: i64) -> rusqlite::Result<Arc<Stage>> {
        if id == -1 {
            return Ok(self.fold.clone());
        }
        usize::try_from(id)
            .ok()
            .and_then(|id| self.stages.get(id))
            .cloned()
            .ok_or_else(|| sql_error("invalid retained loop stage"))
    }

    fn ready(&self, pending: &BTreeSet<usize>) -> BTreeSet<usize> {
        pending
            .iter()
            .copied()
            .filter(|&i| {
                !pending.iter().any(|&j| {
                    self.after
                        .contains(&(self.steps[i].0.stratum, self.steps[j].0.stratum))
                })
            })
            .collect()
    }
}
struct Private {
    parent: i64,
    roots: Vec<(usize, versions::Version)>,
    counts: BTreeMap<String, phase::Counts>,
    cleared: Vec<usize>,
}
struct Arena {
    plan: Arc<Plan>,
    states: Vec<State>,
    private: Vec<Private>,
    failure: Option<String>,
    storage_cpu_ns: Option<u128>,
    shared_roots: usize,
    retained_roots: usize,
    difference_state: Option<i64>,
    differences: BTreeMap<(usize, usize), (versions::Version, versions::Version)>,
    version_nodes: usize,
    context_rows: u64,
    difference_cpu_ns: Option<u128>,
}
impl Arena {
    fn state(&self, handle: i64) -> rusqlite::Result<&State> {
        usize::try_from(handle)
            .ok()
            .and_then(|i| self.states.get(i))
            .ok_or_else(|| sql_error("invalid retained loop state"))
    }
    fn root(&mut self, handle: i64, slot: usize) -> rusqlite::Result<versions::Version> {
        if handle < 0 {
            let private = self.private(handle)?;
            if let Some((_, root)) = private.roots.iter().find(|(id, _)| *id == slot) {
                return Ok(root.clone());
            }
            let parent = private.parent;
            return self.root(parent, slot);
        }
        self.state(handle)?;
        if let Some(&(current, seen, negative)) = self.plan.differences.get(&slot) {
            if self.difference_state != Some(handle) {
                self.differences.clear();
                self.difference_state = Some(handle);
            }
            if !self.differences.contains_key(&(current, seen)) {
                let _span = crate::code_rule_dsl::logic::profile::span(|| {
                    "retained loop version difference".into()
                });
                let cpu = crate::code_rule_dsl::sql_profile::thread_cpu_ns();
                let state = self.state(handle)?;
                let (plus, minus, visited) = state.roots[seen]
                    .difference_roots(&state.roots[current])
                    .map_err(|e| sql_error(e.to_string()))?;
                self.version_nodes += visited;
                self.differences.insert((current, seen), (plus, minus));
                self.difference_cpu_ns = self
                    .difference_cpu_ns
                    .zip(cpu.zip(crate::code_rule_dsl::sql_profile::thread_cpu_ns()))
                    .map(|(total, (start, end))| total + end.saturating_sub(start));
            }
            let (plus, minus) = &self.differences[&(current, seen)];
            Ok(if negative {
                minus.clone()
            } else {
                plus.clone()
            })
        } else {
            self.state(handle)?
                .roots
                .get(slot)
                .cloned()
                .ok_or_else(|| sql_error("invalid version slot"))
        }
    }
    fn live(&self, handle: i64) -> rusqlite::Result<bool> {
        let s = self.state(handle)?;
        if !s.pending.is_empty() && s.waves >= self.plan.limit {
            return Err(sql_error(format!(
                "no fixpoint in {} waves",
                self.plan.limit
            )));
        }
        Ok(!s.pending.is_empty() || !s.ran.is_empty())
    }
}

// Reserve the rest of the 100-statement complete-job target for owner work and publication.
// This phase slice is conservative admission for eligible cold graphs, not whole-session accounting.
pub(super) const HOST_PHASE_LIMIT: usize = 48;

pub(super) fn run(
    _trace: &mut Trace,
    _w: &mut Work<'_>,
    _store: &versions::Store,
    _limit: u32,
) -> Result<Option<u32>> {
    #[cfg(test)]
    let (trace, w, store, limit) = (_trace, _w, _store, _limit);
    #[cfg(test)]
    if FORCE_LOOP.get() && !FORCE_HOST.get() && supported(trace) {
        let frame = Frame::create(w, store, trace)?;
        let pending = (0..trace.steps.len()).collect();
        let mut diagnostics = vec![];
        let result = execute::run(trace, w, store, limit, frame, pending, 0, &mut diagnostics)?;
        plan::insert_many(
            w.db,
            "INSERT INTO temp.wave_delta(wave,rel,rows)",
            diagnostics,
        )
        .map_err(error)?;
        return Ok(result);
    }
    Ok(None)
}
fn supported(trace: &Trace) -> bool {
    candidate(&trace.emitted) && trace.producers.is_empty()
}
pub(super) fn resume(
    trace: &mut Trace,
    w: &mut Work<'_>,
    store: &versions::Store,
    pending: &BTreeSet<usize>,
    waves: u32,
    limit: u32,
    diagnostics: &mut Vec<Vec<SqlValue>>,
) -> Result<Option<u32>> {
    #[cfg(test)]
    if FORCE_HOST.get() {
        return Ok(None);
    }
    if !pending.is_empty() && waves >= limit {
        return Err(error(format!(
            "{}: no fixpoint in {limit} waves",
            trace.emitted.program.name
        )));
    }
    if !supported(trace) {
        return Ok(None);
    }
    let frame = trace.frames.last().unwrap().fork(store)?;
    execute::run(
        trace,
        w,
        store,
        limit,
        frame,
        pending.clone(),
        waves,
        diagnostics,
    )
}

#[cfg(test)]
mod tests;

#[cfg(test)]
thread_local! { static FORCE_HOST: std::cell::Cell<bool> = const {std::cell::Cell::new(false)}; }

#[cfg(test)]
thread_local! { static FORCE_LOOP: std::cell::Cell<bool> = const {std::cell::Cell::new(false)}; }
