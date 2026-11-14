//! One broad argument batch resumes paths at their predicate PC.
use super::Error;
use super::metal_backend::launch::{commit_checked, wait_checked};
use super::metal_backend::{MappedBuffer, pipeline_named};
use super::metal_capture_arguments::{ArgumentInput, CaptureParams};
use super::metal_parallel::{Scan, encode};
use super::metal_successor::{SuccessorInput, SuccessorOutput};
use metal::{BufferRef, CommandBufferRef, CommandQueueRef, Device};
const SOURCE: &str = concat!(
    include_str!("metal_resident.metal"),
    include_str!("metal_capture_arguments.metal"),
    include_str!("metal_inline_predicates.metal")
);
fn size(n: usize) -> Result<u32, Error> {
    u32::try_from(n).map_err(|_| Error::BackendCapacity)
}
fn run(
    command: &CommandBufferRef,
    name: &str,
    buffers: &[&BufferRef],
    parameters: &[u32; 6],
    count: usize,
) -> Result<(), Error> {
    encode(
        command,
        &pipeline_named(SOURCE, name)?,
        name,
        buffers,
        parameters,
        count.div_ceil(256).max(1),
        None,
    );
    Ok(())
}
fn total(scan: &Scan) -> Result<[u32; 4], Error> {
    let v = unsafe { scan.total().contents().cast::<[u32; 4]>().read() };
    if v.contains(&u32::MAX) {
        Err(Error::BackendCapacity)
    } else {
        Ok(v)
    }
}
fn check(status: &MappedBuffer) -> Result<(), Error> {
    match super::metal_register_execution::read::<u32>(status, 1)?[0] {
        0 => Ok(()),
        1 => Err(Error::BackendCapacity),
        _ => Err(Error::BackendFailure),
    }
}
pub(super) struct ResolvedPredicates<'a> {
    pub selected: Scan,
    pub arguments: Scan,
    pub metadata: MappedBuffer,
    pub engine: super::metal_capture_predicates::CapturePredicates<'a>,
    pub status: MappedBuffer,
    parameters: [u32; 6],
}
impl ResolvedPredicates<'_> {
    pub(super) fn count(&self) -> Result<usize, Error> {
        Ok(total(&self.selected)?[0] as usize)
    }
    pub(super) fn encode_publish(
        &self,
        command: &CommandBufferRef,
        input: &SuccessorInput<'_>,
        output: SuccessorOutput<'_>,
        status: &MappedBuffer,
    ) -> Result<(), Error> {
        let mut p = self.parameters;
        p[4] = size(output.states.capacity)?;
        run(
            command,
            "inline_continuation_publish",
            &[
                &input.data.buffer,
                &input.states.columns.buffer,
                &input.states.counts.buffer,
                &input.control.spans.buffer,
                &input.control.path_work.buffer,
                &self.arguments.values.buffer,
                &self.metadata.buffer,
                &self.engine.outcome_counts().buffer,
                &self.engine.work_values().buffer,
                &self.selected.values.buffer,
                self.selected.total(),
                &output.states.columns.buffer,
                &output.states.counts.buffer,
                &output.control.spans.buffer,
                &output.control.path_work.buffer,
                &status.buffer,
            ],
            &p,
            input.states.capacity,
        )
    }
    pub(super) fn check_completed(&self) -> Result<(), Error> {
        check(&self.status)
    }
}
pub(super) fn resolve<'a>(
    device: &Device,
    queue: &CommandQueueRef,
    input: &SuccessorInput<'_>,
    expected: usize,
    programs: &'a super::metal_capture_predicates::PredicatePrograms<'a>,
) -> Result<ResolvedPredicates<'a>, Error> {
    let mut p = [
        size(input.states.capacity)?,
        0,
        0,
        size(input.resident.input.grid.semantic_span_order.len())?,
        0,
        0,
    ];
    let arguments = Scan::new(device, input.states.capacity)?;
    let command = queue.new_command_buffer();
    let validation = super::metal_frontier_validation::encode(device, command, input)?;
    run(
        command,
        "inline_argument_counts",
        &[
            &input.data.buffer,
            &input.states.columns.buffer,
            &input.states.counts.buffer,
            &arguments.values.buffer,
            &validation.status.buffer,
        ],
        &p,
        input.states.capacity,
    )?;
    arguments.encode(command)?;
    commit_checked(command)?;
    wait_checked(command)?;
    validation.check_completed()?;
    p[5] = super::metal_register_execution::read::<u32>(&input.states.counts, 5)?[0];
    let totals = total(&arguments)?;
    if totals[0] as usize != expected || totals[2..] != [0, 0] {
        return Err(Error::BackendFailure);
    }
    p[1] = totals[0];
    p[2] = totals[1];
    let n = totals[0] as usize;
    let c = totals[1] as usize;
    let spans = MappedBuffer::zeroed::<[u32; 2]>(device, n)?;
    let specs = MappedBuffer::zeroed::<[u32; 8]>(device, n)?;
    let metadata = MappedBuffer::zeroed::<[u32; 4]>(device, n)?;
    let work = MappedBuffer::zeroed::<u32>(device, n)?;
    let captures = MappedBuffer::zeroed::<[u32; 4]>(device, c)?;
    let owners = MappedBuffer::zeroed::<u32>(device, c)?;
    let command = queue.new_command_buffer();
    let validation = super::metal_frontier_validation::encode(device, command, input)?;
    let status = &validation.status;
    run(
        command,
        "inline_argument_frames",
        &[
            &input.data.buffer,
            &input.states.columns.buffer,
            &input.states.counts.buffer,
            &input.control.path_work.buffer,
            &arguments.values.buffer,
            arguments.total(),
            &spans.buffer,
            &specs.buffer,
            &metadata.buffer,
            &work.buffer,
            &status.buffer,
        ],
        &p,
        input.states.capacity,
    )?;
    run(
        command,
        "inline_argument_captures",
        &[
            &input.data.buffer,
            &input.states.columns.buffer,
            &input.states.counts.buffer,
            &input.values.captures.buffer,
            &arguments.values.buffer,
            &captures.buffer,
            &owners.buffer,
            &status.buffer,
        ],
        &p,
        c,
    )?;
    commit_checked(command)?;
    wait_checked(command)?;
    validation.check_completed()?;
    let engine =
        super::metal_capture_predicates::CapturePredicates::new_inline(device, programs, n, work)?;
    let parameters = CaptureParams {
        layout: super::metal_capture_arguments::ArgumentLayout::Compact,
        results: totals[0],
        captures: totals[1],
        step: 0,
        nodes: size(input.resident.input.packed.nodes.len())?,
        pairs: 0,
        windows: 0,
        units: 0,
        abort_evaluation: 0,
        merge_width: 0,
    };
    let args = ArgumentInput {
        data: input.data,
        spans: &spans,
        captures: &captures,
        metadata: &metadata,
        owners: &owners,
        specs: &specs,
        status,
        parameters,
    };
    let failures =
        MappedBuffer::zeroed::<u32>(device, input.resident.input.grid.evaluation_spans.len())?;
    engine.execute_arguments(device, queue, input.resident, &args, &failures, false)?;
    // Selection consumes private argument/result rows, not the source frontier.
    let status = validation.status;
    let selected = Scan::new(device, input.states.capacity)?;
    let command = queue.new_command_buffer();
    run(
        command,
        "inline_continuation_counts",
        &[
            &arguments.values.buffer,
            arguments.total(),
            &metadata.buffer,
            &engine.outcome_counts().buffer,
            &selected.values.buffer,
            &status.buffer,
        ],
        &p,
        input.states.capacity,
    )?;
    selected.encode(command)?;
    commit_checked(command)?;
    wait_checked(command)?;
    check(&status)?;
    Ok(ResolvedPredicates {
        selected,
        arguments,
        metadata,
        engine,
        status,
        parameters: p,
    })
}
