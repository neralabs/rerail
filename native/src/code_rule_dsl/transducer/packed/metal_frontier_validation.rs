//! Reuse complete input checks without executing an instruction.
use super::Error;
use super::metal_backend::{MappedBuffer, pipeline_named};
use super::metal_parallel::encode as dispatch;
use super::metal_state_values::{INITIAL_COLUMNS, POSITIONED_COLUMNS};
use super::metal_successor::SuccessorInput;
use metal::{CommandBufferRef, Device};

pub(super) struct ValidatedInput<const FIELDS: usize = POSITIONED_COLUMNS> {
    command: metal::CommandBuffer,
    bindings: [usize; 13],
    capacities: [usize; 5],
    generation: u64,
    resident: usize,
    plan: usize,
    pub status: MappedBuffer,
    pub categories: MappedBuffer,
    _buffers: Vec<MappedBuffer>,
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
fn capacities<const FIELDS: usize>(input: &SuccessorInput<'_, FIELDS>) -> [usize; 5] {
    [
        input.states.capacity,
        input.values.cursor_capacity,
        input.values.repeat_capacity,
        input.values.capture_capacity,
        input.control.digit_capacity,
    ]
}
impl<const FIELDS: usize> ValidatedInput<FIELDS> {
    /// Reuse only within the command that validates these immutable inputs.
    /// A completed command alone cannot prove that shared buffers stayed unchanged.
    pub(super) fn validate_binding(
        &self,
        input: &SuccessorInput<'_, FIELDS>,
        command: &CommandBufferRef,
    ) -> Result<(), Error> {
        super::metal_successor_prepare::validate_input(input, &[])?;
        if self.bindings != bindings(input)
            || self.capacities != capacities(input)
            || self.generation != input.states.generation
            || self.resident != input.resident as *const _ as usize
            || self.plan != input.plan as *const _ as usize
            || !std::ptr::eq(&*self.command, command)
            || command.status() != metal::MTLCommandBufferStatus::NotEnqueued
        {
            return Err(Error::BackendFailure);
        }
        Ok(())
    }
    pub(super) fn check_completed(&self) -> Result<(), Error> {
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
pub(super) fn encode<const FIELDS: usize>(
    device: &Device,
    command: &CommandBufferRef,
    input: &SuccessorInput<'_, FIELDS>,
) -> Result<ValidatedInput<FIELDS>, Error> {
    super::metal_successor_prepare::validate_input(input, &[])?;
    let code =
        super::metal_successor_codegen::compile::<FIELDS>(input.resident, &input.plan.readiness)?;
    let capacity = input.states.capacity;
    let parameters = super::metal_successor_prepare::parameters(input, &code, 0)?;
    let status = MappedBuffer::from_slice(device, &[0u32, u32::MAX, u32::MAX])?;
    let categories = MappedBuffer::zeroed::<u32>(device, capacity)?;
    let sites = MappedBuffer::from_slice(device, &code.sites)?;
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
    let h = &input.states.counts.buffer;
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
        "successor_validate_frontier",
        &[
            &input.data.buffer,
            &input.states.columns.buffer,
            h,
            &input.values.counts.buffer,
            &input.control.spans.buffer,
            &input.control.count.buffer,
            &categories.buffer,
            &sites.buffer,
            &input.values.cursors.buffer,
            &input.values.repeats.buffer,
            &input.values.captures.buffer,
            &input.control.digits.buffer,
            st,
            &input.control.path_work.buffer,
        ],
        capacity.checked_mul(256).ok_or(Error::BackendCapacity)?,
        None,
    )?;
    Ok(ValidatedInput {
        command: command.to_owned(),
        bindings: bindings(input),
        capacities: capacities(input),
        generation: input.states.generation,
        resident: input.resident as *const _ as usize,
        plan: input.plan as *const _ as usize,
        status,
        categories,
        _buffers: vec![sites],
    })
}
