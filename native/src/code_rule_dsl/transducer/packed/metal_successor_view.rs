//! Compare immutable successor views, then retire each source before publication.
use super::super::metal_backend::launch::CheckedComputeEncoder;
use super::super::metal_parallel::{Scan, encode};
use super::*;
use metal::{BufferRef, CommandQueueRef, MTLSize};

#[path = "metal_successor_guard_codegen.rs"]
mod guards;

// Neither bridge is a FrontierRecords value. Only checked conversion can publish it.
struct CopyBridge {
    columns: MappedBuffer,
    spans: MappedBuffer,
    path_work: MappedBuffer,
    count: usize,
}
fn checked(n: usize) -> Result<u32, Error> {
    u32::try_from(n).map_err(|_| Error::BackendCapacity)
}
fn status(buffer: &MappedBuffer) -> Result<(), Error> {
    let values = unsafe { buffer.buffer.contents().cast::<[u32; 4]>().read() };
    match values[0] {
        0 => {}
        1 => return Err(Error::BackendCapacity),
        _ => return Err(Error::BackendFailure),
    }
    if values[1..3] != [u32::MAX, u32::MAX] || values[3] > 1 {
        return Err(Error::BackendFailure);
    }
    Ok(())
}
fn total(scan: &Scan) -> Result<[u32; 4], Error> {
    let v = unsafe { scan.total().contents().cast::<[u32; 4]>().read() };
    if v.contains(&u32::MAX) {
        return Err(Error::BackendCapacity);
    }
    Ok(v)
}
fn dispatch(
    command: &CommandBufferRef,
    source: &str,
    name: &str,
    buffers: &[&BufferRef],
    parameters: &[u32],
    count: usize,
) -> Result<(), Error> {
    encode(
        command,
        &super::super::metal_backend::pipeline_named(source, name)?,
        name,
        buffers,
        parameters,
        count.div_ceil(256).max(1),
        None,
    );
    Ok(())
}
fn complete(
    queue: &CommandQueueRef,
    body: impl FnOnce(&CommandBufferRef) -> Result<(), Error>,
) -> Result<(), Error> {
    metal::objc::rc::autoreleasepool(|| {
        let command = queue.new_command_buffer();
        body(command)?;
        commit_checked(command)?;
        wait_checked(command)
    })
}

fn shader_source(guard_source: &str) -> String {
    let common = include_str!("metal_successor_common.metal")
        .replace(
            "// STEP_HELPERS",
            &include_str!("metal_step_helpers.metal").replace("STEP_ADDRESS", "device"),
        )
        .replace(
            "SUCCESSOR_HEADER_WORDS",
            &super::super::metal_successor_codegen::STEP_HEADER_WORDS.to_string(),
        )
        .replace(
            "PREFIX_EDIT_CAPACITY",
            &super::super::metal_successor_codegen::PREFIX_EDITS.to_string(),
        )
        .replace(
            "SUCCESSOR_UNIT_INSTRUCTIONS",
            &super::super::bounded_repeat_proof::MAX_JOB_INSTRUCTIONS.to_string(),
        );
    format!(
        "#define INPUT_COLUMNS 12u\n{}\n{}\n{common}\n{}\n{}\n{}\n{}",
        include_str!("metal_resident.metal"),
        include_str!("metal_input_layout.metal"),
        include_str!("metal_task_groups.metal"),
        include_str!("metal_ready_batch.metal"),
        guard_source,
        include_str!("metal_successor_view.metal").replace(
            "EXPANDED_UNIT_INSTRUCTIONS",
            &super::super::bounded_repeat_proof::MAX_JOB_INSTRUCTIONS.to_string()
        )
    )
}

pub(super) struct Context<'a> {
    pub device: &'a Device,
    pub queue: &'a CommandQueueRef,
    pub resident: &'a ResidentInput,
    pub plan: &'a ExecutionPlan,
    pub data: &'a MappedBuffer,
}

pub(super) fn execute(
    context: Context<'_>,
    frontier: DynamicFrontier,
    preparation: PreparedSuccessors,
) -> Result<DynamicFrontier, Error> {
    let Context {
        device,
        queue,
        resident,
        plan,
        data,
    } = context;
    let bound = preparation.check_completed()?;
    let needs = preparation.comparison_needs()?;
    if needs.pending {
        return Err(Error::BackendFailure);
    }
    let keys = needs.active_states.ok_or(Error::BackendFailure)?;
    let raw = bound.states;
    if keys > raw || (keys != 0) != needs.active {
        return Err(Error::BackendFailure);
    }
    if raw > 0x0fff_ffff {
        return Err(Error::BackendCapacity);
    }
    let completed = preparation.into_completed(&frontier.input(resident, plan, data))?;
    let super::super::metal_successor_prepare::CompletedSteps {
        status: error,
        steps,
        expansion,
        code,
        mut parameters,
    } = completed;
    let DynamicFrontier {
        states,
        values,
        control,
        batch,
    } = frontier;
    let generation = states
        .generation
        .checked_add(1)
        .ok_or(Error::BackendFailure)?;
    let input_count = read::<u32>(&states.counts, 5)?[0] as usize;
    if input_count > states.capacity {
        return Err(Error::BackendFailure);
    }
    let ControlValues {
        spans: old_input_spans,
        digits: input_digits,
        count: input_digit_count,
        path_work: input_work,
        ..
    } = control;
    drop((batch, input_digit_count, input_work));
    let guard_code = guards::compile(resident, plan)?;
    let guard_ends = MappedBuffer::from_slice(device, &guard_code.ends)?;
    drop(code);
    let source = shader_source(&guard_code.source);
    let slots_count = if keys == 0 {
        0
    } else {
        keys.checked_add(keys.div_ceil(2))
            .ok_or(Error::BackendCapacity)?
            .max(16)
    };
    parameters[1] = checked(raw)?;
    parameters[17] = checked(slots_count)?;
    parameters.extend([checked(input_count)?, 0, 0]);
    let input_values = MappedBuffer::zeroed::<[u32; 6]>(device, input_count)?;
    let input_spans = MappedBuffer::zeroed::<[u32; 2]>(device, input_count)?;
    complete(queue, |command| {
        dispatch(
            command,
            &source,
            "expanded_project_input",
            &[
                &data.buffer,
                &states.columns.buffer,
                &old_input_spans.buffer,
                &input_values.buffer,
                &input_spans.buffer,
                &error.buffer,
            ],
            &parameters,
            input_count,
        )
    })?;
    status(&error)?;
    drop((states, old_input_spans));

    let selected = Scan::new(device, raw)?;
    let position_counts = Scan::new(device, raw.div_ceil(256))?;
    {
        let heads = MappedBuffer::zeroed::<u32>(device, slots_count)?;
        let slots = MappedBuffer::zeroed::<u32>(device, if needs.active { raw } else { 0 })?;
        complete(queue, |command| {
            let base: [&BufferRef; 11] = [
                &data.buffer,
                &input_values.buffer,
                &values.cursors.buffer,
                &values.repeats.buffer,
                &values.captures.buffer,
                &input_spans.buffer,
                &input_digits.buffer,
                &steps.buffer,
                &expansion.values.buffer,
                expansion.total(),
                &error.buffer,
            ];
            if needs.active {
                let mut bindings = base.to_vec();
                bindings.extend([&*heads.buffer, &slots.buffer]);
                let encoder = command.new_compute_command_encoder();
                encoder.set_label("expanded_keys");
                let pipeline =
                    super::super::metal_backend::pipeline_named(&source, "expanded_keys")?;
                encoder.set_compute_pipeline_state(&pipeline);
                encoder.checked_set_buffers(command, &bindings);
                encoder.set_bytes(
                    bindings.len() as u64,
                    std::mem::size_of_val(parameters.as_slice()) as u64,
                    parameters.as_ptr().cast(),
                );
                encoder.checked_dispatch_thread_groups(
                    command,
                    MTLSize::new(raw as u64, 1, 1),
                    MTLSize::new(32, 1, 1),
                );
                encoder.end_encoding();
            }
            let mut bindings = base.to_vec();
            bindings.extend([
                &*heads.buffer,
                &slots.buffer,
                &selected.values.buffer,
                &position_counts.values.buffer,
                &guard_ends.buffer,
            ]);
            dispatch(
                command,
                &source,
                "expanded_counts",
                &bindings,
                &parameters,
                raw,
            )?;
            selected.encode_independent_pair(&position_counts, command)?;
            Ok(())
        })?;
        status(&error)?;
    }
    let lengths = total(&selected)?;
    let positions = total(&position_counts)?;
    if lengths[0] as usize > raw
        || lengths[1..].iter().any(|&n| n as usize > bound.cells)
        || positions[1..] != [0, 0, 0]
        || positions[0] as usize > bound.cells
    {
        return Err(Error::BackendFailure);
    }
    let count = lengths[0] as usize;
    parameters[10..14].copy_from_slice(&[lengths[1], lengths[2], lengths[3], positions[0]]);
    parameters[23] = lengths[0];
    if std::env::var_os("TEMPO_PACKED_BATCH_PROFILE").is_some() {
        eprintln!(
            "TEMPO_METAL_SUCCESSOR_VIEW_COUNTS stage=counted raw={raw} survivors={count} cursors={} repeats={} captures={} digits={}",
            lengths[1], lengths[2], lengths[3], positions[0]
        );
    }
    let bridge = CopyBridge {
        columns: MappedBuffer::zeroed::<[u32; POSITIONED_COLUMNS]>(device, count)?,
        spans: MappedBuffer::zeroed::<[u32; 6]>(device, count)?,
        path_work: MappedBuffer::zeroed::<u32>(device, count)?,
        count,
    };
    complete(queue, |command| {
        dispatch(
            command,
            &source,
            "expanded_metadata",
            &[
                &data.buffer,
                &input_values.buffer,
                &values.cursors.buffer,
                &values.repeats.buffer,
                &values.captures.buffer,
                &input_spans.buffer,
                &input_digits.buffer,
                &steps.buffer,
                &expansion.values.buffer,
                expansion.total(),
                &error.buffer,
                &selected.values.buffer,
                selected.total(),
                &bridge.columns.buffer,
                &guard_ends.buffer,
                &position_counts.values.buffer,
                &bridge.spans.buffer,
                &bridge.path_work.buffer,
            ],
            &parameters,
            raw,
        )
    })?;
    status(&error)?;
    drop((
        input_values,
        guard_ends,
        input_spans,
        selected,
        position_counts,
        expansion,
    ));
    let StateValues {
        cursors: old_cursors,
        repeats: old_repeats,
        captures: old_captures,
        counts: old_counts,
        ..
    } = values;
    drop(old_counts);
    // Completed GPU totals define allocation headers. These are lengths and
    // dimensions only; the host never classifies or interprets a frontier row.
    let state_header = MappedBuffer::from_slice(
        device,
        &[lengths[0], lengths[3], checked(count.div_ceil(256))?, 1, 1],
    )?;
    let value_header = MappedBuffer::from_slice(device, &[lengths[1], lengths[2]])?;
    let digit_header = MappedBuffer::from_slice(device, &[positions[0]])?;
    let ready = super::super::metal_ready_batch::FusedReady::new(
        device,
        resident,
        plan,
        data,
        super::super::metal_ready_batch::FusedReadyFrame {
            columns: &bridge.columns,
            counts: &state_header,
            value_counts: &value_header,
            capacity: count,
            generation,
            values: [
                lengths[1] as usize,
                lengths[2] as usize,
                lengths[3] as usize,
            ],
        },
    )?;
    parameters.extend_from_slice(ready.parameters());
    let cursors = MappedBuffer::zeroed::<[u32; 4]>(device, lengths[1] as usize)?;
    let repeats = MappedBuffer::zeroed::<[u32; 4]>(device, lengths[2] as usize)?;
    let digits = MappedBuffer::zeroed::<u32>(device, positions[0] as usize)?;
    let run_rows =
        |command: &CommandBufferRef, name, buffers: &[&BufferRef]| -> Result<(), Error> {
            let encoder = command.new_compute_command_encoder();
            encoder.set_label(name);
            let pipeline = super::super::metal_backend::pipeline_named(&source, name)?;
            encoder.set_compute_pipeline_state(&pipeline);
            encoder.checked_set_buffers(command, buffers);
            encoder.set_bytes(
                buffers.len() as u64,
                std::mem::size_of_val(parameters.as_slice()) as u64,
                parameters.as_ptr().cast(),
            );
            encoder.checked_dispatch_thread_groups(
                command,
                MTLSize::new(count.max(1) as u64, 1, 1),
                MTLSize::new(32, 1, 1),
            );
            encoder.end_encoding();
            Ok(())
        };
    complete(queue, |command| {
        run_rows(
            command,
            "expanded_copy_small",
            &[
                &data.buffer,
                &bridge.columns.buffer,
                &bridge.spans.buffer,
                &steps.buffer,
                &old_cursors.buffer,
                &old_repeats.buffer,
                &input_digits.buffer,
                &cursors.buffer,
                &repeats.buffer,
                &digits.buffer,
                &error.buffer,
                &state_header.buffer,
                &value_header.buffer,
                &ready.tasks.buffer,
                &ready.status.buffer,
                &ready.counts.buffer,
            ],
        )
    })?;
    status(&error)?;
    ready.check_status()?;
    drop((old_cursors, old_repeats, input_digits));
    let captures = MappedBuffer::zeroed::<[u32; 4]>(device, lengths[3] as usize)?;
    complete(queue, |command| {
        run_rows(
            command,
            "expanded_publish",
            &[
                &data.buffer,
                &bridge.columns.buffer,
                &bridge.spans.buffer,
                &steps.buffer,
                &old_captures.buffer,
                &captures.buffer,
                &error.buffer,
                &state_header.buffer,
                &value_header.buffer,
                &ready.proof.buffer,
                &ready.tasks.buffer,
                &ready.status.buffer,
                &ready.counts.buffer,
            ],
        )
    })?;
    status(&error)?;
    ready.check_status()?;
    drop((old_captures, steps, error));
    let ready_counts = MappedBuffer {
        buffer: ready.counts.buffer.clone(),
    };
    let frontier = DynamicFrontier {
        states: PositionedFrontier {
            columns: bridge.columns,
            counts: state_header,
            capacity: bridge.count,
            generation,
        },
        values: StateValues {
            cursors,
            repeats,
            captures,
            counts: value_header,
            cursor_capacity: lengths[1] as usize,
            repeat_capacity: lengths[2] as usize,
            capture_capacity: lengths[3] as usize,
            generation,
        },
        control: ControlValues {
            spans: bridge.spans,
            digits,
            count: digit_header,
            path_work: bridge.path_work,
            state_capacity: count,
            digit_capacity: positions[0] as usize,
            generation,
        },
        batch: ReadyBatch {
            indices: MappedBuffer::zeroed::<[u32; 4]>(device, count)?,
            counts: ready_counts,
            capacity: count,
            generation,
        },
    };
    metal::objc::rc::autoreleasepool(|| {
        let command = queue.new_command_buffer();
        let pending = ready.encode_selection(
            command,
            super::super::metal_ready_batch::ReadyInput {
                resident,
                plan,
                data,
                states: &frontier.states,
                values: &frontier.values,
            },
            &frontier.batch,
        )?;
        commit_checked(command)?;
        wait_checked(command)?;
        pending.check_completed()
    })?;
    Ok(frontier)
}

#[cfg(test)]
pub(super) fn copy_captures_for_test(
    context: Context<'_>,
    fixtures: &[super::CaptureCopyFixture],
) -> Result<Vec<[u32; 4]>, Error> {
    let Context {
        device,
        queue,
        resident,
        plan,
        data,
    } = context;
    let n = fixtures.len();
    let mut columns = vec![0u32; n * 12];
    let mut spans = vec![0u32; n * 6];
    let mut steps = Vec::new();
    let mut saved = Vec::new();
    let mut cells = 0usize;
    let header = super::super::metal_successor_codegen::STEP_HEADER_WORDS;
    for (row, fixture) in fixtures.iter().enumerate() {
        assert!(fixture.added.len() <= super::super::metal_successor_codegen::PREFIX_EDITS);
        columns[3 * n + row] = checked(steps.len())?;
        columns[10 * n + row] = checked(saved.len())?;
        columns[11 * n + row] = checked(fixture.saved.len() + fixture.added.len())?;
        spans[5 * n + row] = checked(cells)?;
        cells += fixture.saved.len() + fixture.added.len();
        let at = steps.len();
        steps.resize(at + header, 0u32);
        steps[at + 10] = checked(header)? | (checked(header)? << 16);
        steps[at + 11] = checked(header + 4 * fixture.added.len())?;
        steps[at + 12] = 1 | (1 << 8);
        steps[at + 13] = checked(fixture.added.len())? << 16;
        steps[at + 14] = 1;
        steps.extend(fixture.added.iter().flatten().copied());
        saved.extend_from_slice(&fixture.saved);
    }
    let columns = MappedBuffer::from_slice(device, &columns)?;
    let spans = MappedBuffer::from_slice(device, &spans)?;
    let steps = MappedBuffer::from_slice(device, &steps)?;
    let saved = MappedBuffer::from_slice(device, &saved)?;
    let output = MappedBuffer::zeroed::<[u32; 4]>(device, cells)?;
    let error = MappedBuffer::from_slice(device, &[0u32, u32::MAX, u32::MAX, 1])?;
    let mut parameters = vec![0u32; 25];
    parameters[12] = checked(cells)?;
    parameters[23] = checked(n)?;
    parameters[24] = 2;
    let guards = guards::compile(resident, plan)?;
    let source = shader_source(&guards.source);
    complete(queue, |command| {
        let pipeline = super::super::metal_backend::pipeline_named(&source, "expanded_copy")?;
        let encoder = command.new_compute_command_encoder();
        encoder.set_label("expanded_copy");
        encoder.set_compute_pipeline_state(&pipeline);
        encoder.checked_set_buffers(
            command,
            &[
                &data.buffer,
                &columns.buffer,
                &spans.buffer,
                &steps.buffer,
                &saved.buffer,
                &output.buffer,
                &error.buffer,
            ],
        );
        encoder.set_bytes(
            7,
            std::mem::size_of_val(parameters.as_slice()) as u64,
            parameters.as_ptr().cast(),
        );
        encoder.checked_dispatch_thread_groups(
            command,
            MTLSize::new(n.max(1) as u64, 1, 1),
            MTLSize::new(32, 1, 1),
        );
        encoder.end_encoding();
        Ok(())
    })?;
    status(&error)?;
    read::<[u32; 4]>(&output, cells)
}
