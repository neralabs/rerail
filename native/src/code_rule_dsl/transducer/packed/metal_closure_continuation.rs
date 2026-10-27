//! Reconstruct closure exits from original complete values and full positions.
use super::Error;
use super::metal_backend::{MappedBuffer, pipeline_named};
use super::metal_bulk_closure::{self, ClosureExits, PendingClosure};
use super::metal_closure_prepare::{self, PendingPreparation, PreparedRanges, RangeInput};
use super::metal_frontier_validation::ValidatedInput;
use super::metal_parallel::{Scan, encode as dispatch};
use super::metal_position_order::{self, OrderedFrontier, PendingOrder};
use super::metal_successor::{SuccessorInput, SuccessorOutput};
use metal::{CommandBufferRef, Device};
use std::rc::Rc;

pub(super) struct ClosureResult<'a> {
    pub ordered: &'a OrderedFrontier,
    pub ranges: &'a PreparedRanges,
    pub exits: &'a ClosureExits,
}
#[must_use = "Check complete reconstruction before accepting this generation"]
pub(super) struct PendingContinuation {
    validation: Rc<ValidatedInput>,
    upstream: Option<Upstream>,
    _scan: Scan,
    _buffers: Vec<MappedBuffer>,
}
struct Upstream {
    order: PendingOrder,
    preparation: PendingPreparation,
    closure: PendingClosure,
}
impl PendingContinuation {
    pub(super) fn check_completed(self) -> Result<(), Error> {
        self.validation.check_completed()?;
        if let Some(upstream) = self.upstream {
            upstream.order.check_completed()?;
            upstream.preparation.check_completed()?;
            upstream.closure.check_completed()?;
        }
        Ok(())
    }
}

/// One owner retains every producer and shares validation only within this command.
/// Output publication depends on all producer status buffers, including grouping.
pub(super) fn encode_composed(
    device: &Device,
    command: &CommandBufferRef,
    input: &SuccessorInput<'_>,
    closed: ClosureResult<'_>,
    output: SuccessorOutput<'_>,
) -> Result<PendingContinuation, Error> {
    validate(input, &closed, &output)?;
    let order = metal_position_order::encode(device, command, input, closed.ordered)?;
    let preparation = metal_closure_prepare::encode_plan(
        device,
        command,
        RangeInput {
            resident: input.resident,
            plan: input.plan,
            data: input.data,
            states: &closed.ordered.states,
            values: input.values,
        },
        closed.ranges,
    )?;
    let closure = metal_bulk_closure::encode(device, command, &closed.ranges.ranges, closed.exits)?;
    let validation = order.validation(input, command)?;
    encode_validated(
        device,
        command,
        input,
        closed,
        output,
        validation,
        Some(Upstream {
            order,
            preparation,
            closure,
        }),
    )
}

/// Standalone conformance entry point for independently constructed producer fixtures.
/// Production composition must carry producer statuses through `encode_composed`.
#[cfg(test)]
pub(super) fn encode(
    device: &Device,
    command: &CommandBufferRef,
    input: &SuccessorInput<'_>,
    closed: ClosureResult<'_>,
    output: SuccessorOutput<'_>,
) -> Result<PendingContinuation, Error> {
    validate(input, &closed, &output)?;
    let validation = Rc::new(super::metal_frontier_validation::encode(
        device, command, input,
    )?);
    encode_validated(device, command, input, closed, output, validation, None)
}

fn encode_validated(
    device: &Device,
    command: &CommandBufferRef,
    input: &SuccessorInput<'_>,
    closed: ClosureResult<'_>,
    output: SuccessorOutput<'_>,
    validation: Rc<ValidatedInput>,
    upstream: Option<Upstream>,
) -> Result<PendingContinuation, Error> {
    validation.validate_binding(input, command)?;
    validate(input, &closed, &output)?;
    let ranges = &closed.ranges.ranges;
    let capacity = input.states.capacity;
    let scan = Scan::new(device, capacity)?;
    let entries = MappedBuffer::zeroed::<u32>(device, capacity)?;
    let replacements = MappedBuffer::zeroed::<u32>(device, capacity)?;
    let progress = MappedBuffer::zeroed::<[u32; 5]>(device, capacity)?;
    let proof = MappedBuffer::from_slice(device, &input.plan.readiness.entries(input.resident)?)?;
    let p = [
        capacity,
        closed.ordered.states.capacity,
        ranges.domain_capacity,
        ranges.entry_capacity,
        ranges.cell_capacity,
        closed.exits.capacity,
        output.states.capacity,
        output.values.cursor_capacity,
        output.values.repeat_capacity,
        output.values.capture_capacity,
        output.control.digit_capacity,
        input.resident.input.grid.semantic_span_order.len(),
        usize::from(upstream.is_some()),
    ]
    .map(|n| u32::try_from(n).map_err(|_| Error::BackendCapacity))
    .into_iter()
    .collect::<Result<Vec<_>, _>>()?;
    let source = input.plan.readiness.closure.continuation_source();
    let run = |name, buffers: &[&metal::BufferRef], count: usize| -> Result<(), Error> {
        dispatch(
            command,
            &pipeline_named(source, name)?,
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
    let st = &validation.status.buffer;
    let rh = &closed.ordered.states.counts.buffer;
    let eh = &closed.exits.counts.buffer;
    let producer_status = upstream
        .as_ref()
        .map(|upstream| {
            let [preparation, grouping] = upstream.preparation.status_buffers();
            [preparation, grouping, upstream.closure.status_buffer()]
        })
        .unwrap_or([st, st, st]);
    run(
        "reconstruct_initialize",
        &[
            h,
            &input.ready.counts.buffer,
            rh,
            &ranges.counts.buffer,
            eh,
            &validation.categories.buffer,
            &entries.buffer,
            &replacements.buffer,
            &output.states.counts.buffer,
            &output.values.counts.buffer,
            &output.control.count.buffer,
            st,
            producer_status[0],
            producer_status[1],
            producer_status[2],
        ],
        capacity,
    )?;
    run(
        "reconstruct_entries",
        &[
            s,
            h,
            &closed.ordered.states.columns.buffer,
            rh,
            &closed.ordered.originals.buffer,
            &validation.categories.buffer,
            &entries.buffer,
            st,
        ],
        closed.ordered.states.capacity,
    )?;
    run(
        "reconstruct_exits",
        &[
            &input.data.buffer,
            s,
            h,
            &closed.ordered.originals.buffer,
            rh,
            &ranges.domains.buffer,
            &ranges.entries.buffer,
            &ranges.counts.buffer,
            &closed.ranges.nodes.buffer,
            &closed.exits.exits.buffer,
            eh,
            &replacements.buffer,
            st,
        ],
        closed.exits.capacity,
    )?;
    run(
        "reconstruct_progress",
        &[
            &input.data.buffer,
            s,
            h,
            &replacements.buffer,
            &closed.exits.exits.buffer,
            &closed.ranges.nodes.buffer,
            &input.control.path_work.buffer,
            &progress.buffer,
            st,
        ],
        capacity,
    )?;
    run(
        "reconstruct_counts",
        &[
            &input.data.buffer,
            s,
            h,
            &input.control.spans.buffer,
            &replacements.buffer,
            &validation.categories.buffer,
            &proof.buffer,
            &scan.values.buffer,
            st,
        ],
        capacity,
    )?;
    scan.encode(command)?;
    run(
        "reconstruct_boundary",
        &[
            h,
            &input.values.counts.buffer,
            scan.total(),
            &output.states.counts.buffer,
            &output.values.counts.buffer,
            &output.control.count.buffer,
            st,
        ],
        1,
    )?;
    run(
        "reconstruct_publish",
        &[
            &input.data.buffer,
            s,
            h,
            &input.control.spans.buffer,
            &replacements.buffer,
            &closed.exits.exits.buffer,
            &closed.ranges.nodes.buffer,
            &scan.values.buffer,
            &output.states.counts.buffer,
            &output.states.columns.buffer,
            &output.control.spans.buffer,
            st,
            &input.control.digits.buffer,
            &output.control.count.buffer,
            &output.control.digits.buffer,
            &input.values.counts.buffer,
            &input.values.cursors.buffer,
            &input.values.repeats.buffer,
            &input.values.captures.buffer,
            &output.values.cursors.buffer,
            &output.values.repeats.buffer,
            &output.values.captures.buffer,
            &progress.buffer,
            &output.control.path_work.buffer,
        ],
        capacity
            .max(output.control.digit_capacity)
            .max(output.values.cursor_capacity)
            .max(output.values.repeat_capacity)
            .max(output.values.capture_capacity),
    )?;
    Ok(PendingContinuation {
        validation,
        upstream,
        _scan: scan,
        _buffers: vec![entries, replacements, proof, progress],
    })
}

fn validate(
    input: &SuccessorInput<'_>,
    closed: &ClosureResult<'_>,
    output: &SuccessorOutput<'_>,
) -> Result<(), Error> {
    let generation = input.states.generation;
    let next = generation.checked_add(1).ok_or(Error::BackendFailure)?;
    if closed.ordered.states.generation != generation
        || closed.ranges.ranges.generation != generation
        || closed.exits.generation != generation
        || output.states.generation != next
        || output.values.generation != next
        || output.control.generation != next
        || output.control.state_capacity != output.states.capacity
    {
        return Err(Error::BackendFailure);
    }
    let ranges = &closed.ranges.ranges;
    super::metal_successor_prepare::validate_input(
        input,
        &[
            (
                &closed.ordered.states.columns,
                closed.ordered.states.capacity,
                52,
            ),
            (&closed.ordered.states.counts, 5, 4),
            (&closed.ordered.originals, closed.ordered.states.capacity, 4),
            (&ranges.domains, ranges.domain_capacity, 8),
            (&ranges.entries, ranges.entry_capacity, 8),
            (&ranges.allowed, ranges.cell_capacity, 4),
            (&ranges.counts, 3, 4),
            (&closed.ranges.nodes, ranges.cell_capacity, 4),
            (&closed.exits.exits, closed.exits.capacity, 8),
            (&closed.exits.counts, 4, 4),
            (&output.states.columns, output.states.capacity, 48),
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
    )?;
    Ok(())
}
