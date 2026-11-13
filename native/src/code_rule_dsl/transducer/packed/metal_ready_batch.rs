//! Readiness barriers over active root-task work; payloads remain in their arenas.
use super::Error;
use super::metal_backend::MappedBuffer;
use super::metal_grid_plan::{ExecutionPlan, ResidentInput};
use super::metal_state_values::{
    FrontierRecords, INITIAL_COLUMNS, POSITIONED_COLUMNS, StateValues,
};
use metal::{CommandBufferRef, Device};

pub(super) struct ReadyInput<'a, const FIELDS: usize> {
    pub resident: &'a ResidentInput,
    pub plan: &'a ExecutionPlan,
    pub data: &'a MappedBuffer,
    pub states: &'a FrontierRecords<FIELDS>,
    pub values: &'a StateValues,
}
/// Four stable lists of original indices: successors, closure, held, terminals.
/// Each list occupies capacity words. The 17-word header contains four lengths,
/// four uint3 dispatches, then the number of held inline predicates.
/// Initial records concatenate the same four stable lists in capacity words.
pub(super) struct ReadyBatch<const FIELDS: usize = POSITIONED_COLUMNS> {
    pub indices: MappedBuffer,
    pub counts: MappedBuffer,
    pub capacity: usize,
    pub generation: u64,
}
#[must_use = "Check readiness failures after the shared command completes"]
pub(super) struct PendingReady {
    command: metal::CommandBuffer,
    status: MappedBuffer,
    _scan: super::metal_parallel::Scan,
    _tasks: MappedBuffer,
    _proof: MappedBuffer,
}
impl PendingReady {
    pub(super) fn check_completed(self) -> Result<(), Error> {
        super::metal_backend::launch::check_submission()?;
        if self.command.status() != metal::MTLCommandBufferStatus::Completed {
            return Err(Error::BackendFailure);
        }
        match unsafe { self.status.buffer.contents().cast::<u32>().read() } {
            0 => Ok(()),
            1 => Err(Error::BackendCapacity),
            _ => Err(Error::BackendFailure),
        }
    }
}
pub(super) struct FusedReadyFrame<'a> {
    pub columns: &'a MappedBuffer,
    pub counts: &'a MappedBuffer,
    pub value_counts: &'a MappedBuffer,
    pub capacity: usize,
    pub generation: u64,
    pub values: [usize; 3],
}
/// Owns scheduling scratch before canonical output exists. Fused publication
/// initializes and classifies it; selection uses the unchanged stable lists.
pub(super) struct FusedReady {
    pub tasks: MappedBuffer,
    pub proof: MappedBuffer,
    pub status: MappedBuffer,
    pub counts: MappedBuffer,
    parameters: [u32; 11],
    scan: super::metal_parallel::Scan,
    needs_nodes: bool,
    generation: u64,
    fields: usize,
    bindings: [usize; 3],
    frame: [MappedBuffer; 3],
}
impl FusedReady {
    pub(super) fn new(
        device: &Device,
        resident: &ResidentInput,
        plan: &ExecutionPlan,
        data: &MappedBuffer,
        frame: FusedReadyFrame<'_>,
    ) -> Result<Self, Error> {
        Self::prepare::<POSITIONED_COLUMNS>(
            device,
            ReadyShape {
                resident,
                plan,
                data,
                capacity: frame.capacity,
                output: frame.capacity,
                generation: frame.generation,
                values: frame.values,
                buffers: [frame.columns, frame.counts, frame.value_counts],
            },
            MappedBuffer::zeroed::<u32>(device, 17)?,
        )
    }
    pub(super) fn parameters(&self) -> &[u32] {
        &self.parameters
    }
    pub(super) fn check_status(&self) -> Result<(), Error> {
        super::metal_backend::launch::check_submission()?;
        match unsafe { self.status.buffer.contents().cast::<u32>().read() } {
            0 => Ok(()),
            1 => Err(Error::BackendCapacity),
            _ => Err(Error::BackendFailure),
        }
    }
    fn prepare<const FIELDS: usize>(
        device: &Device,
        shape: ReadyShape<'_>,
        counts: MappedBuffer,
    ) -> Result<Self, Error> {
        let ReadyShape {
            resident,
            plan,
            data,
            capacity,
            output,
            generation,
            values,
            buffers,
        } = shape;
        plan.validate(resident)?;
        if ![
            INITIAL_COLUMNS,
            POSITIONED_COLUMNS,
            super::metal_state_values::COLUMNS,
        ]
        .contains(&FIELDS)
        {
            return Err(Error::BackendFailure);
        }
        for (buffer, count, width) in [
            (buffers[0], capacity, FIELDS * 4),
            (buffers[1], 5, 4),
            (buffers[2], 2, 4),
        ] {
            if count.checked_mul(width).ok_or(Error::BackendCapacity)? as u64
                > buffer.buffer.length()
            {
                return Err(Error::BackendFailure);
            }
        }
        let checked = |n| u32::try_from(n).map_err(|_| Error::BackendCapacity);
        let grid = &resident.input.grid;
        let tasks = super::metal_parallel::task_group_capacity(
            capacity.min(grid.header.semantic_tuple_count as usize),
        )?;
        let parameters = [
            checked(capacity)?,
            checked(output)?,
            checked(tasks)?,
            checked(grid.semantic_span_order.len())?,
            checked(resident.input.packed.programs.len())?,
            checked(resident.input.packed.nodes.len())?,
            checked(values[0])?,
            checked(values[1])?,
            checked(values[2])?,
            if FIELDS == POSITIONED_COLUMNS {
                super::metal_state_values::MAX_POSITIONED_KIND
            } else {
                5
            },
            grid.header.semantic_tuple_count,
        ];
        let entries = plan.readiness.entries(resident)?;
        let needs_nodes = entries
            .iter()
            .any(|entry| entry[0] != u32::MAX && entry[1] == 0);
        Ok(Self {
            tasks: MappedBuffer::zeroed::<[u32; 3]>(device, tasks)?,
            proof: MappedBuffer::from_slice(device, &entries)?,
            status: MappedBuffer::from_slice(device, &[2u32])?,
            counts,
            parameters,
            scan: super::metal_parallel::Scan::new(device, capacity.div_ceil(65536))?,
            needs_nodes,
            generation,
            frame: buffers.map(|buffer| MappedBuffer {
                buffer: buffer.buffer.clone(),
            }),
            fields: FIELDS,
            bindings: [
                resident as *const _ as usize,
                plan as *const _ as usize,
                &*data.buffer as *const metal::BufferRef as usize,
            ],
        })
    }
    fn validate<const FIELDS: usize>(
        &self,
        input: &ReadyInput<'_, FIELDS>,
        output: &ReadyBatch<FIELDS>,
    ) -> Result<(), Error> {
        validate(input, output)?;
        let p = &self.parameters;
        if self
            .frame
            .iter()
            .zip([
                &input.states.columns,
                &input.states.counts,
                &input.values.counts,
            ])
            .any(|(bound, actual)| !std::ptr::eq(&*bound.buffer, &*actual.buffer))
            || self.fields != FIELDS
            || self.generation != input.states.generation
            || p[0] as usize != input.states.capacity
            || p[1] as usize != output.capacity
            || p[6] as usize != input.values.cursor_capacity
            || p[7] as usize != input.values.repeat_capacity
            || p[8] as usize != input.values.capture_capacity
            || !std::ptr::eq(&*self.counts.buffer, &*output.counts.buffer)
            || self.bindings
                != [
                    input.resident as *const _ as usize,
                    input.plan as *const _ as usize,
                    &*input.data.buffer as *const metal::BufferRef as usize,
                ]
        {
            return Err(Error::BackendFailure);
        }
        Ok(())
    }
    pub(super) fn encode_selection<const FIELDS: usize>(
        self,
        command: &CommandBufferRef,
        input: ReadyInput<'_, FIELDS>,
        output: &ReadyBatch<FIELDS>,
    ) -> Result<PendingReady, Error> {
        self.validate(&input, output)?;
        let Self {
            tasks: task_buffer,
            proof,
            status,
            counts: _,
            parameters: p,
            scan,
            needs_nodes,
            ..
        } = self;
        let source = source::<FIELDS>();
        let run = |name, buffers: &[&metal::BufferRef], count: usize| -> Result<(), Error> {
            super::metal_parallel::encode(
                command,
                &super::metal_backend::pipeline_named(&source, name)?,
                name,
                buffers,
                &p,
                count.div_ceil(256).max(1),
                None,
            );
            Ok(())
        };
        let s = &input.states.columns.buffer;
        let h = &input.states.counts.buffer;
        let data = &input.data.buffer;
        // Eligible bulk headers consume all entries at the selected rank. Only
        // scalar GREEDY headers need the separate minimum-node reduction.
        if needs_nodes {
            run(
                "ready_nodes",
                &[
                    data,
                    s,
                    h,
                    &proof.buffer,
                    &task_buffer.buffer,
                    &status.buffer,
                ],
                input.states.capacity,
            )?;
        }
        if input.states.capacity <= 8_388_608 {
            run(
                if input.states.capacity <= 65536 {
                    "ready_select_small"
                } else {
                    "ready_select_cooperative"
                },
                &[
                    data,
                    s,
                    h,
                    &proof.buffer,
                    &task_buffer.buffer,
                    &status.buffer,
                    &output.indices.buffer,
                    &output.counts.buffer,
                ],
                256,
            )?;
        } else {
            run(
                "ready_count_blocks",
                &[
                    data,
                    s,
                    h,
                    &proof.buffer,
                    &task_buffer.buffer,
                    &status.buffer,
                    &scan.values.buffer,
                ],
                input.states.capacity.div_ceil(65536) * 256,
            )?;
            scan.encode(command)?;
            run(
                "ready_scatter_blocks",
                &[
                    data,
                    s,
                    h,
                    &proof.buffer,
                    &task_buffer.buffer,
                    &scan.values.buffer,
                    &status.buffer,
                    &output.indices.buffer,
                    scan.total(),
                    &output.counts.buffer,
                ],
                input.states.capacity.div_ceil(65536) * 256,
            )?;
        }

        Ok(PendingReady {
            command: command.to_owned(),
            status,
            _scan: scan,
            _tasks: task_buffer,
            _proof: proof,
        })
    }
}
struct ReadyShape<'a> {
    resident: &'a ResidentInput,
    plan: &'a ExecutionPlan,
    data: &'a MappedBuffer,
    capacity: usize,
    output: usize,
    generation: u64,
    values: [usize; 3],
    buffers: [&'a MappedBuffer; 3],
}
fn source<const FIELDS: usize>() -> String {
    let source = concat!(
        include_str!("metal_resident.metal"),
        include_str!("metal_input_layout.metal"),
        include_str!("metal_task_groups.metal"),
        include_str!("metal_ready_batch.metal")
    );
    format!("#define INPUT_COLUMNS {FIELDS}u\n{source}")
}
fn validate<const FIELDS: usize>(
    input: &ReadyInput<'_, FIELDS>,
    output: &ReadyBatch<FIELDS>,
) -> Result<(), Error> {
    input.plan.validate(input.resident)?;
    super::metal_state_values::validate_storage(
        input.states,
        input.values,
        &[
            (
                &output.indices,
                output.capacity,
                if FIELDS == INITIAL_COLUMNS { 4 } else { 16 },
            ),
            (&output.counts, 17, 4),
            (input.data, 0, 1),
        ],
        output.generation,
    )
}
pub(super) fn encode<const FIELDS: usize>(
    device: &Device,
    command: &CommandBufferRef,
    input: ReadyInput<'_, FIELDS>,
    output: &ReadyBatch<FIELDS>,
) -> Result<PendingReady, Error> {
    validate(&input, output)?;
    let prepared = FusedReady::prepare::<FIELDS>(
        device,
        ReadyShape {
            resident: input.resident,
            plan: input.plan,
            data: input.data,
            capacity: input.states.capacity,
            output: output.capacity,
            generation: input.states.generation,
            values: [
                input.values.cursor_capacity,
                input.values.repeat_capacity,
                input.values.capture_capacity,
            ],
            buffers: [
                &input.states.columns,
                &input.states.counts,
                &input.values.counts,
            ],
        },
        MappedBuffer {
            buffer: output.counts.buffer.clone(),
        },
    )?;
    let source = source::<FIELDS>();
    for (name, buffers, count) in [
        (
            "ready_initialize",
            vec![
                &*input.states.counts.buffer,
                &input.values.counts.buffer,
                &prepared.tasks.buffer,
                &prepared.status.buffer,
                &output.counts.buffer,
            ],
            prepared.parameters[2] as usize,
        ),
        (
            "ready_classify",
            vec![
                &*input.data.buffer,
                &input.states.columns.buffer,
                &input.states.counts.buffer,
                &input.values.counts.buffer,
                &prepared.proof.buffer,
                &prepared.tasks.buffer,
                &prepared.status.buffer,
                &output.counts.buffer,
            ],
            input.states.capacity,
        ),
    ] {
        super::metal_parallel::encode(
            command,
            &super::metal_backend::pipeline_named(&source, name)?,
            name,
            &buffers,
            &prepared.parameters,
            count.div_ceil(256).max(1),
            None,
        );
    }
    prepared.encode_selection(command, input, output)
}
