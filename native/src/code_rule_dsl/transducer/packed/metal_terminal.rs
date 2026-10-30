//! Terminal admission and structural candidate publication. Raw ordinals precede filtering.
use super::Error;
use super::metal_backend::launch::{commit_checked, wait_checked};
use super::metal_backend::{MappedBuffer, pipeline_named};
use super::metal_frontier_reduce::{self, CountedFrontier, PendingReduction};
use super::metal_frontier_validation::ValidatedInput;
use super::metal_parallel::{Scan, encode as dispatch};
use super::metal_position_order::{self, OrderedFrontier, PendingOrder};
use super::metal_state_values::{self, PendingValues, StateValues, ValueFrontier, ValueGroups};
use super::metal_successor::SuccessorInput;
use metal::{CommandBufferRef, CommandQueueRef, Device};
use std::borrow::Borrow;
use std::rc::Rc;
#[path = "metal_terminal_publication_sort.rs"]
pub(super) mod publication_sort;

pub(super) struct TerminalOutput {
    pub results: MappedBuffer,
    pub captures: MappedBuffer,
    /// GPU-owned [result count, capture count].
    pub counts: MappedBuffer,
    pub capacity: usize,
    pub capture_capacity: usize,
    pub generation: u64,
}
#[must_use = "Check terminal publication before accepting candidate records"]
pub(super) struct PendingTerminal {
    validation: Option<Rc<ValidatedInput>>,
    status: MappedBuffer,
    order: Option<PendingOrder>,
    grouping: Option<PendingValues>,
    containment: Option<PendingReduction>,
    _ordered: Option<ValueFrontier>,
    _groups: Option<ValueGroups>,
    _frontiers: Vec<CountedFrontier>,
    _scans: Vec<Scan>,
    _buffers: Vec<MappedBuffer>,
    errors: MappedBuffer,
    program_lengths: Vec<u32>,
}
impl PendingTerminal {
    pub(super) fn check_completed(
        self,
    ) -> Result<super::metal_failure_records::TerminalCompletion, Error> {
        if let Some(validation) = self.validation {
            validation.check_completed()?;
        }
        check_status(&self.status)?;
        if let Some(order) = self.order {
            order.check_completed()?;
        }
        if let Some(grouping) = self.grouping {
            grouping.check_completed()?;
        }
        if let Some(containment) = self.containment {
            containment.check_completed()?;
        }
        super::metal_failure_records::decode(&self.errors, &self.program_lengths)
    }
}
/// Produce structural candidates for the later predicate/transform stage. This
/// stage never applies predicates, renumbers surviving candidates, or publishes pages.
#[cfg(test)]
pub(super) fn encode(
    device: &Device,
    command: &CommandBufferRef,
    input: &SuccessorInput<'_>,
    output: &TerminalOutput,
) -> Result<PendingTerminal, Error> {
    encode_impl(device, command.to_owned(), None, input, output).map(|(pending, _, _)| pending)
}

/// External immutable owners outlive every terminal stage. No frontier borrow
/// crosses the completed ordering boundary.
#[derive(Clone, Copy)]
pub(super) struct TerminalContext<'a> {
    pub resident: &'a super::metal_grid_plan::ResidentInput,
    pub data: &'a MappedBuffer,
}
pub(super) struct TerminalFrame<'a> {
    pub resident: &'a super::metal_grid_plan::ResidentInput,
    pub plan: &'a super::metal_grid_plan::ExecutionPlan,
    pub data: &'a MappedBuffer,
    pub states: &'a super::metal_state_values::PositionedFrontier,
    pub control: &'a super::metal_successor::ControlValues,
    pub ready: &'a super::metal_ready_batch::ReadyBatch,
}
struct TerminalOrdering {
    ordered: OrderedFrontier,
    status: MappedBuffer,
    order: Option<PendingOrder>,
    validation: Option<Rc<ValidatedInput>>,
}
/// Only checked ordering can create this transfer. The input frontier is absent.
pub(super) struct PreparedTerminal<'a> {
    context: TerminalContext<'a>,
    ordering: TerminalOrdering,
    output: TerminalOutput,
    values: StateValues,
}

pub(super) fn prepare<'a>(
    device: &Device,
    queue: &CommandQueueRef,
    context: TerminalContext<'a>,
    frame: TerminalFrame<'_>,
    values: StateValues,
) -> Result<PreparedTerminal<'a>, Error> {
    let input = SuccessorInput {
        resident: frame.resident,
        plan: frame.plan,
        data: frame.data,
        states: frame.states,
        values: &values,
        control: frame.control,
        ready: frame.ready,
    };
    if !std::ptr::eq(context.resident, input.resident) || !std::ptr::eq(context.data, input.data) {
        return Err(Error::BackendFailure);
    }
    super::metal_successor_prepare::validate_input(&input, &[])?;
    let counts = super::metal_register_execution::read::<u32>(&input.states.counts, 2)?;
    if counts[0] as usize > input.states.capacity
        || counts[1] as usize > input.values.capture_capacity
    {
        return Err(Error::BackendFailure);
    }
    let output = TerminalOutput {
        results: MappedBuffer::zeroed::<u32>(device, 0)?,
        captures: MappedBuffer::zeroed::<u32>(device, 0)?,
        counts: MappedBuffer::zeroed::<u32>(device, 2)?,
        capacity: counts[0] as usize,
        capture_capacity: counts[1] as usize,
        generation: input.states.generation,
    };
    let command = new_command(queue);
    let mut ordering = encode_order(device, &command, &input)?;
    commit_checked(&command)?;
    wait_checked(&command)?;
    let validation = ordering.validation.take().ok_or(Error::BackendFailure)?;
    validation.check_completed()?;
    ordering
        .order
        .take()
        .ok_or(Error::BackendFailure)?
        .check_completed()?;
    drop(validation);
    Ok(PreparedTerminal {
        context,
        ordering,
        output,
        values,
    })
}

pub(super) fn execute_prepared(
    device: &Device,
    queue: &CommandQueueRef,
    prepared: PreparedTerminal<'_>,
) -> Result<
    (
        TerminalOutput,
        super::metal_failure_records::TerminalCompletion,
    ),
    Error,
> {
    metal::objc::rc::autoreleasepool(|| {
        let (pending, command, allocated) = encode_ordered(
            device,
            new_command(queue),
            Some(queue),
            prepared.context,
            prepared.values,
            &prepared.output,
            prepared.ordering,
        )?;
        commit_checked(&command)?;
        wait_checked(&command)?;
        let completion = pending.check_completed()?;
        Ok((allocated.ok_or(Error::BackendFailure)?, completion))
    })
}

/// Borrowed fixtures exercise the same physical stages while retaining caller
/// ownership. Production transfers its values to execute_prepared instead.
#[cfg(test)]
pub(super) fn execute(
    device: &Device,
    queue: &CommandQueueRef,
    input: &SuccessorInput<'_>,
) -> Result<
    (
        TerminalOutput,
        super::metal_failure_records::TerminalCompletion,
    ),
    Error,
> {
    let clone_buffer = |buffer: &MappedBuffer| MappedBuffer {
        buffer: buffer.buffer.clone(),
    };
    let values = StateValues {
        cursors: clone_buffer(&input.values.cursors),
        repeats: clone_buffer(&input.values.repeats),
        captures: clone_buffer(&input.values.captures),
        counts: clone_buffer(&input.values.counts),
        cursor_capacity: input.values.cursor_capacity,
        repeat_capacity: input.values.repeat_capacity,
        capture_capacity: input.values.capture_capacity,
        generation: input.values.generation,
    };
    let prepared = prepare(
        device,
        queue,
        TerminalContext {
            resident: input.resident,
            data: input.data,
        },
        TerminalFrame {
            resident: input.resident,
            plan: input.plan,
            data: input.data,
            states: input.states,
            control: input.control,
            ready: input.ready,
        },
        values,
    )?;
    execute_prepared(device, queue, prepared)
}

fn encode_order(
    device: &Device,
    command: &CommandBufferRef,
    input: &SuccessorInput<'_>,
) -> Result<TerminalOrdering, Error> {
    let capacity = input.states.capacity;
    let ordered = OrderedFrontier {
        states: ValueFrontier {
            columns: MappedBuffer::zeroed::<[u32; 13]>(device, capacity)?,
            counts: MappedBuffer::zeroed::<u32>(device, 5)?,
            capacity,
            generation: input.states.generation,
        },
        originals: MappedBuffer::zeroed::<u32>(device, capacity)?,
    };
    let order = metal_position_order::encode_terminals(device, command, input, &ordered)?;
    let validation = order.validation(input, command)?;
    let status = MappedBuffer {
        buffer: validation.status.buffer.clone(),
    };
    Ok(TerminalOrdering {
        ordered,
        status,
        order: Some(order),
        validation: Some(validation),
    })
}

#[cfg(test)]
fn encode_impl(
    device: &Device,
    command: metal::CommandBuffer,
    queue: Option<&CommandQueueRef>,
    input: &SuccessorInput<'_>,
    output: &TerminalOutput,
) -> Result<
    (
        PendingTerminal,
        metal::CommandBuffer,
        Option<TerminalOutput>,
    ),
    Error,
> {
    if queue.is_some() || output.generation != input.states.generation {
        return Err(Error::BackendFailure);
    }
    super::metal_successor_prepare::validate_input(
        input,
        &[
            (&output.results, output.capacity, 40),
            (&output.captures, output.capture_capacity, 36),
            (&output.counts, 2, 4),
        ],
    )?;
    let ordering = encode_order(device, &command, input)?;
    encode_ordered(
        device,
        command,
        None,
        TerminalContext {
            resident: input.resident,
            data: input.data,
        },
        input.values,
        output,
        ordering,
    )
}

fn encode_ordered<V: Borrow<StateValues>>(
    device: &Device,
    mut command: metal::CommandBuffer,
    queue: Option<&CommandQueueRef>,
    context: TerminalContext<'_>,
    values: V,
    output: &TerminalOutput,
    ordering: TerminalOrdering,
) -> Result<
    (
        PendingTerminal,
        metal::CommandBuffer,
        Option<TerminalOutput>,
    ),
    Error,
> {
    let TerminalOrdering {
        ordered,
        status,
        order,
        validation,
    } = ordering;
    let capacity = ordered.states.capacity;
    let generation = ordered.states.generation;
    if output.generation != generation {
        return Err(Error::BackendFailure);
    }
    let OrderedFrontier {
        states: ordered,
        originals,
    } = ordered;
    let originals = queue.is_none().then_some(originals);
    let capture_capacity = values.borrow().capture_capacity;
    let captures = MappedBuffer {
        buffer: values.borrow().captures.buffer.clone(),
    };
    let groups = ValueGroups {
        representatives: MappedBuffer::zeroed::<u32>(device, capacity)?,
        counts: MappedBuffer::zeroed::<u32>(device, 5)?,
        capacity,
        generation,
    };
    // Buckets distribute comparison work. Complete values still decide equality.
    // Keep expected bucket occupancy bounded as the resident barrier grows.
    // The reducer already supports twenty bits. Its exact comparison guard and
    // complete-value equality remain unchanged.
    let bits = capacity.max(1).next_power_of_two().ilog2().min(20);
    let (grouping, grouped) = if let Some(queue) = queue {
        (
            None,
            Some(metal_frontier_reduce::execute_value_groups(
                device,
                queue,
                &ordered,
                &groups,
                values.borrow(),
                bits,
            )?),
        )
    } else {
        (
            Some(metal_state_values::encode_groups(
                device,
                &command,
                &ordered,
                &groups,
                values.borrow(),
                bits,
            )?),
            None,
        )
    };
    let group_status = if let Some(pending) = &grouping {
        pending.status_header()
    } else {
        &*grouped.as_ref().ok_or(Error::BackendFailure)?.header.buffer
    };
    // Complete grouping no longer reads cursor or repeat arenas. For owned
    // production input this retires them; conformance input remains caller-owned.
    drop(values);
    let frontier = || -> Result<CountedFrontier, Error> {
        Ok(CountedFrontier {
            columns: MappedBuffer::zeroed::<[u32; 11]>(device, capacity)?,
            counts: MappedBuffer::zeroed::<u32>(device, 5)?,
            capacity,
            generation,
        })
    };
    let projected = frontier()?;
    // Task, immutable Program index, and terminal PC. No admission identity or
    // branch position is reinterpreted as provenance.
    let sources = MappedBuffer::zeroed::<[u32; 3]>(device, capacity)?;
    let ordered_counts = MappedBuffer {
        buffer: ordered.counts.buffer.clone(),
    };
    let admission = Scan::new(device, capacity)?;
    let task_starts = MappedBuffer::zeroed::<u32>(device, capacity)?;
    let admitted = MappedBuffer::zeroed::<u32>(device, capacity)?;
    let evaluations = context.resident.input.grid.evaluation_spans.len();
    let errors = MappedBuffer::zeroed::<super::metal_failure_records::EvaluationFailureRecord>(
        device,
        evaluations,
    )?;
    let failure_winners = MappedBuffer::from_slice(device, &vec![u32::MAX; evaluations])?;
    let source = concat!(
        include_str!("metal_resident.metal"),
        include_str!("metal_terminal.metal"),
        include_str!("metal_failure_records.metal")
    );
    let mut p = [
        capacity,
        output.capacity,
        output.capture_capacity,
        context.resident.input.grid.semantic_span_order.len(),
        0,
    ]
    .map(|v| u32::try_from(v).map_err(|_| Error::BackendCapacity))
    .into_iter()
    .collect::<Result<Vec<_>, _>>()?;
    let run = |command: &CommandBufferRef,
               name,
               buffers: &[&metal::BufferRef],
               p: &[u32],
               count: usize|
     -> Result<(), Error> {
        dispatch(
            command,
            &pipeline_named(source, name)?,
            name,
            buffers,
            p,
            count.div_ceil(256).max(1),
            None,
        );
        Ok(())
    };
    let s = &ordered.columns.buffer;
    let h = &ordered_counts.buffer;
    let st = &status.buffer;
    run(
        &command,
        "evaluation_failure_first",
        &[&context.data.buffer, s, h, &failure_winners.buffer, st],
        &p,
        capacity,
    )?;
    run(
        &command,
        "evaluation_failure_records",
        &[s, h, &failure_winners.buffer, &errors.buffer, st],
        &[u32::try_from(evaluations).map_err(|_| Error::BackendCapacity)?],
        evaluations,
    )?;
    run(
        &command,
        "terminal_admission_flags",
        &[
            h,
            &groups.representatives.buffer,
            group_status,
            &admission.values.buffer,
            st,
            &context.data.buffer,
            s,
            &errors.buffer,
        ],
        &p,
        capacity,
    )?;
    admission.encode(&command)?;
    run(
        &command,
        "terminal_task_starts",
        &[s, h, &admission.values.buffer, &task_starts.buffer, st],
        &p,
        capacity,
    )?;
    run(
        &command,
        "terminal_project",
        &[
            s,
            h,
            &groups.representatives.buffer,
            &admission.values.buffer,
            admission.total(),
            &task_starts.buffer,
            &projected.columns.buffer,
            &projected.counts.buffer,
            &admitted.buffer,
            st,
            &sources.buffer,
        ],
        &p,
        capacity,
    )?;
    let admitted_total = MappedBuffer {
        buffer: admission.total().to_owned(),
    };
    let groups = if let Some(queue) = queue {
        commit_checked(&command)?;
        wait_checked(&command)?;
        check_status(&status)?;
        command = new_command(queue);
        drop(groups);
        drop(grouped);
        None
    } else {
        Some(groups)
    };
    let admission = queue.is_none().then_some(admission);
    let ordered = queue.is_none().then_some(ordered);
    // Projection has assigned raw ordinals. Only now may a retained maximal
    // query row prove containment, before the general exact pair reducer.
    let projected = if let Some(queue) = queue {
        anchor_projection(device, queue, projected, &captures, capture_capacity)?
    } else {
        projected
    };
    let survivors = CountedFrontier {
        columns: MappedBuffer::zeroed::<[u32; 11]>(device, projected.capacity)?,
        counts: MappedBuffer::zeroed::<u32>(device, 5)?,
        capacity: projected.capacity,
        generation,
    };
    let (containment, contained) = if let Some(queue) = queue {
        (
            None,
            Some(metal_frontier_reduce::execute_completed(
                device,
                queue,
                &projected,
                &survivors,
                &captures,
                capture_capacity,
                bits,
            )?),
        )
    } else {
        (
            Some(metal_frontier_reduce::encode_completed(
                device,
                &command,
                &projected,
                &survivors,
                &captures,
                capture_capacity,
                bits,
            )?),
            None,
        )
    };
    let projected = if queue.is_some() {
        drop(projected);
        None
    } else {
        Some(projected)
    };
    let containment_status = if let Some(pending) = &containment {
        pending.status_header()
    } else {
        &*contained
            .as_ref()
            .ok_or(Error::BackendFailure)?
            .header
            .buffer
    };
    // Exact reduction has completed in staged execution. Its row count prices
    // publication work; a singleton does not need capacity-sized merge passes.
    let publication_capacity = if queue.is_some() {
        let counts = unsafe {
            std::slice::from_raw_parts(survivors.counts.buffer.contents().cast::<u32>(), 5)
        };
        let count = counts[0] as usize;
        if count > survivors.capacity
            || counts[1] as usize > capture_capacity
            || counts[2] as usize != count.div_ceil(256)
            || counts[3..] != [1, 1]
        {
            return Err(Error::BackendFailure);
        }
        count
    } else {
        capacity
    };
    p[0] = u32::try_from(publication_capacity).map_err(|_| Error::BackendCapacity)?;
    let publication = Scan::new(device, publication_capacity)?;
    let metadata = MappedBuffer::zeroed::<[u32; 4]>(device, publication_capacity)?;
    let ids = [
        MappedBuffer::zeroed::<u32>(device, publication_capacity)?,
        MappedBuffer::zeroed::<u32>(device, publication_capacity)?,
    ];
    run(
        &command,
        "terminal_candidates",
        &[
            &context.data.buffer,
            &sources.buffer,
            h,
            &survivors.columns.buffer,
            &survivors.counts.buffer,
            &task_starts.buffer,
            &admitted.buffer,
            &captures.buffer,
            containment_status,
            &metadata.buffer,
            &ids[0].buffer,
            &output.counts.buffer,
            st,
            &admitted_total.buffer,
        ],
        &p,
        publication_capacity,
    )?;
    let mut metadata_inputs = Some([task_starts, admitted, failure_winners, admitted_total]);
    let mut current = publication_sort::encode_tiles(
        &command,
        &survivors.counts.buffer,
        &metadata.buffer,
        [&ids[0].buffer, &ids[1].buffer],
        st,
        publication_capacity,
    )?;
    let mut width = 256;
    while width < publication_capacity {
        p[4] = u32::try_from(width).map_err(|_| Error::BackendCapacity)?;
        run(
            &command,
            "terminal_publication_merge",
            &[
                &survivors.counts.buffer,
                &metadata.buffer,
                &ids[current].buffer,
                &ids[1 - current].buffer,
                st,
            ],
            &p,
            publication_capacity,
        )?;
        current = 1 - current;
        width = width.checked_mul(2).ok_or(Error::BackendCapacity)?;
    }
    run(
        &command,
        "terminal_publication_sizes",
        &[
            &survivors.columns.buffer,
            &survivors.counts.buffer,
            &ids[current].buffer,
            &publication.values.buffer,
            st,
            &context.data.buffer,
            &metadata.buffer,
        ],
        &p,
        publication_capacity,
    )?;
    publication.encode(&command)?;
    let allocated = if let Some(queue) = queue {
        commit_checked(&command)?;
        wait_checked(&command)?;
        check_status(&status)?;
        let counts = unsafe { publication.total().contents().cast::<[u32; 4]>().read() };
        if counts[0] == u32::MAX
            || counts[1] == u32::MAX
            || counts[0] as usize > output.capacity
            || counts[1] as usize > output.capture_capacity
        {
            return Err(Error::BackendCapacity);
        }
        if counts[2..] != [0, 0] {
            return Err(Error::BackendFailure);
        }
        command = new_command(queue);
        // The completed metadata command consumed these inputs. Allocation of
        // literal output must not retain its admission-only predecessors.
        drop(metadata_inputs.take());
        if std::env::var_os("TEMPO_PACKED_BATCH_PROFILE").is_some() {
            eprintln!(
                "TEMPO_METAL_TERMINAL_COUNTS states={capacity} results={} captures={} residentBytes={}",
                counts[0],
                counts[1],
                super::metal_backend::buffer_usage().0
            );
        }
        p[1] = counts[0];
        p[2] = counts[1];
        let allocated = TerminalOutput {
            results: MappedBuffer::zeroed::<super::metal_batch_abi::ResultRecord>(
                device,
                counts[0] as usize,
            )?,
            captures: MappedBuffer::zeroed::<super::metal_batch_abi::FinalCaptureRecord>(
                device,
                counts[1] as usize,
            )?,
            counts: MappedBuffer {
                buffer: output.counts.buffer.clone(),
            },
            capacity: counts[0] as usize,
            capture_capacity: counts[1] as usize,
            generation: output.generation,
        };
        validate_publication(&allocated)?;
        Some(allocated)
    } else {
        None
    };
    let output = allocated.as_ref().unwrap_or(output);

    run(
        &command,
        "terminal_publish",
        &[
            &context.data.buffer,
            &sources.buffer,
            h,
            &survivors.columns.buffer,
            &survivors.counts.buffer,
            &metadata.buffer,
            &ids[current].buffer,
            &publication.values.buffer,
            publication.total(),
            &output.counts.buffer,
            &output.results.buffer,
            &captures.buffer,
            &output.captures.buffer,
            st,
        ],
        &p,
        publication_capacity.max(output.capture_capacity),
    )?;
    let [first, second] = ids;
    let mut retained = vec![metadata, first, second, sources, ordered_counts, captures];
    retained.extend(metadata_inputs.into_iter().flatten());
    retained.extend(originals);
    if let Some(contained) = contained {
        retained.push(contained.header);
    }
    Ok((
        PendingTerminal {
            validation,
            status,
            order,
            grouping,
            containment,
            _ordered: ordered,
            _groups: groups,
            _frontiers: projected.into_iter().chain([survivors]).collect(),
            _scans: admission.into_iter().chain([publication]).collect(),
            _buffers: retained,
            errors,
            program_lengths: context
                .resident
                .input
                .grid
                .evaluation_spans
                .iter()
                .map(|evaluation| {
                    context.resident.input.packed.programs[evaluation.program as usize]
                        .structural_count
                })
                .collect(),
        },
        command,
        allocated,
    ))
}

/// Only terminal projection calls this operation. Its rows have task order,
/// query/ordinary scopes, and contiguous raw ordinals. The GPU checks that shape.
/// Capture equality uses the same ordered geometry relation as exact reduction.
fn anchor_projection(
    device: &Device,
    queue: &CommandQueueRef,
    input: CountedFrontier,
    captures: &MappedBuffer,
    capture_capacity: usize,
) -> Result<CountedFrontier, Error> {
    let source = concat!(
        "#include <metal_stdlib>\nusing namespace metal;\n",
        include_str!("metal_capture_geometry.metal"),
        include_str!("metal_terminal_anchor.metal"),
    );
    let anchors = MappedBuffer::zeroed::<u32>(device, input.capacity)?;
    let flags = Scan::new(device, input.capacity)?;
    let status = MappedBuffer::zeroed::<u32>(device, 1)?;
    let mut parameters = [input.capacity, capture_capacity, 0]
        .map(|n| u32::try_from(n).map_err(|_| Error::BackendCapacity))
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    let run = |command: &CommandBufferRef,
               name,
               buffers: &[&metal::BufferRef],
               p: &[u32]|
     -> Result<(), Error> {
        dispatch(
            command,
            &pipeline_named(source, name)?,
            name,
            buffers,
            p,
            input.capacity.div_ceil(256).max(1),
            None,
        );
        Ok(())
    };
    metal::objc::rc::autoreleasepool(|| -> Result<(), Error> {
        let command = queue.new_command_buffer();
        run(
            command,
            "terminal_anchor_elect",
            &[
                &input.columns.buffer,
                &input.counts.buffer,
                &anchors.buffer,
                &status.buffer,
            ],
            &parameters,
        )?;
        run(
            command,
            "terminal_anchor_flags",
            &[
                &input.columns.buffer,
                &input.counts.buffer,
                &captures.buffer,
                &anchors.buffer,
                &flags.values.buffer,
                &status.buffer,
            ],
            &parameters,
        )?;
        flags.encode(command)?;
        commit_checked(command)?;
        wait_checked(command)?;
        check_status(&status)
    })?;
    let total = unsafe { flags.total().contents().cast::<[u32; 4]>().read() };
    if total[0] == u32::MAX || total[0] as usize > input.capacity || total[1..] != [0, 0, 0] {
        return Err(Error::BackendFailure);
    }
    let capacity = total[0] as usize;
    parameters[2] = total[0];
    let output = CountedFrontier {
        columns: MappedBuffer::zeroed::<[u32; 11]>(device, capacity)?,
        counts: MappedBuffer::zeroed::<u32>(device, 5)?,
        capacity,
        generation: input.generation,
    };
    metal::objc::rc::autoreleasepool(|| -> Result<(), Error> {
        let command = queue.new_command_buffer();
        run(
            command,
            "terminal_anchor_scatter",
            &[
                &input.columns.buffer,
                &input.counts.buffer,
                &flags.values.buffer,
                flags.total(),
                &output.columns.buffer,
                &output.counts.buffer,
                &status.buffer,
            ],
            &parameters,
        )?;
        commit_checked(command)?;
        wait_checked(command)?;
        check_status(&status)
    })?;
    Ok(output)
}

fn new_command(queue: &CommandQueueRef) -> metal::CommandBuffer {
    // The owned command releases retained resources at its completed boundary.
    metal::objc::rc::autoreleasepool(|| queue.new_command_buffer().to_owned())
}

fn check_status(status: &MappedBuffer) -> Result<(), Error> {
    super::metal_backend::launch::check_submission()?;
    match unsafe { status.buffer.contents().cast::<u32>().read() } {
        0 => Ok(()),
        1 => Err(Error::BackendCapacity),
        _ => Err(Error::BackendFailure),
    }
}

fn validate_publication(output: &TerminalOutput) -> Result<(), Error> {
    let buffers = [
        (&output.results, output.capacity, 40usize),
        (&output.captures, output.capture_capacity, 36),
        (&output.counts, 2, 4),
    ];
    for (index, &(buffer, count, width)) in buffers.iter().enumerate() {
        u32::try_from(count).map_err(|_| Error::BackendCapacity)?;
        let bytes = count.checked_mul(width).ok_or(Error::BackendCapacity)?;
        if bytes as u64 > buffer.buffer.length()
            || buffers[index + 1..]
                .iter()
                .any(|(other, _, _)| std::ptr::eq(&*buffer.buffer, &*other.buffer))
        {
            return Err(Error::BackendFailure);
        }
    }
    Ok(())
}

#[cfg(all(test, feature = "metal-tests"))]
#[path = "metal_terminal_anchor_tests.rs"]
mod anchor_tests;
