//! Exact active-key groups live for one successor batch and retain logical minima.
use super::super::metal_backend::pipeline_named;
use super::super::metal_parallel::encode;
use super::{Error, MappedBuffer};
use metal::{BufferRef, CommandBufferRef, Device};

pub(super) struct Input<'a> {
    pub columns: &'a MappedBuffer,
    pub spans: &'a MappedBuffer,
    pub digits: &'a MappedBuffer,
    pub cursors: &'a MappedBuffer,
    pub repeats: &'a MappedBuffer,
    pub captures: &'a MappedBuffer,
    pub candidates: &'a MappedBuffer,
    pub total: &'a BufferRef,
    pub meta: &'a MappedBuffer,
    pub status: &'a MappedBuffer,
}
pub(super) struct Work {
    pub heads: MappedBuffer,
    slots: MappedBuffer,
    pub capacity: usize,
    output: usize,
}
impl Work {
    pub(super) fn new(device: &Device, output: usize, keys: usize) -> Result<Self, Error> {
        if keys > output {
            return Err(Error::BackendFailure);
        }
        // At most two thirds of the slots can contain a candidate. Complete
        // equality and logical minima remain independent of the bucket count.
        let capacity = keys
            .checked_add(keys.div_ceil(2))
            .map(|n| n.max(16))
            .ok_or(Error::BackendCapacity)?;
        u32::try_from(capacity).map_err(|_| Error::BackendCapacity)?;
        Ok(Self {
            heads: MappedBuffer::zeroed::<[u32; 2]>(device, capacity)?,
            slots: MappedBuffer::zeroed::<u32>(device, output)?,
            capacity,
            output,
        })
    }
    pub(super) fn encode(
        &self,
        command: &CommandBufferRef,
        source: &str,
        parameters: &[u32],
        input: Input<'_>,
    ) -> Result<(), Error> {
        encode(
            command,
            &pipeline_named(source, "successor_active_keys")?,
            "successor_active_keys",
            &[
                &input.columns.buffer,
                &input.spans.buffer,
                &input.digits.buffer,
                &input.cursors.buffer,
                &input.repeats.buffer,
                &input.captures.buffer,
                &input.candidates.buffer,
                input.total,
                &self.heads.buffer,
                &self.slots.buffer,
                &input.status.buffer,
            ],
            parameters,
            self.output.max(1),
            None,
        );
        encode(
            command,
            &pipeline_named(source, "successor_active_owners")?,
            "successor_active_owners",
            &[
                &input.columns.buffer,
                &input.candidates.buffer,
                input.total,
                &self.heads.buffer,
                &self.slots.buffer,
                &input.meta.buffer,
                &input.status.buffer,
            ],
            parameters,
            self.output.div_ceil(256).max(1),
            None,
        );
        Ok(())
    }
}
