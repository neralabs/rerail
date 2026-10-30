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
#[cfg(test)]
use std::cell::Cell;
use std::collections::BTreeMap;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, LazyLock, Mutex};

#[cfg(test)]
thread_local! {
    static INVOCATIONS: Cell<usize> = const { Cell::new(0) };
    static HOST_EVENTS: Cell<usize> = const { Cell::new(0) };
}

#[cfg(test)]
pub(super) fn reset_metrics() {
    INVOCATIONS.set(0);
    HOST_EVENTS.set(0);
}

#[cfg(test)]
pub(super) fn metrics() -> (usize, usize) {
    (INVOCATIONS.get(), HOST_EVENTS.get())
}

const METAL_JIT_CACHE_LIMIT: usize = 32;
pub(super) const EXECUTION_LANES: usize = 64;
static JIT_CACHE: LazyLock<Mutex<BTreeMap<String, Engine>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
const METAL_WORKSPACE_POOL_LIMIT: usize = 4;
static METAL_WORKSPACE_POOL: LazyLock<Mutex<Vec<MetalWorkspace>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));
static METAL_DEVICE: LazyLock<Option<Device>> = LazyLock::new(Device::system_default);

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
    fn new(
        device: &Device,
        totals: WorkspaceTotals,
        scheduler_bytes: u32,
        output_storage_bytes: u32,
        task_capacity: usize,
    ) -> Result<Self, Error> {
        let workspace = Self {
            totals,
            scheduler_bytes,
            output_storage_bytes,
            task_capacity,
            generation: 1,
            pending: MappedBuffer::zeroed::<PendingRecord>(device, totals.pending as usize)?,
            captures: MappedBuffer::zeroed::<CaptureRecord>(device, totals.captures as usize)?,
            cursors: MappedBuffer::zeroed::<CaptureRecord>(device, totals.cursors as usize)?,
            repeats: MappedBuffer::zeroed::<RepeatRecord>(device, totals.repeats as usize)?,
            scheduler: MappedBuffer::zeroed::<u8>(device, scheduler_bytes as usize)?,
            results: MappedBuffer::zeroed::<ResultRecord>(
                device,
                totals
                    .candidates
                    .checked_add(totals.results)
                    .ok_or(Error::BackendCapacity)? as usize,
            )?,
            statuses: MappedBuffer::zeroed::<TaskStatus>(device, task_capacity)?,
            value_states: MappedBuffer::zeroed::<u32>(device, totals.value_states as usize)?,
            final_captures: MappedBuffer::zeroed::<FinalCaptureRecord>(
                device,
                totals.output_captures as usize,
            )?,
            output_storage: MappedBuffer::zeroed::<u8>(device, output_storage_bytes as usize)?,
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
            pool.clear();
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

const BATCH_ENTRY: &str = include_str!("metal_batch_runtime.metal");

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

pub(super) struct CachedProgramBuffers {
    key: ProgramBufferKey,
    buffers: Arc<ProgramBuffers>,
}

pub(super) type ProgramBufferCache = Arc<Mutex<Option<CachedProgramBuffers>>>;

#[derive(Clone)]
pub(super) enum Engine {
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
        source.replace_range(output_begin..output_end, "jit_apply_output(outputContext)");
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

const JIT_CONTEXT: &str = "JitContext context={nodes,children,supertypes,words,longs,values,strings,nfaStates,nfaData,nfaRanges,luaInstructions,luaMembers,valueWork,stringBytes,source,assertions,assertionData,captures,cursors,repeats,queuedStates,greedyEntries,pending,results,lineStarts,task,file,program,node,&evaluation,&globalArenas,valueBase,valueCapacity,&status,&path,&pendingCount,&captureCount,&cursorCount,&repeatCount,&resultCount,&candidateHead,&localOrdinal,&keep,&advance};";
const JIT_OUTPUT_CONTEXT: &str = "JitOutputContext outputContext={assertionData,program,task,file,nodes,captures,raw.captureStart,raw.captureCount,finalCaptures,&finalCaptureCount,&captureBefore,outputStorage,&propertyCount,&outputByteCount,source,lineStarts,values,nfaStates,nfaData,nfaRanges,valueWork,resultIndex,&resultProperties,&dynamicProperties,&outputSteps,&evaluation,&sharedEvaluation,&globalArenas,valueBase,valueCapacity};";

fn jit_support(
    bodies: &std::collections::BTreeMap<u8, &'static str>,
    programs: &[Program],
) -> Result<String, Error> {
    let mut source = String::from(
        "struct JitContext{device const Node* nodes;device const uint* children;device const Supertype* supertypes;device const ulong* longs;device const ValueRecord* values;device const StringRecord* strings;device const NfaState* nfaStates;device const uint* nfaData;device const NfaRange* nfaRanges;device const LuaInstruction* luaInstructions;device const LuaMember* luaMembers;device uint* valueWork;device const uchar* stringBytes;device const uchar* source;device const Assertion* assertions;device const uint* assertionData;device Link* captures;device Link* cursors;device RepeatLink* repeats;device Path* pending;device Result* results;device const uint* lineStarts;Task task;File file;Program program;Node node;device Evaluation* evaluation;device GlobalArenas* globalArenas;uint valueBase;uint valueCapacity;device Status* status;thread Path* path;thread uint* pendingCount;thread uint* captureCount;thread uint* cursorCount;thread uint* repeatCount;thread uint* resultCount;thread uint* candidateHead;thread uint* localOrdinal;thread bool* keep;thread bool* advance;};template<uint OP> __attribute__((noinline)) bool jit_step(thread JitContext& context,uint mode,uint operand){device const Node* nodes=context.nodes;device const uint* children=context.children;device const Supertype* supertypes=context.supertypes;device const ulong* longs=context.longs;device const ValueRecord* values=context.values;device const StringRecord* strings=context.strings;device const NfaState* nfaStates=context.nfaStates;device const uint* nfaData=context.nfaData;device const NfaRange* nfaRanges=context.nfaRanges;device const LuaInstruction* luaInstructions=context.luaInstructions;device const LuaMember* luaMembers=context.luaMembers;device uint* valueWork=context.valueWork;device const uchar* stringBytes=context.stringBytes;device const uchar* source=context.source;device const Assertion* assertions=context.assertions;device const uint* assertionData=context.assertionData;device Link* captures=context.captures;device Link* cursors=context.cursors;device RepeatLink* repeats=context.repeats;device Path* pending=context.pending;device Result* results=context.results;device const uint* lineStarts=context.lineStarts;Task task=context.task;File file=context.file;Program program=context.program;Node node=context.node;device Evaluation& evaluation=*context.evaluation;device GlobalArenas& globalArenas=*context.globalArenas;uint valueBase=context.valueBase,valueCapacity=context.valueCapacity;device Status& status=*context.status;thread Path& path=*context.path;thread uint& pendingCount=*context.pendingCount;thread uint& captureCount=*context.captureCount;thread uint& cursorCount=*context.cursorCount;thread uint& repeatCount=*context.repeatCount;thread uint& resultCount=*context.resultCount;thread uint& candidateHead=*context.candidateHead;thread uint& localOrdinal=*context.localOrdinal;bool keep=*context.keep,advance=*context.advance;constexpr uint op=OP;",
    );
    for (index, (opcode, body)) in bodies.iter().enumerate() {
        source.push_str(if index == 0 {
            "if constexpr("
        } else {
            "else if constexpr("
        });
        source.push_str(&format!(
            "OP=={opcode}u){{{}}}",
            body.strip_suffix("break;")
                .unwrap_or(body)
                .replace("return;", "return false;")
        ));
    }
    source.push_str("else{return false;}*context.keep=keep;*context.advance=advance;return true;}");
    for (program_index, program) in programs.iter().enumerate() {
        for value in 0..program.value_programs.len() {
            source.push_str(&format!(
                "__attribute__((noinline)) bool jit_value_{program_index}_{value}(thread JitContext& context){{uint outcome=evaluate_text(context.values,context.strings,context.nfaStates,context.nfaData,context.nfaRanges,context.luaInstructions,context.luaMembers,context.valueWork,context.stringBytes,context.source,context.program,context.node,context.task,{value}u,context.path->steps,*context.evaluation,*context.globalArenas,context.valueBase,context.valueCapacity);if(outcome==2u){{context.status->error=6u;return false;}}if(outcome==3u){{context.status->error=4u;return false;}}if(outcome==4u){{context.status->error=2u;return false;}}*context.keep=outcome==1u;return true;}}"
            ));
        }
        for capture in 0..program.capture_programs.len() {
            source.push_str(&format!(
                "__attribute__((noinline)) bool jit_capture_{program_index}_{capture}(thread JitContext& context){{uint outcome=evaluate_assertion(context.assertions[context.program.captureStart+{capture}u],context.assertionData,context.captures,context.path->captures,context.task,context.file,context.program,context.nodes,context.source,context.strings,context.stringBytes,context.values,context.nfaStates,context.nfaData,context.nfaRanges,context.luaInstructions,context.luaMembers,context.valueWork,context.lineStarts,context.path->steps,*context.evaluation,*context.globalArenas,context.valueBase,context.valueCapacity);if(outcome==2u){{context.status->error=6u;return false;}}if(outcome==3u){{context.status->error=4u;return false;}}if(outcome==4u){{context.status->error=2u;return false;}}*context.keep=outcome==1u;return true;}}"
            ));
        }
    }
    source.push_str("struct JitOutputContext{device const uint* assertionData;Program program;Task task;File file;device const Node* nodes;device const Link* captures;uint head;uint captureCount;device FinalCapture* finalCaptures;thread uint* finalCount;thread uint* captureStart;device uchar* storage;thread uint* propertyCount;thread uint* byteCount;device const uchar* source;device const uint* lineStarts;device const ValueRecord* values;device const NfaState* nfaStates;device const uint* nfaData;device const NfaRange* nfaRanges;device uint* valueWork;uint resultIndex;thread uint* resultProperties;thread uint* dynamicProperties;thread uint* steps;device Evaluation* evaluation;device Evaluation* sharedEvaluation;device GlobalArenas* globalArenas;uint valueBase;uint valueCapacity;};");
    for program_index in 0..programs.len() {
        source.push_str(&format!(
            "__attribute__((noinline)) uint jit_output_{program_index}(thread JitOutputContext& c){{return apply_output(c.assertionData,c.program,c.task,c.file,c.nodes,c.captures,c.head,c.captureCount,c.finalCaptures,*c.finalCount,*c.captureStart,c.storage,*c.propertyCount,*c.byteCount,c.source,c.lineStarts,c.values,c.nfaStates,c.nfaData,c.nfaRanges,c.valueWork,c.resultIndex,*c.resultProperties,*c.dynamicProperties,*c.steps,*c.evaluation,*c.sharedEvaluation,*c.globalArenas,c.valueBase,c.valueCapacity);}}"
        ));
    }
    source.push_str(
        "uint jit_apply_output(thread JitOutputContext& context){switch(context.task.program){",
    );
    for program_index in 0..programs.len() {
        source.push_str(&format!(
            "case {program_index}u:return jit_output_{program_index}(context);"
        ));
    }
    source.push_str("default:return 2u;}}");
    for (program_index, program) in programs.iter().enumerate() {
        source.push_str(&format!(
            "__attribute__((noinline)) bool jit_program_{program_index}(thread JitContext& context){{switch(context.path->pc){{"
        ));
        let native = native_ir::lower(program).map_err(|_| Error::BackendFailure)?;
        for block in &native.structural {
            let pc = block.position;
            let (opcode, mode, operand) = block.operation.encoded();
            if !bodies.contains_key(&opcode) {
                return Err(Error::InvalidOpcode(usize::from(pc)));
            }
            if opcode == TEXT {
                source.push_str(&format!(
                    "case {pc}u:return jit_value_{program_index}_{operand}(context);"
                ));
            } else if opcode == ASSERT {
                source.push_str(&format!("case {pc}u:return true;"));
            } else {
                source.push_str(&format!(
                    "case {pc}u:return jit_step<{opcode}u>(context,{mode}u,{operand}u);"
                ));
            }
        }
        source.push_str("default:context.status->error=5u;return false;}}");
    }
    Ok(source
        .replace("device Path* pending", "device Pending* pending")
        .replace(
            "device const Supertype* supertypes;device const ulong* longs",
            "device const Supertype* supertypes;device const uint* words;device const ulong* longs",
        )
        .replace(
            "device RepeatLink* repeats;device Pending* pending",
            "device RepeatLink* repeats;device QueuedState* queuedStates;device GreedyEntry* greedyEntries;device Pending* pending",
        )
        .replace(
            "device const Supertype* supertypes=context.supertypes;device const ulong* longs",
            "device const Supertype* supertypes=context.supertypes;device const uint* words=context.words;device const ulong* longs",
        )
        .replace(
            "device RepeatLink* repeats=context.repeats;device Pending* pending",
            "device RepeatLink* repeats=context.repeats;device QueuedState* queuedStates=context.queuedStates;device GreedyEntry* greedyEntries=context.greedyEntries;device Pending* pending",
        ))
}

fn jit_kernel_source(programs: &[Program]) -> Result<String, Error> {
    let bodies = opcode_bodies()?;
    let mut dispatch = String::from("switch(task.program){");
    for program_index in 0..programs.len() {
        dispatch.push_str(&format!(
            "case {program_index}u:if(!jit_program_{program_index}(context))return;break;"
        ));
    }
    dispatch.push_str("default:status.error=5u;return;}");
    Ok(kernel_source(
        "",
        Some(&dispatch),
        &jit_support(&bodies, programs)?,
        JIT_CONTEXT,
        JIT_OUTPUT_CONTEXT,
    ))
}

pub(super) fn compile_interpreter() -> Result<Engine, Error> {
    Ok(Engine::Interpreter {
        pipeline: pipeline_named(
            &kernel_source(
                "uint word=words[program.wordStart+path.pc];op=word&255u;mode=(word>>8u)&255u;operand=word>>16u;",
                None,
                "",
                "",
                "",
            ),
            "run_batch",
        )?,
        buffers: Arc::new(Mutex::new(None)),
    })
}

pub(super) fn compile_jit(programs: &[Program]) -> Result<Engine, Error> {
    Ok(Engine::Jit {
        pipeline: pipeline_named(&jit_kernel_source(programs)?, "run_batch")?,
        programs: programs.len(),
        buffers: Arc::new(Mutex::new(None)),
    })
}

fn jit_cache_key(programs: &[Program]) -> Result<String, Error> {
    Ok(format!(
        "metal-jit:{FORMAT_VERSION}:{}:{}",
        std::env::consts::ARCH,
        jit_kernel_source(programs)?
    ))
}

pub(super) fn program_buffer_key(programs: &[Program]) -> Result<ProgramBufferKey, Error> {
    Ok(ProgramBufferKey::from(&PackedBatch::new(
        programs,
        &[],
        &[],
    )?))
}

/// The interpreter, compiled once per process: a lowered matcher dispatches it once per pass.
pub(super) fn compile_interpreter_cached() -> Result<Engine, Error> {
    static INTERPRETER: Mutex<Option<Engine>> = Mutex::new(None);
    let mut cached = INTERPRETER.lock().map_err(|_| Error::BackendFailure)?;
    if let Some(engine) = &*cached {
        return Ok(engine.clone());
    }
    let engine = compile_interpreter()?;
    *cached = Some(engine.clone());
    Ok(engine)
}

pub(super) fn compile_jit_cached(programs: &[Program]) -> Result<Engine, Error> {
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

pub(super) fn execute_outcomes(
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

pub(super) fn execute_packed_with_result_limits(
    engine: &Engine,
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
    result_limits: Option<&[u32]>,
) -> Result<PackedExecution, Error> {
    let _evaluation = super::metal_backend::metal_execution_guard()?;
    if matches!(engine, Engine::Jit { programs: count, .. } if *count != programs.len()) {
        return Err(Error::InvalidAnchor);
    }
    let mut batch = PackedBatch::new(programs, files, anchors).map_err(|error| {
        if std::env::var_os("TEMPO_PACKED_BATCH_PROFILE").is_some() {
            eprintln!("TEMPO_PACKED_BATCH_ERROR stage=pack error={error:?}");
        }
        error
    })?;
    if batch.tasks.is_empty() {
        return Ok(PackedExecution::new(batch, PackedResultPage::empty()));
    }
    let totals = batch
        .assign_workspace_with_result_limits_and_lanes(programs, result_limits, EXECUTION_LANES)
        .map_err(|error| {
            if std::env::var_os("TEMPO_PACKED_BATCH_PROFILE").is_some() {
                eprintln!("TEMPO_PACKED_BATCH_ERROR stage=workspace error={error:?}");
            }
            error
        })?;
    if std::env::var_os("TEMPO_PACKED_BATCH_PROFILE").is_some() {
        eprintln!(
            "TEMPO_PACKED_BATCH_PROFILE backend=metal programs={} tasks={} workspaceBytes={} totals={totals:?}",
            programs.len(),
            batch.tasks.len(),
            batch.workspace_allocation_bytes(totals).unwrap_or(u64::MAX),
        );
    }
    let allocation_started = std::time::Instant::now();
    let device = METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
    let queue = device.new_command_queue();
    queue.set_label("Tempo packed transducer queue");
    let workspace = MetalWorkspaceLease::acquire(
        device,
        totals,
        batch.header.scheduler_byte_count,
        batch.header.workspace_byte_count,
        batch.tasks.len(),
    )?;
    for evaluation in &mut batch.evaluations {
        evaluation.generation = workspace.generation;
    }
    for lane in &mut batch.evaluation_lanes {
        lane.generation = workspace.generation;
    }
    let program_buffers = engine.program_buffers(device, &batch)?;
    let task_buffer = MappedBuffer::from_slice(device, &batch.tasks)?;
    let header_buffer = MappedBuffer::from_slice(device, &[batch.header])?;
    let file_buffer = MappedBuffer::from_slice(device, &batch.files)?;
    let node_buffer = MappedBuffer::from_slice(device, &batch.nodes)?;
    let child_buffer = MappedBuffer::from_slice(device, &batch.children)?;
    let supertype_buffer = MappedBuffer::from_slice(device, &batch.supertypes)?;
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
    let source_buffer = MappedBuffer::from_slice(device, &batch.source)?;
    let value_state_buffer = &workspace.value_states;
    let line_start_buffer = MappedBuffer::from_slice(device, &batch.line_starts)?;
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
        &task_buffer,
        &header_buffer,
        &file_buffer,
        &node_buffer,
        &program_buffers.programs,
        &program_buffers.words,
        &program_buffers.longs,
        &child_buffer,
        &supertype_buffer,
        pending_buffer,
        capture_buffer,
        cursor_buffer,
        repeat_buffer,
        scheduler_buffer,
        result_buffer,
        status_buffer,
        &program_buffers.values,
        &program_buffers.strings,
        &program_buffers.nfa_states,
        &program_buffers.nfa_data,
        &program_buffers.nfa_ranges,
        &program_buffers.lua_instructions,
        &program_buffers.lua_members,
        &program_buffers.assertions,
        &program_buffers.assertion_data,
        &program_buffers.string_bytes,
        &source_buffer,
        value_state_buffer,
        &line_start_buffer,
        final_capture_buffer,
        output_storage_buffer,
    ]
    .into_iter()
    .enumerate()
    {
        encoder.set_buffer(index as u64, Some(&buffer.buffer), 0);
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
    #[cfg(test)]
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
