use super::metal_backend::launch::{
    CheckedComputeEncoder, commit_checked, wait_checked,
};
use super::metal_backend::{MappedBuffer, pipeline_named};
use super::metal_batch_abi::{
    FORMAT_VERSION, FinalCaptureRecord, LinkRecord as CaptureRecord, MetalGlobalArenas,
    PendingRecord, PropertyRecord, RepeatRecord, ResultRecord, TaskStatus,
};
use super::packed_batch::{PackedBatch, PackedExecution, PackedResultPage};
use super::packed_workspace::WorkspaceTotals;
use super::*;
use crate::code_rule_dsl::tape::TapeDoc;
use metal::{ComputePipelineState, Device, MTLSize};
#[cfg(any(test, feature = "test-hooks"))]
use std::cell::Cell;
use std::collections::BTreeMap;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, LazyLock, Mutex};

#[cfg(any(test, feature = "test-hooks"))]
thread_local! {
    static INVOCATIONS: Cell<usize> = const { Cell::new(0) };
    static HOST_EVENTS: Cell<usize> = const { Cell::new(0) };
}

#[cfg(any(test, feature = "test-hooks"))]
pub(in crate::code_rule_dsl) fn reset_metrics() {
    INVOCATIONS.set(0);
    HOST_EVENTS.set(0);
}

#[cfg(any(test, feature = "test-hooks"))]
pub(in crate::code_rule_dsl) fn metrics() -> (usize, usize) {
    (INVOCATIONS.get(), HOST_EVENTS.get())
}

#[path = "metal_label_passes.rs"]
pub mod label_passes;
// Its tests run in the engine's test binary (its facade of this module mounts `metal_tape_tests.rs` as `tape_tests`): they read what the
// engine registers as it loads (the languages, their `.rr` libraries).
#[path = "metal_set_steps.rs"]
pub mod set_steps;
// Block programs on Metal: the Metal crate's.
pub use rerail_metal::block_program;
#[path = "metal_work_queue.rs"]
pub mod work_queue;
#[path = "metal_lattice.rs"]
pub mod lattice;
#[path = "metal_nearest.rs"]
pub mod nearest;
// Pre-pinned blocks: the Metal crate's.
pub use rerail_metal::pool as blocks;

const METAL_JIT_CACHE_LIMIT: usize = 32;
pub(super) const EXECUTION_LANES: usize = 64;
static JIT_CACHE: LazyLock<Mutex<BTreeMap<String, Engine>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
const METAL_WORKSPACE_POOL_LIMIT: usize = 4;
static METAL_WORKSPACE_POOL: LazyLock<Mutex<Vec<MetalWorkspace>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));

/// Frees the workspaces kept for reuse; whether there were any. Kept workspaces count against the
/// buffer budget, so a run that misses capacity frees them before anything else (`label_passes::run`).
pub(super) fn release_workspaces() -> bool {
    let released = METAL_WORKSPACE_POOL.lock().map(|mut pool| std::mem::take(&mut *pool)).unwrap_or_default();
    !released.is_empty()
}
pub use rerail_metal::METAL_DEVICE;

struct MetalWorkspace {
    totals: WorkspaceTotals,
    scheduler_bytes: u32,
    output_storage_bytes: u32,
    task_capacity: usize,
    generation: u32,
    pending: MappedBuffer,
    captures: MappedBuffer,
    cursors: MappedBuffer,
    repeats: MappedBuffer,
    scheduler: MappedBuffer,
    results: MappedBuffer,
    statuses: MappedBuffer,
    value_states: MappedBuffer,
    final_captures: MappedBuffer,
    output_storage: MappedBuffer,
}

impl MetalWorkspace {
    /// The bytes a workspace of these totals holds (`new`'s buffers, before page rounding).
    pub(super) fn bytes_for(totals: WorkspaceTotals, scheduler_bytes: u32, output_storage_bytes: u32, task_capacity: usize) -> usize {
        use std::mem::size_of;
        totals.pending as usize * size_of::<PendingRecord>()
            + (totals.captures as usize + totals.cursors as usize) * size_of::<CaptureRecord>()
            + totals.repeats as usize * size_of::<RepeatRecord>()
            + scheduler_bytes as usize
            + (totals.candidates as usize + totals.results as usize) * size_of::<ResultRecord>()
            + task_capacity * size_of::<TaskStatus>()
            + totals.value_states as usize * 4
            + totals.output_captures as usize * size_of::<FinalCaptureRecord>()
            + output_storage_bytes as usize
    }

    fn new(
        device: &Device,
        totals: WorkspaceTotals,
        scheduler_bytes: u32,
        output_storage_bytes: u32,
        task_capacity: usize,
    ) -> Result<Self, Error> {
        if crate::code_rule_dsl::prof::on() {
            use crate::code_rule_dsl::prof::count;
            use std::mem::size_of;
            count("ws.new", 1);
            count("ws.pending_mb", totals.pending as usize * size_of::<PendingRecord>() >> 20);
            count("ws.captures_mb", totals.captures as usize * size_of::<CaptureRecord>() >> 20);
            count("ws.cursors_mb", totals.cursors as usize * size_of::<CaptureRecord>() >> 20);
            count("ws.repeats_mb", totals.repeats as usize * size_of::<RepeatRecord>() >> 20);
            count("ws.scheduler_mb", scheduler_bytes as usize >> 20);
            count("ws.results_mb", totals.results as usize * size_of::<ResultRecord>() >> 20);
            count("ws.value_states_mb", totals.value_states as usize * 4 >> 20);
            count("ws.output_mb", output_storage_bytes as usize >> 20);
            count("ws.final_captures_mb", totals.output_captures as usize * size_of::<FinalCaptureRecord>() >> 20);
        }
        let workspace = Self {
            totals,
            scheduler_bytes,
            output_storage_bytes,
            task_capacity,
            generation: 1,
            pending: MappedBuffer::untouched::<PendingRecord>(device, totals.pending as usize)?,
            captures: MappedBuffer::untouched::<CaptureRecord>(device, totals.captures as usize)?,
            cursors: MappedBuffer::untouched::<CaptureRecord>(device, totals.cursors as usize)?,
            repeats: MappedBuffer::untouched::<RepeatRecord>(device, totals.repeats as usize)?,
            scheduler: MappedBuffer::untouched::<u8>(device, scheduler_bytes as usize)?,
            results: MappedBuffer::untouched::<ResultRecord>(
                device,
                totals
                    .candidates
                    .checked_add(totals.results)
                    .ok_or(Error::BackendCapacity)? as usize,
            )?,
            statuses: MappedBuffer::untouched::<TaskStatus>(device, task_capacity)?,
            value_states: MappedBuffer::untouched::<u32>(device, totals.value_states as usize)?,
            final_captures: MappedBuffer::untouched::<FinalCaptureRecord>(
                device,
                totals.output_captures as usize,
            )?,
            output_storage: MappedBuffer::untouched::<u8>(device, output_storage_bytes as usize)?,
        };
        for (buffer, label) in [
            (&workspace.pending, "Tempo pending paths"),
            (&workspace.results, "Tempo results"),
            (&workspace.final_captures, "Tempo final captures"),
            (&workspace.scheduler, "Tempo scheduler"),
            (&workspace.captures, "Tempo capture links"),
            (&workspace.cursors, "Tempo cursor links"),
            (&workspace.repeats, "Tempo repeat links"),
            (&workspace.output_storage, "Tempo output storage"),
        ] {
            buffer.buffer.set_label(label);
        }
        Ok(workspace)
    }

    fn fits(
        &self,
        totals: WorkspaceTotals,
        scheduler_bytes: u32,
        output_storage_bytes: u32,
        task_capacity: usize,
    ) -> bool {
        self.totals.pending >= totals.pending
            && self.totals.captures >= totals.captures
            && self.totals.cursors >= totals.cursors
            && self.totals.repeats >= totals.repeats
            && self.totals.query_states >= totals.query_states
            && self.totals.query_members >= totals.query_members
            && self.totals.queued_states >= totals.queued_states
            && self.totals.greedy_entries >= totals.greedy_entries
            && self.totals.value_states >= totals.value_states
            && self.totals.candidates >= totals.candidates
            && self.totals.results >= totals.results
            && self.totals.output_captures >= totals.output_captures
            && self.totals.properties >= totals.properties
            && self.scheduler_bytes >= scheduler_bytes
            && self.output_storage_bytes >= output_storage_bytes
            && self.task_capacity >= task_capacity
    }
}

struct MetalWorkspaceLease {
    workspace: Option<MetalWorkspace>,
    reused: bool,
    generation_span: u32,
}

impl MetalWorkspaceLease {
    /// The workspace's bytes.
    pub(super) fn bytes(&self) -> usize {
        let w = self.workspace.as_ref().expect("a workspace");
        [&w.pending, &w.captures, &w.cursors, &w.repeats, &w.scheduler, &w.results, &w.statuses, &w.value_states, &w.final_captures, &w.output_storage]
            .iter()
            .map(|buffer| buffer.buffer.length() as usize)
            .sum()
    }

    fn acquire(
        device: &Device,
        totals: WorkspaceTotals,
        scheduler_bytes: u32,
        output_storage_bytes: u32,
        task_capacity: usize,
    ) -> Result<Self, Error> {
        let generation_span = u32::try_from(task_capacity)
            .ok()
            .and_then(|tasks| tasks.checked_add(1))
            .and_then(|tasks| tasks.checked_mul(2))
            .ok_or(Error::BackendCapacity)?;
        let mut pool = METAL_WORKSPACE_POOL.lock().unwrap();
        let reusable = pool.iter().position(|workspace| {
            workspace.generation <= u32::MAX - generation_span
                && workspace.fits(totals, scheduler_bytes, output_storage_bytes, task_capacity)
        });
        let (workspace, reused) = if let Some(index) = reusable {
            (pool.swap_remove(index), true)
        } else {
            // The new workspace holds what the pool's did as well, so a run whose levels differ
            // from the last run's fits one workspace rather than replacing it each time (O28).
            let (mut totals, mut scheduler_bytes, mut output_storage_bytes, mut task_capacity) =
                (totals, scheduler_bytes, output_storage_bytes, task_capacity);
            for old in pool.drain(..) {
                totals = totals.widest(old.totals);
                scheduler_bytes = scheduler_bytes.max(old.scheduler_bytes);
                output_storage_bytes = output_storage_bytes.max(old.output_storage_bytes);
                task_capacity = task_capacity.max(old.task_capacity);
            }
            (
                MetalWorkspace::new(
                    device,
                    totals,
                    scheduler_bytes,
                    output_storage_bytes,
                    task_capacity,
                )?,
                false,
            )
        };
        Ok(Self {
            workspace: Some(workspace),
            reused,
            generation_span,
        })
    }
}

impl Deref for MetalWorkspaceLease {
    type Target = MetalWorkspace;

    fn deref(&self) -> &Self::Target {
        self.workspace.as_ref().unwrap()
    }
}

impl DerefMut for MetalWorkspaceLease {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.workspace.as_mut().unwrap()
    }
}

impl Drop for MetalWorkspaceLease {
    fn drop(&mut self) {
        let Some(mut workspace) = self.workspace.take() else {
            return;
        };
        workspace.generation = workspace.generation.saturating_add(self.generation_span);
        let mut pool = METAL_WORKSPACE_POOL.lock().unwrap();
        if pool.len() < METAL_WORKSPACE_POOL_LIMIT {
            pool.push(workspace);
        }
    }
}

const BATCH_ENTRY: &str = include_str!("batch_runtime.uglykl");
/// The tape's reads and readers (decision 0310), before the runtime: every batch reads its documents in the
/// batch's chunk block through the tape view (decision 0032), with no node records: the document table's entry as
/// generated (`slabs::msl`, decision 0289 §4), the reads written once (decision 0310), then the readers.
static TAPE_PREFIX: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| prefix(""));
/// The interpreter of resident label batches: tasks from roots, not records (decision 0033).
static RESIDENT_PREFIX: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| prefix("#define RERAIL_RESIDENT 1\n"));

fn prefix(defines: &str) -> String {
    format!(
        "{defines}{}{}{}",
        crate::code_rule_dsl::residency::slabs::msl(),
        "#include <ukl/reads/tape_z.h>\n",
        include_str!("device_tape.h")
    )
}

#[derive(Clone, Eq, PartialEq)]
pub(super) struct ProgramBufferKey {
    programs: Vec<super::metal_batch_abi::MetalProgram>,
    words: Vec<u32>,
    longs: Vec<u64>,
    values: Vec<super::metal_batch_abi::ValueRecord>,
    strings: Vec<super::metal_batch_abi::MetalString>,
    nfa_states: Vec<super::metal_batch_abi::NfaStateRecord>,
    nfa_data: Vec<u32>,
    nfa_ranges: Vec<super::metal_batch_abi::NfaRangeRecord>,
    lua_instructions: Vec<super::metal_batch_abi::LuaInstructionRecord>,
    lua_members: Vec<super::metal_batch_abi::LuaSetMemberRecord>,
    assertions: Vec<super::packed_batch::AssertionRecord>,
    assertion_data: Vec<u32>,
    string_bytes: Vec<u8>,
}

impl From<&PackedBatch> for ProgramBufferKey {
    fn from(batch: &PackedBatch) -> Self {
        Self {
            programs: batch.programs.clone(),
            words: batch.words.clone(),
            longs: batch.longs.clone(),
            values: batch.values.clone(),
            strings: batch.strings.clone(),
            nfa_states: batch.nfa_states.clone(),
            nfa_data: batch.nfa_data.clone(),
            nfa_ranges: batch.nfa_ranges.clone(),
            lua_instructions: batch.lua_instructions.clone(),
            lua_members: batch.lua_members.clone(),
            assertions: batch.assertions.clone(),
            assertion_data: batch.assertion_data.clone(),
            string_bytes: batch.string_bytes.clone(),
        }
    }
}

pub(super) struct ProgramBuffers {
    programs: MappedBuffer,
    words: MappedBuffer,
    longs: MappedBuffer,
    values: MappedBuffer,
    strings: MappedBuffer,
    nfa_states: MappedBuffer,
    nfa_data: MappedBuffer,
    nfa_ranges: MappedBuffer,
    lua_instructions: MappedBuffer,
    lua_members: MappedBuffer,
    assertions: MappedBuffer,
    assertion_data: MappedBuffer,
    string_bytes: MappedBuffer,
}

pub struct CachedProgramBuffers {
    key: ProgramBufferKey,
    buffers: Arc<ProgramBuffers>,
}

pub(super) type ProgramBufferCache = Arc<Mutex<Option<CachedProgramBuffers>>>;

#[derive(Clone)]
pub enum Engine {
    Interpreter {
        pipeline: ComputePipelineState,
        buffers: ProgramBufferCache,
    },
    Jit {
        pipeline: ComputePipelineState,
        programs: usize,
        buffers: ProgramBufferCache,
    },
}

impl Engine {
    fn pipeline(&self) -> &ComputePipelineState {
        match self {
            Self::Interpreter { pipeline, .. } | Self::Jit { pipeline, .. } => pipeline,
        }
    }

    fn buffers(&self) -> &ProgramBufferCache {
        match self {
            Self::Interpreter { buffers, .. } | Self::Jit { buffers, .. } => buffers,
        }
    }

    fn program_buffers(
        &self,
        device: &Device,
        batch: &PackedBatch,
    ) -> Result<Arc<ProgramBuffers>, Error> {
        let key = ProgramBufferKey::from(batch);
        let mut slot = self.buffers().lock().map_err(|_| Error::BackendFailure)?;
        if let Some(cached) = slot.as_ref()
            && cached.key == key
        {
            return Ok(Arc::clone(&cached.buffers));
        }
        let buffers = Arc::new(ProgramBuffers {
            programs: MappedBuffer::from_slice(device, &batch.programs)?,
            words: MappedBuffer::from_slice(device, &batch.words)?,
            longs: MappedBuffer::from_slice(device, &batch.longs)?,
            values: MappedBuffer::from_slice(device, &batch.values)?,
            strings: MappedBuffer::from_slice(device, &batch.strings)?,
            nfa_states: MappedBuffer::from_slice(device, &batch.nfa_states)?,
            nfa_data: MappedBuffer::from_slice(device, &batch.nfa_data)?,
            nfa_ranges: MappedBuffer::from_slice(device, &batch.nfa_ranges)?,
            lua_instructions: MappedBuffer::from_slice(device, &batch.lua_instructions)?,
            lua_members: MappedBuffer::from_slice(device, &batch.lua_members)?,
            assertions: MappedBuffer::from_slice(device, &batch.assertions)?,
            assertion_data: MappedBuffer::from_slice(device, &batch.assertion_data)?,
            string_bytes: MappedBuffer::from_slice(device, &batch.string_bytes)?,
        });
        *slot = Some(CachedProgramBuffers {
            key,
            buffers: Arc::clone(&buffers),
        });
        Ok(buffers)
    }
}

fn kernel_source(
    fetch: &str,
    native_dispatch: Option<&str>,
    jit_support: &str,
    jit_context: &str,
    jit_output_context: &str,
) -> String {
    let mut source = BATCH_ENTRY
        .replace("/*FETCH*/", fetch)
        .replace("/*JIT_SUPPORT*/", jit_support)
        .replace("/*JIT_CONTEXT*/", jit_context)
        .replace("/*JIT_OUTPUT_CONTEXT*/", jit_output_context);
    let begin = source
        .find("/*OP_SWITCH_BEGIN*/")
        .expect("Metal runtime has an opcode-switch start marker");
    let end = source
        .find("/*OP_SWITCH_END*/")
        .expect("Metal runtime has an opcode-switch end marker")
        + "/*OP_SWITCH_END*/".len();
    if let Some(dispatch) = native_dispatch {
        source.replace_range(begin..end, dispatch);
        let output_begin = source
            .find("/*OUTPUT_BEGIN*/")
            .expect("Metal runtime has an output-call start marker");
        let output_end = source
            .find("/*OUTPUT_END*/")
            .expect("Metal runtime has an output-call end marker")
            + "/*OUTPUT_END*/".len();
        source.replace_range(output_begin..output_end, "jit_apply_output(&outputContext)");
    } else {
        source = source
            .replace("/*OP_SWITCH_BEGIN*/", "")
            .replace("/*OP_SWITCH_END*/", "")
            .replace("/*OUTPUT_BEGIN*/", "")
            .replace("/*OUTPUT_END*/", "");
    }
    source
}

fn opcode_bodies() -> Result<std::collections::BTreeMap<u8, &'static str>, Error> {
    let begin = BATCH_ENTRY
        .find("/*OP_SWITCH_BEGIN*/")
        .ok_or(Error::BackendFailure)?;
    let end = BATCH_ENTRY
        .find("/*OP_SWITCH_END*/")
        .ok_or(Error::BackendFailure)?;
    let mut bodies = std::collections::BTreeMap::new();
    for line in BATCH_ENTRY[begin..end].lines().skip(1) {
        let mut remainder = line.trim();
        let mut opcodes = Vec::new();
        while let Some(case) = remainder.strip_prefix("case ") {
            let (opcode, rest) = case.split_once("u:").ok_or(Error::BackendFailure)?;
            opcodes.push(opcode.parse::<u8>().map_err(|_| Error::BackendFailure)?);
            remainder = rest;
        }
        for opcode in opcodes {
            bodies.insert(opcode, remainder);
        }
    }
    Ok(bodies)
}

/// The JIT's context of a step (`jit_support`): the task's buffers and tables, and its state, which a step reads
/// and writes through these pointers (UKL is C: no references).
const JIT_CONTEXT: &str = "JitContext context={nodes,children,supertypes,words,longs,values,strings,nfaStates,nfaData,nfaRanges,luaInstructions,luaMembers,valueWork,stringBytes,source,assertions,assertionData,captures,cursors,repeats,queuedStates,greedyEntries,pending,results,lineStarts,task,file,program,node,evaluation,globalArenas,valueBase,valueCapacity,status,&path,&pendingCount,&captureCount,&cursorCount,&repeatCount,&resultCount,&candidateHead,&localOrdinal,&keep,&advance};";
const JIT_OUTPUT_CONTEXT: &str = "JitOutputContext outputContext={assertionData,program,task,file,nodes,captures,raw.captureStart,raw.captureCount,finalCaptures,&finalCaptureCount,&captureBefore,outputStorage,&propertyCount,&outputByteCount,source,lineStarts,values,nfaStates,nfaData,nfaRanges,valueWork,resultIndex,&resultProperties,&dynamicProperties,&outputSteps,evaluation,sharedEvaluation,globalArenas,valueBase,valueCapacity};";

/// The JIT's state of a task that a step changes, by name and type: a step works on copies under the names the
/// opcode bodies use (as in `run_task_in`), written back when it finishes. A step that stops the task (`return
/// false`) writes nothing back: the task ends there, as the interpreter's `return` ends it.
const JIT_STATE: [(&str, &str); 10] = [
    ("Path", "path"),
    ("uint32_t", "pendingCount"),
    ("uint32_t", "captureCount"),
    ("uint32_t", "cursorCount"),
    ("uint32_t", "repeatCount"),
    ("uint32_t", "resultCount"),
    ("uint32_t", "candidateHead"),
    ("uint32_t", "localOrdinal"),
    ("bool", "keep"),
    ("bool", "advance"),
];

fn jit_support(
    bodies: &std::collections::BTreeMap<u8, &'static str>,
    programs: &[Program],
) -> Result<String, Error> {
    let mut source = String::from(
        "typedef struct JitContext{UKL_PRIVATE const NodeTable* nodes;UKL_GLOBAL const uint8_t* children;UKL_GLOBAL const Supertype* supertypes;UKL_GLOBAL const uint32_t* words;UKL_GLOBAL const uint64_t* longs;UKL_GLOBAL const ValueRecord* values;UKL_GLOBAL const StringRecord* strings;UKL_GLOBAL const NfaState* nfaStates;UKL_GLOBAL const uint32_t* nfaData;UKL_GLOBAL const NfaRange* nfaRanges;UKL_GLOBAL const LuaInstruction* luaInstructions;UKL_GLOBAL const LuaMember* luaMembers;UKL_GLOBAL uint32_t* valueWork;UKL_GLOBAL const uint8_t* stringBytes;UKL_GLOBAL const uint8_t* source;UKL_GLOBAL const Assertion* assertions;UKL_GLOBAL const uint32_t* assertionData;UKL_GLOBAL Link* captures;UKL_GLOBAL Link* cursors;UKL_GLOBAL RepeatLink* repeats;UKL_GLOBAL QueuedState* queuedStates;UKL_GLOBAL GreedyEntry* greedyEntries;UKL_GLOBAL Pending* pending;UKL_GLOBAL Result* results;UKL_PRIVATE const LineTable* lineStarts;Task task;File file;Program program;Node node;UKL_GLOBAL Evaluation* evaluation;UKL_GLOBAL GlobalArenas* globalArenas;uint32_t valueBase;uint32_t valueCapacity;UKL_GLOBAL Status* status;UKL_PRIVATE Path* path;UKL_PRIVATE uint32_t* pendingCount;UKL_PRIVATE uint32_t* captureCount;UKL_PRIVATE uint32_t* cursorCount;UKL_PRIVATE uint32_t* repeatCount;UKL_PRIVATE uint32_t* resultCount;UKL_PRIVATE uint32_t* candidateHead;UKL_PRIVATE uint32_t* localOrdinal;UKL_PRIVATE bool* keep;UKL_PRIVATE bool* advance;} JitContext;",
    );
    // The names an opcode body reads, from the context (`run_task_in`'s, which the bodies are written in).
    let unpack = "UKL_PRIVATE const NodeTable* nodes=context->nodes;UKL_GLOBAL const uint8_t* children=context->children;UKL_GLOBAL const Supertype* supertypes=context->supertypes;UKL_GLOBAL const uint32_t* words=context->words;UKL_GLOBAL const uint64_t* longs=context->longs;UKL_GLOBAL const ValueRecord* values=context->values;UKL_GLOBAL const StringRecord* strings=context->strings;UKL_GLOBAL const NfaState* nfaStates=context->nfaStates;UKL_GLOBAL const uint32_t* nfaData=context->nfaData;UKL_GLOBAL const NfaRange* nfaRanges=context->nfaRanges;UKL_GLOBAL const LuaInstruction* luaInstructions=context->luaInstructions;UKL_GLOBAL const LuaMember* luaMembers=context->luaMembers;UKL_GLOBAL uint32_t* valueWork=context->valueWork;UKL_GLOBAL const uint8_t* stringBytes=context->stringBytes;UKL_GLOBAL const uint8_t* source=context->source;UKL_GLOBAL const Assertion* assertions=context->assertions;UKL_GLOBAL const uint32_t* assertionData=context->assertionData;UKL_GLOBAL Link* captures=context->captures;UKL_GLOBAL Link* cursors=context->cursors;UKL_GLOBAL RepeatLink* repeats=context->repeats;UKL_GLOBAL QueuedState* queuedStates=context->queuedStates;UKL_GLOBAL GreedyEntry* greedyEntries=context->greedyEntries;UKL_GLOBAL Pending* pending=context->pending;UKL_GLOBAL Result* results=context->results;UKL_PRIVATE const LineTable* lineStarts=context->lineStarts;Task task=context->task;File file=context->file;Program program=context->program;Node node=context->node;UKL_GLOBAL Evaluation* evaluation=context->evaluation;UKL_GLOBAL GlobalArenas* globalArenas=context->globalArenas;uint32_t valueBase=context->valueBase,valueCapacity=context->valueCapacity;UKL_GLOBAL Status* status=context->status;";
    let copy_in: String = JIT_STATE.iter().map(|(ty, name)| format!("{ty} {name}=*context->{name};")).collect();
    let copy_out: String = JIT_STATE.iter().map(|(_, name)| format!("*context->{name}={name};")).collect();
    // One step function an opcode the programs use, its body the interpreter's case.
    let mut used = std::collections::BTreeSet::new();
    let mut lowered = Vec::with_capacity(programs.len());
    for program in programs {
        let native = native_ir::lower(program).map_err(|_| Error::BackendFailure)?;
        for block in &native.structural {
            let (opcode, _, _) = block.operation.encoded();
            if !bodies.contains_key(&opcode) {
                return Err(Error::InvalidOpcode(usize::from(block.position)));
            }
            if opcode != TEXT && opcode != ASSERT {
                used.insert(opcode);
            }
        }
        lowered.push(native);
    }
    for opcode in &used {
        let body = bodies[opcode];
        source.push_str(&format!(
            "UKL_COLD bool jit_step_{opcode}(UKL_PRIVATE JitContext* context,uint32_t mode,uint32_t operand){{{unpack}{copy_in}const uint32_t op={opcode}u;(void)op;{{{}}}{copy_out}return true;}}",
            body.strip_suffix("break;")
                .unwrap_or(body)
                .replace("return;", "return false;")
        ));
    }
    for (program_index, program) in programs.iter().enumerate() {
        for value in 0..program.value_programs.len() {
            source.push_str(&format!(
                "UKL_COLD bool jit_value_{program_index}_{value}(UKL_PRIVATE JitContext* context){{uint32_t outcome=evaluate_text(context->values,context->strings,context->nfaStates,context->nfaData,context->nfaRanges,context->luaInstructions,context->luaMembers,context->valueWork,context->stringBytes,context->source,context->program,context->node,context->task,{value}u,&context->path->steps,context->evaluation,context->globalArenas,context->valueBase,context->valueCapacity);if(outcome==2u){{context->status->error=6u;return false;}}if(outcome==3u){{context->status->error=4u;return false;}}if(outcome==4u){{context->status->error=2u;return false;}}*context->keep=outcome==1u;return true;}}"
            ));
        }
        for capture in 0..program.capture_programs.len() {
            source.push_str(&format!(
                "UKL_COLD bool jit_capture_{program_index}_{capture}(UKL_PRIVATE JitContext* context){{uint32_t outcome=evaluate_assertion(context->assertions[context->program.captureStart+{capture}u],context->assertionData,context->captures,context->path->captures,context->task,context->file,context->program,context->nodes,context->supertypes,context->source,context->strings,context->stringBytes,context->values,context->nfaStates,context->nfaData,context->nfaRanges,context->luaInstructions,context->luaMembers,context->valueWork,context->lineStarts,&context->path->steps,context->evaluation,context->globalArenas,context->valueBase,context->valueCapacity);if(outcome==2u){{context->status->error=6u;return false;}}if(outcome==3u){{context->status->error=4u;return false;}}if(outcome==4u){{context->status->error=2u;return false;}}*context->keep=outcome==1u;return true;}}"
            ));
        }
    }
    source.push_str("typedef struct JitOutputContext{UKL_GLOBAL const uint32_t* assertionData;Program program;Task task;File file;UKL_PRIVATE const NodeTable* nodes;UKL_GLOBAL const Link* captures;uint32_t head;uint32_t captureCount;UKL_GLOBAL FinalCapture* finalCaptures;UKL_PRIVATE uint32_t* finalCount;UKL_PRIVATE uint32_t* captureStart;UKL_GLOBAL uint8_t* storage;UKL_PRIVATE uint32_t* propertyCount;UKL_PRIVATE uint32_t* byteCount;UKL_GLOBAL const uint8_t* source;UKL_PRIVATE const LineTable* lineStarts;UKL_GLOBAL const ValueRecord* values;UKL_GLOBAL const NfaState* nfaStates;UKL_GLOBAL const uint32_t* nfaData;UKL_GLOBAL const NfaRange* nfaRanges;UKL_GLOBAL uint32_t* valueWork;uint32_t resultIndex;UKL_PRIVATE uint32_t* resultProperties;UKL_PRIVATE uint32_t* dynamicProperties;UKL_PRIVATE uint32_t* steps;UKL_GLOBAL Evaluation* evaluation;UKL_GLOBAL Evaluation* sharedEvaluation;UKL_GLOBAL GlobalArenas* globalArenas;uint32_t valueBase;uint32_t valueCapacity;} JitOutputContext;");
    for program_index in 0..programs.len() {
        source.push_str(&format!(
            "UKL_COLD uint32_t jit_output_{program_index}(UKL_PRIVATE JitOutputContext* c){{return apply_output(c->assertionData,c->program,c->task,c->file,c->nodes,c->captures,c->head,c->captureCount,c->finalCaptures,c->finalCount,c->captureStart,c->storage,c->propertyCount,c->byteCount,c->source,c->lineStarts,c->values,c->nfaStates,c->nfaData,c->nfaRanges,c->valueWork,c->resultIndex,c->resultProperties,c->dynamicProperties,c->steps,c->evaluation,c->sharedEvaluation,c->globalArenas,c->valueBase,c->valueCapacity);}}"
        ));
    }
    source.push_str("UKL_INLINE uint32_t jit_apply_output(UKL_PRIVATE JitOutputContext* context){switch(context->task.program){");
    for program_index in 0..programs.len() {
        source.push_str(&format!("case {program_index}u:return jit_output_{program_index}(context);"));
    }
    source.push_str("default:return 2u;}}");
    for (program_index, (program, native)) in programs.iter().zip(&lowered).enumerate() {
        source.push_str(&format!(
            "UKL_COLD bool jit_program_{program_index}(UKL_PRIVATE JitContext* context){{switch(context->path->pc){{"
        ));
        for block in &native.structural {
            let pc = block.position;
            let (opcode, mode, operand) = block.operation.encoded();
            if opcode == TEXT {
                source.push_str(&format!("case {pc}u:return jit_value_{program_index}_{operand}(context);"));
            } else if opcode == ASSERT && !program.tree_sitter_query {
                // An ordinary program tests the capture predicate where it asserts it, on the captures so far, as the
                // CPU's (`ASSERT`); a tree-sitter query's are tested on its candidates at the task's end.
                source.push_str(&format!("case {pc}u:return jit_capture_{program_index}_{operand}(context);"));
            } else if opcode == ASSERT {
                source.push_str(&format!("case {pc}u:return true;"));
            } else {
                source.push_str(&format!("case {pc}u:return jit_step_{opcode}(context,{mode}u,{operand}u);"));
            }
        }
        source.push_str("default:context->status->error=5u;return false;}}");
    }
    Ok(source)
}

fn jit_kernel_source(programs: &[Program]) -> Result<String, Error> {
    let bodies = opcode_bodies()?;
    let mut dispatch = String::from("switch(task.program){");
    for program_index in 0..programs.len() {
        dispatch.push_str(&format!(
            "case {program_index}u:if(!jit_program_{program_index}(&context))return;break;"
        ));
    }
    dispatch.push_str("default:status->error=5u;return;}");
    Ok(format!("{}{}", *TAPE_PREFIX, kernel_source("", Some(&dispatch), &jit_support(&bodies, programs)?, JIT_CONTEXT, JIT_OUTPUT_CONTEXT)))
}

const INTERPRETER_FETCH: &str =
    "uint32_t word=words[program.wordStart+path.pc];op=word&255u;mode=(word>>8u)&255u;operand=word>>16u;";

/// The interpreter over resident tapes (the label passes'), compiled once (until the residency's
/// drop releases it with the other pipelines' caches: it is held by the label runs' callers).
pub(super) fn compile_resident_interpreter_cached() -> Result<Engine, Error> {
    static INTERPRETER: Mutex<Option<Engine>> = Mutex::new(None);
    let mut cached = INTERPRETER.lock().map_err(|_| Error::BackendFailure)?;
    if let Some(engine) = &*cached {
        return Ok(engine.clone());
    }
    let source = format!("{}{}", *RESIDENT_PREFIX, kernel_source(INTERPRETER_FETCH, None, "", "", ""));
    let engine = Engine::Interpreter { pipeline: rerail_metal::runtime::pipeline_named_fixed(&source, "run_batch", None)?, buffers: Arc::new(Mutex::new(None)) };
    *cached = Some(engine.clone());
    Ok(engine)
}

pub fn compile_interpreter() -> Result<Engine, Error> {
    Ok(Engine::Interpreter {
        pipeline: rerail_metal::runtime::pipeline_named_fixed(&format!("{}{}", *TAPE_PREFIX, kernel_source(INTERPRETER_FETCH, None, "", "", "")), "run_batch", None)?,
        buffers: Arc::new(Mutex::new(None)),
    })
}

/// The interpreter with a recursive group's sweep kernel (`label_passes`, `label_sweep`), compiled
/// once per process. A program whose flags carry a route (bit 31; bit 30 captured nodes, bits 8..30
/// the label) marks its label as each result is published; other programs run as in `run_batch`.
pub(in crate::code_rule_dsl) fn compile_sweep_cached() -> Result<ComputePipelineState, Error> {
    static SWEEP: Mutex<Option<ComputePipelineState>> = Mutex::new(None);
    let mut cached = SWEEP.lock().map_err(|_| Error::BackendFailure)?;
    if let Some(pipeline) = &*cached {
        return Ok(pipeline.clone());
    }
    // The label passes' sweep, over resident tapes as their interpreter.
    let source = format!("{}{}", *RESIDENT_PREFIX, kernel_source(INTERPRETER_FETCH, None, "", "", ""));
    let edit = |source: String, from: &str, to: &str| {
        assert_eq!(source.matches(from).count(), 1, "the runtime has one {from}");
        source.replacen(from, to, 1)
    };
    // Labels marked in this sweep must be read as they are marked, not through a read-only cache.
    // (A program's route marks its labels as it publishes: `label_mark` in `run_task_in`.)
    let source = edit(
        source,
        "UKL_GLOBAL const uint32_t* bits=(UKL_GLOBAL const uint32_t*)(values+1);return ((bits[(kind-16384u)*words+node/32u]>>(node%32u))&1u)!=0u;",
        "UKL_GLOBAL uint32_t* bits=(UKL_GLOBAL uint32_t*)(values+1);return ((ukl_atomic_load(&bits[(kind-16384u)*words+node/32u])>>(node%32u))&1u)!=0u;",
    );
    // The sweep's kernel takes the batch's rows (`run_batch`'s), after the runtime it runs tasks with.
    let pipeline = rerail_metal::runtime::pipeline_named_fixed(&format!("{source}\n{}", include_str!("label_sweep.uglykl")), "label_sweep", None)?;
    *cached = Some(pipeline.clone());
    Ok(pipeline)
}

pub(super) fn compile_jit(programs: &[Program]) -> Result<Engine, Error> {
    Ok(Engine::Jit {
        pipeline: pipeline_named(&jit_kernel_source(programs)?, "run_batch")?,
        programs: programs.len(),
        buffers: Arc::new(Mutex::new(None)),
    })
}

pub(super) fn jit_cache_key(programs: &[Program]) -> Result<String, Error> {
    Ok(format!(
        "metal-jit:{FORMAT_VERSION}:{}:{}",
        std::env::consts::ARCH,
        jit_kernel_source(programs)?
    ))
}

pub(super) fn program_buffer_key(programs: &[Program]) -> Result<ProgramBufferKey, Error> {
    Ok(ProgramBufferKey::from(&PackedBatch::new_in_place(programs, &[], &[])?))
}

/// The interpreter, compiled once per process: a lowered matcher dispatches it once per pass.
pub(in crate::code_rule_dsl) fn compile_interpreter_cached() -> Result<Engine, Error> {
    static INTERPRETER: Mutex<Option<Engine>> = Mutex::new(None);
    let mut cached = INTERPRETER.lock().map_err(|_| Error::BackendFailure)?;
    if let Some(engine) = &*cached {
        return Ok(engine.clone());
    }
    let engine = compile_interpreter()?;
    *cached = Some(engine.clone());
    Ok(engine)
}

/// Whether `programs`' kernel is compiled (`compile_jit_cached`), without compiling it.
pub(in crate::code_rule_dsl) fn jit_ready(programs: &[Program]) -> Result<bool, Error> {
    let key = jit_cache_key(programs)?;
    Ok(JIT_CACHE.lock().map_err(|_| Error::BackendFailure)?.contains_key(&key))
}

pub(in crate::code_rule_dsl) fn compile_jit_cached(programs: &[Program]) -> Result<Engine, Error> {
    let key = jit_cache_key(programs)?;
    if let Some(engine) = JIT_CACHE
        .lock()
        .map_err(|_| Error::BackendFailure)?
        .get(&key)
        .cloned()
    {
        return Ok(engine);
    }
    let engine = compile_jit(programs)?;
    let mut cache = JIT_CACHE.lock().map_err(|_| Error::BackendFailure)?;
    if let Some(existing) = cache.get(&key) {
        return Ok(existing.clone());
    }
    if cache.len() == METAL_JIT_CACHE_LIMIT
        && let Some(oldest) = cache.keys().next().cloned()
    {
        cache.remove(&oldest);
    }
    cache.insert(key, engine.clone());
    Ok(engine)
}

pub(super) fn execute(
    engine: &Engine,
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
) -> Result<Vec<Vec<Vec<Output>>>, Error> {
    execute_outcomes(engine, programs, files, anchors)?.into_result()
}

pub(in crate::code_rule_dsl) fn execute_outcomes(
    engine: &Engine,
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
) -> Result<BatchOutcome, Error> {
    execute_packed(engine, programs, files, anchors)?.into_outcome()
}

pub(super) fn execute_packed(
    engine: &Engine,
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
) -> Result<PackedExecution, Error> {
    execute_packed_with_result_limits(engine, programs, files, anchors, None)
}

pub fn execute_packed_with_result_limits(
    engine: &Engine,
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
    result_limits: Option<&[u32]>,
) -> Result<PackedExecution, Error> {
    execute_packed_budgeted(engine, programs, files, anchors, result_limits, super::Budget::current())
}

/// The batch with `budget` (decision 0334): an evaluation that outgrows its steps or states restarts with twice as
/// much (`restarting`).
pub fn execute_packed_budgeted(
    engine: &Engine,
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
    result_limits: Option<&[u32]>,
    budget: super::Budget,
) -> Result<PackedExecution, Error> {
    super::restarting(programs, anchors, budget, &mut |anchors, budget, growth| execute_packed_once(engine, programs, files, anchors, result_limits, budget, growth))
}

fn execute_packed_once(
    engine: &Engine,
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
    result_limits: Option<&[u32]>,
    budget: super::Budget,
    growth: &super::StateGrowth,
) -> Result<PackedExecution, Error> {
    let _evaluation = super::metal_backend::evaluation_scope();
    if matches!(engine, Engine::Jit { programs: count, .. } if *count != programs.len()) {
        return Err(Error::InvalidAnchor);
    }
    // The tasks with no node records: the device reads the documents in the slabs (`DocTable`).
    let mut batch = PackedBatch::new_in_place(programs, files, anchors).map_err(|error| {
        if std::env::var_os("TEMPO_PACKED_BATCH_PROFILE").is_some() {
            eprintln!("TEMPO_PACKED_BATCH_ERROR stage=pack error={error:?}");
        }
        error
    })?;
    batch.apply_budget(budget, growth);
    if batch.tasks.is_empty() {
        return Ok(PackedExecution::new(batch, PackedResultPage::empty()));
    }
    let allocation_started = std::time::Instant::now();
    let device = METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
    let queue = rerail_metal::queue()?;
    // Every lane reserves the widest evaluation's states: a workspace the pool refuses runs with half the lanes,
    // down to one (decision 0334 §9). Fewer lanes run its tasks in fewer at a time; the results are the same.
    let mut lanes = EXECUTION_LANES;
    let (totals, workspace) = loop {
        let totals = batch
            .assign_workspace_with_result_limits_and_lanes(programs, result_limits, lanes)
            .map_err(|error| {
                if std::env::var_os("TEMPO_PACKED_BATCH_PROFILE").is_some() {
                    eprintln!("TEMPO_PACKED_BATCH_ERROR stage=workspace error={error:?}");
                }
                error
            })?;
        if std::env::var_os("TEMPO_PACKED_BATCH_PROFILE").is_some() {
            eprintln!(
                "TEMPO_PACKED_BATCH_PROFILE backend=metal programs={} tasks={} lanes={lanes} workspaceBytes={} totals={totals:?}",
                programs.len(),
                batch.tasks.len(),
                batch.workspace_allocation_bytes(totals).unwrap_or(u64::MAX),
            );
        }
        match MetalWorkspaceLease::acquire(
            device,
            totals,
            batch.header.scheduler_byte_count,
            batch.header.workspace_byte_count,
            batch.tasks.len(),
        ) {
            Err(Error::BackendCapacity) if lanes > 1 => {
                crate::code_rule_dsl::prof::count("metal.fewer_lanes", 1);
                lanes /= 2;
            }
            workspace => break (totals, workspace?),
        }
    };
    for evaluation in &mut batch.evaluations {
        evaluation.generation = workspace.generation;
    }
    for lane in &mut batch.evaluation_lanes {
        lane.generation = workspace.generation;
    }
    let program_buffers = engine.program_buffers(device, &batch)?;
    let task_buffer = MappedBuffer::from_slice(device, &batch.tasks)?;
    let header_buffer = MappedBuffer::from_slice(device, &[batch.header])?;
    // The documents resident, gathered into one block (decision 0289 §4), in the batch's numbering: their views
    // where the files were, and the block where the node records were. The kernel reads nothing from the
    // children, source and line start slots; the block stands in for them.
    let table = crate::code_rule_dsl::residency::slabs::admit_docs(files)?;
    let table = crate::code_rule_dsl::residency::slabs::DocTable::one_block(table).ok_or(Error::BackendCapacity)?;
    if table.nodes != files.iter().map(TapeDoc::transducer_len).sum::<usize>() {
        return Err(Error::BackendFailure);
    }
    let file_buffer = MappedBuffer::from_slice(device, &table.views)?;
    let block = table.block.as_ref().ok_or(Error::BackendFailure)?.buffer();
    // The derived classes alone (`has_supertype`): the classes are the codes', read on the tape.
    let supertypes = batch.metal_supertypes();
    let supertype_buffer = MappedBuffer::from_slice(device, &supertypes)?;
    let pending_buffer = &workspace.pending;
    let capture_buffer = &workspace.captures;
    let cursor_buffer = &workspace.cursors;
    let repeat_buffer = &workspace.repeats;
    let scheduler_buffer = &workspace.scheduler;
    unsafe {
        std::ptr::copy_nonoverlapping(
            std::ptr::from_ref(&batch.global_arenas).cast::<u8>(),
            scheduler_buffer.buffer.contents().cast::<u8>(),
            std::mem::size_of_val(&batch.global_arenas),
        );
        let evaluation_start = std::mem::size_of_val(&batch.global_arenas);
        std::ptr::copy_nonoverlapping(
            batch.evaluations.as_ptr().cast::<u8>(),
            scheduler_buffer
                .buffer
                .contents()
                .cast::<u8>()
                .add(evaluation_start),
            std::mem::size_of_val(batch.evaluations.as_slice()),
        );
        std::ptr::copy_nonoverlapping(
            batch.evaluation_lanes.as_ptr().cast::<u8>(),
            scheduler_buffer
                .buffer
                .contents()
                .cast::<u8>()
                .add(evaluation_start + std::mem::size_of_val(batch.evaluations.as_slice())),
            std::mem::size_of_val(batch.evaluation_lanes.as_slice()),
        );
    }
    let result_buffer = &workspace.results;
    let status_buffer = &workspace.statuses;
    let value_state_buffer = &workspace.value_states;
    let final_capture_buffer = &workspace.final_captures;
    let output_storage_buffer = &workspace.output_storage;
    let allocation_elapsed = allocation_started.elapsed();
    if std::env::var_os("TEMPO_PACKED_BATCH_PROFILE").is_some() {
        eprintln!(
            "TEMPO_METAL_WORKSPACE reused={} generation={}",
            workspace.reused, workspace.generation,
        );
    }
    let command = queue.new_command_buffer();
    command.set_label("Tempo packed transducer batch");
    let encoder = command.new_compute_command_encoder();
    encoder.set_label("Tempo packed transducer kernel");
    encoder.set_compute_pipeline_state(engine.pipeline());
    for (index, buffer) in [
        &task_buffer.buffer,
        &header_buffer.buffer,
        &file_buffer.buffer,
        block,
        &program_buffers.programs.buffer,
        &program_buffers.words.buffer,
        &program_buffers.longs.buffer,
        block,
        &supertype_buffer.buffer,
        &pending_buffer.buffer,
        &capture_buffer.buffer,
        &cursor_buffer.buffer,
        &repeat_buffer.buffer,
        &scheduler_buffer.buffer,
        &result_buffer.buffer,
        &status_buffer.buffer,
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
        block,
        &value_state_buffer.buffer,
        block,
        &final_capture_buffer.buffer,
        &output_storage_buffer.buffer,
    ]
    .into_iter()
    .enumerate()
    {
        encoder.set_buffer(index as u64, Some(buffer), 0);
    }
    let width = engine.pipeline().thread_execution_width().max(1);
    encoder.checked_dispatch_threads(
        command,
        MTLSize::new(batch.evaluation_lanes.len() as u64, 1, 1),
        MTLSize::new(width.min(batch.evaluation_lanes.len() as u64), 1, 1),
    );
    encoder.end_encoding();
    let execution_started = std::time::Instant::now();
    commit_checked(command)?;
    wait_checked(command)?;
    let execution_elapsed = execution_started.elapsed();
    if std::env::var_os("TEMPO_PACKED_BATCH_PROFILE").is_some() {
        eprintln!(
            "TEMPO_PACKED_BATCH_PHASES allocationNanoseconds={} executionNanoseconds={}",
            allocation_elapsed.as_nanos(),
            execution_elapsed.as_nanos(),
        );
    }
    if command.status() == metal::MTLCommandBufferStatus::Error {
        return Err(Error::BackendFailure);
    }
    #[cfg(any(test, feature = "test-hooks"))]
    INVOCATIONS.set(INVOCATIONS.get() + 1);
    batch.global_arenas = unsafe {
        scheduler_buffer
            .buffer
            .contents()
            .cast::<MetalGlobalArenas>()
            .read()
    };
    let statuses = unsafe {
        std::slice::from_raw_parts(
            status_buffer.buffer.contents().cast::<TaskStatus>(),
            batch.tasks.len(),
        )
    };
    if std::env::var_os("TEMPO_PACKED_BATCH_PROFILE").is_some() {
        let mut work = BTreeMap::<u32, (u64, u32)>::new();
        for (task, status) in batch.tasks.iter().zip(statuses) {
            let entry = work.entry(task.program).or_default();
            entry.0 += u64::from(status.reserved);
            entry.1 += 1;
        }
        let mut work = work.into_iter().collect::<Vec<_>>();
        work.sort_by_key(|(_, (total, _))| std::cmp::Reverse(*total));
        eprintln!(
            "TEMPO_PACKED_TASK_WORK {:?}",
            work.into_iter().take(12).collect::<Vec<_>>()
        );
    }
    let results = unsafe {
        std::slice::from_raw_parts(
            result_buffer.buffer.contents().cast::<ResultRecord>(),
            totals
                .candidates
                .checked_add(totals.results)
                .ok_or(Error::BackendCapacity)? as usize,
        )
    };
    let final_captures = unsafe {
        std::slice::from_raw_parts(
            final_capture_buffer
                .buffer
                .contents()
                .cast::<FinalCaptureRecord>(),
            totals.output_captures as usize,
        )
    };
    let output_storage = unsafe {
        std::slice::from_raw_parts(
            output_storage_buffer.buffer.contents().cast::<u8>(),
            batch.header.workspace_byte_count as usize,
        )
    };
    let properties = unsafe {
        std::slice::from_raw_parts(
            output_storage_buffer
                .buffer
                .contents()
                .cast::<PropertyRecord>(),
            totals.properties as usize,
        )
    };
    let page = PackedResultPage::compact(
        &batch,
        statuses,
        results,
        final_captures,
        properties,
        output_storage,
    )?;
    Ok(PackedExecution::new(batch, page))
}
