//! Count surviving finite jobs before allocating compact device frames.
use super::*;

fn completed_status(status: &MappedBuffer) -> Result<(), Error> {
    let values = unsafe { status.buffer.contents().cast::<[u32; 4]>().read() };
    match values[0] {
        0 => {}
        1 => return Err(Error::BackendCapacity),
        _ => return Err(Error::BackendFailure),
    }
    if values[1..3] != [u32::MAX, u32::MAX] || values[3] > 3 {
        return Err(Error::BackendFailure);
    }
    Ok(())
}

pub(super) fn finish<const FIELDS: usize>(
    device: &Device,
    queue: &metal::CommandQueueRef,
    input: &SuccessorInput<'_, FIELDS>,
    mut counted: CountedInputs<FIELDS>,
) -> Result<PreparedSuccessors<FIELDS>, Error> {
    let [jobs, _] = counted.completed_counts()?;
    counted.validate_binding(input)?;
    counted
        .code
        .source
        .insert_str(0, "#define SUCCESSOR_REGISTER_FRAMES 1\n");
    counted.parameters[21] = u32::try_from(jobs).map_err(|_| Error::BackendCapacity)?;
    let blocks = jobs.div_ceil(256);
    let counts = Scan::new(device, blocks)?;
    let run = |command: &CommandBufferRef,
               name,
               output: &[&metal::BufferRef],
               parameters: &[u32]|
     -> Result<(), Error> {
        let mut buffers = vec![
            &*input.data.buffer,
            &input.states.columns.buffer,
            &input.states.counts.buffer,
            &input.values.cursors.buffer,
            &input.values.repeats.buffer,
            &counted.slots.buffer,
            &counted.signatures.values.buffer,
            &input.control.spans.buffer,
            &counted.sites.buffer,
            &input.control.path_work.buffer,
            &counted.status.buffer,
        ];
        buffers.extend_from_slice(output);
        dispatch(
            command,
            &pipeline_named(&counted.code.source, name)?,
            name,
            &buffers,
            parameters,
            blocks.max(1),
            None,
        );
        Ok(())
    };
    metal::objc::rc::autoreleasepool(|| {
        let command = queue.new_command_buffer();
        run(
            command,
            "successor_register_counts",
            &[&counts.values.buffer],
            &counted.parameters,
        )?;
        counts.encode(command)?;
        super::super::metal_backend::launch::commit_checked(command)?;
        super::super::metal_backend::launch::wait_checked(command)?;
        completed_status(&counted.status)?;
        Ok::<_, Error>(())
    })?;
    drop((counted.command, counted.categories));
    let totals = unsafe { counts.total().contents().cast::<[u32; 4]>().read() };
    if totals.contains(&u32::MAX) {
        return Err(Error::BackendCapacity);
    }
    let [frames, words] = [totals[0] as usize, totals[1] as usize];
    if totals[2..] != [0, 0]
        || frames > jobs
        || words
            < frames
                .checked_mul(metal_successor_codegen::STEP_HEADER_WORDS)
                .ok_or(Error::BackendCapacity)?
        || words
            > frames
                .checked_mul(counted.code.maximum_step_words)
                .ok_or(Error::BackendCapacity)?
        || !words.is_multiple_of(4)
    {
        return Err(Error::BackendFailure);
    }
    counted.parameters[18] = totals[0];
    counted.parameters[20] = totals[1];
    let expansion = Scan::new(device, frames)?;
    let steps = MappedBuffer::zeroed::<u32>(device, words)?;
    // Retain only the final command; its checked completion releases all count readers.
    let command = metal::objc::rc::autoreleasepool(|| {
        let command = queue.new_command_buffer();
        run(
            command,
            "successor_register_scatter",
            &[
                &counts.values.buffer,
                &steps.buffer,
                &expansion.values.buffer,
            ],
            &counted.parameters,
        )?;
        expansion.encode(command)?;
        super::super::metal_backend::launch::commit_checked(command)?;
        super::super::metal_backend::launch::wait_checked(command)?;
        completed_status(&counted.status)?;
        Ok::<_, Error>(command.to_owned())
    })?;
    if std::env::var_os("TEMPO_PACKED_BATCH_PROFILE").is_some() {
        eprintln!("TEMPO_METAL_FINITE_FRAME_COUNTS jobs={jobs} frames={frames} words={words}");
    }
    let prepared = PreparedSuccessors {
        command,
        status: counted.status,
        steps,
        expansion,
        code: counted.code,
        parameters: counted.parameters,
        bindings: counted.bindings,
        generation: counted.generation,
        resident: counted.resident,
        plan: counted.plan,
        _scans: vec![],
        _buffers: vec![],
    };
    prepared.check_completed()?;
    Ok(prepared)
}
