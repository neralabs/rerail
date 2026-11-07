//! Canonical value identities for cursor and repeat frames in one active generation.
use super::Error;
use super::metal_backend::MappedBuffer;
use super::metal_backend::launch::check_submission;
use super::metal_frontier_reduce::{PendingReduction, encode_frame_ids};
use metal::{CommandBufferRef, Device};

/// A descriptor is [task, kind, value_start, value_count]. Kind 0 denotes sparse
/// cursors; kind 1 denotes an ordered repeat stack. Values are [position,value,0,0].
/// Cursor positions are strictly increasing slots. Repeat positions are 0..count.
pub(super) struct CountedFrames {
    pub descriptors: MappedBuffer,
    pub values: MappedBuffer,
    /// [frames, valid values, ceil(frames/256), 1, 1], written by the GPU producer.
    pub counts: MappedBuffer,
    pub capacity: usize,
    pub value_capacity: usize,
    pub generation: u64,
}

pub(super) struct FrameIdentities {
    /// Zero means empty. Other handles address descriptor (handle - 1).
    pub ids: MappedBuffer,
    pub counts: MappedBuffer,
    pub capacity: usize,
    pub generation: u64,
}

#[must_use = "Check canonicalization failures after the shared command completes"]
pub(super) struct PendingFrames {
    pending: PendingReduction,
    // Keep the prepared key/span arena alive through command completion.
    _keys: MappedBuffer,
}

impl PendingFrames {
    pub(super) fn check_completed(self) -> Result<(), Error> {
        self.pending.check_completed()
    }
}

pub(super) fn encode(
    device: &Device,
    command: &CommandBufferRef,
    input: &CountedFrames,
    output: &FrameIdentities,
    bucket_bits: u32,
) -> Result<PendingFrames, Error> {
    check_submission()?;
    for (buffer, capacity, width) in [
        (&input.descriptors, input.capacity, 16usize),
        (&input.values, input.value_capacity, 16),
        (&input.counts, 5, 4),
        (&output.ids, output.capacity, 4),
        (&output.counts, 5, 4),
    ] {
        u32::try_from(capacity).map_err(|_| Error::BackendCapacity)?;
        let bytes = capacity.checked_mul(width).ok_or(Error::BackendCapacity)?;
        if bytes as u64 > buffer.buffer.length() {
            return Err(Error::BackendFailure);
        }
    }
    if input.generation != output.generation {
        return Err(Error::BackendFailure);
    }
    let buffers = [
        &*input.descriptors.buffer,
        &*input.values.buffer,
        &*input.counts.buffer,
        &*output.ids.buffer,
        &*output.counts.buffer,
    ];
    for (index, buffer) in buffers.iter().enumerate() {
        if buffers[index + 1..]
            .iter()
            .any(|other| std::ptr::eq(*buffer, *other))
        {
            return Err(Error::BackendFailure);
        }
    }
    let (pending, keys) = encode_frame_ids(device, command, input, output, bucket_bits)?;
    Ok(PendingFrames {
        pending,
        _keys: keys,
    })
}
