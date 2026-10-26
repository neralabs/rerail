//! Capture-bearing native operation transport shared by grouped and single SQL bindings.
use super::*;
use crate::code_rule_dsl::matcher::{cohort, cohort_host};
use std::{
    cell::{Cell as ContextCell, RefCell},
    collections::{BTreeSet, VecDeque},
    rc::Rc,
};

pub(super) struct Task<'a> {
    pub fragment: &'a Fragment,
    pub file: u32,
    pub requests: Vec<Request>,
    pub remaining: Option<usize>,
}
#[derive(Default, serde::Serialize)]
pub(super) struct Metrics {
    pub operations: cohort::Metrics,
    pub semantic: serde_json::Value,
    pub cpu_ns: u64,
    pub wall_ns: u64,
}
struct Contexts<'a> {
    batch: &'a Batch,
    tasks: &'a [Task<'a>],
    memo: Rc<RefCell<matcher::MemoBank>>,
    active_file: Rc<ContextCell<Option<u32>>>,
}
impl cohort_host::Context for Contexts<'_> {
    fn group(&self, owner: u32) -> u32 {
        self.tasks[owner as usize].file
    }
    fn with_group<T>(&mut self, owner: u32, run: impl FnOnce(&mut Self) -> T) -> T {
        let file_id = self.tasks[owner as usize].file;
        if self.active_file.get() == Some(file_id) {
            return run(self);
        }
        let batch = self.batch;
        let file = &batch.files[file_id as usize];
        let memo = self.memo.clone();
        let active = self.active_file.clone();
        struct ActiveFile(Rc<ContextCell<Option<u32>>>, Option<u32>);
        impl Drop for ActiveFile {
            fn drop(&mut self) {
                self.0.set(self.1);
            }
        }
        let _budget = file.context.budget.as_ref().map(|b| b.clone().resume());
        crate::code_rule_dsl::budget::with_allowance(u64::MAX, || {
            with_scopes(file_id as usize, file, || {
                memo.borrow_mut().with_file(file_id, &file.root, || {
                    let _active = ActiveFile(active.clone(), active.replace(Some(file_id)));
                    run(self)
                })
            })
        })
        .0
    }
    fn with<T>(&mut self, owner: u32, run: impl FnOnce() -> T) -> T {
        let task = &self.tasks[owner as usize];
        if self.active_file.get() != Some(task.file) {
            return self.with_group(owner, |contexts| contexts.with(owner, run));
        }
        matcher::begin_clause(&task.fragment.grammar, usize::MAX);
        struct Clause;
        impl Drop for Clause {
            fn drop(&mut self) {
                matcher::end_clause();
            }
        }
        let _clause = Clause;
        run()
    }
    fn cancelled(&self, owner: u32) -> bool {
        self.batch.files[self.tasks[owner as usize].file as usize]
            .context
            .budget
            .as_ref()
            .is_some_and(|b| b.cancelled())
    }
    fn file_root(&self, owner: u32) -> Option<usize> {
        Some(&self.batch.files[self.tasks[owner as usize].file as usize].root as *const _ as usize)
    }
}

pub(super) fn execute(batch: &Batch, tasks: &[Task<'_>]) -> Result<(Vec<Vec<Row>>, Metrics)> {
    let mut programs = cohort::Programs::default();
    for task in tasks {
        programs.lower(&task.fragment.grammar.matcher);
    }
    execute_compiled(batch, tasks, &programs)
}

pub(super) fn execute_compiled(
    batch: &Batch,
    tasks: &[Task<'_>],
    programs: &cohort::Programs,
) -> Result<(Vec<Vec<Row>>, Metrics)> {
    let started = Started::now();
    let entries = tasks
        .iter()
        .map(|t| {
            programs
                .lookup(&t.fragment.grammar.matcher)
                .expect("compiled cohort entry")
        })
        .collect::<Vec<_>>();
    let allowances = tasks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let left = t.remaining.unwrap_or_else(|| {
                t.fragment.allowance.map_or(t.fragment.work, |(base, per)| {
                    base.max(per.saturating_mul(batch.files[t.file as usize].facts.len() as usize))
                })
            });
            (i as u32, left)
        })
        .collect();
    let mut host = cohort_host::NativeHost::new(
        Contexts {
            batch,
            tasks,
            memo: Default::default(),
            active_file: Rc::new(ContextCell::new(None)),
        },
        allowances,
    );
    let mut bank = cohort::Bank::default();
    let mut source = Transport {
        batch,
        tasks,
        entries,
        ready: tasks
            .iter()
            .enumerate()
            .filter(|(_, t)| !t.requests.is_empty())
            .map(|(i, _)| (i, 0, 0))
            .collect(),
        rematches: VecDeque::new(),
        output: (0..tasks.len()).map(|_| Vec::new()).collect(),
        empty: Arc::new(MetaVarEnv::new()),
        following: Arc::new(Vec::new()),
        error: None,
    };
    bank.execute_source(programs, &mut source, &mut host, matcher::wave::TILE, 0);
    if let Some(error) = source.error {
        return Err(error);
    }
    let mut output = source.output;
    let elapsed = started.elapsed();
    for (owner, task) in tasks.iter().enumerate() {
        if let Some(failed) = host.failed(owner as u32) {
            output[owner].clear();
            let truth = match failed {
                cohort::Failure::Cancelled => -5,
                cohort::Failure::Utility(limit) => failure::encode(&format!("{limit:?}"))
                    .ok_or_else(|| invalid("unknown native failure"))?,
            };
            output[owner].push(Row {
                request: -1,
                file: task.file,
                node: 0,
                truth,
                ordinal: 0,
                values: vec![Cell::Null; task.fragment.names.len()],
            });
        }
        output[owner].sort_by_key(|r| (r.request, r.ordinal));
        let mut m = task.fragment.metrics.lock().unwrap();
        m.cohort_requests += task.requests.len();
        m.peak_request_rows = m.peak_request_rows.max(task.requests.len());
        m.completed_roots += if host.failed(owner as u32).is_none() {
            task.requests.len()
        } else {
            0
        };
        m.logical_unknown_roots += output[owner].iter().filter(|r| r.truth == 2).count();
        m.rows += output[owner].len();
        m.remaining_work
            .insert(task.file, host.remaining(owner as u32));
    }
    Ok((
        output,
        Metrics {
            operations: bank.metrics,
            semantic: host.report(),
            cpu_ns: elapsed.cpu,
            wall_ns: elapsed.wall,
        },
    ))
}

/// Refill completed lanes from the same bounded document tile. Enumeration reuses
/// the original input frame and carries its exclusion set into the next demand.
struct Transport<'a> {
    batch: &'a Batch,
    tasks: &'a [Task<'a>],
    entries: Vec<u32>,
    ready: VecDeque<(usize, usize, usize)>,
    rematches: VecDeque<((usize, usize, i64), cohort::Request<'a, TapeDoc>)>,
    output: Vec<Vec<Row>>,
    empty: Arc<MetaVarEnv<'a, TapeDoc>>,
    following: Arc<Vec<(usize, usize)>>,
    error: Option<Error>,
}
impl<'a, 'p> cohort::Source<'a, TapeDoc, cohort_host::NativeHost<'p, Contexts<'a>>>
    for Transport<'a>
{
    type Token = (usize, usize, i64);
    fn next(
        &mut self,
        host: &mut cohort_host::NativeHost<'p, Contexts<'a>>,
    ) -> Option<(Self::Token, cohort::Request<'a, TapeDoc>)> {
        if self.error.is_some() {
            return None;
        }
        while let Some((token, request)) = self.rematches.pop_front() {
            if host.failed(token.0 as u32).is_none() {
                return Some((token, request));
            }
        }
        while let Some((owner, key, taken)) = self.ready.pop_front() {
            if host.failed(owner as u32).is_some() {
                continue;
            }
            let task = &self.tasks[owner];
            let r = &task.requests[key];
            let file = &self.batch.files[task.file as usize];
            if r.file != task.file || r.node >= file.facts.len() {
                self.error = Some(invalid("root outside document"));
                return None;
            }
            let env = if r.inputs.is_empty() {
                self.empty.clone()
            } else {
                let bindings = r
                    .inputs
                    .iter()
                    .map(|(n, k, _)| (n.clone(), *k))
                    .collect::<Vec<_>>();
                let values = r
                    .inputs
                    .iter()
                    .map(|(_, _, v)| v.clone())
                    .collect::<Vec<_>>();
                match capture_env::environment(
                    task.file,
                    &file.facts,
                    |id| matcher::node_at(&file.root, id),
                    &bindings,
                    &values,
                ) {
                    Ok(env) => Arc::new(env),
                    Err(error) => {
                        self.error = Some(error);
                        return None;
                    }
                }
            };
            if key + 1 < task.requests.len() {
                if taken + 1 < 32 {
                    self.ready.push_front((owner, key + 1, taken + 1));
                } else {
                    self.ready.push_back((owner, key + 1, 0));
                }
            }
            return Some((
                (owner, key, 0),
                cohort::Request {
                    program: self.entries[owner],
                    context: owner as u32,
                    node: matcher::node_at(&file.root, r.node),
                    env,
                    three: task.fragment.grammar.three_valued,
                    each: None,
                    following: self.following.clone(),
                },
            ));
        }
        None
    }
    fn completed(
        &mut self,
        (owner, key, ordinal): Self::Token,
        mut request: cohort::Request<'a, TapeDoc>,
        answer: cohort::Result<'a, TapeDoc>,
        host: &mut cohort_host::NativeHost<'p, Contexts<'a>>,
    ) {
        if let Some(failure) = answer.failure {
            host.record_failure(owner as u32, Some(failure));
            return;
        }
        if self.error.is_some()
            || host.failed(owner as u32).is_some()
            || answer.truth == 0
            || (ordinal > 0 && answer.truth != 2)
        {
            return;
        }
        let task = &self.tasks[owner];
        let mut fresh = false;
        let mut exclusion = request.each.take().map(Arc::unwrap_or_clone);
        if answer.truth == 2
            && let Some(name) = &task.fragment.each
        {
            let excluded = exclusion.get_or_insert_with(|| cohort::Exclusion {
                name: name.clone(),
                ranges: BTreeSet::new(),
            });
            if let Some(range) = answer
                .env
                .get_match(name)
                .map(|n| n.range())
                .or_else(|| matcher::exported_span(&answer.env, name))
            {
                fresh = excluded.ranges.insert((range.start, range.end));
            }
            if ordinal > 0 && !fresh {
                return;
            }
        }
        let values = if answer.truth == 2 {
            match capture_values(task.file, &answer.env, &task.fragment.names) {
                Ok(values) => values,
                Err(error) => {
                    self.error = Some(error);
                    return;
                }
            }
        } else {
            vec![Cell::Null; task.fragment.names.len()]
        };
        self.output[owner].push(Row {
            request: key as i64,
            file: task.file,
            node: task.requests[key].node,
            truth: if answer.truth == 2 { 1 } else { 2 },
            ordinal,
            values,
        });
        if fresh {
            self.rematches.push_back((
                (owner, key, ordinal + 1),
                cohort::Request {
                    each: exclusion.map(Arc::new),
                    ..request
                },
            ));
        }
    }
}
