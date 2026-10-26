use super::Error;
use super::metal_batch_abi::{
    ARENA_KIND_COUNT, EvaluationQuotaRecord, LazyArenaBlockRecord, MetalEvaluation,
    MetalGlobalArenas, MetalTask, SharedArenaRecord,
};
use super::native_ir::ArenaKind;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct TaskCapacities {
    pub pending: u32,
    pub captures: u32,
    pub cursors: u32,
    pub repeats: u32,
    pub query_states: u32,
    pub query_members: u32,
    pub queued_states: u32,
    pub greedy_entries: u32,
    pub value_states: u32,
    pub candidates: u32,
    pub results: u32,
    pub output_bytes: u32,
    pub output_captures: u32,
    pub properties: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct WorkspaceTotals {
    pub pending: u32,
    pub captures: u32,
    pub cursors: u32,
    pub repeats: u32,
    pub query_states: u32,
    pub query_members: u32,
    pub queued_states: u32,
    pub greedy_entries: u32,
    pub value_states: u32,
    pub candidates: u32,
    pub results: u32,
    pub output_bytes: u32,
    pub output_captures: u32,
    pub properties: u32,
}

impl WorkspaceTotals {
    pub(super) fn from_global(global: &MetalGlobalArenas) -> Self {
        let capacity = |kind: ArenaKind| global.arenas[kind as usize].capacity;
        Self {
            pending: capacity(ArenaKind::Pending),
            captures: capacity(ArenaKind::Capture),
            cursors: capacity(ArenaKind::Cursor),
            repeats: capacity(ArenaKind::Repeat),
            query_states: capacity(ArenaKind::QueryBucket),
            query_members: capacity(ArenaKind::QueryMember),
            queued_states: capacity(ArenaKind::QueuedState),
            greedy_entries: capacity(ArenaKind::GreedyEntry),
            value_states: capacity(ArenaKind::ValueState),
            candidates: capacity(ArenaKind::Candidate),
            results: capacity(ArenaKind::Result),
            output_bytes: capacity(ArenaKind::OutputByte),
            output_captures: capacity(ArenaKind::FinalCapture),
            properties: capacity(ArenaKind::Property),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct SharedWorkspacePlan {
    pub tasks: Vec<MetalTask>,
    pub evaluations: Vec<MetalEvaluation>,
    pub global: MetalGlobalArenas,
}

impl TaskCapacities {
    fn values(self) -> [u32; ARENA_KIND_COUNT] {
        let mut values = [0; ARENA_KIND_COUNT];
        values[ArenaKind::Pending as usize] = self.pending;
        values[ArenaKind::Capture as usize] = self.captures;
        values[ArenaKind::Cursor as usize] = self.cursors;
        values[ArenaKind::Repeat as usize] = self.repeats;
        values[ArenaKind::QueryBucket as usize] = self.query_states;
        values[ArenaKind::QueryMember as usize] = self.query_members;
        values[ArenaKind::QueuedState as usize] = self.queued_states;
        values[ArenaKind::GreedyEntry as usize] = self.greedy_entries;
        values[ArenaKind::ValueState as usize] = self.value_states;
        values[ArenaKind::Candidate as usize] = self.candidates;
        values[ArenaKind::Result as usize] = self.results;
        values[ArenaKind::FinalCapture as usize] = self.output_captures;
        values[ArenaKind::Property as usize] = self.properties;
        values[ArenaKind::OutputByte as usize] = self.output_bytes;
        values
    }
}

pub(super) fn plan_shared(
    tasks: &[MetalTask],
    capacities: &[TaskCapacities],
    shared_caps: Option<[u32; ARENA_KIND_COUNT]>,
) -> Result<SharedWorkspacePlan, Error> {
    if tasks.len() != capacities.len() {
        return Err(Error::InvalidAnchor);
    }
    let mut plan = SharedWorkspacePlan::default();
    let mut task_index = 0;
    while task_index < tasks.len() {
        let first = tasks[task_index];
        let evaluation_index =
            u32::try_from(plan.evaluations.len()).map_err(|_| Error::BackendCapacity)?;
        let mut limits = [0_u32; ARENA_KIND_COUNT];
        let mut work_limit = 0_u32;
        let mut end = task_index;
        while end < tasks.len()
            && tasks[end].file == first.file
            && tasks[end].program == first.program
        {
            for (kind, (total, value)) in
                limits.iter_mut().zip(capacities[end].values()).enumerate()
            {
                *total = if [
                    ArenaKind::Pending as usize,
                    ArenaKind::Capture as usize,
                    ArenaKind::Cursor as usize,
                    ArenaKind::Repeat as usize,
                    ArenaKind::ValueState as usize,
                    ArenaKind::Candidate as usize,
                    ArenaKind::QueryBucket as usize,
                    ArenaKind::QueryMember as usize,
                    ArenaKind::QueuedState as usize,
                    ArenaKind::GreedyEntry as usize,
                ]
                .contains(&kind)
                {
                    (*total).max(value)
                } else if let Some(caps) = shared_caps {
                    total.saturating_add(value).min(caps[kind])
                } else {
                    total.checked_add(value).ok_or(Error::BackendCapacity)?
                };
            }
            work_limit = work_limit.saturating_add(tasks[end].work_limit);
            plan.tasks.push(MetalTask {
                evaluation: evaluation_index,
                batch_ordinal: tasks[end].batch_ordinal,
                file: tasks[end].file,
                program: tasks[end].program,
                root: tasks[end].root,
                work_limit: tasks[end].work_limit,
            });
            end += 1;
        }
        let mut evaluation = MetalEvaluation {
            generation: 1,
            work_limit,
            task_start: u32::try_from(task_index).map_err(|_| Error::BackendCapacity)?,
            task_count: u32::try_from(end - task_index).map_err(|_| Error::BackendCapacity)?,
            pending: LazyArenaBlockRecord {
                limit: limits[ArenaKind::Pending as usize],
                ..LazyArenaBlockRecord::default()
            },
            capture: LazyArenaBlockRecord {
                limit: limits[ArenaKind::Capture as usize],
                ..LazyArenaBlockRecord::default()
            },
            cursor: LazyArenaBlockRecord {
                limit: limits[ArenaKind::Cursor as usize],
                ..LazyArenaBlockRecord::default()
            },
            repeat: LazyArenaBlockRecord {
                limit: limits[ArenaKind::Repeat as usize],
                ..LazyArenaBlockRecord::default()
            },
            value_state: LazyArenaBlockRecord {
                limit: limits[ArenaKind::ValueState as usize],
                ..LazyArenaBlockRecord::default()
            },
            candidate: LazyArenaBlockRecord {
                limit: limits[ArenaKind::Candidate as usize],
                ..LazyArenaBlockRecord::default()
            },
            query_bucket: LazyArenaBlockRecord {
                limit: limits[ArenaKind::QueryBucket as usize],
                ..LazyArenaBlockRecord::default()
            },
            query_member: LazyArenaBlockRecord {
                limit: limits[ArenaKind::QueryMember as usize],
                ..LazyArenaBlockRecord::default()
            },
            queued_state: LazyArenaBlockRecord {
                limit: limits[ArenaKind::QueuedState as usize],
                ..LazyArenaBlockRecord::default()
            },
            greedy_entry: LazyArenaBlockRecord {
                limit: limits[ArenaKind::GreedyEntry as usize],
                ..LazyArenaBlockRecord::default()
            },
            ..MetalEvaluation::default()
        };
        for (quota, limit) in evaluation.quotas.iter_mut().zip(limits) {
            *quota = EvaluationQuotaRecord { used: 0, limit };
        }
        plan.evaluations.push(evaluation);
        task_index = end;
    }
    for evaluation in &plan.evaluations {
        for (kind, (arena, quota)) in plan
            .global
            .arenas
            .iter_mut()
            .zip(evaluation.quotas)
            .enumerate()
        {
            arena.capacity = if let Some(caps) = shared_caps {
                arena.capacity.saturating_add(quota.limit).min(caps[kind])
            } else {
                arena
                    .capacity
                    .checked_add(quota.limit)
                    .ok_or(Error::BackendCapacity)?
            };
        }
    }
    for arena in &mut plan.global.arenas {
        *arena = SharedArenaRecord {
            start: 0,
            capacity: arena.capacity,
            cursor: 0,
        };
    }
    Ok(plan)
}
