//! Separate exact active-state deduplication and completed-capture containment reduction.
use super::Error;
use super::metal_backend::launch::check_submission;
use super::metal_backend::launch::{commit_checked, wait_checked};
use super::metal_backend::{MappedBuffer, available_buffer_bytes, pipeline_named};
use super::metal_parallel::{Scan, encode};
use metal::{BufferRef, CommandBufferRef, CommandQueueRef, Device};

// Task, PC, node, canonical cursor identity, canonical repeat identity, sibling
// requirement, program, and continuation kind. Every field participates in equality.
pub(super) const KEY_COLUMNS: usize = 8;
pub(super) const COLUMNS: usize = KEY_COLUMNS + 3;
pub(super) const CAPTURE_START: usize = KEY_COLUMNS;
pub(super) const CAPTURE_COUNT: usize = KEY_COLUMNS + 1;
pub(super) const ORDINAL: usize = KEY_COLUMNS + 2;
const SOURCE: &str = concat!(
    include_str!("metal_capture_geometry.metal"),
    include_str!("metal_frontier_reduce.metal"),
    include_str!("metal_frame_identity.metal"),
    include_str!("metal_state_values.metal"),
);
const STATE_ARGUMENTS: u64 = 8 * 4;
const SCAN_ARGUMENTS: u64 = 11 * 4;

#[cfg(test)]
pub(super) struct Frontier {
    pub columns: MappedBuffer,
    pub len: usize,
    pub generation: u64,
}

/// GPU-owned lengths followed by indirect dispatch dimensions: states, valid
/// capture entries, ceil(states / 256), 1, 1. Columns use the state count as stride.
pub(super) struct CountedFrontier {
    pub columns: MappedBuffer,
    pub counts: MappedBuffer,
    pub capacity: usize,
    pub generation: u64,
}

#[must_use = "Check GPU reduction failures after the shared command completes"]
pub(super) struct PendingReduction {
    command: metal::CommandBuffer,
    scratch: Scratch,
}

impl PendingReduction {
    pub(super) fn status_header(&self) -> &BufferRef {
        &self.scratch.header.buffer
    }
    /// The command owner submits and waits. This method never submits or waits.
    pub(super) fn check_completed(self) -> Result<(), Error> {
        check_submission()?;
        if self.command.status() != metal::MTLCommandBufferStatus::Completed {
            return Err(Error::BackendFailure);
        }
        self.scratch.check_status()
    }
}

pub(super) fn encode_active(
    device: &Device,
    command: &CommandBufferRef,
    input: &CountedFrontier,
    output: &CountedFrontier,
    captures: &MappedBuffer,
    capture_capacity: usize,
    bucket_bits: u32,
) -> Result<PendingReduction, Error> {
    encode_counted(
        device,
        command,
        input,
        output,
        (captures, capture_capacity),
        bucket_bits,
        ReductionKind::ActiveExact,
    )
}

pub(super) fn encode_completed(
    device: &Device,
    command: &CommandBufferRef,
    input: &CountedFrontier,
    output: &CountedFrontier,
    captures: &MappedBuffer,
    capture_capacity: usize,
    bucket_bits: u32,
) -> Result<PendingReduction, Error> {
    encode_counted(
        device,
        command,
        input,
        output,
        (captures, capture_capacity),
        bucket_bits,
        ReductionKind::CompletedContainment,
    )
}

fn encode_counted(
    device: &Device,
    command: &CommandBufferRef,
    input: &CountedFrontier,
    output: &CountedFrontier,
    (captures, capture_capacity): (&MappedBuffer, usize),
    bucket_bits: u32,
    kind: ReductionKind,
) -> Result<PendingReduction, Error> {
    check_submission()?;
    for frontier in [input, output] {
        validate_columns(&frontier.columns, frontier.capacity)?;
        if frontier.counts.buffer.length() < 5 * 4 {
            return Err(Error::BackendFailure);
        }
    }
    validate_captures(captures, capture_capacity)?;
    if input.generation != output.generation {
        return Err(Error::BackendFailure);
    }
    let buffers = [
        &*input.columns.buffer,
        &*input.counts.buffer,
        &*output.columns.buffer,
        &*output.counts.buffer,
        &*captures.buffer,
    ];
    for (index, buffer) in buffers.iter().enumerate() {
        if buffers[index + 1..]
            .iter()
            .any(|other| std::ptr::eq(*buffer, *other))
        {
            return Err(Error::BackendFailure);
        }
    }
    let scratch = Scratch::new(
        device,
        input.capacity,
        output.capacity,
        capture_capacity,
        bucket_bits,
        kind,
        [0, 0],
    )?;
    let initialize = pipeline_named(SOURCE, "frontier_initialize")?;
    encode(
        command,
        &initialize,
        "frontier_initialize",
        &[
            &input.counts.buffer,
            &scratch.parameters.buffer,
            &scratch.header.buffer,
        ],
        &[],
        1,
        None,
    );
    scratch.encode(
        command,
        &input.columns.buffer,
        &captures.buffer,
        &output.columns.buffer,
        Some(&output.counts.buffer),
        None,
    )?;
    Ok(PendingReduction {
        command: command.to_owned(),
        scratch,
    })
}

/// Frame equality shares the bounded bucket/scan machinery, but has a separate
/// typed input and value comparison. Its private columns are keys plus a value
/// span; frame cells never enter capture geometry or containment operations.
pub(super) fn encode_frame_ids(
    device: &Device,
    command: &CommandBufferRef,
    input: &super::metal_frame_identity::CountedFrames,
    output: &super::metal_frame_identity::FrameIdentities,
    bucket_bits: u32,
) -> Result<(PendingReduction, MappedBuffer), Error> {
    let scratch = Scratch::new(
        device,
        input.capacity,
        output.capacity,
        input.value_capacity,
        bucket_bits,
        ReductionKind::FrameIdentity,
        [0, 0],
    )?;
    let keys = MappedBuffer::zeroed::<u32>(
        device,
        input
            .capacity
            .checked_mul(COLUMNS)
            .ok_or(Error::BackendCapacity)?,
    )?;
    for (name, buffers, groups, indirect) in [
        (
            "frontier_initialize",
            vec![
                &*input.counts.buffer,
                &*scratch.parameters.buffer,
                &*scratch.header.buffer,
            ],
            1,
            None,
        ),
        (
            "frontier_frame_prepare",
            vec![
                &*input.descriptors.buffer,
                &*keys.buffer,
                &*scratch.dominated.buffer,
                &*scratch.header.buffer,
                &*scratch.parameters.buffer,
            ],
            0,
            Some((&*scratch.header.buffer, STATE_ARGUMENTS)),
        ),
    ] {
        let pipeline = pipeline_named(SOURCE, name)?;
        encode(command, &pipeline, name, &buffers, &[], groups, indirect);
    }
    scratch.encode(
        command,
        &keys.buffer,
        &input.values.buffer,
        &output.ids.buffer,
        Some(&output.counts.buffer),
        None,
    )?;
    Ok((
        PendingReduction {
            command: command.to_owned(),
            scratch,
        },
        keys,
    ))
}

/// Complete-value reduction shares scans and pair bounds, while its typed input
/// and kernels preserve the distinct value-span schema.
pub(super) fn encode_state_values(
    device: &Device,
    command: &CommandBufferRef,
    input: &super::metal_state_values::ValueFrontier,
    output: &super::metal_state_values::ValueFrontier,
    values: &super::metal_state_values::StateValues,
    bucket_bits: u32,
) -> Result<PendingReduction, Error> {
    encode_values(
        device,
        command,
        input,
        ValueOutput::Reduced(output),
        values,
        bucket_bits,
    )
}

pub(super) fn encode_value_groups(
    device: &Device,
    command: &CommandBufferRef,
    input: &super::metal_state_values::ValueFrontier,
    output: &super::metal_state_values::ValueGroups,
    values: &super::metal_state_values::StateValues,
    bucket_bits: u32,
) -> Result<PendingReduction, Error> {
    encode_values(
        device,
        command,
        input,
        ValueOutput::Groups(output),
        values,
        bucket_bits,
    )
}

enum ValueOutput<'a> {
    Reduced(&'a super::metal_state_values::ValueFrontier),
    Groups(&'a super::metal_state_values::ValueGroups),
}

fn encode_values(
    device: &Device,
    command: &CommandBufferRef,
    input: &super::metal_state_values::ValueFrontier,
    output: ValueOutput<'_>,
    values: &super::metal_state_values::StateValues,
    bucket_bits: u32,
) -> Result<PendingReduction, Error> {
    let (kind, data, counts, capacity) = match output {
        ValueOutput::Reduced(out) => (
            ReductionKind::StateValues,
            &*out.columns.buffer,
            &*out.counts.buffer,
            out.capacity,
        ),
        ValueOutput::Groups(out) => (
            ReductionKind::StateGroups,
            &*out.representatives.buffer,
            &*out.counts.buffer,
            out.capacity,
        ),
    };
    let scratch = Scratch::new(
        device,
        input.capacity,
        capacity,
        values.capture_capacity,
        bucket_bits,
        kind,
        [values.cursor_capacity, values.repeat_capacity],
    )?;
    let pipeline = pipeline_named(SOURCE, "state_value_initialize")?;
    encode(
        command,
        &pipeline,
        "state_value_initialize",
        &[
            &input.counts.buffer,
            &values.counts.buffer,
            &scratch.parameters.buffer,
            &scratch.header.buffer,
        ],
        &[],
        1,
        None,
    );
    scratch.encode(
        command,
        &input.columns.buffer,
        &values.captures.buffer,
        data,
        Some(counts),
        Some(values),
    )?;
    Ok(PendingReduction {
        command: command.to_owned(),
        scratch,
    })
}

/// A checked reduction retains only its status for a later GPU consumer.
/// Pair storage follows completed GPU counts and never reserves the hard upper bound.
pub(super) struct CompletedReduction {
    pub header: MappedBuffer,
}

pub(super) fn execute_value_groups(
    device: &Device,
    queue: &CommandQueueRef,
    input: &super::metal_state_values::ValueFrontier,
    output: &super::metal_state_values::ValueGroups,
    values: &super::metal_state_values::StateValues,
    bucket_bits: u32,
) -> Result<CompletedReduction, Error> {
    super::metal_state_values::validate_storage(
        input,
        values,
        &[
            (&output.representatives, output.capacity, 4),
            (&output.counts, 5, 4),
        ],
        output.generation,
    )?;
    execute_staged(
        device,
        queue,
        input.capacity,
        output.capacity,
        values.capture_capacity,
        bucket_bits,
        ReductionKind::StateGroups,
        &input.columns.buffer,
        &input.counts.buffer,
        &output.representatives.buffer,
        &output.counts.buffer,
        &values.captures.buffer,
        Some(values),
    )
}

pub(super) fn execute_completed(
    device: &Device,
    queue: &CommandQueueRef,
    input: &CountedFrontier,
    output: &CountedFrontier,
    captures: &MappedBuffer,
    capture_capacity: usize,
    bucket_bits: u32,
) -> Result<CompletedReduction, Error> {
    check_submission()?;
    for frontier in [input, output] {
        validate_columns(&frontier.columns, frontier.capacity)?;
        if frontier.counts.buffer.length() < 20 {
            return Err(Error::BackendFailure);
        }
    }
    validate_captures(captures, capture_capacity)?;
    if input.generation != output.generation {
        return Err(Error::BackendFailure);
    }
    let buffers = [
        &*input.columns.buffer,
        &*input.counts.buffer,
        &*output.columns.buffer,
        &*output.counts.buffer,
        &*captures.buffer,
    ];
    for (index, buffer) in buffers.iter().enumerate() {
        if buffers[index + 1..]
            .iter()
            .any(|other| std::ptr::eq(*buffer, *other))
        {
            return Err(Error::BackendFailure);
        }
    }
    execute_staged(
        device,
        queue,
        input.capacity,
        output.capacity,
        capture_capacity,
        bucket_bits,
        ReductionKind::CompletedContainment,
        &input.columns.buffer,
        &input.counts.buffer,
        &output.columns.buffer,
        &output.counts.buffer,
        &captures.buffer,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn execute_staged(
    device: &Device,
    queue: &CommandQueueRef,
    capacity: usize,
    output_capacity: usize,
    capture_capacity: usize,
    bucket_bits: u32,
    kind: ReductionKind,
    input: &BufferRef,
    counts: &BufferRef,
    output: &BufferRef,
    output_counts: &BufferRef,
    captures: &BufferRef,
    values: Option<&super::metal_state_values::StateValues>,
) -> Result<CompletedReduction, Error> {
    let mut scratch = Scratch::with_pair_storage(
        device,
        capacity,
        output_capacity,
        capture_capacity,
        bucket_bits,
        kind,
        values.map_or([0, 0], |v| [v.cursor_capacity, v.repeat_capacity]),
        true,
    )?;
    metal::objc::rc::autoreleasepool(|| -> Result<(), Error> {
        let command = queue.new_command_buffer();
        let name = if values.is_some() {
            "state_value_initialize"
        } else {
            "frontier_initialize"
        };
        let mut buffers = vec![counts];
        if let Some(values) = values {
            buffers.push(&values.counts.buffer);
        }
        buffers.extend([&*scratch.parameters.buffer, &*scratch.header.buffer]);
        encode(
            command,
            &pipeline_named(SOURCE, name)?,
            name,
            &buffers,
            &[],
            1,
            None,
        );
        scratch.encode_stage(
            command,
            input,
            captures,
            output,
            Some(output_counts),
            values,
            ReductionStage::Count,
        )?;
        commit_checked(command)?;
        wait_checked(command)?;
        scratch.check_status()
    })?;
    let pairs = unsafe {
        scratch
            .comparison_scan
            .total()
            .contents()
            .cast::<u32>()
            .read()
    };
    let parameters = scratch.parameters.buffer.contents().cast::<u32>();
    let limit = unsafe { parameters.add(2).read() };
    if pairs > limit {
        return Err(Error::BackendCapacity);
    }
    scratch.rejected = MappedBuffer::zeroed::<u32>(device, pairs as usize)?;
    // This is allocation metadata, not a host decision about any candidate.
    unsafe {
        parameters.add(2).write(pairs);
    }
    metal::objc::rc::autoreleasepool(|| -> Result<(), Error> {
        let command = queue.new_command_buffer();
        scratch.encode_stage(
            command,
            input,
            captures,
            output,
            Some(output_counts),
            values,
            ReductionStage::Finish,
        )?;
        commit_checked(command)?;
        wait_checked(command)?;
        scratch.check_status()
    })?;
    Ok(CompletedReduction {
        header: scratch.header,
    })
}

fn checked(value: usize) -> Result<u32, Error> {
    u32::try_from(value).map_err(|_| Error::BackendCapacity)
}

fn validate_columns(columns: &MappedBuffer, capacity: usize) -> Result<(), Error> {
    checked(capacity)?;
    let bytes = capacity
        .checked_mul(COLUMNS * 4)
        .ok_or(Error::BackendCapacity)?;
    if bytes as u64 > columns.buffer.length() {
        return Err(Error::BackendFailure);
    }
    Ok(())
}

fn validate_captures(captures: &MappedBuffer, capacity: usize) -> Result<(), Error> {
    checked(capacity)?;
    let bytes = capacity.checked_mul(16).ok_or(Error::BackendCapacity)?;
    if bytes as u64 > captures.buffer.length() {
        return Err(Error::BackendFailure);
    }
    Ok(())
}

#[repr(u32)]
#[derive(Clone, Copy, PartialEq)]
enum ReductionKind {
    CompletedContainment,
    ActiveExact,
    FrameIdentity,
    StateValues,
    StateGroups,
}

#[derive(Clone, Copy, PartialEq)]
enum ReductionStage {
    All,
    Count,
    Finish,
}

struct Scratch {
    bucket_scan: Scan,
    comparison_scan: Scan,
    survivor_scan: Option<Scan>,
    slots: MappedBuffer,
    items: MappedBuffer,
    dominated: MappedBuffer,
    rejected: MappedBuffer,
    header: MappedBuffer,
    parameters: MappedBuffer,
    count: usize,
    kind: ReductionKind,
}

impl Scratch {
    fn new(
        device: &Device,
        count: usize,
        output_capacity: usize,
        capture_capacity: usize,
        bucket_bits: u32,
        kind: ReductionKind,
        frame_capacities: [usize; 2],
    ) -> Result<Self, Error> {
        Self::with_pair_storage(
            device,
            count,
            output_capacity,
            capture_capacity,
            bucket_bits,
            kind,
            frame_capacities,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn with_pair_storage(
        device: &Device,
        count: usize,
        output_capacity: usize,
        capture_capacity: usize,
        bucket_bits: u32,
        kind: ReductionKind,
        frame_capacities: [usize; 2],
        deferred: bool,
    ) -> Result<Self, Error> {
        if bucket_bits > 20 {
            return Err(Error::BackendFailure);
        }
        let buckets = 1usize << bucket_bits;
        let bucket_scan = Scan::new(device, buckets)?;
        let comparison_scan = Scan::new(device, count)?;
        let survivor_scan = if matches!(
            kind,
            ReductionKind::FrameIdentity | ReductionKind::StateGroups
        ) {
            None
        } else {
            Some(Scan::new(device, count)?)
        };
        let slots = MappedBuffer::zeroed::<[u32; 2]>(device, count)?;
        let items = MappedBuffer::zeroed::<u32>(device, count)?;
        let dominated = MappedBuffer::zeroed::<u32>(device, count)?;
        let levels = comparison_scan.levels();
        let header = MappedBuffer::zeroed::<u32>(device, 11 + levels * 3)?;
        let pair_capacity = count
            .checked_mul(count)
            .ok_or(Error::BackendCapacity)?
            .min(16 * 1024 * 1024)
            .min(available_buffer_bytes(device).saturating_sub(1024 * 1024) / 4);
        if count != 0 && pair_capacity == 0 {
            return Err(Error::BackendCapacity);
        }
        let rejected =
            MappedBuffer::zeroed::<u32>(device, if deferred { 0 } else { pair_capacity })?;
        let parameters = MappedBuffer::from_slice(
            device,
            &[
                checked(count)?,
                checked(buckets)?,
                checked(pair_capacity)?,
                checked(capture_capacity)?,
                kind as u32,
                checked(count)?,
                checked(capture_capacity)?,
                checked(output_capacity)?,
                checked(levels)?,
                checked(frame_capacities[0])?,
                checked(frame_capacities[1])?,
            ],
        )?;
        Ok(Self {
            bucket_scan,
            comparison_scan,
            survivor_scan,
            slots,
            items,
            dominated,
            rejected,
            header,
            parameters,
            count,
            kind,
        })
    }

    fn check_status(&self) -> Result<(), Error> {
        let status = unsafe { self.header.buffer.contents().cast::<u32>().add(6).read() };
        if status & 2 != 0 {
            return Err(Error::BackendFailure);
        }
        if status != 0 {
            return Err(Error::BackendCapacity);
        }
        Ok(())
    }

    fn encode(
        &self,
        command: &CommandBufferRef,
        input: &BufferRef,
        captures: &BufferRef,
        output: &BufferRef,
        output_counts: Option<&BufferRef>,
        values: Option<&super::metal_state_values::StateValues>,
    ) -> Result<(), Error> {
        self.encode_stage(
            command,
            input,
            captures,
            output,
            output_counts,
            values,
            ReductionStage::All,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_stage(
        &self,
        command: &CommandBufferRef,
        input: &BufferRef,
        captures: &BufferRef,
        output: &BufferRef,
        output_counts: Option<&BufferRef>,
        values: Option<&super::metal_state_values::StateValues>,
        stage: ReductionStage,
    ) -> Result<(), Error> {
        debug_assert_eq!(
            matches!(
                self.kind,
                ReductionKind::StateValues | ReductionKind::StateGroups
            ),
            values.is_some()
        );
        let counted = output_counts.is_some();
        let dispatch = |name, buffers: &[&BufferRef], groups, indirect| -> Result<(), Error> {
            let pipeline = pipeline_named(SOURCE, name)?;
            let mut buffers = buffers.to_vec();
            buffers.push(&self.parameters.buffer);
            encode(command, &pipeline, name, &buffers, &[], groups, indirect);
            Ok(())
        };
        let state_indirect = counted.then_some((&*self.header.buffer, STATE_ARGUMENTS));
        let groups = if counted { 0 } else { self.count.div_ceil(256) };
        let scan = |scan: &Scan| {
            if counted {
                scan.encode_counted(
                    command,
                    &self.parameters.buffer,
                    &self.header.buffer,
                    SCAN_ARGUMENTS,
                )
            } else {
                scan.encode(command)
            }
        };
        if stage != ReductionStage::Finish {
            if self.kind == ReductionKind::StateGroups {
                dispatch(
                    "state_value_group_seed",
                    &[&self.dominated.buffer],
                    groups,
                    state_indirect,
                )?;
            }
            let mut bucket_buffers =
                vec![input, &*self.bucket_scan.values.buffer, &*self.slots.buffer];
            if let Some(values) = values {
                bucket_buffers.extend([
                    &*values.cursors.buffer,
                    &*values.repeats.buffer,
                    captures,
                    &*values.counts.buffer,
                    &*self.header.buffer,
                ]);
            }
            dispatch(
                if values.is_some() {
                    "state_value_bucket_count"
                } else {
                    "frontier_bucket_count"
                },
                &bucket_buffers,
                groups,
                state_indirect,
            )?;
            self.bucket_scan.encode(command)?;
            dispatch(
                "frontier_bucket_scatter",
                &[
                    &self.slots.buffer,
                    &self.bucket_scan.values.buffer,
                    &self.items.buffer,
                ],
                groups,
                state_indirect,
            )?;
            let mut count_buffers = vec![
                input,
                &*self.slots.buffer,
                &*self.bucket_scan.values.buffer,
                self.bucket_scan.total(),
                &*self.comparison_scan.values.buffer,
                &*self.header.buffer,
            ];
            if let Some(values) = values {
                count_buffers.push(&values.counts.buffer);
            }
            dispatch(
                if values.is_some() {
                    "state_value_comparison_count"
                } else {
                    "frontier_comparison_count"
                },
                &count_buffers,
                groups,
                state_indirect,
            )?;
            scan(&self.comparison_scan)?;
            dispatch(
                "frontier_comparison_boundary",
                &[self.comparison_scan.total(), &self.header.buffer],
                1,
                None,
            )?;
        }
        if stage == ReductionStage::Count {
            return Ok(());
        }
        let mut compare_buffers = vec![
            input,
            captures,
            &*self.slots.buffer,
            &*self.bucket_scan.values.buffer,
            &*self.items.buffer,
            &*self.comparison_scan.values.buffer,
            self.comparison_scan.total(),
            &*self.rejected.buffer,
            &*self.header.buffer,
        ];
        if let Some(values) = values {
            compare_buffers.extend([&*values.cursors.buffer, &*values.repeats.buffer]);
        }
        let compare = match self.kind {
            ReductionKind::FrameIdentity => "frontier_compare_frames",
            ReductionKind::StateValues | ReductionKind::StateGroups => "state_value_compare",
            _ => "frontier_compare_captures",
        };
        dispatch(compare, &compare_buffers, 0, Some((&self.header.buffer, 0)))?;
        if self.kind == ReductionKind::StateGroups {
            dispatch(
                "state_value_group_pairs",
                &[
                    input,
                    &self.slots.buffer,
                    &self.bucket_scan.values.buffer,
                    &self.items.buffer,
                    &self.comparison_scan.values.buffer,
                    self.comparison_scan.total(),
                    &self.rejected.buffer,
                    &self.dominated.buffer,
                ],
                0,
                Some((&self.header.buffer, 12)),
            )?;
            return dispatch(
                "state_value_group_publish",
                &[
                    &self.dominated.buffer,
                    output,
                    output_counts.ok_or(Error::BackendFailure)?,
                    &self.header.buffer,
                ],
                0,
                Some((&self.header.buffer, SCAN_ARGUMENTS)),
            );
        }
        if self.kind == ReductionKind::FrameIdentity {
            dispatch(
                "frontier_frame_pairs",
                &[
                    input,
                    &self.slots.buffer,
                    &self.bucket_scan.values.buffer,
                    &self.items.buffer,
                    &self.comparison_scan.values.buffer,
                    self.comparison_scan.total(),
                    &self.rejected.buffer,
                    &self.dominated.buffer,
                ],
                0,
                Some((&self.header.buffer, 12)),
            )?;
            return dispatch(
                "frontier_frame_publish",
                &[
                    &self.dominated.buffer,
                    output,
                    output_counts.ok_or(Error::BackendFailure)?,
                    &self.header.buffer,
                ],
                0,
                Some((&self.header.buffer, SCAN_ARGUMENTS)),
            );
        }
        let survivor_scan = self.survivor_scan.as_ref().ok_or(Error::BackendFailure)?;
        dispatch(
            if values.is_some() {
                "state_value_reduce_pairs"
            } else {
                "frontier_reduce_pairs"
            },
            &[
                input,
                &self.slots.buffer,
                &self.bucket_scan.values.buffer,
                &self.items.buffer,
                &self.comparison_scan.values.buffer,
                self.comparison_scan.total(),
                &self.rejected.buffer,
                &self.dominated.buffer,
            ],
            0,
            Some((&self.header.buffer, 12)),
        )?;
        dispatch(
            "frontier_survivor_count",
            &[
                &self.dominated.buffer,
                &survivor_scan.values.buffer,
                &self.header.buffer,
            ],
            groups,
            state_indirect,
        )?;
        scan(survivor_scan)?;
        if let Some(counts) = output_counts {
            // At least one group publishes the empty result's valid count header.
            dispatch(
                if values.is_some() {
                    "state_value_scatter"
                } else {
                    "frontier_counted_scatter"
                },
                &[
                    input,
                    &self.dominated.buffer,
                    &survivor_scan.values.buffer,
                    survivor_scan.total(),
                    output,
                    counts,
                    &self.header.buffer,
                ],
                0,
                Some((&self.header.buffer, SCAN_ARGUMENTS)),
            )?;
        } else {
            dispatch(
                "frontier_survivor_scatter",
                &[
                    input,
                    &self.dominated.buffer,
                    &survivor_scan.values.buffer,
                    survivor_scan.total(),
                    output,
                ],
                groups,
                None,
            )?;
        }
        Ok(())
    }
}

// Standalone wrappers are independent component tests. Production commands own
// submission and completion through encode_active and PendingReduction.
#[cfg(test)]
pub(super) fn reduce_completed(
    device: &Device,
    frontier: &Frontier,
    captures: &MappedBuffer,
    capture_count: usize,
    bucket_bits: u32,
) -> Result<Frontier, Error> {
    metal::objc::rc::autoreleasepool(|| {
        reduce_impl(
            device,
            frontier,
            captures,
            capture_count,
            bucket_bits,
            ReductionKind::CompletedContainment,
        )
    })
}

#[cfg(test)]
pub(super) fn reduce_active(
    device: &Device,
    frontier: &Frontier,
    captures: &MappedBuffer,
    capture_count: usize,
    bucket_bits: u32,
) -> Result<Frontier, Error> {
    metal::objc::rc::autoreleasepool(|| {
        reduce_impl(
            device,
            frontier,
            captures,
            capture_count,
            bucket_bits,
            ReductionKind::ActiveExact,
        )
    })
}

#[cfg(test)]
fn reduce_impl(
    device: &Device,
    frontier: &Frontier,
    captures: &MappedBuffer,
    capture_count: usize,
    bucket_bits: u32,
    kind: ReductionKind,
) -> Result<Frontier, Error> {
    check_submission()?;
    let n = frontier.len;
    validate_columns(&frontier.columns, n)?;
    validate_captures(captures, capture_count)?;
    if bucket_bits > 20 {
        return Err(Error::BackendFailure);
    }
    let columns = MappedBuffer::zeroed::<u32>(
        device,
        n.checked_mul(COLUMNS).ok_or(Error::BackendCapacity)?,
    )?;
    if n == 0 {
        return Ok(Frontier {
            columns,
            len: 0,
            generation: frontier.generation,
        });
    }
    let scratch = Scratch::new(device, n, n, capture_count, bucket_bits, kind, [0, 0])?;
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    scratch.encode(
        command,
        &frontier.columns.buffer,
        &captures.buffer,
        &columns.buffer,
        None,
        None,
    )?;
    commit_checked(command)?;
    wait_checked(command)?;
    scratch.check_status()?;
    let total = scratch
        .survivor_scan
        .as_ref()
        .ok_or(Error::BackendFailure)?
        .total();
    let len = unsafe { total.contents().cast::<u32>().read() } as usize;
    if len > n {
        return Err(Error::BackendFailure);
    }
    Ok(Frontier {
        columns,
        len,
        generation: frontier.generation,
    })
}
