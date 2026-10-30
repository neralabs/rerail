//! A lowered matcher's label passes on Metal in one command buffer (06-first-slice S2, O23).
//!
//! The labels live in a prefix of the batch's supertype buffer, which `has_supertype` reads as
//! derived classes: element 0 is {label count, words per label}, then one bitset per label over the
//! batch's global nodes. Passes run a dependency level at a time, so the launches grow with the
//! depth of the label graph, not its size (the evaluation's launch budget, `metal_launch_budget`):
//! one interpreter `run_batch` with every program of the level on one shared workspace (its
//! scheduler image copied in by a blit), `label_sink` turning their results into scratch bits or
//! counts, the clause's pattern kernel (`patterns`), and `label_settle` writing each target:
//! from scratch, or as a complement or count bound computed per word. A recursive group's rounds
//! are encoded too and skipped on the GPU once settled (`label_round`), so a clause is one
//! command buffer.

#[path = "metal_label_patterns.rs"]
mod patterns;

use super::super::PackedPattern;
use super::super::metal_batch_abi::MetalSupertype;
use super::super::native_ir::ArenaKind;
use super::*;
use super::super::metal_batch_abi::MetalEvaluation;
use metal::{Buffer, CommandBuffer};
use std::collections::HashMap;

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
    /// The global `roots` where `plan.patterns[pattern]` matches and each named constraint's
    /// single-node capture has its label (an unbound or multi-node capture is not checked).
    Pattern {
        target: u32,
        pattern: usize,
        roots: Vec<u32>,
        constraints: Vec<(String, u32)>,
    },
    Empty { target: u32 },
}

pub(in crate::code_rule_dsl) struct Plan<'a> {
    pub(in crate::code_rule_dsl) labels: usize,
    /// Count slots (`Pass::Count::slot`).
    pub(in crate::code_rule_dsl) counts: usize,
    /// The clause's ast-grep patterns, matched on the GPU (`patterns`).
    pub(in crate::code_rule_dsl) patterns: Vec<&'a PackedPattern>,
    /// Groups in order, each as its dependency levels: a level's passes read only earlier
    /// levels' labels. A recursive group repeats until no label of it changes.
    pub(in crate::code_rule_dsl) steps: Vec<(bool, Vec<Vec<Pass<'a>>>)>,
    /// Whether a recursive group's programs step to siblings (`NEXT`, `FOLLOWING`), so that
    /// its rounds follow sibling runs as well as depth.
    pub(in crate::code_rule_dsl) siblings: bool,
}

#[derive(Debug, Default)]
pub(in crate::code_rule_dsl) struct Outcome {
    /// Every label's words after the passes.
    pub(in crate::code_rule_dsl) labels: Vec<u32>,
    pub(in crate::code_rule_dsl) words: usize,
    pub(in crate::code_rule_dsl) command_buffers: usize,
    pub(in crate::code_rule_dsl) dispatches: usize,
    /// Why the host waited before the end: workspace growth, a recursive group that needed more
    /// rounds than were encoded (the whole clause again), the upload budget.
    pub(in crate::code_rule_dsl) waits: [usize; 3],
    /// Rounds encoded per recursive group.
    pub(in crate::code_rule_dsl) rounds: usize,
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
    round: u32,
    finish: u32,
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
const KERNELS: [&str; 3] = ["label_sink", "label_settle", "label_round"];
/// The launches a clause's encoding aims to stay within, below the evaluation's budget
/// (`metal_launch_budget`); a recursive group's rounds are encoded from what is left.
const LAUNCHES: usize = 560;
/// At most this many rounds of a recursive group are encoded in one go.
const MOST_ROUNDS: usize = 64;
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

/// The clause's pattern kernel, compiled once per distinct source.
fn pattern_pipeline(patterns: &[&PackedPattern]) -> Result<Option<(ComputePipelineState, Vec<Vec<String>>)>, Error> {
    static PIPELINES: LazyLock<Mutex<HashMap<String, ComputePipelineState>>> = LazyLock::new(Default::default);
    if patterns.is_empty() {
        return Ok(None);
    }
    let (source, names) = patterns::source(patterns);
    let mut cached = PIPELINES.lock().map_err(|_| Error::BackendFailure)?;
    if let Some(pipeline) = cached.get(&source) {
        return Ok(Some((pipeline.clone(), names)));
    }
    let pipeline = pipeline_named(&source, patterns::KERNEL)?;
    cached.insert(source, pipeline.clone());
    Ok(Some((pipeline, names)))
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
    /// Elements in `supertypes`: a label batch's header says so, which `run_batch` reads as
    /// "the prefix head is there".
    supertype_count: u32,
}

/// A level's programs as an interpreter batch, built once: every round encodes the same tasks
/// with a fresh scheduler image.
struct Prepared {
    tasks: MappedBuffer,
    header: MappedBuffer,
    routes: MappedBuffer,
    program: Arc<ProgramBuffers>,
    arenas: MetalGlobalArenas,
    evaluations: Vec<MetalEvaluation>,
    lanes: Vec<MetalEvaluation>,
    task_count: usize,
    result_capacity: usize,
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
    /// The clause's pattern kernel and each pattern's metavariables in index order.
    patterns: Option<(ComputePipelineState, Vec<Vec<String>>)>,
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
    /// By the level's address in the plan; `None` for a level with no tasks.
    prepared: HashMap<usize, Option<Prepared>>,
    segment_bytes: usize,
    encoded: bool,
    outcome: Outcome,
}

pub(in crate::code_rule_dsl) fn run(plan: &Plan<'_>, files: &[TapeDoc]) -> Result<Outcome, Error> {
    let _evaluation = super::super::metal_backend::metal_execution_guard()?;
    let device = METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
    // A level's launches: the programs' dispatch and sink, the derived labels, the settle.
    let launches = |level: &Vec<Pass<'_>>| {
        let programs = level.iter().any(|pass| matches!(pass, Pass::Program { .. } | Pass::Count { .. }));
        let patterns = level.iter().any(|pass| matches!(pass, Pass::Pattern { .. }));
        2 * usize::from(programs) + usize::from(patterns) + 1
    };
    let fixed = plan
        .steps
        .iter()
        .filter(|(recursive, _)| !recursive)
        .flat_map(|(_, levels)| levels)
        .map(launches)
        .sum::<usize>();
    // Each round: its levels and its boundary; each group one finishing boundary.
    let per_round = plan
        .steps
        .iter()
        .filter(|(recursive, _)| *recursive)
        .map(|(_, levels)| levels.iter().map(launches).sum::<usize>() + 1)
        .sum::<usize>();
    let groups = plan.steps.iter().filter(|(recursive, _)| *recursive).count();
    // A recursive group walks one tree edge per round (`matcher::lower` admits recursion only
    // through walks), so the tapes' deepest node or longest run of siblings, plus a round to
    // see nothing change, is the expected count. It is not a proof: a group still changing in
    // its last round is run again with more.
    let expected = files
        .iter()
        .map(|tape| {
            let mut depth = vec![0_usize; tape.transducer_len()];
            let mut longest = 0;
            for node in 0..tape.transducer_len() as u32 {
                if let Some(parent) = tape.transducer_parent(node) {
                    depth[node as usize] = depth[parent as usize] + 1;
                }
                longest = longest.max(depth[node as usize]);
                if plan.siblings {
                    longest = longest.max(tape.transducer_children(node).count());
                }
            }
            longest
        })
        .max()
        .unwrap_or(0)
        + 2;
    // What the budget leaves for rounds, after `used` launches.
    let affordable = |used: usize| {
        if per_round == 0 {
            0
        } else {
            (LAUNCHES.saturating_sub(used + fixed + groups) / per_round).clamp(2, MOST_ROUNDS)
        }
    };
    // Every round the tapes' shape expects, so that a clause is one command buffer; fewer first
    // would save the launches of skipped rounds but cost a rerun where a group needs more.
    let mut rounds = affordable(0).min(expected);
    let nodes = files.iter().map(TapeDoc::transducer_len).sum::<usize>();
    let (mut retries, mut used, mut buffers) = (0, 0, 0);
    loop {
        let mut outcome = encode(device, plan, files, rounds)?;
        used += outcome.dispatches;
        buffers += outcome.command_buffers;
        // A group still changing in its last encoded round: everything after it read unsettled
        // labels, so the clause runs again with the rounds the tapes' shape expects and the
        // budget affords, then more, up to the CPU's bound.
        if outcome.waits[1] == 0 {
            outcome.waits[1] = retries;
            outcome.dispatches = used;
            outcome.command_buffers = buffers;
            return Ok(outcome);
        }
        if rounds > nodes + 1 {
            return Err(Error::WorkLimit);
        }
        retries += 1;
        rounds = (rounds * 2).max(affordable(used).min(expected));
    }
}

/// The clause's passes in one command buffer, each recursive group as `rounds` rounds.
fn encode(device: &Device, plan: &Plan<'_>, files: &[TapeDoc], rounds: usize) -> Result<Outcome, Error> {
    let nodes = files.iter().map(TapeDoc::transducer_len).sum::<usize>();
    let words = nodes.div_ceil(32).max(1);
    let queue = device.new_command_queue();
    queue.set_label("Rerail label passes");
    let command = queue.new_command_buffer().to_owned();
    command.set_label("Rerail label passes");
    let mut runner = Runner {
        device,
        engine: compile_interpreter_cached()?,
        kernels: pipelines()?,
        patterns: pattern_pipeline(&plan.patterns)?,
        command,
        queue,
        files,
        tapes: tapes(device, files, plan, words)?,
        words,
        nodes,
        scratch: MappedBuffer::zeroed::<u32>(device, plan.labels * words)?,
        counts: MappedBuffer::zeroed::<u32>(device, (plan.counts * nodes).max(1))?,
        flags: MappedBuffer::zeroed::<u32>(device, 4)?,
        workspace: None,
        retained: Vec::new(),
        programs: Vec::new(),
        prepared: HashMap::new(),
        segment_bytes: 0,
        encoded: false,
        outcome: Outcome {
            words,
            rounds,
            ..Outcome::default()
        },
    };
    runner.size(plan)?;
    for (recursive, levels) in &plan.steps {
        if !recursive {
            for level in levels {
                runner.level(level)?;
            }
            continue;
        }
        // Rounds as the CPU's, on the GPU: every member once per round; a round after one that
        // changed nothing is skipped (`label_round`), and the group's end records whether it
        // settled.
        for round in 0..rounds {
            runner.boundary(round, false);
            for level in levels {
                runner.level(level)?;
            }
        }
        runner.boundary(rounds, true);
    }
    runner.flush()?;
    if runner.flag(0) != 0 {
        return Err(Error::BackendFailure);
    }
    runner.outcome.waits[1] = runner.flag(3) as usize;
    runner.outcome.labels = unsafe {
        std::slice::from_raw_parts(
            runner.tapes.supertypes.buffer.contents().cast::<u32>().add(2),
            plan.labels * words,
        )
        .to_vec()
    };
    Ok(runner.outcome)
}

/// The tape buffers, from a batch of the files with no tasks; the label prefix starts empty.
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
    supertypes.resize(prefix, MetalSupertype { node: 0, kind: 0 });
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
        supertype_count: supertypes.len() as u32,
        supertypes: MappedBuffer::from_slice(device, &supertypes)?,
    })
}

impl Runner<'_> {
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
        let state = self.kernels[pipeline].clone();
        self.dispatch(&state, KERNELS[pipeline], buffers, shape, threads);
    }

    /// The clause's pattern kernel over the level's (pattern, root) items.
    fn pattern_kernel(&mut self, buffers: &[(&Buffer, u64)], shape: Shape) -> Result<(), Error> {
        let state = self.patterns.as_ref().ok_or(Error::BackendFailure)?.0.clone();
        self.dispatch(&state, patterns::KERNEL, buffers, shape, shape.item_count as usize);
        Ok(())
    }

    fn dispatch(&mut self, state: &ComputePipelineState, label: &str, buffers: &[(&Buffer, u64)], shape: Shape, threads: usize) {
        if threads == 0 {
            return;
        }
        let encoder = self.command.new_compute_command_encoder();
        encoder.set_label(label);
        encoder.set_compute_pipeline_state(state);
        for (index, (buffer, offset)) in buffers.iter().enumerate() {
            encoder.set_buffer(index as u64, Some(buffer), *offset);
        }
        encoder.set_bytes(
            buffers.len() as u64,
            std::mem::size_of::<Shape>() as u64,
            std::ptr::from_ref(&shape).cast(),
        );
        let width = state.thread_execution_width().max(1);
        encoder.checked_dispatch_threads(
            &self.command,
            MTLSize::new(threads as u64, 1, 1),
            MTLSize::new(width.min(threads as u64), 1, 1),
        );
        encoder.end_encoding();
        self.encoded = true;
        self.outcome.dispatches += 1;
    }

    /// A recursive group's round boundary (`label_round`).
    fn boundary(&mut self, round: usize, finish: bool) {
        let (flags, head) = (self.flags.buffer.clone(), self.tapes.supertypes.buffer.clone());
        let shape = Shape {
            round: round as u32,
            finish: u32::from(finish),
            ..self.shape()
        };
        self.kernel(2, &[(&flags, 0), (&head, 0)], shape, 1);
    }

    /// Every level's batch, built once, and one workspace for all of them, sized before anything
    /// is encoded so the host never waits to grow it.
    fn size(&mut self, plan: &Plan<'_>) -> Result<(), Error> {
        let mut widest_seen: Option<(WorkspaceTotals, u32, u32, usize)> = None;
        for level in plan.steps.iter().flat_map(|(_, levels)| levels) {
            let passes = programs_of(level);
            if passes.is_empty() {
                continue;
            }
            let (programs, anchors) = self.batch_inputs(&passes);
            let mut batch = PackedBatch::new(&programs, self.files, &anchors)?;
            if batch.tasks.is_empty() {
                self.prepared.insert(level.as_ptr() as usize, None);
                continue;
            }
            batch.header.supertype_count = self.tapes.supertype_count;
            let totals = batch.assign_workspace_with_result_limits_and_lanes(&programs, None, EXECUTION_LANES)?;
            let seen = (totals, batch.header.scheduler_byte_count, batch.header.workspace_byte_count, batch.tasks.len());
            widest_seen = Some(match widest_seen {
                None => seen,
                Some(old) => (widest(old.0, seen.0), old.1.max(seen.1), old.2.max(seen.2), old.3.max(seen.3)),
            });
            let mut routes = vec![[0_u32; 2]; batch.tasks.len()];
            for task in &batch.tasks {
                let (_, _, mode, slot) = passes[task.program as usize];
                routes[task.batch_ordinal as usize] = [mode, slot];
            }
            let prepared = Prepared {
                tasks: MappedBuffer::from_slice(self.device, &batch.tasks)?,
                header: MappedBuffer::from_slice(self.device, &[batch.header])?,
                routes: MappedBuffer::from_slice(self.device, &routes)?,
                program: self.engine.program_buffers(self.device, &batch)?,
                arenas: batch.global_arenas,
                task_count: batch.tasks.len(),
                result_capacity: batch.global_arenas.arenas[ArenaKind::Result as usize].capacity as usize,
                evaluations: std::mem::take(&mut batch.evaluations),
                lanes: std::mem::take(&mut batch.evaluation_lanes),
            };
            self.prepared.insert(level.as_ptr() as usize, Some(prepared));
        }
        if let Some((totals, scheduler_bytes, output_bytes, tasks)) = widest_seen {
            let mut lease = MetalWorkspaceLease::acquire(self.device, totals, scheduler_bytes, output_bytes, tasks)?;
            // Each encoded level advances the generation itself.
            lease.generation_span = 0;
            self.workspace = Some(Workspace {
                lease,
                totals,
                scheduler_bytes,
                output_bytes,
                tasks,
            });
        }
        Ok(())
    }

    fn batch_inputs(&self, passes: &[(&Program, &Vec<Vec<u32>>, u32, u32)]) -> (Vec<Program>, Vec<Vec<Vec<u32>>>) {
        let programs = passes.iter().map(|(program, ..)| (*program).clone()).collect::<Vec<_>>();
        let anchors = (0..self.files.len())
            .map(|file| passes.iter().map(|(_, roots, ..)| roots[file].clone()).collect())
            .collect();
        (programs, anchors)
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
        let programs = programs_of(passes);
        // How each target settles: from scratch, or computed by `label_settle` per word.
        let mut settles = Vec::new();
        let mut matches = Vec::new();
        let mut constraints = Vec::new();
        for pass in passes {
            settles.push(match pass {
                Pass::Program { target, .. } | Pass::Empty { target } => Op {
                    target: *target,
                    ..Op::default()
                },
                Pass::Count {
                    target,
                    slot,
                    min,
                    max,
                    ..
                } => Op {
                    kind: 2,
                    target: *target,
                    slot: *slot,
                    minimum: *min,
                    maximum: *max,
                    of: 0,
                },
                Pass::Complement { target, of } => Op {
                    kind: 1,
                    target: *target,
                    of: *of,
                    ..Op::default()
                },
                Pass::Pattern {
                    target,
                    pattern,
                    roots,
                    constraints: named,
                } => {
                    let vars = &self.patterns.as_ref().ok_or(Error::BackendFailure)?.1[*pattern];
                    let start = (constraints.len() / 2) as u32;
                    for (name, label) in named {
                        if let Some(var) = vars.iter().position(|known| known == name) {
                            constraints.extend([var as u32, *label]);
                        }
                    }
                    let count = (constraints.len() / 2) as u32 - start;
                    for root in roots {
                        matches.extend([*pattern as u32, *target, *root, start, count]);
                    }
                    Op {
                        target: *target,
                        ..Op::default()
                    }
                }
            });
        }
        if !programs.is_empty() {
            self.programs(passes.as_ptr() as usize)?;
        }
        let labels = self.tapes.supertypes.buffer.clone();
        let (scratch, counts, flags) =
            (self.scratch.buffer.clone(), self.counts.buffer.clone(), self.flags.buffer.clone());
        if !matches.is_empty() {
            let item_count = (matches.len() / 5) as u32;
            let (items, constraints) = (self.upload(&matches)?, self.upload(&constraints)?);
            let tapes = &self.tapes;
            let (nodes, children, source) =
                (tapes.nodes.buffer.clone(), tapes.children.buffer.clone(), tapes.source.buffer.clone());
            let shape = Shape {
                item_count,
                ..self.shape()
            };
            self.pattern_kernel(
                &[
                    (&nodes, 0),
                    (&children, 0),
                    (&source, 0),
                    (&labels, 8),
                    (&scratch, 0),
                    (&flags, 0),
                    (&items, 0),
                    (&constraints, 0),
                ],
                shape,
            )?;
        }
        let target_count = settles.len();
        let settles = self.upload(&settles)?;
        let shape = Shape {
            target_count: target_count as u32,
            ..self.shape()
        };
        self.kernel(
            1,
            &[(&labels, 8), (&scratch, 0), (&flags, 0), (&settles, 0), (&counts, 0)],
            shape,
            target_count * self.words,
        );
        if self.segment_bytes > SEGMENT_BYTES {
            self.outcome.waits[2] += 1;
            self.flush()?;
        }
        Ok(())
    }

    /// The level's prepared batch, then `label_sink` routing each result by its task's program.
    fn programs(&mut self, level: usize) -> Result<(), Error> {
        let Some(Some(prepared)) = self.prepared.get(&level) else {
            return Ok(());
        };
        let workspace = self.workspace.as_mut().ok_or(Error::BackendFailure)?;
        let lease = &mut workspace.lease;
        let generation = lease.generation;
        let span = u32::try_from(prepared.task_count + 1).map_err(|_| Error::BackendCapacity)? * 2;
        lease.generation = lease.generation.checked_add(span).ok_or(Error::BackendCapacity)?;
        let stamp = |evaluations: &[MetalEvaluation]| {
            let mut evaluations = evaluations.to_vec();
            evaluations.iter_mut().for_each(|evaluation| evaluation.generation = generation);
            evaluations
        };
        let mut image = bytes_of(std::slice::from_ref(&prepared.arenas)).to_vec();
        image.extend_from_slice(bytes_of(&stamp(&prepared.evaluations)));
        image.extend_from_slice(bytes_of(&stamp(&prepared.lanes)));
        let (tasks, lanes_count, capacity) = (prepared.task_count, prepared.lanes.len(), prepared.result_capacity);
        let image_length = image.len() as u64;
        let image = self.upload(&image)?;
        let prepared = self.prepared.get(&level).and_then(Option::as_ref).expect("prepared");
        let (task_buffer, header_buffer, routes, program_buffers) = (
            prepared.tasks.buffer.clone(),
            prepared.header.buffer.clone(),
            prepared.routes.buffer.clone(),
            Arc::clone(&prepared.program),
        );
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
        let lanes = lanes_count as u64;
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

/// A level's programs: (program, roots per file, sink mode, scratch label or count slot).
fn programs_of<'p>(passes: &'p [Pass<'_>]) -> Vec<(&'p Program, &'p Vec<Vec<u32>>, u32, u32)> {
    passes
        .iter()
        .filter_map(|pass| match pass {
            Pass::Program {
                target,
                program,
                roots,
                captured,
            } => Some((*program, roots, if *captured { 1 } else { 0 }, *target)),
            Pass::Count {
                slot,
                program,
                roots,
                captured,
                ..
            } => Some((*program, roots, if *captured { 3 } else { 2 }, *slot)),
            _ => None,
        })
        .collect()
}
