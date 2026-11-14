//! Shared target selection and maximal adjacent capture suffixes.
use super::Error;
use super::metal_backend::MappedBuffer;
use super::metal_output_dispatch::OperatorInput;
use super::metal_parallel::Scan;
use metal::{CommandBufferRef, Device};

pub(super) struct CaptureSelection {
    pub targets: MappedBuffer,
    selected: Scan,
    indices: MappedBuffer,
    results: usize,
    captures: usize,
}
impl CaptureSelection {
    pub(super) fn new(device: &Device, results: usize, captures: usize) -> Result<Self, Error> {
        Ok(Self {
            targets: MappedBuffer::zeroed::<[u32; 2]>(device, results)?,
            selected: Scan::new(device, captures)?,
            indices: MappedBuffer::zeroed::<u32>(device, captures)?,
            results,
            captures,
        })
    }
    pub(super) fn encode_targets(
        &self,
        command: &CommandBufferRef,
        input: &OperatorInput<'_>,
    ) -> Result<(), Error> {
        if input.parameters.results as usize != self.results
            || input.parameters.captures as usize != self.captures
        {
            return Err(Error::BackendFailure);
        }
        input.run(
            command,
            "output_selection_initialize",
            &[&self.targets.buffer],
            self.results,
        )?;
        input.run(
            command,
            "output_selection_counts",
            &[
                &input.data.buffer,
                &input.captures.buffer,
                &input.metadata.buffer,
                &input.owners.buffer,
                &self.targets.buffer,
                &self.selected.values.buffer,
                &input.status.buffer,
            ],
            self.captures,
        )
    }
    pub(super) fn encode_adjacency(
        &self,
        command: &CommandBufferRef,
        input: &OperatorInput<'_>,
    ) -> Result<(), Error> {
        self.selected.encode(command)?;
        input.run(
            command,
            "output_selection_indices",
            &[
                &self.selected.values.buffer,
                self.selected.total(),
                &self.indices.buffer,
                &input.status.buffer,
            ],
            self.captures,
        )?;
        input.run(
            command,
            "output_selection_breaks",
            &[
                &input.data.buffer,
                &input.captures.buffer,
                &input.metadata.buffer,
                &input.owners.buffer,
                &self.targets.buffer,
                &self.selected.values.buffer,
                self.selected.total(),
                &self.indices.buffer,
                &input.status.buffer,
            ],
            self.captures,
        )?;
        input.run(
            command,
            "output_selection_apply",
            &[
                &input.captures.buffer,
                &input.owners.buffer,
                &self.targets.buffer,
                &self.selected.values.buffer,
                self.selected.total(),
                &input.status.buffer,
            ],
            self.captures,
        )
    }
}
