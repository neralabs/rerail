//! Count virtual expansion blocks, then write exact live typed continuations.
use super::super::metal_parallel::{Scan, encode};
use super::*;

fn status(buffer: &MappedBuffer) -> Result<(), Error> {
    match read::<u32>(buffer, 1)?[0] {
        0 => Ok(()),
        1 => Err(Error::BackendCapacity),
        _ => Err(Error::BackendFailure),
    }
}
fn total(scan: &Scan) -> Result<[u32; 4], Error> {
    let value = unsafe { scan.total().contents().cast::<[u32; 4]>().read() };
    if value.contains(&u32::MAX) {
        return Err(Error::BackendCapacity);
    }
    Ok(value)
}
fn checked(n: usize) -> Result<u32, Error> {
    u32::try_from(n).map_err(|_| Error::BackendCapacity)
}

pub(super) fn finish(
    seeds: InitialSeeds<'_>,
    device: &Device,
    queue: &metal::CommandQueueRef,
    work_class: WorkClass,
) -> Result<DynamicFrontier, Error> {
    let resident = seeds.resident;
    let plan = seeds.plan;
    let data = seeds.data;
    let root_count = seeds.records.capacity;
    let records = &seeds.records;
    let code = super::super::metal_initial_range_codegen::compile(resident, plan)?;
    if code.programs.len() != resident.input.packed.programs.len() {
        return Err(Error::BackendFailure);
    }
    let units = MappedBuffer::from_slice(device, &code.programs)?;
    let source = code.source;
    let mut parameters = [
        checked(root_count)?,
        0,
        0,
        checked(resident.input.grid.header.semantic_tuple_count as usize)?,
        checked(resident.input.grid.semantic_span_order.len())?,
        checked(resident.input.packed.programs.len())?,
        checked(resident.input.packed.nodes.len())?,
        0,
        0,
        0,
        0,
        0,
        work_class as u32,
    ];
    let starts = MappedBuffer::zeroed::<u32>(
        device,
        root_count.checked_add(1).ok_or(Error::BackendCapacity)?,
    )?;
    let root_blocks = Scan::new(device, root_count.div_ceil(256))?;
    let error = MappedBuffer::zeroed::<u32>(device, 1)?;
    let run = |command: &CommandBufferRef,
               name: &str,
               buffers: &[&metal::BufferRef],
               parameters: &[u32],
               groups: usize|
     -> Result<(), Error> {
        encode(
            command,
            &super::super::metal_backend::pipeline_named(&source, name)?,
            name,
            buffers,
            parameters,
            groups.max(1),
            None,
        );
        Ok(())
    };
    metal::objc::rc::autoreleasepool(|| {
        let command = queue.new_command_buffer();
        if root_count != 0 {
            run(
                command,
                "initial_range_root_counts",
                &[
                    &data.buffer,
                    &records.columns.buffer,
                    &records.counts.buffer,
                    &starts.buffer,
                    &root_blocks.values.buffer,
                    &error.buffer,
                    &units.buffer,
                ],
                &parameters,
                root_count.div_ceil(256),
            )?;
        }
        root_blocks.encode(command)?;
        run(
            command,
            "initial_range_root_offsets",
            &[
                &starts.buffer,
                &root_blocks.values.buffer,
                root_blocks.total(),
                &error.buffer,
            ],
            &parameters,
            root_count
                .checked_add(1)
                .ok_or(Error::BackendCapacity)?
                .div_ceil(256),
        )?;
        commit_checked(command)?;
        wait_checked(command)?;
        status(&error)
    })?;
    let work = total(&root_blocks)?;
    if work[1..] != [0, 0, 0] {
        return Err(Error::BackendFailure);
    }
    let blocks = (work[0] as usize).div_ceil(256);
    parameters[1] = work[0];
    parameters[2] = checked(blocks)?;
    let counts = Scan::new(device, blocks)?;
    let digits = Scan::new(device, blocks)?;
    metal::objc::rc::autoreleasepool(|| {
        let command = queue.new_command_buffer();
        if blocks != 0 {
            run(
                command,
                "initial_range_counts",
                &[
                    &data.buffer,
                    &records.columns.buffer,
                    &records.counts.buffer,
                    &starts.buffer,
                    &counts.values.buffer,
                    &digits.values.buffer,
                    &error.buffer,
                    &units.buffer,
                ],
                &parameters,
                blocks,
            )?;
        }
        counts.encode(command)?;
        digits.encode(command)?;
        commit_checked(command)?;
        wait_checked(command)?;
        status(&error)
    })?;
    let values = total(&counts)?;
    let control = total(&digits)?;
    if control[1..] != [0, 0, 0] || values[0] > work[0] {
        return Err(Error::BackendFailure);
    }
    if std::env::var_os("TEMPO_PACKED_BATCH_PROFILE").is_some() {
        eprintln!(
            "TEMPO_METAL_INITIAL_RANGE_COUNTS {}",
            serde_json::json!({
                "class": format!("{work_class:?}"), "roots": root_count,
                "potentialRows": work[0], "states": values[0], "cursors": values[1],
                "repeats": values[2], "captures": values[3], "digits": control[0],
                "residentBytes": data.buffer.length(),
            })
        );
    }
    let capacity = FrontierCapacity {
        states: values[0] as usize,
        cursors: values[1] as usize,
        repeats: values[2] as usize,
        captures: values[3] as usize,
        digits: control[0] as usize,
    };
    let frontier = DynamicFrontier::allocate_exact(device, &capacity, 1)?;
    parameters[7..12].copy_from_slice(&[values[0], values[1], values[2], values[3], control[0]]);
    metal::objc::rc::autoreleasepool(|| {
        let command = queue.new_command_buffer();
        if blocks != 0 {
            run(
                command,
                "initial_range_scatter",
                &[
                    &data.buffer,
                    &records.columns.buffer,
                    &records.counts.buffer,
                    &starts.buffer,
                    &counts.values.buffer,
                    &digits.values.buffer,
                    &frontier.states.columns.buffer,
                    &frontier.control.spans.buffer,
                    &frontier.control.path_work.buffer,
                    &frontier.values.cursors.buffer,
                    &frontier.values.repeats.buffer,
                    &frontier.values.captures.buffer,
                    &frontier.control.digits.buffer,
                    &error.buffer,
                    &units.buffer,
                ],
                &parameters,
                blocks,
            )?;
        }
        run(
            command,
            "initial_range_headers",
            &[
                &frontier.states.counts.buffer,
                &frontier.values.counts.buffer,
                &frontier.control.count.buffer,
                &error.buffer,
            ],
            &parameters,
            1,
        )?;
        commit_checked(command)?;
        wait_checked(command)?;
        status(&error)
    })?;
    drop((seeds, starts, root_blocks, counts, digits, error, units));
    metal::objc::rc::autoreleasepool(|| {
        let command = queue.new_command_buffer();
        let ready = frontier.schedule(device, command, resident, plan, data)?;
        commit_checked(command)?;
        wait_checked(command)?;
        ready.check_completed()
    })?;
    if work_class == super::super::metal_initial_range_codegen::WorkClass::Completed {
        let ready = read::<u32>(&frontier.batch.counts, 17)?;
        if ready[..3] != [0, 0, 0] || ready[16] != 0 || ready[3] as usize != capacity.states {
            return Err(Error::BackendFailure);
        }
    }
    Ok(frontier)
}
