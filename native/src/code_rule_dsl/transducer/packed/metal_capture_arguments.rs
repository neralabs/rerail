//! Typed capture arguments shared by terminal queries and inline continuations.
use super::Error;
use super::metal_backend::{MappedBuffer, pipeline_named};
use super::metal_batch_abi::{FinalCaptureRecord, ResultRecord};
use super::metal_output_dispatch::{OperatorInput, OutputParams, SOURCE};
use metal::{BufferRef, CommandBufferRef};
use std::mem::size_of;

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ArgumentLayout {
    Compact = 0,
    OutputRecords = 1,
}

#[derive(Clone, Copy)]
pub(super) struct CaptureParams {
    pub results: u32,
    pub captures: u32,
    pub step: u32,
    pub nodes: u32,
    pub pairs: u32,
    pub windows: u32,
    pub units: u32,
    pub abort_evaluation: u32,
    pub merge_width: u32,
    pub layout: ArgumentLayout,
}
impl CaptureParams {
    fn for_query(input: OutputParams, nodes: usize) -> Result<Self, Error> {
        Ok(Self {
            results: input.results,
            captures: input.captures,
            step: 0,
            nodes: u32::try_from(nodes).map_err(|_| Error::BackendCapacity)?,
            pairs: 0,
            windows: 0,
            units: 0,
            abort_evaluation: 1,
            merge_width: 0,
            layout: ArgumentLayout::OutputRecords,
        })
    }
    pub(super) fn run(
        &self,
        command: &CommandBufferRef,
        name: &str,
        buffers: &[&BufferRef],
        count: usize,
    ) -> Result<(), Error> {
        direct(command, name, buffers, self, count)
    }
}
pub(super) fn direct(
    command: &CommandBufferRef,
    name: &str,
    buffers: &[&BufferRef],
    p: &CaptureParams,
    count: usize,
) -> Result<(), Error> {
    super::metal_parallel::encode(
        command,
        &pipeline_named(SOURCE, name)?,
        name,
        buffers,
        &[
            p.results,
            p.captures,
            p.step,
            p.nodes,
            p.pairs,
            p.windows,
            p.units,
            p.abort_evaluation,
            p.merge_width,
            p.layout as u32,
        ],
        count.div_ceil(256).max(1),
        None,
    );
    Ok(())
}
#[derive(Clone, Copy)]
pub(super) struct ArgumentInput<'a> {
    pub data: &'a MappedBuffer,
    pub spans: &'a MappedBuffer,
    pub captures: &'a MappedBuffer,
    pub metadata: &'a MappedBuffer,
    pub owners: &'a MappedBuffer,
    pub specs: &'a MappedBuffer,
    pub status: &'a MappedBuffer,
    pub parameters: CaptureParams,
}
impl ArgumentInput<'_> {
    pub(super) fn validate_storage(
        &self,
        nodes: usize,
        query: bool,
        auxiliary: [(&MappedBuffer, usize, usize); 3],
    ) -> Result<(), Error> {
        let p = self.parameters;
        if p.nodes as usize != nodes || p.abort_evaluation != u32::from(query) {
            return Err(Error::BackendFailure);
        }
        let (span_stride, capture_stride, output_records) = match p.layout {
            ArgumentLayout::Compact => (size_of::<[u32; 2]>(), size_of::<[u32; 4]>(), false),
            ArgumentLayout::OutputRecords => (
                size_of::<ResultRecord>(),
                size_of::<FinalCaptureRecord>(),
                true,
            ),
        };
        if output_records && (!query || !std::ptr::eq(&*self.specs.buffer, &*self.data.buffer)) {
            return Err(Error::BackendFailure);
        }
        let mut buffers = vec![
            (self.spans, p.results as usize, span_stride),
            (self.captures, p.captures as usize, capture_stride),
            (self.metadata, p.results as usize, 16),
            (self.owners, p.captures as usize, 4),
            (self.status, 1, 4),
            auxiliary[0],
            auxiliary[1],
            auxiliary[2],
        ];
        if !output_records {
            buffers.push((self.specs, p.results as usize, 32));
        }
        for (index, (buffer, count, stride)) in buffers.iter().enumerate() {
            let bytes = count.checked_mul(*stride).ok_or(Error::BackendCapacity)?;
            if buffer.buffer.length() < bytes as u64
                || std::ptr::eq(&*buffer.buffer, &*self.data.buffer)
                || buffers[..index]
                    .iter()
                    .any(|(other, _, _)| std::ptr::eq(&*buffer.buffer, &*other.buffer))
            {
                return Err(Error::BackendFailure);
            }
        }
        Ok(())
    }

    pub(super) fn encode_validation(
        &self,
        command: &CommandBufferRef,
        query: bool,
    ) -> Result<(), Error> {
        self.run(
            command,
            if query {
                "capture_query_arguments_validate"
            } else {
                "capture_inline_arguments_validate"
            },
            &[
                &self.data.buffer,
                &self.spans.buffer,
                &self.captures.buffer,
                &self.metadata.buffer,
                &self.owners.buffer,
                &self.specs.buffer,
                &self.status.buffer,
            ],
            self.parameters.results.max(self.parameters.captures) as usize,
        )
    }

    pub(super) fn run(
        &self,
        command: &CommandBufferRef,
        name: &str,
        buffers: &[&BufferRef],
        count: usize,
    ) -> Result<(), Error> {
        self.parameters.run(command, name, buffers, count)
    }
}
/// A borrowed query view binds the exact output and immutable Program owners.
pub(super) struct QueryArguments<'a> {
    input: ArgumentInput<'a>,
}
impl<'a> QueryArguments<'a> {
    pub(super) fn new(output: &OperatorInput<'a>, nodes: usize) -> Result<Self, Error> {
        Ok(Self {
            input: ArgumentInput {
                data: output.data,
                spans: output.rows,
                captures: output.captures,
                // OutputRecords derives specifications from this immutable code owner.
                specs: output.data,
                metadata: output.metadata,
                owners: output.owners,
                status: output.status,
                parameters: CaptureParams::for_query(output.parameters, nodes)?,
            },
        })
    }
    pub(super) fn input(&self) -> ArgumentInput<'a> {
        self.input
    }
}
