//! One resident upload, broad GPU stages, and disjoint scratch partitions.
use super::metal_backend::launch::{CheckedComputeEncoder, commit_checked, wait_checked};
use super::metal_backend::{MappedBuffer, PrivateInputUpload, pipeline_named};
use super::metal_batch_abi::{FinalCaptureRecord, NONE, ResultRecord};
use super::metal_grid_plan::{ExecutionPlan, ResidentInput, StageKind};
use super::metal_initial_range_codegen::{self, WorkClass};
use super::metal_parallel::Scan;
use super::metal_register_codegen::{self, RegisterClass, RegisterCode};
use super::*;
use metal::{BufferRef, CommandBufferRef, ComputePipelineState, Device, MTLSize};
use std::mem::size_of;

#[repr(u32)]
pub(super) enum RegisterOutputClass {
    All = 0,
    SharedOperators = 1,
    ConstantProperties = 2,
}

fn u32_size(value: usize) -> Result<u32, Error> {
    u32::try_from(value).map_err(|_| Error::BackendCapacity)
}
fn bytes<T: Copy>(values: &[T]) -> &[u8] {
    // All uploaded types contain initialized scalar fields and no padding.
    unsafe { std::slice::from_raw_parts(values.as_ptr().cast(), std::mem::size_of_val(values)) }
}

pub(super) fn upload(device: &Device, resident: &ResidentInput) -> Result<MappedBuffer, Error> {
    let batch = &resident.input.packed;
    let grid = &resident.input.grid;
    let mut relations = vec![[NONE; 2]; batch.nodes.len()];
    let mut fields = BTreeSet::new();
    for (index, node) in batch.nodes.iter().enumerate() {
        if node.parent != NONE {
            let parent = relations
                .get_mut(node.parent as usize)
                .ok_or(Error::InvalidAnchor)?;
            if node.flags & 1 != 0 {
                if parent[0] == NONE {
                    parent[0] = u32_size(index)?;
                }
                parent[1] = u32_size(index)?;
            }
            if node.field != 0 {
                fields.insert([node.parent, node.field]);
            }
        }
    }
    let fields = fields.into_iter().collect::<Vec<_>>();
    let mut supertypes = batch
        .supertypes
        .iter()
        .map(|entry| [entry.node, entry.kind])
        .collect::<Vec<_>>();
    supertypes.sort_unstable();
    supertypes.dedup();
    let mut positions = vec![0_u32; grid.block_spans.len()];
    let mut first = vec![u32::MAX; grid.evaluation_spans.len()];
    for (position, &id) in grid.semantic_span_order.iter().enumerate() {
        positions[id as usize] = u32_size(position)?;
        let evaluation = grid.block_spans[id as usize].evaluation_span as usize;
        first[evaluation] = first[evaluation].min(u32_size(position)?);
    }
    for root in &grid.roots {
        let file = batch
            .files
            .get(root.file as usize)
            .ok_or(Error::InvalidAnchor)?;
        if root.node >= file.node_count {
            return Err(Error::InvalidAnchor);
        }
    }
    let unlimited = vec![u32::MAX; resident.witness_limits.len()];
    let structural_limits = if super::metal_typed_output::needed(resident) {
        &unlimited
    } else {
        &resident.witness_limits
    };
    let named_order = super::metal_named_order::build(batch)?;
    let value_prefixes = super::value_prefix::build(batch)?;
    let arrays = [
        bytes(&batch.nodes),
        bytes(&batch.files),
        bytes(&grid.roots),
        bytes(&grid.selectors),
        bytes(&grid.selector_data),
        bytes(&grid.evaluation_spans),
        bytes(&grid.block_spans),
        bytes(&grid.blocks),
        bytes(&batch.words),
        bytes(&batch.programs),
        bytes(&batch.longs),
        bytes(&relations),
        bytes(&fields),
        bytes(&supertypes),
        bytes(&grid.semantic_span_order),
        bytes(&positions),
        bytes(&first),
        bytes(structural_limits),
        bytes(&batch.children),
    ];
    let semantic_arrays = [
        bytes(&batch.source),
        bytes(&batch.strings),
        bytes(&batch.string_bytes),
        bytes(&batch.line_starts),
        bytes(&batch.assertion_data),
        bytes(&resident.witness_limits),
    ];
    let arrays = arrays
        .into_iter()
        .enumerate()
        .chain(
            semantic_arrays
                .into_iter()
                .enumerate()
                .map(|(index, data)| (index + 22, data)),
        )
        .chain([(33, bytes(&batch.values)), (34, bytes(&batch.assertions))])
        .chain([
            (35, bytes(resident.input.children.field_nodes())),
            (36, bytes(resident.input.children.kind_nodes())),
            (39, bytes(&named_order)),
            (40, bytes(&value_prefixes)),
        ])
        .collect::<Vec<_>>();
    resident.input.children.validate(batch)?;
    let mut header = [0_u32; 41];
    let mut length = size_of_val(&header);
    for &(index, array) in &arrays {
        length = length.checked_add(15).ok_or(Error::BackendCapacity)? / 16 * 16;
        header[index] = u32_size(length)?;
        length = length
            .checked_add(array.len())
            .ok_or(Error::BackendCapacity)?;
    }
    header[19] = grid.header.root_count;
    header[20] = u32_size(fields.len())?;
    header[21] = u32_size(supertypes.len())?;
    header[28] = u32_size(grid.semantic_span_order.len())?;
    header[29] = u32_size(grid.evaluation_spans.len())?;
    header[30] = u32_size(batch.programs.len())?;
    header[31] = u32_size(batch.source.len())?;
    header[32] = u32_size(batch.assertion_data.len())?;
    header[37] = u32_size(resident.input.children.field_nodes().len())?;
    header[38] = u32_size(resident.input.children.kind_nodes().len())?;
    let buffer = MappedBuffer::private_capacity::<u8>(device, length)?;
    let mut upload = PrivateInputUpload::new(device)?;
    upload.write(&buffer, 0, &header)?;
    for &(index, array) in &arrays {
        let aligned = array.len() / 4 * 4;
        upload.write(&buffer, header[index] as usize, &array[..aligned])?;
        if aligned != array.len() {
            // Blits require four-byte units. The header retains the exact
            // semantic length; the next array begins on a 16-byte boundary.
            let mut tail = [0u8; 4];
            tail[..array.len() - aligned].copy_from_slice(&array[aligned..]);
            upload.write(&buffer, header[index] as usize + aligned, &tail)?;
        }
    }
    upload.finish()?;
    Ok(buffer)
}

struct Partition {
    first: usize,
    count: usize,
    direct: Vec<u32>,
    bounded: Vec<u32>,
    result_bound: usize,
    capture_bound: usize,
    eval_first: u32,
    eval_count: u32,
}

fn partitions(
    resident: &ResidentInput,
    code: &RegisterCode,
    span_limit: u32,
    constant: Option<&super::metal_constant_output::ConstantOutput<'_>>,
) -> Result<Vec<Partition>, Error> {
    let grid = &resident.input.grid;
    let mut partitions = Vec::new();
    let mut first = 0;
    while first < grid.semantic_span_order.len() {
        let mut direct = BTreeSet::new();
        let mut bounded = BTreeSet::new();
        let mut result_bound = 0usize;
        let mut capture_bound = 0usize;
        let mut end = first;
        while end < grid.semantic_span_order.len() && end - first < span_limit as usize {
            let span = grid.block_spans[grid.semantic_span_order[end] as usize];
            if let Some(constant) = constant {
                let evaluation = grid.evaluation_spans[span.evaluation_span as usize];
                if constant.owns(evaluation.program as usize)? {
                    end += 1;
                    continue;
                }
            }
            let block = grid.blocks[span.execution_block as usize];
            let Some(bound) = code.bounds.get(&block.dispatch_signature) else {
                end += 1;
                continue;
            };
            let results = (span.root_count as usize)
                .checked_mul(bound.matches)
                .ok_or(Error::BackendCapacity)?;
            result_bound = result_bound
                .checked_add(results)
                .ok_or(Error::BackendCapacity)?;
            capture_bound = capture_bound
                .checked_add(
                    results
                        .checked_mul(bound.captures)
                        .ok_or(Error::BackendCapacity)?,
                )
                .ok_or(Error::BackendCapacity)?;
            match bound.class {
                RegisterClass::Direct => {
                    direct.insert(span.execution_block);
                }
                RegisterClass::Bounded => {
                    bounded.insert(span.execution_block);
                }
            }
            end += 1;
        }
        if end == first {
            return Err(Error::BackendCapacity);
        }
        let eval_first = grid.block_spans[grid.semantic_span_order[first] as usize].evaluation_span;
        let eval_last =
            grid.block_spans[grid.semantic_span_order[end - 1] as usize].evaluation_span;
        if !direct.is_empty() || !bounded.is_empty() {
            partitions.push(Partition {
                first,
                count: end - first,
                direct: direct.into_iter().collect(),
                bounded: bounded.into_iter().collect(),
                result_bound,
                capture_bound,
                eval_first,
                eval_count: eval_last - eval_first + 1,
            });
        }
        first = end;
    }
    Ok(partitions)
}

struct Pipelines {
    roots: ComputePipelineState,
    direct: ComputePipelineState,
    bounded: ComputePipelineState,
    boundary: ComputePipelineState,
    scatter: ComputePipelineState,
    progress: ComputePipelineState,
}
impl Pipelines {
    fn new(source: &str) -> Result<Self, Error> {
        Ok(Self {
            roots: pipeline_named(source, "register_roots")?,
            direct: pipeline_named(source, "register_direct_count")?,
            bounded: pipeline_named(source, "register_bounded_count")?,
            boundary: pipeline_named(source, "register_boundary")?,
            scatter: pipeline_named(source, "register_scatter")?,
            progress: pipeline_named(source, "register_progress")?,
        })
    }
}

fn encode(
    command: &CommandBufferRef,
    pipeline: &ComputePipelineState,
    label: &str,
    buffers: &[&BufferRef],
    parameters: &[u32],
    groups: usize,
) {
    let encoder = command.new_compute_command_encoder();
    encoder.set_label(label);
    encoder.set_compute_pipeline_state(pipeline);
    encoder.checked_set_buffers(command, buffers);
    if !parameters.is_empty() {
        encoder.set_bytes(
            buffers.len() as u64,
            std::mem::size_of_val(parameters) as u64,
            parameters.as_ptr().cast(),
        );
    }
    encoder.checked_dispatch_thread_groups(
        command,
        MTLSize::new(groups as u64, 1, 1),
        MTLSize::new(256, 1, 1),
    );
    encoder.end_encoding();
}

#[cfg(test)]
thread_local! {
    static PAYLOAD_READS: std::cell::Cell<[usize;4]> = const { std::cell::Cell::new([0;4]) };
}
#[cfg(test)]
pub(super) fn take_payload_reads() -> [usize; 4] {
    PAYLOAD_READS.with(|counts| counts.replace([0; 4]))
}
#[cfg(test)]
fn observe_payload_read<T>(count: usize) {
    let name = std::any::type_name::<T>();
    let index = [
        "::ResultRecord",
        "::FinalCaptureRecord",
        "::PropertyRecord",
        "u8",
    ]
    .iter()
    .position(|suffix| name.ends_with(suffix));
    if let Some(index) = index {
        PAYLOAD_READS.with(|counts| {
            let mut next = counts.get();
            next[index] += count;
            counts.set(next);
        });
    }
}

pub(super) fn read<T: Copy>(buffer: &MappedBuffer, count: usize) -> Result<Vec<T>, Error> {
    let length = count
        .checked_mul(size_of::<T>())
        .ok_or(Error::BackendFailure)?;
    if length as u64 > buffer.buffer.length() {
        return Err(Error::BackendFailure);
    }
    #[cfg(test)]
    observe_payload_read::<T>(count);
    Ok(unsafe { std::slice::from_raw_parts(buffer.buffer.contents().cast::<T>(), count).to_vec() })
}

pub(super) fn execute(
    resident: &ResidentInput,
    plan: &ExecutionPlan,
    span_limit: u32,
) -> Result<super::metal_output_pages::SavedOutput, Error> {
    super::metal_backend::launch::check_submission()?;
    super::metal_capture_predicates::validate(resident)?;
    super::metal_typed_output::validate_operators(resident)?;
    let grid = &resident.input.grid;
    if grid.blocks.is_empty() {
        return super::metal_output_pages::SavedOutput::empty(resident);
    }
    let device = Device::system_default().ok_or(Error::UnsupportedBackend)?;
    let code = metal_register_codegen::compile(resident)?;
    let constant = super::metal_constant_output::ConstantOutput::new(resident, &code)?;
    let pipelines = Pipelines::new(&code.source)?;
    let input = upload(&device, resident)?;
    let roots = MappedBuffer::zeroed::<u32>(&device, grid.roots.len())?;
    let progress = MappedBuffer::zeroed::<u32>(&device, grid.evaluation_spans.len())?;
    let partitions = partitions(resident, &code, span_limit, constant.as_ref())?;
    let queue = device.new_command_queue();
    let predicate_programs =
        super::metal_capture_predicates::PredicatePrograms::new(&device, &queue, resident, &input)?;
    let separate_completed = metal_initial_range_codegen::mixed_classes(resident, plan)?;
    let (initial, completed_seeds) = if plan
        .stages
        .iter()
        .any(|stage| stage.kind == StageKind::SeedDynamicFrontier)
    {
        metal::objc::rc::autoreleasepool(|| {
            let command = queue.new_command_buffer();
            encode(
                command,
                &pipelines.roots,
                "register_roots",
                &[&input.buffer, &roots.buffer],
                &[],
                grid.roots.len().div_ceil(256).max(1),
            );
            let pending = super::metal_dynamic_execution::encode_start(
                &device, command, resident, plan, &input, &roots,
            )?;
            commit_checked(command)?;
            wait_checked(command)?;
            let seeds = pending.finish_seeds(&device, &queue, resident, plan, &input)?;
            let completed = separate_completed.then(|| seeds.clone());
            let initial = seeds.execute(
                &device,
                &queue,
                if separate_completed {
                    WorkClass::Continuing
                } else {
                    WorkClass::All
                },
            )?;
            Ok::<_, Error>((Some(initial), completed))
        })?
    } else {
        (None, None)
    };
    let mut candidates = Vec::new();
    let mut errors = vec![None; grid.evaluation_spans.len()];
    let repeated = plan
        .stages
        .iter()
        .any(|stage| stage.kind == StageKind::CountRepeatedBlocks);
    let producer_count = partitions.len()
        + usize::from(initial.is_some())
        + usize::from(completed_seeds.is_some())
        + usize::from(repeated);
    let mut typed = (producer_count > 1).then(|| {
        super::metal_typed_output::OutputCollector::new(
            &device,
            &queue,
            resident,
            &input,
            &predicate_programs,
        )
    });
    let dynamic_started = initial.is_some();
    if (repeated || constant.is_some()) && !dynamic_started && partitions.is_empty() {
        let command = queue.new_command_buffer();
        encode(
            command,
            &pipelines.roots,
            "register_roots",
            &[&input.buffer, &roots.buffer],
            &[],
            grid.roots.len().div_ceil(256).max(1),
        );
        commit_checked(command)?;
        wait_checked(command)?;
    }
    // These disjoint work classes retain exact task ordinals. Finish active
    // frontiers before retaining completed finite output arenas.
    if let Some(initial) = initial {
        let dynamic = super::metal_dynamic_execution::finish_ranges(
            &device,
            &queue,
            resident,
            plan,
            &input,
            initial,
            &predicate_programs,
        )?;
        errors = dynamic.errors;
        if let Some(collector) = &mut typed {
            collector.push(dynamic.candidates, &errors)?;
        } else {
            candidates.push(dynamic.candidates);
        }
    }
    // Completed finite programs have no continuation or shared task admission.
    // Execute their disjoint class after continuing frontiers release their arenas.
    if let Some(seeds) = completed_seeds {
        let initial = seeds.execute(&device, &queue, WorkClass::Completed)?;
        let completed = super::metal_dynamic_execution::finish_ranges(
            &device,
            &queue,
            resident,
            plan,
            &input,
            initial,
            &predicate_programs,
        )?;
        if completed.errors.len() != errors.len() {
            return Err(Error::BackendFailure);
        }
        for (destination, error) in errors.iter_mut().zip(completed.errors) {
            if let Some(error) = error {
                if destination.is_some() {
                    return Err(Error::BackendFailure);
                }
                *destination = Some(error);
            }
        }
        if let Some(collector) = &mut typed {
            collector.push(completed.candidates, &errors)?;
        } else {
            candidates.push(completed.candidates);
        }
    }
    for (index, partition) in partitions.iter().enumerate() {
        let batch = metal::objc::rc::autoreleasepool(|| {
            let unlimited = Scan::new(&device, partition.count)?;
            let limited = !super::metal_typed_output::needed(resident)
                && resident
                    .witness_limits
                    .iter()
                    .any(|&limit| limit != u32::MAX);
            let clipped = if limited {
                Some(Scan::new(&device, partition.count)?)
            } else {
                None
            };
            let exact = match &clipped {
                Some(scan) => scan,
                None => &unlimited,
            };
            let header = MappedBuffer::zeroed::<[u32; 4]>(&device, 2)?;
            let direct = MappedBuffer::from_slice(&device, &partition.direct)?;
            let bounded = MappedBuffer::from_slice(&device, &partition.bounded)?;
            let all_ids = partition
                .direct
                .iter()
                .chain(&partition.bounded)
                .copied()
                .collect::<Vec<_>>();
            let all = MappedBuffer::from_slice(&device, &all_ids)?;
            let mut parameters = [
                u32_size(partition.first)?,
                u32_size(partition.count)?,
                0,
                0,
                partition.eval_first,
                partition.eval_count,
                // Retain the proven producer bound within the counter domain.
                // Completed counts, rather than this bound, authorize storage.
                u32_size(partition.result_bound.min(u32::MAX as usize))?,
                u32_size(partition.capture_bound.min(u32::MAX as usize))?,
                if constant.is_some() {
                    RegisterOutputClass::SharedOperators
                } else {
                    RegisterOutputClass::All
                } as u32,
            ];
            let command = queue.new_command_buffer();
            if index == 0 && !dynamic_started {
                encode(
                    command,
                    &pipelines.roots,
                    "register_roots",
                    &[&input.buffer, &roots.buffer],
                    &[],
                    grid.roots.len().div_ceil(256),
                );
            }
            for pass in 0..if limited { 2 } else { 1 } {
                parameters[3] = pass;
                for (ids, count, pipeline, label) in [
                    (
                        &direct,
                        partition.direct.len(),
                        &pipelines.direct,
                        if pass == 1 {
                            "register_direct_limit"
                        } else {
                            "register_direct_count"
                        },
                    ),
                    (
                        &bounded,
                        partition.bounded.len(),
                        &pipelines.bounded,
                        if pass == 1 {
                            "register_bounded_limit"
                        } else {
                            "register_bounded_count"
                        },
                    ),
                ] {
                    if count != 0 {
                        encode(
                            command,
                            pipeline,
                            label,
                            &[
                                &input.buffer,
                                &roots.buffer,
                                &ids.buffer,
                                if pass == 1 {
                                    &exact.values.buffer
                                } else {
                                    &unlimited.values.buffer
                                },
                                &unlimited.values.buffer,
                                &progress.buffer,
                            ],
                            &parameters,
                            count,
                        );
                    }
                }
                if pass == 1 {
                    exact.encode_labeled(command, "register_scan", "register_add_parent")?;
                } else {
                    unlimited.encode_labeled(command, "register_scan", "register_add_parent")?;
                }
            }
            encode(
                command,
                &pipelines.boundary,
                "register_boundary",
                &[exact.total(), &header.buffer],
                &parameters,
                1,
            );
            commit_checked(command)?;
            wait_checked(command)?;
            let completed = read::<[u32; 4]>(&header, 2)?;
            if completed[1] != [parameters[0], parameters[0] + parameters[1], 0, 1]
                || completed[0][0] > parameters[6]
                || completed[0][1] > parameters[7]
                || completed[0][2..] != [0, 0]
                || (completed[0][0] == 0 && completed[0][1] != 0)
            {
                return Err(Error::BackendFailure);
            }
            if completed[0][..2].contains(&u32::MAX) {
                // Saturation cannot authorize output storage or a row address.
                return Err(Error::BackendCapacity);
            }
            // Completed counts authorize exact output storage. The immutable
            // input and offsets retain the original witness and occurrence order.
            parameters[6] = completed[0][0];
            parameters[7] = completed[0][1];
            let result_buffer =
                MappedBuffer::zeroed::<ResultRecord>(&device, parameters[6] as usize)?;
            let capture_buffer =
                MappedBuffer::zeroed::<FinalCaptureRecord>(&device, parameters[7] as usize)?;
            let command = queue.new_command_buffer();
            encode(
                command,
                &pipelines.scatter,
                "register_scatter",
                &[
                    &input.buffer,
                    &roots.buffer,
                    &all.buffer,
                    &exact.values.buffer,
                    &unlimited.values.buffer,
                    &progress.buffer,
                    &result_buffer.buffer,
                    &capture_buffer.buffer,
                ],
                &parameters,
                all_ids.len(),
            );
            encode(
                command,
                &pipelines.progress,
                "register_progress",
                &[
                    &input.buffer,
                    &exact.values.buffer,
                    exact.total(),
                    &progress.buffer,
                ],
                &parameters,
                (partition.eval_count as usize).div_ceil(256),
            );
            commit_checked(command)?;
            wait_checked(command)?;
            let header = read::<[u32; 4]>(&header, 2)?;
            if header[1] != [parameters[0], parameters[0] + parameters[1], 0, 1]
                || header[0][0] > parameters[6]
                || header[0][1] > parameters[7]
                || header[0][2..] != [0, 0]
            {
                return Err(Error::BackendFailure);
            }
            Ok(super::metal_candidate_batch::CandidateBatch {
                results: result_buffer,
                captures: capture_buffer,
                result_count: header[0][0] as usize,
                capture_count: header[0][1] as usize,
            })
        })?;
        if let Some(collector) = &mut typed {
            collector.push(batch, &errors)?;
        } else {
            candidates.push(batch);
        }
    }
    if repeated {
        let output =
            super::metal_terminal_repeat::execute(&device, &queue, resident, plan, &input, &roots)?;
        if output.errors.len() != errors.len() {
            return Err(Error::BackendFailure);
        }
        for (index, error) in output.errors.into_iter().enumerate() {
            if let Some(error) = error {
                if errors[index].is_some()
                    || !plan
                        .readiness
                        .repetitions()
                        .contains_key(&grid.evaluation_spans[index].program)
                {
                    return Err(Error::BackendFailure);
                }
                errors[index] = Some(error);
            }
        }
        if let Some(collector) = &mut typed {
            collector.push(output.candidates, &errors)?;
        } else {
            candidates.push(output.candidates);
        }
    }
    for batch in &candidates {
        if batch.result_count == 0 && batch.capture_count != 0 {
            return Err(Error::BackendFailure);
        }
    }
    candidates.retain(|batch| batch.result_count != 0);
    let (arenas, errors) = if let Some(collector) = typed {
        collector.finish(errors)?
    } else if !super::metal_typed_output::needed(resident) && candidates.len() <= 1 {
        let arenas = if let Some(batch) = candidates.pop() {
            super::metal_output_pages::OutputArenas {
                rows: batch.results,
                captures: batch.captures,
                properties: MappedBuffer::zeroed::<super::metal_batch_abi::PropertyRecord>(
                    &device, 0,
                )?,
                bytes: MappedBuffer::zeroed::<u8>(&device, 0)?,
                layout: None,
                counts: [
                    u32_size(batch.result_count)?,
                    u32_size(batch.capture_count)?,
                    0,
                    0,
                ],
            }
        } else {
            super::metal_output_pages::OutputArenas::empty(&device)?
        };
        (arenas, errors)
    } else {
        super::metal_typed_output::execute(
            &device,
            &queue,
            resident,
            &input,
            candidates,
            errors,
            &predicate_programs,
        )?
    };
    if let Some(constant) = constant {
        for (evaluation, error) in grid.evaluation_spans.iter().zip(&errors) {
            if error.is_some() && constant.owns(evaluation.program as usize)? {
                return Err(Error::BackendFailure);
            }
        }
        let output = constant.execute(&device, &queue, &input, &roots, &progress, arenas)?;
        super::metal_output_pages::SavedOutput::from_semantic_columns(
            resident, device, queue, input, output, errors,
        )
    } else {
        super::metal_output_pages::SavedOutput::new(resident, device, queue, input, arenas, errors)
    }
}

/// Assemble already ordered, disjoint evaluation outputs. No semantic comparison,
/// candidate filtering, or frontier materialization occurs at this public boundary.
#[cfg(test)]
fn assemble_evaluations(
    resident: &ResidentInput,
    registers: (Vec<ResultRecord>, Vec<FinalCaptureRecord>),
    dynamic: (Vec<ResultRecord>, Vec<FinalCaptureRecord>),
) -> Result<(Vec<ResultRecord>, Vec<FinalCaptureRecord>), Error> {
    let streams = [registers, dynamic];
    assemble_evaluation_streams(resident, &streams)
}

#[cfg(test)]
fn assemble_evaluation_streams(
    resident: &ResidentInput,
    streams: &[(Vec<ResultRecord>, Vec<FinalCaptureRecord>)],
) -> Result<(Vec<ResultRecord>, Vec<FinalCaptureRecord>), Error> {
    let mut positions = vec![0usize; streams.len()];
    let mut capture_positions = vec![0u32; streams.len()];
    let mut results = Vec::new();
    let mut captures = Vec::new();
    for evaluation in &resident.input.grid.evaluation_spans {
        let end = evaluation
            .semantic_start
            .checked_add(evaluation.root_count)
            .ok_or(Error::BackendCapacity)?;
        let mut previous_task = None;
        for ((stream, position), capture_position) in streams
            .iter()
            .zip(&mut positions)
            .zip(&mut capture_positions)
        {
            let count = stream.0[*position..].partition_point(|record| record.task_ordinal < end);
            if count == 0 {
                continue;
            }
            if previous_task.is_some_and(|previous| previous >= stream.0[*position].task_ordinal)
                || stream.0[*position].task_ordinal < evaluation.semantic_start
            {
                return Err(Error::BackendFailure);
            }
            let records = &stream.0[*position..*position + count];
            let first = *capture_position;
            let last = records[count - 1];
            previous_task = Some(last.task_ordinal);
            let end = last
                .capture_start
                .checked_add(last.capture_count)
                .ok_or(Error::BackendFailure)?;
            let values = stream
                .1
                .get(first as usize..end as usize)
                .ok_or(Error::BackendFailure)?;
            let base = u32_size(captures.len())?;
            for record in records {
                if record.capture_start != *capture_position {
                    return Err(Error::BackendFailure);
                }
                *capture_position = capture_position
                    .checked_add(record.capture_count)
                    .ok_or(Error::BackendFailure)?;
                let mut record = *record;
                record.capture_start = record
                    .capture_start
                    .checked_sub(first)
                    .and_then(|offset| base.checked_add(offset))
                    .ok_or(Error::BackendFailure)?;
                results.push(record);
            }
            captures.extend_from_slice(values);
            *position += count;
        }
    }
    if streams
        .iter()
        .zip(positions)
        .zip(capture_positions)
        .any(|((stream, position), captures)| {
            stream.0.len() != position || stream.1.len() != captures as usize
        })
    {
        return Err(Error::BackendFailure);
    }
    Ok((results, captures))
}

#[cfg(feature = "metal-tests")]
#[test]
fn output_assembly_rejects_orphaned_overlapping_and_missing_capture_values() {
    let (program, file, _) = super::metal_frontier_conformance::convergence_case(2);
    let resident = ResidentInput::new(
        super::metal_grid::MetalGridInput::build(
            &[program],
            &[file.clone(), file],
            &[vec![vec![1]], vec![vec![1]]],
        )
        .unwrap(),
        None,
    )
    .unwrap();
    let first = ResultRecord {
        capture_count: 1,
        ..ResultRecord::default()
    };
    let second = ResultRecord {
        task_ordinal: 1,
        capture_count: 1,
        ..ResultRecord::default()
    };
    let cell = FinalCaptureRecord::default();
    let joined = assemble_evaluations(
        &resident,
        (vec![first], vec![cell]),
        (vec![second], vec![cell]),
    )
    .unwrap();
    assert_eq!(
        joined.0,
        vec![
            first,
            ResultRecord {
                capture_start: 1,
                ..second
            }
        ]
    );
    assert_eq!(joined.1, vec![cell, cell]);
    for (records, captures) in [
        (
            vec![ResultRecord {
                capture_start: 1,
                ..first
            }],
            vec![cell, cell],
        ),
        (vec![first, second], vec![cell, cell]),
        (vec![first], vec![]),
        (vec![first], vec![cell, cell]),
    ] {
        assert_eq!(
            assemble_evaluations(&resident, (records, captures), (vec![], vec![])),
            Err(Error::BackendFailure)
        );
    }
}
