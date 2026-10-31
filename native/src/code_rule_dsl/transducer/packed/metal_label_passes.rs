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
//! command buffer. A group of programs alone runs as sweeps instead (`label_sweep`): one launch
//! per sweep, each thread a file in node order, each result marking its label at once, so the
//! sweeps follow the recursion's direction rather than one tree edge per round.

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
    /// Per step, for a recursive group: the kinds its recursive labels' nodes have (`None`:
    /// any), and whether its programs step to siblings (`NEXT`, `FOLLOWING`). Its rounds follow
    /// chains of such nodes, nested or (stepping to siblings) side by side.
    pub(in crate::code_rule_dsl) chains: Vec<(Option<Vec<u16>>, bool)>,
    /// The labels' words before the passes (as `Outcome::labels`), where steps before them ran
    /// elsewhere; empty otherwise.
    pub(in crate::code_rule_dsl) initial: Option<Vec<u32>>,
}

/// Whether a recursive group runs as sweeps: its passes are programs (or empty), whose results only
/// add to their labels, or complements as well, where its programs mark their roots (a node's
/// verdicts are then all its own, computed at it in a reverse sweep).
fn sweeps(levels: &[Vec<Pass<'_>>]) -> bool {
    let passes = || levels.iter().flatten();
    passes().all(|pass| matches!(pass, Pass::Program { .. } | Pass::Empty { .. } | Pass::Complement { .. }))
        && passes().any(|pass| matches!(pass, Pass::Program { .. }))
        && (!complements(levels) || passes().all(|pass| !matches!(pass, Pass::Program { captured: true, .. })))
}

/// A group with complements sweeps in reverse preorder only: its walks move forward in preorder.
fn complements(levels: &[Vec<Pass<'_>>]) -> bool {
    levels.iter().flatten().any(|pass| matches!(pass, Pass::Complement { .. }))
}

/// Sweeps encoded per group: a recursion that runs one way settles in the first or second (the
/// directions alternate), one that turns once in the third, and one more sees nothing change.
const SWEEPS: usize = 4;

/// A label level's interpreter lanes, and the states of each transient kind a task may hold
/// (O28). A lane takes tasks from the level's cursor, one after another, and reserves its states
/// up front, so lanes times states is the arena: 64 lanes of 65,536 states left a level of 10^5
/// tasks on two SIMD groups. 1,024 lanes of 4,096 states hold the same records, sixteen times as
/// wide. A task that needs more fails its batch, which runs again as before (`WIDE`).
const NARROW: (usize, Option<u32>) = (1024, Some(4096));
/// The program backend's lanes and states: every task's worst case.
const WIDE: (usize, Option<u32>) = (EXECUTION_LANES, None);

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
    /// Whether a task outgrew `NARROW`'s states, so the batch ran again with `WIDE`'s.
    pub(in crate::code_rule_dsl) widened: bool,
    /// Whether a micro walk outgrew its frames, so every program ran on the interpreter.
    pub(in crate::code_rule_dsl) interpreted: bool,
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
    recursive: u32,
    last_level: u32,
    file_count: u32,
    /// A micro walk's steps (`compile_micro::steps` for the largest file and program).
    steps: u32,
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
const KERNELS: [&str; 4] = ["label_sink", "label_settle", "label_round", "label_micro"];
/// The launches a clause's encoding aims to stay within, below the evaluation's budget
/// (`metal_launch_budget`); a recursive group's rounds are encoded from what is left.
const LAUNCHES: usize = 560;
/// At most this many rounds of a recursive group are encoded in one go.
const MOST_ROUNDS: usize = 64;
/// Bytes of per-level uploads one command buffer carries before the host waits for it.
const SEGMENT_BYTES: usize = 64 << 20;

/// Compiles the label passes' pipelines (the interpreter, the sweep and the label kernels) on a
/// thread, once a process: each is compiled from source, 0.6 s together, and a pass that needs
/// one before it is ready waits for the rest of its compile (they are cached behind locks).
pub(in crate::code_rule_dsl) fn warm() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        std::thread::spawn(|| {
            let _ = compile_interpreter_cached();
            let _ = pipelines();
            let _ = compile_sweep_cached();
        });
    });
}

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

/// A level's micro walks (`label_micro`): per pass {word start, word count, scratch label or
/// count slot, mode (as `label_sink`'s routes), item start, item count, first text slot, 0}, the passes' words, and their starts
/// (global nodes).
struct Walks {
    passes: MappedBuffer,
    /// Each text slot's bits (`Micro::texts`), one label's words a slot.
    texts: MappedBuffer,
    words: MappedBuffer,
    items: MappedBuffer,
    pass_count: usize,
    item_count: usize,
    steps: u32,
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
    /// `label_sweep`, where a group runs as sweeps.
    sweep: Option<ComputePipelineState>,
    /// By a sweeping group's address: per file the start of its tasks, then their indices by
    /// (root, program). Its task buffer carries them after the tasks.
    orders: HashMap<usize, Vec<u32>>,
    /// The clause's pattern kernel and each pattern's metavariables in index order.
    patterns: Option<(ComputePipelineState, Vec<Vec<String>>)>,
    files: &'a [TapeDoc],
    tapes: Tapes,
    /// Labels in the prefix.
    plan_labels: usize,
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
    /// A level's lanes and each task's states; a sweeping group's are always `WIDE`.
    lanes: (usize, Option<u32>),
    /// Whether a level's micro walks run as such, and each level's, by its address.
    micro: bool,
    walks: HashMap<usize, Walks>,
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
    // A group of rounds ends with `label_round`; a group of sweeps is checked by the host.
    let groups = plan.steps.iter().filter(|(recursive, levels)| *recursive && !sweeps(levels)).count();
    // A recursive group's label grows (or settles) one node of a chain per round: a node joins
    // in the round after the node its walk reaches. Its nodes have the label's kinds, so the
    // longest chain of such nodes, nested along a path or (where the group steps to siblings)
    // side by side, plus a round to see nothing change, is the expected count. It is not a
    // proof: a group still changing in its last round reruns the clause with more.
    let expected = plan
        .chains
        .iter()
        .zip(&plan.steps)
        .map(|((kinds, siblings), (_, levels))| {
            if sweeps(levels) {
                return SWEEPS;
            }
            let has = |tape: &TapeDoc, node: u32| {
                kinds.as_ref().is_none_or(|kinds| kinds.binary_search(&tape.transducer_kind(node)).is_ok())
            };
            files
                .iter()
                .map(|tape| {
                    let mut nested = vec![0_usize; tape.transducer_len()];
                    let mut longest = 0;
                    for node in 0..tape.transducer_len() as u32 {
                        let above = tape.transducer_parent(node).map_or(0, |parent| nested[parent as usize]);
                        nested[node as usize] = above + usize::from(has(tape, node));
                        longest = longest.max(nested[node as usize]);
                        if *siblings {
                            let mut run = 0;
                            for child in tape.transducer_children(node) {
                                run = if has(tape, child) { run + 1 } else { 0 };
                                longest = longest.max(run);
                            }
                        }
                    }
                    longest
                })
                .max()
                .unwrap_or(0)
                + 2
        })
        .collect::<Vec<_>>();
    // Each group's rounds: as expected, all capped alike where the budget cannot afford that.
    let per_group = plan
        .steps
        .iter()
        .map(|(recursive, levels)| match (*recursive, sweeps(levels)) {
            (false, _) => 0,
            (true, true) => 1,
            (true, false) => levels.iter().map(launches).sum::<usize>(),
        })
        .collect::<Vec<_>>();
    let plan_rounds = |used: usize, wanted: &[usize]| {
        let budget = LAUNCHES.saturating_sub(used + fixed + groups);
        let rounds = |cap: usize| {
            plan.steps
                .iter()
                .zip(wanted)
                .map(|((recursive, _), wanted)| if *recursive { (*wanted).clamp(2, cap.max(2)) } else { 0 })
                .collect::<Vec<_>>()
        };
        let cost = |rounds: &[usize]| rounds.iter().zip(&per_group).map(|(rounds, launches)| rounds * launches).sum::<usize>();
        let mut cap = MOST_ROUNDS;
        while cap > 2 && cost(&rounds(cap)) > budget {
            cap -= 1;
        }
        rounds(cap)
    };
    let mut wanted = expected.clone();
    let mut rounds = plan_rounds(0, &wanted);
    let nodes = files.iter().map(TapeDoc::transducer_len).sum::<usize>();
    let (mut retries, mut used, mut buffers) = (0, 0, 0);
    let (mut lanes, mut micro) = (NARROW, true);
    loop {
        let mut outcome = match encode(device, plan, files, &rounds, lanes, micro) {
            // A task's states outgrew the narrow lanes' (or another task failed, which fails again).
            Err(Error::BackendFailure) if lanes != WIDE => {
                crate::code_rule_dsl::prof::count("run.widened", 1);
                lanes = WIDE;
                continue;
            }
            // A micro walk outgrew its frames, captures or steps.
            Err(Error::BackendCapacity) if micro => {
                crate::code_rule_dsl::prof::count("run.interpreted", 1);
                micro = false;
                continue;
            }
            outcome => outcome?,
        };
        outcome.interpreted = !micro;
        outcome.widened = lanes == WIDE;
        used += outcome.dispatches;
        buffers += outcome.command_buffers;
        if outcome.waits[1] == 0 {
            outcome.waits[1] = retries;
            outcome.dispatches = used;
            outcome.command_buffers = buffers;
            return Ok(outcome);
        }
        if rounds.iter().all(|rounds| *rounds > nodes + 1 || *rounds == 0) {
            return Err(Error::WorkLimit);
        }
        retries += 1;
        wanted = rounds.iter().map(|rounds| rounds * 2).collect();
        rounds = plan_rounds(used, &wanted);
    }
}

/// The clause's passes in one command buffer, each recursive group as `rounds` rounds.
/// `lanes`: a level's lanes and each task's states (`NARROW`, `WIDE`); `micro`: whether a pass
/// whose program is a micro walk runs as one (`label_micro`).
fn encode(
    device: &Device,
    plan: &Plan<'_>,
    files: &[TapeDoc],
    rounds: &[usize],
    lanes: (usize, Option<u32>),
    micro: bool,
) -> Result<Outcome, Error> {
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
        sweep: if plan.steps.iter().any(|(recursive, levels)| *recursive && sweeps(levels)) {
            Some(compile_sweep_cached()?)
        } else {
            None
        },
        orders: HashMap::new(),
        patterns: pattern_pipeline(&plan.patterns)?,
        command,
        queue,
        files,
        tapes: crate::code_rule_dsl::prof::time("gpu.tapes", || tapes(device, files, plan, words))?,
        plan_labels: plan.labels,
        words,
        nodes,
        scratch: MappedBuffer::zeroed::<u32>(device, plan.labels * words)?,
        counts: MappedBuffer::zeroed::<u32>(device, (plan.counts * nodes).max(1))?,
        flags: MappedBuffer::zeroed::<u32>(device, 6)?,
        workspace: None,
        retained: Vec::new(),
        programs: Vec::new(),
        prepared: HashMap::new(),
        segment_bytes: 0,
        lanes,
        micro,
        walks: HashMap::new(),
        encoded: false,
        outcome: Outcome {
            words,
            rounds: rounds.iter().copied().max().unwrap_or(0),
            ..Outcome::default()
        },
    };
    crate::code_rule_dsl::prof::time("gpu.size", || runner.size(plan))?;
    let _encode = crate::code_rule_dsl::prof::Stage::start("gpu.encode");
    for (group, ((recursive, levels), &rounds)) in plan.steps.iter().zip(rounds).enumerate() {
        if !recursive {
            for level in levels {
                runner.level(level, None)?;
            }
            continue;
        }
        if sweeps(levels) {
            let reverse = complements(levels);
            for round in 0..rounds {
                runner.sweep(levels.as_ptr() as usize, group, round, reverse)?;
            }
            continue;
        }
        // Rounds as the CPU's, on the GPU: every member once per round; once a round changes
        // nothing the rest are skipped (`label_settle`), and the group's end records whether it
        // settled (`label_round`).
        for round in 0..rounds {
            for (at, level) in levels.iter().enumerate() {
                runner.level(level, Some((round, at + 1 == levels.len())))?;
            }
        }
        runner.boundary(rounds);
    }
    drop(_encode);
    crate::code_rule_dsl::prof::time("gpu.flush", || runner.flush())?;
    // A micro walk that outgrew its frames: the passes again on the interpreter.
    if runner.flag(0) & 2 != 0 {
        return Err(Error::BackendCapacity);
    }
    if runner.flag(0) != 0 {
        return Err(Error::BackendFailure);
    }
    // The sweeps' words: a failure, and per group whether its last sweep still changed a label.
    let sweep_words = unsafe {
        std::slice::from_raw_parts(
            runner.tapes.supertypes.buffer.contents().cast::<u32>().add(2 + plan.labels * words),
            2 + 2 * plan.steps.len(),
        )
        .to_vec()
    };
    if sweep_words[1] != 0 {
        return Err(Error::BackendFailure);
    }
    let unsettled = plan
        .steps
        .iter()
        .zip(rounds)
        .enumerate()
        .filter(|(group, ((recursive, levels), rounds))| {
            *recursive && sweeps(levels) && **rounds > 0 && sweep_words[2 + 2 * group + (**rounds - 1) % 2] != 0
        })
        .count();
    runner.outcome.waits[1] = runner.flag(3) as usize + unsettled;
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
    // The labels, then the sweeps' round, failure and per group (by step) changes (`label_sweep`).
    let prefix = 1 + (plan.labels * words + 2 + 2 * plan.steps.len()).div_ceil(2);
    let mut supertypes = Vec::with_capacity(prefix + batch.supertypes.len());
    supertypes.push(MetalSupertype {
        node: plan.labels as u32,
        kind: words as u32,
    });
    supertypes.resize(prefix, MetalSupertype { node: 0, kind: 0 });
    // The prefix's words after its head: the labels.
    for (at, &word) in plan.initial.iter().flatten().enumerate().take(plan.labels * words) {
        let slot = &mut supertypes[1 + at / 2];
        if at % 2 == 0 {
            slot.node = word;
        } else {
            slot.kind = word;
        }
    }
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

    /// A recursive group's end after `rounds` rounds (`label_round`).
    fn boundary(&mut self, rounds: usize) {
        let (flags, head) = (self.flags.buffer.clone(), self.tapes.supertypes.buffer.clone());
        let shape = Shape {
            round: rounds as u32,
            finish: 1,
            ..self.shape()
        };
        self.kernel(2, &[(&flags, 0), (&head, 0)], shape, 1);
    }

    /// Every level's batch, built once, and one workspace for all of them, sized before anything
    /// is encoded so the host never waits to grow it.
    fn size(&mut self, plan: &Plan<'_>) -> Result<(), Error> {
        let mut widest_seen: Option<(WorkspaceTotals, u32, u32, usize)> = None;
        // A sweeping group is one batch of all its programs, by its address; a level is one by
        // its own.
        let batches = plan.steps.iter().flat_map(|(recursive, levels)| {
            if *recursive && sweeps(levels) {
                vec![(levels.as_ptr() as usize, levels.iter().flat_map(|level| programs_of(level)).collect::<Vec<_>>(), Some(levels))]
            } else {
                levels.iter().map(|level| (level.as_ptr() as usize, programs_of(level), None)).collect()
            }
        });
        let bases = self.files.iter().scan(0_u32, |base, tape| {
            let at = *base;
            *base += tape.transducer_len() as u32;
            Some(at)
        }).collect::<Vec<_>>();
        for (key, mut passes, sweep) in batches {
            // A level's micro walks run as such (`label_micro`); its other programs are tasks.
            if self.micro && sweep.is_none() {
                let (mut records, mut words, mut items) = (Vec::new(), Vec::new(), Vec::new());
                let (largest, mut longest) = (self.files.iter().map(TapeDoc::transducer_len).max().unwrap_or(0), 0);
                // Each text slot's bits, `self.words` words a slot over every file's nodes.
                let (mut texts, mut slots) = (Vec::<u32>::new(), 0_u32);
                let label_words = self.words;
                let files = self.files;
                // Each distinct (value, kind)'s slot row, computed once for the run's levels.
                let mut rows = HashMap::<String, Option<u32>>::new();
                passes.retain(|&(program, roots, mode, target)| {
                    let Some(walk) = Micro::of(program) else { return true };
                    // The walk's slots as rows of `texts`, contiguous from `text_start`; a row
                    // already computed for another pass is copied.
                    let mut own = Vec::new();
                    for slot in 0..walk.texts.len() {
                        let key = walk.slot_key(program, slot);
                        let row = match rows.get(&key) {
                            Some(row) => *row,
                            None => {
                                use rayon::prelude::*;
                                let _texts = crate::code_rule_dsl::prof::Stage::start("size.texts");
                                let per_file = files.par_iter().map(|tape| walk.slot(program, slot, tape)).collect::<Option<Vec<_>>>();
                                let row = per_file.map(|per_file| {
                                    let row = slots;
                                    slots += 1;
                                    texts.resize(texts.len() + label_words, 0);
                                    let at = row as usize * label_words;
                                    for (bits, base) in per_file.iter().zip(&bases) {
                                        for (index, &word) in bits.iter().enumerate() {
                                            let mut word = word;
                                            while word != 0 {
                                                let node = *base + (index * 64) as u32 + word.trailing_zeros();
                                                texts[at + node as usize / 32] |= 1 << (node % 32);
                                                word &= word - 1;
                                            }
                                        }
                                    }
                                    row
                                });
                                rows.insert(key, row);
                                row
                            }
                        };
                        let Some(row) = row else { return true };
                        own.push(row);
                    }
                    // A pass reads slot `s` at `text_start + s`: its rows, copied where shared.
                    let text_start = slots;
                    for &row in &own {
                        let from = row as usize * label_words;
                        texts.extend_from_within(from..from + label_words);
                    }
                    slots += own.len() as u32;
                    let start = items.len() as u32;
                    for (roots, base) in roots.iter().zip(&bases) {
                        items.extend(roots.iter().map(|node| base + node));
                    }
                    records.push([words.len() as u32, walk.words.len() as u32, target, mode, start, items.len() as u32 - start, text_start, 0]);
                    longest = longest.max(walk.words.len());
                    words.extend_from_slice(&walk.words);
                    false
                });
                crate::code_rule_dsl::prof::count("size.walks", items.len());
                if !items.is_empty() {
                    if texts.is_empty() {
                        texts.push(0);
                    }
                    let walks = Walks {
                        texts: MappedBuffer::from_slice(self.device, &texts)?,
                        steps: super::super::micro_steps(largest, longest),
                        pass_count: records.len(),
                        item_count: items.len(),
                        passes: MappedBuffer::from_slice(self.device, &records)?,
                        words: MappedBuffer::from_slice(self.device, &words)?,
                        items: MappedBuffer::from_slice(self.device, &items)?,
                    };
                    self.walks.insert(key, walks);
                }
            }
            if passes.is_empty() {
                continue;
            }
            if crate::code_rule_dsl::prof::on() {
                for &(program, roots, _, _) in &passes {
                    let reason = Micro::refusal(program);
                    crate::code_rule_dsl::prof::count_named(format!("refused.{reason}"), roots.iter().map(Vec::len).sum());
                }
            }
            let (programs, anchors) = self.batch_inputs(&passes);
            let mut batch = crate::code_rule_dsl::prof::time("size.batch", || PackedBatch::new(&programs, self.files, &anchors))?;
            crate::code_rule_dsl::prof::count("size.tasks", batch.tasks.len());
            if batch.tasks.is_empty() {
                self.prepared.insert(key, None);
                continue;
            }
            if let Some(levels) = sweep {
                // The route in the program's flags marks its label as it publishes (`label_sweep`).
                for (program, (_, _, mode, target)) in batch.programs.iter_mut().zip(&passes) {
                    program.flags |= 0x8000_0000 | (u32::from(*mode == 1) << 30) | (target << 8);
                }
                // Each program's and complement's place among a node's work: the group's order.
                let (mut positions, mut ops) = (Vec::new(), Vec::new());
                for (at, pass) in levels.iter().flatten().enumerate() {
                    match pass {
                        Pass::Program { .. } => positions.push(at as u32),
                        Pass::Complement { target, of } => ops.push((at as u32, [*target, *of])),
                        _ => {}
                    }
                }
                // Entries per file: (node, place, item).
                let mut entries = vec![Vec::new(); self.files.len()];
                for (index, task) in batch.tasks.iter().enumerate() {
                    entries[task.file as usize].push((task.root, positions[task.program as usize], index as u32));
                }
                for (file, entries) in batch.files.iter().zip(&mut entries) {
                    for node in file.node_start..file.node_start + file.node_count {
                        for (op, (at, _)) in ops.iter().enumerate() {
                            entries.push((node, *at, 0x8000_0000 | op as u32));
                        }
                    }
                }
                let mut order = vec![0_u32];
                for entries in &mut entries {
                    entries.sort_unstable();
                    order.push(order.last().copied().unwrap_or(0) + entries.len() as u32);
                }
                for (node, _, item) in entries.into_iter().flatten() {
                    order.extend([item, node]);
                }
                order.extend(ops.iter().flat_map(|(_, op)| *op));
                self.orders.insert(key, order);
            }
            batch.header.supertype_count = self.tapes.supertype_count;
            let (lanes, states) = if sweep.is_some() { WIDE } else { self.lanes };
            batch.state_cap = states;
            let totals = crate::code_rule_dsl::prof::time("size.assign", || batch.assign_workspace_with_result_limits_and_lanes(&programs, None, lanes))?;
            crate::code_rule_dsl::prof::count("size.evaluations", batch.evaluations.len());
            let seen = (totals, batch.header.scheduler_byte_count, batch.header.workspace_byte_count, batch.tasks.len());
            widest_seen = Some(match widest_seen {
                None => seen,
                Some(old) => (old.0.widest(seen.0), old.1.max(seen.1), old.2.max(seen.2), old.3.max(seen.3)),
            });
            let mut routes = vec![[0_u32; 2]; batch.tasks.len()];
            for task in &batch.tasks {
                let (_, _, mode, slot) = passes[task.program as usize];
                routes[task.batch_ordinal as usize] = [mode, slot];
            }
            let mut tasks = bytes_of(&batch.tasks).to_vec();
            if let Some(order) = self.orders.get(&key) {
                tasks.extend_from_slice(bytes_of(order));
            }
            let prepared = Prepared {
                tasks: MappedBuffer::from_slice(self.device, &tasks)?,
                header: MappedBuffer::from_slice(self.device, &[batch.header])?,
                routes: MappedBuffer::from_slice(self.device, &routes)?,
                program: crate::code_rule_dsl::prof::time("size.program_buffers", || self.engine.program_buffers(self.device, &batch))?,
                arenas: batch.global_arenas,
                task_count: batch.tasks.len(),
                result_capacity: batch.global_arenas.arenas[ArenaKind::Result as usize].capacity as usize,
                evaluations: std::mem::take(&mut batch.evaluations),
                lanes: std::mem::take(&mut batch.evaluation_lanes),
            };
            self.prepared.insert(key, Some(prepared));
        }
        if let Some((totals, scheduler_bytes, output_bytes, tasks)) = widest_seen {
            let mut lease = crate::code_rule_dsl::prof::time("size.lease", || MetalWorkspaceLease::acquire(self.device, totals, scheduler_bytes, output_bytes, tasks))?;
            crate::code_rule_dsl::prof::count("size.lease_mb", lease.bytes() >> 20);
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
            file_count: self.files.len() as u32,
            ..Shape::default()
        }
    }

    /// One dependency level: its programs in one dispatch, their results sunk, the derived
    /// labels, then every target settled.
    /// `round`: in a recursive group, the round and whether this is its last level.
    fn level(&mut self, passes: &[Pass<'_>], round: Option<(usize, bool)>) -> Result<(), Error> {
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
            self.walk(passes.as_ptr() as usize);
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
            round: round.map_or(0, |(round, _)| round as u32),
            recursive: u32::from(round.is_some()),
            last_level: u32::from(round.is_some_and(|(_, last)| last)),
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

    /// The batch's scheduler image, with a fresh generation, copied into the workspace.
    fn stage(&mut self, key: usize) -> Result<(), Error> {
        let Some(Some(prepared)) = self.prepared.get(&key) else {
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
        let image_length = image.len() as u64;
        crate::code_rule_dsl::prof::count("stage.image_kb", image.len() >> 10);
        let image = self.upload(&image)?;
        let lease = &self.workspace.as_ref().expect("a workspace").lease;
        let blit = self.command.new_blit_command_encoder();
        blit.set_label("Rerail label pass scheduler image");
        blit.copy_from_buffer(&image, 0, &lease.scheduler.buffer, 0, image_length);
        blit.end_encoding();
        Ok(())
    }

    /// The interpreter's buffers for a prepared batch, in `run_batch`'s order.
    fn bound(&self, prepared: &Prepared) -> Vec<Buffer> {
        let lease = &self.workspace.as_ref().expect("a workspace").lease;
        let tapes = &self.tapes;
        let program_buffers = &prepared.program;
        [
            &prepared.tasks.buffer,
            &prepared.header.buffer,
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
        .cloned()
        .collect()
    }

    /// One sweep of the group at step `group` (`label_sweep`), its changed flag cleared first.
    fn sweep(&mut self, key: usize, group: usize, round: usize, reverse: bool) -> Result<(), Error> {
        let Some(Some(prepared)) = self.prepared.get(&key) else {
            return Ok(());
        };
        let lanes = prepared.lanes.len() as u64;
        self.stage(key)?;
        let prepared = self.prepared.get(&key).and_then(Option::as_ref).expect("prepared");
        let buffers = self.bound(prepared);
        // The round, and its changed flag cleared, in the prefix after the labels.
        let at = 4 * (2 + self.plan_labels * self.words) as u64;
        let word = u32::try_from(round).ok().filter(|round| *round < 1 << 16).ok_or(Error::BackendCapacity)?
            | u32::try_from(group).ok().filter(|group| *group < 1 << 15).ok_or(Error::BackendCapacity)? << 16
            | u32::from(reverse) << 31;
        let round_word = self.upload(&[word])?;
        let head = self.tapes.supertypes.buffer.clone();
        let blit = self.command.new_blit_command_encoder();
        blit.set_label("Rerail label sweep round");
        blit.copy_from_buffer(&round_word, 0, &head, at, 4);
        blit.fill_buffer(&head, metal::NSRange::new(at + 4 * (2 + 2 * group as u64 + round as u64 % 2), 4), 0);
        blit.end_encoding();
        let state = self.sweep.clone().ok_or(Error::BackendFailure)?;
        let encoder = self.command.new_compute_command_encoder();
        encoder.set_label("label_sweep");
        encoder.set_compute_pipeline_state(&state);
        for (index, buffer) in buffers.iter().enumerate() {
            encoder.set_buffer(index as u64, Some(buffer), 0);
        }
        let width = state.thread_execution_width().max(1);
        encoder.checked_dispatch_threads(&self.command, MTLSize::new(lanes, 1, 1), MTLSize::new(width.min(lanes), 1, 1));
        encoder.end_encoding();
        self.encoded = true;
        self.outcome.dispatches += 1;
        Ok(())
    }

    /// The level's micro walks (`label_micro`), one thread per (pass, start), marking scratch as
    /// `label_sink` marks the tasks' results.
    fn walk(&mut self, level: usize) {
        let Some(walks) = self.walks.get(&level) else { return };
        let (passes, words, items) = (walks.passes.buffer.clone(), walks.words.buffer.clone(), walks.items.buffer.clone());
        let texts = walks.texts.buffer.clone();
        let shape = Shape { item_count: walks.item_count as u32, target_count: walks.pass_count as u32, steps: walks.steps, ..self.shape() };
        let tapes = &self.tapes;
        let (nodes, children, supertypes, files) =
            (tapes.nodes.buffer.clone(), tapes.children.buffer.clone(), tapes.supertypes.buffer.clone(), tapes.files.buffer.clone());
        let (scratch, flags, counts) = (self.scratch.buffer.clone(), self.flags.buffer.clone(), self.counts.buffer.clone());
        let buffers = [
            (&nodes, 0),
            (&children, 0),
            (&supertypes, 0),
            (&files, 0),
            (&passes, 0),
            (&words, 0),
            (&items, 0),
            (&scratch, 0),
            (&flags, 0),
            (&texts, 0),
            (&counts, 0),
        ];
        self.kernel(3, &buffers, shape, shape.item_count as usize);
    }

    /// The level's prepared batch, then `label_sink` routing each result by its task's program.
    fn programs(&mut self, level: usize) -> Result<(), Error> {
        let Some(Some(prepared)) = self.prepared.get(&level) else {
            return Ok(());
        };
        let (tasks, lanes_count, capacity) = (prepared.task_count, prepared.lanes.len(), prepared.result_capacity);
        self.stage(level)?;
        let prepared = self.prepared.get(&level).and_then(Option::as_ref).expect("prepared");
        let routes = prepared.routes.buffer.clone();
        let buffers = self.bound(prepared);
        let lease = &self.workspace.as_ref().expect("a workspace").lease;
        let encoder = self.command.new_compute_command_encoder();
        encoder.set_label("Rerail label pass programs");
        encoder.set_compute_pipeline_state(self.engine.pipeline());
        for (index, buffer) in buffers.iter().enumerate() {
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
