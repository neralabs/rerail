//! Durable GPU output and bounded, checked publication without semantic replay.
use super::Error;
use super::metal_backend::launch::{commit_checked, wait_checked};
use super::metal_backend::{MappedBuffer, pipeline_named};
use super::metal_batch_abi::{FinalCaptureRecord, PropertyRecord, ResultRecord};
use super::metal_grid_plan::{OutputBoundary, ResidentInput};
use super::metal_register_execution::read;
use super::packed_batch::{GridFragment, GridPublication, PackedExecution};
use metal::{CommandQueue, Device};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

const SOURCE: &str = concat!(
    include_str!("metal_resident.metal"),
    include_str!("metal_column_view.metal"),
    include_str!("metal_parallel.metal"),
    include_str!("metal_output_pages.metal"),
);
#[cfg(test)]
#[path = "metal_typed_checkpoint_tests.rs"]
mod checkpoint_tests;
#[path = "metal_saved_columns.rs"]
mod columns;
#[path = "metal_typed_merge.rs"]
mod merge;
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

/// Completed typed relations retain their checked physical format between producers.
pub(super) struct TypedCheckpoint {
    arenas: columns::SavedArenas,
}

impl TypedCheckpoint {
    pub(super) fn new(
        device: &Device,
        queue: &metal::CommandQueueRef,
        arenas: OutputArenas,
    ) -> Result<Self, Error> {
        arenas.validate()?;
        if arenas.counts[0] != 0 && arenas.layout.is_none() {
            return Err(Error::BackendFailure);
        }
        let checkpoint = Self {
            arenas: columns::save(device, queue, arenas)?,
        };
        checkpoint.validate()?;
        Ok(checkpoint)
    }

    fn validate(&self) -> Result<(), Error> {
        match &self.arenas {
            columns::SavedArenas::Literal(arenas) => {
                arenas.validate()?;
                if arenas.counts[0] != 0 && arenas.layout.is_none() {
                    return Err(Error::BackendFailure);
                }
            }
            columns::SavedArenas::Encoded {
                headers,
                payload,
                bytes,
                counts,
                typed,
                words,
                groups,
            } => {
                let expected = [counts[0], counts[1], counts[2], counts[0]]
                    .into_iter()
                    .zip([10u64, 9, 8, 4])
                    .map(|(count, fields)| u64::from(count).div_ceil(256) * fields)
                    .sum::<u64>();
                if !typed
                    || counts[0] == 0
                    || expected != u64::from(*groups)
                    || u64::from(*groups) * 8 > headers.buffer.length()
                    || !words.is_multiple_of(8)
                    || *words > 500_000_000
                    || u64::from(*words) * 4 > payload.buffer.length()
                    || u64::from(counts[3]) > bytes.buffer.length()
                {
                    return Err(Error::BackendFailure);
                }
            }
            columns::SavedArenas::Semantic(_) => return Err(Error::BackendFailure),
        }
        Ok(())
    }

    pub(super) fn merge(
        device: &Device,
        queue: &metal::CommandQueueRef,
        resident: &ResidentInput,
        data: &MappedBuffer,
        inputs: [Self; 2],
        errors: &[Option<Error>],
        apply_limits: bool,
    ) -> Result<OutputArenas, Error> {
        merge::execute(device, queue, resident, data, inputs, errors, apply_limits)
    }

    pub(super) fn merge_all(
        device: &Device,
        queue: &metal::CommandQueueRef,
        resident: &ResidentInput,
        data: &MappedBuffer,
        mut inputs: Vec<Self>,
        errors: &[Option<Error>],
    ) -> Result<OutputArenas, Error> {
        if errors.len() != resident.input.grid.evaluation_spans.len() {
            return Err(Error::BackendFailure);
        }
        if inputs.is_empty() {
            return OutputArenas::empty(device);
        }
        if inputs.len() == 1 {
            inputs.push(Self::new(device, queue, OutputArenas::empty(device)?)?);
        }
        let mut inputs = inputs.into_iter();
        let mut output = inputs.next().ok_or(Error::BackendFailure)?;
        let remaining = inputs.len();
        for (index, next) in inputs.enumerate() {
            let last = index + 1 == remaining;
            let merged = Self::merge(device, queue, resident, data, [output, next], errors, last)?;
            if last {
                return Ok(merged);
            }
            output = Self::new(device, queue, merged)?;
        }
        Err(Error::BackendFailure)
    }
}

#[derive(Clone, Copy)]
pub(super) struct PageBounds {
    pub rows: u32,
    pub captures: u32,
    pub properties: u32,
    pub bytes: u32,
}
impl Default for PageBounds {
    fn default() -> Self {
        Self {
            rows: 4 * 1024 * 1024,
            captures: 8 * 1024 * 1024,
            properties: 1024 * 1024,
            bytes: 64 * 1024 * 1024,
        }
    }
}
impl PageBounds {
    fn validate(self) -> Result<Self, Error> {
        if self.rows == 0 || self.rows > 16 * 1024 * 1024 {
            return Err(Error::BackendCapacity);
        }
        Ok(self)
    }
}

pub(super) struct OutputArenas {
    pub rows: MappedBuffer,
    pub captures: MappedBuffer,
    pub properties: MappedBuffer,
    pub bytes: MappedBuffer,
    /// Typed rows: (property count, byte count, property base, byte base).
    /// None denotes structural output with no properties or pooled bytes.
    pub layout: Option<MappedBuffer>,
    pub counts: [u32; 4],
}
impl OutputArenas {
    pub(super) fn empty(device: &Device) -> Result<Self, Error> {
        Ok(Self {
            rows: MappedBuffer::zeroed::<ResultRecord>(device, 0)?,
            captures: MappedBuffer::zeroed::<FinalCaptureRecord>(device, 0)?,
            properties: MappedBuffer::zeroed::<PropertyRecord>(device, 0)?,
            bytes: MappedBuffer::zeroed::<u8>(device, 0)?,
            layout: None,
            counts: [0; 4],
        })
    }
    pub(super) fn validate(&self) -> Result<(), Error> {
        for (buffer, count, stride) in [
            (
                &self.rows,
                self.counts[0],
                std::mem::size_of::<ResultRecord>(),
            ),
            (
                &self.captures,
                self.counts[1],
                std::mem::size_of::<FinalCaptureRecord>(),
            ),
            (
                &self.properties,
                self.counts[2],
                std::mem::size_of::<PropertyRecord>(),
            ),
            (&self.bytes, self.counts[3], 1),
        ] {
            if (count as usize)
                .checked_mul(stride)
                .ok_or(Error::BackendCapacity)?
                > buffer.buffer.length() as usize
            {
                return Err(Error::BackendFailure);
            }
        }
        if let Some(layout) = &self.layout {
            if self.counts[0] as u64 * 16 > layout.buffer.length() {
                return Err(Error::BackendFailure);
            }
        } else if self.counts[2..] != [0, 0] {
            return Err(Error::BackendFailure);
        }
        if self.counts[0] == 0 && self.counts != [0; 4] {
            return Err(Error::BackendFailure);
        }
        Ok(())
    }
}

pub(super) struct SavedOutput {
    publication: GridPublication,
    gpu: Option<SavedGpu>,
    files: usize,
    terminal_span: u32,
    boundary: OutputBoundary,
    cursor: u32,
    next_file: u32,
    bounds: PageBounds,
}
struct SavedGpu {
    device: Device,
    queue: CommandQueue,
    input: MappedBuffer,
    arenas: columns::SavedArenas,
}
impl SavedOutput {
    pub(super) fn from_semantic_columns(
        resident: &ResidentInput,
        device: Device,
        queue: CommandQueue,
        input: MappedBuffer,
        arenas: super::metal_semantic_columns::SemanticColumns,
        errors: Vec<Option<Error>>,
    ) -> Result<Self, Error> {
        arenas.validate(resident, &input)?;
        let mut saved = Self::metadata(resident, errors)?;
        if arenas.counts[0] != 0 {
            saved.gpu = Some(SavedGpu {
                device,
                queue,
                input,
                arenas: columns::SavedArenas::Semantic(arenas),
            });
        } else {
            if arenas.counts != [0; 4] {
                return Err(Error::BackendFailure);
            }
            saved.boundary.semantic_span = saved.terminal_span;
            saved.boundary.block_lane = 0;
            saved.next_file = u32::MAX;
        }
        Ok(saved)
    }
    pub(super) fn new(
        resident: &ResidentInput,
        device: Device,
        queue: CommandQueue,
        input: MappedBuffer,
        arenas: OutputArenas,
        errors: Vec<Option<Error>>,
    ) -> Result<Self, Error> {
        arenas.validate()?;
        let mut saved = Self::metadata(resident, errors)?;
        if arenas.counts[0] != 0 {
            let arenas = columns::save(&device, &queue, arenas)?;
            saved.gpu = Some(SavedGpu {
                device,
                queue,
                input,
                arenas,
            });
        } else {
            saved.boundary.semantic_span = saved.terminal_span;
            saved.boundary.block_lane = 0;
            saved.next_file = u32::MAX;
        }
        Ok(saved)
    }
    pub(super) fn empty(resident: &ResidentInput) -> Result<Self, Error> {
        let mut saved = Self::metadata(
            resident,
            vec![None; resident.input.grid.evaluation_spans.len()],
        )?;
        saved.boundary.semantic_span = saved.terminal_span;
        saved.boundary.block_lane = 0;
        saved.next_file = u32::MAX;
        Ok(saved)
    }
    fn metadata(resident: &ResidentInput, errors: Vec<Option<Error>>) -> Result<Self, Error> {
        let grid = &resident.input.grid;
        let generation = NEXT_GENERATION
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| Error::BackendCapacity)?;
        let lane = grid
            .semantic_span_order
            .first()
            .map(|&i| grid.block_spans[i as usize].block_lane_start)
            .unwrap_or(0);
        Ok(Self {
            publication: GridPublication::new(
                Arc::clone(&resident.input.packed),
                &grid.evaluation_spans,
                errors,
            )?,
            gpu: None,
            files: resident.input.packed.files.len(),
            terminal_span: u32::try_from(grid.semantic_span_order.len())
                .map_err(|_| Error::BackendCapacity)?,
            boundary: OutputBoundary {
                semantic_span: 0,
                block_lane: lane,
                local_output_ordinal: 0,
                frontier_generation: generation,
            },
            cursor: 0,
            next_file: 0,
            bounds: PageBounds::default(),
        })
    }
    pub(super) fn with_bounds(mut self, bounds: PageBounds) -> Result<Self, Error> {
        self.bounds = bounds.validate()?;
        Ok(self)
    }
    fn page(
        &mut self,
        boundary: OutputBoundary,
        file_end: usize,
    ) -> Result<Option<GridFragment>, Error> {
        if boundary != self.boundary || file_end > self.files {
            return Err(Error::InvalidAnchor);
        }
        if self.next_file as usize >= file_end {
            return Ok(None);
        }
        let gpu = self.gpu.as_ref().ok_or(Error::BackendFailure)?;
        let a = &gpu.arenas;
        let counts = a.counts();
        if self.cursor >= counts[0] {
            return Err(Error::BackendFailure);
        }
        let bounds = self.bounds.validate()?;
        let p = [
            counts[0],
            counts[1],
            counts[2],
            counts[3],
            u32::from(a.typed()),
            self.cursor,
            u32::try_from(file_end).map_err(|_| Error::BackendCapacity)?,
            bounds.rows,
            bounds.captures,
            bounds.properties,
            bounds.bytes,
            boundary.semantic_span,
            boundary.block_lane,
            boundary.local_output_ordinal,
            a.encoding()[0],
            a.encoding()[1],
            a.encoding()[2],
            a.semantic_metadata()[0],
            a.semantic_metadata()[1],
            a.semantic_metadata()[2],
            a.semantic_metadata()[3],
        ];
        let candidates = bounds.rows.min(counts[0] - self.cursor) as usize;
        let offsets = MappedBuffer::zeroed::<[u32; 4]>(&gpu.device, candidates + 1)?;
        let group_count = candidates.div_ceil(16384);
        let groups = super::metal_parallel::Scan::new(&gpu.device, group_count)?;
        let selected = MappedBuffer::zeroed::<u32>(&gpu.device, group_count)?;
        let header = MappedBuffer::zeroed::<u32>(&gpu.device, 10)?;
        let [
            stored_rows,
            stored_captures,
            stored_properties,
            stored_bytes,
            layout,
            column_headers,
        ] = a.buffers();
        let [
            semantic_prefixes,
            semantic_segments,
            complex_rows,
            complex_captures,
            complex_properties,
            complex_layout,
        ] = a.semantic_buffers();
        metal::objc::rc::autoreleasepool(|| -> Result<(), Error> {
            let command = gpu.queue.new_command_buffer();
            super::metal_parallel::encode(
                command,
                &pipeline_named(SOURCE, "output_page_count")?,
                "output_page_count",
                &[
                    &gpu.input.buffer,
                    &stored_rows.buffer,
                    &layout.buffer,
                    &offsets.buffer,
                    &header.buffer,
                    &groups.values.buffer,
                    &selected.buffer,
                    &column_headers.buffer,
                    &semantic_prefixes.buffer,
                    &semantic_segments.buffer,
                    &complex_rows.buffer,
                    &complex_layout.buffer,
                ],
                &p,
                group_count,
                None,
            );
            if group_count > 1 {
                super::metal_parallel::encode(
                    command,
                    &pipeline_named(SOURCE, "output_page_boundary")?,
                    "output_page_boundary",
                    &[
                        &gpu.input.buffer,
                        &stored_rows.buffer,
                        &offsets.buffer,
                        &groups.values.buffer,
                        groups.total(),
                        &selected.buffer,
                        &header.buffer,
                        &column_headers.buffer,
                        &semantic_prefixes.buffer,
                        &semantic_segments.buffer,
                        &complex_rows.buffer,
                    ],
                    &p,
                    1,
                    None,
                );
            }
            commit_checked(command)?;
            wait_checked(command)
        })?;
        let h = read::<u32>(&header, 10)?;
        match h[9] {
            0 => {}
            1 => return Err(Error::BackendCapacity),
            _ => return Err(Error::BackendFailure),
        }
        if h[0] > bounds.rows
            || h[1] > bounds.captures
            || h[2] > bounds.properties
            || h[3] > bounds.bytes
            || self.cursor.checked_add(h[0]) != Some(h[4])
            || h[4] > counts[0]
            || h[5] > self.terminal_span
            || (h[0] == 0 && (h[8] as usize) < file_end)
        {
            return Err(Error::BackendFailure);
        }
        let next = OutputBoundary {
            semantic_span: h[5],
            block_lane: h[6],
            local_output_ordinal: h[7],
            frontier_generation: boundary.frontier_generation,
        };
        if h[0] != 0 {
            super::metal_grid_plan::ProgressBoundary::Output.check_advance(
                super::metal_grid_plan::ContinuationPoint::Output(boundary),
                super::metal_grid_plan::ContinuationPoint::Output(next),
            )?;
        }
        let fragment = if h[0] == 0 {
            None
        } else {
            let rows = MappedBuffer::zeroed::<ResultRecord>(&gpu.device, h[0] as usize)?;
            let caps = MappedBuffer::zeroed::<FinalCaptureRecord>(&gpu.device, h[1] as usize)?;
            let props = MappedBuffer::zeroed::<PropertyRecord>(&gpu.device, h[2] as usize)?;
            let bytes = MappedBuffer::zeroed::<u8>(&gpu.device, h[3] as usize)?;
            let count = *h[..4].iter().max().ok_or(Error::BackendFailure)? as usize;
            metal::objc::rc::autoreleasepool(|| -> Result<(), Error> {
                let command = gpu.queue.new_command_buffer();
                super::metal_parallel::encode(
                    command,
                    &pipeline_named(SOURCE, "output_page_scatter")?,
                    "output_page_scatter",
                    &[
                        &stored_rows.buffer,
                        &stored_captures.buffer,
                        &stored_properties.buffer,
                        &stored_bytes.buffer,
                        &layout.buffer,
                        &offsets.buffer,
                        &header.buffer,
                        &rows.buffer,
                        &caps.buffer,
                        &props.buffer,
                        &bytes.buffer,
                        &groups.values.buffer,
                        &column_headers.buffer,
                        &semantic_prefixes.buffer,
                        &semantic_segments.buffer,
                        &complex_rows.buffer,
                        &complex_captures.buffer,
                        &complex_properties.buffer,
                        &complex_layout.buffer,
                    ],
                    &p,
                    count.div_ceil(256),
                    None,
                );
                commit_checked(command)?;
                wait_checked(command)
            })?;
            if read::<u32>(&header, 10)? != h {
                return Err(Error::BackendFailure);
            }
            Some(self.publication.validate_page(
                read(&rows, h[0] as usize)?,
                read(&caps, h[1] as usize)?,
                read(&props, h[2] as usize)?,
                read(&bytes, h[3] as usize)?,
            )?)
        };
        self.cursor = h[4];
        self.boundary = next;
        self.next_file = h[8];
        Ok(fragment)
    }
    fn interval(&mut self, files: std::ops::Range<usize>) -> Result<Vec<GridFragment>, Error> {
        let mut fragments = Vec::new();
        while (self.next_file as usize) < files.end {
            if let Some(fragment) = self.page(self.boundary, files.end)? {
                fragments.push(fragment);
            }
        }
        Ok(fragments)
    }
    pub(super) fn execute(mut self) -> Result<PackedExecution, Error> {
        let files = 0..self.files;
        let fragments = self.interval(files.clone())?;
        self.publication.execution(fragments, files)
    }
    pub(super) fn execute_pages(
        mut self,
        consumer: &mut super::metal_grid_executor::PageConsumer<'_>,
    ) -> Result<(), Error> {
        for first in (0..self.files).step_by(64) {
            let end = (first + 64).min(self.files);
            let fragments = self.interval(first..end)?;
            consumer(self.publication.execution(fragments, first..end)?, end)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "metal_saved_output_contract_tests.rs"]
mod tests;
