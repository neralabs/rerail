//! Shared predicate requests advance by bounded byte chunks and compact their IDs.
use super::Error;
use super::metal_backend::launch::{commit_checked, wait_checked};
use super::metal_backend::{MappedBuffer, pipeline_named};
use super::metal_output_dispatch::SOURCE;
use super::metal_parallel::encode;
use super::metal_register_execution::read;
use metal::{BufferRef, CommandBufferRef, CommandQueueRef, Device};

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct RegexRequest {
    pub value: u32,
    pub source_start: u32,
    pub source_length: u32,
    pub search_start: u32,
    pub budget: u32,
    pub scratch: u32,
    pub width: u32,
    pub at: u32,
    pub phase: u32,
    pub status: u32,
    pub used: u32,
    pub match_start: u32,
    pub match_end: u32,
    pub mode: u32,
    pub owner: u32,
    pub evaluation: u32,
    pub progress: u32,
    pub strip_cursor: u32,
    pub strip_limit: u32,
    pub strip_next: u32,
}
#[derive(Clone, Copy)]
struct RegexParams {
    requests: u32,
    active: u32,
    scratch_slots: u32,
    code_words: u32,
    budget_owners: u32,
    evaluations: u32,
    generation: u32,
    unit_bytes: u32,
    abort_evaluation: u32,
    unit_actions: u32,
    external_source: u32,
    source_bytes: u32,
    owner_work_limit: u32,
    scratch_plan: ScratchPlan,
    auxiliary_width: u32,
    workers: u32,
}
struct ExecutionOptions<'a> {
    bytes: u32,
    actions: u32,
    snapshot: Option<StripSnapshot<'a>>,
}
impl RegexParams {
    fn words(self) -> [u32; 16] {
        [
            self.requests,
            self.active,
            self.scratch_slots,
            self.code_words,
            self.budget_owners,
            self.evaluations,
            self.generation,
            self.unit_bytes,
            self.abort_evaluation,
            self.unit_actions,
            self.external_source,
            self.source_bytes,
            self.owner_work_limit,
            self.scratch_plan as u32,
            self.auxiliary_width,
            self.workers,
        ]
    }
    fn run(
        self,
        command: &CommandBufferRef,
        name: &str,
        buffers: &[&BufferRef],
        groups: usize,
    ) -> Result<(), Error> {
        encode(
            command,
            &pipeline_named(SOURCE, name)?,
            name,
            buffers,
            &self.words(),
            groups,
            None,
        );
        Ok(())
    }
}
// One queue covers the complete batch. Parallelism bounds temporary state;
// the target never refuses a wide request that can fit with one worker.
const MAX_REGEX_WORKERS: usize = 256;
const AUXILIARY_TARGET_BYTES: usize = 32 * 1024 * 1024;
fn worker_count(active: usize, width: u32, scratch_slots: usize) -> Result<usize, Error> {
    let bytes = (width as usize)
        .checked_mul(12)
        .ok_or(Error::BackendCapacity)?;
    let capacity = match AUXILIARY_TARGET_BYTES.checked_div(bytes) {
        Some(capacity) => capacity.max(1),
        None => MAX_REGEX_WORKERS,
    };
    let previous_capacity = match scratch_slots.checked_div(width as usize) {
        None => MAX_REGEX_WORKERS,
        Some(0) => return Err(Error::BackendFailure),
        Some(covered) => covered,
    };
    Ok(active
        .min(MAX_REGEX_WORKERS)
        .min(capacity)
        .min(previous_capacity))
}
fn size(value: usize) -> Result<u32, Error> {
    u32::try_from(value).map_err(|_| Error::BackendCapacity)
}
fn check(status: &MappedBuffer) -> Result<(), Error> {
    match read::<u32>(status, 1)?[0] {
        0 => Ok(()),
        1 => Err(Error::BackendCapacity),
        _ => Err(Error::BackendFailure),
    }
}

pub(super) struct RegexProgram {
    pub code: MappedBuffer,
    code_words: u32,
}
/// Physical scratch selection precedes execution and never changes after refusal.
#[derive(Clone, Copy)]
#[repr(u32)]
pub(super) enum ScratchPlan {
    Device = 0,
    CooperativePredicates = 1,
}
/// These rows and scratch bounds come from the checked GPU eligibility scan.
pub(super) struct RegexBatch {
    pub scratch_plan: ScratchPlan,
    pub requests: MappedBuffer,
    pub ids: MappedBuffer,
    pub count: usize,
    /// Exact covered length of the compact scratch arena, not spare capacity.
    pub scratch_slots: usize,
}
pub(super) struct RegexInput<'a> {
    pub data: &'a MappedBuffer,
    pub status: &'a MappedBuffer,
    pub budget_owners: usize,
}
pub(super) struct SharedWork<'a> {
    pub values: &'a MappedBuffer,
    pub failures: &'a MappedBuffer,
    pub evaluations: usize,
    pub abort_evaluation: bool,
}
#[derive(Clone, Copy)]
pub(super) struct StripSnapshot<'a> {
    pub bytes: &'a MappedBuffer,
    pub length: usize,
    pub mask: &'a MappedBuffer,
    pub work_limit: u32,
}
impl RegexProgram {
    pub(super) fn new(
        device: &Device,
        batch: &super::packed_batch::PackedBatch,
    ) -> Result<Self, Error> {
        let words = super::metal_regex_code::compile(batch)?;
        Ok(Self {
            code: MappedBuffer::from_slice(device, &words)?,
            code_words: size(words.len())?,
        })
    }
    pub(super) fn execute(
        &self,
        device: &Device,
        queue: &CommandQueueRef,
        input: RegexInput<'_>,
        batch: RegexBatch,
        work: SharedWork<'_>,
    ) -> Result<MappedBuffer, Error> {
        self.execute_units(
            device,
            queue,
            input,
            batch,
            work,
            ExecutionOptions {
                bytes: 4096,
                actions: 32768,
                snapshot: None,
            },
        )
    }
    #[cfg(test)]
    pub(super) fn execute_with_unit_size(
        &self,
        device: &Device,
        queue: &CommandQueueRef,
        input: RegexInput<'_>,
        batch: RegexBatch,
        work: SharedWork<'_>,
        unit_bytes: u32,
    ) -> Result<MappedBuffer, Error> {
        self.execute_units(
            device,
            queue,
            input,
            batch,
            work,
            ExecutionOptions {
                bytes: unit_bytes,
                actions: unit_bytes,
                snapshot: None,
            },
        )
    }
    pub(super) fn code_words(&self) -> u32 {
        self.code_words
    }
    pub(super) fn execute_strip(
        &self,
        device: &Device,
        queue: &CommandQueueRef,
        input: RegexInput<'_>,
        batch: RegexBatch,
        work: SharedWork<'_>,
        snapshot: StripSnapshot<'_>,
    ) -> Result<MappedBuffer, Error> {
        self.execute_units(
            device,
            queue,
            input,
            batch,
            work,
            ExecutionOptions {
                bytes: 4096,
                actions: 32768,
                snapshot: Some(snapshot),
            },
        )
    }
    #[cfg(test)]
    pub(super) fn execute_strip_in_units(
        &self,
        device: &Device,
        queue: &CommandQueueRef,
        input: RegexInput<'_>,
        batch: RegexBatch,
        work: SharedWork<'_>,
        configuration: (StripSnapshot<'_>, u32),
    ) -> Result<MappedBuffer, Error> {
        self.execute_units(
            device,
            queue,
            input,
            batch,
            work,
            ExecutionOptions {
                bytes: configuration.1,
                actions: configuration.1,
                snapshot: Some(configuration.0),
            },
        )
    }
    fn execute_units(
        &self,
        device: &Device,
        queue: &CommandQueueRef,
        input: RegexInput<'_>,
        batch: RegexBatch,
        work: SharedWork<'_>,
        units: ExecutionOptions<'_>,
    ) -> Result<MappedBuffer, Error> {
        if units.bytes == 0 || units.bytes > 65536 || units.actions == 0 || units.actions > 65536 {
            return Err(Error::BackendFailure);
        }
        let snapshot = units.snapshot;
        let RegexBatch {
            scratch_plan,
            requests,
            mut ids,
            count,
            scratch_slots,
        } = batch;
        let mut owners = vec![
            input.data,
            &self.code,
            &requests,
            &ids,
            input.status,
            work.values,
            work.failures,
        ];
        if let Some(snapshot) = snapshot {
            if snapshot.work_limit as usize > super::MAX_STEPS_PER_PATH
                || snapshot.length as u64 > snapshot.bytes.buffer.length()
                || snapshot
                    .length
                    .checked_mul(4)
                    .ok_or(Error::BackendCapacity)? as u64
                    > snapshot.mask.buffer.length()
            {
                return Err(Error::BackendFailure);
            }
            owners.extend([snapshot.bytes, snapshot.mask]);
        }
        for (index, owner) in owners.iter().enumerate() {
            if owners[..index]
                .iter()
                .any(|other| std::ptr::eq(&*owner.buffer, &*other.buffer))
            {
                return Err(Error::BackendFailure);
            }
        }
        for (buffer, count, stride) in [
            (&requests, count, std::mem::size_of::<RegexRequest>()),
            (&ids, count, 4),
            (work.values, input.budget_owners, 4),
            (work.failures, work.evaluations, 4),
            (input.status, 1, 4),
        ] {
            let bytes = count.checked_mul(stride).ok_or(Error::BackendCapacity)?;
            if bytes as u64 > buffer.buffer.length() {
                return Err(Error::BackendFailure);
            }
        }
        let mut p = RegexParams {
            scratch_plan,
            auxiliary_width: 0,
            workers: 0,
            requests: size(count)?,
            active: size(count)?,
            scratch_slots: size(scratch_slots)?,
            code_words: self.code_words,
            budget_owners: size(input.budget_owners)?,
            evaluations: size(work.evaluations)?,
            generation: 0,
            unit_bytes: units.bytes,
            abort_evaluation: u32::from(work.abort_evaluation),
            unit_actions: units.actions,
            external_source: u32::from(snapshot.is_some()),
            source_bytes: snapshot.map_or(Ok(0), |s| size(s.length))?,
            owner_work_limit: snapshot.map_or(super::MAX_STEPS_PER_PATH as u32, |s| s.work_limit),
        };
        if count == 0 {
            if scratch_slots != 0 {
                return Err(Error::BackendFailure);
            }
            check(input.status)?;
            if snapshot.is_none_or(|s| s.length == 0) {
                return Ok(requests);
            }
        }
        let modes = MappedBuffer::zeroed::<u32>(device, 2)?;
        metal::objc::rc::autoreleasepool(|| -> Result<(), Error> {
            let command = queue.new_command_buffer();
            p.run(
                command,
                "value_frontier_preflight",
                &[
                    &input.data.buffer,
                    &self.code.buffer,
                    &requests.buffer,
                    &ids.buffer,
                    &input.status.buffer,
                    &modes.buffer,
                    &snapshot.map_or(input.data, |s| s.bytes).buffer,
                    &snapshot.map_or(input.data, |s| s.mask).buffer,
                ],
                count.max(snapshot.map_or(0, |s| s.length)).div_ceil(256),
            )?;
            commit_checked(command)?;
            wait_checked(command)?;
            check(input.status)
        })?;
        let metadata = read::<u32>(&modes, 2)?;
        let find = match metadata[0] {
            0 => false,
            1 => true,
            _ => return Err(Error::BackendFailure),
        };
        p.auxiliary_width = metadata[1];
        if p.auxiliary_width != 0 && !p.auxiliary_width.is_power_of_two() {
            return Err(Error::BackendFailure);
        }
        drop(modes);
        let pool_slots = worker_count(count, p.auxiliary_width, scratch_slots)?
            .checked_mul(p.auxiliary_width as usize)
            .ok_or(Error::BackendCapacity)?;
        size(pool_slots)?;
        let current = MappedBuffer::zeroed::<u32>(device, scratch_slots)?;
        let first_rank = MappedBuffer::zeroed::<u32>(device, pool_slots)?;
        let first_unit = MappedBuffer::zeroed::<u32>(device, pool_slots)?;
        let sorted = MappedBuffer::zeroed::<u32>(device, pool_slots)?;
        // Range tags persist across chunks and remain request-owned. Only NFA
        // ranking scratch retires after each worker completes its current unit.
        let captures = find
            .then(|| MappedBuffer::zeroed::<[u32; 2]>(device, scratch_slots))
            .transpose()?;
        let next_captures = find
            .then(|| MappedBuffer::zeroed::<[u32; 2]>(device, scratch_slots))
            .transpose()?;
        while p.active != 0 {
            p.workers = size(worker_count(
                p.active as usize,
                p.auxiliary_width,
                scratch_slots,
            )?)?;
            let next = MappedBuffer::zeroed::<u32>(device, p.active as usize)?;
            let queue_state = MappedBuffer::zeroed::<u32>(device, 2)?;
            metal::objc::rc::autoreleasepool(|| -> Result<(), Error> {
                let command = queue.new_command_buffer();
                p.run(
                    command,
                    "regex_frontier_advance",
                    &[
                        &input.data.buffer,
                        &self.code.buffer,
                        &requests.buffer,
                        &ids.buffer,
                        &current.buffer,
                        &first_rank.buffer,
                        &first_unit.buffer,
                        &sorted.buffer,
                        &next.buffer,
                        &input.status.buffer,
                        &work.values.buffer,
                        &work.failures.buffer,
                        // Preflight proves predicate batches never read or write
                        // range tags. Their unused bindings need no allocation.
                        &captures.as_ref().unwrap_or(&current).buffer,
                        &next_captures.as_ref().unwrap_or(&current).buffer,
                        &snapshot.map_or(input.data, |s| s.bytes).buffer,
                        &snapshot.map_or(&current, |s| s.mask).buffer,
                        &queue_state.buffer,
                    ],
                    p.workers as usize,
                )?;
                commit_checked(command)?;
                wait_checked(command)?;
                check(input.status)
            })?;
            let completed = read::<u32>(&queue_state, 2)?;
            // Every active request is claimed once. Pending IDs retain request
            // identity; their scheduling order has no output relation meaning.
            if completed[0] != p.active || completed[1] > p.active {
                return Err(Error::BackendFailure);
            }
            p.active = completed[1];
            p.generation = p.generation.checked_add(1).ok_or(Error::BackendFailure)?;
            ids = next;
        }
        Ok(requests)
    }
}
