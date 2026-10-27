//! Explicit conversion from full branch positions to ranked frontier records.
use super::Error;
use super::metal_backend::{MappedBuffer, pipeline_named};
use super::metal_frontier_validation::{self, ValidatedInput};
use super::metal_parallel::{Scan, encode as dispatch};
use super::metal_state_values::ValueFrontier;
use super::metal_successor::SuccessorInput;
use metal::{CommandBufferRef, Device};
use std::rc::Rc;

pub(super) struct OrderedFrontier {
    pub states: ValueFrontier,
    /// Ranked entry -> original positioned record. No semantic identity is lost.
    pub originals: MappedBuffer,
}
#[must_use = "Check ordering and complete input validation after command completion"]
pub(super) struct PendingOrder {
    validation: Rc<ValidatedInput>,
    _scan: Scan,
    _buffers: Vec<MappedBuffer>,
}
impl PendingOrder {
    pub(super) fn validation(
        &self,
        input: &SuccessorInput<'_>,
        command: &CommandBufferRef,
    ) -> Result<Rc<ValidatedInput>, Error> {
        self.validation.validate_binding(input, command)?;
        Ok(Rc::clone(&self.validation))
    }
    pub(super) fn check_completed(self) -> Result<(), Error> {
        self.validation.check_completed()
    }
}
pub(super) fn encode(
    device: &Device,
    command: &CommandBufferRef,
    input: &SuccessorInput<'_>,
    output: &OrderedFrontier,
) -> Result<PendingOrder, Error> {
    encode_mode(device, command, input, output, false)
}

/// A complete terminal barrier uses the same full-position comparator as closure entries.
pub(super) fn encode_terminals(
    device: &Device,
    command: &CommandBufferRef,
    input: &SuccessorInput<'_>,
    output: &OrderedFrontier,
) -> Result<PendingOrder, Error> {
    encode_mode(device, command, input, output, true)
}
fn encode_mode(
    device: &Device,
    command: &CommandBufferRef,
    input: &SuccessorInput<'_>,
    output: &OrderedFrontier,
    terminals: bool,
) -> Result<PendingOrder, Error> {
    if output.states.generation != input.states.generation {
        return Err(Error::BackendFailure);
    }
    super::metal_successor_prepare::validate_input(
        input,
        &[
            (&output.states.columns, output.states.capacity, 52),
            (&output.states.counts, 5, 4),
            (&output.originals, output.states.capacity, 4),
        ],
    )?;
    let validation = Rc::new(metal_frontier_validation::encode(device, command, input)?);
    let capacity = input.states.capacity;
    let tasks = super::metal_parallel::task_group_capacity(
        capacity.min(input.resident.input.grid.header.semantic_tuple_count as usize),
    )?;
    let ids = [
        MappedBuffer::zeroed::<u32>(device, capacity)?,
        MappedBuffer::zeroed::<u32>(device, capacity)?,
    ];
    let task_data = MappedBuffer::zeroed::<[u32; 6]>(device, tasks)?;
    let proof = MappedBuffer::from_slice(device, &input.plan.readiness.entries(input.resident)?)?;
    let scan = Scan::new(device, capacity)?;
    let mut p = [
        u32::try_from(capacity).map_err(|_| Error::BackendCapacity)?,
        u32::try_from(output.states.capacity).map_err(|_| Error::BackendCapacity)?,
        tasks as u32,
        0,
    ];
    let source = concat!(
        include_str!("metal_resident.metal"),
        include_str!("metal_task_groups.metal"),
        include_str!("metal_position_order.metal")
    );
    let run = |name, buffers: &[&metal::BufferRef], p: &[u32], groups| -> Result<(), Error> {
        dispatch(
            command,
            &pipeline_named(source, name)?,
            name,
            buffers,
            p,
            groups,
            None,
        );
        Ok(())
    };
    let s = &input.states.columns.buffer;
    let h = &input.states.counts.buffer;
    let st = &validation.status.buffer;
    run(
        "position_initialize",
        &[
            &ids[0].buffer,
            &task_data.buffer,
            &output.states.counts.buffer,
        ],
        &p,
        capacity.max(tasks).div_ceil(256).max(1),
    )?;
    run(
        "position_local_sort",
        &[
            s,
            h,
            &input.control.spans.buffer,
            &input.control.digits.buffer,
            &ids[0].buffer,
            &ids[1].buffer,
            st,
        ],
        &p,
        capacity.div_ceil(16).max(1),
    )?;
    let mut current = 1;
    let mut width = 16usize;
    while width < capacity {
        p[3] = u32::try_from(width).map_err(|_| Error::BackendCapacity)?;
        run(
            "position_merge",
            &[
                s,
                h,
                &input.control.spans.buffer,
                &input.control.digits.buffer,
                &ids[current].buffer,
                &ids[1 - current].buffer,
                st,
            ],
            &p,
            capacity.max(1),
        )?;
        current = 1 - current;
        width = width.checked_mul(2).ok_or(Error::BackendCapacity)?;
    }
    run(
        if terminals {
            "terminal_order_select"
        } else {
            "position_select"
        },
        &[
            &input.data.buffer,
            s,
            h,
            &input.control.spans.buffer,
            &input.control.digits.buffer,
            &ids[current].buffer,
            &validation.categories.buffer,
            &proof.buffer,
            &task_data.buffer,
            &scan.values.buffer,
            st,
        ],
        &p,
        capacity.max(1),
    )?;
    if !terminals {
        run(
            "position_scopes",
            &[
                &input.data.buffer,
                s,
                h,
                &validation.categories.buffer,
                &proof.buffer,
                &task_data.buffer,
                st,
            ],
            &p,
            capacity.div_ceil(256).max(1),
        )?;
    }
    scan.encode(command)?;
    run(
        "position_boundary",
        &[h, scan.total(), st, &output.states.counts.buffer],
        &p,
        1,
    )?;
    run(
        if terminals {
            "terminal_order_scatter"
        } else {
            "position_scatter"
        },
        &[
            s,
            h,
            &ids[current].buffer,
            &validation.categories.buffer,
            &task_data.buffer,
            &scan.values.buffer,
            st,
            &output.states.columns.buffer,
            &output.states.counts.buffer,
            &output.originals.buffer,
        ],
        &p,
        capacity.div_ceil(256).max(1),
    )?;
    let [first, second] = ids;
    Ok(PendingOrder {
        validation,
        _scan: scan,
        _buffers: vec![first, second, task_data, proof],
    })
}
