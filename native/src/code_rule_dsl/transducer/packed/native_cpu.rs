use super::{
    BatchOutcome, Error, Output, Program,
    metal_batch_abi::{
        ARENA_KIND_COUNT, FORMAT_VERSION, FinalCaptureRecord, GreedyEntryRecord, LinkRecord,
        LuaInstructionRecord, LuaSetMemberRecord, MetalNode, MetalSupertype, NONE, NfaRangeRecord,
        NfaStateRecord, PENDING_FOLLOWING, PENDING_GREEDY, PENDING_PATH, PathRecord, PendingRecord,
        PropertyRecord, QueryBucketRecord, QueryMemberRecord, QueuedStateRecord, RepeatRecord,
        ResultRecord, TaskStatus, ValueRecord,
    },
    native_cpu_capture, native_cpu_output, native_cpu_reduction, native_cpu_scheduler,
    native_cpu_value,
    native_ir::{ArenaKind, FunctionRef, NativeModule, StructuralOperation},
    packed_batch::{PackedBatch, PackedExecution, PackedResultPage},
    value_packing::PackedValueTables,
};
use crate::code_rule_dsl::tape::TapeDoc;
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlags, condcodes::IntCC, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Switch};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module, default_libcall_names};
use rayon::prelude::*;
use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::mem;
use std::ops::{Deref, DerefMut};
use std::ptr::NonNull;
use std::sync::{Arc, LazyLock, Mutex};

pub struct DemandZeroRegion<T> {
    pointer: NonNull<T>,
    count: usize,
    mapped_bytes: usize,
    marker: PhantomData<T>,
}

// wasm32 has no mmap: the region is a zeroed heap allocation (committed up front, which a browser's linear
// memory does anyway).
#[cfg(target_family = "wasm")]
impl<T> DemandZeroRegion<T> {
    fn layout(count: usize) -> Result<std::alloc::Layout, Error> {
        let bytes = count.checked_mul(mem::size_of::<T>()).ok_or(Error::BackendCapacity)?.max(1);
        std::alloc::Layout::from_size_align(bytes, mem::align_of::<T>().max(16)).map_err(|_| Error::BackendCapacity)
    }

    fn new(count: usize) -> Result<Self, Error> {
        let layout = Self::layout(count)?;
        // SAFETY: the layout has a non-zero size.
        let raw = unsafe { std::alloc::alloc_zeroed(layout) };
        let pointer = NonNull::new(raw.cast::<T>()).ok_or(Error::BackendCapacity)?;
        Ok(Self {
            pointer,
            count,
            mapped_bytes: layout.size(),
            marker: PhantomData,
        })
    }
}

#[cfg(target_family = "wasm")]
impl<T> Drop for DemandZeroRegion<T> {
    fn drop(&mut self) {
        let layout = Self::layout(self.count).expect("the layout new() allocated with");
        // SAFETY: allocated by `new` with this layout.
        unsafe { std::alloc::dealloc(self.pointer.as_ptr().cast(), layout) };
    }
}

#[cfg(unix)]
impl<T> DemandZeroRegion<T> {
    pub fn new(count: usize) -> Result<Self, Error> {
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if page <= 0 {
            return Err(Error::BackendFailure);
        }
        let page = page as usize;
        let wanted = count
            .checked_mul(mem::size_of::<T>())
            .map(|bytes| bytes.max(1))
            .ok_or(Error::BackendCapacity)?;
        let mapped_bytes = wanted
            .checked_add(page - 1)
            .map(|bytes| bytes / page * page)
            .ok_or(Error::BackendCapacity)?;
        let raw = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                mapped_bytes,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANON,
                -1,
                0,
            )
        };
        if raw == libc::MAP_FAILED {
            return Err(Error::BackendCapacity);
        }
        let pointer = NonNull::new(raw.cast::<T>()).ok_or(Error::BackendFailure)?;
        Ok(Self {
            pointer,
            count,
            mapped_bytes,
            marker: PhantomData,
        })
    }
}

impl<T> Deref for DemandZeroRegion<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        unsafe { std::slice::from_raw_parts(self.pointer.as_ptr(), self.count) }
    }
}

impl<T> DerefMut for DemandZeroRegion<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { std::slice::from_raw_parts_mut(self.pointer.as_ptr(), self.count) }
    }
}

#[cfg(unix)]
impl<T> Drop for DemandZeroRegion<T> {
    fn drop(&mut self) {
        unsafe {
            libc::madvise(
                self.pointer.as_ptr().cast(),
                self.mapped_bytes,
                libc::MADV_DONTNEED,
            );
            libc::munmap(self.pointer.as_ptr().cast(), self.mapped_bytes);
        }
    }
}

// Windows: committed VirtualAlloc pages are demand-zero, as an anonymous mapping's are.
#[cfg(windows)]
impl<T> DemandZeroRegion<T> {
    fn new(count: usize) -> Result<Self, Error> {
        use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_READWRITE, VirtualAlloc};
        let mapped_bytes = count
            .checked_mul(mem::size_of::<T>())
            .map(|bytes| bytes.max(1))
            .ok_or(Error::BackendCapacity)?;
        // SAFETY: a fresh region at an address the system picks; null on failure.
        let raw = unsafe { VirtualAlloc(std::ptr::null(), mapped_bytes, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE) };
        let pointer = NonNull::new(raw.cast::<T>()).ok_or(Error::BackendCapacity)?;
        Ok(Self {
            pointer,
            count,
            mapped_bytes,
            marker: PhantomData,
        })
    }
}

#[cfg(windows)]
impl<T> Drop for DemandZeroRegion<T> {
    fn drop(&mut self) {
        use windows_sys::Win32::System::Memory::{MEM_RELEASE, VirtualFree};
        // SAFETY: the region `new` reserved; MEM_RELEASE frees the whole reservation (size 0).
        unsafe { VirtualFree(self.pointer.as_ptr().cast(), 0, MEM_RELEASE) };
    }
}

#[repr(C)]
pub struct NativeTaskContext {
    pub nodes: *const MetalNode,
    pub node_count: u32,
    pub children: *const u32,
    pub child_count: u32,
    pub supertypes: *const MetalSupertype,
    pub supertype_count: u32,
    pub source: *const u8,
    pub source_length: u32,
    pub values: *const ValueRecord,
    pub value_count: u32,
    pub nfa_states: *const NfaStateRecord,
    pub nfa_state_count: u32,
    pub nfa_data: *const u32,
    pub nfa_data_count: u32,
    pub nfa_ranges: *const NfaRangeRecord,
    pub nfa_range_count: u32,
    pub lua_instructions: *const LuaInstructionRecord,
    pub lua_instruction_count: u32,
    pub lua_members: *const LuaSetMemberRecord,
    pub lua_member_count: u32,
    pub value_work: *mut u32,
    pub value_start: u32,
    pub value_capacity: u32,
    pub current_node: u32,
    pub start_node: u32,
    pub task_ordinal: u32,
    pub pending: *mut PendingRecord,
    pub pending_count: u32,
    pub pending_quota_limit: u32,
    pub current_steps: u32,
    pub current_captures: u32,
    pub current_cursors: u32,
    pub current_repeats: u32,
    pub current_following: u32,
    pub current_capture_hash: u32,
    pub current_cursor_hash: u32,
    pub current_repeat_hash: u32,
    pub task_work_limit: u32,
    pub path_work_limit: u32,
    pub captures: *const LinkRecord,
    pub cursors: *const LinkRecord,
    pub repeats: *const RepeatRecord,
    pub query_buckets: *mut QueryBucketRecord,
    pub query_bucket_capacity: u32,
    pub query_members: *mut QueryMemberRecord,
    pub query_member_count: u32,
    pub query_member_quota_limit: u32,
    pub queued_states: *mut QueuedStateRecord,
    pub queued_state_quota_limit: u32,
    pub greedy_entries: *mut GreedyEntryRecord,
    pub greedy_entry_quota_limit: u32,
    pub maximum_cursor_slot: u32,
    pub capture_count: u32,
    pub capture_quota_limit: u32,
    pub cursor_count: u32,
    pub cursor_quota_limit: u32,
    pub repeat_count: u32,
    pub repeat_quota_limit: u32,
    pub results: *mut ResultRecord,
    pub result_quota_limit: u32,
    pub line_starts: *const u32,
    pub line_count: u32,
    pub final_captures: *mut FinalCaptureRecord,
    pub final_capture_capacity: u32,
    pub properties: *mut PropertyRecord,
    pub property_quota_limit: u32,
    pub output_bytes: *mut u8,
    pub output_byte_quota_limit: u32,
    pub status: *mut TaskStatus,
    /// The batch's derived classes (`PackedBatch::derived`), which `SUPERTYPE` of a derived class
    /// tests as one bit.
    pub derived: *const u32,
    pub derived_classes: u32,
    pub derived_words: u32,
}

// Each packed task owns disjoint mutable ranges. Shared pointers are read-only.
unsafe impl Send for NativeTaskContext {}

pub(super) type NativeTaskEntry = unsafe extern "C" fn(*mut NativeTaskContext);

pub(super) type TaskEntry = unsafe extern "C" fn(*mut TaskStatus);

pub struct CompiledTaskShell {
    _module: JITModule,
    entry: TaskEntry,
}

impl CompiledTaskShell {
    pub fn execute(&self, status: &mut TaskStatus) {
        unsafe { (self.entry)(status) };
    }
}

pub fn compile_task_shell(_native: &NativeModule) -> Result<CompiledTaskShell, Error> {
    let builder = JITBuilder::new(default_libcall_names()).map_err(|_| Error::BackendFailure)?;
    let mut module = JITModule::new(builder);
    let mut signature = module.make_signature();
    signature
        .params
        .push(AbiParam::new(module.target_config().pointer_type()));
    let function_id = module
        .declare_function("tempo_native_task", Linkage::Local, &signature)
        .map_err(|_| Error::BackendFailure)?;
    let mut context = module.make_context();
    context.func.signature = signature;
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = function.create_block();
        function.append_block_params_for_function_params(entry);
        function.switch_to_block(entry);
        let status = function.block_params(entry)[0];
        let zero = function.ins().iconst(types::I32, 0);
        let flags = MemFlags::trusted();
        for offset in (0..mem::size_of::<TaskStatus>()).step_by(mem::size_of::<u32>()) {
            function.ins().store(flags, zero, status, offset as i32);
        }
        function.ins().return_(&[]);
        function.seal_all_blocks();
        function.finalize();
    }
    super::native_cpu_codegen::define_function(&mut module, function_id, &mut context)
        .map_err(|_| Error::BackendFailure)?;
    module.clear_context(&mut context);
    module
        .finalize_definitions()
        .map_err(|_| Error::BackendFailure)?;
    let code = module.get_finalized_function(function_id);
    let entry = unsafe { mem::transmute::<*const u8, TaskEntry>(code) };
    Ok(CompiledTaskShell {
        _module: module,
        entry,
    })
}

pub struct CompiledStraightLine {
    _module: JITModule,
    value_tables: PackedValueTables,
    entry: NativeTaskEntry,
    #[cfg(any(test, feature = "test-hooks"))]
    pub cursor_equal: unsafe extern "C" fn(*mut NativeTaskContext, u32, u32) -> u32,
    #[cfg(any(test, feature = "test-hooks"))]
    pub repeat_equal: unsafe extern "C" fn(*mut NativeTaskContext, u32, u32) -> u32,
    #[cfg(any(test, feature = "test-hooks"))]
    pub capture_contains: unsafe extern "C" fn(*mut NativeTaskContext, u32, u32) -> u32,
    #[cfg(any(test, feature = "test-hooks"))]
    pub query_admit: unsafe extern "C" fn(*mut NativeTaskContext, u32, u32) -> u32,
    #[cfg(any(test, feature = "test-hooks"))]
    pub exact_state_equal:
        unsafe extern "C" fn(*mut NativeTaskContext, u32, u32, u32, u32, u32, u32) -> u32,
    #[cfg(any(test, feature = "test-hooks"))]
    pub queue_insert: unsafe extern "C" fn(*mut NativeTaskContext, u32, u32) -> u32,
    #[cfg(any(test, feature = "test-hooks"))]
    pub greedy_schedule: unsafe extern "C" fn(*mut NativeTaskContext, u32) -> u64,
    #[cfg(any(test, feature = "test-hooks"))]
    pub greedy_take: unsafe extern "C" fn(*mut NativeTaskContext, u32) -> u64,
}

// Finalized JIT modules and packed constants are immutable during execution.
unsafe impl Send for CompiledStraightLine {}
unsafe impl Sync for CompiledStraightLine {}

// A wave's programs (its labels' and roots') fit: past this the oldest key goes.
const CPU_JIT_CACHE_LIMIT: usize = 4096;
static CPU_JIT_CACHE: LazyLock<Mutex<BTreeMap<String, Arc<CompiledStraightLine>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

impl CompiledStraightLine {
    pub fn execute(&self, context: &mut NativeTaskContext) {
        context.values = self.value_tables.values.as_ptr();
        context.value_count = self.value_tables.values.len() as u32;
        context.nfa_states = self.value_tables.nfa_states.as_ptr();
        context.nfa_state_count = self.value_tables.nfa_states.len() as u32;
        context.nfa_data = self.value_tables.nfa_data.as_ptr();
        context.nfa_data_count = self.value_tables.nfa_data.len() as u32;
        context.nfa_ranges = self.value_tables.nfa_ranges.as_ptr();
        context.nfa_range_count = self.value_tables.nfa_ranges.len() as u32;
        context.lua_instructions = self.value_tables.lua_instructions.as_ptr();
        context.lua_instruction_count = self.value_tables.lua_instructions.len() as u32;
        context.lua_members = self.value_tables.lua_members.as_ptr();
        context.lua_member_count = self.value_tables.lua_members.len() as u32;
        self.execute_prepacked(context);
    }

    fn execute_prepacked(&self, context: &mut NativeTaskContext) {
        unsafe { (self.entry)(context) };
    }
}

fn sibling_requirement(native: &NativeModule, position: Option<u16>) -> i64 {
    position
        .map(
            |position| match native.structural[usize::from(position)].operation {
                StructuralOperation::Next { .. } => 1,
                StructuralOperation::First { .. } => 2,
                StructuralOperation::Last { .. } => 3,
                _ => 0,
            },
        )
        .unwrap_or(0)
}

fn pending_path_field(field: usize) -> i32 {
    (mem::offset_of!(PendingRecord, path) + field) as i32
}

fn mix_hash(
    function: &mut FunctionBuilder<'_>,
    hash: cranelift_codegen::ir::Value,
    value: cranelift_codegen::ir::Value,
) -> cranelift_codegen::ir::Value {
    let left = function.ins().ishl_imm(hash, 6);
    let right = function.ins().ushr_imm(hash, 2);
    let mixed = function.ins().iadd(value, left);
    let mixed = function.ins().iadd(mixed, right);
    let mixed = function.ins().iadd_imm(mixed, 0x9e37_79b9_u32 as i64);
    function.ins().bxor(hash, mixed)
}

fn emit_task_error(
    function: &mut FunctionBuilder<'_>,
    task: cranelift_codegen::ir::Value,
    code: i64,
) {
    let flags = MemFlags::trusted();
    let status = context_pointer(function, task, mem::offset_of!(NativeTaskContext, status));
    let error = function.ins().iconst(types::I32, code);
    function.ins().store(
        flags,
        error,
        status,
        mem::offset_of!(TaskStatus, error) as i32,
    );
    let zero = function.ins().iconst(types::I32, 0);
    for offset in [
        mem::offset_of!(TaskStatus, result_count),
        mem::offset_of!(TaskStatus, capture_count),
        mem::offset_of!(TaskStatus, final_capture_count),
        mem::offset_of!(TaskStatus, property_count),
        mem::offset_of!(TaskStatus, output_byte_count),
    ] {
        function.ins().store(flags, zero, status, offset as i32);
    }
}

fn context_pointer(
    function: &mut FunctionBuilder<'_>,
    context: cranelift_codegen::ir::Value,
    offset: usize,
) -> cranelift_codegen::ir::Value {
    let pointer_type = function.func.dfg.value_type(context);
    function
        .ins()
        .load(pointer_type, MemFlags::trusted(), context, offset as i32)
}

pub fn compile_straight_line(native: &NativeModule) -> Result<CompiledStraightLine, Error> {
    let mut value_tables = PackedValueTables::with_unicode_word_data();
    value_tables.push_program(&native.value_programs)?;
    for instruction in &native.output_program.instructions {
        if let super::output::OutputInstruction::Strip { regex, .. } = instruction {
            value_tables.push_regex(regex)?;
        }
    }
    if native.capture_programs.iter().any(|program| {
        !matches!(
            program,
            super::value::CaptureProgram::Text { .. }
                | super::value::CaptureProgram::AnyOf { .. }
                | super::value::CaptureProgram::Kind { .. }
                | super::value::CaptureProgram::Contains { .. }
                | super::value::CaptureProgram::Relation { .. }
                | super::value::CaptureProgram::Equal { .. }
                | super::value::CaptureProgram::SameLine { .. }
        )
    }) {
        return Err(Error::UnsupportedBackend);
    }
    for block in &native.structural {
        if !matches!(
            block.operation,
            StructuralOperation::Stop
                | StructuralOperation::Kind { .. }
                | StructuralOperation::Field { .. }
                | StructuralOperation::Flags { .. }
                | StructuralOperation::Text { .. }
                | StructuralOperation::Assert { .. }
                | StructuralOperation::Size { .. }
                | StructuralOperation::First { .. }
                | StructuralOperation::Last { .. }
                | StructuralOperation::AbsentField { .. }
                | StructuralOperation::Supertype { .. }
                | StructuralOperation::Child { .. }
                | StructuralOperation::Next { .. }
                | StructuralOperation::Descend { .. }
                | StructuralOperation::Following { .. }
                | StructuralOperation::Parent
                | StructuralOperation::SaveCursor { .. }
                | StructuralOperation::RestoreCursor { .. }
                | StructuralOperation::Capture { .. }
                | StructuralOperation::RepeatBegin { .. }
                | StructuralOperation::RepeatBump { .. }
                | StructuralOperation::RepeatEnd { .. }
                | StructuralOperation::Fork { .. }
                | StructuralOperation::Goto { .. }
                | StructuralOperation::Emit { .. }
        ) {
            return Err(Error::UnsupportedBackend);
        }
    }

    let builder = JITBuilder::new(default_libcall_names()).map_err(|_| Error::BackendFailure)?;
    let mut module = JITModule::new(builder);
    let scheduler = native_cpu_scheduler::define(&mut module).map_err(|error| {
        #[cfg(any(test, feature = "test-hooks"))]
        eprintln!("native scheduler definition failed: {error:?}");
        #[cfg(not(any(test, feature = "test-hooks")))]
        let _ = error;
        Error::BackendFailure
    })?;
    let _ = (
        scheduler.cursor_value,
        scheduler.cursor_equal,
        scheduler.repeat_equal,
        scheduler.capture_contains,
        scheduler.query_admit,
        scheduler.exact_state_equal,
        scheduler.queue_insert,
        scheduler.queue_remove,
        scheduler.greedy_schedule,
        scheduler.greedy_take,
        scheduler.path_enqueue,
    );
    let value_functions = native_cpu_value::define(&mut module, native).map_err(|error| {
        #[cfg(any(test, feature = "test-hooks"))]
        eprintln!("native value definition failed: {error:?}");
        #[cfg(not(any(test, feature = "test-hooks")))]
        let _ = error;
        Error::BackendFailure
    })?;
    let capture_functions =
        native_cpu_capture::define(&mut module, native, &value_functions.evaluators).map_err(
            |error| {
                #[cfg(any(test, feature = "test-hooks"))]
                eprintln!("native capture definition failed: {error:?}");
                #[cfg(not(any(test, feature = "test-hooks")))]
                let _ = error;
                Error::BackendFailure
            },
        )?;
    let reduction_function = native_cpu_reduction::define(
        &mut module,
        scheduler.capture_contains,
        if native.tree_sitter_query {
            &capture_functions
        } else {
            &[]
        },
        native.tree_sitter_query && native.has_repeats,
    )
    .map_err(|error| {
        #[cfg(any(test, feature = "test-hooks"))]
        eprintln!("native reduction definition failed: {error:?}");
        #[cfg(not(any(test, feature = "test-hooks")))]
        let _ = error;
        Error::BackendFailure
    })?;
    let output_function =
        native_cpu_output::define(&mut module, native, value_functions.regex_find).map_err(
            |error| {
                #[cfg(any(test, feature = "test-hooks"))]
                eprintln!("native output definition failed: {error:?}");
                #[cfg(not(any(test, feature = "test-hooks")))]
                let _ = error;
                Error::UnsupportedBackend
            },
        )?;
    let pointer_type = module.target_config().pointer_type();
    let mut signature = module.make_signature();
    signature.params.push(AbiParam::new(pointer_type));
    let function_id = module
        .declare_function("tempo_native_straight_line", Linkage::Local, &signature)
        .map_err(|_| Error::BackendFailure)?;
    let mut context = module.make_context();
    context.func.signature = signature;
    let query_scheduler = native.tree_sitter_query && native.has_repeats;
    let cursor_value = module.declare_func_in_func(scheduler.cursor_value, &mut context.func);
    let value_functions = value_functions
        .evaluators
        .into_iter()
        .map(|function| module.declare_func_in_func(function, &mut context.func))
        .collect::<Vec<_>>();
    let capture_functions = capture_functions
        .into_iter()
        .map(|function| module.declare_func_in_func(function, &mut context.func))
        .collect::<Vec<_>>();
    let reduction_function = module.declare_func_in_func(reduction_function, &mut context.func);
    let output_function = module.declare_func_in_func(output_function, &mut context.func);
    let query_admit = query_scheduler
        .then(|| module.declare_func_in_func(scheduler.query_admit, &mut context.func));
    let queue_insert = query_scheduler
        .then(|| module.declare_func_in_func(scheduler.queue_insert, &mut context.func));
    let queue_remove = query_scheduler
        .then(|| module.declare_func_in_func(scheduler.queue_remove, &mut context.func));
    let greedy_schedule = module.declare_func_in_func(scheduler.greedy_schedule, &mut context.func);
    let greedy_take = module.declare_func_in_func(scheduler.greedy_take, &mut context.func);
    let path_enqueue = module.declare_func_in_func(scheduler.path_enqueue, &mut context.func);
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = function.create_block();
        function.append_block_params_for_function_params(entry);
        let resume = function.create_block();
        let done = function.create_block();
        let capacity_error = function.create_block();
        let work_error = function.create_block();
        let operand_error = function.create_block();
        let invalid_input = function.create_block();
        let invalid_instruction = function.create_block();
        let blocks = native
            .structural
            .iter()
            .map(|_| function.create_block())
            .collect::<Vec<_>>();
        let resume_targets = blocks
            .iter()
            .map(|_| function.create_block())
            .collect::<Vec<_>>();
        function.switch_to_block(entry);
        let task = function.block_params(entry)[0];

        let flags = MemFlags::trusted();
        function.ins().jump(blocks[0], &[]);

        for (index, block) in native.structural.iter().enumerate() {
            function.switch_to_block(blocks[index]);
            let status = context_pointer(
                &mut function,
                task,
                mem::offset_of!(NativeTaskContext, status),
            );
            let pc = function.ins().iconst(types::I32, i64::from(block.position));
            function.ins().store(
                flags,
                pc,
                status,
                mem::offset_of!(TaskStatus, program_counter) as i32,
            );
            if let Some(query_admit) = query_admit {
                let sibling = match block.operation {
                    StructuralOperation::Next { .. } => 1,
                    StructuralOperation::First { .. } => 2,
                    StructuralOperation::Last { .. } => 3,
                    _ => 0,
                };
                let sibling = function.ins().iconst(types::I32, sibling);
                let call = function.ins().call(query_admit, &[task, pc, sibling]);
                let admission = function.inst_results(call)[0];
                let accepted = function.ins().icmp_imm(IntCC::Equal, admission, 1);
                let check_capacity = function.create_block();
                let execute = function.create_block();
                function
                    .ins()
                    .brif(accepted, execute, &[], check_capacity, &[]);
                function.switch_to_block(check_capacity);
                let rejected = function.ins().icmp_imm(IntCC::Equal, admission, 0);
                let check_work = function.create_block();
                function.ins().brif(rejected, resume, &[], check_work, &[]);
                function.switch_to_block(check_work);
                let exhausted = function.ins().icmp_imm(IntCC::Equal, admission, 3);
                function
                    .ins()
                    .brif(exhausted, work_error, &[], capacity_error, &[]);
                function.switch_to_block(execute);
            }
            let aggregate_work = function.ins().load(
                types::I32,
                flags,
                status,
                mem::offset_of!(TaskStatus, reserved) as i32,
            );
            let prior_aggregate_work = aggregate_work;
            let aggregate_work = function.ins().iadd_imm(aggregate_work, 1);
            function.ins().store(
                flags,
                aggregate_work,
                status,
                mem::offset_of!(TaskStatus, reserved) as i32,
            );
            let work_limit = function.ins().load(
                types::I32,
                flags,
                task,
                mem::offset_of!(NativeTaskContext, task_work_limit) as i32,
            );
            let aggregate_within_limit =
                function
                    .ins()
                    .icmp(IntCC::UnsignedLessThanOrEqual, aggregate_work, work_limit);
            let aggregate_did_not_overflow = function.ins().icmp(
                IntCC::UnsignedGreaterThanOrEqual,
                aggregate_work,
                prior_aggregate_work,
            );
            let aggregate_within_limit = function
                .ins()
                .band(aggregate_within_limit, aggregate_did_not_overflow);
            let check_path_work = function.create_block();
            function.ins().brif(
                aggregate_within_limit,
                check_path_work,
                &[],
                work_error,
                &[],
            );
            function.switch_to_block(check_path_work);
            let steps = function.ins().load(
                types::I32,
                flags,
                task,
                mem::offset_of!(NativeTaskContext, current_steps) as i32,
            );
            let prior_steps = steps;
            let steps = function.ins().iadd_imm(prior_steps, 1);
            function.ins().store(
                flags,
                steps,
                task,
                mem::offset_of!(NativeTaskContext, current_steps) as i32,
            );
            let work_limit = function.ins().load(
                types::I32,
                flags,
                task,
                mem::offset_of!(NativeTaskContext, path_work_limit) as i32,
            );
            let within_limit =
                function
                    .ins()
                    .icmp(IntCC::UnsignedLessThanOrEqual, steps, work_limit);
            let did_not_overflow =
                function
                    .ins()
                    .icmp(IntCC::UnsignedGreaterThanOrEqual, steps, prior_steps);
            let within_limit = function.ins().band(within_limit, did_not_overflow);
            let body = function.create_block();
            function
                .ins()
                .brif(within_limit, body, &[], work_error, &[]);
            function.switch_to_block(body);
            let nodes = context_pointer(
                &mut function,
                task,
                mem::offset_of!(NativeTaskContext, nodes),
            );
            let current = function.ins().load(
                types::I32,
                flags,
                task,
                mem::offset_of!(NativeTaskContext, current_node) as i32,
            );
            let node_stride = function
                .ins()
                .imul_imm(current, mem::size_of::<MetalNode>() as i64);
            let node_offset = function.ins().uextend(pointer_type, node_stride);
            let node = function.ins().iadd(nodes, node_offset);
            let next = block.next.map(|next| blocks[usize::from(next)]);
            match block.operation {
                StructuralOperation::Stop => {
                    function.ins().jump(resume, &[]);
                }
                StructuralOperation::Kind { kind } => {
                    let actual = function.ins().load(
                        types::I32,
                        flags,
                        node,
                        mem::offset_of!(MetalNode, kind) as i32,
                    );
                    let keep = function
                        .ins()
                        .icmp_imm(IntCC::Equal, actual, i64::from(kind));
                    function
                        .ins()
                        .brif(keep, next.unwrap_or(resume), &[], resume, &[]);
                }
                StructuralOperation::Field { field } => {
                    let actual = function.ins().load(
                        types::I32,
                        flags,
                        node,
                        mem::offset_of!(MetalNode, field) as i32,
                    );
                    let keep = function
                        .ins()
                        .icmp_imm(IntCC::Equal, actual, i64::from(field));
                    function
                        .ins()
                        .brif(keep, next.unwrap_or(resume), &[], resume, &[]);
                }
                StructuralOperation::Flags {
                    required,
                    forbidden,
                } => {
                    let actual = function.ins().load(
                        types::I32,
                        flags,
                        node,
                        mem::offset_of!(MetalNode, flags) as i32,
                    );
                    let required_bits = function.ins().band_imm(actual, i64::from(required));
                    let required_ok =
                        function
                            .ins()
                            .icmp_imm(IntCC::Equal, required_bits, i64::from(required));
                    let forbidden_bits = function.ins().band_imm(actual, i64::from(forbidden));
                    let forbidden_ok = function.ins().icmp_imm(IntCC::Equal, forbidden_bits, 0);
                    let keep = function.ins().band(required_ok, forbidden_ok);
                    function
                        .ins()
                        .brif(keep, next.unwrap_or(resume), &[], resume, &[]);
                }
                StructuralOperation::Assert {
                    function: FunctionRef::Capture(_),
                } if native.tree_sitter_query => {
                    function.ins().jump(next.unwrap_or(resume), &[]);
                }
                StructuralOperation::Text {
                    function: FunctionRef::Value(value),
                }
                | StructuralOperation::Assert {
                    function: FunctionRef::Capture(value),
                } => {
                    let call = if matches!(
                        native.structural[index].operation,
                        StructuralOperation::Text { .. }
                    ) {
                        function
                            .ins()
                            .call(value_functions[usize::from(value)], &[task, current])
                    } else {
                        function
                            .ins()
                            .call(capture_functions[usize::from(value)], &[task])
                    };
                    let outcome = function.inst_results(call)[0];
                    let kept = function.ins().icmp_imm(IntCC::Equal, outcome, 1);
                    let not_kept = function.create_block();
                    function
                        .ins()
                        .brif(kept, next.unwrap_or(resume), &[], not_kept, &[]);
                    function.switch_to_block(not_kept);
                    let rejected = function.ins().icmp_imm(IntCC::Equal, outcome, 0);
                    let classify_error = function.create_block();
                    function
                        .ins()
                        .brif(rejected, resume, &[], classify_error, &[]);
                    function.switch_to_block(classify_error);
                    let exhausted_work = function.ins().icmp_imm(IntCC::Equal, outcome, 3);
                    let classify_capacity = function.create_block();
                    function
                        .ins()
                        .brif(exhausted_work, work_error, &[], classify_capacity, &[]);
                    function.switch_to_block(classify_capacity);
                    let exhausted_capacity = function.ins().icmp_imm(IntCC::Equal, outcome, 4);
                    function.ins().brif(
                        exhausted_capacity,
                        capacity_error,
                        &[],
                        invalid_instruction,
                        &[],
                    );
                }
                StructuralOperation::Size { mode, operand } if mode & 0x80 == 0 => {
                    let start = function.ins().load(
                        types::I32,
                        flags,
                        node,
                        mem::offset_of!(MetalNode, source_start) as i32,
                    );
                    let end = function.ins().load(
                        types::I32,
                        flags,
                        node,
                        mem::offset_of!(MetalNode, source_end) as i32,
                    );
                    let actual = function.ins().isub(end, start);
                    let condition = match mode {
                        0 => IntCC::Equal,
                        1 => IntCC::UnsignedGreaterThanOrEqual,
                        2 => IntCC::UnsignedLessThanOrEqual,
                        _ => return Err(Error::UnsupportedBackend),
                    };
                    let keep = function
                        .ins()
                        .icmp_imm(condition, actual, i64::from(operand));
                    function
                        .ins()
                        .brif(keep, next.unwrap_or(resume), &[], resume, &[]);
                }
                StructuralOperation::Size { mode, operand } => {
                    let start = function.ins().load(
                        types::I32,
                        flags,
                        node,
                        mem::offset_of!(MetalNode, source_start) as i32,
                    );
                    let end = function.ins().load(
                        types::I32,
                        flags,
                        node,
                        mem::offset_of!(MetalNode, source_end) as i32,
                    );
                    let actual = function.ins().isub(end, start);
                    let actual = function.ins().uextend(types::I64, actual);
                    let expected = native.longs[usize::from(operand)];
                    let expected = function.ins().iconst(types::I64, expected as i64);
                    let condition = match mode & 0x7f {
                        0 => IntCC::Equal,
                        1 => IntCC::UnsignedGreaterThanOrEqual,
                        2 => IntCC::UnsignedLessThanOrEqual,
                        _ => return Err(Error::UnsupportedBackend),
                    };
                    let keep = function.ins().icmp(condition, actual, expected);
                    function
                        .ins()
                        .brif(keep, next.unwrap_or(resume), &[], resume, &[]);
                }
                StructuralOperation::First { named } | StructuralOperation::Last { named } => {
                    let parent = function.ins().load(
                        types::I32,
                        flags,
                        node,
                        mem::offset_of!(MetalNode, parent) as i32,
                    );
                    let root = function
                        .ins()
                        .icmp_imm(IntCC::Equal, parent, i64::from(NONE));
                    let accept_root = function.create_block();
                    let inspect_parent = function.create_block();
                    function
                        .ins()
                        .brif(root, accept_root, &[], inspect_parent, &[]);
                    function.switch_to_block(accept_root);
                    function.ins().jump(next.unwrap_or(resume), &[]);
                    function.switch_to_block(inspect_parent);
                    let parent_stride = function
                        .ins()
                        .imul_imm(parent, mem::size_of::<MetalNode>() as i64);
                    let parent_stride = function.ins().uextend(pointer_type, parent_stride);
                    let parent_node = function.ins().iadd(nodes, parent_stride);
                    let child_start = function.ins().load(
                        types::I32,
                        flags,
                        parent_node,
                        mem::offset_of!(MetalNode, child_start) as i32,
                    );
                    let child_count = function.ins().load(
                        types::I32,
                        flags,
                        parent_node,
                        mem::offset_of!(MetalNode, child_count) as i32,
                    );
                    let children = context_pointer(
                        &mut function,
                        task,
                        mem::offset_of!(NativeTaskContext, children),
                    );
                    let scan = function.create_block();
                    function.append_block_param(scan, types::I32);
                    function.append_block_param(scan, types::I32);
                    let inspect = function.create_block();
                    function.append_block_param(inspect, types::I32);
                    function.append_block_param(inspect, types::I32);
                    let finish = function.create_block();
                    function.append_block_param(finish, types::I32);
                    let zero = function.ins().iconst(types::I32, 0);
                    let none = function.ins().iconst(types::I32, i64::from(NONE));
                    function.ins().jump(scan, &[zero.into(), none.into()]);
                    function.switch_to_block(scan);
                    let index = function.block_params(scan)[0];
                    let found = function.block_params(scan)[1];
                    let in_range = function
                        .ins()
                        .icmp(IntCC::UnsignedLessThan, index, child_count);
                    function.ins().brif(
                        in_range,
                        inspect,
                        &[index.into(), found.into()],
                        finish,
                        &[found.into()],
                    );
                    function.switch_to_block(inspect);
                    let index = function.block_params(inspect)[0];
                    let found = function.block_params(inspect)[1];
                    let absolute = function.ins().iadd(child_start, index);
                    let offset = function
                        .ins()
                        .imul_imm(absolute, mem::size_of::<u32>() as i64);
                    let offset = function.ins().uextend(pointer_type, offset);
                    let address = function.ins().iadd(children, offset);
                    let child = function.ins().load(types::I32, flags, address, 0);
                    let eligible = if named {
                        let stride = function
                            .ins()
                            .imul_imm(child, mem::size_of::<MetalNode>() as i64);
                        let stride = function.ins().uextend(pointer_type, stride);
                        let child_node = function.ins().iadd(nodes, stride);
                        let child_flags = function.ins().load(
                            types::I32,
                            flags,
                            child_node,
                            mem::offset_of!(MetalNode, flags) as i32,
                        );
                        let named_bit = function.ins().band_imm(child_flags, 1);
                        function.ins().icmp_imm(IntCC::NotEqual, named_bit, 0)
                    } else {
                        function.ins().icmp(IntCC::Equal, index, index)
                    };
                    let choose = if matches!(block.operation, StructuralOperation::First { .. }) {
                        let missing = function
                            .ins()
                            .icmp_imm(IntCC::Equal, found, i64::from(NONE));
                        function.ins().band(eligible, missing)
                    } else {
                        eligible
                    };
                    let found = function.ins().select(choose, child, found);
                    let index = function.ins().iadd_imm(index, 1);
                    function.ins().jump(scan, &[index.into(), found.into()]);
                    function.switch_to_block(finish);
                    let found = function.block_params(finish)[0];
                    let keep = function.ins().icmp(IntCC::Equal, found, current);
                    function
                        .ins()
                        .brif(keep, next.unwrap_or(resume), &[], resume, &[]);
                }
                StructuralOperation::Supertype { kind } if kind >= crate::code_rule_dsl::tape::DERIVED_CLASS_BASE => {
                    // A derived class is one bit of the batch's bitsets; a class past them is absent.
                    let class = u32::from(kind - crate::code_rule_dsl::tape::DERIVED_CLASS_BASE);
                    let classes = function.ins().load(
                        types::I32,
                        flags,
                        task,
                        mem::offset_of!(NativeTaskContext, derived_classes) as i32,
                    );
                    let present = function
                        .ins()
                        .icmp_imm(IntCC::UnsignedGreaterThan, classes, i64::from(class));
                    let test = function.create_block();
                    function.ins().brif(present, test, &[], resume, &[]);
                    function.switch_to_block(test);
                    let bits = context_pointer(
                        &mut function,
                        task,
                        mem::offset_of!(NativeTaskContext, derived),
                    );
                    let words = function.ins().load(
                        types::I32,
                        flags,
                        task,
                        mem::offset_of!(NativeTaskContext, derived_words) as i32,
                    );
                    let row = function.ins().imul_imm(words, i64::from(class));
                    let column = function.ins().ushr_imm(current, 5);
                    let index = function.ins().iadd(row, column);
                    let offset = function.ins().imul_imm(index, 4);
                    let offset = function.ins().uextend(pointer_type, offset);
                    let address = function.ins().iadd(bits, offset);
                    let word = function.ins().load(types::I32, flags, address, 0);
                    let shift = function.ins().band_imm(current, 31);
                    let bit = function.ins().ushr(word, shift);
                    let bit = function.ins().band_imm(bit, 1);
                    function
                        .ins()
                        .brif(bit, next.unwrap_or(resume), &[], resume, &[]);
                }
                StructuralOperation::Supertype { kind } => {
                    // The batch's memberships are sorted by (node, kind), as the interpreters'
                    // binary search reads them: the lower bound of (current, kind), then one test.
                    let memberships = context_pointer(
                        &mut function,
                        task,
                        mem::offset_of!(NativeTaskContext, supertypes),
                    );
                    let membership_count = function.ins().load(
                        types::I32,
                        flags,
                        task,
                        mem::offset_of!(NativeTaskContext, supertype_count) as i32,
                    );
                    let key = |function: &mut FunctionBuilder<'_>, node: cranelift_codegen::ir::Value, kind: cranelift_codegen::ir::Value| {
                        let node = function.ins().uextend(types::I64, node);
                        let node = function.ins().ishl_imm(node, 32);
                        let kind = function.ins().uextend(types::I64, kind);
                        function.ins().bor(node, kind)
                    };
                    let member = |function: &mut FunctionBuilder<'_>, index: cranelift_codegen::ir::Value| {
                        let stride = function
                            .ins()
                            .imul_imm(index, mem::size_of::<MetalSupertype>() as i64);
                        let stride = function.ins().uextend(pointer_type, stride);
                        let membership = function.ins().iadd(memberships, stride);
                        let node = function.ins().load(
                            types::I32,
                            flags,
                            membership,
                            mem::offset_of!(MetalSupertype, node) as i32,
                        );
                        let kind = function.ins().load(
                            types::I32,
                            flags,
                            membership,
                            mem::offset_of!(MetalSupertype, kind) as i32,
                        );
                        (node, kind)
                    };
                    let wanted = function.ins().iconst(types::I32, i64::from(kind));
                    let target = key(&mut function, current, wanted);
                    let search = function.create_block();
                    function.append_block_param(search, types::I32);
                    function.append_block_param(search, types::I32);
                    let probe = function.create_block();
                    function.append_block_param(probe, types::I32);
                    function.append_block_param(probe, types::I32);
                    let finish = function.create_block();
                    function.append_block_param(finish, types::I32);
                    let test = function.create_block();
                    function.append_block_param(test, types::I32);
                    let zero = function.ins().iconst(types::I32, 0);
                    function
                        .ins()
                        .jump(search, &[zero.into(), membership_count.into()]);
                    function.switch_to_block(search);
                    let low = function.block_params(search)[0];
                    let high = function.block_params(search)[1];
                    let open = function.ins().icmp(IntCC::UnsignedLessThan, low, high);
                    function.ins().brif(
                        open,
                        probe,
                        &[low.into(), high.into()],
                        finish,
                        &[low.into()],
                    );
                    function.switch_to_block(probe);
                    let low = function.block_params(probe)[0];
                    let high = function.block_params(probe)[1];
                    let width = function.ins().isub(high, low);
                    let half = function.ins().ushr_imm(width, 1);
                    let middle = function.ins().iadd(low, half);
                    let (node, member_kind) = member(&mut function, middle);
                    let probed = key(&mut function, node, member_kind);
                    let below = function.ins().icmp(IntCC::UnsignedLessThan, probed, target);
                    let past = function.ins().iadd_imm(middle, 1);
                    let low = function.ins().select(below, past, low);
                    let high = function.ins().select(below, high, middle);
                    function.ins().jump(search, &[low.into(), high.into()]);
                    function.switch_to_block(finish);
                    let low = function.block_params(finish)[0];
                    let in_range =
                        function
                            .ins()
                            .icmp(IntCC::UnsignedLessThan, low, membership_count);
                    function
                        .ins()
                        .brif(in_range, test, &[low.into()], resume, &[]);
                    function.switch_to_block(test);
                    let low = function.block_params(test)[0];
                    let (node, member_kind) = member(&mut function, low);
                    let node_count = function.ins().load(
                        types::I32,
                        flags,
                        task,
                        mem::offset_of!(NativeTaskContext, node_count) as i32,
                    );
                    let valid_node = function.ins().icmp(IntCC::UnsignedLessThan, node, node_count);
                    let compare = function.create_block();
                    function
                        .ins()
                        .brif(valid_node, compare, &[], invalid_input, &[]);
                    function.switch_to_block(compare);
                    let found = key(&mut function, node, member_kind);
                    let matched = function.ins().icmp(IntCC::Equal, found, target);
                    function
                        .ins()
                        .brif(matched, next.unwrap_or(resume), &[], resume, &[]);
                }
                StructuralOperation::AbsentField { field } => {
                    let child_start = function.ins().load(
                        types::I32,
                        flags,
                        node,
                        mem::offset_of!(MetalNode, child_start) as i32,
                    );
                    let child_count = function.ins().load(
                        types::I32,
                        flags,
                        node,
                        mem::offset_of!(MetalNode, child_count) as i32,
                    );
                    let child_end = function.ins().iadd(child_start, child_count);
                    let no_overflow = function.ins().icmp(
                        IntCC::UnsignedGreaterThanOrEqual,
                        child_end,
                        child_start,
                    );
                    let packed_child_count = function.ins().load(
                        types::I32,
                        flags,
                        task,
                        mem::offset_of!(NativeTaskContext, child_count) as i32,
                    );
                    let in_bounds = function.ins().icmp(
                        IntCC::UnsignedLessThanOrEqual,
                        child_end,
                        packed_child_count,
                    );
                    let valid = function.ins().band(no_overflow, in_bounds);
                    let scan = function.create_block();
                    function.append_block_param(scan, types::I32);
                    function
                        .ins()
                        .brif(valid, scan, &[child_start.into()], invalid_input, &[]);
                    function.switch_to_block(scan);
                    let index = function.block_params(scan)[0];
                    let has_child = function
                        .ins()
                        .icmp(IntCC::UnsignedLessThan, index, child_end);
                    let inspect = function.create_block();
                    function
                        .ins()
                        .brif(has_child, inspect, &[], next.unwrap_or(resume), &[]);
                    function.switch_to_block(inspect);
                    let children = context_pointer(
                        &mut function,
                        task,
                        mem::offset_of!(NativeTaskContext, children),
                    );
                    let offset = function.ins().imul_imm(index, mem::size_of::<u32>() as i64);
                    let offset = function.ins().uextend(pointer_type, offset);
                    let address = function.ins().iadd(children, offset);
                    let child = function.ins().load(types::I32, flags, address, 0);
                    let node_count = function.ins().load(
                        types::I32,
                        flags,
                        task,
                        mem::offset_of!(NativeTaskContext, node_count) as i32,
                    );
                    let valid_child =
                        function
                            .ins()
                            .icmp(IntCC::UnsignedLessThan, child, node_count);
                    let inspect_field = function.create_block();
                    function
                        .ins()
                        .brif(valid_child, inspect_field, &[], invalid_input, &[]);
                    function.switch_to_block(inspect_field);
                    let stride = function
                        .ins()
                        .imul_imm(child, mem::size_of::<MetalNode>() as i64);
                    let stride = function.ins().uextend(pointer_type, stride);
                    let child_node = function.ins().iadd(nodes, stride);
                    let actual = function.ins().load(
                        types::I32,
                        flags,
                        child_node,
                        mem::offset_of!(MetalNode, field) as i32,
                    );
                    let present = function
                        .ins()
                        .icmp_imm(IntCC::Equal, actual, i64::from(field));
                    let advance = function.create_block();
                    function.ins().brif(present, resume, &[], advance, &[]);
                    function.switch_to_block(advance);
                    let index = function.ins().iadd_imm(index, 1);
                    function.ins().jump(scan, &[index.into()]);
                }
                StructuralOperation::Child { named } => {
                    let child_count = function.ins().load(
                        types::I32,
                        flags,
                        node,
                        mem::offset_of!(MetalNode, child_count) as i32,
                    );
                    let child_start = function.ins().load(
                        types::I32,
                        flags,
                        node,
                        mem::offset_of!(MetalNode, child_start) as i32,
                    );
                    let children = context_pointer(
                        &mut function,
                        task,
                        mem::offset_of!(NativeTaskContext, children),
                    );
                    let scan = function.create_block();
                    function.append_block_param(scan, types::I32);
                    let inspect = function.create_block();
                    function.append_block_param(inspect, types::I32);
                    let found = function.create_block();
                    function.append_block_param(found, types::I32);
                    let reverse = function.create_block();
                    function.append_block_param(reverse, types::I32);
                    function.append_block_param(reverse, types::I32);
                    let reverse_inspect = function.create_block();
                    function.append_block_param(reverse_inspect, types::I32);
                    function.append_block_param(reverse_inspect, types::I32);
                    let execute = function.create_block();
                    function.append_block_param(execute, types::I32);
                    let zero = function.ins().iconst(types::I32, 0);
                    function.ins().jump(scan, &[zero.into()]);
                    function.switch_to_block(scan);
                    let index = function.block_params(scan)[0];
                    let in_range = function
                        .ins()
                        .icmp(IntCC::UnsignedLessThan, index, child_count);
                    function
                        .ins()
                        .brif(in_range, inspect, &[index.into()], resume, &[]);
                    function.switch_to_block(inspect);
                    let index = function.block_params(inspect)[0];
                    let absolute = function.ins().iadd(child_start, index);
                    let offset = function
                        .ins()
                        .imul_imm(absolute, mem::size_of::<u32>() as i64);
                    let offset = function.ins().uextend(pointer_type, offset);
                    let address = function.ins().iadd(children, offset);
                    let child = function.ins().load(types::I32, flags, address, 0);
                    let eligible = if named {
                        let stride = function
                            .ins()
                            .imul_imm(child, mem::size_of::<MetalNode>() as i64);
                        let stride = function.ins().uextend(pointer_type, stride);
                        let child_node = function.ins().iadd(nodes, stride);
                        let child_flags = function.ins().load(
                            types::I32,
                            flags,
                            child_node,
                            mem::offset_of!(MetalNode, flags) as i32,
                        );
                        let named_bit = function.ins().band_imm(child_flags, 1);
                        function.ins().icmp_imm(IntCC::NotEqual, named_bit, 0)
                    } else {
                        function.ins().icmp(IntCC::Equal, index, index)
                    };
                    let advance_scan = function.create_block();
                    function
                        .ins()
                        .brif(eligible, found, &[child.into()], advance_scan, &[]);
                    function.switch_to_block(advance_scan);
                    let index = function.ins().iadd_imm(index, 1);
                    function.ins().jump(scan, &[index.into()]);
                    function.switch_to_block(found);
                    let first = function.block_params(found)[0];
                    function
                        .ins()
                        .jump(reverse, &[child_count.into(), first.into()]);
                    function.switch_to_block(reverse);
                    let index = function.block_params(reverse)[0];
                    let first = function.block_params(reverse)[1];
                    let remaining = function.ins().icmp_imm(IntCC::NotEqual, index, 0);
                    function.ins().brif(
                        remaining,
                        reverse_inspect,
                        &[index.into(), first.into()],
                        execute,
                        &[first.into()],
                    );
                    function.switch_to_block(reverse_inspect);
                    let reverse_index = function.block_params(reverse_inspect)[0];
                    let index = function.ins().iadd_imm(reverse_index, -1);
                    let first = function.block_params(reverse_inspect)[1];
                    let absolute = function.ins().iadd(child_start, index);
                    let offset = function
                        .ins()
                        .imul_imm(absolute, mem::size_of::<u32>() as i64);
                    let offset = function.ins().uextend(pointer_type, offset);
                    let address = function.ins().iadd(children, offset);
                    let child = function.ins().load(types::I32, flags, address, 0);
                    let reached_first = function.ins().icmp(IntCC::Equal, child, first);
                    let consider = function.create_block();
                    function
                        .ins()
                        .brif(reached_first, execute, &[first.into()], consider, &[]);
                    function.switch_to_block(consider);
                    let eligible = if named {
                        let stride = function
                            .ins()
                            .imul_imm(child, mem::size_of::<MetalNode>() as i64);
                        let stride = function.ins().uextend(pointer_type, stride);
                        let child_node = function.ins().iadd(nodes, stride);
                        let child_flags = function.ins().load(
                            types::I32,
                            flags,
                            child_node,
                            mem::offset_of!(MetalNode, flags) as i32,
                        );
                        let named_bit = function.ins().band_imm(child_flags, 1);
                        function.ins().icmp_imm(IntCC::NotEqual, named_bit, 0)
                    } else {
                        function.ins().icmp(IntCC::Equal, index, index)
                    };
                    let enqueue = function.create_block();
                    function.ins().brif(
                        eligible,
                        enqueue,
                        &[],
                        reverse,
                        &[index.into(), first.into()],
                    );
                    function.switch_to_block(enqueue);
                    let target_pc = function
                        .ins()
                        .iconst(types::I32, i64::from(block.next.unwrap_or(block.position)));
                    let sibling = function
                        .ins()
                        .iconst(types::I32, sibling_requirement(native, block.next));
                    let mode = function.ins().iconst(types::I32, 0);
                    let kind = function.ins().iconst(types::I32, i64::from(PENDING_PATH));
                    let call = function
                        .ins()
                        .call(path_enqueue, &[task, target_pc, child, mode, sibling, kind]);
                    let outcome = function.inst_results(call)[0];
                    let ok = function.ins().icmp_imm(IntCC::NotEqual, outcome, 2);
                    function.ins().brif(
                        ok,
                        reverse,
                        &[index.into(), first.into()],
                        capacity_error,
                        &[],
                    );
                    function.switch_to_block(execute);
                    let first = function.block_params(execute)[0];
                    function.ins().store(
                        flags,
                        first,
                        task,
                        mem::offset_of!(NativeTaskContext, current_node) as i32,
                    );
                    function.ins().jump(next.unwrap_or(resume), &[]);
                }
                StructuralOperation::Next { named } => {
                    let offset = if named {
                        mem::offset_of!(MetalNode, next_named)
                    } else {
                        mem::offset_of!(MetalNode, next_any)
                    };
                    let target = function.ins().load(types::I32, flags, node, offset as i32);
                    let found = function
                        .ins()
                        .icmp_imm(IntCC::NotEqual, target, i64::from(NONE));
                    let advance = function.create_block();
                    function.ins().brif(found, advance, &[], resume, &[]);
                    function.switch_to_block(advance);
                    function.ins().store(
                        flags,
                        target,
                        task,
                        mem::offset_of!(NativeTaskContext, current_node) as i32,
                    );
                    function.ins().jump(next.unwrap_or(resume), &[]);
                }
                StructuralOperation::Descend { named } => {
                    let end = function.ins().load(
                        types::I32,
                        flags,
                        node,
                        mem::offset_of!(MetalNode, descendant_end) as i32,
                    );
                    let first_candidate = function.ins().iadd_imm(current, 1);
                    let scan = function.create_block();
                    function.append_block_param(scan, types::I32);
                    let inspect = function.create_block();
                    function.append_block_param(inspect, types::I32);
                    let found = function.create_block();
                    function.append_block_param(found, types::I32);
                    let reverse = function.create_block();
                    function.append_block_param(reverse, types::I32);
                    function.append_block_param(reverse, types::I32);
                    let reverse_inspect = function.create_block();
                    function.append_block_param(reverse_inspect, types::I32);
                    function.append_block_param(reverse_inspect, types::I32);
                    let execute = function.create_block();
                    function.append_block_param(execute, types::I32);
                    function.ins().jump(scan, &[first_candidate.into()]);
                    function.switch_to_block(scan);
                    let candidate = function.block_params(scan)[0];
                    let in_range = function.ins().icmp(IntCC::UnsignedLessThan, candidate, end);
                    function
                        .ins()
                        .brif(in_range, inspect, &[candidate.into()], resume, &[]);
                    function.switch_to_block(inspect);
                    let candidate = function.block_params(inspect)[0];
                    let eligible = if named {
                        let stride = function
                            .ins()
                            .imul_imm(candidate, mem::size_of::<MetalNode>() as i64);
                        let stride = function.ins().uextend(pointer_type, stride);
                        let candidate_node = function.ins().iadd(nodes, stride);
                        let candidate_flags = function.ins().load(
                            types::I32,
                            flags,
                            candidate_node,
                            mem::offset_of!(MetalNode, flags) as i32,
                        );
                        let named_bit = function.ins().band_imm(candidate_flags, 1);
                        function.ins().icmp_imm(IntCC::NotEqual, named_bit, 0)
                    } else {
                        function.ins().icmp(IntCC::Equal, candidate, candidate)
                    };
                    let advance = function.create_block();
                    function
                        .ins()
                        .brif(eligible, found, &[candidate.into()], advance, &[]);
                    function.switch_to_block(advance);
                    let candidate = function.ins().iadd_imm(candidate, 1);
                    function.ins().jump(scan, &[candidate.into()]);
                    function.switch_to_block(found);
                    let first = function.block_params(found)[0];
                    function.ins().jump(reverse, &[end.into(), first.into()]);
                    function.switch_to_block(reverse);
                    let candidate = function.block_params(reverse)[0];
                    let first = function.block_params(reverse)[1];
                    let remaining =
                        function
                            .ins()
                            .icmp(IntCC::UnsignedGreaterThan, candidate, first);
                    function.ins().brif(
                        remaining,
                        reverse_inspect,
                        &[candidate.into(), first.into()],
                        execute,
                        &[first.into()],
                    );
                    function.switch_to_block(reverse_inspect);
                    let reverse_candidate = function.block_params(reverse_inspect)[0];
                    let candidate = function.ins().iadd_imm(reverse_candidate, -1);
                    let first = function.block_params(reverse_inspect)[1];
                    let reached_first = function.ins().icmp(IntCC::Equal, candidate, first);
                    let consider = function.create_block();
                    function
                        .ins()
                        .brif(reached_first, execute, &[first.into()], consider, &[]);
                    function.switch_to_block(consider);
                    let eligible = if named {
                        let stride = function
                            .ins()
                            .imul_imm(candidate, mem::size_of::<MetalNode>() as i64);
                        let stride = function.ins().uextend(pointer_type, stride);
                        let candidate_node = function.ins().iadd(nodes, stride);
                        let candidate_flags = function.ins().load(
                            types::I32,
                            flags,
                            candidate_node,
                            mem::offset_of!(MetalNode, flags) as i32,
                        );
                        let named_bit = function.ins().band_imm(candidate_flags, 1);
                        function.ins().icmp_imm(IntCC::NotEqual, named_bit, 0)
                    } else {
                        function.ins().icmp(IntCC::Equal, candidate, candidate)
                    };
                    let enqueue = function.create_block();
                    function.ins().brif(
                        eligible,
                        enqueue,
                        &[],
                        reverse,
                        &[candidate.into(), first.into()],
                    );
                    function.switch_to_block(enqueue);
                    let target_pc = function
                        .ins()
                        .iconst(types::I32, i64::from(block.next.unwrap_or(block.position)));
                    let sibling = function
                        .ins()
                        .iconst(types::I32, sibling_requirement(native, block.next));
                    let mode = function.ins().iconst(types::I32, 0);
                    let kind = function.ins().iconst(types::I32, i64::from(PENDING_PATH));
                    let call = function.ins().call(
                        path_enqueue,
                        &[task, target_pc, candidate, mode, sibling, kind],
                    );
                    let outcome = function.inst_results(call)[0];
                    let ok = function.ins().icmp_imm(IntCC::NotEqual, outcome, 2);
                    function.ins().brif(
                        ok,
                        reverse,
                        &[candidate.into(), first.into()],
                        capacity_error,
                        &[],
                    );
                    function.switch_to_block(execute);
                    let first = function.block_params(execute)[0];
                    function.ins().store(
                        flags,
                        first,
                        task,
                        mem::offset_of!(NativeTaskContext, current_node) as i32,
                    );
                    function.ins().jump(next.unwrap_or(resume), &[]);
                }
                StructuralOperation::Following { named } => {
                    let offset = if named {
                        mem::offset_of!(MetalNode, next_named)
                    } else {
                        mem::offset_of!(MetalNode, next_any)
                    };
                    let first = function.ins().load(types::I32, flags, node, offset as i32);
                    let found = function
                        .ins()
                        .icmp_imm(IntCC::NotEqual, first, i64::from(NONE));
                    let advance = function.create_block();
                    function.ins().brif(found, advance, &[], resume, &[]);
                    function.switch_to_block(advance);
                    let first_stride = function
                        .ins()
                        .imul_imm(first, mem::size_of::<MetalNode>() as i64);
                    let first_offset = function.ins().uextend(pointer_type, first_stride);
                    let first_node = function.ins().iadd(nodes, first_offset);
                    let second = function
                        .ins()
                        .load(types::I32, flags, first_node, offset as i32);
                    let has_second =
                        function
                            .ins()
                            .icmp_imm(IntCC::NotEqual, second, i64::from(NONE));
                    let enqueue = function.create_block();
                    let execute_first = function.create_block();
                    function
                        .ins()
                        .brif(has_second, enqueue, &[], execute_first, &[]);
                    function.switch_to_block(enqueue);
                    let continuation_pc = function
                        .ins()
                        .iconst(types::I32, i64::from(block.next.unwrap_or(block.position)));
                    let mode = function.ins().iconst(types::I32, if named { 2 } else { 1 });
                    let sibling_requirement = block
                        .next
                        .map(
                            |position| match native.structural[usize::from(position)].operation {
                                StructuralOperation::Next { .. } => 1,
                                StructuralOperation::First { .. } => 2,
                                StructuralOperation::Last { .. } => 3,
                                _ => 0,
                            },
                        )
                        .unwrap_or(0);
                    let sibling_requirement =
                        function.ins().iconst(types::I32, sibling_requirement);
                    let kind = function
                        .ins()
                        .iconst(types::I32, i64::from(PENDING_FOLLOWING));
                    let call = function.ins().call(
                        path_enqueue,
                        &[
                            task,
                            continuation_pc,
                            second,
                            mode,
                            sibling_requirement,
                            kind,
                        ],
                    );
                    let outcome = function.inst_results(call)[0];
                    let has_capacity = function.ins().icmp_imm(IntCC::NotEqual, outcome, 2);
                    function
                        .ins()
                        .brif(has_capacity, execute_first, &[], capacity_error, &[]);
                    function.switch_to_block(execute_first);
                    function.ins().store(
                        flags,
                        first,
                        task,
                        mem::offset_of!(NativeTaskContext, current_node) as i32,
                    );
                    function.ins().jump(next.unwrap_or(resume), &[]);
                }
                StructuralOperation::Parent => {
                    let target = function.ins().load(
                        types::I32,
                        flags,
                        node,
                        mem::offset_of!(MetalNode, parent) as i32,
                    );
                    let found = function
                        .ins()
                        .icmp_imm(IntCC::NotEqual, target, i64::from(NONE));
                    let advance = function.create_block();
                    function.ins().brif(found, advance, &[], resume, &[]);
                    function.switch_to_block(advance);
                    function.ins().store(
                        flags,
                        target,
                        task,
                        mem::offset_of!(NativeTaskContext, current_node) as i32,
                    );
                    function.ins().jump(next.unwrap_or(resume), &[]);
                }
                StructuralOperation::SaveCursor { slot } => {
                    let count = function.ins().load(
                        types::I32,
                        flags,
                        task,
                        mem::offset_of!(NativeTaskContext, cursor_count) as i32,
                    );
                    let capacity = function.ins().load(
                        types::I32,
                        flags,
                        task,
                        mem::offset_of!(NativeTaskContext, cursor_quota_limit) as i32,
                    );
                    let available = function
                        .ins()
                        .icmp(IntCC::UnsignedLessThan, count, capacity);
                    let write = function.create_block();
                    function
                        .ins()
                        .brif(available, write, &[], capacity_error, &[]);
                    function.switch_to_block(write);
                    let head = function.ins().load(
                        types::I32,
                        flags,
                        task,
                        mem::offset_of!(NativeTaskContext, current_cursors) as i32,
                    );
                    let slot_value = function.ins().iconst(types::I32, i64::from(slot));
                    let call = function.ins().call(cursor_value, &[task, head, slot_value]);
                    let old = function.inst_results(call)[0];
                    let records = context_pointer(
                        &mut function,
                        task,
                        mem::offset_of!(NativeTaskContext, cursors),
                    );
                    let stride = function
                        .ins()
                        .imul_imm(count, mem::size_of::<LinkRecord>() as i64);
                    let stride = function.ins().uextend(pointer_type, stride);
                    let record = function.ins().iadd(records, stride);
                    for (offset, value) in [
                        (mem::offset_of!(LinkRecord, previous), head),
                        (mem::offset_of!(LinkRecord, first), slot_value),
                        (mem::offset_of!(LinkRecord, second), current),
                    ] {
                        function.ins().store(flags, value, record, offset as i32);
                    }
                    let new_head = function.ins().iadd_imm(count, 1);
                    function.ins().store(
                        flags,
                        new_head,
                        task,
                        mem::offset_of!(NativeTaskContext, cursor_count) as i32,
                    );
                    function.ins().store(
                        flags,
                        new_head,
                        task,
                        mem::offset_of!(NativeTaskContext, current_cursors) as i32,
                    );
                    let hash = function.ins().load(
                        types::I32,
                        flags,
                        task,
                        mem::offset_of!(NativeTaskContext, current_cursor_hash) as i32,
                    );
                    let old_mix = mix_hash(&mut function, slot_value, old);
                    let new_mix = mix_hash(&mut function, slot_value, current);
                    let hash = function.ins().bxor(hash, old_mix);
                    let hash = function.ins().bxor(hash, new_mix);
                    function.ins().store(
                        flags,
                        hash,
                        task,
                        mem::offset_of!(NativeTaskContext, current_cursor_hash) as i32,
                    );
                    function.ins().jump(next.unwrap_or(resume), &[]);
                }
                StructuralOperation::RestoreCursor { slot } => {
                    let head = function.ins().load(
                        types::I32,
                        flags,
                        task,
                        mem::offset_of!(NativeTaskContext, current_cursors) as i32,
                    );
                    let slot = function.ins().iconst(types::I32, i64::from(slot));
                    let call = function.ins().call(cursor_value, &[task, head, slot]);
                    let target = function.inst_results(call)[0];
                    let found = function
                        .ins()
                        .icmp_imm(IntCC::NotEqual, target, i64::from(NONE));
                    let restore = function.create_block();
                    function.ins().brif(found, restore, &[], operand_error, &[]);
                    function.switch_to_block(restore);
                    function.ins().store(
                        flags,
                        target,
                        task,
                        mem::offset_of!(NativeTaskContext, current_node) as i32,
                    );
                    function.ins().jump(next.unwrap_or(resume), &[]);
                }
                StructuralOperation::Capture { label } => {
                    let count = function.ins().load(
                        types::I32,
                        flags,
                        task,
                        mem::offset_of!(NativeTaskContext, capture_count) as i32,
                    );
                    let capacity = function.ins().load(
                        types::I32,
                        flags,
                        task,
                        mem::offset_of!(NativeTaskContext, capture_quota_limit) as i32,
                    );
                    let available = function
                        .ins()
                        .icmp(IntCC::UnsignedLessThan, count, capacity);
                    let write = function.create_block();
                    function
                        .ins()
                        .brif(available, write, &[], capacity_error, &[]);
                    function.switch_to_block(write);
                    let head = function.ins().load(
                        types::I32,
                        flags,
                        task,
                        mem::offset_of!(NativeTaskContext, current_captures) as i32,
                    );
                    let records = context_pointer(
                        &mut function,
                        task,
                        mem::offset_of!(NativeTaskContext, captures),
                    );
                    let stride = function
                        .ins()
                        .imul_imm(count, mem::size_of::<LinkRecord>() as i64);
                    let stride = function.ins().uextend(pointer_type, stride);
                    let record = function.ins().iadd(records, stride);
                    let label = function.ins().iconst(types::I32, i64::from(label));
                    for (offset, value) in [
                        (mem::offset_of!(LinkRecord, previous), head),
                        (mem::offset_of!(LinkRecord, first), label),
                        (mem::offset_of!(LinkRecord, second), current),
                    ] {
                        function.ins().store(flags, value, record, offset as i32);
                    }
                    let new_head = function.ins().iadd_imm(count, 1);
                    function.ins().store(
                        flags,
                        new_head,
                        task,
                        mem::offset_of!(NativeTaskContext, capture_count) as i32,
                    );
                    function.ins().store(
                        flags,
                        new_head,
                        status,
                        mem::offset_of!(TaskStatus, capture_count) as i32,
                    );
                    function.ins().store(
                        flags,
                        new_head,
                        task,
                        mem::offset_of!(NativeTaskContext, current_captures) as i32,
                    );
                    let hash = function.ins().load(
                        types::I32,
                        flags,
                        task,
                        mem::offset_of!(NativeTaskContext, current_capture_hash) as i32,
                    );
                    let identity = mix_hash(&mut function, label, current);
                    let hash = mix_hash(&mut function, hash, identity);
                    function.ins().store(
                        flags,
                        hash,
                        task,
                        mem::offset_of!(NativeTaskContext, current_capture_hash) as i32,
                    );
                    function.ins().jump(next.unwrap_or(resume), &[]);
                }
                StructuralOperation::RepeatBegin { id } => {
                    let count = function.ins().load(
                        types::I32,
                        flags,
                        task,
                        mem::offset_of!(NativeTaskContext, repeat_count) as i32,
                    );
                    let capacity = function.ins().load(
                        types::I32,
                        flags,
                        task,
                        mem::offset_of!(NativeTaskContext, repeat_quota_limit) as i32,
                    );
                    let available = function
                        .ins()
                        .icmp(IntCC::UnsignedLessThan, count, capacity);
                    let write = function.create_block();
                    function
                        .ins()
                        .brif(available, write, &[], capacity_error, &[]);
                    function.switch_to_block(write);
                    let head = function.ins().load(
                        types::I32,
                        flags,
                        task,
                        mem::offset_of!(NativeTaskContext, current_repeats) as i32,
                    );
                    let old_hash = function.ins().load(
                        types::I32,
                        flags,
                        task,
                        mem::offset_of!(NativeTaskContext, current_repeat_hash) as i32,
                    );
                    let records = context_pointer(
                        &mut function,
                        task,
                        mem::offset_of!(NativeTaskContext, repeats),
                    );
                    let stride = function
                        .ins()
                        .imul_imm(count, mem::size_of::<RepeatRecord>() as i64);
                    let stride = function.ins().uextend(pointer_type, stride);
                    let record = function.ins().iadd(records, stride);
                    let id = function.ins().iconst(types::I32, i64::from(id));
                    for (offset, value) in [
                        (mem::offset_of!(RepeatRecord, previous), head),
                        (mem::offset_of!(RepeatRecord, value), id),
                        (mem::offset_of!(RepeatRecord, previous_hash), old_hash),
                    ] {
                        function.ins().store(flags, value, record, offset as i32);
                    }
                    let new_head = function.ins().iadd_imm(count, 1);
                    function.ins().store(
                        flags,
                        new_head,
                        task,
                        mem::offset_of!(NativeTaskContext, repeat_count) as i32,
                    );
                    function.ins().store(
                        flags,
                        new_head,
                        task,
                        mem::offset_of!(NativeTaskContext, current_repeats) as i32,
                    );
                    let hash = mix_hash(&mut function, old_hash, id);
                    function.ins().store(
                        flags,
                        hash,
                        task,
                        mem::offset_of!(NativeTaskContext, current_repeat_hash) as i32,
                    );
                    function.ins().jump(next.unwrap_or(resume), &[]);
                }
                StructuralOperation::RepeatBump { id } | StructuralOperation::RepeatEnd { id } => {
                    let head = function.ins().load(
                        types::I32,
                        flags,
                        task,
                        mem::offset_of!(NativeTaskContext, current_repeats) as i32,
                    );
                    let present = function.ins().icmp_imm(IntCC::NotEqual, head, 0);
                    let inspect = function.create_block();
                    function
                        .ins()
                        .brif(present, inspect, &[], operand_error, &[]);
                    function.switch_to_block(inspect);
                    let records = context_pointer(
                        &mut function,
                        task,
                        mem::offset_of!(NativeTaskContext, repeats),
                    );
                    let index = function.ins().iadd_imm(head, -1);
                    let stride = function
                        .ins()
                        .imul_imm(index, mem::size_of::<RepeatRecord>() as i64);
                    let stride = function.ins().uextend(pointer_type, stride);
                    let record = function.ins().iadd(records, stride);
                    let value = function.ins().load(
                        types::I32,
                        flags,
                        record,
                        mem::offset_of!(RepeatRecord, value) as i32,
                    );
                    let valid = function.ins().icmp_imm(IntCC::Equal, value, i64::from(id));
                    let apply = function.create_block();
                    function.ins().brif(valid, apply, &[], operand_error, &[]);
                    function.switch_to_block(apply);
                    if matches!(block.operation, StructuralOperation::RepeatEnd { .. }) {
                        let previous = function.ins().load(
                            types::I32,
                            flags,
                            record,
                            mem::offset_of!(RepeatRecord, previous) as i32,
                        );
                        let previous_hash = function.ins().load(
                            types::I32,
                            flags,
                            record,
                            mem::offset_of!(RepeatRecord, previous_hash) as i32,
                        );
                        function.ins().store(
                            flags,
                            previous,
                            task,
                            mem::offset_of!(NativeTaskContext, current_repeats) as i32,
                        );
                        function.ins().store(
                            flags,
                            previous_hash,
                            task,
                            mem::offset_of!(NativeTaskContext, current_repeat_hash) as i32,
                        );
                    }
                    function.ins().jump(next.unwrap_or(resume), &[]);
                }
                StructuralOperation::Fork {
                    greedy: false,
                    target,
                } => {
                    if let Some(queue_insert) = queue_insert {
                        let target_value = function.ins().iconst(types::I32, i64::from(target));
                        let sibling = match native.structural[usize::from(target)].operation {
                            StructuralOperation::Next { .. } => 1,
                            StructuralOperation::First { .. } => 2,
                            StructuralOperation::Last { .. } => 3,
                            _ => 0,
                        };
                        let sibling = function.ins().iconst(types::I32, sibling);
                        let call = function
                            .ins()
                            .call(queue_insert, &[task, target_value, sibling]);
                        let outcome = function.inst_results(call)[0];
                        let inserted = function.ins().icmp_imm(IntCC::Equal, outcome, 1);
                        let not_inserted = function.create_block();
                        let append = function.create_block();
                        function
                            .ins()
                            .brif(inserted, append, &[], not_inserted, &[]);
                        function.switch_to_block(not_inserted);
                        let duplicate = function.ins().icmp_imm(IntCC::Equal, outcome, 0);
                        function.ins().brif(
                            duplicate,
                            next.unwrap_or(resume),
                            &[],
                            capacity_error,
                            &[],
                        );
                        function.switch_to_block(append);
                    }
                    let count = function.ins().load(
                        types::I32,
                        flags,
                        task,
                        mem::offset_of!(NativeTaskContext, pending_count) as i32,
                    );
                    let capacity = function.ins().load(
                        types::I32,
                        flags,
                        task,
                        mem::offset_of!(NativeTaskContext, pending_quota_limit) as i32,
                    );
                    let available = function
                        .ins()
                        .icmp(IntCC::UnsignedLessThan, count, capacity);
                    let write = function.create_block();
                    function
                        .ins()
                        .brif(available, write, &[], capacity_error, &[]);
                    function.switch_to_block(write);
                    let pending = context_pointer(
                        &mut function,
                        task,
                        mem::offset_of!(NativeTaskContext, pending),
                    );
                    let stride = function
                        .ins()
                        .imul_imm(count, mem::size_of::<PendingRecord>() as i64);
                    let path_offset = function.ins().uextend(pointer_type, stride);
                    let path = function.ins().iadd(pending, path_offset);
                    let kind = function.ins().iconst(types::I32, i64::from(PENDING_PATH));
                    function.ins().store(
                        flags,
                        kind,
                        path,
                        mem::offset_of!(PendingRecord, kind) as i32,
                    );
                    let target = function.ins().iconst(types::I32, i64::from(target));
                    function.ins().store(
                        flags,
                        target,
                        path,
                        pending_path_field(mem::offset_of!(PathRecord, pc)),
                    );
                    function.ins().store(
                        flags,
                        current,
                        path,
                        pending_path_field(mem::offset_of!(PathRecord, node)),
                    );
                    let start = function.ins().load(
                        types::I32,
                        flags,
                        task,
                        mem::offset_of!(NativeTaskContext, start_node) as i32,
                    );
                    function.ins().store(
                        flags,
                        start,
                        path,
                        pending_path_field(mem::offset_of!(PathRecord, start)),
                    );
                    function.ins().store(
                        flags,
                        steps,
                        path,
                        pending_path_field(mem::offset_of!(PathRecord, steps)),
                    );
                    for (context_offset, path_offset) in [
                        (
                            mem::offset_of!(NativeTaskContext, current_captures),
                            mem::offset_of!(PathRecord, captures),
                        ),
                        (
                            mem::offset_of!(NativeTaskContext, current_cursors),
                            mem::offset_of!(PathRecord, cursors),
                        ),
                        (
                            mem::offset_of!(NativeTaskContext, current_repeats),
                            mem::offset_of!(PathRecord, repeats),
                        ),
                        (
                            mem::offset_of!(NativeTaskContext, current_following),
                            mem::offset_of!(PathRecord, following),
                        ),
                        (
                            mem::offset_of!(NativeTaskContext, current_capture_hash),
                            mem::offset_of!(PathRecord, capture_hash),
                        ),
                        (
                            mem::offset_of!(NativeTaskContext, current_cursor_hash),
                            mem::offset_of!(PathRecord, cursor_hash),
                        ),
                        (
                            mem::offset_of!(NativeTaskContext, current_repeat_hash),
                            mem::offset_of!(PathRecord, repeat_hash),
                        ),
                    ] {
                        let value =
                            function
                                .ins()
                                .load(types::I32, flags, task, context_offset as i32);
                        function
                            .ins()
                            .store(flags, value, path, pending_path_field(path_offset));
                    }
                    let count = function.ins().iadd_imm(count, 1);
                    function.ins().store(
                        flags,
                        count,
                        task,
                        mem::offset_of!(NativeTaskContext, pending_count) as i32,
                    );
                    function.ins().jump(next.unwrap_or(resume), &[]);
                }
                StructuralOperation::Fork {
                    greedy: true,
                    target,
                } => {
                    let target_value = function.ins().iconst(types::I32, i64::from(target));
                    let call = function.ins().call(greedy_schedule, &[task, target_value]);
                    let outcome = function.inst_results(call)[0];
                    let failed = function.ins().icmp_imm(IntCC::Equal, outcome, -1);
                    let scheduled = function.create_block();
                    function
                        .ins()
                        .brif(failed, capacity_error, &[], scheduled, &[]);
                    function.switch_to_block(scheduled);
                    let inserted = function.ins().ushr_imm(outcome, 32);
                    let inserted = function.ins().icmp_imm(IntCC::NotEqual, inserted, 0);
                    let append = function.create_block();
                    function
                        .ins()
                        .brif(inserted, append, &[], next.unwrap_or(resume), &[]);
                    function.switch_to_block(append);
                    let count = function.ins().load(
                        types::I32,
                        MemFlags::trusted(),
                        task,
                        mem::offset_of!(NativeTaskContext, pending_count) as i32,
                    );
                    let capacity = function.ins().load(
                        types::I32,
                        MemFlags::trusted(),
                        task,
                        mem::offset_of!(NativeTaskContext, pending_quota_limit) as i32,
                    );
                    let has_capacity =
                        function
                            .ins()
                            .icmp(IntCC::UnsignedLessThan, count, capacity);
                    let write = function.create_block();
                    function
                        .ins()
                        .brif(has_capacity, write, &[], capacity_error, &[]);
                    function.switch_to_block(write);
                    let pending = context_pointer(
                        &mut function,
                        task,
                        mem::offset_of!(NativeTaskContext, pending),
                    );
                    let offset = function
                        .ins()
                        .imul_imm(count, mem::size_of::<PendingRecord>() as i64);
                    let offset = function.ins().uextend(pointer_type, offset);
                    let record = function.ins().iadd(pending, offset);
                    let kind = function.ins().iconst(types::I32, i64::from(PENDING_GREEDY));
                    function.ins().store(
                        MemFlags::trusted(),
                        kind,
                        record,
                        mem::offset_of!(PendingRecord, kind) as i32,
                    );
                    let slot = function.ins().ireduce(types::I32, outcome);
                    function.ins().store(
                        MemFlags::trusted(),
                        slot,
                        record,
                        mem::offset_of!(PendingRecord, greedy_slot) as i32,
                    );
                    let one = function.ins().iconst(types::I32, 1);
                    let next_count = function.ins().iadd(count, one);
                    function.ins().store(
                        MemFlags::trusted(),
                        next_count,
                        task,
                        mem::offset_of!(NativeTaskContext, pending_count) as i32,
                    );
                    function.ins().jump(next.unwrap_or(resume), &[]);
                }
                StructuralOperation::Goto { target } => {
                    function.ins().jump(blocks[usize::from(target)], &[]);
                }
                StructuralOperation::Emit { label, .. } => {
                    let status = context_pointer(
                        &mut function,
                        task,
                        mem::offset_of!(NativeTaskContext, status),
                    );
                    let count = function.ins().load(
                        types::I32,
                        flags,
                        status,
                        mem::offset_of!(TaskStatus, result_count) as i32,
                    );
                    let capacity = function.ins().load(
                        types::I32,
                        flags,
                        task,
                        mem::offset_of!(NativeTaskContext, result_quota_limit) as i32,
                    );
                    let available = function
                        .ins()
                        .icmp(IntCC::UnsignedLessThan, count, capacity);
                    let write = function.create_block();
                    function
                        .ins()
                        .brif(available, write, &[], capacity_error, &[]);
                    function.switch_to_block(write);
                    let results = context_pointer(
                        &mut function,
                        task,
                        mem::offset_of!(NativeTaskContext, results),
                    );
                    let stride = function
                        .ins()
                        .imul_imm(count, mem::size_of::<ResultRecord>() as i64);
                    let result_offset = function.ins().uextend(pointer_type, stride);
                    let result = function.ins().iadd(results, result_offset);
                    let capture_head = function.ins().load(
                        types::I32,
                        flags,
                        task,
                        mem::offset_of!(NativeTaskContext, current_captures) as i32,
                    );
                    let count_captures = function.create_block();
                    function.append_block_param(count_captures, types::I32);
                    function.append_block_param(count_captures, types::I32);
                    let count_capture = function.create_block();
                    function.append_block_param(count_capture, types::I32);
                    function.append_block_param(count_capture, types::I32);
                    let captures_counted = function.create_block();
                    function.append_block_param(captures_counted, types::I32);
                    let zero_count = function.ins().iconst(types::I32, 0);
                    function
                        .ins()
                        .jump(count_captures, &[capture_head.into(), zero_count.into()]);
                    function.switch_to_block(count_captures);
                    let capture_cursor = function.block_params(count_captures)[0];
                    let capture_count = function.block_params(count_captures)[1];
                    let has_capture = function.ins().icmp_imm(IntCC::NotEqual, capture_cursor, 0);
                    function.ins().brif(
                        has_capture,
                        count_capture,
                        &[capture_cursor.into(), capture_count.into()],
                        captures_counted,
                        &[capture_count.into()],
                    );
                    function.switch_to_block(count_capture);
                    let capture_cursor = function.block_params(count_capture)[0];
                    let capture_count = function.block_params(count_capture)[1];
                    let records = context_pointer(
                        &mut function,
                        task,
                        mem::offset_of!(NativeTaskContext, captures),
                    );
                    let index = function.ins().iadd_imm(capture_cursor, -1);
                    let stride = function
                        .ins()
                        .imul_imm(index, mem::size_of::<LinkRecord>() as i64);
                    let stride = function.ins().uextend(pointer_type, stride);
                    let record = function.ins().iadd(records, stride);
                    let previous = function.ins().load(
                        types::I32,
                        flags,
                        record,
                        mem::offset_of!(LinkRecord, previous) as i32,
                    );
                    let capture_count = function.ins().iadd_imm(capture_count, 1);
                    function
                        .ins()
                        .jump(count_captures, &[previous.into(), capture_count.into()]);
                    function.switch_to_block(captures_counted);
                    let capture_count = function.block_params(captures_counted)[0];
                    let ordinal = function.ins().load(
                        types::I32,
                        flags,
                        task,
                        mem::offset_of!(NativeTaskContext, task_ordinal) as i32,
                    );
                    let start = function.ins().load(
                        types::I32,
                        flags,
                        task,
                        mem::offset_of!(NativeTaskContext, start_node) as i32,
                    );
                    let zero = function.ins().iconst(types::I32, 0);
                    function.ins().store(
                        flags,
                        ordinal,
                        result,
                        mem::offset_of!(ResultRecord, task_ordinal) as i32,
                    );
                    function.ins().store(
                        flags,
                        count,
                        result,
                        mem::offset_of!(ResultRecord, local_ordinal) as i32,
                    );
                    function.ins().store(
                        flags,
                        start,
                        result,
                        mem::offset_of!(ResultRecord, start) as i32,
                    );
                    let label = function.ins().iconst(types::I32, i64::from(label));
                    function.ins().store(
                        flags,
                        label,
                        result,
                        mem::offset_of!(ResultRecord, label) as i32,
                    );
                    function.ins().store(
                        flags,
                        capture_head,
                        result,
                        mem::offset_of!(ResultRecord, capture_start) as i32,
                    );
                    function.ins().store(
                        flags,
                        capture_count,
                        result,
                        mem::offset_of!(ResultRecord, capture_count) as i32,
                    );
                    for offset in [
                        mem::offset_of!(ResultRecord, property_start),
                        mem::offset_of!(ResultRecord, property_count),
                        mem::offset_of!(ResultRecord, dynamic_start),
                        mem::offset_of!(ResultRecord, dynamic_count),
                    ] {
                        function.ins().store(flags, zero, result, offset as i32);
                    }
                    let one = function.ins().iadd_imm(count, 1);
                    function.ins().store(
                        flags,
                        one,
                        status,
                        mem::offset_of!(TaskStatus, result_count) as i32,
                    );
                    if native.key_stop {
                        // Set mode with one label: the task's key is found; its pending paths go.
                        function.ins().store(
                            flags,
                            zero,
                            task,
                            mem::offset_of!(NativeTaskContext, pending_count) as i32,
                        );
                        function.ins().jump(done, &[]);
                    } else {
                        function.ins().jump(resume, &[]);
                    }
                }
                _ => unreachable!("unsupported operations were rejected before code generation"),
            }
        }

        for (index, resume_target) in resume_targets.iter().copied().enumerate() {
            function.switch_to_block(resume_target);
            let pc = function.ins().iconst(types::I32, index as i64);
            let status = context_pointer(
                &mut function,
                task,
                mem::offset_of!(NativeTaskContext, status),
            );
            function.ins().store(
                flags,
                pc,
                status,
                mem::offset_of!(TaskStatus, program_counter) as i32,
            );
            if let Some(queue_remove) = queue_remove {
                let sibling = match native.structural[index].operation {
                    StructuralOperation::Next { .. } => 1,
                    StructuralOperation::First { .. } => 2,
                    StructuralOperation::Last { .. } => 3,
                    _ => 0,
                };
                let sibling = function.ins().iconst(types::I32, sibling);
                function.ins().call(queue_remove, &[task, pc, sibling]);
            }
            let mode = function.ins().load(
                types::I32,
                flags,
                task,
                mem::offset_of!(NativeTaskContext, current_following) as i32,
            );
            let zero = function.ins().iconst(types::I32, 0);
            function.ins().store(
                flags,
                zero,
                task,
                mem::offset_of!(NativeTaskContext, current_following) as i32,
            );
            let is_following = function.ins().icmp_imm(IntCC::NotEqual, mode, 0);
            let continue_path = function.create_block();
            let continue_following = function.create_block();
            function
                .ins()
                .brif(is_following, continue_following, &[], continue_path, &[]);
            function.switch_to_block(continue_following);
            let current = function.ins().load(
                types::I32,
                flags,
                task,
                mem::offset_of!(NativeTaskContext, current_node) as i32,
            );
            let nodes = context_pointer(
                &mut function,
                task,
                mem::offset_of!(NativeTaskContext, nodes),
            );
            let stride = function
                .ins()
                .imul_imm(current, mem::size_of::<MetalNode>() as i64);
            let offset = function.ins().uextend(pointer_type, stride);
            let node = function.ins().iadd(nodes, offset);
            let next_any = function.ins().load(
                types::I32,
                flags,
                node,
                mem::offset_of!(MetalNode, next_any) as i32,
            );
            let next_named = function.ins().load(
                types::I32,
                flags,
                node,
                mem::offset_of!(MetalNode, next_named) as i32,
            );
            let any_mode = function.ins().icmp_imm(IntCC::Equal, mode, 1);
            let later = function.ins().select(any_mode, next_any, next_named);
            let has_later = function
                .ins()
                .icmp_imm(IntCC::NotEqual, later, i64::from(NONE));
            let enqueue_later = function.create_block();
            function
                .ins()
                .brif(has_later, enqueue_later, &[], continue_path, &[]);
            function.switch_to_block(enqueue_later);
            let sibling_requirement = match native.structural[index].operation {
                StructuralOperation::Next { .. } => 1,
                StructuralOperation::First { .. } => 2,
                StructuralOperation::Last { .. } => 3,
                _ => 0,
            };
            let sibling_requirement = function.ins().iconst(types::I32, sibling_requirement);
            let kind = function
                .ins()
                .iconst(types::I32, i64::from(PENDING_FOLLOWING));
            let call = function.ins().call(
                path_enqueue,
                &[task, pc, later, mode, sibling_requirement, kind],
            );
            let outcome = function.inst_results(call)[0];
            let has_capacity = function.ins().icmp_imm(IntCC::NotEqual, outcome, 2);
            function
                .ins()
                .brif(has_capacity, continue_path, &[], capacity_error, &[]);
            function.switch_to_block(continue_path);
            function.ins().jump(blocks[index], &[]);
        }

        function.switch_to_block(resume);
        let pending_count = function.ins().load(
            types::I32,
            flags,
            task,
            mem::offset_of!(NativeTaskContext, pending_count) as i32,
        );
        let has_pending = function.ins().icmp_imm(IntCC::NotEqual, pending_count, 0);
        let pop = function.create_block();
        function.ins().brif(has_pending, pop, &[], done, &[]);
        function.switch_to_block(pop);
        let pending_count = function.ins().iadd_imm(pending_count, -1);
        function.ins().store(
            flags,
            pending_count,
            task,
            mem::offset_of!(NativeTaskContext, pending_count) as i32,
        );
        let pending = context_pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, pending),
        );
        let stride = function
            .ins()
            .imul_imm(pending_count, mem::size_of::<PendingRecord>() as i64);
        let path_offset = function.ins().uextend(pointer_type, stride);
        let path = function.ins().iadd(pending, path_offset);
        let kind = function.ins().load(
            types::I32,
            flags,
            path,
            mem::offset_of!(PendingRecord, kind) as i32,
        );
        let is_greedy = function
            .ins()
            .icmp_imm(IntCC::Equal, kind, i64::from(PENDING_GREEDY));
        let normal_pop = function.create_block();
        let greedy_pop = function.create_block();
        let normal_dispatch = function.create_block();
        function.append_block_param(normal_dispatch, types::I32);
        let greedy_dispatch = function.create_block();
        function.append_block_param(greedy_dispatch, types::I32);
        function
            .ins()
            .brif(is_greedy, greedy_pop, &[], normal_pop, &[]);

        function.switch_to_block(normal_pop);
        let resumed_pc = function.ins().load(
            types::I32,
            flags,
            path,
            pending_path_field(mem::offset_of!(PathRecord, pc)),
        );
        let resumed_node = function.ins().load(
            types::I32,
            flags,
            path,
            pending_path_field(mem::offset_of!(PathRecord, node)),
        );
        function.ins().store(
            flags,
            resumed_node,
            task,
            mem::offset_of!(NativeTaskContext, current_node) as i32,
        );
        let resumed_start = function.ins().load(
            types::I32,
            flags,
            path,
            pending_path_field(mem::offset_of!(PathRecord, start)),
        );
        function.ins().store(
            flags,
            resumed_start,
            task,
            mem::offset_of!(NativeTaskContext, start_node) as i32,
        );
        for (context_offset, path_offset) in [
            (
                mem::offset_of!(NativeTaskContext, current_captures),
                mem::offset_of!(PathRecord, captures),
            ),
            (
                mem::offset_of!(NativeTaskContext, current_cursors),
                mem::offset_of!(PathRecord, cursors),
            ),
            (
                mem::offset_of!(NativeTaskContext, current_repeats),
                mem::offset_of!(PathRecord, repeats),
            ),
            (
                mem::offset_of!(NativeTaskContext, current_following),
                mem::offset_of!(PathRecord, following),
            ),
            (
                mem::offset_of!(NativeTaskContext, current_capture_hash),
                mem::offset_of!(PathRecord, capture_hash),
            ),
            (
                mem::offset_of!(NativeTaskContext, current_cursor_hash),
                mem::offset_of!(PathRecord, cursor_hash),
            ),
            (
                mem::offset_of!(NativeTaskContext, current_repeat_hash),
                mem::offset_of!(PathRecord, repeat_hash),
            ),
        ] {
            let value =
                function
                    .ins()
                    .load(types::I32, flags, path, pending_path_field(path_offset));
            function
                .ins()
                .store(flags, value, task, context_offset as i32);
        }
        let resumed_steps = function.ins().load(
            types::I32,
            flags,
            path,
            pending_path_field(mem::offset_of!(PathRecord, steps)),
        );
        function.ins().store(
            flags,
            resumed_steps,
            task,
            mem::offset_of!(NativeTaskContext, current_steps) as i32,
        );
        function.ins().jump(normal_dispatch, &[resumed_pc.into()]);

        function.switch_to_block(greedy_pop);
        let slot = function.ins().load(
            types::I32,
            flags,
            path,
            mem::offset_of!(PendingRecord, greedy_slot) as i32,
        );
        let call = function.ins().call(greedy_take, &[task, slot]);
        let outcome = function.inst_results(call)[0];
        let stale = function.ins().icmp_imm(IntCC::Equal, outcome, -1);
        let greedy_live = function.create_block();
        function.ins().brif(stale, resume, &[], greedy_live, &[]);
        function.switch_to_block(greedy_live);
        let resumed_pc = function.ins().ireduce(types::I32, outcome);
        function.ins().jump(greedy_dispatch, &[resumed_pc.into()]);

        function.switch_to_block(normal_dispatch);
        let resumed_pc = function.block_params(normal_dispatch)[0];
        let mut dispatch = Switch::new();
        for (position, target) in resume_targets.iter().copied().enumerate() {
            dispatch.set_entry(position as u128, target);
        }
        dispatch.emit(&mut function, resumed_pc, done);

        function.switch_to_block(greedy_dispatch);
        let resumed_pc = function.block_params(greedy_dispatch)[0];
        let mut dispatch = Switch::new();
        for (position, target) in blocks.iter().copied().enumerate() {
            dispatch.set_entry(position as u128, target);
        }
        dispatch.emit(&mut function, resumed_pc, done);

        function.switch_to_block(work_error);
        emit_task_error(&mut function, task, 4);
        function.ins().jump(done, &[]);

        function.switch_to_block(capacity_error);
        emit_task_error(&mut function, task, 2);
        function.ins().jump(done, &[]);

        function.switch_to_block(operand_error);
        emit_task_error(&mut function, task, 7);
        function.ins().jump(done, &[]);

        function.switch_to_block(invalid_input);
        emit_task_error(&mut function, task, 1);
        function.ins().jump(done, &[]);

        function.switch_to_block(invalid_instruction);
        emit_task_error(&mut function, task, 6);
        function.ins().jump(done, &[]);

        function.switch_to_block(done);
        let status = context_pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, status),
        );
        let error = function.ins().load(
            types::I32,
            flags,
            status,
            mem::offset_of!(TaskStatus, error) as i32,
        );
        let successful = function.ins().icmp_imm(IntCC::Equal, error, 0);
        let apply_output = function.create_block();
        let return_task = function.create_block();
        function
            .ins()
            .brif(successful, apply_output, &[], return_task, &[]);
        function.switch_to_block(apply_output);
        let reduction_call = function.ins().call(reduction_function, &[task]);
        let reduction_outcome = function.inst_results(reduction_call)[0];
        let reduction_ok = function.ins().icmp_imm(IntCC::Equal, reduction_outcome, 0);
        let run_output = function.create_block();
        let reduction_error = function.create_block();
        function
            .ins()
            .brif(reduction_ok, run_output, &[], reduction_error, &[]);
        function.switch_to_block(reduction_error);
        let reduction_work = function.ins().icmp_imm(IntCC::Equal, reduction_outcome, 3);
        let reduction_not_work = function.create_block();
        function
            .ins()
            .brif(reduction_work, work_error, &[], reduction_not_work, &[]);
        function.switch_to_block(reduction_not_work);
        let reduction_capacity = function.ins().icmp_imm(IntCC::Equal, reduction_outcome, 4);
        function.ins().brif(
            reduction_capacity,
            capacity_error,
            &[],
            invalid_instruction,
            &[],
        );
        function.switch_to_block(run_output);
        let call = function.ins().call(output_function, &[task]);
        let outcome = function.inst_results(call)[0];
        let output_ok = function.ins().icmp_imm(IntCC::Equal, outcome, 0);
        let output_error = function.create_block();
        function
            .ins()
            .brif(output_ok, return_task, &[], output_error, &[]);
        function.switch_to_block(output_error);
        let output_invalid = function.ins().icmp_imm(IntCC::Equal, outcome, 2);
        let output_not_invalid = function.create_block();
        function.ins().brif(
            output_invalid,
            invalid_instruction,
            &[],
            output_not_invalid,
            &[],
        );
        function.switch_to_block(output_not_invalid);
        let output_work = function.ins().icmp_imm(IntCC::Equal, outcome, 3);
        let output_not_work = function.create_block();
        function
            .ins()
            .brif(output_work, work_error, &[], output_not_work, &[]);
        function.switch_to_block(output_not_work);
        let output_capacity = function.ins().icmp_imm(IntCC::Equal, outcome, 4);
        function.ins().brif(
            output_capacity,
            capacity_error,
            &[],
            invalid_instruction,
            &[],
        );
        function.switch_to_block(return_task);
        function.ins().return_(&[]);
        function.seal_all_blocks();
        function.finalize();
    }
    super::native_cpu_codegen::define_function(&mut module, function_id, &mut context)
        .map_err(|_| Error::BackendFailure)?;
    module.clear_context(&mut context);
    module
        .finalize_definitions()
        .map_err(|_| Error::BackendFailure)?;
    let code = module.get_finalized_function(function_id);
    let entry = unsafe { mem::transmute::<*const u8, NativeTaskEntry>(code) };
    #[cfg(any(test, feature = "test-hooks"))]
    let cursor_equal = unsafe {
        mem::transmute::<*const u8, unsafe extern "C" fn(*mut NativeTaskContext, u32, u32) -> u32>(
            module.get_finalized_function(scheduler.cursor_equal),
        )
    };
    #[cfg(any(test, feature = "test-hooks"))]
    let repeat_equal = unsafe {
        mem::transmute::<*const u8, unsafe extern "C" fn(*mut NativeTaskContext, u32, u32) -> u32>(
            module.get_finalized_function(scheduler.repeat_equal),
        )
    };
    #[cfg(any(test, feature = "test-hooks"))]
    let capture_contains = unsafe {
        mem::transmute::<*const u8, unsafe extern "C" fn(*mut NativeTaskContext, u32, u32) -> u32>(
            module.get_finalized_function(scheduler.capture_contains),
        )
    };
    #[cfg(any(test, feature = "test-hooks"))]
    let query_admit = unsafe {
        mem::transmute::<*const u8, unsafe extern "C" fn(*mut NativeTaskContext, u32, u32) -> u32>(
            module.get_finalized_function(scheduler.query_admit),
        )
    };
    #[cfg(any(test, feature = "test-hooks"))]
    let queue_insert = unsafe {
        mem::transmute::<*const u8, unsafe extern "C" fn(*mut NativeTaskContext, u32, u32) -> u32>(
            module.get_finalized_function(scheduler.queue_insert),
        )
    };
    #[cfg(any(test, feature = "test-hooks"))]
    let greedy_schedule = unsafe {
        mem::transmute::<*const u8, unsafe extern "C" fn(*mut NativeTaskContext, u32) -> u64>(
            module.get_finalized_function(scheduler.greedy_schedule),
        )
    };
    #[cfg(any(test, feature = "test-hooks"))]
    let greedy_take = unsafe {
        mem::transmute::<*const u8, unsafe extern "C" fn(*mut NativeTaskContext, u32) -> u64>(
            module.get_finalized_function(scheduler.greedy_take),
        )
    };
    #[cfg(any(test, feature = "test-hooks"))]
    let exact_state_equal = unsafe {
        mem::transmute::<
            *const u8,
            unsafe extern "C" fn(*mut NativeTaskContext, u32, u32, u32, u32, u32, u32) -> u32,
        >(module.get_finalized_function(scheduler.exact_state_equal))
    };
    Ok(CompiledStraightLine {
        _module: module,
        value_tables,
        entry,
        #[cfg(any(test, feature = "test-hooks"))]
        cursor_equal,
        #[cfg(any(test, feature = "test-hooks"))]
        repeat_equal,
        #[cfg(any(test, feature = "test-hooks"))]
        capture_contains,
        #[cfg(any(test, feature = "test-hooks"))]
        query_admit,
        #[cfg(any(test, feature = "test-hooks"))]
        exact_state_equal,
        #[cfg(any(test, feature = "test-hooks"))]
        queue_insert,
        #[cfg(any(test, feature = "test-hooks"))]
        greedy_schedule,
        #[cfg(any(test, feature = "test-hooks"))]
        greedy_take,
    })
}

// N's (5bf45ff0), which bb4c9605's merge dropped where G's version of this file replaced the
// mechanism; kept as copied (open item O21).
#[allow(dead_code)]
pub(super) fn warm_jit(program: &Program) -> Result<(), Error> {
    compile_cached(program).map(|_| ())
}

fn cache_key(program: &Program) -> String {
    format!(
        "cpu-jit:{FORMAT_VERSION}:{}:{}",
        std::env::consts::ARCH,
        program.plain_digest()
    )
}

/// One key for a batch's programs (`compilation::request`).
pub(super) fn batch_key(programs: &[Program]) -> String {
    programs.iter().map(|program| program.plain_digest()).collect::<Vec<_>>().join(",")
}

/// Whether every one of `programs` is compiled (`compile_cached`), without compiling any.
pub(super) fn compiled(programs: &[Program]) -> Result<bool, Error> {
    let cache = CPU_JIT_CACHE.lock().map_err(|_| Error::BackendFailure)?;
    Ok(programs.iter().all(|program| cache.contains_key(&cache_key(program))))
}

pub(super) fn compile_cached(program: &Program) -> Result<Arc<CompiledStraightLine>, Error> {
    crate::code_rule_dsl::prof::count("jit.programs", 1);
    let key = crate::code_rule_dsl::prof::time("jit.key", || cache_key(program));
    if let Some(compiled) = CPU_JIT_CACHE
        .lock()
        .map_err(|_| Error::BackendFailure)?
        .get(&key)
        .cloned()
    {
        return Ok(compiled);
    }
    crate::code_rule_dsl::prof::count("jit.misses", 1);
    let native = crate::code_rule_dsl::prof::time("jit.lower", || super::native_ir::lower(program)).map_err(|_| Error::BackendFailure)?;
    let compiled = Arc::new(crate::code_rule_dsl::prof::time("jit.straight_line", || compile_straight_line(&native))?);
    let mut cache = CPU_JIT_CACHE.lock().map_err(|_| Error::BackendFailure)?;
    if let Some(existing) = cache.get(&key) {
        return Ok(Arc::clone(existing));
    }
    if cache.len() == CPU_JIT_CACHE_LIMIT
        && let Some(oldest) = cache.keys().next().cloned()
    {
        cache.remove(&oldest);
    }
    cache.insert(key, Arc::clone(&compiled));
    Ok(compiled)
}

pub fn execute_batch(
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
) -> Result<Vec<Vec<Vec<Output>>>, Error> {
    execute_batch_outcomes(programs, files, anchors)?.into_result()
}

pub(super) fn execute_batch_outcomes(
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
) -> Result<BatchOutcome, Error> {
    execute_batch_packed(programs, files, anchors)?.into_outcome()
}

pub(super) fn execute_batch_packed(
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
) -> Result<PackedExecution, Error> {
    execute_batch_packed_with_result_limits(programs, files, anchors, None)
}

pub(super) fn execute_batch_packed_with_result_limits(
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
    result_limits: Option<&[u32]>,
) -> Result<PackedExecution, Error> {
    execute_batch_packed_with_work_limit(
        programs,
        files,
        anchors,
        result_limits,
        super::MAX_STEPS_PER_PATH as u32,
    )
}

#[cfg(all(any(test, feature = "test-hooks"), target_os = "macos"))]
pub fn execute_terminal_reference_with_work_limit(
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
    result_limits: Option<&[u32]>,
    work_limit: u32,
) -> Result<PackedExecution, Error> {
    if work_limit as usize > super::MAX_STEPS_PER_PATH
        || programs
            .iter()
            .any(|p| super::terminal_repeat_proof::prove(p).is_none())
    {
        return Err(Error::InvalidAnchor);
    }
    execute_batch_packed_with_work_limit(programs, files, anchors, result_limits, work_limit)
}

pub fn execute_batch_packed_with_work_limit(
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
    result_limits: Option<&[u32]>,
    work_limit: u32,
) -> Result<PackedExecution, Error> {
    let stage = crate::code_rule_dsl::prof::Stage::start("jit.batch");
    let mut batch = PackedBatch::new(programs, files, anchors)?;
    drop(stage);
    crate::code_rule_dsl::prof::count("jit.tasks", batch.tasks.len());
    if batch.tasks.is_empty() {
        return Ok(PackedExecution::new(batch, PackedResultPage::empty()));
    }
    let totals = crate::code_rule_dsl::prof::time("jit.assign", || batch.assign_workspace_with_result_limits(programs, result_limits))?;
    let setup = crate::code_rule_dsl::prof::Stage::start("jit.setup");
    if std::env::var_os("TEMPO_PACKED_BATCH_PROFILE").is_some() {
        eprintln!(
            "TEMPO_PACKED_BATCH_PROFILE backend=cpu programs={} tasks={} workspaceBytes={}",
            programs.len(),
            batch.tasks.len(),
            batch.workspace_allocation_bytes(totals).unwrap_or(u64::MAX),
        );
    }
    // A cold process compiles each program once (06 S7: 71 programs, 0.3 s); they compile in
    // parallel.
    let compiled = crate::code_rule_dsl::prof::time("jit.compile", || programs
        .par_iter()
        .map(crate::code_rule_dsl::logic::profile::carried("native compile (pool)", compile_cached))
        .collect::<Result<Vec<_>, Error>>())?;
    let allocation_started = std::time::Instant::now();
    let regions = crate::code_rule_dsl::prof::Stage::start("jit.regions");
    let mut pending = DemandZeroRegion::<PendingRecord>::new(totals.pending as usize)?;
    let mut captures = DemandZeroRegion::<LinkRecord>::new(totals.captures as usize)?;
    let mut cursors = DemandZeroRegion::<LinkRecord>::new(totals.cursors as usize)?;
    let mut repeats = DemandZeroRegion::<RepeatRecord>::new(totals.repeats as usize)?;
    let mut query_buckets =
        DemandZeroRegion::<QueryBucketRecord>::new(totals.query_states as usize)?;
    let mut query_members =
        DemandZeroRegion::<QueryMemberRecord>::new(totals.query_members as usize)?;
    let mut queued_states =
        DemandZeroRegion::<QueuedStateRecord>::new(totals.queued_states as usize)?;
    let mut greedy_entries =
        DemandZeroRegion::<GreedyEntryRecord>::new(totals.greedy_entries as usize)?;
    let mut value_work = DemandZeroRegion::<u32>::new(totals.value_states as usize)?;
    let result_records = totals
        .candidates
        .checked_add(totals.results)
        .ok_or(Error::BackendCapacity)?;
    let mut results = DemandZeroRegion::<ResultRecord>::new(result_records as usize)?;
    let mut statuses = DemandZeroRegion::<TaskStatus>::new(batch.tasks.len())?;
    let mut final_captures =
        DemandZeroRegion::<FinalCaptureRecord>::new(totals.output_captures as usize)?;
    let mut properties = DemandZeroRegion::<PropertyRecord>::new(totals.properties as usize)?;
    let mut output_bytes = DemandZeroRegion::<u8>::new(batch.header.workspace_byte_count as usize)?;
    let mut evaluation_starts = vec![[0_u32; ARENA_KIND_COUNT]; batch.evaluations.len()];
    let mut arena_cursors = [0_u32; ARENA_KIND_COUNT];
    for (evaluation_index, evaluation) in batch.evaluations.iter().enumerate() {
        evaluation_starts[evaluation_index] = arena_cursors;
        for (cursor, quota) in arena_cursors.iter_mut().zip(evaluation.quotas) {
            *cursor = cursor
                .checked_add(quota.limit)
                .ok_or(Error::BackendCapacity)?;
        }
    }
    drop(regions);
    let _contexts = crate::code_rule_dsl::prof::Stage::start("jit.contexts");
    let mut contexts = Vec::with_capacity(batch.tasks.len());

    for (task_index, task) in batch.tasks.iter().enumerate() {
        let program = &batch.programs[task.program as usize];
        let arena_starts = evaluation_starts[task.evaluation as usize];
        let file = &batch.files[task.file as usize];
        let next_value_start = batch
            .programs
            .get(task.program as usize + 1)
            .map_or(batch.values.len() as u32, |next| next.value_start);
        let value_count = next_value_start
            .checked_sub(program.value_start)
            .ok_or(Error::BackendFailure)?;
        let mut context: NativeTaskContext = unsafe { mem::zeroed() };
        context.nodes = batch.nodes.as_ptr();
        context.node_count = batch.nodes.len() as u32;
        context.children = batch.children.as_ptr();
        context.child_count = batch.children.len() as u32;
        context.supertypes = batch.supertypes.as_ptr();
        context.supertype_count = batch.supertypes.len() as u32;
        context.source = batch.source.as_ptr();
        context.source_length = batch.source.len() as u32;
        context.values = unsafe { batch.values.as_ptr().add(program.value_start as usize) };
        context.value_count = value_count;
        context.nfa_states = batch.nfa_states.as_ptr();
        context.nfa_state_count = batch.nfa_states.len() as u32;
        context.nfa_data = batch.nfa_data.as_ptr();
        context.nfa_data_count = batch.nfa_data.len() as u32;
        context.nfa_ranges = batch.nfa_ranges.as_ptr();
        context.nfa_range_count = batch.nfa_ranges.len() as u32;
        context.lua_instructions = batch.lua_instructions.as_ptr();
        context.lua_instruction_count = batch.lua_instructions.len() as u32;
        context.lua_members = batch.lua_members.as_ptr();
        context.lua_member_count = batch.lua_members.len() as u32;
        context.value_work = unsafe {
            value_work
                .as_mut_ptr()
                .add(arena_starts[ArenaKind::ValueState as usize] as usize)
        };
        context.value_capacity = batch.evaluation_limit(task, ArenaKind::ValueState);
        context.current_node = task.root;
        context.start_node = task.root;
        context.task_ordinal = task.batch_ordinal;
        context.pending = unsafe {
            pending
                .as_mut_ptr()
                .add(arena_starts[ArenaKind::Pending as usize] as usize)
        };
        context.pending_quota_limit = batch.evaluation_limit(task, ArenaKind::Pending);
        context.task_work_limit = super::MAX_WORK_PER_TASK as u32;
        context.path_work_limit = work_limit;
        context.captures = unsafe {
            captures
                .as_mut_ptr()
                .add(arena_starts[ArenaKind::Capture as usize] as usize)
        };
        context.capture_quota_limit = batch.evaluation_limit(task, ArenaKind::Capture);
        context.cursors = unsafe {
            cursors
                .as_mut_ptr()
                .add(arena_starts[ArenaKind::Cursor as usize] as usize)
        };
        context.cursor_quota_limit = batch.evaluation_limit(task, ArenaKind::Cursor);
        context.repeats = unsafe {
            repeats
                .as_mut_ptr()
                .add(arena_starts[ArenaKind::Repeat as usize] as usize)
        };
        context.repeat_quota_limit = batch.evaluation_limit(task, ArenaKind::Repeat);
        context.query_buckets = unsafe {
            query_buckets
                .as_mut_ptr()
                .add(arena_starts[ArenaKind::QueryBucket as usize] as usize)
        };
        context.query_bucket_capacity = batch.evaluation_limit(task, ArenaKind::QueryBucket);
        context.query_members = unsafe {
            query_members
                .as_mut_ptr()
                .add(arena_starts[ArenaKind::QueryMember as usize] as usize)
        };
        context.query_member_quota_limit = batch.evaluation_limit(task, ArenaKind::QueryMember);
        context.queued_states = unsafe {
            queued_states
                .as_mut_ptr()
                .add(arena_starts[ArenaKind::QueuedState as usize] as usize)
        };
        context.queued_state_quota_limit = batch.evaluation_limit(task, ArenaKind::QueuedState);
        context.greedy_entries = unsafe {
            greedy_entries
                .as_mut_ptr()
                .add(arena_starts[ArenaKind::GreedyEntry as usize] as usize)
        };
        context.greedy_entry_quota_limit = batch.evaluation_limit(task, ArenaKind::GreedyEntry);
        context.maximum_cursor_slot = program.maximum_cursor_slot;
        context.results = unsafe {
            results
                .as_mut_ptr()
                .add(totals.candidates as usize + arena_starts[ArenaKind::Result as usize] as usize)
        };
        context.result_quota_limit = batch.evaluation_limit(task, ArenaKind::Result);
        context.line_starts = unsafe { batch.line_starts.as_ptr().add(file.line_start as usize) };
        context.line_count = file.line_count;
        context.final_captures = unsafe {
            final_captures
                .as_mut_ptr()
                .add(arena_starts[ArenaKind::FinalCapture as usize] as usize)
        };
        context.final_capture_capacity = batch.evaluation_limit(task, ArenaKind::FinalCapture);
        context.properties = unsafe {
            properties
                .as_mut_ptr()
                .add(arena_starts[ArenaKind::Property as usize] as usize)
        };
        context.property_quota_limit = batch.evaluation_limit(task, ArenaKind::Property);
        context.output_bytes = unsafe {
            output_bytes.as_mut_ptr().add(
                (batch.arena_start(ArenaKind::OutputByte)
                    + arena_starts[ArenaKind::OutputByte as usize]) as usize,
            )
        };
        context.output_byte_quota_limit = batch.evaluation_limit(task, ArenaKind::OutputByte);
        context.status = &mut statuses[task_index];
        context.derived = batch.derived.as_ptr();
        context.derived_classes = batch.derived_classes;
        context.derived_words = batch.derived_words;
        contexts.push((task.program as usize, context));
    }
    drop(_contexts);
    let allocation_elapsed = allocation_started.elapsed();
    let entries = compiled
        .iter()
        .map(|compiled| compiled.entry)
        .collect::<Vec<_>>();
    struct TaskPackedOutput {
        results: Vec<ResultRecord>,
        captures: Vec<FinalCaptureRecord>,
        properties: Vec<PropertyRecord>,
        bytes: Vec<u8>,
    }
    let execution_started = std::time::Instant::now();
    drop(setup);
    let run = crate::code_rule_dsl::prof::Stage::start("jit.run");
    // An evaluation's tasks share its arenas, so they run in order; its tasks are contiguous
    // (`plan_shared_workspace`), so the evaluations run in parallel, each on one worker. Locking
    // each task's evaluation instead parked the pool's workers on a few hot evaluations' locks
    // (06 S7: a range's run ~4× its longest evaluation).
    let mut spans = Vec::with_capacity(batch.evaluations.len());
    {
        let mut rest = &mut contexts[..];
        let mut offset = 0_usize;
        for evaluation in &batch.evaluations {
            let (start, count) = (evaluation.task_start as usize, evaluation.task_count as usize);
            if start != offset || count > rest.len() {
                return Err(Error::BackendFailure);
            }
            let (head, tail) = std::mem::take(&mut rest).split_at_mut(count);
            spans.push(head);
            rest = tail;
            offset += count;
        }
        if !rest.is_empty() {
            return Err(Error::BackendFailure);
        }
    }
    let task_outputs = (|| {
        spans
            .into_par_iter()
            .map(|span| span.iter_mut().map(|(program, context)| {
                // The queue uses tombstones within a task. Keeping a full
                // table of tombstones across roots turns each new insert into
                // a capacity-length probe. These records reference only the
                // current task's histories and cannot survive its execution.
                unsafe {
                    if context.queued_state_quota_limit != 0 {
                        std::ptr::write_bytes(
                            context.queued_states,
                            0,
                            context.queued_state_quota_limit as usize,
                        );
                    }
                    if context.greedy_entry_quota_limit != 0 {
                        std::ptr::write_bytes(
                            context.greedy_entries,
                            0,
                            context.greedy_entry_quota_limit as usize,
                        );
                    }
                }
                unsafe { entries[*program](context) };
                let status = unsafe { *context.status };
                if status.error != 0 {
                    return Ok(TaskPackedOutput {
                        results: Vec::new(),
                        captures: Vec::new(),
                        properties: Vec::new(),
                        bytes: Vec::new(),
                    });
                }
                Ok(TaskPackedOutput {
                    results: unsafe {
                        std::slice::from_raw_parts(context.results, status.result_count as usize)
                            .to_vec()
                    },
                    captures: unsafe {
                        std::slice::from_raw_parts(
                            context.final_captures,
                            status.final_capture_count as usize,
                        )
                        .to_vec()
                    },
                    properties: unsafe {
                        std::slice::from_raw_parts(
                            context.properties,
                            status.property_count as usize,
                        )
                        .to_vec()
                    },
                    bytes: unsafe {
                        std::slice::from_raw_parts(
                            context.output_bytes,
                            status.output_byte_count as usize,
                        )
                        .to_vec()
                    },
                })
            }).collect::<Result<Vec<_>, Error>>())
            .collect::<Result<Vec<Vec<_>>, Error>>()
    })()?
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    drop(run);
    let _compact = crate::code_rule_dsl::prof::Stage::start("jit.compact");
    let mut compact_results = Vec::new();
    let mut compact_captures = Vec::new();
    let mut compact_properties = Vec::new();
    let mut compact_bytes = Vec::new();
    for output in task_outputs {
        let capture_base =
            u32::try_from(compact_captures.len()).map_err(|_| Error::BackendCapacity)?;
        let property_base =
            u32::try_from(compact_properties.len()).map_err(|_| Error::BackendCapacity)?;
        let byte_base = u32::try_from(compact_bytes.len())
            .map_err(|_| Error::BackendCapacity)?
            .checked_add(batch.arena_start(ArenaKind::OutputByte))
            .ok_or(Error::BackendCapacity)?;
        let result_base = batch
            .arena_start(ArenaKind::Result)
            .checked_add(u32::try_from(compact_results.len()).map_err(|_| Error::BackendCapacity)?)
            .ok_or(Error::BackendCapacity)?;
        let result_owners = output
            .results
            .iter()
            .enumerate()
            .map(|(index, result)| {
                Ok((
                    result.local_ordinal,
                    result_base
                        .checked_add(u32::try_from(index).map_err(|_| Error::BackendCapacity)?)
                        .ok_or(Error::BackendCapacity)?,
                ))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        for mut record in output.results {
            record.capture_start = record
                .capture_start
                .checked_add(capture_base)
                .ok_or(Error::BackendCapacity)?;
            record.property_start = record
                .property_start
                .checked_add(property_base)
                .ok_or(Error::BackendCapacity)?;
            record.dynamic_start = record
                .dynamic_start
                .checked_add(property_base)
                .ok_or(Error::BackendCapacity)?;
            compact_results.push(record);
        }
        for mut capture in output.captures {
            capture.property_start = capture
                .property_start
                .checked_add(property_base)
                .ok_or(Error::BackendCapacity)?;
            if capture.text_start != NONE {
                capture.text_start = capture
                    .text_start
                    .checked_add(byte_base)
                    .ok_or(Error::BackendCapacity)?;
            }
            compact_captures.push(capture);
        }
        for mut property in output.properties {
            if property.kind == 0 || property.kind == 2 {
                property.owner = result_owners
                    .iter()
                    .find_map(|(local, global)| (*local == property.owner).then_some(*global))
                    .ok_or(Error::BackendFailure)?;
            }
            if property.kind == 1 {
                property.owner = property
                    .owner
                    .checked_add(capture_base)
                    .ok_or(Error::BackendCapacity)?;
            } else if property.kind == 2 {
                property.key_start = property
                    .key_start
                    .checked_add(byte_base)
                    .ok_or(Error::BackendCapacity)?;
                if property.value == 1 {
                    property.value_start = property
                        .value_start
                        .checked_add(byte_base)
                        .ok_or(Error::BackendCapacity)?;
                }
            }
            compact_properties.push(property);
        }
        compact_bytes.extend_from_slice(&output.bytes);
    }
    let execution_elapsed = execution_started.elapsed();
    if std::env::var_os("TEMPO_PACKED_BATCH_PROFILE").is_some() {
        eprintln!(
            "TEMPO_PACKED_BATCH_PHASES allocationNanoseconds={} executionNanoseconds={}",
            allocation_elapsed.as_nanos(),
            execution_elapsed.as_nanos(),
        );
    }

    let result_start = batch.arena_start(ArenaKind::Result) as usize;
    results[result_start..result_start + compact_results.len()].copy_from_slice(&compact_results);
    final_captures[..compact_captures.len()].copy_from_slice(&compact_captures);
    properties[..compact_properties.len()].copy_from_slice(&compact_properties);
    let output_start = batch.arena_start(ArenaKind::OutputByte) as usize;
    output_bytes[output_start..output_start + compact_bytes.len()].copy_from_slice(&compact_bytes);
    batch.global_arenas.arenas[ArenaKind::Result as usize].cursor =
        u32::try_from(compact_results.len()).map_err(|_| Error::BackendCapacity)?;
    batch.global_arenas.arenas[ArenaKind::FinalCapture as usize].cursor =
        u32::try_from(compact_captures.len()).map_err(|_| Error::BackendCapacity)?;
    batch.global_arenas.arenas[ArenaKind::Property as usize].cursor =
        u32::try_from(compact_properties.len()).map_err(|_| Error::BackendCapacity)?;
    batch.global_arenas.arenas[ArenaKind::OutputByte as usize].cursor =
        u32::try_from(compact_bytes.len()).map_err(|_| Error::BackendCapacity)?;
    let page = PackedResultPage::compact(
        &batch,
        &statuses,
        &results,
        &final_captures,
        &properties,
        &output_bytes,
    )?;
    Ok(PackedExecution::new(batch, page))
}

// tests: a test the engine hosts (it reads above this crate; tools/cratify/test-hosts.tsv)
