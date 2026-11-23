use rerail_blocks::vector::Vector;
use rerail_metal::lane::{self, HostMapped};
use super::metal_batch_abi::{
    FORMAT_VERSION, FinalCaptureRecord, LinkRecord as CaptureRecord, MetalGlobalArenas,
    PendingRecord, PropertyRecord, RepeatRecord, ResultRecord, TaskStatus,
};
use super::packed_batch::{PackedBatch, PackedExecution, PackedResultPage};
use super::packed_workspace::WorkspaceTotals;
use super::*;
use crate::code_rule_dsl::tape::TapeDoc;
#[cfg(any(test, feature = "test-hooks"))]
use std::cell::Cell;
use std::collections::{BTreeMap, HashMap};
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex};

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
// engine registers as it loads (the languages, their `.rerail` libraries).
#[path = "metal_set_steps.rs"]
pub mod set_steps;
// Block programs on Metal: the Metal crate's.
pub use rerail_metal_programs::block_program;
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
#[derive(Default)]
struct MetalPrograms(Mutex<BTreeMap<String, Engine>>);
const METAL_WORKSPACE_POOL_LIMIT: usize = 4;
#[derive(Default)]
struct MetalWorkspaces(Mutex<Vec<MetalWorkspace>>);

/// Frees the workspaces kept for reuse; whether there were any. Kept workspaces count against the
/// buffer budget, so a run that misses capacity frees them before anything else (`label_passes::run`).
pub(super) fn release_workspaces() -> bool {
    let held_ = crate::code_rule_dsl::logic::profile::machine::<MetalWorkspaces>();
    let released = held_.0.lock().map(|mut pool| std::mem::take(&mut *pool)).unwrap_or_default();
    !released.is_empty()
}

/// The pool's evictor for the kept workspaces (`gpu::shared::set_evictor`, Q203): one released, the largest, where
/// the pool is not being taken from now (it is called from inside allocations, the pool's lock perhaps this thread's).
/// Kept between waves, they held the budget's room until a wave was refused and freed them all.
fn evict_workspace() -> bool {
    let held_ = crate::code_rule_dsl::logic::profile::machine::<MetalWorkspaces>();
    let Ok(mut pool) = held_.0.try_lock() else { return false };
    let Some(at) = (0..pool.len()).max_by_key(|&i| MetalWorkspace::bytes_for(pool[i].totals, pool[i].scheduler_bytes, pool[i].output_storage_bytes, pool[i].task_capacity)) else { return false };
    let gone = pool.swap_remove(at);
    drop(pool);
    drop(gone);
    crate::code_rule_dsl::prof::count("residency.workspace_evicted", 1);
    true
}

struct MetalWorkspace {
    totals: WorkspaceTotals,
    scheduler_bytes: u32,
    output_storage_bytes: u32,
    task_capacity: usize,
    generation: u32,
    pending: HostMapped,
    captures: HostMapped,
    cursors: HostMapped,
    repeats: HostMapped,
    scheduler: HostMapped,
    results: HostMapped,
    statuses: HostMapped,
    value_states: HostMapped,
    final_captures: HostMapped,
    output_storage: HostMapped,
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

    /// The profile's counts of a new workspace's buffers, in MB.
    fn count_buffer_sizes(totals: WorkspaceTotals, scheduler_bytes: u32, output_storage_bytes: u32) {
        use crate::code_rule_dsl::prof::count;
        use std::mem::size_of;
        count("ws.new", 1);
        count("ws.pending_mb", (totals.pending as usize * size_of::<PendingRecord>()) >> 20);
        count("ws.captures_mb", (totals.captures as usize * size_of::<CaptureRecord>()) >> 20);
        count("ws.cursors_mb", (totals.cursors as usize * size_of::<CaptureRecord>()) >> 20);
        count("ws.repeats_mb", (totals.repeats as usize * size_of::<RepeatRecord>()) >> 20);
        count("ws.scheduler_mb", scheduler_bytes as usize >> 20);
        count("ws.results_mb", (totals.results as usize * size_of::<ResultRecord>()) >> 20);
        count("ws.value_states_mb", (totals.value_states as usize * 4) >> 20);
        count("ws.output_mb", output_storage_bytes as usize >> 20);
        count("ws.final_captures_mb", (totals.output_captures as usize * size_of::<FinalCaptureRecord>()) >> 20);
    }

    fn new(
        totals: WorkspaceTotals,
        scheduler_bytes: u32,
        output_storage_bytes: u32,
        task_capacity: usize,
    ) -> Result<Self, Error> {
        if crate::code_rule_dsl::prof::on() {
            Self::count_buffer_sizes(totals, scheduler_bytes, output_storage_bytes);
        }
        Ok(Self {
            totals,
            scheduler_bytes,
            output_storage_bytes,
            task_capacity,
            generation: 1,
            pending: HostMapped::zeroed::<PendingRecord>(totals.pending as usize)?,
            captures: HostMapped::zeroed::<CaptureRecord>(totals.captures as usize)?,
            cursors: HostMapped::zeroed::<CaptureRecord>(totals.cursors as usize)?,
            repeats: HostMapped::zeroed::<RepeatRecord>(totals.repeats as usize)?,
            scheduler: HostMapped::zeroed::<u8>(scheduler_bytes as usize)?,
            results: HostMapped::zeroed::<ResultRecord>(totals.candidates.checked_add(totals.results).ok_or(Error::BackendCapacity)? as usize)?,
            statuses: HostMapped::zeroed::<TaskStatus>(task_capacity)?,
            value_states: HostMapped::zeroed::<u32>(totals.value_states as usize)?,
            final_captures: HostMapped::zeroed::<FinalCaptureRecord>(totals.output_captures as usize)?,
            output_storage: HostMapped::zeroed::<u8>(output_storage_bytes as usize)?,
        })
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
            .map(|buffer| buffer.byte_length())
            .sum()
    }

    fn acquire(
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
        // The workspaces' evictor, registered once: code the pool calls (`gpu::shared::set_evictor`), which reads the
        // machine's workspaces.
        static EVICTOR: std::sync::Once = std::sync::Once::new();
        EVICTOR.call_once(|| rerail_accel::gpu::shared::set_evictor(evict_workspace));
        let held_ = crate::code_rule_dsl::logic::profile::machine::<MetalWorkspaces>();
        let mut pool = held_.0.lock().unwrap();
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
        let held_ = crate::code_rule_dsl::logic::profile::machine::<MetalWorkspaces>();
        let mut pool = held_.0.lock().unwrap();
        if pool.len() < METAL_WORKSPACE_POOL_LIMIT {
            pool.push(workspace);
        }
    }
}

const BATCH_ENTRY: &str = include_str!("../../../../../../src/code_rule_dsl/transducer/packed/batch_runtime.uglykl");
/// The interpreter of resident label batches (tasks from roots, not records: decision 0033), over a batch of tapes in
/// `format` (0: mixed) as the label kernels read it ([`tape_view`]): the format told once, at the compile (decision
/// 0576). A run places its tapes in format 15 or 16 (a 16z tape as 16), so its interpreter and sweep hold one
/// format's reads, and no 16z read.
fn resident_prefix(format: u32) -> String {
    format!("#define RERAIL_RESIDENT 1\n{}", tape_view(format))
}

/// The device's tape view (decision 0032), which the label and pattern kernels read: the
/// document table's entry as generated (`slabs::msl`, decisions 0289 §4 and 0310), the reads written once in CLIF,
/// then the readers. The reads are format 15's and 16's (`<ukl/reads/tape.h>`), or 16z's too, through the
/// batch's cache (`<ukl/reads/tape_z.h>`, decision 0353 step 4b) for a batch of 16z or of mixed formats
/// ([`tape_view`]): a pipeline's 16z bodies are compiled though its format makes them dead, and doubled a format
/// 15 label kernel's compile.
///
/// The batch's tape format is the program's specialization constant (`tape_format`, `UKL_CONSTANT_U32`: Metal's
/// function constant 0, a define elsewhere; 0 for a mixed batch): each view load of the format field reads it, so a
/// batch of one format runs reads without the other format's branches (t41: label_micro 388 to about 300 ms at
/// s1000). Every kernel of these sources is made with a value ([`pipeline_of`], [`pipelines`]).
pub(super) static VIEW_PIECES: std::sync::LazyLock<[String; 2]> = std::sync::LazyLock::new(|| {
    ["#include <ukl/reads/tape.h>\n", "#include <ukl/reads/tape_z.h>\n"].map(|reads| {
        format!(
            "{}UKL_CONSTANT_U32(tape_format)\n#define clif_view(v,a) ((tape_format!=0u&&(a)=={}u)?tape_format:(v)[(a)/4u])\n{}{}",
            crate::code_rule_dsl::residency::slabs::ukl(),
            std::mem::offset_of!(crate::code_rule_dsl::residency::slabs::DocView, format),
            reads,
            include_str!("../../../../../../src/code_rule_dsl/transducer/packed/device_tape.h")
        )
    })
});
/// Whether a batch of tapes in `format` (0: mixed) reads 16z ([`VIEW_PIECES`]).
pub(super) fn reads_z(format: u32) -> bool {
    format != 15 && format != 16
}
/// The tape view for a batch of tapes in `format` (0: mixed).
pub(super) fn tape_view(format: u32) -> &'static str {
    &VIEW_PIECES[usize::from(reads_z(format))]
}

#[derive(Clone, Eq, PartialEq)]
pub(super) struct ProgramBufferKey {
    programs: Vec<super::metal_batch_abi::MetalProgram>,
    words: Vector<u32>,
    longs: Vector<u64>,
    values: Vec<super::metal_batch_abi::ValueRecord>,
    strings: Vec<super::metal_batch_abi::MetalString>,
    nfa_states: Vec<super::metal_batch_abi::NfaStateRecord>,
    nfa_data: Vector<u32>,
    nfa_ranges: Vec<super::metal_batch_abi::NfaRangeRecord>,
    lua_instructions: Vec<super::metal_batch_abi::LuaInstructionRecord>,
    lua_members: Vec<super::metal_batch_abi::LuaSetMemberRecord>,
    assertions: Vec<super::packed_batch::AssertionRecord>,
    assertion_data: Vector<u32>,
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
    programs: HostMapped,
    words: HostMapped,
    longs: HostMapped,
    values: HostMapped,
    strings: HostMapped,
    nfa_states: HostMapped,
    nfa_data: HostMapped,
    nfa_ranges: HostMapped,
    lua_instructions: HostMapped,
    lua_members: HostMapped,
    assertions: HostMapped,
    assertion_data: HostMapped,
    string_bytes: HostMapped,
}

pub struct CachedProgramBuffers {
    key: ProgramBufferKey,
    buffers: Arc<ProgramBuffers>,
}

pub(super) type ProgramBufferCache = Arc<Mutex<Option<CachedProgramBuffers>>>;

#[derive(Clone)]
pub enum Engine {
    Interpreter {
        kernel: lane::Kernel,
        buffers: ProgramBufferCache,
    },
    Jit {
        kernel: lane::Kernel,
        programs: usize,
        /// The tape format the kernel reads (0: mixed), as the interpreter's (decision 0576 §8, §23).
        format: u32,
        buffers: ProgramBufferCache,
    },
}

impl Engine {
    fn kernel(&self) -> &lane::Kernel {
        match self {
            Self::Interpreter { kernel, .. } | Self::Jit { kernel, .. } => kernel,
        }
    }

    fn buffers(&self) -> &ProgramBufferCache {
        match self {
            Self::Interpreter { buffers, .. } | Self::Jit { buffers, .. } => buffers,
        }
    }

    fn program_buffers(
        &self,
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
            programs: HostMapped::from_slice(&batch.programs)?,
            words: HostMapped::from_slice(&batch.words)?,
            longs: HostMapped::from_slice(&batch.longs)?,
            values: HostMapped::from_slice(&batch.values)?,
            strings: HostMapped::from_slice(&batch.strings)?,
            nfa_states: HostMapped::from_slice(&batch.nfa_states)?,
            nfa_data: HostMapped::from_slice(&batch.nfa_data)?,
            nfa_ranges: HostMapped::from_slice(&batch.nfa_ranges)?,
            lua_instructions: HostMapped::from_slice(&batch.lua_instructions)?,
            lua_members: HostMapped::from_slice(&batch.lua_members)?,
            assertions: HostMapped::from_slice(&batch.assertions)?,
            assertion_data: HostMapped::from_slice(&batch.assertion_data)?,
            string_bytes: HostMapped::from_slice(&batch.string_bytes)?,
        });
        *slot = Some(CachedProgramBuffers {
            key,
            buffers: Arc::clone(&buffers),
        });
        Ok(buffers)
    }
}

fn kernel_source(fetch: &str, native_dispatch: Option<&str>, jit_support: &str, jit_context: &str) -> String {
    let mut source = BATCH_ENTRY
        .replace("/*FETCH*/", fetch)
        .replace("/*JIT_SUPPORT*/", jit_support)
        .replace("/*JIT_CONTEXT*/", jit_context)
        .replace("/*JIT_OUTPUT_CONTEXT*/", "");
    let begin = source
        .find("/*OP_SWITCH_BEGIN*/")
        .expect("Metal runtime has an opcode-switch start marker");
    let end = source
        .find("/*OP_SWITCH_END*/")
        .expect("Metal runtime has an opcode-switch end marker")
        + "/*OP_SWITCH_END*/".len();
    if let Some(dispatch) = native_dispatch {
        source.replace_range(begin..end, dispatch);
        source = source.replace("/*OUTPUT_BEGIN*/", "").replace("/*OUTPUT_END*/", "");
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
    // Each inlined: one call of each, in `jit_step`'s switch (on an Arc 140V 11.2 s for a kernel of 116 programs, 10.3 s
    // with the steps called).
    for opcode in &used {
        let body = bodies[opcode];
        source.push_str(&format!(
            "UKL_INLINE bool jit_step_{opcode}(UKL_PRIVATE JitContext* context,uint32_t mode,uint32_t operand){{{unpack}{copy_in}const uint32_t op={opcode}u;(void)op;{{{}}}{copy_out}return true;}}",
            body.strip_suffix("break;")
                .unwrap_or(body)
                .replace("return;", "return false;")
        ));
    }
    // A text test's and an asserted capture predicate's functions, numbered across the programs (the table's
    // operand for them).
    let mut values = Vec::new();
    let mut checks = Vec::new();
    for (program_index, program) in programs.iter().enumerate() {
        for value in 0..program.value_programs.len() {
            values.push((program_index, value));
            source.push_str(&format!(
                "UKL_COLD bool jit_value_{program_index}_{value}(UKL_PRIVATE JitContext* context){{uint32_t outcome=evaluate_text(context->values,context->strings,context->nfaStates,context->nfaData,context->nfaRanges,context->luaInstructions,context->luaMembers,context->valueWork,context->stringBytes,context->source,context->program,context->node,context->task,{value}u,&context->path->steps,context->evaluation,context->globalArenas,context->valueBase,context->valueCapacity);if(outcome==2u){{context->status->error=6u;return false;}}if(outcome==3u){{context->status->error=4u;return false;}}if(outcome==4u){{context->status->error=2u;return false;}}*context->keep=outcome==1u;return true;}}"
            ));
        }
        for capture in 0..program.capture_programs.len() {
            checks.push((program_index, capture));
            source.push_str(&format!(
                "UKL_COLD bool jit_capture_{program_index}_{capture}(UKL_PRIVATE JitContext* context){{uint32_t outcome=evaluate_assertion(context->assertions[context->program.captureStart+{capture}u],context->assertionData,context->captures,context->path->captures,context->task,context->file,context->program,context->nodes,context->supertypes,context->source,context->strings,context->stringBytes,context->values,context->nfaStates,context->nfaData,context->nfaRanges,context->luaInstructions,context->luaMembers,context->valueWork,context->lineStarts,&context->path->steps,context->evaluation,context->globalArenas,context->valueBase,context->valueCapacity);if(outcome==2u){{context->status->error=6u;return false;}}if(outcome==3u){{context->status->error=4u;return false;}}if(outcome==4u){{context->status->error=2u;return false;}}*context->keep=outcome==1u;return true;}}"
            ));
        }
    }
    // The programs' steps, one word a step at `JIT_START[program] + pc` (`JIT_NONE` where no block starts): the
    // opcode, its mode and its operand, as the interpreter's words; a text test `JIT_VALUE` and an asserted predicate
    // `JIT_CHECK` with the function's number, a tree-sitter query's assertion `JIT_PASS` (its predicates are tested on
    // its candidates at the task's end). Read by one switch that calls each step once (decision 0576 §23): a switch
    // per program, each case a call, was a kernel's code per program, and Intel's compiler took 45.8 s over the 116
    // programs of `all_backends_execute_every_verified_instruction_and_mode` (the steps are calls, so their operands,
    // constants in a case, were never folded in).
    let mut starts = Vec::with_capacity(programs.len() + 1);
    let mut table: Vec<u32> = Vec::new();
    let (mut value_at, mut check_at) = (0usize, 0usize);
    for (program_index, (program, native)) in programs.iter().zip(&lowered).enumerate() {
        starts.push(table.len() as u32);
        let len = native.structural.iter().map(|block| usize::from(block.position) + 1).max().unwrap_or(0);
        let base = table.len();
        table.resize(base + len, JIT_NONE);
        for block in &native.structural {
            let (opcode, mode, operand) = block.operation.encoded();
            let word = if opcode == TEXT {
                let k = values[value_at..].iter().position(|&(p, v)| p == program_index && v == usize::from(operand)).ok_or(Error::BackendFailure)? + value_at;
                u32::from(JIT_VALUE) | (k as u32) << 16
            } else if opcode == ASSERT && !program.tree_sitter_query {
                let k = checks[check_at..].iter().position(|&(p, c)| p == program_index && c == usize::from(operand)).ok_or(Error::BackendFailure)? + check_at;
                u32::from(JIT_CHECK) | (k as u32) << 16
            } else if opcode == ASSERT {
                u32::from(JIT_PASS)
            } else {
                u32::from(opcode) | u32::from(mode) << 8 | u32::from(operand) << 16
            };
            table[base + usize::from(block.position)] = word;
        }
        value_at += values[value_at..].iter().take_while(|&&(p, _)| p == program_index).count();
        check_at += checks[check_at..].iter().take_while(|&&(p, _)| p == program_index).count();
    }
    starts.push(table.len() as u32);
    if values.len() > 0xffff || checks.len() > 0xffff {
        return Err(Error::BackendCapacity);
    }
    let list = |v: &[u32]| v.iter().map(|w| format!("{w}u")).collect::<Vec<_>>().join(",");
    if table.is_empty() {
        table.push(JIT_NONE);
    }
    source.push_str(&format!(
        "UKL_TABLE uint32_t JIT_START[{}]={{{}}};UKL_TABLE uint32_t JIT_STEPS[{}]={{{}}};",
        starts.len(),
        list(&starts),
        table.len(),
        list(&table)
    ));
    source.push_str(&format!(
        "UKL_INLINE bool jit_step(UKL_PRIVATE JitContext* context){{uint32_t p=context->task.program,pc=context->path->pc;if(p>={programs}u||pc>=JIT_START[p+1u]-JIT_START[p]){{context->status->error=5u;return false;}}uint32_t word=JIT_STEPS[JIT_START[p]+pc],mode=(word>>8u)&255u,operand=word>>16u;switch(word&255u){{",
        programs = programs.len()
    ));
    for opcode in &used {
        source.push_str(&format!("case {opcode}u:return jit_step_{opcode}(context,mode,operand);"));
    }
    if !values.is_empty() {
        source.push_str(&format!("case {JIT_VALUE}u:switch(operand){{"));
        for (k, (p, v)) in values.iter().enumerate() {
            source.push_str(&format!("case {k}u:return jit_value_{p}_{v}(context);"));
        }
        source.push_str("default:break;}break;");
    }
    if !checks.is_empty() {
        source.push_str(&format!("case {JIT_CHECK}u:switch(operand){{"));
        for (k, (p, c)) in checks.iter().enumerate() {
            source.push_str(&format!("case {k}u:return jit_capture_{p}_{c}(context);"));
        }
        source.push_str("default:break;}break;");
    }
    source.push_str(&format!("case {JIT_PASS}u:return true;default:break;}}context->status->error=5u;return false;}}"));
    Ok(source)
}

/// The JIT's table words (`jit_support`) that are no opcode: no block, a text test, an asserted predicate, a
/// query's assertion.
const JIT_NONE: u32 = 255;
const JIT_VALUE: u8 = 254;
const JIT_CHECK: u8 = 253;
const JIT_PASS: u8 = 252;

/// The JIT's kernel for `programs` over a batch of tapes in `format` (0: mixed): the format's reads alone
/// ([`tape_view`]), as the interpreter's (decision 0576 §8, §23).
fn jit_kernel_source(programs: &[Program], format: u32) -> Result<String, Error> {
    let bodies = opcode_bodies()?;
    Ok(format!("{}{}", tape_view(format), kernel_source("", Some("if(!jit_step(&context))return;"), &jit_support(&bodies, programs)?, JIT_CONTEXT)))
}

const INTERPRETER_FETCH: &str =
    "uint32_t word=words[program.wordStart+path.pc];op=word&255u;mode=(word>>8u)&255u;operand=word>>16u;";

/// The interpreter over resident tapes (the label passes'), compiled once (until the residency's
/// drop releases it with the other pipelines' caches: it is held by the label runs' callers).
pub(super) fn compile_resident_interpreter_cached(format: u32) -> Result<Engine, Error> {
    #[derive(Default)]
    struct ResidentInterpreter(Mutex<HashMap<u32, Engine>>);
    let held_ = crate::code_rule_dsl::logic::profile::machine::<ResidentInterpreter>();
    let mut cached = held_.0.lock().map_err(|_| Error::BackendFailure)?;
    if let Some(engine) = cached.get(&format) {
        return Ok(engine.clone());
    }
    let source = format!("{}{}", resident_prefix(format), kernel_source(INTERPRETER_FETCH, None, "", ""));
    lane::device()?;
    let engine = Engine::Interpreter { kernel: lane::kernel_fixed(&source, "run_batch", Some(format)), buffers: Arc::new(Mutex::new(None)) };
    cached.insert(format, engine.clone());
    Ok(engine)
}

/// The program backend's fixed kernels, to compile ahead of their first use (`MatcherDevice::kernels_ahead`): the set
/// steps', the lattice's and the nearest jumps' kernels, and the interpreter over tapes, the label passes' kernels,
/// the interpreter over resident tapes and the sweep of formats 15 and 16 (the formats a run places its tapes in:
/// `label_passes::warm`).
/// Each is asked for as its use asks for it, so its use finds it.
pub(in crate::code_rule_dsl) fn kernels_ahead() -> Vec<super::device::KernelAhead> {
    fn made(name: &str, make: impl Fn() -> Result<(), Error> + Send + Sync + 'static) -> super::device::KernelAhead {
        let make = move || {
            let mut made = Ok(());
            rerail_accel::gpu::released(|| rerail_accel::gpu::caught(|| made = make().map_err(|e| format!("{e:?}"))))?;
            made
        };
        (name.to_owned(), Box::new(make))
    }
    let mut v = vec![
        made("set steps", || {
            drop(set_steps::kernel());
            Ok(())
        }),
        made("lattice", || {
            drop(lattice::kernel());
            Ok(())
        }),
        made("nearest jumps", || {
            drop(nearest::kernel());
            Ok(())
        }),
    ];
    for format in [15u32, 16] {
        v.push(made(&format!("interpreter {format}"), move || interpreter_kernel(format).map(drop)));
        v.push(made(&format!("resident interpreter {format}"), move || compile_resident_interpreter_cached(format).map(drop)));
        v.push(made(&format!("label passes {format}"), move || label_passes::compile_labels(format)));
        // Not where NVRTC compiles (`rerail_metal::nvrtc_unbounded`).
        if !rerail_metal::nvrtc_unbounded() {
            v.push(made(&format!("label sweep {format}"), move || compile_sweep_cached(format).map(drop)));
        }
    }
    v
}

/// The interpreter over tapes, of the run's format (`label_passes::run_format`): a batch's launch takes the kernel of
/// its own format (`interpreter_kernel`), this one where the batch is the run's.
pub fn compile_interpreter() -> Result<Engine, Error> {
    Ok(Engine::Interpreter { kernel: interpreter_kernel(label_passes::run_format())?, buffers: Arc::new(Mutex::new(None)) })
}

/// The interpreter over a batch of tapes in `format` (0: mixed), compiled once per format: its tape reads are the
/// format's alone (decision 0576), as the label kernels' are ([`tape_view`]).
fn interpreter_kernel(format: u32) -> Result<lane::Kernel, Error> {
    #[derive(Default)]
    struct Interpreters(Mutex<HashMap<u32, lane::Kernel>>);
    let held_ = crate::code_rule_dsl::logic::profile::machine::<Interpreters>();
    let mut cached = held_.0.lock().map_err(|_| Error::BackendFailure)?;
    if let Some(kernel) = cached.get(&format) {
        return Ok(kernel.clone());
    }
    lane::device()?;
    let kernel = lane::kernel_fixed(&format!("{}{}", tape_view(format), kernel_source(INTERPRETER_FETCH, None, "", "")), "run_batch", Some(format));
    cached.insert(format, kernel.clone());
    Ok(kernel)
}

/// The interpreter with a recursive group's sweep kernel (`label_passes`, `label_sweep`), compiled
/// once per process. A program whose flags carry a route (bit 31; bit 30 captured nodes, bits 8..30
/// the label) marks its label as each result is published; other programs run as in `run_batch`.
pub(in crate::code_rule_dsl) fn compile_sweep_cached(format: u32) -> Result<lane::Kernel, Error> {
    #[derive(Default)]
    struct Sweep(Mutex<HashMap<u32, lane::Kernel>>);
    let held_ = crate::code_rule_dsl::logic::profile::machine::<Sweep>();
    let mut cached = held_.0.lock().map_err(|_| Error::BackendFailure)?;
    if let Some(pipeline) = cached.get(&format) {
        return Ok(pipeline.clone());
    }
    // The label passes' sweep, over resident tapes as their interpreter, of the batch's format.
    let source = format!("{}{}", resident_prefix(format), kernel_source(INTERPRETER_FETCH, None, "", ""));
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
    lane::device()?;
    let kernel = lane::kernel_fixed(&format!("{source}\n{}", include_str!("label_sweep.uglykl")), "label_sweep", Some(format));
    cached.insert(format, kernel.clone());
    Ok(kernel)
}

pub(super) fn compile_jit(programs: &[Program], format: u32) -> Result<Engine, Error> {
    lane::device()?;
    Ok(Engine::Jit {
        kernel: lane::kernel(&jit_kernel_source(programs, format)?, "run_batch", Some(format)),
        programs: programs.len(),
        format,
        buffers: Arc::new(Mutex::new(None)),
    })
}

pub(super) fn jit_cache_key(programs: &[Program], format: u32) -> Result<String, Error> {
    Ok(format!(
        "metal-jit:{FORMAT_VERSION}:{}:{}",
        std::env::consts::ARCH,
        jit_kernel_source(programs, format)?
    ))
}

pub(super) fn program_buffer_key(programs: &[Program]) -> Result<ProgramBufferKey, Error> {
    Ok(ProgramBufferKey::from(&PackedBatch::new_in_place(programs, &[], &[])?))
}

/// The interpreter, compiled once per process: a lowered matcher dispatches it once per pass.
pub(in crate::code_rule_dsl) fn compile_interpreter_cached() -> Result<Engine, Error> {
    #[derive(Default)]
    struct Interpreter(Mutex<Option<Engine>>);
    let held_ = crate::code_rule_dsl::logic::profile::machine::<Interpreter>();
    let mut cached = held_.0.lock().map_err(|_| Error::BackendFailure)?;
    if let Some(engine) = &*cached {
        return Ok(engine.clone());
    }
    let engine = compile_interpreter()?;
    *cached = Some(engine.clone());
    Ok(engine)
}

/// Whether `programs`' kernel is compiled (`compile_jit_cached`), without compiling it.
pub(in crate::code_rule_dsl) fn jit_ready(programs: &[Program]) -> Result<bool, Error> {
    let key = jit_cache_key(programs, label_passes::run_format())?;
    Ok(crate::code_rule_dsl::logic::profile::shared::<MetalPrograms>().0.lock().map_err(|_| Error::BackendFailure)?.contains_key(&key))
}

/// `programs`' kernel over the run's tape format (`label_passes::run_format`), compiled once: a batch of another
/// format takes its own at its launch (`compile_jit_in`).
pub(in crate::code_rule_dsl) fn compile_jit_cached(programs: &[Program]) -> Result<Engine, Error> {
    compile_jit_in(programs, label_passes::run_format())
}

/// `programs`' kernel over tapes in `format` (0: mixed), compiled once.
fn compile_jit_in(programs: &[Program], format: u32) -> Result<Engine, Error> {
    let key = jit_cache_key(programs, format)?;
    if let Some(engine) = crate::code_rule_dsl::logic::profile::shared::<MetalPrograms>().0
        .lock()
        .map_err(|_| Error::BackendFailure)?
        .get(&key)
        .cloned()
    {
        return Ok(engine);
    }
    let engine = compile_jit(programs, format)?;
    let held_ = crate::code_rule_dsl::logic::profile::shared::<MetalPrograms>();
    let mut cache = held_.0.lock().map_err(|_| Error::BackendFailure)?;
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
    anchors: &[Vec<Vector<u32>>],
) -> Result<Vec<Vec<Vec<Output>>>, Error> {
    execute_outcomes(engine, programs, files, anchors)?.into_result()
}

pub(in crate::code_rule_dsl) fn execute_outcomes(
    engine: &Engine,
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vector<u32>>],
) -> Result<BatchOutcome, Error> {
    execute_packed(engine, programs, files, anchors)?.into_outcome()
}

pub(super) fn execute_packed(
    engine: &Engine,
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vector<u32>>],
) -> Result<PackedExecution, Error> {
    execute_packed_with_result_limits(engine, programs, files, anchors, None)
}

pub fn execute_packed_with_result_limits(
    engine: &Engine,
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vector<u32>>],
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
    anchors: &[Vec<Vector<u32>>],
    result_limits: Option<&[u32]>,
    budget: super::Budget,
) -> Result<PackedExecution, Error> {
    super::restarting(programs, anchors, budget, &mut |anchors, budget, growth| execute_packed_once(engine, programs, files, anchors, result_limits, budget, growth))
}

fn execute_packed_once(
    engine: &Engine,
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vector<u32>>],
    result_limits: Option<&[u32]>,
    budget: super::Budget,
    growth: &super::StateGrowth,
) -> Result<PackedExecution, Error> {
    let _evaluation = super::metal_backend::evaluation_scope();
    let mut batch = pack_batch(engine, programs, files, anchors)?;
    batch.apply_budget(budget, growth);
    if batch.tasks.is_empty() {
        return Ok(PackedExecution::new(batch, PackedResultPage::empty()));
    }
    let allocation_started = std::time::Instant::now();
    lane::device()?;
    let (totals, workspace) = lease_workspace(&mut batch, programs, result_limits)?;
    execute_on_device(engine, programs, files, batch, totals, &workspace, allocation_started)
}

fn pack_batch(
    engine: &Engine,
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vector<u32>>],
) -> Result<PackedBatch, Error> {
    if matches!(engine, Engine::Jit { programs: count, .. } if *count != programs.len()) {
        return Err(Error::InvalidAnchor);
    }
    // The tasks with no node records: the device reads the documents in the slabs (`DocTable`).
    PackedBatch::new_in_place(programs, files, anchors).map_err(|error| print_batch_error("pack", error))
}

/// `error`, printed with the stage that failed under `RERAIL_PACKED_BATCH_PROFILE`.
fn print_batch_error(stage: &str, error: Error) -> Error {
    if std::env::var_os("RERAIL_PACKED_BATCH_PROFILE").is_some() {
        eprintln!("RERAIL_PACKED_BATCH_ERROR stage={stage} error={error:?}");
    }
    error
}

/// The batch's workspace totals at `lanes` lanes.
fn assign_batch_workspace(
    batch: &mut PackedBatch,
    programs: &[Program],
    result_limits: Option<&[u32]>,
    lanes: usize,
) -> Result<WorkspaceTotals, Error> {
    let totals = batch
        .assign_workspace_with_result_limits_and_lanes(programs, result_limits, lanes)
        .map_err(|error| print_batch_error("workspace", error))?;
    if std::env::var_os("RERAIL_PACKED_BATCH_PROFILE").is_some() {
        eprintln!(
            "RERAIL_PACKED_BATCH_PROFILE backend=metal programs={} tasks={} lanes={lanes} workspaceBytes={} totals={totals:?}",
            programs.len(),
            batch.tasks.len(),
            batch.workspace_allocation_bytes(totals).unwrap_or(u64::MAX),
        );
    }
    Ok(totals)
}

/// A workspace for `batch` and its totals, with the batch's evaluations and lanes in its generation. Every lane
/// reserves the widest evaluation's states: a workspace the pool refuses runs with half the lanes, down to one
/// (decision 0334 §9). Fewer lanes run its tasks in fewer at a time; the results are the same.
fn lease_workspace(
    batch: &mut PackedBatch,
    programs: &[Program],
    result_limits: Option<&[u32]>,
) -> Result<(WorkspaceTotals, MetalWorkspaceLease), Error> {
    let mut lanes = EXECUTION_LANES;
    let (totals, workspace) = loop {
        let totals = assign_batch_workspace(batch, programs, result_limits, lanes)?;
        match MetalWorkspaceLease::acquire(
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
    Ok((totals, workspace))
}

/// Runs the packed batch on the device: what it binds is held here until its results are read.
fn execute_on_device(
    engine: &Engine,
    programs: &[Program],
    files: &[TapeDoc],
    batch: PackedBatch,
    totals: WorkspaceTotals,
    workspace: &MetalWorkspaceLease,
    allocation_started: std::time::Instant,
) -> Result<PackedExecution, Error> {
    let (program_buffers, task_buffer, header_buffer) = upload_batch(engine, &batch)?;
    // The documents resident, gathered into one block (decision 0289 §4), in the batch's numbering: their views
    // where the files were, and the block where the node records were. The kernel reads nothing from the
    // children, source and line start slots; the block stands in for them.
    let table = resident_doc_table(files)?;
    let (file_buffer, block, supertype_buffer) = upload_documents(&table, &batch)?;
    copy_scheduler_state(&workspace.scheduler, &batch);
    let allocation_elapsed = allocation_started.elapsed();
    print_workspace_profile(workspace);
    // The rows by name (`run_batch`'s): the block stands for the node records, children, source and line starts.
    let buffers = [
        ("tasks", task_buffer.buffer()),
        ("header", header_buffer.buffer()),
        ("files", file_buffer.buffer()),
        ("nodes", block),
        ("programs", program_buffers.programs.buffer()),
        ("words", program_buffers.words.buffer()),
        ("longs", program_buffers.longs.buffer()),
        ("children", block),
        ("supertypes", supertype_buffer.buffer()),
        ("pending", workspace.pending.buffer()),
        ("captures", workspace.captures.buffer()),
        ("cursors", workspace.cursors.buffer()),
        ("repeats", workspace.repeats.buffer()),
        ("schedulerStorage", workspace.scheduler.buffer()),
        ("results", workspace.results.buffer()),
        ("statuses", workspace.statuses.buffer()),
        ("values", program_buffers.values.buffer()),
        ("strings", program_buffers.strings.buffer()),
        ("nfaStates", program_buffers.nfa_states.buffer()),
        ("nfaData", program_buffers.nfa_data.buffer()),
        ("nfaRanges", program_buffers.nfa_ranges.buffer()),
        ("luaInstructions", program_buffers.lua_instructions.buffer()),
        ("luaMembers", program_buffers.lua_members.buffer()),
        ("assertions", program_buffers.assertions.buffer()),
        ("assertionData", program_buffers.assertion_data.buffer()),
        ("stringBytes", program_buffers.string_bytes.buffer()),
        ("source", block),
        ("valueWork", workspace.value_states.buffer()),
        ("lineStarts", block),
        ("finalCaptures", workspace.final_captures.buffer()),
        ("outputStorage", workspace.output_storage.buffer()),
    ];
    dispatch_batch(engine, programs, buffers, &table, batch.evaluation_lanes.len() as u64, allocation_elapsed)?;
    collect_results(batch, totals, workspace)
}

/// The batch's programs, tasks and header on the device.
fn upload_batch(engine: &Engine, batch: &PackedBatch) -> Result<(Arc<ProgramBuffers>, HostMapped, HostMapped), Error> {
    let program_buffers = engine.program_buffers(batch)?;
    let task_buffer = HostMapped::from_slice(&batch.tasks)?;
    let header_buffer = HostMapped::from_slice(&[batch.header])?;
    Ok((program_buffers, task_buffer, header_buffer))
}

/// The documents resident, in one table: their views, the block, the batch's other slabs and the 16z cache.
fn resident_doc_table(files: &[TapeDoc]) -> Result<crate::code_rule_dsl::residency::slabs::DocTable, Error> {
    let table = crate::code_rule_dsl::residency::slabs::admit_docs(files)?;
    let table = crate::code_rule_dsl::residency::slabs::DocTable::new(table).ok_or(Error::BackendCapacity)?;
    if table.nodes != files.iter().map(TapeDoc::transducer_len).sum::<usize>() {
        return Err(Error::BackendFailure);
    }
    Ok(table)
}

/// The table's views and block, and the batch's supertypes, on the device.
fn upload_documents<'table>(
    table: &'table crate::code_rule_dsl::residency::slabs::DocTable,
    batch: &PackedBatch,
) -> Result<(HostMapped, &'table lane::Buffer, HostMapped), Error> {
    let file_buffer = HostMapped::from_slice(&table.views)?;
    let block = table.block.as_ref().ok_or(Error::BackendFailure)?.device_buffer();
    // The derived classes alone (`has_supertype`): the classes are the codes', read on the tape.
    let supertypes = batch.metal_supertypes();
    let supertype_buffer = HostMapped::from_slice(&supertypes)?;
    Ok((file_buffer, block, supertype_buffer))
}

/// Writes the batch's global arenas, then its evaluations, then its evaluation lanes, to the scheduler buffer.
fn copy_scheduler_state(scheduler_buffer: &HostMapped, batch: &PackedBatch) {
    unsafe {
        std::ptr::copy_nonoverlapping(
            std::ptr::from_ref(&batch.global_arenas).cast::<u8>(),
            scheduler_buffer.host(),
            std::mem::size_of_val(&batch.global_arenas),
        );
        let evaluation_start = std::mem::size_of_val(&batch.global_arenas);
        std::ptr::copy_nonoverlapping(
            batch.evaluations.as_ptr().cast::<u8>(),
            scheduler_buffer.host().add(evaluation_start),
            std::mem::size_of_val(batch.evaluations.as_slice()),
        );
        std::ptr::copy_nonoverlapping(
            batch.evaluation_lanes.as_ptr().cast::<u8>(),
            scheduler_buffer.host().add(evaluation_start + std::mem::size_of_val(batch.evaluations.as_slice())),
            std::mem::size_of_val(batch.evaluation_lanes.as_slice()),
        );
    }
}

fn print_workspace_profile(workspace: &MetalWorkspaceLease) {
    if std::env::var_os("RERAIL_PACKED_BATCH_PROFILE").is_some() {
        eprintln!(
            "RERAIL_METAL_WORKSPACE reused={} generation={}",
            workspace.reused, workspace.generation,
        );
    }
}

/// Launches the kernel over `buffers` for `lanes` evaluation lanes, the table's other slabs and cache used, and waits
/// for it. A JIT engine compiled for another format than the batch's compiles `programs`' kernel for the batch's.
fn dispatch_batch(
    engine: &Engine,
    programs: &[Program],
    buffers: [(&str, &lane::Buffer); 31],
    table: &crate::code_rule_dsl::residency::slabs::DocTable,
    lanes: u64,
    allocation_elapsed: std::time::Duration,
) -> Result<(), Error> {
    let mut command = lane::Command::new("Rerail packed transducer batch");
    // The interpreter, or the programs' kernel, of the batch's format (0: mixed).
    let format = table.format().unwrap_or(0);
    let interpreter;
    let jit;
    let kernel = match engine {
        Engine::Interpreter { .. } => {
            interpreter = interpreter_kernel(format)?;
            &interpreter
        }
        Engine::Jit { format: compiled, .. } if *compiled == format => engine.kernel(),
        Engine::Jit { .. } => {
            jit = compile_jit_in(programs, format)?;
            jit.kernel()
        }
    };
    let mut launch = command.launch(kernel, "Rerail packed transducer kernel");
    for (name, buffer) in buffers {
        launch = launch.buffer(name, buffer);
    }
    // The batch's other slabs, which its reads reach from the block by the views' offsets (decision 0353 step 4,
    // 0471), and the 16z documents' cache.
    for slab in &table.slabs {
        launch = launch.uses_read(slab.device_buffer());
    }
    if let Some(cache) = &table.cache {
        launch = launch.uses(cache);
    }
    // A thread a lane, and no more: `run_batch` reads its lane by its thread.
    launch.threads_exact(lanes, 32);
    let execution_started = std::time::Instant::now();
    command.run()?;
    let execution_elapsed = execution_started.elapsed();
    if std::env::var_os("RERAIL_PACKED_BATCH_PROFILE").is_some() {
        eprintln!(
            "RERAIL_PACKED_BATCH_PHASES allocationNanoseconds={} executionNanoseconds={}",
            allocation_elapsed.as_nanos(),
            execution_elapsed.as_nanos(),
        );
    }
    Ok(())
}

/// The batch's results, read from the workspace the kernel wrote them to.
fn collect_results(mut batch: PackedBatch, totals: WorkspaceTotals, workspace: &MetalWorkspace) -> Result<PackedExecution, Error> {
    #[cfg(any(test, feature = "test-hooks"))]
    INVOCATIONS.set(INVOCATIONS.get() + 1);
    batch.global_arenas = unsafe {
        workspace.scheduler.host().cast::<MetalGlobalArenas>().read()
    };
    let statuses = unsafe {
        std::slice::from_raw_parts(
            workspace.statuses.host().cast::<TaskStatus>(),
            batch.tasks.len(),
        )
    };
    if std::env::var_os("RERAIL_PACKED_BATCH_PROFILE").is_some() {
        let mut work = BTreeMap::<u32, (u64, u32)>::new();
        for (task, status) in batch.tasks.iter().zip(statuses) {
            let entry = work.entry(task.program).or_default();
            entry.0 += u64::from(status.reserved);
            entry.1 += 1;
        }
        let mut work = work.into_iter().collect::<Vec<_>>();
        work.sort_by_key(|(_, (total, _))| std::cmp::Reverse(*total));
        eprintln!(
            "RERAIL_PACKED_TASK_WORK {:?}",
            work.into_iter().take(12).collect::<Vec<_>>()
        );
    }
    let results = unsafe {
        std::slice::from_raw_parts(
            workspace.results.host().cast::<ResultRecord>(),
            totals
                .candidates
                .checked_add(totals.results)
                .ok_or(Error::BackendCapacity)? as usize,
        )
    };
    let final_captures = unsafe {
        std::slice::from_raw_parts(
            workspace.final_captures.host().cast::<FinalCaptureRecord>(),
            totals.output_captures as usize,
        )
    };
    let output_storage = unsafe {
        std::slice::from_raw_parts(
            workspace.output_storage.host(),
            batch.header.workspace_byte_count as usize,
        )
    };
    let properties = unsafe {
        std::slice::from_raw_parts(
            workspace.output_storage.host().cast::<PropertyRecord>(),
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
