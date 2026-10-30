//! Shared transforms, accepted counts, and exact typed arena publication.
use super::Error;
use super::metal_backend::MappedBuffer;
use super::metal_backend::launch::{commit_checked, wait_checked};
use super::metal_batch_abi::{FinalCaptureRecord, PropertyRecord, ResultRecord};
use super::metal_candidate_batch::CandidateBatch;
use super::metal_grid_plan::ResidentInput;
use super::metal_output_dispatch::{OperatorInput, OutputParams, direct as run};
use super::metal_parallel::Scan;
use super::metal_register_execution::read;
use metal::{CommandQueueRef, Device};

#[cfg(all(test, feature = "metal-tests"))]
#[path = "metal_typed_batch_tests.rs"]
mod batch_tests;

pub(super) fn needed(resident: &ResidentInput) -> bool {
    resident.input.grid.evaluation_spans.iter().any(|span| {
        let pg = resident.input.packed.programs[span.program as usize];
        pg.output_count != 0 || pg.capture_count != 0
    })
}

pub(super) fn validate_operators(resident: &ResidentInput) -> Result<(), Error> {
    super::metal_grid_contract::validate(resident)
}

fn size(value: usize) -> Result<u32, Error> {
    u32::try_from(value).map_err(|_| Error::BackendCapacity)
}

fn total(scan: &Scan) -> Result<[u32; 4], Error> {
    let values = unsafe { scan.total().contents().cast::<[u32; 4]>().read() };
    if values.contains(&u32::MAX) {
        return Err(Error::BackendCapacity);
    }
    Ok(values)
}

fn check(status: &MappedBuffer) -> Result<(), Error> {
    match read::<u32>(status, 1)?[0] {
        0 => Ok(()),
        1 => Err(Error::BackendCapacity),
        _ => Err(Error::BackendFailure),
    }
}

fn operator_kinds(
    resident: &ResidentInput,
    maximum: usize,
    programs: &std::collections::BTreeSet<u32>,
) -> Vec<u8> {
    let mut kinds = vec![0u8; maximum];
    for &index in programs {
        let pg = resident.input.packed.programs[index as usize];
        let start = pg.output_start as usize;
        for (step, instruction) in resident.input.packed.assertion_data
            [start..start + pg.output_count as usize * 6]
            .chunks_exact(6)
            .enumerate()
        {
            kinds[step] |= 1u8 << instruction[0];
        }
    }
    kinds
}

pub(super) fn execute(
    device: &Device,
    queue: &CommandQueueRef,
    resident: &ResidentInput,
    data: &MappedBuffer,
    batches: Vec<CandidateBatch>,
    errors: Vec<Option<Error>>,
    programs: &super::metal_capture_predicates::PredicatePrograms<'_>,
) -> Result<(super::metal_output_pages::OutputArenas, Vec<Option<Error>>), Error> {
    let context = TypedContext {
        device,
        queue,
        resident,
        data,
        programs,
    };
    validate_operators(resident)?;
    if errors.len() != resident.input.grid.evaluation_spans.len() {
        return Err(Error::BackendFailure);
    }
    if batches.is_empty() {
        return Ok((
            super::metal_output_pages::OutputArenas::empty(device)?,
            errors,
        ));
    }
    let mut batches = batches;
    if batches.len() == 1 {
        return execute_producer(
            &context,
            batches.pop().ok_or(Error::BackendFailure)?,
            errors,
            true,
        );
    }
    let mut collector = OutputCollector::new(device, queue, resident, data, programs);
    for batch in batches {
        collector.push(batch, &errors)?;
    }
    collector.finish(errors)
}

/// Complete producers retire before the next structural execution class starts.
/// Global limits and late failures remain pending until all classes finish.
pub(super) struct OutputCollector<'a, 'p> {
    context: TypedContext<'a, 'p>,
    completed: Vec<super::metal_output_pages::TypedCheckpoint>,
    errors: Vec<Option<Error>>,
}
impl<'a, 'p> OutputCollector<'a, 'p> {
    pub(super) fn new(
        device: &'a Device,
        queue: &'a CommandQueueRef,
        resident: &'a ResidentInput,
        data: &'a MappedBuffer,
        programs: &'a super::metal_capture_predicates::PredicatePrograms<'p>,
    ) -> Self {
        Self {
            context: TypedContext {
                device,
                queue,
                resident,
                data,
                programs,
            },
            completed: vec![],
            errors: vec![None; resident.input.grid.evaluation_spans.len()],
        }
    }
    fn collect_errors(&mut self, found: Vec<Option<Error>>) -> Result<(), Error> {
        if self.errors.len() != found.len() {
            return Err(Error::BackendFailure);
        }
        for (destination, error) in self.errors.iter_mut().zip(found) {
            if let Some(error) = error {
                if destination.as_ref().is_some_and(|prior| *prior != error) {
                    return Err(Error::BackendFailure);
                }
                *destination = Some(error);
            }
        }
        Ok(())
    }
    pub(super) fn push(
        &mut self,
        batch: CandidateBatch,
        structural_errors: &[Option<Error>],
    ) -> Result<(), Error> {
        let (output, found) =
            execute_producer(&self.context, batch, structural_errors.to_vec(), false)?;
        self.collect_errors(found)?;
        self.completed
            .push(super::metal_output_pages::TypedCheckpoint::new(
                self.context.device,
                self.context.queue,
                output,
            )?);
        Ok(())
    }
    pub(super) fn finish(
        mut self,
        structural_errors: Vec<Option<Error>>,
    ) -> Result<(super::metal_output_pages::OutputArenas, Vec<Option<Error>>), Error> {
        self.collect_errors(structural_errors)?;
        let TypedContext {
            device,
            queue,
            resident,
            data,
            ..
        } = self.context;
        let output = super::metal_output_pages::TypedCheckpoint::merge_all(
            device,
            queue,
            resident,
            data,
            self.completed,
            &self.errors,
        )?;
        Ok((output, self.errors))
    }
}

#[derive(Clone, Copy)]
struct TypedContext<'a, 'p> {
    device: &'a Device,
    queue: &'a CommandQueueRef,
    resident: &'a ResidentInput,
    data: &'a MappedBuffer,
    programs: &'a super::metal_capture_predicates::PredicatePrograms<'p>,
}

fn execute_producer(
    context: &TypedContext<'_, '_>,
    batch: CandidateBatch,
    mut errors: Vec<Option<Error>>,
    apply_limits: bool,
) -> Result<(super::metal_output_pages::OutputArenas, Vec<Option<Error>>), Error> {
    let TypedContext {
        device,
        queue,
        resident,
        data,
        programs,
    } = *context;
    validate_operators(resident)?;
    if errors.len() != resident.input.grid.evaluation_spans.len() {
        return Err(Error::BackendFailure);
    }
    {
        if batch.result_count == 0 && batch.capture_count != 0 {
            return Err(Error::BackendFailure);
        }
        for (buffer, count, stride) in [
            (
                &batch.results,
                batch.result_count,
                std::mem::size_of::<ResultRecord>(),
            ),
            (
                &batch.captures,
                batch.capture_count,
                std::mem::size_of::<FinalCaptureRecord>(),
            ),
        ] {
            if count.checked_mul(stride).ok_or(Error::BackendCapacity)?
                > buffer.buffer.length() as usize
            {
                return Err(Error::BackendFailure);
            }
        }
    }
    let result_count = batch.result_count;
    let capture_count = batch.capture_count;
    if result_count == 0 {
        return Ok((
            super::metal_output_pages::OutputArenas::empty(device)?,
            errors,
        ));
    }
    let (rows, captures) = (batch.results, batch.captures);
    let meta = MappedBuffer::zeroed::<[u32; 4]>(device, result_count)?;
    let ids = [
        MappedBuffer::zeroed::<u32>(device, result_count)?,
        MappedBuffer::zeroed::<u32>(device, result_count)?,
    ];
    let owners = MappedBuffer::zeroed::<u32>(device, capture_count)?;
    let failures = MappedBuffer::from_slice(
        device,
        &errors
            .iter()
            .map(|error| u32::from(error.is_some()))
            .collect::<Vec<_>>(),
    )?;
    let status = MappedBuffer::zeroed::<u32>(device, 1)?;
    let membership = MappedBuffer::zeroed::<u32>(device, resident.input.packed.programs.len())?;
    let initial = OutputParams {
        results: size(result_count)?,
        captures: size(capture_count)?,
        step: 0,
        max_operators: 0,
        merge_width: 0,
        source_results: 0,
        source_captures: 0,
        result_base: 0,
        capture_base: 0,
        dynamic_slots: 0,
        transform_bytes: 0,
    };
    metal::objc::rc::autoreleasepool(|| -> Result<(), Error> {
        let command = queue.new_command_buffer();
        run(
            command,
            "output_initialize_results",
            &[
                &data.buffer,
                &rows.buffer,
                &meta.buffer,
                &ids[0].buffer,
                &failures.buffer,
                &status.buffer,
                &membership.buffer,
            ],
            &initial,
            result_count,
        )?;
        run(
            command,
            "output_initialize_captures",
            &[
                &data.buffer,
                &rows.buffer,
                &captures.buffer,
                &meta.buffer,
                &owners.buffer,
                &status.buffer,
            ],
            &initial,
            capture_count,
        )?;
        commit_checked(command)?;
        wait_checked(command)?;
        check(&status)
    })?;
    let flags = read::<u32>(&membership, resident.input.packed.programs.len())?;
    if flags.iter().any(|&flag| flag > 1) {
        return Err(Error::BackendFailure);
    }
    let active = flags
        .iter()
        .enumerate()
        .filter_map(|(index, &flag)| (flag == 1).then_some(index as u32))
        .collect::<std::collections::BTreeSet<_>>();
    if active.is_empty() {
        return Err(Error::BackendFailure);
    }
    drop(membership);
    let scoped = programs.scoped(&active)?;
    let programs = &scoped;
    // Query comparisons finish before any transform workspace is allocated.
    // Their scratch has no consumer after acceptance and must retire here.
    if let Some(predicates) =
        super::metal_capture_predicates::CapturePredicates::new(device, programs, result_count)?
    {
        predicates.execute(
            device,
            queue,
            resident,
            &OperatorInput {
                data,
                rows: &rows,
                captures: &captures,
                metadata: &meta,
                owners: &owners,
                status: &status,
                parameters: initial,
            },
            &failures,
        )?;
    }
    let text = MappedBuffer::zeroed::<[u32; 4]>(device, capture_count)?;
    let max_ops = active
        .iter()
        .map(|&id| resident.input.packed.programs[id as usize].output_count as usize)
        .max()
        .ok_or(Error::BackendFailure)?;
    let entities = result_count
        .checked_add(capture_count)
        .ok_or(Error::BackendCapacity)?;
    let slot_count = entities
        .checked_mul(max_ops)
        .ok_or(Error::BackendCapacity)?;
    size(entities)?;
    size(slot_count)?;
    let ranks = MappedBuffer::zeroed::<u32>(device, result_count)?;
    let slots = MappedBuffer::zeroed::<u32>(device, slot_count)?;
    let firsts = MappedBuffer::from_slice(device, &vec![u32::MAX; result_count])?;
    let kinds = operator_kinds(resident, max_ops, &active);
    let mut strips = kinds
        .iter()
        .any(|kind| kind & (1 << 3) != 0)
        .then(|| {
            super::metal_output_strip::StripOutput::new(
                device,
                result_count,
                super::MAX_STEPS_PER_PATH as u32,
            )
        })
        .transpose()?;
    let selection = kinds
        .iter()
        .any(|kind| kind & ((1 << 4) | (1 << 6)) != 0)
        .then(|| {
            super::metal_output_selection::CaptureSelection::new(
                device,
                result_count,
                capture_count,
            )
        })
        .transpose()?;
    let mime = super::metal_output_mime::MimeOutput::new(
        device,
        result_count,
        max_ops,
        kinds.iter().any(|kind| kind & (1 << 6) != 0),
    )?;
    let cap_scan = Scan::new(device, capture_count)?;
    let accepted = Scan::new(device, result_count)?;
    let limited = (apply_limits
        && resident
            .witness_limits
            .iter()
            .any(|&limit| limit != u32::MAX))
    .then(|| Scan::new(device, result_count))
    .transpose()?;
    let final_scan = limited.as_ref().unwrap_or(&accepted);
    let mut p = OutputParams {
        results: size(result_count)?,
        captures: size(capture_count)?,
        step: 0,
        max_operators: size(max_ops)?,
        merge_width: 0,
        source_results: 0,
        source_captures: 0,
        result_base: 0,
        capture_base: 0,
        dynamic_slots: size(mime.slots())?,
        transform_bytes: 0,
    };
    let mut current = 0;
    metal::objc::rc::autoreleasepool(|| -> Result<(), Error> {
        let mut command = queue.new_command_buffer();
        for (step, &kind) in kinds.iter().enumerate() {
            p.step = size(step)?;
            let input = OperatorInput {
                data,
                rows: &rows,
                captures: &captures,
                metadata: &meta,
                owners: &owners,
                status: &status,
                parameters: p,
            };
            if kind & ((1 << 4) | (1 << 6)) != 0 {
                let selection = selection.as_ref().ok_or(Error::BackendFailure)?;
                selection.encode_targets(command, &input)?;
                if kind & (1 << 4) != 0 {
                    selection.encode_adjacency(command, &input)?;
                }
                if kind & (1 << 6) != 0 {
                    mime.encode_step(command, &input, &selection.targets)?;
                }
            }
            run(
                command,
                "output_apply_results",
                &[
                    &data.buffer,
                    &rows.buffer,
                    &meta.buffer,
                    &slots.buffer,
                    &status.buffer,
                ],
                &p,
                result_count,
            )?;
            run(
                command,
                "output_apply_captures",
                &[
                    &data.buffer,
                    &rows.buffer,
                    &captures.buffer,
                    &meta.buffer,
                    &owners.buffer,
                    &text.buffer,
                    &slots.buffer,
                    &status.buffer,
                ],
                &p,
                capture_count,
            )?;
            if kind & (1 << 3) != 0 {
                commit_checked(command)?;
                wait_checked(command)?;
                check(&status)?;
                let strips = strips.as_mut().ok_or(Error::BackendFailure)?;
                strips.execute_step(
                    device,
                    queue,
                    super::metal_output_strip::StripStep {
                        input,
                        text: &text,
                        failures: &failures,
                        evaluations: resident.input.grid.evaluation_spans.len(),
                        program: programs.output_regex()?,
                    },
                )?;
                p.transform_bytes = strips.length();
                command = queue.new_command_buffer();
            }
        }
        run(
            command,
            "output_capture_sizes",
            &[
                &captures.buffer,
                &meta.buffer,
                &owners.buffer,
                &text.buffer,
                &cap_scan.values.buffer,
                &status.buffer,
                &firsts.buffer,
            ],
            &p,
            capture_count,
        )?;
        cap_scan.encode(command)?;
        let input = OperatorInput {
            data,
            rows: &rows,
            captures: &captures,
            metadata: &meta,
            owners: &owners,
            status: &status,
            parameters: p,
        };
        mime.encode_sizes(command, &input)?;
        let mut width = 1usize;
        if result_count > 1 {
            run(
                command,
                "output_sort_tiles",
                &[
                    &rows.buffer,
                    &ids[current].buffer,
                    &ids[1 - current].buffer,
                    &status.buffer,
                    &data.buffer,
                    &captures.buffer,
                    &firsts.buffer,
                    &meta.buffer,
                ],
                &p,
                result_count,
            )?;
            current = 1 - current;
            width = 256;
        }
        while width < result_count {
            p.merge_width = size(width)?;
            run(
                command,
                "output_merge",
                &[
                    &rows.buffer,
                    &ids[current].buffer,
                    &ids[1 - current].buffer,
                    &status.buffer,
                    &data.buffer,
                    &captures.buffer,
                    &firsts.buffer,
                    &meta.buffer,
                ],
                &p,
                result_count,
            )?;
            current = 1 - current;
            width = width.checked_mul(2).ok_or(Error::BackendCapacity)?;
        }
        run(
            command,
            "output_candidate_sizes",
            &[
                &rows.buffer,
                &meta.buffer,
                &ids[current].buffer,
                &cap_scan.values.buffer,
                cap_scan.total(),
                &accepted.values.buffer,
                &ranks.buffer,
                &status.buffer,
                &mime.offsets.values.buffer,
                mime.offsets.total(),
            ],
            &p,
            result_count,
        )?;
        accepted.encode(command)?;
        if let Some(limited) = &limited {
            run(
                command,
                "output_limit_sizes",
                &[
                    &data.buffer,
                    &rows.buffer,
                    &meta.buffer,
                    &ids[current].buffer,
                    &cap_scan.values.buffer,
                    cap_scan.total(),
                    &accepted.values.buffer,
                    &limited.values.buffer,
                    &status.buffer,
                    &mime.offsets.values.buffer,
                    mime.offsets.total(),
                ],
                &p,
                result_count,
            )?;
            limited.encode(command)?;
        }
        commit_checked(command)?;
        wait_checked(command)?;
        check(&status)
    })?;
    let failure_codes = read::<u32>(&failures, errors.len())?;
    for (code, error) in failure_codes.into_iter().zip(&mut errors) {
        match (code, error.as_ref()) {
            (0, None) | (1, Some(_)) => {}
            (2, None) => *error = Some(Error::WorkLimit),
            _ => return Err(Error::BackendFailure),
        }
    }
    total(&cap_scan)?;
    total(&accepted)?;
    mime.check_completed()?;
    let totals = total(final_scan)?;
    if totals[0] as usize > result_count
        || totals[1] as usize > capture_count
        || totals[2] as usize > slot_count
    {
        return Err(Error::BackendFailure);
    }
    let output_layout = MappedBuffer::zeroed::<[u32; 4]>(device, totals[0] as usize)?;
    let output_rows = MappedBuffer::zeroed::<ResultRecord>(device, totals[0] as usize)?;
    let output_captures = MappedBuffer::zeroed::<FinalCaptureRecord>(device, totals[1] as usize)?;
    let properties = MappedBuffer::zeroed::<PropertyRecord>(device, totals[2] as usize)?;
    let storage = MappedBuffer::zeroed::<u8>(device, totals[3] as usize)?;
    metal::objc::rc::autoreleasepool(|| -> Result<(), Error> {
        let command = queue.new_command_buffer();
        run(
            command,
            "output_publish",
            &[
                &data.buffer,
                &rows.buffer,
                &captures.buffer,
                &meta.buffer,
                &ids[current].buffer,
                &owners.buffer,
                &ranks.buffer,
                &text.buffer,
                &slots.buffer,
                &cap_scan.values.buffer,
                cap_scan.total(),
                &final_scan.values.buffer,
                final_scan.total(),
                &status.buffer,
                &mime.snapshots.buffer,
                &mime.offsets.values.buffer,
                mime.offsets.total(),
                &strips
                    .as_ref()
                    .and_then(super::metal_output_strip::StripOutput::bytes)
                    .unwrap_or(data)
                    .buffer,
                &output_rows.buffer,
                &output_captures.buffer,
                &properties.buffer,
                &storage.buffer,
                &output_layout.buffer,
            ],
            &p,
            result_count
                .max(capture_count)
                .max(slot_count)
                .max(totals[3] as usize),
        )?;
        commit_checked(command)?;
        wait_checked(command)?;
        check(&status)
    })?;
    Ok((
        super::metal_output_pages::OutputArenas {
            rows: output_rows,
            captures: output_captures,
            properties,
            bytes: storage,
            layout: Some(output_layout),
            counts: totals,
        },
        errors,
    ))
}
