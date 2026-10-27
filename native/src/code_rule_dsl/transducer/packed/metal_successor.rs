//! Bounded successor values and live control metadata, owned by one generation.
use super::Error;
use super::metal_backend::MappedBuffer;
use super::metal_grid_plan::{ExecutionPlan, ResidentInput};
use super::metal_ready_batch::ReadyBatch;
use super::metal_state_values::{
    FrontierRecords, POSITIONED_COLUMNS, PositionedFrontier, StateValues,
};
use metal::{BufferRef, CommandBufferRef, Device};

#[path = "metal_successor_active.rs"]
mod active;
#[path = "metal_successor_comparisons.rs"]
mod comparisons;

/// Six SoA columns: position, pending body, and latest offer spans (start/count).
/// Digits encode logical branch choices. They are not state identity or public ordinals.
/// Non-pending states have empty body/offer spans. Pending kinds are 6 query, 7 ordinary.
/// Initial records have no physical span or work entries; both are intrinsically zero.
pub(super) struct ControlValues<const FIELDS: usize = POSITIONED_COLUMNS> {
    pub spans: MappedBuffer,
    /// Executed structural instructions and inline values, inherited by each path.
    pub path_work: MappedBuffer,
    pub digits: MappedBuffer,
    pub count: MappedBuffer,
    pub state_capacity: usize,
    pub digit_capacity: usize,
    pub generation: u64,
}
pub(super) struct SuccessorInput<'a, const FIELDS: usize = POSITIONED_COLUMNS> {
    pub resident: &'a ResidentInput,
    pub plan: &'a ExecutionPlan,
    pub data: &'a MappedBuffer,
    pub states: &'a FrontierRecords<FIELDS>,
    pub values: &'a StateValues,
    pub control: &'a ControlValues<FIELDS>,
    pub ready: &'a ReadyBatch<FIELDS>,
}
pub(super) struct SuccessorOutput<'a> {
    pub states: &'a PositionedFrontier,
    pub values: &'a StateValues,
    pub control: &'a ControlValues,
}
#[derive(Clone, Copy)]
pub(super) struct CountLimits {
    pub states: usize,
    pub cursors: usize,
    pub repeats: usize,
    pub captures: usize,
    pub digits: usize,
}
impl CountLimits {
    fn from_output(output: &SuccessorOutput<'_>) -> Self {
        Self {
            states: output.states.capacity,
            cursors: output.values.cursor_capacity,
            repeats: output.values.repeat_capacity,
            captures: output.values.capture_capacity,
            digits: output.control.digit_capacity,
        }
    }
}

/// Current-generation reduction scratch. Only completed scalar totals cross to the owner.
pub(super) struct SuccessorCounts<const FIELDS: usize = POSITIONED_COLUMNS> {
    command: metal::CommandBuffer,
    preparation: super::metal_successor_prepare::PreparedSuccessors<FIELDS>,
    comparisons: Option<comparisons::Work>,
    sizes: super::metal_parallel::Scan,
    task_scan: super::metal_parallel::Scan,
    _task_keys: MappedBuffer,
    survivors: super::metal_parallel::Scan,
    candidates: MappedBuffer,
    columns: MappedBuffer,
    spans: MappedBuffer,
    cursors: MappedBuffer,
    repeats: MappedBuffer,
    captures: MappedBuffer,
    digits: MappedBuffer,
    meta: MappedBuffer,
    parameters: Vec<u32>,
    limits: CountLimits,
    output_work: usize,
}
impl<const FIELDS: usize> SuccessorCounts<FIELDS> {
    pub(super) fn into_publication<'a>(
        self,
        input: SuccessorInput<'a, FIELDS>,
    ) -> Result<CompletedPublication<'a, FIELDS>, Error> {
        let capacity = self.check_completed()?;
        self.preparation.validate_binding(&input, &self.command)?;
        let pending = self.preparation.comparison_needs()?.pending;
        let cells = unsafe { self.sizes.total().contents().cast::<[u32; 4]>().read() };
        let capacity = CountLimits {
            states: capacity.states,
            cursors: cells[0] as usize,
            repeats: cells[1] as usize,
            captures: cells[2] as usize,
            digits: cells[3] as usize,
        };
        let retain_total = |scan: &super::metal_parallel::Scan| MappedBuffer {
            buffer: scan.total().to_owned(),
        };
        let total = retain_total(&self.preparation.expansion);
        let sizes = retain_total(&self.sizes);
        let sums = retain_total(&self.survivors);
        let peers = self
            .comparisons
            .and_then(comparisons::Work::into_publication);
        if pending != peers.is_some() {
            return Err(Error::BackendFailure);
        }
        let peers = peers.map(|(offers, alive)| PublicationPeers {
            tasks: self.task_scan.values,
            offers,
            alive,
        });
        // Completed command owners, scans, and comparison scratch drop here,
        // before the generation owner allocates publication or readiness storage.
        Ok(CompletedPublication {
            input,
            capacity,
            source: self.preparation.code.source,
            parameters: self.parameters,
            threads: self.limits.states.max(self.output_work),
            status: self.preparation.status,
            steps: self.preparation.steps,
            columns: self.columns,
            spans: self.spans,
            candidates: self.candidates,
            cursors: self.cursors,
            repeats: self.repeats,
            captures: self.captures,
            digits: self.digits,
            meta: self.meta,
            offsets: self.survivors.values,
            total,
            sizes,
            sums,
            peers,
        })
    }
    pub(super) fn check_completed(
        &self,
    ) -> Result<super::metal_successor_prepare::Capacity, Error> {
        if self.command.status() != metal::MTLCommandBufferStatus::Completed {
            return Err(Error::BackendFailure);
        }
        self.preparation.check_completed()?;
        let states = unsafe { self.survivors.total().contents().cast::<u32>().read() } as usize;
        let cells =
            unsafe { std::slice::from_raw_parts(self.sizes.total().contents().cast::<u32>(), 4) };
        if states > self.limits.states
            || cells
                .iter()
                .zip([
                    self.limits.cursors,
                    self.limits.repeats,
                    self.limits.captures,
                    self.limits.digits,
                ])
                .any(|(&count, limit)| count == u32::MAX || count as usize > limit)
        {
            return Err(Error::BackendCapacity);
        }
        Ok(super::metal_successor_prepare::Capacity {
            states,
            cells: cells.iter().copied().max().unwrap_or(0) as usize,
        })
    }
}
#[cfg(test)]
#[must_use = "Check successor failures after the shared command completes"]
pub(super) struct PendingSuccessors<const FIELDS: usize = POSITIONED_COLUMNS> {
    command: metal::CommandBuffer,
    counts: SuccessorCounts<FIELDS>,
}
#[cfg(test)]
impl<const FIELDS: usize> PendingSuccessors<FIELDS> {
    pub(super) fn check_completed(self) -> Result<(), Error> {
        if self.command.status() != metal::MTLCommandBufferStatus::Completed {
            return Err(Error::BackendFailure);
        }
        self.counts.check_completed().map(|_| ())
    }
}
fn validate_output<const FIELDS: usize>(
    input: &SuccessorInput<'_, FIELDS>,
    output: &SuccessorOutput<'_>,
) -> Result<(), Error> {
    let next = input
        .states
        .generation
        .checked_add(1)
        .ok_or(Error::BackendFailure)?;
    if output.states.generation != next
        || output.values.generation != next
        || output.control.generation != next
        || output.control.state_capacity != output.states.capacity
    {
        return Err(Error::BackendFailure);
    }
    super::metal_successor_prepare::validate_input(
        input,
        &[
            (
                &output.states.columns,
                output.states.capacity,
                POSITIONED_COLUMNS * 4,
            ),
            (&output.states.counts, 5, 4),
            (&output.values.cursors, output.values.cursor_capacity, 16),
            (&output.values.repeats, output.values.repeat_capacity, 16),
            (&output.values.captures, output.values.capture_capacity, 16),
            (&output.values.counts, 2, 4),
            (&output.control.spans, output.control.state_capacity, 24),
            (&output.control.digits, output.control.digit_capacity, 4),
            (&output.control.count, 1, 4),
            (&output.control.path_work, output.control.state_capacity, 4),
        ],
    )
}
#[cfg(test)]
pub(super) fn encode<const FIELDS: usize>(
    device: &Device,
    queue: &metal::CommandQueueRef,
    command: &CommandBufferRef,
    input: SuccessorInput<'_, FIELDS>,
    output: SuccessorOutput<'_>,
) -> Result<PendingSuccessors<FIELDS>, Error> {
    validate_output(&input, &output)?;
    let preparation = super::metal_successor_prepare::prepare(device, queue, &input)?;
    encode_prepared(device, command, input, output, preparation)
}
#[cfg(test)]
pub(super) fn encode_prepared<const FIELDS: usize>(
    device: &Device,
    command: &CommandBufferRef,
    input: SuccessorInput<'_, FIELDS>,
    output: SuccessorOutput<'_>,
    preparation: super::metal_successor_prepare::PreparedSuccessors<FIELDS>,
) -> Result<PendingSuccessors<FIELDS>, Error> {
    validate_output(&input, &output)?;
    let counts = encode_counts(
        device,
        command,
        &input,
        preparation,
        CountLimits::from_output(&output),
    )?;
    encode_publish(command, &input, output, counts)
}

pub(super) fn encode_counts<const FIELDS: usize>(
    device: &Device,
    command: &CommandBufferRef,
    input: &SuccessorInput<'_, FIELDS>,
    preparation: super::metal_successor_prepare::PreparedSuccessors<FIELDS>,
    limits: CountLimits,
) -> Result<SuccessorCounts<FIELDS>, Error> {
    use super::metal_backend::pipeline_named;
    use super::metal_parallel::{Scan, encode as dispatch};
    preparation.validate_binding(input, command)?;
    let code = &preparation.code;
    let expansion = &preparation.expansion;
    let steps = &preparation.steps;
    let status = &preparation.status;
    let out = limits.states;
    let pair_capacity = out.saturating_mul(out).min(16_777_216);
    let output_work = limits
        .cursors
        .max(limits.repeats)
        .max(limits.captures)
        .max(limits.digits)
        .max(1);
    let work = pair_capacity.saturating_mul(output_work).min(16_777_216);
    let needs = preparation.comparison_needs()?;
    let tasks = if needs.pending {
        super::metal_parallel::task_group_capacity(
            out.min(input.resident.input.grid.header.semantic_tuple_count as usize),
        )?
    } else {
        0
    };
    let comparison_work = (needs.active || needs.pending)
        .then(|| comparisons::Work::new(device, out, pair_capacity, work, tasks, needs))
        .transpose()?;
    let active_slots = comparison_work
        .as_ref()
        .map_or(0, comparisons::Work::active_slots);
    let mut parameters = preparation.parameters.clone();
    for (index, value) in [
        (1, out),
        (10, limits.cursors),
        (11, limits.repeats),
        (12, limits.captures),
        (13, limits.digits),
        (15, pair_capacity),
        (16, work),
        (17, active_slots),
        (19, tasks),
    ] {
        parameters[index] = u32::try_from(value).map_err(|_| Error::BackendCapacity)?;
    }
    let sizes = Scan::new(device, tasks.max(out))?;
    // Independent columns share one scan: task counts in x, live choices in y.
    let task_scan = Scan::new(device, tasks.max(out))?;
    let task_keys = MappedBuffer::zeroed::<u32>(device, tasks)?;
    // Empty slots carry no semantic task identity. The GPU supplies every key.
    unsafe {
        std::slice::from_raw_parts_mut(task_keys.buffer.contents().cast::<u32>(), tasks)
            .fill(u32::MAX);
    }
    let survivors = Scan::new(device, out)?;
    let candidates = MappedBuffer::zeroed::<[u32; 9]>(device, out)?;
    let columns = MappedBuffer::zeroed::<[u32; POSITIONED_COLUMNS]>(device, out)?;
    let spans = MappedBuffer::zeroed::<[u32; 6]>(device, out)?;
    let cursors = MappedBuffer::zeroed::<[u32; 4]>(device, limits.cursors)?;
    let repeats = MappedBuffer::zeroed::<[u32; 4]>(device, limits.repeats)?;
    let captures = MappedBuffer::zeroed::<[u32; 4]>(device, limits.captures)?;
    let digits = MappedBuffer::zeroed::<u32>(device, limits.digits)?;
    let meta = MappedBuffer::zeroed::<[u32; 7]>(device, out)?;
    let run = |name, buffers: &[&metal::BufferRef], count: usize| -> Result<(), Error> {
        let pipeline = pipeline_named(&code.source, name)?;
        dispatch(
            command,
            &pipeline,
            name,
            buffers,
            &parameters,
            count.div_ceil(256).max(1),
            None,
        );
        Ok(())
    };
    let h = &input.states.counts.buffer;
    let s = &input.states.columns.buffer;
    let isp = &input.control.spans.buffer;
    let data = &input.data.buffer;
    let st = &status.buffer;
    run(
        "successor_candidates",
        &[
            data,
            s,
            h,
            isp,
            &steps.buffer,
            &expansion.values.buffer,
            expansion.total(),
            &candidates.buffer,
            &sizes.values.buffer,
            &task_scan.values.buffer,
            &task_scan.values.buffer,
            st,
            &task_keys.buffer,
        ],
        out,
    )?;
    sizes.encode_pair(&task_scan, command)?;
    run(
        "successor_materialize",
        &[
            data,
            s,
            h,
            isp,
            &steps.buffer,
            &candidates.buffer,
            expansion.total(),
            &sizes.values.buffer,
            sizes.total(),
            &columns.buffer,
            &spans.buffer,
            st,
            &meta.buffer,
            &comparison_work
                .as_ref()
                .and_then(comparisons::Work::heads)
                .unwrap_or(&meta)
                .buffer,
            &input.values.cursors.buffer,
            &input.values.repeats.buffer,
            &input.values.captures.buffer,
            &input.control.digits.buffer,
            &task_scan.values.buffer,
            &expansion.values.buffer,
            &cursors.buffer,
            &repeats.buffer,
            &captures.buffer,
            &digits.buffer,
        ],
        out.max(active_slots).max(output_work),
    )?;
    if let Some(work) = &comparison_work {
        work.encode(
            command,
            &code.source,
            &parameters,
            comparisons::Input {
                columns: &columns,
                spans: &spans,
                digits: &digits,
                cursors: &cursors,
                repeats: &repeats,
                captures: &captures,
                candidates: &candidates,
                expansion,
                task_scan: &task_scan,
                meta: &meta,
                status,
            },
        )?;
    }
    if out <= 65536 {
        run(
            "successor_survivors",
            &[
                &candidates.buffer,
                expansion.total(),
                &meta.buffer,
                &survivors.values.buffer,
                st,
                survivors.total(),
            ],
            256,
        )?;
    } else {
        run(
            "successor_final_counts",
            &[
                &candidates.buffer,
                expansion.total(),
                &meta.buffer,
                &survivors.values.buffer,
                st,
            ],
            out,
        )?;
        survivors.encode(command)?;
    }
    Ok(SuccessorCounts {
        command: command.to_owned(),
        preparation,
        comparisons: comparison_work,
        sizes,
        task_scan,
        _task_keys: task_keys,
        survivors,
        candidates,
        columns,
        spans,
        cursors,
        repeats,
        captures,
        digits,
        meta,
        parameters,
        limits,
        output_work,
    })
}

#[cfg(test)]
pub(super) fn encode_publish<const FIELDS: usize>(
    command: &CommandBufferRef,
    input: &SuccessorInput<'_, FIELDS>,
    output: SuccessorOutput<'_>,
    counts: SuccessorCounts<FIELDS>,
) -> Result<PendingSuccessors<FIELDS>, Error> {
    validate_output(input, &output)?;
    counts.preparation.validate_binding(input, command)?;
    if !std::ptr::eq(&*counts.command, command) {
        let capacity = counts.check_completed()?;
        let lengths =
            unsafe { std::slice::from_raw_parts(counts.sizes.total().contents().cast::<u32>(), 4) };
        if capacity.states > output.states.capacity
            || lengths
                .iter()
                .zip([
                    output.values.cursor_capacity,
                    output.values.repeat_capacity,
                    output.values.capture_capacity,
                    output.control.digit_capacity,
                ])
                .any(|(&count, limit)| count as usize > limit)
        {
            return Err(Error::BackendCapacity);
        }
    } else {
        let limits = CountLimits::from_output(&output);
        if counts.limits.states > limits.states
            || counts.limits.cursors > limits.cursors
            || counts.limits.repeats > limits.repeats
            || counts.limits.captures > limits.captures
            || counts.limits.digits > limits.digits
        {
            return Err(Error::BackendCapacity);
        }
    }
    let SuccessorCounts {
        preparation,
        comparisons: comparison_work,
        sizes,
        task_scan,
        survivors,
        candidates,
        columns,
        spans,
        cursors,
        repeats,
        captures,
        digits,
        meta,
        parameters,
        limits,
        output_work,
        ..
    } = &counts;
    let expansion = &preparation.expansion;
    let steps = &preparation.steps;
    let st = &preparation.status.buffer;
    let out = limits.states;
    // Without query or pending candidates, publish never reads these bindings.
    let offers = comparison_work
        .as_ref()
        .and_then(comparisons::Work::offers)
        .unwrap_or(meta);
    let alive = comparison_work
        .as_ref()
        .and_then(comparisons::Work::alive)
        .unwrap_or(meta);
    encode_publication(
        command,
        &preparation.code.source,
        parameters,
        PublicationView {
            columns: &columns.buffer,
            spans: &spans.buffer,
            candidates: &candidates.buffer,
            total: expansion.total(),
            sizes: sizes.total(),
            offsets: &survivors.values.buffer,
            sums: survivors.total(),
            tasks: &task_scan.values.buffer,
            offers: &offers.buffer,
            meta: &meta.buffer,
            alive: &alive.buffer,
            status: st,
            steps: &steps.buffer,
            cursors: &cursors.buffer,
            repeats: &repeats.buffer,
            captures: &captures.buffer,
            digits: &digits.buffer,
        },
        &output,
        out.max(*output_work),
    )?;
    Ok(PendingSuccessors {
        command: command.to_owned(),
        counts,
    })
}

struct PublicationPeers {
    tasks: MappedBuffer,
    offers: MappedBuffer,
    alive: MappedBuffer,
}

/// Owned publication data after checked comparison and survivor completion.
/// The input lifetime prevents a different input from replacing its binding.
pub(super) struct CompletedPublication<'a, const FIELDS: usize> {
    input: SuccessorInput<'a, FIELDS>,
    capacity: CountLimits,
    source: String,
    parameters: Vec<u32>,
    threads: usize,
    status: MappedBuffer,
    steps: MappedBuffer,
    columns: MappedBuffer,
    spans: MappedBuffer,
    candidates: MappedBuffer,
    cursors: MappedBuffer,
    repeats: MappedBuffer,
    captures: MappedBuffer,
    digits: MappedBuffer,
    meta: MappedBuffer,
    offsets: MappedBuffer,
    total: MappedBuffer,
    sizes: MappedBuffer,
    sums: MappedBuffer,
    peers: Option<PublicationPeers>,
}

#[must_use = "Check completed publication before accepting the next frontier"]
pub(super) struct PendingPublication<'a, const FIELDS: usize> {
    command: metal::CommandBuffer,
    publication: CompletedPublication<'a, FIELDS>,
    headers: [MappedBuffer; 3],
}
impl<const FIELDS: usize> PendingPublication<'_, FIELDS> {
    pub(super) fn check_completed(self) -> Result<(), Error> {
        super::metal_backend::launch::check_submission()?;
        if self.command.status() != metal::MTLCommandBufferStatus::Completed {
            return Err(Error::BackendFailure);
        }
        let status = unsafe {
            self.publication
                .status
                .buffer
                .contents()
                .cast::<[u32; 4]>()
                .read()
        };
        match status[0] {
            0 => {}
            1 => return Err(Error::BackendCapacity),
            _ => return Err(Error::BackendFailure),
        }
        if status[1..3] != [u32::MAX, u32::MAX] || status[3] > 3 {
            return Err(Error::BackendFailure);
        }
        let capacity = self.publication.capacity;
        let states = u32::try_from(capacity.states).map_err(|_| Error::BackendCapacity)?;
        let [cursors, repeats, captures, digits] = [
            capacity.cursors,
            capacity.repeats,
            capacity.captures,
            capacity.digits,
        ]
        .map(|v| u32::try_from(v).map_err(|_| Error::BackendCapacity));
        let state_header = unsafe { self.headers[0].buffer.contents().cast::<[u32; 5]>().read() };
        let value_header = unsafe { self.headers[1].buffer.contents().cast::<[u32; 2]>().read() };
        let control_header = unsafe { self.headers[2].buffer.contents().cast::<u32>().read() };
        if state_header != [states, captures?, states.div_ceil(256), 1, 1]
            || value_header != [cursors?, repeats?]
            || control_header != digits?
        {
            return Err(Error::BackendFailure);
        }
        Ok(())
    }
}
impl<'a, const FIELDS: usize> CompletedPublication<'a, FIELDS> {
    pub(super) fn capacity(&self) -> CountLimits {
        self.capacity
    }
    pub(super) fn encode(
        self,
        command: &CommandBufferRef,
        output: SuccessorOutput<'_>,
    ) -> Result<PendingPublication<'a, FIELDS>, Error> {
        validate_output(&self.input, &output)?;
        let limits = CountLimits::from_output(&output);
        if self.capacity.states > limits.states
            || self.capacity.cursors > limits.cursors
            || self.capacity.repeats > limits.repeats
            || self.capacity.captures > limits.captures
            || self.capacity.digits > limits.digits
        {
            return Err(Error::BackendCapacity);
        }
        // These peer arguments are unreachable when the completed producer has
        // no pending offers. Their slots retain an existing unread binding.
        let (tasks, offers, alive) = self
            .peers
            .as_ref()
            .map_or((&self.meta, &self.meta, &self.meta), |peers| {
                (&peers.tasks, &peers.offers, &peers.alive)
            });
        encode_publication(
            command,
            &self.source,
            &self.parameters,
            PublicationView {
                columns: &self.columns.buffer,
                spans: &self.spans.buffer,
                candidates: &self.candidates.buffer,
                total: &self.total.buffer,
                sizes: &self.sizes.buffer,
                offsets: &self.offsets.buffer,
                sums: &self.sums.buffer,
                tasks: &tasks.buffer,
                offers: &offers.buffer,
                meta: &self.meta.buffer,
                alive: &alive.buffer,
                status: &self.status.buffer,
                steps: &self.steps.buffer,
                cursors: &self.cursors.buffer,
                repeats: &self.repeats.buffer,
                captures: &self.captures.buffer,
                digits: &self.digits.buffer,
            },
            &output,
            self.threads,
        )?;
        let retain = |buffer: &MappedBuffer| MappedBuffer {
            buffer: buffer.buffer.clone(),
        };
        let headers = [
            retain(&output.states.counts),
            retain(&output.values.counts),
            retain(&output.control.count),
        ];
        Ok(PendingPublication {
            command: command.to_owned(),
            publication: self,
            headers,
        })
    }
}

struct PublicationView<'a> {
    columns: &'a BufferRef,
    spans: &'a BufferRef,
    candidates: &'a BufferRef,
    total: &'a BufferRef,
    sizes: &'a BufferRef,
    offsets: &'a BufferRef,
    sums: &'a BufferRef,
    tasks: &'a BufferRef,
    offers: &'a BufferRef,
    meta: &'a BufferRef,
    alive: &'a BufferRef,
    status: &'a BufferRef,
    steps: &'a BufferRef,
    cursors: &'a BufferRef,
    repeats: &'a BufferRef,
    captures: &'a BufferRef,
    digits: &'a BufferRef,
}
fn encode_publication(
    command: &CommandBufferRef,
    source: &str,
    parameters: &[u32],
    input: PublicationView<'_>,
    output: &SuccessorOutput<'_>,
    threads: usize,
) -> Result<(), Error> {
    let name = "successor_scatter";
    super::metal_parallel::encode(
        command,
        &super::metal_backend::pipeline_named(source, name)?,
        name,
        &[
            input.columns,
            input.spans,
            input.candidates,
            input.total,
            input.sizes,
            input.offsets,
            input.sums,
            input.tasks,
            input.offers,
            input.meta,
            input.alive,
            &output.states.columns.buffer,
            &output.control.spans.buffer,
            &output.states.counts.buffer,
            &output.values.counts.buffer,
            &output.control.count.buffer,
            input.status,
            input.steps,
            &output.control.path_work.buffer,
            input.cursors,
            input.repeats,
            input.captures,
            input.digits,
            &output.values.cursors.buffer,
            &output.values.repeats.buffer,
            &output.values.captures.buffer,
            &output.control.digits.buffer,
        ],
        parameters,
        threads.div_ceil(256).max(1),
        None,
    );
    Ok(())
}
