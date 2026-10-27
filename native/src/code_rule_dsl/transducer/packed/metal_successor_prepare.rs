//! Count bounded successor work before the resident owner allocates its arenas.
use super::Error;
use super::metal_backend::{MappedBuffer, pipeline_named};
use super::metal_parallel::{Scan, encode as dispatch};
use super::metal_state_values::{INITIAL_COLUMNS, POSITIONED_COLUMNS};
use super::metal_successor::SuccessorInput;
use super::metal_successor_codegen::{self, Code};
use metal::{CommandBufferRef, Device};

#[path = "metal_successor_register.rs"]
mod register;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Capacity {
    pub states: usize,
    /// A linear upper bound for each arena, not a valid payload length.
    pub cells: usize,
}

#[derive(Clone, Copy)]
pub(super) struct ComparisonNeeds {
    pub active: bool,
    pub pending: bool,
    /// Completed GPU upper bound for exact-query candidates. A shared command
    /// cannot expose its count yet and reserves against all output candidates.
    pub active_states: Option<usize>,
}

pub(super) struct PreparedSuccessors<const FIELDS: usize = POSITIONED_COLUMNS> {
    command: metal::CommandBuffer,
    pub status: MappedBuffer,
    pub steps: MappedBuffer,
    pub expansion: Scan,
    pub code: Code,
    pub parameters: Vec<u32>,
    bindings: [usize; 13],
    generation: u64,
    resident: usize,
    plan: usize,
    _scans: Vec<Scan>,
    _buffers: Vec<MappedBuffer>,
}

/// Completed frame storage with no retained producer command or input readers.
pub(super) struct CompletedSteps {
    pub status: MappedBuffer,
    pub steps: MappedBuffer,
    pub expansion: Scan,
    pub code: Code,
    pub parameters: Vec<u32>,
}
impl<const FIELDS: usize> PreparedSuccessors<FIELDS> {
    pub(super) fn into_completed(
        self,
        input: &SuccessorInput<'_, FIELDS>,
    ) -> Result<CompletedSteps, Error> {
        self.check_completed()?;
        self.validate_binding(input, &self.command)?;
        Ok(CompletedSteps {
            status: self.status,
            steps: self.steps,
            expansion: self.expansion,
            code: self.code,
            parameters: self.parameters,
        })
    }
}

/// The completed input scan owns exact logical job offsets and signature groups.
/// No Step arena exists until this producer proves its scalar allocation count.
pub(super) struct CountedInputs<const FIELDS: usize = POSITIONED_COLUMNS> {
    command: metal::CommandBuffer,
    status: MappedBuffer,
    signatures: Scan,
    code: Code,
    parameters: Vec<u32>,
    categories: MappedBuffer,
    sites: MappedBuffer,
    slots: MappedBuffer,
    bindings: [usize; 13],
    generation: u64,
    resident: usize,
    plan: usize,
}

/// The grouped producer converts completed scan offsets into bounded jobs.
/// Its input scan retires before any finite frame arena exists.
pub(super) struct GroupedInputs<const FIELDS: usize = POSITIONED_COLUMNS> {
    command: metal::CommandBuffer,
    counted: CountedInputs<FIELDS>,
    items: MappedBuffer,
    groups: Scan,
    arguments: MappedBuffer,
}

impl<const FIELDS: usize> GroupedInputs<FIELDS> {
    pub(super) fn check_completed(&self) -> Result<usize, Error> {
        super::metal_backend::launch::check_submission()?;
        if self.command.status() != metal::MTLCommandBufferStatus::Completed {
            return Err(Error::BackendFailure);
        }
        self.counted.check_completed()
    }
}
impl<const FIELDS: usize> CountedInputs<FIELDS> {
    pub(super) fn check_completed(&self) -> Result<usize, Error> {
        self.completed_counts().map(|counts| counts[0])
    }
    fn completed_counts(&self) -> Result<[usize; 2], Error> {
        super::metal_backend::launch::check_submission()?;
        if self.command.status() != metal::MTLCommandBufferStatus::Completed {
            return Err(Error::BackendFailure);
        }
        let status =
            unsafe { std::slice::from_raw_parts(self.status.buffer.contents().cast::<u32>(), 4) };
        match status[0] {
            0 => {}
            1 => return Err(Error::BackendCapacity),
            _ => return Err(Error::BackendFailure),
        }
        if status[1..] != [u32::MAX, u32::MAX, 0] {
            return Err(Error::BackendFailure);
        }
        let total = unsafe { self.signatures.total().contents().cast::<[u32; 4]>().read() };
        if total.contains(&u32::MAX) {
            return Err(Error::BackendCapacity);
        }
        let maximum = (self.parameters[0] as usize)
            .checked_mul(self.code.variants)
            .ok_or(Error::BackendCapacity)?;
        let minimum_words = (total[0] as usize)
            .checked_mul(metal_successor_codegen::STEP_HEADER_WORDS)
            .ok_or(Error::BackendCapacity)?;
        let maximum_words = (total[0] as usize)
            .checked_mul(self.code.maximum_step_words)
            .ok_or(Error::BackendCapacity)?;
        if total[0] != total[1]
            || total[3] != 0
            || total[0] as usize > maximum
            || !(minimum_words..=maximum_words).contains(&(total[2] as usize))
            || !total[2].is_multiple_of(4)
        {
            return Err(Error::BackendFailure);
        }
        Ok([total[0] as usize, total[2] as usize])
    }
    fn validate_binding(&self, input: &SuccessorInput<'_, FIELDS>) -> Result<(), Error> {
        if self.bindings != bindings(input)
            || self.generation != input.states.generation
            || self.resident != input.resident as *const _ as usize
            || self.plan != input.plan as *const _ as usize
            || self.parameters[0] as usize != input.states.capacity
            || self.parameters[6] as usize != input.values.cursor_capacity
            || self.parameters[7] as usize != input.values.repeat_capacity
            || self.parameters[8] as usize != input.values.capture_capacity
            || self.parameters[9] as usize != input.control.digit_capacity
        {
            return Err(Error::BackendFailure);
        }
        Ok(())
    }
}

fn bindings<const FIELDS: usize>(input: &SuccessorInput<'_, FIELDS>) -> [usize; 13] {
    [
        &input.states.columns,
        &input.states.counts,
        &input.values.cursors,
        &input.values.repeats,
        &input.values.captures,
        &input.values.counts,
        &input.control.spans,
        &input.control.digits,
        &input.control.count,
        &input.control.path_work,
        &input.ready.indices,
        &input.ready.counts,
        input.data,
    ]
    .map(|b| (&*b.buffer as *const metal::BufferRef) as usize)
}

pub(super) fn validate_input<const FIELDS: usize>(
    input: &SuccessorInput<'_, FIELDS>,
    extra: &[(&MappedBuffer, usize, usize)],
) -> Result<(), Error> {
    input.plan.validate(input.resident)?;
    if ![INITIAL_COLUMNS, POSITIONED_COLUMNS].contains(&FIELDS)
        || (FIELDS == INITIAL_COLUMNS && input.control.digit_capacity != 0)
        || input.control.generation != input.states.generation
        || input.ready.generation != input.states.generation
        || input.control.state_capacity != input.states.capacity
        || input.ready.capacity != input.states.capacity
    {
        return Err(Error::BackendFailure);
    }
    let mut buffers = vec![
        (
            &input.control.spans,
            input.control.state_capacity,
            if FIELDS == INITIAL_COLUMNS { 0 } else { 24 },
        ),
        (&input.control.digits, input.control.digit_capacity, 4),
        (&input.control.count, 1, 4),
        (
            &input.control.path_work,
            input.control.state_capacity,
            if FIELDS == INITIAL_COLUMNS { 0 } else { 4 },
        ),
        (
            &input.ready.indices,
            input.ready.capacity,
            if FIELDS == INITIAL_COLUMNS { 4 } else { 16 },
        ),
        (&input.ready.counts, 17, 4),
        (input.data, 0, 1),
    ];
    buffers.extend_from_slice(extra);
    super::metal_state_values::validate_storage(
        input.states,
        input.values,
        &buffers,
        input.states.generation,
    )
}

impl<const FIELDS: usize> PreparedSuccessors<FIELDS> {
    /// Completed bounded work proves whether any candidate needs comparisons.
    /// An encoder that shares the preparation command retains the full stages.
    pub(super) fn comparison_needs(&self) -> Result<ComparisonNeeds, Error> {
        if self.command.status() != metal::MTLCommandBufferStatus::Completed {
            return Ok(ComparisonNeeds {
                active: true,
                pending: true,
                active_states: None,
            });
        }
        self.check_completed()?;
        let flags = unsafe { self.status.buffer.contents().cast::<u32>().add(3).read() };
        Ok(ComparisonNeeds {
            active: flags & 1 != 0,
            pending: flags & 2 != 0,
            active_states: Some(unsafe {
                self.expansion
                    .total()
                    .contents()
                    .cast::<u32>()
                    .add(3)
                    .read() as usize
            }),
        })
    }

    pub(super) fn validate_binding(
        &self,
        input: &SuccessorInput<'_, FIELDS>,
        command: &CommandBufferRef,
    ) -> Result<(), Error> {
        if self.bindings != bindings(input)
            || self.generation != input.states.generation
            || self.resident != input.resident as *const _ as usize
            || self.plan != input.plan as *const _ as usize
            || self.parameters[0] as usize != input.states.capacity
            || self.parameters[6] as usize != input.values.cursor_capacity
            || self.parameters[7] as usize != input.values.repeat_capacity
            || self.parameters[8] as usize != input.values.capture_capacity
            || self.parameters[9] as usize != input.control.digit_capacity
            || (!std::ptr::eq(&*self.command, command)
                && self.command.status() != metal::MTLCommandBufferStatus::Completed)
        {
            return Err(Error::BackendFailure);
        }
        Ok(())
    }

    pub(super) fn check_completed(&self) -> Result<Capacity, Error> {
        super::metal_backend::launch::check_submission()?;
        if self.command.status() != metal::MTLCommandBufferStatus::Completed {
            return Err(Error::BackendFailure);
        }
        let status =
            unsafe { std::slice::from_raw_parts(self.status.buffer.contents().cast::<u32>(), 4) };
        match status[0] {
            0 => {}
            1 => return Err(Error::BackendCapacity),
            _ => return Err(Error::BackendFailure),
        }
        if status[1] != u32::MAX {
            return Err(Error::BackendFailure);
        }
        if status[2] != u32::MAX {
            return Err(Error::UnsupportedFeature(
                "Metal shared predicate frontiers are not implemented",
            ));
        }
        if status[3] > 3 {
            return Err(Error::BackendFailure);
        }
        let totals = unsafe {
            std::slice::from_raw_parts(self.expansion.total().contents().cast::<u32>(), 4)
        };
        if totals.contains(&u32::MAX) {
            return Err(Error::BackendCapacity);
        }
        if totals[2] != self.parameters[20]
            || totals[3] > totals[0]
            || (totals[3] != 0) != (status[3] & 1 != 0)
        {
            return Err(Error::BackendFailure);
        }
        Ok(Capacity {
            states: totals[0] as usize,
            cells: totals[1] as usize,
        })
    }
}

/// One complete shader parameter layout for execution and validation callers.
pub(super) fn parameters<const FIELDS: usize>(
    input: &SuccessorInput<'_, FIELDS>,
    code: &Code,
    step_capacity: usize,
) -> Result<Vec<u32>, Error> {
    let bounds = [
        input.states.capacity,
        0,
        input.resident.input.grid.header.semantic_tuple_count as usize,
        input.resident.input.grid.semantic_span_order.len(),
        input.resident.input.packed.programs.len(),
        input.resident.input.packed.nodes.len(),
        input.values.cursor_capacity,
        input.values.repeat_capacity,
        input.values.capture_capacity,
        input.control.digit_capacity,
        0,
        0,
        0,
        0,
        code.signatures,
        0,
        0,
        0,
        step_capacity,
        0, // Current candidate task-group slots; filled after completed counts.
        0, // Completed GPU word count for variable finite-prefix frames.
        0, // Original input/variant jobs for private-frame compaction.
    ];
    bounds
        .into_iter()
        .map(|n| u32::try_from(n).map_err(|_| Error::BackendCapacity))
        .collect::<Result<Vec<_>, _>>()
}

pub(super) fn encode_inputs<const FIELDS: usize>(
    device: &Device,
    command: &CommandBufferRef,
    input: &SuccessorInput<'_, FIELDS>,
) -> Result<CountedInputs<FIELDS>, Error> {
    validate_input(input, &[])?;
    let code = metal_successor_codegen::compile::<FIELDS>(input.resident, &input.plan.readiness)?;
    let capacity = input.states.capacity;
    let parameters = parameters(input, &code, 0)?;
    // x scans signature buckets; y scans logical input/variant positions;
    // z scans the signature-sized frame words for those same logical jobs.
    // Each column has one checked domain. The shorter domain has zero tails.
    let signatures = Scan::new(device, capacity.max(code.signatures + 1))?;
    let status = MappedBuffer::from_slice(device, &[0u32, u32::MAX, u32::MAX, 0])?;
    let categories = MappedBuffer::zeroed::<u32>(device, capacity)?;
    let sites = MappedBuffer::from_slice(device, &code.sites)?;
    let slots = MappedBuffer::zeroed::<[u32; 4]>(device, capacity)?;
    let run = |name, buffers: &[&metal::BufferRef], count: usize, indirect| -> Result<(), Error> {
        let pipeline = pipeline_named(&code.source, name)?;
        dispatch(
            command,
            &pipeline,
            name,
            buffers,
            &parameters,
            count.div_ceil(256).max(1),
            indirect,
        );
        Ok(())
    };
    let h = &input.states.counts.buffer;
    let s = &input.states.columns.buffer;
    let st = &status.buffer;
    run(
        "successor_lists",
        &[
            h,
            &input.ready.indices.buffer,
            &input.ready.counts.buffer,
            &categories.buffer,
            st,
            &input.values.counts.buffer,
            &input.control.count.buffer,
        ],
        capacity
            .checked_mul(if FIELDS == INITIAL_COLUMNS { 1 } else { 4 })
            .ok_or(Error::BackendCapacity)?,
        None,
    )?;
    run(
        "successor_inputs_checked",
        &[
            &input.data.buffer,
            s,
            h,
            &input.values.counts.buffer,
            &input.control.spans.buffer,
            &input.control.count.buffer,
            &categories.buffer,
            &sites.buffer,
            &signatures.values.buffer,
            &slots.buffer,
            st,
            &input.control.path_work.buffer,
            &input.values.cursors.buffer,
            &input.values.repeats.buffer,
            &input.values.captures.buffer,
            &input.control.digits.buffer,
        ],
        capacity.checked_mul(256).ok_or(Error::BackendCapacity)?,
        None,
    )?;
    signatures.encode(command)?;
    Ok(CountedInputs {
        command: command.to_owned(),
        status,
        signatures,
        code,
        parameters,
        categories,
        sites,
        slots,
        bindings: bindings(input),
        generation: input.states.generation,
        resident: input.resident as *const _ as usize,
        plan: input.plan as *const _ as usize,
    })
}

pub(super) fn encode_groups<const FIELDS: usize>(
    device: &Device,
    command: &CommandBufferRef,
    input: &SuccessorInput<'_, FIELDS>,
    mut counted: CountedInputs<FIELDS>,
) -> Result<GroupedInputs<FIELDS>, Error> {
    counted.validate_binding(input)?;
    let [step_capacity, word_capacity] = counted.completed_counts()?;
    counted.parameters[18] = u32::try_from(step_capacity).map_err(|_| Error::BackendCapacity)?;
    counted.parameters[20] = u32::try_from(word_capacity).map_err(|_| Error::BackendCapacity)?;
    let groups = Scan::new(device, counted.code.signatures + 1)?;
    let arguments = MappedBuffer::zeroed::<u32>(device, 3)?;
    let code = &counted.code;
    let signatures = &counted.signatures;
    let st = &counted.status.buffer;
    let run = |name, buffers: &[&metal::BufferRef], count: usize, indirect| -> Result<(), Error> {
        dispatch(
            command,
            &pipeline_named(&code.source, name)?,
            name,
            buffers,
            &counted.parameters,
            count.div_ceil(256).max(1),
            indirect,
        );
        Ok(())
    };
    if code.signatures < 4096 {
        run(
            "successor_groups",
            &[
                &signatures.values.buffer,
                signatures.total(),
                &groups.values.buffer,
                groups.total(),
                &arguments.buffer,
                st,
            ],
            256,
            None,
        )?;
    } else {
        run(
            "successor_group_counts",
            &[
                &signatures.values.buffer,
                signatures.total(),
                &groups.values.buffer,
            ],
            code.signatures + 1,
            None,
        )?;
        groups.encode(command)?;
        run(
            "successor_input_arguments",
            &[groups.total(), &arguments.buffer, st, signatures.total()],
            1,
            None,
        )?;
    }
    let items = MappedBuffer::zeroed::<[u32; 2]>(device, step_capacity)?;
    dispatch(
        command,
        &pipeline_named(&counted.code.source, "successor_group_items")?,
        "successor_group_items",
        &[
            &input.states.counts.buffer,
            &counted.slots.buffer,
            &counted.signatures.values.buffer,
            &items.buffer,
            &counted.status.buffer,
            &groups.values.buffer,
        ],
        &counted.parameters,
        input
            .states
            .capacity
            .max(counted.code.signatures + 1)
            .div_ceil(256)
            .max(1),
        None,
    );
    Ok(GroupedInputs {
        command: command.to_owned(),
        counted,
        items,
        groups,
        arguments,
    })
}

pub(super) fn encode_steps<const FIELDS: usize>(
    device: &Device,
    command: &CommandBufferRef,
    input: &SuccessorInput<'_, FIELDS>,
    grouped: GroupedInputs<FIELDS>,
) -> Result<PreparedSuccessors<FIELDS>, Error> {
    let step_capacity = grouped.check_completed()?;
    grouped.counted.validate_binding(input)?;
    let GroupedInputs {
        command: grouped_command,
        counted,
        items,
        groups,
        arguments,
    } = grouped;
    let CountedInputs {
        command: counted_command,
        status,
        signatures,
        code,
        parameters,
        categories,
        sites,
        slots,
        bindings,
        generation,
        resident,
        plan,
    } = counted;
    // These completed producers have no consumer after the typed group handoff.
    // Explicit drops precede allocation; partial destructuring would retain them.
    drop((grouped_command, counted_command, signatures, categories));
    let word_capacity = parameters[20] as usize;
    let expansion = Scan::new(device, step_capacity)?;
    let steps = MappedBuffer::zeroed::<u32>(device, word_capacity)?;
    let h = &input.states.counts.buffer;
    let s = &input.states.columns.buffer;
    let st = &status.buffer;
    let run = |name, buffers: &[&metal::BufferRef], count: usize, indirect| -> Result<(), Error> {
        dispatch(
            command,
            &pipeline_named(&code.source, name)?,
            name,
            buffers,
            &parameters,
            count.div_ceil(256).max(1),
            indirect,
        );
        Ok(())
    };
    run(
        "successor_prefix",
        &[
            &input.data.buffer,
            s,
            h,
            &input.values.cursors.buffer,
            &input.values.repeats.buffer,
            &slots.buffer,
            &groups.values.buffer,
            groups.total(),
            &items.buffer,
            &steps.buffer,
            &expansion.values.buffer,
            st,
            &input.control.spans.buffer,
            &sites.buffer,
            &input.control.path_work.buffer,
        ],
        0,
        Some((&arguments.buffer, 0)),
    )?;
    expansion.encode(command)?;
    Ok(PreparedSuccessors {
        command: command.to_owned(),
        status,
        steps,
        expansion,
        code,
        parameters,
        bindings,
        generation,
        resident,
        plan,
        _scans: vec![groups],
        _buffers: vec![sites, slots, items, arguments],
    })
}

pub(super) fn finish_inputs<const FIELDS: usize>(
    device: &Device,
    queue: &metal::CommandQueueRef,
    input: &SuccessorInput<'_, FIELDS>,
    counted: CountedInputs<FIELDS>,
) -> Result<PreparedSuccessors<FIELDS>, Error> {
    let counts = counted.completed_counts()?;
    // A large finite job rectangle is counted in private frames first. Small
    // exact arenas keep the composed producer and its lower dispatch count.
    if counts[1].checked_mul(4).ok_or(Error::BackendCapacity)? > 16 * 1024 * 1024 {
        return register::finish(device, queue, input, counted);
    }
    let grouped = metal::objc::rc::autoreleasepool(|| {
        let command = queue.new_command_buffer();
        let grouped = encode_groups(device, command, input, counted)?;
        super::metal_backend::launch::commit_checked(command)?;
        super::metal_backend::launch::wait_checked(command)?;
        grouped.check_completed()?;
        Ok::<_, Error>(grouped)
    })?;
    let mut prepared = metal::objc::rc::autoreleasepool(|| {
        let command = queue.new_command_buffer();
        let prepared = encode_steps(device, command, input, grouped)?;
        super::metal_backend::launch::commit_checked(command)?;
        super::metal_backend::launch::wait_checked(command)?;
        prepared.check_completed()?;
        Ok::<_, Error>(prepared)
    })?;
    // The checked command completed every prefix reader. Successor execution
    // consumes only steps, expansion, and status; retain scratch only for
    // callers that encode both producers and consumers in the same command.
    prepared._scans.clear();
    prepared._buffers.clear();
    Ok(prepared)
}

pub(super) fn prepare<const FIELDS: usize>(
    device: &Device,
    queue: &metal::CommandQueueRef,
    input: &SuccessorInput<'_, FIELDS>,
) -> Result<PreparedSuccessors<FIELDS>, Error> {
    let counted = metal::objc::rc::autoreleasepool(|| {
        let command = queue.new_command_buffer();
        let counted = encode_inputs(device, command, input)?;
        super::metal_backend::launch::commit_checked(command)?;
        super::metal_backend::launch::wait_checked(command)?;
        counted.check_completed()?;
        Ok::<_, Error>(counted)
    })?;
    finish_inputs(device, queue, input, counted)
}

#[cfg(test)]
pub(super) fn prepare_register<const FIELDS: usize>(
    device: &Device,
    queue: &metal::CommandQueueRef,
    input: &SuccessorInput<'_, FIELDS>,
) -> Result<PreparedSuccessors<FIELDS>, Error> {
    let counted = metal::objc::rc::autoreleasepool(|| {
        let command = queue.new_command_buffer();
        let counted = encode_inputs(device, command, input)?;
        super::metal_backend::launch::commit_checked(command)?;
        super::metal_backend::launch::wait_checked(command)?;
        counted.check_completed()?;
        Ok::<_, Error>(counted)
    })?;
    register::finish(device, queue, input, counted)
}
