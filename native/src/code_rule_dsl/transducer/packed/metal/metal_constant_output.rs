//! Count and write a broad finite class directly into final literal columns.
use super::Error;
use super::metal_backend::launch::{commit_checked, wait_checked};
use super::metal_backend::{MappedBuffer, pipeline_named};
use super::metal_grid_plan::ResidentInput;
use super::metal_output_pages::OutputArenas;
use super::metal_parallel::{Scan, encode};
use super::metal_register_codegen::RegisterCode;
use super::metal_register_execution::read;
use super::metal_semantic_columns::SemanticColumns;
use metal::{CommandQueueRef, Device};
use std::collections::BTreeSet;

const COMPACT_THRESHOLD: usize = 256 * 1024 * 1024;
#[cfg(test)]
#[path = "metal_finite_literal_class_tests.rs"]
mod finite_literal_tests;
pub(super) const MAX_CONSTANT_DIRECTIVES: u32 = 256;

pub(super) fn constant_program(resident: &ResidentInput, program: usize) -> Result<bool, Error> {
    let packed = &resident.input.packed;
    let pg = packed.programs.get(program).ok_or(Error::BackendFailure)?;
    let words = pg
        .output_count
        .checked_mul(6)
        .ok_or(Error::BackendCapacity)?;
    let end = pg
        .output_start
        .checked_add(words)
        .ok_or(Error::BackendCapacity)?;
    let instructions = packed
        .assertion_data
        .get(pg.output_start as usize..end as usize)
        .ok_or(Error::BackendFailure)?;
    Ok(finite_capture_literals(resident, pg)?
        && pg.output_count <= MAX_CONSTANT_DIRECTIVES
        && instructions.chunks_exact(6).all(|op| op[0] <= 1))
}

fn finite_capture_literals(
    resident: &ResidentInput,
    pg: &super::metal_batch_abi::MetalProgram,
) -> Result<bool, Error> {
    if pg.capture_count == 0 {
        return Ok(true);
    }
    if pg.flags & 1 == 0 || pg.capture_count > MAX_CONSTANT_DIRECTIVES {
        return Ok(false);
    }
    let batch = &resident.input.packed;
    let structural_end = pg
        .structural_start
        .checked_add(pg.structural_count)
        .ok_or(Error::BackendFailure)?;
    let capture_end = pg
        .capture_start
        .checked_add(pg.capture_count)
        .ok_or(Error::BackendFailure)?;
    let words = batch
        .words
        .get(pg.structural_start as usize..structural_end as usize)
        .ok_or(Error::BackendFailure)?;
    let captures = words
        .iter()
        .filter(|&&w| super::split(w).0 == super::CAPTURE)
        .count();
    let mut comparisons = 0usize;
    for assertion in batch
        .assertions
        .get(pg.capture_start as usize..capture_end as usize)
        .ok_or(Error::BackendFailure)?
    {
        let strings = match assertion.kind {
            1 => {
                let value_index = pg
                    .value_start
                    .checked_add(assertion.second)
                    .ok_or(Error::BackendFailure)?;
                let value = batch
                    .values
                    .get(value_index as usize)
                    .ok_or(Error::BackendFailure)?;
                if value.kind > 1 {
                    return Ok(false);
                }
                vec![value.operand]
            }
            2 if assertion.data_count <= MAX_CONSTANT_DIRECTIVES => {
                let end = assertion
                    .data_start
                    .checked_add(assertion.data_count)
                    .ok_or(Error::BackendFailure)?;
                batch
                    .assertion_data
                    .get(assertion.data_start as usize..end as usize)
                    .ok_or(Error::BackendFailure)?
                    .to_vec()
            }
            _ => return Ok(false),
        };
        comparisons = comparisons
            .checked_add(strings.len().max(1))
            .ok_or(Error::BackendCapacity)?;
        for string in strings {
            let index = pg
                .string_start
                .checked_add(string)
                .ok_or(Error::BackendFailure)?;
            if batch
                .strings
                .get(index as usize)
                .ok_or(Error::BackendFailure)?
                .byte_length
                > 64
            {
                return Ok(false);
            }
        }
    }
    Ok(captures
        .checked_mul(comparisons)
        .and_then(|n| n.checked_mul(66))
        .ok_or(Error::BackendCapacity)?
        <= super::MAX_STEPS_PER_PATH)
}

pub(super) struct ConstantOutput<'a> {
    resident: &'a ResidentInput,
    code: &'a RegisterCode,
    programs: Vec<bool>,
    blocks: Vec<u32>,
}

impl<'a> ConstantOutput<'a> {
    pub(super) fn new(
        resident: &'a ResidentInput,
        code: &'a RegisterCode,
    ) -> Result<Option<Self>, Error> {
        let grid = &resident.input.grid;
        let eligible = (0..resident.input.packed.programs.len())
            .map(|program| constant_program(resident, program))
            .collect::<Result<Vec<_>, _>>()?;
        let mut programs = vec![false; resident.input.packed.programs.len()];
        let mut blocks = BTreeSet::new();
        let mut raw_bound = 0_usize;
        for span in &grid.block_spans {
            let block = grid.blocks[span.execution_block as usize];
            let Some(bound) = code.bounds.get(&block.dispatch_signature) else {
                continue;
            };
            let evaluation = grid.evaluation_spans[span.evaluation_span as usize];
            let program = evaluation.program as usize;
            if !eligible[program] {
                continue;
            }
            programs[program] = true;
            blocks.insert(span.execution_block);
            let bytes = bound
                .captures
                .checked_mul(std::mem::size_of::<
                    super::metal_batch_abi::FinalCaptureRecord,
                >())
                .and_then(|caps| {
                    caps.checked_add(std::mem::size_of::<super::metal_batch_abi::ResultRecord>())
                })
                .and_then(|stride| stride.checked_mul(bound.matches))
                .and_then(|bytes| bytes.checked_mul(span.root_count as usize))
                .ok_or(Error::BackendCapacity)?;
            raw_bound = raw_bound.checked_add(bytes).ok_or(Error::BackendCapacity)?;
        }
        // This selects storage for an immutable work class before execution.
        // Errors and malformed producers never switch physical formats.
        Ok((raw_bound >= COMPACT_THRESHOLD).then(|| Self {
            resident,
            code,
            programs,
            blocks: blocks.into_iter().collect(),
        }))
    }

    pub(super) fn owns(&self, program: usize) -> Result<bool, Error> {
        self.programs
            .get(program)
            .copied()
            .ok_or(Error::BackendFailure)
    }

    pub(super) fn execute(
        &self,
        device: &Device,
        queue: &CommandQueueRef,
        data: &MappedBuffer,
        roots: &MappedBuffer,
        progress: &MappedBuffer,
        mut complex: OutputArenas,
    ) -> Result<SemanticColumns, Error> {
        complex.validate()?;
        let grid = &self.resident.input.grid;
        let spans = grid.semantic_span_order.len();
        let n = u32::try_from(spans).map_err(|_| Error::BackendCapacity)?;
        let evaluations =
            u32::try_from(grid.evaluation_spans.len()).map_err(|_| Error::BackendCapacity)?;
        let limited = self
            .programs
            .iter()
            .zip(&self.resident.witness_limits)
            .any(|(owned, limit)| *owned && *limit != u32::MAX);
        let blocks = MappedBuffer::from_slice(device, &self.blocks)?;
        let unlimited = Scan::new(device, spans)?;
        let clipped = limited.then(|| Scan::new(device, spans)).transpose()?;
        let final_scan = clipped.as_ref().unwrap_or(&unlimited);
        let mut p = [
            0,
            n,
            u32::try_from(self.blocks.len()).map_err(|_| Error::BackendCapacity)?,
            0,
            0,
            evaluations,
            u32::MAX,
            u32::MAX,
            super::metal_register_execution::RegisterOutputClass::ConstantProperties as u32,
        ];
        metal::objc::rc::autoreleasepool(|| -> Result<(), Error> {
            let command = queue.new_command_buffer();
            encode(
                command,
                &pipeline_named(&self.code.source, "register_direct_count")?,
                "constant_output_count",
                &[
                    &data.buffer,
                    &roots.buffer,
                    &blocks.buffer,
                    &unlimited.values.buffer,
                    &unlimited.values.buffer,
                    &progress.buffer,
                ],
                &p,
                self.blocks.len(),
                None,
            );
            unlimited.encode(command)?;
            if limited {
                p[3] = 1;
                encode(
                    command,
                    &pipeline_named(&self.code.source, "register_direct_count")?,
                    "constant_output_limit",
                    &[
                        &data.buffer,
                        &roots.buffer,
                        &blocks.buffer,
                        &final_scan.values.buffer,
                        &unlimited.values.buffer,
                        &progress.buffer,
                    ],
                    &p,
                    self.blocks.len(),
                    None,
                );
                final_scan.encode(command)?;
            }
            commit_checked(command)?;
            wait_checked(command)
        })?;
        let static_total = checked_total(final_scan)?;
        checked_total(&unlimited)?;
        if static_total[3] != 0 {
            return Err(Error::BackendFailure);
        }
        // Structural output's explicit empty layout belongs to this producer.
        // A semantic column view always receives a complete layout arena.
        if complex.layout.is_none() && complex.counts[0] != 0 {
            complex.layout = Some(MappedBuffer::zeroed::<[u32; 4]>(
                device,
                complex.counts[0] as usize,
            )?);
        }
        let segments = MappedBuffer::zeroed::<[u32; 12]>(device, spans)?;
        let sizes = Scan::new(device, spans)?;
        let semantic = Scan::new(device, spans)?;
        let status = MappedBuffer::zeroed::<u32>(device, 1)?;
        let layout = complex.layout.as_ref().unwrap_or(&complex.rows);
        metal::objc::rc::autoreleasepool(|| -> Result<(), Error> {
            let command = queue.new_command_buffer();
            encode(
                command,
                &pipeline_named(&self.code.source, "constant_output_plan")?,
                "constant_output_plan",
                &[
                    &data.buffer,
                    &final_scan.values.buffer,
                    final_scan.total(),
                    &complex.rows.buffer,
                    &layout.buffer,
                    &segments.buffer,
                    &sizes.values.buffer,
                    &semantic.values.buffer,
                    &status.buffer,
                ],
                &complex.counts,
                spans.div_ceil(256),
                None,
            );
            sizes.encode_pair(&semantic, command)?;
            commit_checked(command)?;
            wait_checked(command)
        })?;
        check(&status)?;
        let allocation = checked_total(&sizes)?;
        let counts = checked_total(&semantic)?;
        let expected =
            std::array::from_fn::<_, 4, _>(|i| static_total[i].checked_add(complex.counts[i]));
        if expected.iter().any(Option::is_none) {
            return Err(Error::BackendCapacity);
        }
        if counts != expected.map(Option::unwrap) || allocation[2..] != [0, 0] {
            return Err(Error::BackendFailure);
        }
        if allocation[1] > 500_000_000 || !allocation[1].is_multiple_of(8) {
            return Err(Error::BackendCapacity);
        }
        let headers = MappedBuffer::zeroed::<[u32; 2]>(device, allocation[0] as usize)?;
        let payload = MappedBuffer::zeroed::<u32>(device, allocation[1] as usize)?;
        metal::objc::rc::autoreleasepool(|| -> Result<(), Error> {
            let command = queue.new_command_buffer();
            encode(
                command,
                &pipeline_named(&self.code.source, "constant_output_headers")?,
                "constant_output_headers",
                &[
                    &data.buffer,
                    &sizes.values.buffer,
                    &segments.buffer,
                    &headers.buffer,
                    &status.buffer,
                ],
                &[n, allocation[0], allocation[1], 0],
                spans.div_ceil(256),
                None,
            );
            let mut scatter = p.to_vec();
            scatter.extend([allocation[0], allocation[1]]);
            encode(
                command,
                &pipeline_named(&self.code.source, "constant_output_scatter")?,
                "constant_output_scatter",
                &[
                    &data.buffer,
                    &roots.buffer,
                    &blocks.buffer,
                    &unlimited.values.buffer,
                    &progress.buffer,
                    &segments.buffer,
                    &headers.buffer,
                    &payload.buffer,
                    &status.buffer,
                ],
                &scatter,
                self.blocks.len(),
                None,
            );
            commit_checked(command)?;
            wait_checked(command)
        })?;
        check(&status)?;
        let prefixes = semantic.values;
        Ok(SemanticColumns {
            prefixes,
            segments,
            headers,
            payload,
            complex,
            counts,
            words: allocation[1],
            groups: allocation[0],
            spans: n,
            owner: [
                self.resident as *const _ as usize,
                (&*data.buffer as *const metal::BufferRef) as usize,
            ],
        })
    }
}

fn checked_total(scan: &Scan) -> Result<[u32; 4], Error> {
    let total = unsafe { scan.total().contents().cast::<[u32; 4]>().read() };
    if total.contains(&u32::MAX) {
        Err(Error::BackendCapacity)
    } else {
        Ok(total)
    }
}
fn check(status: &MappedBuffer) -> Result<(), Error> {
    match read::<u32>(status, 1)?[0] {
        0 => Ok(()),
        1 => Err(Error::BackendCapacity),
        _ => Err(Error::BackendFailure),
    }
}
