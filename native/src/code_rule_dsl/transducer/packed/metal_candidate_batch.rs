//! Structural output stays on the device until shared output operators finish.
use super::Error;
use super::metal_backend::MappedBuffer;
use super::metal_batch_abi::{FinalCaptureRecord, ResultRecord};
#[cfg(test)]
use super::metal_register_execution::read;
use metal::Device;

pub(super) struct CandidateBatch {
    pub results: MappedBuffer,
    pub captures: MappedBuffer,
    pub result_count: usize,
    pub capture_count: usize,
}

impl CandidateBatch {
    pub(super) fn empty(device: &Device) -> Result<Self, Error> {
        Ok(Self {
            results: MappedBuffer::zeroed::<ResultRecord>(device, 0)?,
            captures: MappedBuffer::zeroed::<FinalCaptureRecord>(device, 0)?,
            result_count: 0,
            capture_count: 0,
        })
    }

    /// Serialization is the final boundary. No semantic stage consumes these rows again.
    #[cfg(test)]
    pub(super) fn read(self) -> Result<(Vec<ResultRecord>, Vec<FinalCaptureRecord>), Error> {
        Ok((
            read(&self.results, self.result_count)?,
            read(&self.captures, self.capture_count)?,
        ))
    }
}
