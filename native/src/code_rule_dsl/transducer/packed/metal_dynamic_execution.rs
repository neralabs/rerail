//! Generation ownership inside the sole resident executor.
//!
//! The GPU counts a bounded unit before the owner allocates its successor arenas.
//! The owner reads two allocation bounds. It never reconstructs semantic states.
use super::Error;
use super::metal_backend::MappedBuffer;
use super::metal_backend::launch::{commit_checked, wait_checked};
use super::metal_bulk_closure::{ClosureExits, ClosureRanges};
use super::metal_closure_continuation::{self, ClosureResult};
use super::metal_closure_prepare::{self, PreparedRanges, RangeInput};
use super::metal_frontier_seed::{self, CountedSeeds, SeedInput, TASK_SEED_COLUMNS};
use super::metal_grid_plan::{ExecutionPlan, ResidentInput};
use super::metal_initial_range_codegen::WorkClass;
use super::metal_position_order::OrderedFrontier;
use super::metal_ready_batch::{self, PendingReady, ReadyBatch, ReadyInput};
use super::metal_register_execution::read;
use super::metal_state_values::ValueFrontier;
use super::metal_state_values::{
    FrontierRecords, INITIAL_COLUMNS, POSITIONED_COLUMNS, PositionedFrontier, StateValues,
};
use super::metal_successor::{self, ControlValues, SuccessorInput, SuccessorOutput};
use super::metal_successor_prepare::{self, Capacity, PreparedSuccessors};
use super::metal_terminal;
use metal::{CommandBufferRef, Device};
use std::rc::Rc;

#[path = "metal_successor_view.rs"]
mod successor_view;

#[cfg(test)]
thread_local! {static FORCE_SUCCESSOR_VIEWS:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
#[cfg(test)]
pub(super) struct ViewConformanceScope(bool);
#[cfg(test)]
impl ViewConformanceScope {
    pub(super) fn enter() -> Self {
        Self(FORCE_SUCCESSOR_VIEWS.with(|flag| flag.replace(true)))
    }
    pub(super) fn active() -> bool {
        FORCE_SUCCESSOR_VIEWS.with(|flag| flag.get())
    }
}
#[cfg(test)]
impl Drop for ViewConformanceScope {
    fn drop(&mut self) {
        FORCE_SUCCESSOR_VIEWS.with(|flag| flag.set(self.0));
    }
}
fn use_successor_views(device: &Device, bound: &Capacity) -> Result<bool, Error> {
    // Materialization retains raw arenas while it allocates canonical arenas.
    // Completed GPU bounds describe both copies. A narrow frontier can still
    // carry long capture lists and branch positions; row count cannot price it.
    let arena_width = 3 * std::mem::size_of::<[u32; 4]>() + std::mem::size_of::<u32>();
    let row_width = (POSITIONED_COLUMNS + 6 + 1) * std::mem::size_of::<u32>();
    let materialized = bound
        .cells
        .checked_mul(arena_width)
        .and_then(|bytes| {
            bound
                .states
                .checked_mul(row_width)
                .and_then(|rows| bytes.checked_add(rows))
        })
        .and_then(|bytes| bytes.checked_mul(2))
        .ok_or(Error::BackendCapacity)?;
    if bound.states >= 65_536 || materialized > super::metal_backend::available_buffer_bytes(device)
    {
        return Ok(true);
    }
    #[cfg(test)]
    {
        Ok(FORCE_SUCCESSOR_VIEWS.with(|flag| flag.get()))
    }
    #[cfg(not(test))]
    {
        Ok(false)
    }
}

#[cfg(all(test, feature = "metal-tests"))]
#[path = "metal_dynamic_execution_tests.rs"]
mod tests;

#[path = "metal_initial_range.rs"]
mod initial_range;

pub(super) struct DynamicFrontier<const FIELDS: usize = POSITIONED_COLUMNS> {
    states: FrontierRecords<FIELDS>,
    values: StateValues,
    control: ControlValues<FIELDS>,
    batch: ReadyBatch<FIELDS>,
}

#[must_use = "Propagate evaluation failures with the structural results"]
pub(super) struct DynamicOutput {
    pub candidates: super::metal_candidate_batch::CandidateBatch,
    pub errors: Vec<Option<Error>>,
}
/// One checked task array feeds every disjoint initial work class.
#[derive(Clone)]
pub(super) struct InitialSeeds<'a> {
    records: Rc<FrontierRecords<TASK_SEED_COLUMNS>>,
    resident: &'a ResidentInput,
    plan: &'a ExecutionPlan,
    data: &'a MappedBuffer,
}
impl InitialSeeds<'_> {
    pub(super) fn execute(
        self,
        device: &Device,
        queue: &metal::CommandQueueRef,
        class: WorkClass,
    ) -> Result<InitialRangeExecution, Error> {
        initial_range::finish(self, device, queue, class).map(InitialRangeExecution)
    }
}
pub(super) struct InitialRangeExecution(DynamicFrontier);
struct FrontierCapacity {
    states: usize,
    cursors: usize,
    repeats: usize,
    captures: usize,
    digits: usize,
}
impl<const FIELDS: usize> DynamicFrontier<FIELDS> {
    fn allocate(device: &Device, bound: &Capacity, generation: u64) -> Result<Self, Error> {
        Self::allocate_exact(
            device,
            &FrontierCapacity {
                states: bound.states,
                cursors: bound.cells,
                repeats: bound.cells,
                captures: bound.cells,
                digits: bound.cells,
            },
            generation,
        )
    }
    fn allocate_exact(
        device: &Device,
        bound: &FrontierCapacity,
        generation: u64,
    ) -> Result<Self, Error> {
        let capacity = bound.states;
        if ![INITIAL_COLUMNS, POSITIONED_COLUMNS].contains(&FIELDS)
            || (FIELDS == INITIAL_COLUMNS
                && (generation != 0
                    || bound.cursors != 0
                    || bound.repeats != 0
                    || bound.captures != 0
                    || bound.digits != 0))
        {
            return Err(Error::BackendFailure);
        }
        let control_capacity = if FIELDS == INITIAL_COLUMNS {
            0
        } else {
            capacity
        };
        let list_capacity = capacity
            .checked_mul(if FIELDS == INITIAL_COLUMNS { 1 } else { 4 })
            .ok_or(Error::BackendCapacity)?;
        Ok(Self {
            states: FrontierRecords {
                columns: MappedBuffer::zeroed::<[u32; FIELDS]>(device, capacity)?,
                counts: MappedBuffer::zeroed::<u32>(device, 5)?,
                capacity,
                generation,
            },
            values: StateValues {
                cursors: MappedBuffer::zeroed::<[u32; 4]>(device, bound.cursors)?,
                repeats: MappedBuffer::zeroed::<[u32; 4]>(device, bound.repeats)?,
                captures: MappedBuffer::zeroed::<[u32; 4]>(device, bound.captures)?,
                counts: MappedBuffer::zeroed::<u32>(device, 2)?,
                cursor_capacity: bound.cursors,
                repeat_capacity: bound.repeats,
                capture_capacity: bound.captures,
                generation,
            },
            control: ControlValues {
                path_work: MappedBuffer::zeroed::<u32>(device, control_capacity)?,
                spans: MappedBuffer::zeroed::<[u32; 6]>(device, control_capacity)?,
                digits: MappedBuffer::zeroed::<u32>(device, bound.digits)?,
                count: MappedBuffer::zeroed::<u32>(device, 1)?,
                state_capacity: capacity,
                digit_capacity: bound.digits,
                generation,
            },
            batch: ReadyBatch {
                indices: MappedBuffer::zeroed::<u32>(device, list_capacity)?,
                counts: MappedBuffer::zeroed::<u32>(device, 17)?,
                capacity,
                generation,
            },
        })
    }
    fn input<'a>(
        &'a self,
        resident: &'a ResidentInput,
        plan: &'a ExecutionPlan,
        data: &'a MappedBuffer,
    ) -> SuccessorInput<'a, FIELDS> {
        SuccessorInput {
            resident,
            plan,
            data,
            states: &self.states,
            values: &self.values,
            control: &self.control,
            ready: &self.batch,
        }
    }
    fn schedule(
        &self,
        device: &Device,
        command: &CommandBufferRef,
        resident: &ResidentInput,
        plan: &ExecutionPlan,
        data: &MappedBuffer,
    ) -> Result<PendingReady, Error> {
        metal_ready_batch::encode(
            device,
            command,
            ReadyInput {
                resident,
                plan,
                data,
                states: &self.states,
                values: &self.values,
            },
            &self.batch,
        )
    }
}

#[must_use = "Complete the seed, readiness, and prefix checks before allocation"]
pub(super) struct PendingStart {
    seeds: CountedSeeds,
    roots: MappedBuffer,
}
#[cfg(test)]
pub(super) struct InitialExecution {
    frontier: DynamicFrontier<INITIAL_COLUMNS>,
    prepared: Option<(PreparedSuccessors<INITIAL_COLUMNS>, Capacity)>,
}
#[cfg(test)]
impl InitialExecution {
    fn positioned(
        self,
        device: &Device,
        queue: &metal::CommandQueueRef,
        resident: &ResidentInput,
        plan: &ExecutionPlan,
        data: &MappedBuffer,
    ) -> Result<DynamicFrontier, Error> {
        let Self { frontier, prepared } = self;
        if let Some((preparation, bound)) = prepared {
            return advance(
                device,
                queue,
                frontier.input(resident, plan, data),
                preparation,
                &bound,
            );
        }
        // No record exists to reinterpret. Verify the completed producer's
        // canonical empty headers before transferring its zero-length owners.
        metal_successor_prepare::validate_input(&frontier.input(resident, plan, data), &[])?;
        if frontier.states.capacity != 0
            || read::<u32>(&frontier.states.counts, 5)? != [0, 0, 0, 1, 1]
            || read::<u32>(&frontier.values.counts, 2)? != [0, 0]
            || read::<u32>(&frontier.control.count, 1)? != [0]
            || read::<u32>(&frontier.batch.counts, 17)?
                != [0, 0, 0, 0, 0, 1, 1, 0, 1, 1, 0, 1, 1, 0, 1, 1, 0]
        {
            return Err(Error::BackendFailure);
        }
        let DynamicFrontier {
            states,
            values,
            control,
            batch,
        } = frontier;
        Ok(DynamicFrontier {
            states: FrontierRecords {
                columns: states.columns,
                counts: states.counts,
                capacity: states.capacity,
                generation: states.generation,
            },
            values,
            control: ControlValues {
                spans: control.spans,
                path_work: control.path_work,
                digits: control.digits,
                count: control.count,
                state_capacity: control.state_capacity,
                digit_capacity: control.digit_capacity,
                generation: control.generation,
            },
            batch: ReadyBatch {
                indices: batch.indices,
                counts: batch.counts,
                capacity: batch.capacity,
                generation: batch.generation,
            },
        })
    }
}
impl PendingStart {
    pub(super) fn finish_seeds<'a>(
        self,
        device: &Device,
        queue: &metal::CommandQueueRef,
        resident: &'a ResidentInput,
        plan: &'a ExecutionPlan,
        data: &'a MappedBuffer,
    ) -> Result<InitialSeeds<'a>, Error> {
        let count = self.seeds.count()?;
        let records = FrontierRecords::<TASK_SEED_COLUMNS> {
            columns: MappedBuffer::zeroed::<u32>(device, count)?,
            counts: MappedBuffer::zeroed::<u32>(device, 5)?,
            capacity: count,
            generation: 0,
        };
        metal::objc::rc::autoreleasepool(|| {
            let command = queue.new_command_buffer();
            let pending = metal_frontier_seed::encode_scatter(
                command,
                SeedInput {
                    resident,
                    plan,
                    data,
                    roots: &self.roots,
                    spans: 0..resident.input.grid.semantic_span_order.len(),
                },
                &records,
                self.seeds,
            )?;
            commit_checked(command)?;
            wait_checked(command)?;
            pending.check_completed()
        })?;
        Ok(InitialSeeds {
            records: Rc::new(records),
            resident,
            plan,
            data,
        })
    }
    #[cfg(test)]
    pub(super) fn finish(
        self,
        device: &Device,
        queue: &metal::CommandQueueRef,
        resident: &ResidentInput,
        plan: &ExecutionPlan,
        data: &MappedBuffer,
    ) -> Result<InitialExecution, Error> {
        let count = self.seeds.count()?;
        let frontier = DynamicFrontier::<INITIAL_COLUMNS>::allocate(
            device,
            &Capacity {
                states: count,
                cells: 0,
            },
            0,
        )?;
        metal::objc::rc::autoreleasepool(|| {
            let command = queue.new_command_buffer();
            let seeds = metal_frontier_seed::encode_scatter(
                command,
                SeedInput {
                    resident,
                    plan,
                    data,
                    roots: &self.roots,
                    spans: 0..resident.input.grid.semantic_span_order.len(),
                },
                &frontier.states,
                self.seeds,
            )?;
            let ready = frontier.schedule(device, command, resident, plan, data)?;
            commit_checked(command)?;
            wait_checked(command)?;
            seeds.check_completed()?;
            ready.check_completed()?;
            Ok::<_, Error>(())
        })?;
        // Query validation rejects a GREEDY header at PC zero. A bulk-ready
        // initial row therefore indicates an inconsistent scheduling producer.
        if read::<u32>(&frontier.batch.counts, 17)?[1] != 0 {
            return Err(Error::BackendFailure);
        }
        let prepared = if count != 0 {
            let preparation = metal_successor_prepare::prepare(
                device,
                queue,
                &frontier.input(resident, plan, data),
            )?;
            let bound = preparation.check_completed()?;
            Some((preparation, bound))
        } else {
            None
        };
        Ok(InitialExecution { frontier, prepared })
    }
}
pub(super) fn encode_start(
    device: &Device,
    command: &CommandBufferRef,
    resident: &ResidentInput,
    plan: &ExecutionPlan,
    data: &MappedBuffer,
    roots: &MappedBuffer,
) -> Result<PendingStart, Error> {
    let seeds = metal_frontier_seed::encode_counts(
        device,
        command,
        &SeedInput {
            resident,
            plan,
            data,
            roots,
            spans: 0..resident.input.grid.semantic_span_order.len(),
        },
    )?;
    Ok(PendingStart {
        seeds,
        roots: retain(roots),
    })
}

fn advance<const FIELDS: usize>(
    device: &Device,
    queue: &metal::CommandQueueRef,
    input: SuccessorInput<'_, FIELDS>,
    preparation: PreparedSuccessors<FIELDS>,
    bound: &Capacity,
) -> Result<DynamicFrontier, Error> {
    #[cfg(all(test, feature = "metal-tests"))]
    if std::env::var_os("TEMPO_METAL_PROFILE_EXPANSION").is_some() {
        tests::profile_expansion(device, queue, &input, &preparation)?;
    }
    if std::env::var_os("TEMPO_PACKED_BATCH_PROFILE").is_some() {
        let needs = preparation.comparison_needs()?;
        eprintln!(
            "TEMPO_METAL_SUCCESSOR_BOUNDS {}",
            serde_json::json!({
                "generation": input.states.generation,
                "inputColumns": FIELDS,
                "inputStates": input.states.capacity,
                "steps": preparation.parameters[18],
                "stepWords": preparation.parameters[20],
                "outputStates": bound.states,
                "outputCells": bound.cells,
                "activeKeys": needs.active_states,
                "pending": needs.pending,
                "residentBytes": input.data.buffer.length(),
            })
        );
    }
    let generation = input
        .states
        .generation
        .checked_add(1)
        .ok_or(Error::BackendFailure)?;
    let counts = metal::objc::rc::autoreleasepool(|| {
        let command = queue.new_command_buffer();
        let counts = metal_successor::encode_counts(
            device,
            command,
            &input,
            preparation,
            metal_successor::CountLimits {
                states: bound.states,
                cursors: bound.cells,
                repeats: bound.cells,
                captures: bound.cells,
                digits: bound.cells,
            },
        )?;
        commit_checked(command)?;
        wait_checked(command)?;
        counts.check_completed()?;
        Ok::<_, Error>(counts)
    })?;
    let resident = input.resident;
    let plan = input.plan;
    let data = input.data;
    let publication = counts.into_publication(input)?;
    let capacity = publication.capacity();
    let frontier = DynamicFrontier::<POSITIONED_COLUMNS>::allocate_exact(
        device,
        &FrontierCapacity {
            states: capacity.states,
            cursors: capacity.cursors,
            repeats: capacity.repeats,
            captures: capacity.captures,
            digits: capacity.digits,
        },
        generation,
    )?;
    metal::objc::rc::autoreleasepool(|| {
        let command = queue.new_command_buffer();
        let successors = publication.encode(
            command,
            SuccessorOutput {
                states: &frontier.states,
                values: &frontier.values,
                control: &frontier.control,
            },
        )?;
        commit_checked(command)?;
        wait_checked(command)?;
        successors.check_completed()
    })?;
    // Publication has no later reader. Release its command and source storage
    // before readiness allocates task grouping for the completed frontier.
    metal::objc::rc::autoreleasepool(|| {
        let command = queue.new_command_buffer();
        let ready = frontier.schedule(device, command, resident, plan, data)?;
        commit_checked(command)?;
        wait_checked(command)?;
        ready.check_completed()
    })?;
    Ok(frontier)
}

fn ordered(device: &Device, input: &SuccessorInput<'_>) -> Result<OrderedFrontier, Error> {
    let capacity = input.states.capacity;
    Ok(OrderedFrontier {
        states: ValueFrontier {
            columns: MappedBuffer::zeroed::<[u32; 13]>(device, capacity)?,
            counts: MappedBuffer::zeroed::<u32>(device, 5)?,
            capacity,
            generation: input.states.generation,
        },
        originals: MappedBuffer::zeroed::<u32>(device, capacity)?,
    })
}

fn close(
    device: &Device,
    queue: &metal::CommandQueueRef,
    input: &SuccessorInput<'_>,
) -> Result<DynamicFrontier, Error> {
    // Count exact domains without allocating a state-count × node-count arena.
    // The next command revalidates its immutable input; validation never crosses commands.
    let bounds = metal::objc::rc::autoreleasepool(|| {
        let ranked = ordered(device, input)?;
        let counts = MappedBuffer::zeroed::<u32>(device, 3)?;
        let command = queue.new_command_buffer();
        let validation = super::metal_frontier_validation::encode(device, command, input)?;
        super::metal_parallel::encode(
            command,
            &super::metal_backend::pipeline_named(
                include_str!("metal_dynamic_execution.metal"),
                "closure_count_headers",
            )?,
            "closure_count_headers",
            &[
                &input.states.columns.buffer,
                &input.states.counts.buffer,
                &input.ready.indices.buffer,
                &input.ready.counts.buffer,
                &validation.status.buffer,
                &ranked.states.columns.buffer,
                &ranked.states.counts.buffer,
            ],
            &[u32::try_from(input.states.capacity).map_err(|_| Error::BackendCapacity)?],
            input.states.capacity.div_ceil(256).max(1),
            None,
        );

        let preparation = metal_closure_prepare::encode_counts(
            device,
            command,
            RangeInput {
                resident: input.resident,
                plan: input.plan,
                data: input.data,
                states: &ranked.states,
                values: input.values,
            },
            &counts,
        )?;
        commit_checked(command)?;
        wait_checked(command)?;
        validation.check_completed()?;
        preparation.check_completed()?;
        read::<u32>(&counts, 3)
    })?;
    metal::objc::rc::autoreleasepool(|| {
        let generation = input.states.generation;
        let ranked = ordered(device, input)?;
        let [domains, entries, cells] =
            [bounds[0] as usize, bounds[1] as usize, bounds[2] as usize];
        let ranges = PreparedRanges {
            ranges: ClosureRanges {
                domains: MappedBuffer::zeroed::<[u32; 2]>(device, domains)?,
                entries: MappedBuffer::zeroed::<[u32; 2]>(device, entries)?,
                allowed: MappedBuffer::zeroed::<u32>(device, cells)?,
                counts: MappedBuffer::zeroed::<u32>(device, 3)?,
                domain_capacity: domains,
                entry_capacity: entries,
                cell_capacity: cells,
                generation,
            },
            nodes: MappedBuffer::zeroed::<u32>(device, cells)?,
        };
        let exits = ClosureExits {
            exits: MappedBuffer::zeroed::<[u32; 2]>(device, entries)?,
            counts: MappedBuffer::zeroed::<u32>(device, 4)?,
            capacity: entries,
            generation,
        };
        let states = read::<u32>(&input.states.counts, 2)?;
        let values = read::<u32>(&input.values.counts, 2)?;
        let digits = read::<u32>(&input.control.count, 1)?[0] as usize;
        let bound = Capacity {
            states: states[0] as usize,
            cells: digits
                .checked_add(entries)
                .ok_or(Error::BackendCapacity)?
                .max(states[1] as usize)
                .max(values[0] as usize)
                .max(values[1] as usize),
        };
        let output = DynamicFrontier::allocate(
            device,
            &bound,
            generation.checked_add(1).ok_or(Error::BackendFailure)?,
        )?;
        let command = queue.new_command_buffer();
        let pending = metal_closure_continuation::encode_composed(
            device,
            command,
            input,
            ClosureResult {
                ordered: &ranked,
                ranges: &ranges,
                exits: &exits,
            },
            SuccessorOutput {
                states: &output.states,
                values: &output.values,
                control: &output.control,
            },
        )?;
        let ready = output.schedule(device, command, input.resident, input.plan, input.data)?;
        commit_checked(command)?;
        wait_checked(command)?;
        pending.check_completed()?;
        ready.check_completed()?;
        Ok(output)
    })
}

fn retain(buffer: &MappedBuffer) -> MappedBuffer {
    MappedBuffer {
        buffer: buffer.buffer.clone(),
    }
}
fn resolve_predicates(
    device: &Device,
    queue: &metal::CommandQueueRef,
    input: &SuccessorInput<'_>,
    expected: usize,
    programs: &super::metal_capture_predicates::PredicatePrograms<'_>,
) -> Result<DynamicFrontier, Error> {
    let resolved =
        super::metal_inline_predicates::resolve(device, queue, input, expected, programs)?;
    let capacity = resolved.count()?;
    let generation = input
        .states
        .generation
        .checked_add(1)
        .ok_or(Error::BackendFailure)?;
    let frontier = DynamicFrontier {
        states: PositionedFrontier {
            columns: MappedBuffer::zeroed::<[u32; POSITIONED_COLUMNS]>(device, capacity)?,
            counts: MappedBuffer::zeroed::<u32>(device, 5)?,
            capacity,
            generation,
        },
        // Inline predicates only read these immutable arenas. They remain resident.
        values: StateValues {
            cursors: retain(&input.values.cursors),
            repeats: retain(&input.values.repeats),
            captures: retain(&input.values.captures),
            counts: retain(&input.values.counts),
            cursor_capacity: input.values.cursor_capacity,
            repeat_capacity: input.values.repeat_capacity,
            capture_capacity: input.values.capture_capacity,
            generation,
        },
        control: ControlValues {
            spans: MappedBuffer::zeroed::<[u32; 6]>(device, capacity)?,
            path_work: MappedBuffer::zeroed::<u32>(device, capacity)?,
            digits: retain(&input.control.digits),
            count: retain(&input.control.count),
            state_capacity: capacity,
            digit_capacity: input.control.digit_capacity,
            generation,
        },
        batch: ReadyBatch {
            indices: MappedBuffer::zeroed::<[u32; 4]>(device, capacity)?,
            counts: MappedBuffer::zeroed::<u32>(device, 17)?,
            capacity,
            generation,
        },
    };
    let command = queue.new_command_buffer();
    let validation = super::metal_frontier_validation::encode(device, command, input)?;
    resolved.encode_publish(
        command,
        input,
        SuccessorOutput {
            states: &frontier.states,
            values: &frontier.values,
            control: &frontier.control,
        },
        &validation.status,
    )?;
    let ready = frontier.schedule(device, command, input.resident, input.plan, input.data)?;
    commit_checked(command)?;
    wait_checked(command)?;
    validation.check_completed()?;
    ready.check_completed()?;
    resolved.check_completed()?;
    Ok(frontier)
}

/// The sole executor owns every generation and reads only bounded count headers.
pub(super) fn finish_ranges(
    device: &Device,
    queue: &metal::CommandQueueRef,
    resident: &ResidentInput,
    plan: &ExecutionPlan,
    data: &MappedBuffer,
    initial: InitialRangeExecution,
    programs: &super::metal_capture_predicates::PredicatePrograms<'_>,
) -> Result<DynamicOutput, Error> {
    finish_frontier(device, queue, resident, plan, data, initial.0, programs)
}

fn finish_frontier(
    device: &Device,
    queue: &metal::CommandQueueRef,
    resident: &ResidentInput,
    plan: &ExecutionPlan,
    data: &MappedBuffer,
    mut frontier: DynamicFrontier,
    programs: &super::metal_capture_predicates::PredicatePrograms<'_>,
) -> Result<DynamicOutput, Error> {
    loop {
        let counts = read::<u32>(&frontier.batch.counts, 17)?;
        if counts[0] == 0 && counts[1] == 0 && counts[16] == 0 {
            if counts[2] != 0 {
                return Err(Error::BackendFailure);
            }
            break;
        }
        let generation = frontier.states.generation;
        frontier = if counts[16] != 0 {
            resolve_predicates(
                device,
                queue,
                &frontier.input(resident, plan, data),
                counts[16] as usize,
                programs,
            )?
        } else if counts[1] != 0 {
            close(device, queue, &frontier.input(resident, plan, data))?
        } else {
            let preparation = metal_successor_prepare::prepare(
                device,
                queue,
                &frontier.input(resident, plan, data),
            )?;
            let bound = preparation.check_completed()?;
            if use_successor_views(device, &bound)? && !preparation.comparison_needs()?.pending {
                successor_view::execute(
                    successor_view::Context {
                        device,
                        queue,
                        resident,
                        plan,
                        data,
                    },
                    frontier,
                    preparation,
                )?
            } else {
                advance(
                    device,
                    queue,
                    frontier.input(resident, plan, data),
                    preparation,
                    &bound,
                )?
            }
        };
        if frontier.states.generation != generation.checked_add(1).ok_or(Error::BackendFailure)? {
            return Err(Error::BackendFailure);
        }
    }
    if resident.input.grid.evaluation_spans.iter().any(|span| {
        let pg = resident.input.packed.programs[span.program as usize];
        pg.flags & 1 == 0 && pg.capture_count != 0
    }) {
        let normalized = super::metal_terminal_capture_order::normalize(
            device,
            queue,
            &frontier.input(resident, plan, data),
        )?;
        frontier.states = normalized.states;
        frontier.values.captures = normalized.captures;
        frontier.values.capture_capacity = normalized.capture_count;
        // Row identities and continuation positions retain their original meaning.
    }
    metal::objc::rc::autoreleasepool(|| {
        let DynamicFrontier {
            states,
            values,
            control,
            batch,
        } = frontier;
        let prepared = metal_terminal::prepare(
            device,
            queue,
            metal_terminal::TerminalContext { resident, data },
            metal_terminal::TerminalFrame {
                resident,
                plan,
                data,
                states: &states,
                control: &control,
                ready: &batch,
            },
            values,
        )?;
        // Ordering has checked every immutable input. No later consumer reads
        // positioned states, work, branch positions, or readiness.
        drop((states, control, batch));
        let (output, completion) = metal_terminal::execute_prepared(device, queue, prepared)?;
        let counts = read::<u32>(&output.counts, 2)?;
        Ok(DynamicOutput {
            candidates: super::metal_candidate_batch::CandidateBatch {
                results: output.results,
                captures: output.captures,
                result_count: counts[0] as usize,
                capture_count: counts[1] as usize,
            },
            errors: completion.errors,
        })
    })
}

#[cfg(test)]
pub(super) struct ViewTestInput {
    pub private_frames: bool,
    pub states: PositionedFrontier,
    pub values: StateValues,
    pub control: ControlValues,
}
#[cfg(test)]
pub(super) fn execute_view_for_test(
    device: &Device,
    queue: &metal::CommandQueueRef,
    resident: &ResidentInput,
    plan: &ExecutionPlan,
    data: &MappedBuffer,
    input: ViewTestInput,
) -> Result<ViewTestInput, Error> {
    let private_frames = input.private_frames;
    let capacity = input.states.capacity;
    let generation = input.states.generation;
    let frontier = DynamicFrontier {
        states: input.states,
        values: input.values,
        control: input.control,
        batch: ReadyBatch {
            indices: MappedBuffer::zeroed::<[u32; 4]>(device, capacity)?,
            counts: MappedBuffer::zeroed::<u32>(device, 17)?,
            capacity,
            generation,
        },
    };
    metal::objc::rc::autoreleasepool(|| {
        let command = queue.new_command_buffer();
        let ready = frontier.schedule(device, command, resident, plan, data)?;
        commit_checked(command)?;
        wait_checked(command)?;
        ready.check_completed()
    })?;
    let preparation = if private_frames {
        metal_successor_prepare::prepare_register(
            device,
            queue,
            &frontier.input(resident, plan, data),
        )?
    } else {
        metal_successor_prepare::prepare(device, queue, &frontier.input(resident, plan, data))?
    };
    let output = successor_view::execute(
        successor_view::Context {
            device,
            queue,
            resident,
            plan,
            data,
        },
        frontier,
        preparation,
    )?;
    Ok(ViewTestInput {
        private_frames,
        states: output.states,
        values: output.values,
        control: output.control,
    })
}

#[cfg(test)]
pub(super) struct CaptureCopyFixture {
    pub saved: Vec<[u32; 4]>,
    pub added: Vec<[u32; 4]>,
}
#[cfg(test)]
pub(super) fn copy_captures_for_test(
    device: &Device,
    queue: &metal::CommandQueueRef,
    resident: &ResidentInput,
    plan: &ExecutionPlan,
    data: &MappedBuffer,
    fixtures: &[CaptureCopyFixture],
) -> Result<Vec<[u32; 4]>, Error> {
    successor_view::copy_captures_for_test(
        successor_view::Context {
            device,
            queue,
            resident,
            plan,
            data,
        },
        fixtures,
    )
}
