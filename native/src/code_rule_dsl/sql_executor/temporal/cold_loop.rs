//! A cold positive graph constructs its complete retained trajectory in one SQL submission.
//! SQLite evaluates signed context transitions and folds. Callbacks only build immutable
//! storage and schedule the next gate from frontier lengths; they never execute SQL.
use super::*;
mod compile;
mod execute;
pub(super) mod rows;
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
    journals: Vec<usize>,
}
struct Stage {
    template: phase::Template,
    snapshots: Vec<usize>,
    outputs: Vec<usize>,
    expansion: usize,
    arms: usize,
}
struct Plan {
    stages: BTreeMap<usize, Arc<Stage>>,
    fold: Arc<Stage>,
    differences: BTreeMap<usize, (usize, usize, bool)>,
    targets: BTreeMap<usize, String>,
    inputs: Vec<(usize, usize)>,
    operators: BTreeMap<usize, Operator>,
    contexts: Vec<reductions::Retained>,
    frontiers: BTreeMap<String, usize>,
    steps: Arc<Vec<(schedule::Step, Option<Query>)>>,
    after: BTreeSet<(u32, u32)>,
    pipeline: BTreeSet<usize>,
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
            .and_then(|id| self.stages.get(&id))
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
    fn pipeline_ready(&self, pending: &BTreeSet<usize>, ready: &BTreeSet<usize>) -> bool {
        pending.difference(ready).any(|&id| {
            self.pipeline.contains(&id)
                && pending.iter().all(|&j| {
                    !self
                        .after
                        .contains(&(self.steps[id].0.stratum, self.steps[j].0.stratum))
                        || ready.contains(&j) && self.pipeline.contains(&j)
                })
        })
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
        // Return to the owner before evaluating any stage outside this checked segment.
        Ok(!self.plan.pipeline_ready(&s.pending, &s.active)
            && s.active.iter().all(|id| self.plan.stages.contains_key(id))
            && (!s.pending.is_empty() || !s.ran.is_empty()))
    }
}

// Keep short or sparse host replays on their cached statements.
pub(super) const HOST_PHASE_LIMIT: usize = 48;

pub(super) struct Progress {
    pub waves: u32,
    pub pending: BTreeSet<usize>,
    pub needs_close: bool,
}

/// Admit entire checked set components. Unsupported components remain in the same
/// trace and will see the same preceding-wave versions at their original gates.
fn selection(
    trace: &Trace,
    frame: &Frame,
    store: &versions::Store,
    ready: &BTreeSet<usize>,
) -> BTreeSet<usize> {
    let mut groups = BTreeMap::<&BTreeSet<String>, Vec<usize>>::new();
    for (id, (step, _)) in trace.steps.iter().enumerate() {
        groups
            .entry(&trace.components.members[&step.output])
            .or_default()
            .push(id);
    }
    let lattices = trace
        .relations
        .iter()
        .filter(|(_, spec)| spec.lattice)
        .map(|(name, _)| name.clone())
        .collect();
    let mut selected = BTreeSet::new();
    let mut active_components = BTreeSet::new();
    let mut recursive = false;
    let mut active_recursive = false;
    for (members, ids) in groups {
        if ids.iter().all(|&id| {
            let op = &frame.operators[id];
            let (_, query) = &trace.steps[id];
            !op.sql_kernel
                && op.opaque.is_none()
                && op.phase_eligible
                && query.as_ref().is_some_and(|q| q.ctes.is_empty())
                && op.run.reductions.as_ref().is_some_and(|states| {
                    states.values().all(|s| {
                        s.version_loop_eligible()
                            || op.ready
                                && s.version_loop_frozen_scalars(|input| {
                                    op.pending.get(input).is_some_and(|journal| {
                                        store.version_at(frame.scope(), &journal.plus).len() == 0
                                            && store.version_at(frame.scope(), &journal.minus).len()
                                                == 0
                                    })
                                })
                    })
                })
        }) && plan::acyclic::version_loop(&trace.steps, members, &lattices)
            .decision
            .proven
        {
            recursive |= members.len() > 1;
            if ids
                .iter()
                .any(|id| ready.contains(id) || frame.operators[*id].active)
            {
                active_recursive |= members.len() > 1;
                active_components.extend(ids.iter().copied());
            }
            selected.extend(ids);
        }
    }
    if recursive && selected.len() <= 32 {
        selected
    } else if active_recursive && active_components.len() <= 64 {
        active_components
    } else {
        BTreeSet::new()
    }
}

pub(super) fn run(
    _trace: &mut Trace,
    _w: &mut Work<'_>,
    _store: &versions::Store,
    _limit: u32,
) -> Result<Option<u32>> {
    #[cfg(test)]
    let (trace, w, store, limit) = (_trace, _w, _store, _limit);
    #[cfg(test)]
    if FORCE_LOOP.get()
        && !FORCE_HOST.get()
        && candidate(&trace.emitted)
        && trace.producers.is_empty()
    {
        let frame = Frame::create(w, store, trace)?;
        let pending = (0..trace.steps.len()).collect();
        let selected = selection(trace, &frame, store, &pending);
        if selected.len() == trace.steps.len() {
            let mut diagnostics = vec![];
            let result = execute::run(
                trace,
                w,
                store,
                limit,
                frame,
                selected,
                pending,
                0,
                0,
                true,
                &mut diagnostics,
            )?;
            plan::insert_many(
                w.db,
                "INSERT INTO temp.wave_delta(wave,rel,rows)",
                diagnostics,
            )
            .map_err(error)?;
            return Ok(result.map(|p| p.waves));
        }
        store.discard_scope(w.db, frame.scope())?;
    }
    Ok(None)
}

pub(super) fn resume(
    trace: &mut Trace,
    w: &mut Work<'_>,
    store: &versions::Store,
    pending: &BTreeSet<usize>,
    at: usize,
    waves: u32,
    limit: u32,
    preparing: bool,
    diagnostics: &mut Vec<Vec<SqlValue>>,
) -> Result<Option<Progress>> {
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
    let base = &trace.frames[at - 1];
    let ready = trace.ready(pending);
    let selected = selection(trace, base, store, &ready);
    if selected.is_empty()
        || !ready.is_subset(&selected)
        || base
            .operators
            .iter()
            .enumerate()
            .any(|(id, op)| op.active && !selected.contains(&id))
    {
        return Ok(None);
    }
    if ready.iter().any(|&id| {
        ready.iter().any(|&j| {
            trace
                .after
                .contains(&(trace.steps[id].0.stratum, trace.steps[j].0.stratum))
        })
    }) {
        return Ok(None);
    }
    let frame = base.fork(store)?;
    execute::run(
        trace,
        w,
        store,
        limit,
        frame,
        selected,
        pending.clone(),
        at,
        waves,
        preparing,
        diagnostics,
    )
}

#[cfg(test)]
mod tests;

#[cfg(test)]
thread_local! { static FORCE_HOST: std::cell::Cell<bool> = const {std::cell::Cell::new(false)}; }

#[cfg(test)]
thread_local! { static FORCE_LOOP: std::cell::Cell<bool> = const {std::cell::Cell::new(false)}; }
