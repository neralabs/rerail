//! Merge complete typed producers on the GPU; apply evaluation failures and limits once.
use super::super::Error;
use super::super::metal_backend::MappedBuffer;
use super::super::metal_backend::launch::{commit_checked, wait_checked};
use super::super::metal_batch_abi::{FinalCaptureRecord, PropertyRecord, ResultRecord};
use super::super::metal_grid_plan::ResidentInput;
use super::super::metal_parallel::{Scan, encode};
use super::super::metal_register_execution::read;
use super::{OutputArenas, TypedCheckpoint};
use metal::{CommandQueueRef, Device};

fn check(status: &MappedBuffer) -> Result<(), Error> {
    match read::<u32>(status, 1)?[0] {
        0 => Ok(()),
        1 => Err(Error::BackendCapacity),
        _ => Err(Error::BackendFailure),
    }
}
fn total(scan: &Scan) -> Result<[u32; 4], Error> {
    let counts = unsafe { scan.total().contents().cast::<[u32; 4]>().read() };
    if counts.contains(&u32::MAX) {
        return Err(Error::BackendCapacity);
    }
    Ok(counts)
}

const SOURCE: &str = concat!(
    include_str!("metal_resident.metal"),
    include_str!("metal_column_view.metal"),
    include_str!("metal_typed_merge.metal")
);

pub(super) fn execute(
    device: &Device,
    queue: &CommandQueueRef,
    resident: &ResidentInput,
    data: &MappedBuffer,
    inputs: [TypedCheckpoint; 2],
    errors: &[Option<Error>],
    apply_limits: bool,
) -> Result<OutputArenas, Error> {
    if errors.len() != resident.input.grid.evaluation_spans.len() {
        return Err(Error::BackendFailure);
    }
    for input in &inputs {
        input.validate()?;
    }
    let mut bound = [0u32; 4];
    for (i, value) in bound.iter_mut().enumerate() {
        *value = inputs[0].arenas.counts()[i]
            .checked_add(inputs[1].arenas.counts()[i])
            .ok_or(Error::BackendCapacity)?;
    }
    if bound[0] == 0 {
        return OutputArenas::empty(device);
    }
    let mut parameters = inputs
        .iter()
        .flat_map(|input| input.arenas.counts())
        .collect::<Vec<_>>();
    parameters.push(u32::from(apply_limits));
    parameters.extend(inputs.iter().flat_map(|input| input.arenas.encoding()));
    let scan = Scan::new(device, bound[0] as usize)?;
    let status = MappedBuffer::zeroed::<u32>(device, 1)?;
    let failures = MappedBuffer::from_slice(
        device,
        &errors
            .iter()
            .map(|error| u32::from(error.is_some()))
            .collect::<Vec<_>>(),
    )?;
    let [ar, ac, ap, ab, al, ah] = inputs[0].arenas.buffers();
    let [br, bc, bp, bb, bl, bh] = inputs[1].arenas.buffers();
    metal::objc::rc::autoreleasepool(|| -> Result<(), Error> {
        let command = queue.new_command_buffer();
        encode(
            command,
            &super::super::metal_backend::pipeline_named(SOURCE, "typed_merge_counts")?,
            "typed_merge_counts",
            &[
                &data.buffer,
                &ar.buffer,
                &al.buffer,
                &br.buffer,
                &bl.buffer,
                &failures.buffer,
                &scan.values.buffer,
                &status.buffer,
                &ah.buffer,
                &bh.buffer,
            ],
            &parameters,
            (bound[0] as usize).div_ceil(256),
            None,
        );
        scan.encode(command)?;
        commit_checked(command)?;
        wait_checked(command)?;
        check(&status)
    })?;
    let counts = total(&scan)?;
    if counts
        .iter()
        .zip(bound)
        .any(|(&count, bound)| count > bound)
    {
        return Err(Error::BackendFailure);
    }
    let output = OutputArenas {
        rows: MappedBuffer::zeroed::<ResultRecord>(device, counts[0] as usize)?,
        captures: MappedBuffer::zeroed::<FinalCaptureRecord>(device, counts[1] as usize)?,
        properties: MappedBuffer::zeroed::<PropertyRecord>(device, counts[2] as usize)?,
        bytes: MappedBuffer::zeroed::<u8>(device, counts[3] as usize)?,
        layout: Some(MappedBuffer::zeroed::<[u32; 4]>(
            device,
            counts[0] as usize,
        )?),
        counts,
    };
    metal::objc::rc::autoreleasepool(|| -> Result<(), Error> {
        let command = queue.new_command_buffer();
        encode(
            command,
            &super::super::metal_backend::pipeline_named(SOURCE, "typed_merge_scatter")?,
            "typed_merge_scatter",
            &[
                &ar.buffer,
                &ac.buffer,
                &ap.buffer,
                &ab.buffer,
                &al.buffer,
                &br.buffer,
                &bc.buffer,
                &bp.buffer,
                &bb.buffer,
                &bl.buffer,
                &scan.values.buffer,
                scan.total(),
                &status.buffer,
                &output.rows.buffer,
                &output.captures.buffer,
                &output.properties.buffer,
                &output.bytes.buffer,
                &output.layout.as_ref().ok_or(Error::BackendFailure)?.buffer,
                &ah.buffer,
                &bh.buffer,
            ],
            &parameters,
            (*bound.iter().max().ok_or(Error::BackendFailure)? as usize).div_ceil(256),
            None,
        );
        commit_checked(command)?;
        wait_checked(command)?;
        check(&status)
    })?;
    output.validate()?;
    Ok(output)
}
