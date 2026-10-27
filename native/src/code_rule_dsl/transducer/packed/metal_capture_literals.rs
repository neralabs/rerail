//! Exact byte comparison frontiers for capture arguments and literal sets.
use super::Error;
use super::metal_backend::MappedBuffer;
use super::metal_backend::launch::{commit_checked, wait_checked};
use super::metal_capture_arguments::ArgumentInput;
use super::metal_capture_groups::CaptureGroups;
use super::metal_parallel::Scan;
use metal::{CommandQueueRef, Device};

// Twelve scalar words match PredicatePair without Rust or Metal vector padding.
#[repr(C)]
#[derive(Clone, Copy)]
struct PredicatePair {
    owner: u32,
    member: u32,
    mode: u32,
    left_kind: u32,
    right_kind: u32,
    left_start: u32,
    left_length: u32,
    right_start: u32,
    right_length: u32,
    windows: u32,
    units: u32,
    answer: u32,
}

pub(super) fn execute(
    device: &Device,
    queue: &CommandQueueRef,
    input: &ArgumentInput<'_>,
    groups: &CaptureGroups,
    counts: &MappedBuffer,
) -> Result<(), Error> {
    let mut p = input.parameters;
    let scan = Scan::new(device, p.captures as usize)?;
    let members = MappedBuffer::zeroed::<u32>(device, p.captures as usize)?;
    let command = queue.new_command_buffer();
    p.run(
        command,
        "predicate_literal_counts",
        &[
            &input.data.buffer,
            &input.spans.buffer,
            &input.captures.buffer,
            &input.metadata.buffer,
            &input.owners.buffer,
            &groups.ids.buffer,
            &groups.ranks.buffer,
            &scan.values.buffer,
            &members.buffer,
            &counts.buffer,
            &input.status.buffer,
            &input.specs.buffer,
        ],
        p.captures as usize,
    )?;
    scan.encode(command)?;
    commit_checked(command)?;
    wait_checked(command)?;
    check(input.status)?;
    let totals = total(&scan)?;
    if totals[1..] != [0, 0, 0] {
        return Err(Error::BackendFailure);
    }
    let count = totals[0];
    let pairs = MappedBuffer::zeroed::<PredicatePair>(device, count as usize)?;
    let work = Scan::new(device, count as usize)?;
    p.pairs = count;
    let command = queue.new_command_buffer();
    p.run(
        command,
        "predicate_literal_scatter",
        &[
            &input.data.buffer,
            &input.spans.buffer,
            &input.captures.buffer,
            &input.metadata.buffer,
            &input.owners.buffer,
            &groups.ids.buffer,
            &groups.ranks.buffer,
            &scan.values.buffer,
            &pairs.buffer,
            &work.values.buffer,
            &input.status.buffer,
            &input.specs.buffer,
        ],
        count as usize,
    )?;
    work.encode(command)?;
    commit_checked(command)?;
    wait_checked(command)?;
    check(input.status)?;
    let total = total(&work)?;
    if total[2..] != [0, 0] {
        return Err(Error::BackendFailure);
    }
    let windows = MappedBuffer::zeroed::<u32>(device, total[0] as usize)?;
    p.pairs = count;
    p.windows = total[0];
    let command = queue.new_command_buffer();
    p.run(
        command,
        "predicate_literal_window_init",
        &[&windows.buffer],
        total[0] as usize,
    )?;
    p.units = total[1];
    p.run(
        command,
        "predicate_literal_compare",
        &[
            &input.data.buffer,
            &pairs.buffer,
            &work.values.buffer,
            &windows.buffer,
            &input.status.buffer,
        ],
        total[1] as usize,
    )?;
    p.run(
        command,
        "predicate_literal_windows",
        &[
            &pairs.buffer,
            &work.values.buffer,
            &windows.buffer,
            &input.status.buffer,
        ],
        total[0] as usize,
    )?;
    p.run(
        command,
        "predicate_literal_results",
        &[
            &input.data.buffer,
            &pairs.buffer,
            &input.metadata.buffer,
            &counts.buffer,
            &members.buffer,
            &input.status.buffer,
            &input.specs.buffer,
        ],
        count as usize,
    )?;
    p.run(
        command,
        "predicate_literal_members",
        &[
            &input.data.buffer,
            &input.captures.buffer,
            &input.metadata.buffer,
            &input.owners.buffer,
            &counts.buffer,
            &members.buffer,
            &input.status.buffer,
            &input.specs.buffer,
        ],
        p.captures as usize,
    )?;
    commit_checked(command)?;
    wait_checked(command)?;
    check(input.status)
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
