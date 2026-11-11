//! MIME properties retain source snapshots and use one shared byte domain.
use super::Error;
use super::metal_backend::MappedBuffer;
use super::metal_output_dispatch::OperatorInput;
use super::metal_parallel::Scan;
use metal::{CommandBufferRef, Device};

pub(super) struct MimeOutput {
    pub snapshots: MappedBuffer,
    pub offsets: Scan,
    work: Scan,
    slashes: MappedBuffer,
    arguments: MappedBuffer,
    results: usize,
    operators: usize,
    slots: usize,
}
impl MimeOutput {
    pub(super) fn new(
        device: &Device,
        results: usize,
        operators: usize,
        enabled: bool,
    ) -> Result<Self, Error> {
        let slots = if enabled {
            results
                .checked_mul(operators)
                .ok_or(Error::BackendCapacity)?
        } else {
            0
        };
        Ok(Self {
            snapshots: MappedBuffer::zeroed::<[u32; 4]>(device, slots)?,
            offsets: Scan::new(device, slots)?,
            work: Scan::new(device, if enabled { results } else { 0 })?,
            slashes: MappedBuffer::zeroed::<u32>(device, slots)?,
            arguments: MappedBuffer::zeroed::<u32>(device, 3)?,
            results,
            operators,
            slots,
        })
    }
    pub(super) fn encode_step(
        &self,
        command: &CommandBufferRef,
        input: &OperatorInput<'_>,
        targets: &MappedBuffer,
    ) -> Result<(), Error> {
        if self.slots == 0
            || input.parameters.results as usize != self.results
            || input.parameters.max_operators as usize != self.operators
            || input.parameters.dynamic_slots as usize != self.slots
        {
            return Err(Error::BackendFailure);
        }
        input.run(
            command,
            "output_mime_snapshot",
            &[
                &input.data.buffer,
                &input.rows.buffer,
                &input.captures.buffer,
                &input.metadata.buffer,
                &targets.buffer,
                &self.snapshots.buffer,
                &self.work.values.buffer,
                &input.status.buffer,
                &input.owners.buffer,
            ],
            self.results,
        )?;
        self.work.encode(command)?;
        input.run(
            command,
            "output_mime_arguments",
            &[
                self.work.total(),
                &self.arguments.buffer,
                &input.status.buffer,
            ],
            1,
        )?;
        input.indirect(
            command,
            "output_mime_slashes",
            &[
                &input.data.buffer,
                &self.snapshots.buffer,
                &self.work.values.buffer,
                self.work.total(),
                &self.slashes.buffer,
                &input.status.buffer,
            ],
            &self.arguments,
        )?;
        input.run(
            command,
            "output_mime_finish",
            &[
                &self.snapshots.buffer,
                &self.slashes.buffer,
                &input.status.buffer,
            ],
            self.results,
        )
    }
    pub(super) fn encode_sizes(
        &self,
        command: &CommandBufferRef,
        input: &OperatorInput<'_>,
    ) -> Result<(), Error> {
        if input.parameters.results as usize != self.results
            || input.parameters.max_operators as usize != self.operators
            || input.parameters.dynamic_slots as usize != self.slots
        {
            return Err(Error::BackendFailure);
        }
        if self.slots == 0 {
            return Ok(());
        }
        input.run(
            command,
            "output_dynamic_sizes",
            &[
                &self.snapshots.buffer,
                &input.metadata.buffer,
                &self.offsets.values.buffer,
                &input.status.buffer,
                &input.data.buffer,
            ],
            self.slots,
        )?;
        self.offsets.encode(command)
    }
    pub(super) fn check_completed(&self) -> Result<(), Error> {
        let total = unsafe { self.offsets.total().contents().cast::<[u32; 4]>().read() };
        if total.contains(&u32::MAX) {
            return Err(Error::BackendCapacity);
        }
        Ok(())
    }
    pub(super) fn slots(&self) -> usize {
        self.slots
    }
}
