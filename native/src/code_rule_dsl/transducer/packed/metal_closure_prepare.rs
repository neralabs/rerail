//! Resident range production for fresh, logically ordered closure headers.
use super::metal_backend::MappedBuffer;
use super::metal_bulk_closure::ClosureRanges;
use super::metal_grid_plan::ResidentInput;
use super::metal_state_values::{StateValues, ValueFrontier};
use super::{Error, Program};
use metal::{CommandBufferRef, Device};

/// The scheduler supplies a complete fresh header batch, before any header in
/// that scope expands. Array order is logical entry priority, with gaps retained
/// in each state's ordinal. Partially expanded GREEDY scopes are not this input.
#[cfg(test)]
pub(super) struct ClosureInput<'a> {
    pub resident: &'a ResidentInput,
    pub programs: &'a [Program],
    pub data: &'a MappedBuffer,
    pub states: &'a ValueFrontier,
    pub values: &'a StateValues,
}

pub(super) struct PreparedRanges {
    pub ranges: ClosureRanges,
    /// Resident node at each cell address, for continuation reconstruction.
    pub nodes: MappedBuffer,
}

use super::metal_backend::launch::check_submission;
use super::metal_backend::pipeline_named;
use super::metal_grid_plan::ExecutionPlan;
use super::metal_parallel::{Scan, encode as dispatch};
use super::metal_state_values::{self, COLUMNS, PendingValues, ValueGroups};
use super::{FIELD, FLAGS, KIND, SIZE, SIZE_LONG, closure_proof, split};
use metal::BufferRef;
use std::fmt::Write as _;

#[must_use = "Check preparation failures after the caller-owned command completes"]
pub(super) struct PendingPreparation {
    command: metal::CommandBuffer,
    grouping: PendingValues,
    // Keep every private input and scan allocation alive through completion.
    _projected: ValueFrontier,
    _groups: ValueGroups,
    _scan: Scan,
    _scratch: Vec<MappedBuffer>,
    status: MappedBuffer,
}
impl PendingPreparation {
    pub(super) fn status_buffers(&self) -> [&BufferRef; 2] {
        [&self.status.buffer, self.grouping.status_header()]
    }
    pub(super) fn check_completed(self) -> Result<(), Error> {
        check_submission()?;
        if self.command.status() != metal::MTLCommandBufferStatus::Completed {
            return Err(Error::BackendFailure);
        }
        self.grouping.check_completed()?;
        match unsafe { self.status.buffer.contents().cast::<u32>().read() } {
            0 => Ok(()),
            1 => Err(Error::BackendCapacity),
            _ => Err(Error::BackendFailure),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ClosureCode {
    source: String,
    continuation_source: String,
    descriptors: Vec<[u32; 4]>,
}

impl ClosureCode {
    pub(super) fn continuation_source(&self) -> &str {
        &self.continuation_source
    }
}

fn guard_expression(program: &Program, instruction: u32) -> Result<String, Error> {
    let (op, mode, arg) = split(instruction);
    Ok(match op {
        KIND => format!("n.kind=={arg}u"),
        FIELD => format!("n.field=={arg}u"),
        FLAGS => format!("(n.flags&{mode}u)=={mode}u && (n.flags&{arg}u)==0u"),
        SIZE => {
            let comparison = match mode & !SIZE_LONG {
                0 => "==",
                1 => ">=",
                2 => "<=",
                _ => return Err(Error::BackendFailure),
            };
            let value = if mode & SIZE_LONG == 0 {
                u64::from(arg)
            } else {
                program.longs[usize::from(arg)]
            };
            format!("ulong(n.source_end-n.source_start){comparison}{value}ul")
        }
        _ => return Err(Error::BackendFailure),
    })
}

pub(super) fn compile(programs: &[Program]) -> Result<ClosureCode, Error> {
    let mut descriptors = Vec::new();
    let mut cases = String::new();
    let mut continuation = String::new();
    for (program_index, program) in programs.iter().enumerate() {
        for region in closure_proof::regions(program)? {
            let expressions = region
                .guards
                .iter()
                .map(|&word| guard_expression(program, word))
                .collect::<Result<Vec<_>, _>>()?;
            write!(cases, "case {}u:return true", descriptors.len()).unwrap();
            writeln!(
                continuation,
                "if(program=={program_index}u && pc=={}u){{",
                region.header
            )
            .unwrap();
            for (index, expression) in expressions.iter().enumerate() {
                write!(cases, " && ({expression})").unwrap();
                writeln!(continuation, "if(!({expression}))return {index}u;").unwrap();
            }
            cases.push_str(";\n");
            writeln!(continuation, "return {}u;}}", expressions.len()).unwrap();
            descriptors.push([
                u32::try_from(program_index).map_err(|_| Error::BackendCapacity)?,
                region.header as u32,
                u32::from(region.repeat),
                0,
            ]);
        }
    }
    let source = concat!(
        include_str!("metal_resident.metal"),
        include_str!("metal_closure_prepare.metal")
    )
    .replace("// GENERATED_CLOSURE_GUARDS", &cases);
    let continuation_source = concat!(
        include_str!("metal_resident.metal"),
        include_str!("metal_closure_continuation.metal")
    )
    .replace("// GENERATED_RECONSTRUCTION_GUARDS", &continuation);
    Ok(ClosureCode {
        source,
        continuation_source,
        descriptors,
    })
}

pub(super) struct RangeInput<'a> {
    pub resident: &'a ResidentInput,
    pub plan: &'a ExecutionPlan,
    pub data: &'a MappedBuffer,
    pub states: &'a ValueFrontier,
    pub values: &'a StateValues,
}

#[cfg(test)]
pub(super) fn encode(
    device: &Device,
    command: &CommandBufferRef,
    input: ClosureInput<'_>,
    output: &PreparedRanges,
) -> Result<PendingPreparation, Error> {
    let plan = ExecutionPlan::new(input.resident, input.programs)?;
    encode_plan(
        device,
        command,
        RangeInput {
            resident: input.resident,
            plan: &plan,
            data: input.data,
            states: input.states,
            values: input.values,
        },
        output,
    )
}

pub(super) fn encode_plan(
    device: &Device,
    command: &CommandBufferRef,
    input: RangeInput<'_>,
    output: &PreparedRanges,
) -> Result<PendingPreparation, Error> {
    encode_mode(device, command, input, &output.ranges.counts, Some(output))
}

/// Count the exact full-value domains before allocating sibling cells.
pub(super) fn encode_counts(
    device: &Device,
    command: &CommandBufferRef,
    input: RangeInput<'_>,
    counts: &MappedBuffer,
) -> Result<PendingPreparation, Error> {
    encode_mode(device, command, input, counts, None)
}

fn encode_mode(
    device: &Device,
    command: &CommandBufferRef,
    input: RangeInput<'_>,
    counts: &MappedBuffer,
    output: Option<&PreparedRanges>,
) -> Result<PendingPreparation, Error> {
    check_submission()?;
    input.plan.validate(input.resident)?;
    let mut storage = vec![(input.data, 22, 4), (counts, 3, 4)];
    let (domains, entries, cells, generation) = if let Some(output) = output {
        let r = &output.ranges;
        storage.extend([
            (&r.domains, r.domain_capacity, 8),
            (&r.entries, r.entry_capacity, 8),
            (&r.allowed, r.cell_capacity, 4),
            (&output.nodes, r.cell_capacity, 4),
        ]);
        (
            r.domain_capacity,
            r.entry_capacity,
            r.cell_capacity,
            r.generation,
        )
    } else {
        (
            u32::MAX as usize,
            u32::MAX as usize,
            u32::MAX as usize,
            input.states.generation,
        )
    };
    metal_state_values::validate_storage(input.states, input.values, &storage, generation)?;
    let ClosureCode {
        source,
        descriptors: descriptor_values,
        ..
    } = &input.plan.readiness.closure;
    let batch = &input.resident.input.packed;
    let capacity = input.states.capacity;
    let scan = Scan::new(device, capacity)?;
    let parameters = [
        capacity,
        input.values.cursor_capacity,
        input.values.repeat_capacity,
        input.values.capture_capacity,
        domains,
        entries,
        cells,
        batch.nodes.len(),
        batch.children.len(),
        descriptor_values.len(),
        scan.levels(),
        input.data.buffer.length() as usize,
    ]
    .map(|n| u32::try_from(n).map_err(|_| Error::BackendCapacity))
    .into_iter()
    .collect::<Result<Vec<_>, _>>()?;
    let projected = ValueFrontier {
        columns: MappedBuffer::zeroed::<u32>(
            device,
            capacity
                .checked_mul(COLUMNS)
                .ok_or(Error::BackendCapacity)?,
        )?,
        counts: MappedBuffer::zeroed::<u32>(device, 5)?,
        capacity,
        generation: input.states.generation,
    };
    let groups = ValueGroups {
        representatives: MappedBuffer::zeroed::<u32>(device, capacity)?,
        counts: MappedBuffer::zeroed::<u32>(device, 5)?,
        capacity,
        generation: input.states.generation,
    };
    let descriptors = MappedBuffer::from_slice(device, descriptor_values)?;
    let metadata = MappedBuffer::zeroed::<[u32; 4]>(device, capacity)?;
    let regions = MappedBuffer::zeroed::<u32>(device, capacity)?;
    let lengths = MappedBuffer::zeroed::<u32>(device, 5)?;
    let arguments = MappedBuffer::zeroed::<u32>(device, scan.levels() * 3)?;
    let guard_arguments = MappedBuffer::zeroed::<u32>(device, 3)?;
    let work = MappedBuffer::zeroed::<[u32; 4]>(device, capacity)?;
    let status = MappedBuffer::zeroed::<u32>(device, 1)?;
    let launch =
        |name, buffers: &[&BufferRef], indirect: Option<(&BufferRef, u64)>| -> Result<(), Error> {
            dispatch(
                command,
                &pipeline_named(source, name)?,
                name,
                buffers,
                &parameters,
                1,
                indirect,
            );
            Ok(())
        };
    launch(
        "prepare_initialize",
        &[
            &input.data.buffer,
            &input.states.counts.buffer,
            &input.values.counts.buffer,
            &lengths.buffer,
            &arguments.buffer,
            &status.buffer,
            &counts.buffer,
        ],
        None,
    )?;
    let states = Some((&*arguments.buffer, 0));
    launch(
        "prepare_project",
        &[
            &input.data.buffer,
            &input.states.columns.buffer,
            &lengths.buffer,
            &input.values.repeats.buffer,
            &input.values.counts.buffer,
            &descriptors.buffer,
            &projected.columns.buffer,
            &metadata.buffer,
            &regions.buffer,
            &status.buffer,
        ],
        states,
    )?;
    launch(
        "prepare_project_counts",
        &[&lengths.buffer, &status.buffer, &projected.counts.buffer],
        None,
    )?;
    let grouping =
        metal_state_values::encode_groups(device, command, &projected, &groups, input.values, 13)?;
    launch(
        "prepare_count",
        &[
            &groups.counts.buffer,
            &groups.representatives.buffer,
            &metadata.buffer,
            &scan.values.buffer,
        ],
        states,
    )?;
    scan.encode_counted(command, &groups.counts.buffer, &arguments.buffer, 0)?;
    launch(
        "prepare_range_counts",
        &[
            scan.total(),
            &lengths.buffer,
            &groups.counts.buffer,
            &status.buffer,
            &counts.buffer,
            &guard_arguments.buffer,
        ],
        None,
    )?;
    if let Some(output) = output {
        let ranges = &output.ranges;
        launch(
            "prepare_pack",
            &[
                &groups.counts.buffer,
                &groups.representatives.buffer,
                &metadata.buffer,
                &scan.values.buffer,
                &status.buffer,
                &ranges.domains.buffer,
                &ranges.entries.buffer,
                &work.buffer,
            ],
            states,
        )?;
        launch(
            "prepare_guards",
            &[
                &input.data.buffer,
                &metadata.buffer,
                &regions.buffer,
                &ranges.domains.buffer,
                &ranges.counts.buffer,
                &work.buffer,
                &ranges.allowed.buffer,
                &output.nodes.buffer,
            ],
            Some((&guard_arguments.buffer, 0)),
        )?;
    }
    Ok(PendingPreparation {
        command: command.to_owned(),
        grouping,
        _projected: projected,
        _groups: groups,
        _scan: scan,
        _scratch: vec![
            descriptors,
            metadata,
            regions,
            lengths,
            arguments,
            guard_arguments,
            work,
        ],
        status,
    })
}
