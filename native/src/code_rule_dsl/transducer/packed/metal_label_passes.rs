//! A lowered matcher's label passes on Metal in one command buffer (06-first-slice S2, O23).
//!
//! The labels live in a prefix of the batch's supertype buffer, which `has_supertype` reads as
//! derived classes: element 0 is {label count, words per label}, then one bitset per label over the
//! batch's global nodes. Passes run a dependency level at a time, so the launches grow with the
//! depth of the label graph, not its size (the evaluation's launch budget, `metal_launch_budget`):
//! one interpreter `run_batch` with every program of the level on one shared workspace (its
//! scheduler image copied in by a blit), `label_sink` turning their results into scratch bits or
//! counts, `label_derive` for complements, count bounds and constraints, and `label_settle` moving
//! the level's scratch into its labels. The host waits only to grow the workspace, past a
//! segment's upload budget, and once per round of a recursive group (whose end is a changed flag).

use super::super::metal_batch_abi::MetalSupertype;
use super::super::native_ir::ArenaKind;
use super::*;
use metal::{Buffer, CommandBuffer};

/// A pass writes the label `target` (an index, not a class); labels it reads are its classes.
pub(in crate::code_rule_dsl) enum Pass<'a> {
    /// A program's roots or captured nodes.
    Program {
        target: u32,
        program: &'a Program,
        roots: Vec<Vec<u32>>,
        captured: bool,
    },
    /// The nodes with between `min` and `max` of a program's outputs, by root or captured node,
    /// tallied in count slot `slot`.
    Count {
        target: u32,
        slot: u32,
        program: &'a Program,
        roots: Vec<Vec<u32>>,
        captured: bool,
        min: u32,
        max: u32,
    },
    Complement { target: u32, of: u32 },
    /// A constrained pattern's matches: (global root, its constrained captures' (global node,
    /// label)), from the host pattern matcher as `demand`'s pattern producers.
    Constrained { target: u32, rows: Vec<(u32, Vec<(u32, u32)>)> },
    Empty { target: u32 },
}

pub(in crate::code_rule_dsl) struct Plan<'a> {
    pub(in crate::code_rule_dsl) labels: usize,
    /// Count slots (`Pass::Count::slot`).
    pub(in crate::code_rule_dsl) counts: usize,
    /// Labels known before the passes (pattern leaves), `labels` × `words` words.
    pub(in crate::code_rule_dsl) initial: Vec<u32>,
    /// Groups in order, each as its dependency levels: a level's passes read only earlier
    /// levels' labels. A recursive group repeats until no label of it changes.
    pub(in crate::code_rule_dsl) steps: Vec<(bool, Vec<Vec<Pass<'a>>>)>,
}

#[derive(Debug, Default)]
pub(in crate::code_rule_dsl) struct Outcome {
    /// Every label's words after the passes.
    pub(in crate::code_rule_dsl) labels: Vec<u32>,
    pub(in crate::code_rule_dsl) words: usize,
    pub(in crate::code_rule_dsl) command_buffers: usize,
    pub(in crate::code_rule_dsl) dispatches: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Shape {
    node_count: u32,
    words: u32,
    result_arena: u32,
    task_count: u32,
    item_count: u32,
    target_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Op {
    kind: u32,
    target: u32,
    of: u32,
    slot: u32,
    minimum: u32,
    maximum: u32,
}

const SOURCE: &str = include_str!("metal_label_passes.metal");
const KERNELS: [&str; 3] = ["label_sink", "label_derive", "label_settle"];
/// Bytes of per-level uploads one command buffer carries before the host waits for it.
const SEGMENT_BYTES: usize = 64 << 20;

fn pipelines() -> Result<Vec<ComputePipelineState>, Error> {
    static PIPELINES: Mutex<Option<Vec<ComputePipelineState>>> = Mutex::new(None);
    let mut cached = PIPELINES.lock().map_err(|_| Error::BackendFailure)?;
    if let Some(pipelines) = &*cached {
        return Ok(pipelines.clone());
    }
    let pipelines = KERNELS
        .iter()
        .map(|name| pipeline_named(SOURCE, name))
        .collect::<Result<Vec<_>, _>>()?;
    *cached = Some(pipelines.clone());
    Ok(pipelines)
}

fn widest(left: WorkspaceTotals, right: WorkspaceTotals) -> WorkspaceTotals {
    WorkspaceTotals {
        pending: left.pending.max(right.pending),
        captures: left.captures.max(right.captures),
        cursors: left.cursors.max(right.cursors),
        repeats: left.repeats.max(right.repeats),
        query_states: left.query_states.max(right.query_states),
        query_members: left.query_members.max(right.query_members),
        queued_states: left.queued_states.max(right.queued_states),
        greedy_entries: left.greedy_entries.max(right.greedy_entries),
        value_states: left.value_states.max(right.value_states),
        candidates: left.candidates.max(right.candidates),
        results: left.results.max(right.results),
        output_bytes: left.output_bytes.max(right.output_bytes),
        output_captures: left.output_captures.max(right.output_captures),
        properties: left.properties.max(right.properties),
    }
}

fn bytes_of<T: Copy>(values: &[T]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), std::mem::size_of_val(values)) }
}

/// The batch's tape buffers, shared by every level.
struct Tapes {
    files: MappedBuffer,
    nodes: MappedBuffer,
    children: MappedBuffer,
    source: MappedBuffer,
    line_starts: MappedBuffer,
    /// The label prefix, then the grammar's (node, class) pairs.
    supertypes: MappedBuffer,
}

/// A workspace lease with what it was sized for.
struct Workspace {
    lease: MetalWorkspaceLease,
    totals: WorkspaceTotals,
    scheduler_bytes: u32,
    output_bytes: u32,
    tasks: usize,
}

struct Runner<'a> {
    device: &'a Device,
    queue: metal::CommandQueue,
    engine: Engine,
    kernels: Vec<ComputePipelineState>,
    files: &'a [TapeDoc],
    tapes: Tapes,
    words: usize,
    nodes: usize,
    scratch: MappedBuffer,
    counts: MappedBuffer,
    flags: MappedBuffer,
    workspace: Option<Workspace>,
    command: CommandBuffer,
    retained: Vec<MappedBuffer>,
    programs: Vec<Arc<ProgramBuffers>>,
    segment_bytes: usize,
    encoded: bool,
    outcome: Outcome,
}

pub(in crate::code_rule_dsl) fn run(plan: &Plan<'_>, files: &[TapeDoc]) -> Result<Outcome, Error> {
    let _evaluation = super::super::metal_backend::metal_execution_guard()?;
    let device = METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
    let nodes = files.iter().map(TapeDoc::transducer_len).sum::<usize>();
    let words = nodes.div_ceil(32).max(1);
    if plan.initial.len() != plan.labels * words {
        return Err(Error::InvalidOperand(0));
    }
    let queue = device.new_command_queue();
    queue.set_label("Rerail label passes");
    let command = queue.new_command_buffer().to_owned();
    command.set_label("Rerail label passes");
    let mut runner = Runner {
        device,
        engine: compile_interpreter_cached()?,
        kernels: pipelines()?,
        command,
        queue,
        files,
        tapes: tapes(device, files, plan, words)?,
        words,
        nodes,
        scratch: MappedBuffer::zeroed::<u32>(device, plan.labels * words)?,
        counts: MappedBuffer::zeroed::<u32>(device, (plan.counts * nodes).max(1))?,
        flags: MappedBuffer::zeroed::<u32>(device, 2)?,
        workspace: None,
        retained: Vec::new(),
        programs: Vec::new(),
        segment_bytes: 0,
        encoded: false,
        outcome: Outcome {
            words,
            ..Outcome::default()
        },
    };
    for (recursive, levels) in &plan.steps {
        if !recursive {
            for level in levels {
                runner.level(level)?;
            }
            continue;
        }
        // Rounds as the CPU's: every member once per round, until a round changes nothing.
        let mut settled = false;
        for _round in 0..=nodes + 1 {
            runner.flush()?;
            runner.set_flag(1, 0);
            for level in levels {
                runner.level(level)?;
            }
            runner.flush()?;
            if runner.flag(1) == 0 {
                settled = true;
                break;
            }
        }
        if !settled {
            return Err(Error::WorkLimit);
        }
    }
    runner.flush()?;
    if runner.flag(0) != 0 {
        return Err(Error::BackendFailure);
    }
    runner.outcome.labels = unsafe {
        std::slice::from_raw_parts(
            runner.tapes.supertypes.buffer.contents().cast::<u32>().add(2),
            plan.labels * words,
        )
        .to_vec()
    };
    Ok(runner.outcome)
}

/// The tape buffers, from a batch of the files with no tasks; the label prefix holds `initial`.
fn tapes(device: &Device, files: &[TapeDoc], plan: &Plan<'_>, words: usize) -> Result<Tapes, Error> {
    let program = Program::new(vec![super::super::word(EMIT, 0, 0)], vec![b"tape".to_vec()], Vec::new(), vec![0], Vec::new())?;
    let anchors = vec![vec![Vec::new()]; files.len()];
    let batch = PackedBatch::new(std::slice::from_ref(&program), files, &anchors)?;
    let prefix = 1 + (plan.labels * words).div_ceil(2);
    let mut supertypes = Vec::with_capacity(prefix + batch.supertypes.len());
    supertypes.push(MetalSupertype {
        node: plan.labels as u32,
        kind: words as u32,
    });
    let mut initial = plan.initial.clone();
    initial.resize((prefix - 1) * 2, 0);
    supertypes.extend(initial.chunks(2).map(|pair| MetalSupertype { node: pair[0], kind: pair[1] }));
    supertypes.extend(batch.supertypes.iter().map(|pair| MetalSupertype { node: pair.node, kind: pair.kind }));
    let mut batch_files = batch.files.clone();
    for file in &mut batch_files {
        file.supertype_start += prefix as u32;
    }
    Ok(Tapes {
        files: MappedBuffer::from_slice(device, &batch_files)?,
        nodes: MappedBuffer::from_slice(device, &batch.nodes)?,
        children: MappedBuffer::from_slice(device, &batch.children)?,
        source: MappedBuffer::from_slice(device, &batch.source)?,
        line_starts: MappedBuffer::from_slice(device, &batch.line_starts)?,
        supertypes: MappedBuffer::from_slice(device, &supertypes)?,
    })
}

impl Runner<'_> {
    fn set_flag(&self, index: usize, value: u32) {
        unsafe { *self.flags.buffer.contents().cast::<u32>().add(index) = value };
    }

    fn flag(&self, index: usize) -> u32 {
        unsafe { *self.flags.buffer.contents().cast::<u32>().add(index) }
    }

    /// Commits what is encoded and waits for it.
    fn flush(&mut self) -> Result<(), Error> {
        if !self.encoded {
            return Ok(());
        }
        commit_checked(&self.command)?;
        wait_checked(&self.command)?;
        self.outcome.command_buffers += 1;
        self.retained.clear();
        self.programs.clear();
        self.segment_bytes = 0;
        self.encoded = false;
        self.command = self.queue.new_command_buffer().to_owned();
        self.command.set_label("Rerail label passes");
        Ok(())
    }

    fn upload<T: Copy>(&mut self, values: &[T]) -> Result<Buffer, Error> {
        let buffer = if values.is_empty() {
            MappedBuffer::zeroed::<T>(self.device, 1)?
        } else {
            MappedBuffer::from_slice(self.device, values)?
        };
        self.segment_bytes += buffer.buffer.length() as usize;
        let handle = buffer.buffer.clone();
        self.retained.push(buffer);
        Ok(handle)
    }

    /// One label kernel over `threads`, its buffers bound in order and `shape` after them.
    fn kernel(&mut self, pipeline: usize, buffers: &[(&Buffer, u64)], shape: Shape, threads: usize) {
        if threads == 0 {
            return;
        }
        let encoder = self.command.new_compute_command_encoder();
        encoder.set_label(KERNELS[pipeline]);
        encoder.set_compute_pipeline_state(&self.kernels[pipeline]);
        for (index, (buffer, offset)) in buffers.iter().enumerate() {
            encoder.set_buffer(index as u64, Some(buffer), *offset);
        }
        encoder.set_bytes(
            buffers.len() as u64,
            std::mem::size_of::<Shape>() as u64,
            std::ptr::from_ref(&shape).cast(),
        );
        let width = self.kernels[pipeline].thread_execution_width().max(1);
        encoder.checked_dispatch_threads(
            &self.command,
            MTLSize::new(threads as u64, 1, 1),
            MTLSize::new(width.min(threads as u64), 1, 1),
        );
        encoder.end_encoding();
        self.encoded = true;
        self.outcome.dispatches += 1;
    }

    fn shape(&self) -> Shape {
        Shape {
            node_count: self.nodes as u32,
            words: self.words as u32,
            ..Shape::default()
        }
    }

    /// One dependency level: its programs in one dispatch, their results sunk, the derived
    /// labels, then every target settled.
    fn level(&mut self, passes: &[Pass<'_>]) -> Result<(), Error> {
        let mut programs = Vec::new();
        let mut ops = Vec::new();
        let mut items = Vec::new();
        let mut rows = Vec::new();
        let mut constraints = Vec::new();
        let mut targets = Vec::new();
        for pass in passes {
            match pass {
                Pass::Program {
                    target,
                    program,
                    roots,
                    captured,
                } => programs.push((*program, roots, if *captured { 1 } else { 0 }, *target)),
                Pass::Count {
                    target,
                    slot,
                    program,
                    roots,
                    captured,
                    min,
                    max,
                } => {
                    programs.push((*program, roots, if *captured { 3 } else { 2 }, *slot));
                    items.extend((0..self.words as u32).map(|index| [ops.len() as u32, index]));
                    ops.push(Op { kind: 2, target: *target, slot: *slot, minimum: *min, maximum: *max, of: 0 });
                }
                Pass::Complement { target, of } => {
                    items.extend((0..self.words as u32).map(|index| [ops.len() as u32, index]));
                    ops.push(Op { kind: 1, target: *target, of: *of, ..Op::default() });
                }
                Pass::Constrained { target, rows: matches } => {
                    for (root, checks) in matches {
                        items.push([ops.len() as u32, (rows.len() / 3) as u32]);
                        rows.extend([*root, (constraints.len() / 2) as u32, checks.len() as u32]);
                        for (node, label) in checks {
                            constraints.extend([*node, *label]);
                        }
                    }
                    ops.push(Op { kind: 3, target: *target, ..Op::default() });
                }
                Pass::Empty { .. } => {}
            }
            targets.push(match pass {
                Pass::Program { target, .. }
                | Pass::Count { target, .. }
                | Pass::Complement { target, .. }
                | Pass::Constrained { target, .. }
                | Pass::Empty { target } => *target,
            });
        }
        if !programs.is_empty() {
            self.programs(&programs)?;
        }
        let labels = self.tapes.supertypes.buffer.clone();
        let (scratch, counts, flags) =
            (self.scratch.buffer.clone(), self.counts.buffer.clone(), self.flags.buffer.clone());
        if !items.is_empty() {
            let (ops, items, rows, constraints) =
                (self.upload(&ops)?, self.upload(&items)?, self.upload(&rows)?, self.upload(&constraints)?);
            let shape = Shape {
                item_count: (items.length() / 8) as u32,
                ..self.shape()
            };
            let count = shape.item_count as usize;
            self.kernel(
                1,
                &[(&labels, 8), (&scratch, 0), (&counts, 0), (&ops, 0), (&items, 0), (&rows, 0), (&constraints, 0)],
                shape,
                count,
            );
        }
        let target_count = targets.len();
        let targets = self.upload(&targets)?;
        let shape = Shape {
            target_count: target_count as u32,
            ..self.shape()
        };
        self.kernel(2, &[(&labels, 8), (&scratch, 0), (&flags, 0), (&targets, 0)], shape, target_count * self.words);
        if self.segment_bytes > SEGMENT_BYTES {
            self.flush()?;
        }
        Ok(())
    }

    /// The level's programs as one interpreter batch over their roots, then `label_sink` routing
    /// each result by its task's program: (program, roots per file, mode, slot).
    fn programs(&mut self, passes: &[(&Program, &Vec<Vec<u32>>, u32, u32)]) -> Result<(), Error> {
        let programs = passes.iter().map(|(program, ..)| (*program).clone()).collect::<Vec<_>>();
        let anchors = (0..self.files.len())
            .map(|file| passes.iter().map(|(_, roots, ..)| roots[file].clone()).collect())
            .collect::<Vec<Vec<Vec<u32>>>>();
        let mut batch = PackedBatch::new(&programs, self.files, &anchors)?;
        if batch.tasks.is_empty() {
            return Ok(());
        }
        let totals = batch.assign_workspace_with_result_limits_and_lanes(&programs, None, EXECUTION_LANES)?;
        let (scheduler_bytes, output_bytes, tasks) = (
            batch.header.scheduler_byte_count,
            batch.header.workspace_byte_count,
            batch.tasks.len(),
        );
        let fits = self
            .workspace
            .as_ref()
            .is_some_and(|workspace| workspace.lease.fits(totals, scheduler_bytes, output_bytes, tasks));
        if !fits {
            self.flush()?;
            let (totals, scheduler_bytes, output_bytes, tasks) = match self.workspace.take() {
                Some(old) => (
                    widest(old.totals, totals),
                    old.scheduler_bytes.max(scheduler_bytes),
                    old.output_bytes.max(output_bytes),
                    old.tasks.max(tasks),
                ),
                None => (totals, scheduler_bytes, output_bytes, tasks),
            };
            let mut lease = MetalWorkspaceLease::acquire(self.device, totals, scheduler_bytes, output_bytes, tasks)?;
            // Each level advances the generation itself.
            lease.generation_span = 0;
            self.workspace = Some(Workspace {
                lease,
                totals,
                scheduler_bytes,
                output_bytes,
                tasks,
            });
        }
        let lease = &mut self.workspace.as_mut().expect("a workspace").lease;
        let generation = lease.generation;
        let span = u32::try_from(tasks + 1).map_err(|_| Error::BackendCapacity)? * 2;
        lease.generation = lease.generation.checked_add(span).ok_or(Error::BackendCapacity)?;
        for evaluation in batch.evaluations.iter_mut().chain(&mut batch.evaluation_lanes) {
            evaluation.generation = generation;
        }
        let mut image = bytes_of(std::slice::from_ref(&batch.global_arenas)).to_vec();
        image.extend_from_slice(bytes_of(&batch.evaluations));
        image.extend_from_slice(bytes_of(&batch.evaluation_lanes));
        let image_length = image.len() as u64;
        let image = self.upload(&image)?;
        let task_buffer = self.upload(&batch.tasks)?;
        let header_buffer = self.upload(&[batch.header])?;
        let mut routes = vec![[0_u32; 2]; tasks];
        for task in &batch.tasks {
            let (_, _, mode, slot) = passes[task.program as usize];
            routes[task.batch_ordinal as usize] = [mode, slot];
        }
        let routes = self.upload(&routes)?;
        let program_buffers = self.engine.program_buffers(self.device, &batch)?;
        self.programs.push(Arc::clone(&program_buffers));
        let lease = &self.workspace.as_ref().expect("a workspace").lease;
        let tapes = &self.tapes;
        let blit = self.command.new_blit_command_encoder();
        blit.set_label("Rerail label pass scheduler image");
        blit.copy_from_buffer(&image, 0, &lease.scheduler.buffer, 0, image_length);
        blit.end_encoding();
        let encoder = self.command.new_compute_command_encoder();
        encoder.set_label("Rerail label pass programs");
        encoder.set_compute_pipeline_state(self.engine.pipeline());
        for (index, buffer) in [
            &task_buffer,
            &header_buffer,
            &tapes.files.buffer,
            &tapes.nodes.buffer,
            &program_buffers.programs.buffer,
            &program_buffers.words.buffer,
            &program_buffers.longs.buffer,
            &tapes.children.buffer,
            &tapes.supertypes.buffer,
            &lease.pending.buffer,
            &lease.captures.buffer,
            &lease.cursors.buffer,
            &lease.repeats.buffer,
            &lease.scheduler.buffer,
            &lease.results.buffer,
            &lease.statuses.buffer,
            &program_buffers.values.buffer,
            &program_buffers.strings.buffer,
            &program_buffers.nfa_states.buffer,
            &program_buffers.nfa_data.buffer,
            &program_buffers.nfa_ranges.buffer,
            &program_buffers.lua_instructions.buffer,
            &program_buffers.lua_members.buffer,
            &program_buffers.assertions.buffer,
            &program_buffers.assertion_data.buffer,
            &program_buffers.string_bytes.buffer,
            &tapes.source.buffer,
            &lease.value_states.buffer,
            &tapes.line_starts.buffer,
            &lease.final_captures.buffer,
            &lease.output_storage.buffer,
        ]
        .into_iter()
        .enumerate()
        {
            encoder.set_buffer(index as u64, Some(buffer), 0);
        }
        let lanes = batch.evaluation_lanes.len() as u64;
        let width = self.engine.pipeline().thread_execution_width().max(1);
        encoder.checked_dispatch_threads(&self.command, MTLSize::new(lanes, 1, 1), MTLSize::new(width.min(lanes), 1, 1));
        encoder.end_encoding();
        self.encoded = true;
        self.outcome.dispatches += 1;
        let (scheduler, results, captures, statuses) = (
            lease.scheduler.buffer.clone(),
            lease.results.buffer.clone(),
            lease.final_captures.buffer.clone(),
            lease.statuses.buffer.clone(),
        );
        let (scratch, counts, flags) =
            (self.scratch.buffer.clone(), self.counts.buffer.clone(), self.flags.buffer.clone());
        let capacity = batch.global_arenas.arenas[ArenaKind::Result as usize].capacity as usize;
        let shape = Shape {
            result_arena: ArenaKind::Result as u32,
            task_count: tasks as u32,
            ..self.shape()
        };
        self.kernel(
            0,
            &[
                (&scheduler, 0),
                (&results, 0),
                (&captures, 0),
                (&statuses, 0),
                (&routes, 0),
                (&scratch, 0),
                (&counts, 0),
                (&flags, 0),
            ],
            shape,
            capacity.max(tasks),
        );
        Ok(())
    }
}
