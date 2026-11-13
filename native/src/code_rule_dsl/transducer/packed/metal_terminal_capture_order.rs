//! Explicit terminal conversion preserves ordinary argument order until execution ends.
use super::Error;
use super::metal_backend::launch::{commit_checked, wait_checked};
use super::metal_backend::{MappedBuffer, pipeline_named};
use super::metal_parallel::{Scan, encode};
use super::metal_state_values::{POSITIONED_COLUMNS, PositionedFrontier};
use super::metal_successor::SuccessorInput;
use metal::{BufferRef, CommandBufferRef, CommandQueueRef, Device};
const SOURCE: &str = concat!(
    include_str!("metal_resident.metal"),
    include_str!("metal_terminal_capture_order.metal")
);
pub(super) struct TerminalCaptureOrder {
    pub states: PositionedFrontier,
    pub captures: MappedBuffer,
    pub capture_count: usize,
}
fn size(n: usize) -> Result<u32, Error> {
    u32::try_from(n).map_err(|_| Error::BackendCapacity)
}
fn run(
    command: &CommandBufferRef,
    name: &str,
    buffers: &[&BufferRef],
    p: &[u32; 4],
    count: usize,
) -> Result<(), Error> {
    encode(
        command,
        &pipeline_named(SOURCE, name)?,
        name,
        buffers,
        p,
        count.div_ceil(256).max(1),
        None,
    );
    Ok(())
}
pub(super) fn normalize(
    device: &Device,
    queue: &CommandQueueRef,
    input: &SuccessorInput<'_>,
) -> Result<TerminalCaptureOrder, Error> {
    let counts = Scan::new(device, input.states.capacity)?;
    let maximum = MappedBuffer::zeroed::<u32>(device, 1)?;
    let mut p = [size(input.states.capacity)?, 0, 0, 0];
    let command = queue.new_command_buffer();
    let validation = super::metal_frontier_validation::encode(device, command, input)?;
    run(
        command,
        "terminal_capture_sizes",
        &[
            &input.states.columns.buffer,
            &input.states.counts.buffer,
            &counts.values.buffer,
            &maximum.buffer,
            &validation.status.buffer,
        ],
        &p,
        input.states.capacity,
    )?;
    counts.encode(command)?;
    commit_checked(command)?;
    wait_checked(command)?;
    validation.check_completed()?;
    let total = unsafe { counts.total().contents().cast::<[u32; 4]>().read() };
    if total.contains(&u32::MAX) {
        return Err(Error::BackendCapacity);
    }
    if total[1..] != [0, 0, 0] {
        return Err(Error::BackendFailure);
    }
    let n = super::metal_register_execution::read::<u32>(&input.states.counts, 1)?[0];
    let maximum = super::metal_register_execution::read::<u32>(&maximum, 1)?[0];
    if maximum > total[0] {
        return Err(Error::BackendFailure);
    }
    p[1] = n;
    p[2] = total[0];
    let states = PositionedFrontier {
        columns: MappedBuffer::zeroed::<[u32; POSITIONED_COLUMNS]>(device, input.states.capacity)?,
        counts: MappedBuffer::zeroed::<u32>(device, 5)?,
        capacity: input.states.capacity,
        generation: input.states.generation,
    };
    let mut captures = [
        MappedBuffer::zeroed::<[u32; 4]>(device, total[0] as usize)?,
        MappedBuffer::zeroed::<[u32; 4]>(device, total[0] as usize)?,
    ];
    let command = queue.new_command_buffer();
    let validation = super::metal_frontier_validation::encode(device, command, input)?;
    run(
        command,
        "terminal_capture_project",
        &[
            &input.states.columns.buffer,
            &input.states.counts.buffer,
            &input.values.captures.buffer,
            &counts.values.buffer,
            &states.columns.buffer,
            &states.counts.buffer,
            &captures[0].buffer,
            &validation.status.buffer,
        ],
        &p,
        (n as usize).max(total[0] as usize),
    )?;
    let mut current = 0;
    for width in (0..32)
        .map(|shift| 1u32 << shift)
        .take_while(|&width| width < maximum)
    {
        p[3] = width;
        run(
            command,
            "terminal_capture_merge",
            &[
                &input.data.buffer,
                &states.columns.buffer,
                &counts.values.buffer,
                &captures[current].buffer,
                &captures[1 - current].buffer,
                &validation.status.buffer,
            ],
            &p,
            total[0] as usize,
        )?;
        current = 1 - current;
    }
    commit_checked(command)?;
    wait_checked(command)?;
    validation.check_completed()?;
    if current == 1 {
        captures.swap(0, 1);
    }
    let captures = captures.into_iter().next().ok_or(Error::BackendFailure)?;
    Ok(TerminalCaptureOrder {
        states,
        captures,
        capture_count: total[0] as usize,
    })
}
