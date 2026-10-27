//! Broad initial-state generation from shared resident selectors.
use super::metal_backend::launch::check_submission;
use super::metal_backend::{MappedBuffer, pipeline_named};
use super::metal_grid_plan::{ExecutionPlan, ResidentInput, StageKind, WorkDomain};
use super::metal_parallel::{Scan, encode as dispatch};
use super::metal_state_values::{COLUMNS, FrontierRecords, INITIAL_COLUMNS, POSITIONED_COLUMNS};
use super::{
    ABSENT_FIELD, CAPTURE, CHILD, DESCEND, Error, FIELD, FLAGS, FOLLOWING, KIND, SAVE_CURSOR, SIZE,
    SUPERTYPE, split,
};
use metal::{CommandBufferRef, Device};
use std::ops::Range;

/// Task-only initial records derive their root and Program from immutable selectors.
/// They are not an active frontier or a value-arena input.
pub(super) const TASK_SEED_COLUMNS: usize = 1;

fn source(resident: &ResidentInput) -> Result<String, Error> {
    Ok(format!(
        "{}\n{}\n{}",
        include_str!("metal_resident.metal"),
        super::metal_initial_prefix::guards(resident)?,
        include_str!("metal_frontier_seed.metal")
    ))
}

pub(super) struct SeedInput<'a> {
    pub resident: &'a ResidentInput,
    pub plan: &'a ExecutionPlan,
    pub data: &'a MappedBuffer,
    pub roots: &'a MappedBuffer,
    pub spans: Range<usize>,
}

#[must_use = "Check seed failures after the shared command completes"]
pub(super) struct PendingSeeds {
    command: metal::CommandBuffer,
    counted: CountedSeeds,
}

/// GPU-owned eligibility and exact span prefixes precede state allocation.
pub(super) struct CountedSeeds {
    command: metal::CommandBuffer,
    source: String,
    scan: Scan,
    blocks: MappedBuffer,
    masks: MappedBuffer,
    probes: MappedBuffer,
    status: MappedBuffer,
    parameters: [u32; 8],
    bindings: [usize; 4],
}

impl CountedSeeds {
    pub(super) fn count(&self) -> Result<usize, Error> {
        check_submission()?;
        if self.command.status() != metal::MTLCommandBufferStatus::Completed {
            return Err(Error::BackendFailure);
        }
        match unsafe { self.status.buffer.contents().cast::<u32>().read() } {
            0 => {}
            1 => return Err(Error::BackendCapacity),
            _ => return Err(Error::BackendFailure),
        }
        let total = unsafe { self.scan.total().contents().cast::<[u32; 4]>().read() };
        if total[0] == u32::MAX {
            return Err(Error::BackendCapacity);
        }
        if total[1..] != [0, 0, 0] || total[0] > self.parameters[7] {
            return Err(Error::BackendFailure);
        }
        if std::env::var_os("TEMPO_PACKED_BATCH_PROFILE").is_some() {
            eprintln!(
                "TEMPO_METAL_SEED_COUNTS logical={} live={}",
                self.parameters[7], total[0]
            );
        }
        Ok(total[0] as usize)
    }
}

impl PendingSeeds {
    /// Submission and waiting belong to the caller. Scratch stays live until this check.
    pub(super) fn check_completed(self) -> Result<(), Error> {
        check_submission()?;
        if self.command.status() != metal::MTLCommandBufferStatus::Completed {
            return Err(Error::BackendFailure);
        }
        match unsafe { self.counted.status.buffer.contents().cast::<u32>().read() } {
            0 => Ok(()),
            1 => Err(Error::BackendCapacity),
            _ => Err(Error::BackendFailure),
        }
    }
}

fn checked(value: usize) -> Result<u32, Error> {
    u32::try_from(value).map_err(|_| Error::BackendCapacity)
}

fn bindings(input: &SeedInput<'_>) -> [usize; 4] {
    [
        input.resident as *const _ as usize,
        input.plan as *const _ as usize,
        &*input.data.buffer as *const metal::BufferRef as usize,
        &*input.roots.buffer as *const metal::BufferRef as usize,
    ]
}

/// Initial guards and edits cannot change the current node or raise an operand
/// error. A proof stops before branches, predicates, restores, and repeat work.
/// Its result tests root eligibility; successful states still start at PC zero.
fn probes(resident: &ResidentInput) -> Vec<[u32; 8]> {
    resident
        .input
        .packed
        .programs
        .iter()
        .map(|pg| {
            let words = &resident.input.packed.words[pg.structural_start as usize
                ..(pg.structural_start + pg.structural_count) as usize];
            let mut probe = [0, 0, 0, u32::MAX];
            // Selector key, kind/field index, required field, required kind.
            let mut child = [u32::MAX, 0, u32::MAX, u32::MAX];
            for (pc, &word) in words.iter().enumerate().take(64) {
                let (op, mode, operand) = split(word);
                match op {
                    KIND => {
                        if probe[3] != u32::MAX && probe[3] != u32::from(operand) {
                            probe[1] |= 1;
                            probe[2] |= 1;
                            break;
                        }
                        probe[3] = u32::from(operand);
                    }
                    FLAGS => {
                        probe[1] |= u32::from(mode);
                        probe[2] |= u32::from(operand);
                    }
                    FIELD | SIZE | SUPERTYPE | ABSENT_FIELD | CAPTURE | SAVE_CURSOR => {}
                    CHILD | DESCEND | FOLLOWING => {
                        probe[0] = u32::from(op) | (u32::from(mode) << 8);
                        if op == CHILD
                            && let Some((key, by_kind)) = resident.input.children.seed_selector(
                                words,
                                pc,
                                (probe[3] != u32::MAX).then_some(probe[3]),
                            )
                        {
                            let (field, kind) = super::metal_child_index::selectors(words, pc);
                            let operand = |at: Option<usize>| {
                                at.map_or(u32::MAX, |at| u32::from(split(words[at]).2))
                            };
                            child = [key, u32::from(by_kind), operand(field), operand(kind)];
                        }
                        break;
                    }
                    _ => break,
                }
            }
            [
                probe[0], probe[1], probe[2], probe[3], child[0], child[1], child[2], child[3],
            ]
        })
        .collect()
}

pub(super) fn encode_counts(
    device: &Device,
    command: &CommandBufferRef,
    input: &SeedInput<'_>,
) -> Result<CountedSeeds, Error> {
    check_submission()?;
    input.plan.validate(input.resident)?;
    let span_count = input
        .spans
        .end
        .checked_sub(input.spans.start)
        .ok_or(Error::InvalidAnchor)?;
    if input.spans.end > input.resident.input.grid.semantic_span_order.len() {
        return Err(Error::InvalidAnchor);
    }
    let root_bytes = input
        .resident
        .input
        .grid
        .roots
        .len()
        .checked_mul(4)
        .ok_or(Error::BackendCapacity)?;
    if input.roots.buffer.length() < root_bytes as u64
        || std::ptr::eq(&*input.data.buffer, &*input.roots.buffer)
    {
        return Err(Error::BackendFailure);
    }
    let blocks = blocks(input.plan)?;
    let block_count = blocks.len();
    let probes = probes(input.resident);
    if std::env::var_os("TEMPO_PACKED_BATCH_PROFILE").is_some() {
        let grid = &input.resident.input.grid;
        let mut positions = vec![0usize; grid.semantic_span_order.len()];
        for (position, &span) in grid.semantic_span_order.iter().enumerate() {
            positions[span as usize] = position;
        }
        let mut roots = std::collections::BTreeMap::<u32, usize>::new();
        for &id in &blocks {
            let block = grid.blocks[id as usize];
            for id in block.block_span_start..block.block_span_start + block.block_span_count {
                let span = grid.block_spans[id as usize];
                if input.spans.contains(&positions[id as usize]) {
                    *roots
                        .entry(grid.evaluation_spans[span.evaluation_span as usize].program)
                        .or_default() += span.root_count as usize;
                }
            }
        }
        eprintln!("TEMPO_METAL_SEED_DOMAIN {}", serde_json::json!(roots.iter().map(|(&program, &roots)|
            serde_json::json!({"program": program, "roots": roots, "probe": probes[program as usize]})).collect::<Vec<_>>()));
    }
    let counted = CountedSeeds {
        command: command.to_owned(),
        source: source(input.resident)?,
        scan: Scan::new(device, span_count)?,
        blocks: MappedBuffer::from_slice(device, &blocks)?,
        masks: MappedBuffer::zeroed::<[u32; 8]>(device, block_count)?,
        probes: MappedBuffer::from_slice(device, &probes)?,
        status: MappedBuffer::zeroed::<u32>(device, 1)?,
        parameters: [
            checked(input.spans.start)?,
            checked(span_count)?,
            checked(block_count)?,
            0,
            0,
            0,
            0,
            checked(capacity(input.resident, input.plan)?)?,
        ],
        bindings: bindings(input),
    };
    if block_count != 0 {
        dispatch(
            command,
            &pipeline_named(&counted.source, "seed_span_counts")?,
            "seed_span_counts",
            &[
                &input.data.buffer,
                &input.roots.buffer,
                &counted.blocks.buffer,
                &counted.scan.values.buffer,
                &counted.masks.buffer,
                &counted.status.buffer,
                &counted.probes.buffer,
            ],
            &counted.parameters,
            block_count,
            None,
        );
    }
    counted.scan.encode(command)?;
    Ok(counted)
}

/// Immutable block metadata defines the exact seed domain. Task ordinals still
/// address the complete resident evaluation; direct tuples never become states.
pub(super) fn capacity(resident: &ResidentInput, plan: &ExecutionPlan) -> Result<usize, Error> {
    plan.validate(resident)?;
    let mut seen = std::collections::BTreeSet::new();
    blocks(plan)?.into_iter().try_fold(0usize, |total, id| {
        if !seen.insert(id) {
            return Err(Error::InvalidAnchor);
        }
        let block = resident
            .input
            .grid
            .blocks
            .get(id as usize)
            .ok_or(Error::InvalidAnchor)?;
        total
            .checked_add(block.logical_count as usize)
            .ok_or(Error::BackendCapacity)
    })
}
pub(super) fn blocks(plan: &ExecutionPlan) -> Result<Vec<u32>, Error> {
    let mut blocks = Vec::new();
    for stage in &plan.stages {
        if stage.kind != StageKind::SeedDynamicFrontier {
            continue;
        }
        let WorkDomain::Blocks(groups) = &stage.work else {
            return Err(Error::InvalidAnchor);
        };
        for group in groups {
            blocks.extend(group.blocks.clone());
        }
    }
    Ok(blocks)
}

#[cfg(test)]
pub(super) fn encode<const FIELDS: usize>(
    device: &Device,
    command: &CommandBufferRef,
    input: SeedInput<'_>,
    output: &FrontierRecords<FIELDS>,
) -> Result<PendingSeeds, Error> {
    // Validate output before encoding the producer, including refusal fixtures.
    validate_output(&input, output)?;
    let counted = encode_counts(device, command, &input)?;
    encode_scatter(command, input, output, counted)
}

fn validate_output<const FIELDS: usize>(
    input: &SeedInput<'_>,
    output: &FrontierRecords<FIELDS>,
) -> Result<(), Error> {
    check_submission()?;
    if ![
        TASK_SEED_COLUMNS,
        INITIAL_COLUMNS,
        COLUMNS,
        POSITIONED_COLUMNS,
    ]
    .contains(&FIELDS)
    {
        return Err(Error::BackendFailure);
    }
    input.plan.validate(input.resident)?;
    let grid = &input.resident.input.grid;
    input
        .spans
        .end
        .checked_sub(input.spans.start)
        .ok_or(Error::InvalidAnchor)?;
    let column_bytes = output
        .capacity
        .checked_mul(FIELDS * 4)
        .ok_or(Error::BackendCapacity)?;
    let root_bytes = grid
        .roots
        .len()
        .checked_mul(4)
        .ok_or(Error::BackendCapacity)?;
    if input.spans.end > grid.semantic_span_order.len() {
        return Err(Error::InvalidAnchor);
    }
    if output.generation != 0
        || output.columns.buffer.length() < column_bytes as u64
        || output.counts.buffer.length() < 20
        || input.roots.buffer.length() < root_bytes as u64
    {
        return Err(Error::BackendFailure);
    }
    checked(output.capacity)?;
    let buffers = [
        &*input.data.buffer,
        &*input.roots.buffer,
        &*output.columns.buffer,
        &*output.counts.buffer,
    ];
    for (i, buffer) in buffers.iter().enumerate() {
        if buffers[i + 1..]
            .iter()
            .any(|other| std::ptr::eq(*buffer, *other))
        {
            return Err(Error::BackendFailure);
        }
    }
    Ok(())
}

pub(super) fn encode_scatter<const FIELDS: usize>(
    command: &CommandBufferRef,
    input: SeedInput<'_>,
    output: &FrontierRecords<FIELDS>,
    mut counted: CountedSeeds,
) -> Result<PendingSeeds, Error> {
    validate_output(&input, output)?;
    if !std::ptr::eq(command, &*counted.command) {
        counted.count()?;
    }
    if counted.bindings != bindings(&input)
        || counted.parameters[0] as usize != input.spans.start
        || counted.parameters[1] as usize != input.spans.end - input.spans.start
    {
        return Err(Error::BackendFailure);
    }
    let source = format!("#define SEED_COLUMNS {FIELDS}u\n{}", counted.source);
    counted.parameters[6] = checked(output.capacity)?;
    let parameters = counted.parameters;
    let block_count = parameters[2] as usize;
    let pending = PendingSeeds {
        command: command.to_owned(),
        counted,
    };
    let boundary = pipeline_named(&source, "seed_boundary")?;
    dispatch(
        command,
        &boundary,
        "seed_boundary",
        &[
            pending.counted.scan.total(),
            &output.counts.buffer,
            &pending.counted.status.buffer,
        ],
        &parameters,
        1,
        None,
    );
    if block_count != 0 {
        let scatter = pipeline_named(&source, "seed_scatter")?;
        dispatch(
            command,
            &scatter,
            "seed_scatter",
            &[
                &input.data.buffer,
                &input.roots.buffer,
                &pending.counted.blocks.buffer,
                &pending.counted.scan.values.buffer,
                &output.columns.buffer,
                &output.counts.buffer,
                &pending.counted.status.buffer,
                &pending.counted.masks.buffer,
            ],
            &parameters,
            block_count,
            None,
        );
    }
    Ok(pending)
}
