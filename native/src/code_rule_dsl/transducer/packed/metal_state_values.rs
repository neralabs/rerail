//! Complete active states with typed value spans, independent of frame storage identity.
use super::Error;
use super::metal_backend::MappedBuffer;
use super::metal_backend::launch::check_submission;
use super::metal_frontier_reduce::{PendingReduction, encode_state_values};
use metal::{CommandBufferRef, Device};

pub(super) const KEY_FIELDS: usize = 6;
pub(super) const CURSOR_START: usize = 6;
pub(super) const CURSOR_COUNT: usize = 7;
pub(super) const REPEAT_START: usize = 8;
pub(super) const REPEAT_COUNT: usize = 9;
pub(super) const CAPTURE_START: usize = 10;
pub(super) const CAPTURE_COUNT: usize = 11;
pub(super) const ORDINAL: usize = 12;
pub(super) const COLUMNS: usize = 13;

/// SoA: task, PC, node, sibling requirement, Program, continuation kind;
/// cursor span, repeat span, capture span, logical ordinal. A span is start/count.
/// Kinds: 0 query active, 1 query terminal, 2 ordinary active, 3 ordinary terminal,
/// 4 query named-navigation guard, 5 ordinary named-navigation guard.
/// Kinds 8 and 9 retain ordered query/ordinary operand failures. Pending exits
/// (kinds 6 and 7) require positioned control values and cannot enter this reducer.
/// "Query" here requires repetition-based admission. Other queries retain occurrences.
/// No column denotes a canonical frame identity.
pub(super) type ValueFrontier = FrontierRecords<COLUMNS>;

pub(super) const POSITIONED_COLUMNS: usize = 12;
/// Initial task, original root, and Program. PC, values, positions, and work are
/// intrinsically zero. Only generation zero can use this schema.
pub(super) const INITIAL_COLUMNS: usize = 3;
pub(super) const MAX_POSITIONED_KIND: u32 = 13;
/// Complete state records without a scalar ordinal. Successor APIs require the
/// matching ControlValues, whose full branch positions are the ordering authority.
/// Kinds 6 and 7 additionally denote query and ordinary pending GREEDY exits.
/// Kinds 8 and 9 are finished query/ordinary operand failures at their exact PC.
/// Kinds 10/11 suspend query/ordinary values, and 12/13 retain finished work failures.
/// Physical record order has no semantic meaning. Ranked consumers accept only
/// ValueFrontier, so they require an explicit ordering conversion at their boundary.
pub(super) type PositionedFrontier = FrontierRecords<POSITIONED_COLUMNS>;

/// Shared storage layout, with distinct types for ranked and positioned records.
pub(super) struct FrontierRecords<const FIELDS: usize> {
    pub columns: MappedBuffer,
    /// [states, valid captures, ceil(states/256), 1, 1]. State count is the stride.
    pub counts: MappedBuffer,
    pub capacity: usize,
    pub generation: u64,
}

/// Distinct typed arenas. Cursor/repeat cells are [position,value,0,0].
/// Captures are [node,label,start,end]. Query frames retain stable geometry order.
/// Ordinary frames with capture predicates retain emission order during execution.
/// Terminal conversion sorts successful ordinary captures before containment and publication.
/// Other ordinary producers order equal geometry by label.
/// Arenas stay live while any active state or terminal candidate refers to them.
pub(super) struct StateValues {
    pub cursors: MappedBuffer,
    pub repeats: MappedBuffer,
    pub captures: MappedBuffer,
    /// GPU-owned [valid cursor cells, valid repeat cells]. Capture count belongs
    /// to the frontier header, so there is one authority for every valid length.
    pub counts: MappedBuffer,
    pub cursor_capacity: usize,
    pub repeat_capacity: usize,
    pub capture_capacity: usize,
    pub generation: u64,
}

#[must_use = "Check reduction failures after the shared command completes"]
pub(super) struct PendingValues(PendingReduction);
impl PendingValues {
    pub(super) fn status_header(&self) -> &metal::BufferRef {
        self.0.status_header()
    }
    pub(super) fn check_completed(self) -> Result<(), Error> {
        self.0.check_completed()
    }
}

pub(super) fn validate_storage<const FIELDS: usize>(
    input: &FrontierRecords<FIELDS>,
    values: &StateValues,
    output: &[(&MappedBuffer, usize, usize)],
    generation: u64,
) -> Result<(), Error> {
    check_submission()?;
    if ![INITIAL_COLUMNS, COLUMNS, POSITIONED_COLUMNS].contains(&FIELDS)
        || (FIELDS == INITIAL_COLUMNS
            && (input.generation != 0
                || values.cursor_capacity != 0
                || values.repeat_capacity != 0
                || values.capture_capacity != 0))
    {
        return Err(Error::BackendFailure);
    }
    let mut allocations = vec![
        (&input.columns, input.capacity, FIELDS * 4),
        (&input.counts, 5, 4),
        (&values.cursors, values.cursor_capacity, 16),
        (&values.repeats, values.repeat_capacity, 16),
        (&values.captures, values.capture_capacity, 16),
        (&values.counts, 2, 4),
    ];
    allocations.extend_from_slice(output);
    for (index, (buffer, capacity, width)) in allocations.iter().enumerate() {
        u32::try_from(*capacity).map_err(|_| Error::BackendCapacity)?;
        let bytes = capacity.checked_mul(*width).ok_or(Error::BackendCapacity)?;
        if bytes as u64 > buffer.buffer.length() {
            return Err(Error::BackendFailure);
        }
        if allocations[index + 1..]
            .iter()
            .any(|(other, _, _)| std::ptr::eq(&*buffer.buffer, &*other.buffer))
        {
            return Err(Error::BackendFailure);
        }
    }
    if input.generation != generation || input.generation != values.generation {
        return Err(Error::BackendFailure);
    }
    Ok(())
}

pub(super) fn encode(
    device: &Device,
    command: &CommandBufferRef,
    input: &ValueFrontier,
    output: &ValueFrontier,
    values: &StateValues,
    bucket_bits: u32,
) -> Result<PendingValues, Error> {
    validate_storage(
        input,
        values,
        &[
            (&output.columns, output.capacity, COLUMNS * 4),
            (&output.counts, 5, 4),
        ],
        output.generation,
    )?;
    encode_state_values(device, command, input, output, values, bucket_bits).map(PendingValues)
}

/// Transient batch membership, not a replacement for complete state keys.
/// Every live state names its earliest equal input position. Ordinary occurrences
/// name themselves. These labels have meaning only for this input generation.
/// They locate equal values; they do not select a winning logical ordinal.
pub(super) struct ValueGroups {
    pub representatives: MappedBuffer,
    /// Same counted header as ValueFrontier; every input receives one label.
    pub counts: MappedBuffer,
    pub capacity: usize,
    pub generation: u64,
}

pub(super) fn encode_groups(
    device: &Device,
    command: &CommandBufferRef,
    input: &ValueFrontier,
    output: &ValueGroups,
    values: &StateValues,
    bucket_bits: u32,
) -> Result<PendingValues, Error> {
    validate_storage(
        input,
        values,
        &[
            (&output.representatives, output.capacity, 4),
            (&output.counts, 5, 4),
        ],
        output.generation,
    )?;
    super::metal_frontier_reduce::encode_value_groups(
        device,
        command,
        input,
        output,
        values,
        bucket_bits,
    )
    .map(PendingValues)
}
