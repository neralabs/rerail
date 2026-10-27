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
use crate::WORK_POOL;
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

struct DemandZeroRegion<T> {
    pointer: NonNull<T>,
    count: usize,
    mapped_bytes: usize,
    marker: PhantomData<T>,
}

impl<T> DemandZeroRegion<T> {
    fn new(count: usize) -> Result<Self, Error> {
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

#[repr(C)]
pub(super) struct NativeTaskContext {
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
}

// Each packed task owns disjoint mutable ranges. Shared pointers are read-only.
unsafe impl Send for NativeTaskContext {}

pub(super) type NativeTaskEntry = unsafe extern "C" fn(*mut NativeTaskContext);

pub(super) type TaskEntry = unsafe extern "C" fn(*mut TaskStatus);

pub(super) struct CompiledTaskShell {
    _module: JITModule,
    entry: TaskEntry,
}

impl CompiledTaskShell {
    pub(super) fn execute(&self, status: &mut TaskStatus) {
        unsafe { (self.entry)(status) };
    }
}

pub(super) fn compile_task_shell(_native: &NativeModule) -> Result<CompiledTaskShell, Error> {
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

pub(super) struct CompiledStraightLine {
    _module: JITModule,
    value_tables: PackedValueTables,
    entry: NativeTaskEntry,
    #[cfg(test)]
    cursor_equal: unsafe extern "C" fn(*mut NativeTaskContext, u32, u32) -> u32,
    #[cfg(test)]
    repeat_equal: unsafe extern "C" fn(*mut NativeTaskContext, u32, u32) -> u32,
    #[cfg(test)]
    capture_contains: unsafe extern "C" fn(*mut NativeTaskContext, u32, u32) -> u32,
    #[cfg(test)]
    query_admit: unsafe extern "C" fn(*mut NativeTaskContext, u32, u32) -> u32,
    #[cfg(test)]
    exact_state_equal:
        unsafe extern "C" fn(*mut NativeTaskContext, u32, u32, u32, u32, u32, u32) -> u32,
    #[cfg(test)]
    queue_insert: unsafe extern "C" fn(*mut NativeTaskContext, u32, u32) -> u32,
    #[cfg(test)]
    greedy_schedule: unsafe extern "C" fn(*mut NativeTaskContext, u32) -> u64,
    #[cfg(test)]
    greedy_take: unsafe extern "C" fn(*mut NativeTaskContext, u32) -> u64,
}

// Finalized JIT modules and packed constants are immutable during execution.
unsafe impl Send for CompiledStraightLine {}
unsafe impl Sync for CompiledStraightLine {}

const CPU_JIT_CACHE_LIMIT: usize = 512;
static CPU_JIT_CACHE: LazyLock<Mutex<BTreeMap<String, Arc<CompiledStraightLine>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

impl CompiledStraightLine {
    pub(super) fn execute(&self, context: &mut NativeTaskContext) {
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

pub(super) fn compile_straight_line(native: &NativeModule) -> Result<CompiledStraightLine, Error> {
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
        #[cfg(test)]
        eprintln!("native scheduler definition failed: {error:?}");
        #[cfg(not(test))]
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
        #[cfg(test)]
        eprintln!("native value definition failed: {error:?}");
        #[cfg(not(test))]
        let _ = error;
        Error::BackendFailure
    })?;
    let capture_functions =
        native_cpu_capture::define(&mut module, native, &value_functions.evaluators).map_err(
            |error| {
                #[cfg(test)]
                eprintln!("native capture definition failed: {error:?}");
                #[cfg(not(test))]
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
        #[cfg(test)]
        eprintln!("native reduction definition failed: {error:?}");
        #[cfg(not(test))]
        let _ = error;
        Error::BackendFailure
    })?;
    let output_function =
        native_cpu_output::define(&mut module, native, value_functions.regex_find).map_err(
            |error| {
                #[cfg(test)]
                eprintln!("native output definition failed: {error:?}");
                #[cfg(not(test))]
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
                StructuralOperation::Supertype { kind } => {
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
                    let node_count = function.ins().load(
                        types::I32,
                        flags,
                        task,
                        mem::offset_of!(NativeTaskContext, node_count) as i32,
                    );
                    let scan = function.create_block();
                    function.append_block_param(scan, types::I32);
                    let inspect = function.create_block();
                    function.append_block_param(inspect, types::I32);
                    let compare = function.create_block();
                    function.append_block_param(compare, types::I32);
                    function.append_block_param(compare, types::I32);
                    let zero = function.ins().iconst(types::I32, 0);
                    function.ins().jump(scan, &[zero.into()]);
                    function.switch_to_block(scan);
                    let index = function.block_params(scan)[0];
                    let in_range =
                        function
                            .ins()
                            .icmp(IntCC::UnsignedLessThan, index, membership_count);
                    function
                        .ins()
                        .brif(in_range, inspect, &[index.into()], resume, &[]);
                    function.switch_to_block(inspect);
                    let index = function.block_params(inspect)[0];
                    let stride = function
                        .ins()
                        .imul_imm(index, mem::size_of::<MetalSupertype>() as i64);
                    let stride = function.ins().uextend(pointer_type, stride);
                    let membership = function.ins().iadd(memberships, stride);
                    let member_node = function.ins().load(
                        types::I32,
                        flags,
                        membership,
                        mem::offset_of!(MetalSupertype, node) as i32,
                    );
                    let valid_node =
                        function
                            .ins()
                            .icmp(IntCC::UnsignedLessThan, member_node, node_count);
                    function.ins().brif(
                        valid_node,
                        compare,
                        &[index.into(), member_node.into()],
                        invalid_input,
                        &[],
                    );
                    function.switch_to_block(compare);
                    let index = function.block_params(compare)[0];
                    let member_node = function.block_params(compare)[1];
                    let stride = function
                        .ins()
                        .imul_imm(index, mem::size_of::<MetalSupertype>() as i64);
                    let stride = function.ins().uextend(pointer_type, stride);
                    let membership = function.ins().iadd(memberships, stride);
                    let member_kind = function.ins().load(
                        types::I32,
                        flags,
                        membership,
                        mem::offset_of!(MetalSupertype, kind) as i32,
                    );
                    let same_node = function.ins().icmp(IntCC::Equal, member_node, current);
                    let same_kind =
                        function
                            .ins()
                            .icmp_imm(IntCC::Equal, member_kind, i64::from(kind));
                    let matched = function.ins().band(same_node, same_kind);
                    let advance = function.create_block();
                    function
                        .ins()
                        .brif(matched, next.unwrap_or(resume), &[], advance, &[]);
                    function.switch_to_block(advance);
                    let index = function.ins().iadd_imm(index, 1);
                    function.ins().jump(scan, &[index.into()]);
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
                    function.ins().jump(resume, &[]);
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
    #[cfg(test)]
    let cursor_equal = unsafe {
        mem::transmute::<*const u8, unsafe extern "C" fn(*mut NativeTaskContext, u32, u32) -> u32>(
            module.get_finalized_function(scheduler.cursor_equal),
        )
    };
    #[cfg(test)]
    let repeat_equal = unsafe {
        mem::transmute::<*const u8, unsafe extern "C" fn(*mut NativeTaskContext, u32, u32) -> u32>(
            module.get_finalized_function(scheduler.repeat_equal),
        )
    };
    #[cfg(test)]
    let capture_contains = unsafe {
        mem::transmute::<*const u8, unsafe extern "C" fn(*mut NativeTaskContext, u32, u32) -> u32>(
            module.get_finalized_function(scheduler.capture_contains),
        )
    };
    #[cfg(test)]
    let query_admit = unsafe {
        mem::transmute::<*const u8, unsafe extern "C" fn(*mut NativeTaskContext, u32, u32) -> u32>(
            module.get_finalized_function(scheduler.query_admit),
        )
    };
    #[cfg(test)]
    let queue_insert = unsafe {
        mem::transmute::<*const u8, unsafe extern "C" fn(*mut NativeTaskContext, u32, u32) -> u32>(
            module.get_finalized_function(scheduler.queue_insert),
        )
    };
    #[cfg(test)]
    let greedy_schedule = unsafe {
        mem::transmute::<*const u8, unsafe extern "C" fn(*mut NativeTaskContext, u32) -> u64>(
            module.get_finalized_function(scheduler.greedy_schedule),
        )
    };
    #[cfg(test)]
    let greedy_take = unsafe {
        mem::transmute::<*const u8, unsafe extern "C" fn(*mut NativeTaskContext, u32) -> u64>(
            module.get_finalized_function(scheduler.greedy_take),
        )
    };
    #[cfg(test)]
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
        #[cfg(test)]
        cursor_equal,
        #[cfg(test)]
        repeat_equal,
        #[cfg(test)]
        capture_contains,
        #[cfg(test)]
        query_admit,
        #[cfg(test)]
        exact_state_equal,
        #[cfg(test)]
        queue_insert,
        #[cfg(test)]
        greedy_schedule,
        #[cfg(test)]
        greedy_take,
    })
}

pub(super) fn compile_cached(program: &Program) -> Result<Arc<CompiledStraightLine>, Error> {
    let key = format!(
        "cpu-jit:{FORMAT_VERSION}:{}:{}",
        std::env::consts::ARCH,
        program.canonical_digest("", b"")
    );
    if let Some(compiled) = CPU_JIT_CACHE
        .lock()
        .map_err(|_| Error::BackendFailure)?
        .get(&key)
        .cloned()
    {
        return Ok(compiled);
    }
    let native = super::native_ir::lower(program).map_err(|_| Error::BackendFailure)?;
    let compiled = Arc::new(compile_straight_line(&native)?);
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

pub(super) fn execute_batch(
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

#[cfg(all(test, target_os = "macos"))]
pub(super) fn execute_terminal_reference_with_work_limit(
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

fn execute_batch_packed_with_work_limit(
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
    result_limits: Option<&[u32]>,
    work_limit: u32,
) -> Result<PackedExecution, Error> {
    let mut batch = PackedBatch::new(programs, files, anchors)?;
    if batch.tasks.is_empty() {
        return Ok(PackedExecution::new(batch, PackedResultPage::empty()));
    }
    let totals = batch.assign_workspace_with_result_limits(programs, result_limits)?;
    if std::env::var_os("TEMPO_PACKED_BATCH_PROFILE").is_some() {
        eprintln!(
            "TEMPO_PACKED_BATCH_PROFILE backend=cpu programs={} tasks={} workspaceBytes={}",
            programs.len(),
            batch.tasks.len(),
            batch.workspace_allocation_bytes(totals).unwrap_or(u64::MAX),
        );
    }
    let compiled = programs
        .iter()
        .map(compile_cached)
        .collect::<Result<Vec<_>, Error>>()?;
    let allocation_started = std::time::Instant::now();
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
        contexts.push((task.program as usize, context));
    }
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
    let evaluation_locks = (0..batch.evaluations.len())
        .map(|_| Mutex::new(()))
        .collect::<Vec<_>>();
    let execution_started = std::time::Instant::now();
    let task_outputs = WORK_POOL.install(|| {
        contexts
            .par_iter_mut()
            .enumerate()
            .map(|(task_index, (program, context))| {
                let evaluation = batch.tasks[task_index].evaluation as usize;
                let _evaluation = evaluation_locks[evaluation]
                    .lock()
                    .map_err(|_| Error::BackendFailure)?;
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
            })
            .collect::<Result<Vec<_>, Error>>()
    })?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::{
        tape::{Grammar, TapeImage},
        transducer::packed::{CAPTURE, EMIT, KIND, Program, native_ir, word},
    };
    use ast_grep_core::tree_sitter::StrDoc;
    use crate::code_rule_dsl::language::GrammarDialect as SupportLang;
    use ast_grep_core::tree_sitter::LanguageExt;
    use std::sync::Arc;

    #[test]
    fn demand_zero_cpu_workspace_has_typed_zeroed_storage() {
        let mut values = DemandZeroRegion::<u32>::new(17).unwrap();
        assert_eq!(&*values, &[0; 17]);
        values[8] = 42;
        assert_eq!(values[8], 42);
        assert!(DemandZeroRegion::<u32>::new(0).unwrap().is_empty());
    }

    fn tape(source: &str) -> TapeDoc {
        let grammar = Grammar::new(SupportLang::Python).unwrap();
        let native = StrDoc::new(source, SupportLang::Python);
        let image =
            TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
        TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap()
    }

    #[test]
    fn structural_interpreter_publication_preserves_raw_results() {
        let files = [tape(
            "# first\n# second\n# third\ndef example(value: tuple[int, None]):\n    return value\n",
        )];
        for query in [
            "(comment)+ @comment.around",
            "(identifier) @identifier",
            "(type (_ [(identifier) (none)]? @type (_ [(identifier) (none)]? @type)))",
        ] {
            let programs = [Program::compile_tree_sitter_query_pattern(
                query,
                &SupportLang::Python.get_ts_language(),
            )
            .unwrap()];
            let kinds = programs[0].leading_kinds().unwrap();
            let anchors = vec![vec![
                (0..files[0].transducer_len() as u32)
                    .filter(|&node| kinds.contains(&files[0].transducer_kind(node)))
                    .collect::<Vec<_>>(),
            ]];
            let expected = super::execute_batch(&programs, &files, &anchors).unwrap();
            let actual = crate::code_rule_dsl::transducer::packed::interpreter_reference(
                &programs,
                &files[0],
                &anchors[0],
                &[u32::MAX],
            )
            .unwrap()
            .into_outcome()
            .unwrap();
            assert_eq!(actual.outputs, expected, "{query}");
            assert_eq!(actual.errors, vec![vec![None]]);
        }
    }

    #[test]
    fn interpreter_publication_preserves_every_output_directive_table() {
        let files = [tape("ALPHA\n'application/json'\n")];
        let queries = [
            r#"((identifier) @inner @outer (#offset! @inner 0 1 0 -1) (#downcase! @outer) (#set! role "trimmed") (#set! flag) (#set! @outer kind "raw"))"#,
            "((string (string_content) @mime) (#set-lang-from-mimetype! @mime))",
        ];
        for (index, query) in queries.iter().enumerate() {
            let program = Program::compile_tree_sitter_query_pattern(
                query,
                &SupportLang::Python.get_ts_language(),
            )
            .unwrap();
            let roots = (0..files[0].transducer_len() as u32).collect::<Vec<_>>();
            let anchors = vec![vec![roots.clone()]];
            let expected = crate::code_rule_dsl::transducer::packed::execute_batch(
                std::slice::from_ref(&program),
                &files,
                Some(&anchors),
            )
            .unwrap();
            assert_eq!(expected[0][0].len(), 1, "{query}");
            let row = &expected[0][0][0];
            let resolve = |(key, value): &(u16, Option<u16>)| {
                (
                    program.strings[usize::from(*key)].clone(),
                    value.map(|value| program.strings[usize::from(value)].clone()),
                )
            };
            if index == 0 {
                assert_eq!(row.captures.len(), 2);
                assert_eq!(
                    row.captures
                        .iter()
                        .map(|capture| program.label_name(capture.label).unwrap().to_owned())
                        .collect::<Vec<_>>(),
                    ["inner".to_owned(), "outer".to_owned()]
                );
                assert_eq!(row.captures[0].span, 1..4);
                assert_eq!(row.captures[1].span, 0..5);
                assert_eq!(row.captures[0].text, None);
                assert_eq!(row.captures[1].text.as_deref(), Some(b"alpha".as_slice()));
                assert_eq!(
                    row.properties.iter().map(&resolve).collect::<Vec<_>>(),
                    [
                        (b"role".to_vec(), Some(b"trimmed".to_vec())),
                        (b"flag".to_vec(), None),
                    ]
                );
                assert!(row.captures[0].properties.is_empty());
                assert_eq!(
                    row.captures[1]
                        .properties
                        .iter()
                        .map(&resolve)
                        .collect::<Vec<_>>(),
                    [(b"kind".to_vec(), Some(b"raw".to_vec())),]
                );
                assert!(row.dynamic_properties.is_empty());
            } else {
                assert_eq!(row.captures.len(), 1);
                assert_eq!(row.captures[0].span, 7..23);
                assert_eq!(row.captures[0].text, None);
                assert!(row.properties.is_empty());
                assert_eq!(
                    row.dynamic_properties,
                    [(b"language".to_vec(), Some(b"json".to_vec())),]
                );
            }
            #[cfg(target_os = "macos")]
            let scope = crate::code_rule_dsl::transducer::packed::MetalTestScope::enter();
            let actual = crate::code_rule_dsl::transducer::packed::interpreter_reference(
                std::slice::from_ref(&program),
                &files[0],
                std::slice::from_ref(&roots),
                &[u32::MAX],
            )
            .expect("the independent publication oracle must support output directives")
            .into_outcome()
            .unwrap();
            assert_eq!(actual.outputs, expected, "{query}");
            assert_eq!(actual.errors, vec![vec![None]]);
            #[cfg(target_os = "macos")]
            assert!(scope.snapshot().is_empty());
        }
    }

    #[test]
    fn interpreter_publication_preserves_limits_errors_and_original_task_gaps() {
        use crate::code_rule_dsl::transducer::packed::{RESTORE_CURSOR, interpreter_reference};
        let files = [tape("ALPHA\n'application/json'\n")];
        let language = SupportLang::Python.get_ts_language();
        let programs = [
            Program::compile_tree_sitter_query_pattern(
                "((identifier) @name (#set! role \"name\"))",
                &language,
            )
            .unwrap(),
            Program::new(
                vec![word(RESTORE_CURSOR, 0, 0), word(EMIT, 0, 0)],
                vec![b"error".to_vec()],
                vec![],
                vec![0],
                vec![],
            )
            .unwrap(),
            Program::compile_tree_sitter_query_pattern(
                "((string (string_content) @mime) (#set-lang-from-mimetype! @mime))",
                &language,
            )
            .unwrap(),
        ];
        let roots = (0..files[0].transducer_len() as u32).collect::<Vec<_>>();
        let anchors = vec![roots.clone(), vec![0], roots.clone()];
        let limits = [0, 0, 1];
        let mut expected = crate::code_rule_dsl::transducer::packed::execute_batch_outcomes(
            &programs,
            &files,
            std::slice::from_ref(&anchors),
        )
        .unwrap();
        expected.outputs[0][0].clear();
        expected.outputs[0][2].truncate(1);
        let actual = interpreter_reference(&programs, &files[0], &anchors, &limits)
            .unwrap()
            .into_outcome()
            .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(
            actual.errors,
            [vec![None, Some(Error::InvalidOperand(0)), None]]
        );
        assert_eq!(actual.outputs[0][2].len(), 1);
        let string = roots
            .iter()
            .copied()
            .find(|&node| {
                files[0].transducer_kind(node) == language.id_for_node_kind("string", true)
            })
            .unwrap();
        let row = &actual.outputs[0][2][0];
        assert_eq!(row.start, string);
        assert_eq!(
            row.task_ordinal as usize,
            roots.len() + 1 + roots.iter().position(|&node| node == string).unwrap()
        );
        assert_eq!(
            row.dynamic_properties,
            [(b"language".to_vec(), Some(b"json".to_vec()))]
        );
        assert!(matches!(
            interpreter_reference(&programs, &files[0], &anchors[..2], &limits),
            Err(Error::InvalidAnchor)
        ));
        assert!(matches!(
            interpreter_reference(&programs, &files[0], &anchors, &limits[..2]),
            Err(Error::InvalidAnchor)
        ));
        let invalid_roots = vec![vec![1, 0], vec![0], roots];
        assert!(matches!(
            interpreter_reference(&programs, &files[0], &invalid_roots, &limits),
            Err(Error::InvalidAnchor)
        ));
    }

    #[test]
    #[ignore = "pinned pattern 277 and argparse output-directive reference regression"]
    fn interpreter_output_reference_preserves_pinned_argparse_results() {
        let repository =
            std::path::PathBuf::from(std::env::var("TEMPO_ALL_SCM_REPOSITORY").unwrap());
        let query_root = std::env::var("TEMPO_PYTHON_SCM_QUERY_DIR").unwrap();
        let mut paths = std::fs::read_dir(query_root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "scm"))
            .collect::<Vec<_>>();
        paths.sort();
        let patterns = paths
            .iter()
            .flat_map(|path| {
                Program::tree_sitter_top_level_patterns(&std::fs::read_to_string(path).unwrap())
                    .unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(patterns.len(), 335);
        let program = Program::compile_tree_sitter_query_pattern(
            &patterns[277],
            &SupportLang::Python.get_ts_language(),
        )
        .unwrap();
        assert!(!program.output_program.instructions.is_empty());
        let original =
            std::fs::read_to_string(repository.join("Lib/test/test_argparse.py")).unwrap();
        for (source, root_count, match_count) in [
            (original.clone(), 3284, 0),
            (
                format!("{original}\nre.compile(r'CPU_REFERENCE_PROOF')\n"),
                3285,
                1,
            ),
        ] {
            let files = [tape(&source)];
            let call = SupportLang::Python
                .get_ts_language()
                .id_for_node_kind("call", true);
            let roots = (0..files[0].transducer_len() as u32)
                .filter(|&node| files[0].transducer_kind(node) == call)
                .collect::<Vec<_>>();
            assert_eq!(roots.len(), root_count);
            let anchors = vec![vec![roots.clone()]];
            let expected = crate::code_rule_dsl::transducer::packed::execute_batch(
                std::slice::from_ref(&program),
                &files,
                Some(&anchors),
            )
            .unwrap();
            assert_eq!(expected[0][0].len(), match_count);
            if match_count == 1 {
                let row = &expected[0][0][0];
                assert_eq!(row.task_ordinal as usize, root_count - 1);
                assert_eq!(
                    row.captures
                        .iter()
                        .map(|capture| (
                            program.label_name(capture.label).unwrap().to_owned(),
                            source[capture.span.clone()].to_owned(),
                        ))
                        .collect::<Vec<_>>(),
                    [
                        ("_re".to_owned(), "re".to_owned()),
                        (
                            "injection.content".to_owned(),
                            "CPU_REFERENCE_PROOF".to_owned()
                        ),
                    ]
                );
                assert_eq!(
                    row.properties
                        .iter()
                        .map(|(key, value)| (
                            program.string_value(*key).unwrap().to_owned(),
                            value.map(|value| program.string_value(value).unwrap().to_owned()),
                        ))
                        .collect::<Vec<_>>(),
                    [("injection.language".to_owned(), Some("regex".to_owned())),]
                );
            }
            let actual = crate::code_rule_dsl::transducer::packed::interpreter_reference(
                std::slice::from_ref(&program),
                &files[0],
                std::slice::from_ref(&roots),
                &[u32::MAX],
            )
            .expect("the pinned output-directive reference must complete without JIT workspace")
            .into_outcome()
            .unwrap();
            assert_eq!(actual.outputs, expected);
            assert_eq!(actual.errors, vec![vec![None]]);
            eprintln!(
                "TEMPO_ARGPARSE_REFERENCE_EVIDENCE {}",
                serde_json::json!({
                    "roots": roots.len(), "matches": actual.outputs[0][0].len(),
                    "captures": actual.outputs[0][0].iter().map(|row| row.captures.len()).sum::<usize>(),
                    "matchProperties": actual.outputs[0][0].iter().map(|row| row.properties.len()).sum::<usize>(),
                })
            );
        }
    }

    #[test]
    #[ignore = "offline independent interpreter timing for the CPU corpus reference"]
    fn cpu_quantifier_interpreter_reference_profile() {
        let path = std::env::var("TEMPO_CPU_QUANTIFIER_SOURCE").unwrap();
        let source = std::fs::read_to_string(path).unwrap();
        let files = [tape(&source)];
        let programs = [Program::compile_tree_sitter_query_pattern(
            "(comment)+ @comment.around",
            &SupportLang::Python.get_ts_language(),
        )
        .unwrap()];
        let kinds = programs[0].leading_kinds().unwrap();
        let anchors = vec![vec![
            (0..files[0].transducer_len() as u32)
                .filter(|&node| kinds.contains(&files[0].transducer_kind(node)))
                .collect::<Vec<_>>(),
        ]];
        let started = std::time::Instant::now();
        let output =
            crate::code_rule_dsl::transducer::packed::execute_batch(&programs, &files, Some(&anchors))
                .unwrap();
        eprintln!(
            "TEMPO_INTERPRETER_QUANTIFIER_REFERENCE roots={} matches={} captures={} seconds={:.6}",
            anchors[0][0].len(),
            output[0][0].len(),
            output[0][0]
                .iter()
                .map(|result| result.captures.len())
                .sum::<usize>(),
            started.elapsed().as_secs_f64()
        );
    }

    #[test]
    fn nested_optional_types_keep_interpreter_candidate_ordinals() {
        let query = "(type (_ [(identifier) (none)]? @type (_ [(identifier) (none)]? @type (_ [(identifier) (none)]? @type (_ [(identifier) (none)]? @type)))))";
        let programs = [Program::compile_tree_sitter_query_pattern(
            query,
            &SupportLang::Python.get_ts_language(),
        )
        .unwrap()];
        let files = [tape(
            "fields: ClassVar[dict[str, Field[str]]]\nfile: IO[str] | IO[bytes] | None\n",
        )];
        let anchors = vec![vec![
            (0..files[0].transducer_len() as u32).collect::<Vec<_>>(),
        ]];
        let expected =
            crate::code_rule_dsl::transducer::packed::execute_batch(&programs, &files, Some(&anchors))
                .unwrap();
        assert_eq!(
            super::execute_batch(&programs, &files, &anchors).unwrap(),
            expected
        );
    }

    #[test]
    fn shared_cpu_query_storage_isolated_between_semantic_roots() {
        let program = Program::compile_tree_sitter_query_pattern(
            "(comment)+ @comment.around",
            &SupportLang::Python.get_ts_language(),
        )
        .unwrap();
        let files = [tape("# first\n# second\nvalue\n# third\n")];
        let anchors = vec![vec![
            (0..files[0].transducer_len() as u32).collect::<Vec<_>>(),
        ]];
        let expected = crate::code_rule_dsl::transducer::packed::execute_batch(
            std::slice::from_ref(&program),
            &files,
            Some(&anchors),
        )
        .unwrap();
        let actual = super::execute_batch(&[program], &files, &anchors).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn callback_free_cpu_jit_executes_the_shared_packed_batch() {
        let grammar = SupportLang::Python.get_ts_language();
        let kind = grammar.id_for_node_kind("identifier", true);
        let program = Program::new(
            vec![word(KIND, 0, kind), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
            vec![b"match".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let files = vec![tape("first\n"), tape("second\n")];
        let anchors = files
            .iter()
            .map(|file| vec![(0..file.transducer_len() as u32).collect::<Vec<_>>()])
            .collect::<Vec<_>>();
        let expected = crate::code_rule_dsl::transducer::packed::execute_batch(
            std::slice::from_ref(&program),
            &files,
            Some(&anchors),
        )
        .unwrap();
        let actual = super::execute_batch(&[program], &files, &anchors).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn callback_free_cpu_jit_uses_per_program_packed_value_ranges() {
        use crate::code_rule_dsl::transducer::packed::{Directive, TEXT, TextPredicate};

        let programs = [b"FIRST".to_vec(), b"SECOND".to_vec()]
            .into_iter()
            .map(|expected| {
                Program::new(
                    vec![word(TEXT, 0, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
                    vec![expected.clone(), b"match".to_vec()],
                    vec![TextPredicate::Equal(0)],
                    vec![1],
                    Vec::new(),
                )
                .unwrap()
                .with_directives(vec![Directive::Downcase { capture: 0 }])
                .unwrap()
            })
            .collect::<Vec<_>>();
        let files = vec![tape("FIRST"), tape("SECOND")];
        let anchors = files
            .iter()
            .map(|file| {
                programs
                    .iter()
                    .map(|_| (0..file.transducer_len() as u32).collect::<Vec<_>>())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let expected =
            crate::code_rule_dsl::transducer::packed::execute_batch(&programs, &files, Some(&anchors))
                .unwrap();
        let actual = super::execute_batch(&programs, &files, &anchors).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn cranelift_task_shell_writes_status_without_a_rust_callout() {
        let program = Program::new(
            vec![word(EMIT, 0, 0)],
            vec![b"match".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let native = native_ir::lower(&program).unwrap();
        let compiled = compile_task_shell(&native).unwrap();
        let mut status = TaskStatus {
            error: 1,
            result_count: 2,
            capture_count: 3,
            program_counter: 4,
            final_capture_count: 5,
            property_count: 6,
            output_byte_count: 7,
            reserved: 8,
        };
        compiled.execute(&mut status);
        assert_eq!(status, TaskStatus::default());
    }

    #[test]
    fn straight_line_cranelift_executes_kind_flags_size_and_emit() {
        use crate::code_rule_dsl::transducer::packed::{FLAGS, KIND, SIZE};

        let program = Program::new(
            vec![
                word(KIND, 0, 7),
                word(FLAGS, 1, 2),
                word(SIZE, 0, 4),
                word(EMIT, 0, 0),
            ],
            vec![b"match".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let native = native_ir::lower(&program).unwrap();
        let compiled = compile_straight_line(&native).unwrap();
        let nodes = [MetalNode {
            source_start: 3,
            source_end: 7,
            kind: 7,
            flags: 1,
            ..MetalNode::default()
        }];
        let mut results = [ResultRecord::default(); 1];
        let mut status = TaskStatus::default();
        let mut context = NativeTaskContext {
            nodes: nodes.as_ptr(),
            node_count: 1,
            children: std::ptr::null(),
            child_count: 0,
            supertypes: std::ptr::null(),
            supertype_count: 0,
            source: std::ptr::null(),
            source_length: 0,
            values: std::ptr::null(),
            value_count: 0,
            nfa_states: std::ptr::null(),
            nfa_state_count: 0,
            nfa_data: std::ptr::null(),
            nfa_data_count: 0,
            nfa_ranges: std::ptr::null(),
            nfa_range_count: 0,
            lua_instructions: std::ptr::null(),
            lua_instruction_count: 0,
            lua_members: std::ptr::null(),
            lua_member_count: 0,
            value_work: std::ptr::null_mut(),
            value_start: 0,
            value_capacity: 0,
            current_node: 0,
            start_node: 0,
            task_ordinal: 9,
            pending: std::ptr::null_mut(),
            pending_count: 0,
            pending_quota_limit: 0,
            current_steps: 0,
            current_captures: 0,
            current_cursors: 0,
            current_repeats: 0,
            current_following: 0,
            current_capture_hash: 0,
            current_cursor_hash: 0,
            current_repeat_hash: 0,
            task_work_limit: 16,
            path_work_limit: 16,
            captures: std::ptr::null(),
            cursors: std::ptr::null(),
            repeats: std::ptr::null(),
            query_buckets: std::ptr::null_mut(),
            query_bucket_capacity: 0,
            query_members: std::ptr::null_mut(),
            query_member_count: 0,
            query_member_quota_limit: 0,
            queued_states: std::ptr::null_mut(),
            queued_state_quota_limit: 0,
            greedy_entries: std::ptr::null_mut(),
            greedy_entry_quota_limit: 0,
            maximum_cursor_slot: 0,
            capture_count: 0,
            capture_quota_limit: 0,
            cursor_count: 0,
            cursor_quota_limit: 0,
            repeat_count: 0,
            repeat_quota_limit: 0,
            results: results.as_mut_ptr(),
            result_quota_limit: 1,
            line_starts: std::ptr::null(),
            line_count: 0,
            final_captures: std::ptr::null_mut(),
            final_capture_capacity: 0,
            properties: std::ptr::null_mut(),
            property_quota_limit: 0,
            output_bytes: std::ptr::null_mut(),
            output_byte_quota_limit: 0,
            status: &mut status,
        };
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 1);
        assert_eq!(results[0].task_ordinal, 9);
        assert_eq!(results[0].local_ordinal, 0);
        assert_eq!(results[0].label, 0);
    }

    #[test]
    fn straight_line_cranelift_rejects_without_emitting() {
        use crate::code_rule_dsl::transducer::packed::KIND;

        let program = Program::new(
            vec![word(KIND, 0, 8), word(EMIT, 0, 0)],
            vec![b"match".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let native = native_ir::lower(&program).unwrap();
        let compiled = compile_straight_line(&native).unwrap();
        let nodes = [MetalNode {
            kind: 7,
            ..MetalNode::default()
        }];
        let mut results = [ResultRecord::default(); 1];
        let mut status = TaskStatus::default();
        let mut context = NativeTaskContext {
            nodes: nodes.as_ptr(),
            node_count: 1,
            children: std::ptr::null(),
            child_count: 0,
            supertypes: std::ptr::null(),
            supertype_count: 0,
            source: std::ptr::null(),
            source_length: 0,
            values: std::ptr::null(),
            value_count: 0,
            nfa_states: std::ptr::null(),
            nfa_state_count: 0,
            nfa_data: std::ptr::null(),
            nfa_data_count: 0,
            nfa_ranges: std::ptr::null(),
            nfa_range_count: 0,
            lua_instructions: std::ptr::null(),
            lua_instruction_count: 0,
            lua_members: std::ptr::null(),
            lua_member_count: 0,
            value_work: std::ptr::null_mut(),
            value_start: 0,
            value_capacity: 0,
            current_node: 0,
            start_node: 0,
            task_ordinal: 0,
            pending: std::ptr::null_mut(),
            pending_count: 0,
            pending_quota_limit: 0,
            current_steps: 0,
            current_captures: 0,
            current_cursors: 0,
            current_repeats: 0,
            current_following: 0,
            current_capture_hash: 0,
            current_cursor_hash: 0,
            current_repeat_hash: 0,
            task_work_limit: 16,
            path_work_limit: 16,
            captures: std::ptr::null(),
            cursors: std::ptr::null(),
            repeats: std::ptr::null(),
            query_buckets: std::ptr::null_mut(),
            query_bucket_capacity: 0,
            query_members: std::ptr::null_mut(),
            query_member_count: 0,
            query_member_quota_limit: 0,
            queued_states: std::ptr::null_mut(),
            queued_state_quota_limit: 0,
            greedy_entries: std::ptr::null_mut(),
            greedy_entry_quota_limit: 0,
            maximum_cursor_slot: 0,
            capture_count: 0,
            capture_quota_limit: 0,
            cursor_count: 0,
            cursor_quota_limit: 0,
            repeat_count: 0,
            repeat_quota_limit: 0,
            results: results.as_mut_ptr(),
            result_quota_limit: 1,
            line_starts: std::ptr::null(),
            line_count: 0,
            final_captures: std::ptr::null_mut(),
            final_capture_capacity: 0,
            properties: std::ptr::null_mut(),
            property_quota_limit: 0,
            output_bytes: std::ptr::null_mut(),
            output_byte_quota_limit: 0,
            status: &mut status,
        };
        compiled.execute(&mut context);
        assert_eq!(
            status,
            TaskStatus {
                reserved: 1,
                ..TaskStatus::default()
            }
        );
    }

    #[test]
    fn cranelift_scheduler_preserves_fallthrough_before_deferred_fork_order() {
        use crate::code_rule_dsl::transducer::packed::{FORK, STOP};

        let program = Program::new(
            vec![
                word(FORK, 0, 3),
                word(EMIT, 0, 0),
                word(STOP, 0, 0),
                word(EMIT, 0, 1),
            ],
            vec![b"first".to_vec(), b"second".to_vec()],
            Vec::new(),
            vec![0, 1],
            Vec::new(),
        )
        .unwrap();
        let native = native_ir::lower(&program).unwrap();
        let compiled = compile_straight_line(&native).unwrap();
        let nodes = [MetalNode::default()];
        let mut pending = [PendingRecord::default(); 1];
        let captures = [LinkRecord::default(); 11];
        let mut final_captures = [FinalCaptureRecord::default(); 2];
        let mut results = [ResultRecord::default(); 2];
        let mut status = TaskStatus::default();
        let mut context = NativeTaskContext {
            nodes: nodes.as_ptr(),
            node_count: 1,
            children: std::ptr::null(),
            child_count: 0,
            supertypes: std::ptr::null(),
            supertype_count: 0,
            source: std::ptr::null(),
            source_length: 0,
            values: std::ptr::null(),
            value_count: 0,
            nfa_states: std::ptr::null(),
            nfa_state_count: 0,
            nfa_data: std::ptr::null(),
            nfa_data_count: 0,
            nfa_ranges: std::ptr::null(),
            nfa_range_count: 0,
            lua_instructions: std::ptr::null(),
            lua_instruction_count: 0,
            lua_members: std::ptr::null(),
            lua_member_count: 0,
            value_work: std::ptr::null_mut(),
            value_start: 0,
            value_capacity: 0,
            current_node: 0,
            start_node: 0,
            task_ordinal: 0,
            pending: pending.as_mut_ptr(),
            pending_count: 0,
            pending_quota_limit: 1,
            current_steps: 0,
            current_captures: 11,
            current_cursors: 12,
            current_repeats: 13,
            current_following: 0,
            current_capture_hash: 15,
            current_cursor_hash: 16,
            current_repeat_hash: 17,
            task_work_limit: 16,
            path_work_limit: 16,
            captures: captures.as_ptr(),
            cursors: std::ptr::null(),
            repeats: std::ptr::null(),
            query_buckets: std::ptr::null_mut(),
            query_bucket_capacity: 0,
            query_members: std::ptr::null_mut(),
            query_member_count: 0,
            query_member_quota_limit: 0,
            queued_states: std::ptr::null_mut(),
            queued_state_quota_limit: 0,
            greedy_entries: std::ptr::null_mut(),
            greedy_entry_quota_limit: 0,
            maximum_cursor_slot: 0,
            capture_count: 0,
            capture_quota_limit: 0,
            cursor_count: 0,
            cursor_quota_limit: 0,
            repeat_count: 0,
            repeat_quota_limit: 0,
            results: results.as_mut_ptr(),
            result_quota_limit: 2,
            line_starts: std::ptr::null(),
            line_count: 0,
            final_captures: final_captures.as_mut_ptr(),
            final_capture_capacity: final_captures.len() as u32,
            properties: std::ptr::null_mut(),
            property_quota_limit: 0,
            output_bytes: std::ptr::null_mut(),
            output_byte_quota_limit: 0,
            status: &mut status,
        };
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 2);
        assert_eq!(
            (
                context.current_captures,
                context.current_cursors,
                context.current_repeats,
                context.current_following,
                context.current_capture_hash,
                context.current_cursor_hash,
                context.current_repeat_hash,
            ),
            (11, 12, 13, 0, 15, 16, 17)
        );
        assert_eq!([results[0].label, results[1].label], [0, 1]);
        assert_eq!([results[0].local_ordinal, results[1].local_ordinal], [0, 1]);
    }

    #[test]
    fn cranelift_output_sorts_results_by_start_then_first_capture_node() {
        use crate::code_rule_dsl::transducer::packed::{CAPTURE, FORK, NEXT, STOP};

        let program = Program::new(
            vec![
                word(FORK, 0, 5),
                word(NEXT, 0, 0),
                word(CAPTURE, 0, 0),
                word(EMIT, 0, 0),
                word(STOP, 0, 0),
                word(CAPTURE, 0, 0),
                word(EMIT, 0, 0),
            ],
            vec![b"capture".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let compiled = compile_straight_line(&native_ir::lower(&program).unwrap()).unwrap();
        let nodes = [
            MetalNode {
                next_any: 1,
                next_named: 1,
                parent: NONE,
                source_end: 2,
                ..MetalNode::default()
            },
            MetalNode {
                parent: NONE,
                next_any: NONE,
                next_named: NONE,
                source_start: 1,
                source_end: 2,
                ..MetalNode::default()
            },
        ];
        let mut pending = [PendingRecord::default(); 1];
        let mut captures = [LinkRecord::default(); 2];
        let mut final_captures = [FinalCaptureRecord::default(); 2];
        let mut results = [ResultRecord::default(); 2];
        let mut status = TaskStatus::default();
        let mut context: NativeTaskContext = unsafe { mem::zeroed() };
        context.nodes = nodes.as_ptr();
        context.node_count = nodes.len() as u32;
        context.current_node = 0;
        context.pending = pending.as_mut_ptr();
        context.pending_quota_limit = 1;
        context.task_work_limit = 32;
        context.path_work_limit = 32;
        context.captures = captures.as_mut_ptr();
        context.capture_quota_limit = captures.len() as u32;
        context.results = results.as_mut_ptr();
        context.result_quota_limit = results.len() as u32;
        context.final_captures = final_captures.as_mut_ptr();
        context.final_capture_capacity = final_captures.len() as u32;
        context.status = &mut status;
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 2);
        assert_eq!(
            [
                final_captures[results[0].capture_start as usize].node,
                final_captures[results[1].capture_start as usize].node,
            ],
            [0, 1],
        );
        assert_eq!([results[0].local_ordinal, results[1].local_ordinal], [1, 0]);
    }

    #[test]
    fn cranelift_greedy_fork_replaces_one_stable_pending_token() {
        use crate::code_rule_dsl::transducer::packed::{FORK, GOTO};

        let program = Program::new(
            vec![
                word(FORK, 1, 5),
                word(FORK, 0, 3),
                word(GOTO, 0, 3),
                word(FORK, 1, 5),
                word(EMIT, 0, 0),
                word(EMIT, 0, 1),
            ],
            vec![b"first".to_vec(), b"second".to_vec()],
            Vec::new(),
            vec![0, 1],
            Vec::new(),
        )
        .unwrap();
        let mut native = native_ir::lower(&program).unwrap();
        native.tree_sitter_query = true;
        native.has_repeats = true;
        let compiled = compile_straight_line(&native).unwrap();
        let nodes = [MetalNode::default()];
        let mut pending = [PendingRecord::default(); 2];
        let mut query_buckets = [QueryBucketRecord::default(); 8];
        let mut query_members = [QueryMemberRecord::default(); 6];
        let mut queued_states = [QueuedStateRecord::default(); 1];
        let mut greedy_entries = [GreedyEntryRecord::default(); 1];
        let mut results = [ResultRecord::default(); 2];
        let mut status = TaskStatus::default();
        let mut context = NativeTaskContext {
            nodes: nodes.as_ptr(),
            node_count: 1,
            children: std::ptr::null(),
            child_count: 0,
            supertypes: std::ptr::null(),
            supertype_count: 0,
            source: std::ptr::null(),
            source_length: 0,
            values: std::ptr::null(),
            value_count: 0,
            nfa_states: std::ptr::null(),
            nfa_state_count: 0,
            nfa_data: std::ptr::null(),
            nfa_data_count: 0,
            nfa_ranges: std::ptr::null(),
            nfa_range_count: 0,
            lua_instructions: std::ptr::null(),
            lua_instruction_count: 0,
            lua_members: std::ptr::null(),
            lua_member_count: 0,
            value_work: std::ptr::null_mut(),
            value_start: 0,
            value_capacity: 0,
            current_node: 0,
            start_node: 0,
            task_ordinal: 0,
            pending: pending.as_mut_ptr(),
            pending_count: 0,
            pending_quota_limit: pending.len() as u32,
            current_steps: 0,
            current_captures: 0,
            current_cursors: 0,
            current_repeats: 0,
            current_following: 0,
            current_capture_hash: 0,
            current_cursor_hash: 0,
            current_repeat_hash: 0,
            task_work_limit: 64,
            path_work_limit: 64,
            captures: std::ptr::null(),
            cursors: std::ptr::null(),
            repeats: std::ptr::null(),
            query_buckets: query_buckets.as_mut_ptr(),
            query_bucket_capacity: query_buckets.len() as u32,
            query_members: query_members.as_mut_ptr(),
            query_member_count: 0,
            query_member_quota_limit: query_members.len() as u32,
            queued_states: queued_states.as_mut_ptr(),
            queued_state_quota_limit: queued_states.len() as u32,
            greedy_entries: greedy_entries.as_mut_ptr(),
            greedy_entry_quota_limit: greedy_entries.len() as u32,
            maximum_cursor_slot: 0,
            capture_count: 0,
            capture_quota_limit: 0,
            cursor_count: 0,
            cursor_quota_limit: 0,
            repeat_count: 0,
            repeat_quota_limit: 0,
            results: results.as_mut_ptr(),
            result_quota_limit: results.len() as u32,
            line_starts: std::ptr::null(),
            line_count: 0,
            final_captures: std::ptr::null_mut(),
            final_capture_capacity: 0,
            properties: std::ptr::null_mut(),
            property_quota_limit: 0,
            output_bytes: std::ptr::null_mut(),
            output_byte_quota_limit: 0,
            status: &mut status,
        };
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 1);
        assert_eq!(results[0].label, 0);
        assert_eq!(greedy_entries[0].occupied, 2);
        assert_eq!(greedy_entries[0].path.pc, 5);
        assert!(greedy_entries[0].path.steps > 1);
        assert_eq!(context.pending_count, 0);

        query_buckets.fill(QueryBucketRecord::default());
        query_members.fill(QueryMemberRecord::default());
        queued_states.fill(QueuedStateRecord::default());
        greedy_entries.fill(GreedyEntryRecord::default());
        pending.fill(PendingRecord::default());
        results.fill(ResultRecord::default());
        status = TaskStatus::default();
        context.pending_count = 0;
        context.current_steps = 0;
        context.query_member_count = 0;
        context.greedy_entry_quota_limit = 0;
        compiled.execute(&mut context);
        assert_eq!(status.error, 2);
        assert_eq!(status.result_count, 0);
    }

    #[test]
    fn cranelift_following_keeps_one_lazy_continuation_for_all_and_named_siblings() {
        use crate::code_rule_dsl::transducer::packed::metal_batch_abi::PENDING_FOLLOWING;
        use crate::code_rule_dsl::transducer::packed::{FOLLOWING, NAMED};

        fn run(mode: u8, expected: u32, expected_last: u32) {
            let program = Program::new(
                vec![word(FOLLOWING, mode, 0), word(EMIT, 0, 0)],
                vec![b"match".to_vec()],
                Vec::new(),
                vec![0],
                Vec::new(),
            )
            .unwrap();
            let mut native = native_ir::lower(&program).unwrap();
            native.tree_sitter_query = true;
            native.has_repeats = true;
            let compiled = compile_straight_line(&native).unwrap();
            let mut nodes = [MetalNode {
                parent: NONE,
                next_any: NONE,
                next_named: NONE,
                ..MetalNode::default()
            }; 4];
            nodes[2].source_start = 4;
            nodes[2].source_end = 4;
            match (mode, expected) {
                (_, 0) => {}
                (0, 1) => nodes[0].next_any = 3,
                (0, 3) => {
                    nodes[0].next_any = 1;
                    nodes[1].next_any = 2;
                    nodes[2].next_any = 3;
                }
                (NAMED, 2) => {
                    nodes[0].next_named = 2;
                    nodes[2].next_named = 3;
                }
                _ => panic!("unsupported following test shape"),
            }
            let mut pending = [PendingRecord::default(); 1];
            let mut query_buckets = [QueryBucketRecord::default(); 8];
            let mut query_members = [QueryMemberRecord::default(); 4];
            let mut queued_states = [QueuedStateRecord::default(); 1];
            let mut results = [ResultRecord::default(); 3];
            let mut status = TaskStatus::default();
            let mut context = NativeTaskContext {
                nodes: nodes.as_ptr(),
                node_count: nodes.len() as u32,
                children: std::ptr::null(),
                child_count: 0,
                supertypes: std::ptr::null(),
                supertype_count: 0,
                source: std::ptr::null(),
                source_length: 0,
                values: std::ptr::null(),
                value_count: 0,
                nfa_states: std::ptr::null(),
                nfa_state_count: 0,
                nfa_data: std::ptr::null(),
                nfa_data_count: 0,
                nfa_ranges: std::ptr::null(),
                nfa_range_count: 0,
                lua_instructions: std::ptr::null(),
                lua_instruction_count: 0,
                lua_members: std::ptr::null(),
                lua_member_count: 0,
                value_work: std::ptr::null_mut(),
                value_start: 0,
                value_capacity: 0,
                current_node: 0,
                start_node: 0,
                task_ordinal: 0,
                pending: pending.as_mut_ptr(),
                pending_count: 0,
                pending_quota_limit: pending.len() as u32,
                current_steps: 0,
                current_captures: 0,
                current_cursors: 0,
                current_repeats: 0,
                current_following: 0,
                current_capture_hash: 0,
                current_cursor_hash: 0,
                current_repeat_hash: 0,
                task_work_limit: 16,
                path_work_limit: 16,
                captures: std::ptr::null(),
                cursors: std::ptr::null(),
                repeats: std::ptr::null(),
                query_buckets: query_buckets.as_mut_ptr(),
                query_bucket_capacity: query_buckets.len() as u32,
                query_members: query_members.as_mut_ptr(),
                query_member_count: 0,
                query_member_quota_limit: query_members.len() as u32,
                queued_states: queued_states.as_mut_ptr(),
                queued_state_quota_limit: queued_states.len() as u32,
                greedy_entries: std::ptr::null_mut(),
                greedy_entry_quota_limit: 0,
                maximum_cursor_slot: 0,
                capture_count: 0,
                capture_quota_limit: 0,
                cursor_count: 0,
                cursor_quota_limit: 0,
                repeat_count: 0,
                repeat_quota_limit: 0,
                results: results.as_mut_ptr(),
                result_quota_limit: results.len() as u32,
                line_starts: std::ptr::null(),
                line_count: 0,
                final_captures: std::ptr::null_mut(),
                final_capture_capacity: 0,
                properties: std::ptr::null_mut(),
                property_quota_limit: 0,
                output_bytes: std::ptr::null_mut(),
                output_byte_quota_limit: 0,
                status: &mut status,
            };
            compiled.execute(&mut context);
            assert_eq!(status.error, 0);
            assert_eq!(status.result_count, u32::from(expected > 0));
            assert_eq!(context.current_node, expected_last);
            assert_eq!(context.pending_count, 0);
            assert_eq!(context.query_member_count, expected + 1);
            if expected > 1 {
                assert_eq!(pending[0].kind, PENDING_FOLLOWING);
                assert_eq!(queued_states[0].occupied, 2);
            }
        }

        run(0, 0, 0);
        run(0, 1, 3);
        run(0, 3, 3);
        run(NAMED, 2, 3);
    }

    #[test]
    fn cranelift_query_admission_suppresses_an_exact_reentered_state() {
        use crate::code_rule_dsl::transducer::packed::{FORK, GOTO};

        let program = Program::new(
            vec![
                word(FORK, 0, 3),
                word(FORK, 0, 3),
                word(GOTO, 0, 3),
                word(EMIT, 0, 0),
            ],
            vec![b"match".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let mut native = native_ir::lower(&program).unwrap();
        native.tree_sitter_query = true;
        native.has_repeats = true;
        let compiled = compile_straight_line(&native).unwrap();
        let nodes = [MetalNode::default()];
        let mut pending = [PendingRecord::default(); 1];
        let mut query_buckets = [QueryBucketRecord::default(); 8];
        let mut query_members = [QueryMemberRecord::default(); 4];
        let mut queued_states = [QueuedStateRecord::default(); 1];
        let mut results = [ResultRecord::default(); 2];
        let mut status = TaskStatus::default();
        let mut context = NativeTaskContext {
            nodes: nodes.as_ptr(),
            node_count: 1,
            children: std::ptr::null(),
            child_count: 0,
            supertypes: std::ptr::null(),
            supertype_count: 0,
            source: std::ptr::null(),
            source_length: 0,
            values: std::ptr::null(),
            value_count: 0,
            nfa_states: std::ptr::null(),
            nfa_state_count: 0,
            nfa_data: std::ptr::null(),
            nfa_data_count: 0,
            nfa_ranges: std::ptr::null(),
            nfa_range_count: 0,
            lua_instructions: std::ptr::null(),
            lua_instruction_count: 0,
            lua_members: std::ptr::null(),
            lua_member_count: 0,
            value_work: std::ptr::null_mut(),
            value_start: 0,
            value_capacity: 0,
            current_node: 0,
            start_node: 0,
            task_ordinal: 0,
            pending: pending.as_mut_ptr(),
            pending_count: 0,
            pending_quota_limit: 1,
            current_steps: 0,
            current_captures: 0,
            current_cursors: 0,
            current_repeats: 0,
            current_following: 0,
            current_capture_hash: 0,
            current_cursor_hash: 0,
            current_repeat_hash: 0,
            task_work_limit: 64,
            path_work_limit: 64,
            captures: std::ptr::null(),
            cursors: std::ptr::null(),
            repeats: std::ptr::null(),
            query_buckets: query_buckets.as_mut_ptr(),
            query_bucket_capacity: query_buckets.len() as u32,
            query_members: query_members.as_mut_ptr(),
            query_member_count: 0,
            query_member_quota_limit: query_members.len() as u32,
            queued_states: queued_states.as_mut_ptr(),
            queued_state_quota_limit: queued_states.len() as u32,
            greedy_entries: std::ptr::null_mut(),
            greedy_entry_quota_limit: 0,
            maximum_cursor_slot: 0,
            capture_count: 0,
            capture_quota_limit: 0,
            cursor_count: 0,
            cursor_quota_limit: 0,
            repeat_count: 0,
            repeat_quota_limit: 0,
            results: results.as_mut_ptr(),
            result_quota_limit: results.len() as u32,
            line_starts: std::ptr::null(),
            line_count: 0,
            final_captures: std::ptr::null_mut(),
            final_capture_capacity: 0,
            properties: std::ptr::null_mut(),
            property_quota_limit: 0,
            output_bytes: std::ptr::null_mut(),
            output_byte_quota_limit: 0,
            status: &mut status,
        };
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 1);
        assert_eq!(context.pending_count, 0);
        assert_eq!(context.query_member_count, 4);
        assert_eq!(queued_states[0].occupied, 2);
    }

    #[test]
    fn cranelift_scheduler_reports_work_and_pending_capacity_exactly() {
        use crate::code_rule_dsl::transducer::packed::FORK;

        let program = Program::new(
            vec![word(FORK, 0, 1), word(EMIT, 0, 0)],
            vec![b"match".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let native = native_ir::lower(&program).unwrap();
        let compiled = compile_straight_line(&native).unwrap();
        let nodes = [MetalNode::default()];
        let mut results = [ResultRecord::default(); 2];
        let mut status = TaskStatus::default();
        let mut context = NativeTaskContext {
            nodes: nodes.as_ptr(),
            node_count: 1,
            children: std::ptr::null(),
            child_count: 0,
            supertypes: std::ptr::null(),
            supertype_count: 0,
            source: std::ptr::null(),
            source_length: 0,
            values: std::ptr::null(),
            value_count: 0,
            nfa_states: std::ptr::null(),
            nfa_state_count: 0,
            nfa_data: std::ptr::null(),
            nfa_data_count: 0,
            nfa_ranges: std::ptr::null(),
            nfa_range_count: 0,
            lua_instructions: std::ptr::null(),
            lua_instruction_count: 0,
            lua_members: std::ptr::null(),
            lua_member_count: 0,
            value_work: std::ptr::null_mut(),
            value_start: 0,
            value_capacity: 0,
            current_node: 0,
            start_node: 0,
            task_ordinal: 0,
            pending: std::ptr::null_mut(),
            pending_count: 0,
            pending_quota_limit: 0,
            current_steps: 0,
            current_captures: 0,
            current_cursors: 0,
            current_repeats: 0,
            current_following: 0,
            current_capture_hash: 0,
            current_cursor_hash: 0,
            current_repeat_hash: 0,
            task_work_limit: 16,
            path_work_limit: 16,
            captures: std::ptr::null(),
            cursors: std::ptr::null(),
            repeats: std::ptr::null(),
            query_buckets: std::ptr::null_mut(),
            query_bucket_capacity: 0,
            query_members: std::ptr::null_mut(),
            query_member_count: 0,
            query_member_quota_limit: 0,
            queued_states: std::ptr::null_mut(),
            queued_state_quota_limit: 0,
            greedy_entries: std::ptr::null_mut(),
            greedy_entry_quota_limit: 0,
            maximum_cursor_slot: 0,
            capture_count: 0,
            capture_quota_limit: 0,
            cursor_count: 0,
            cursor_quota_limit: 0,
            repeat_count: 0,
            repeat_quota_limit: 0,
            results: results.as_mut_ptr(),
            result_quota_limit: 2,
            line_starts: std::ptr::null(),
            line_count: 0,
            final_captures: std::ptr::null_mut(),
            final_capture_capacity: 0,
            properties: std::ptr::null_mut(),
            property_quota_limit: 0,
            output_bytes: std::ptr::null_mut(),
            output_byte_quota_limit: 0,
            status: &mut status,
        };
        compiled.execute(&mut context);
        assert_eq!(status.error, 2);
        assert_eq!(status.result_count, 0);

        let mut pending = [PendingRecord::default(); 1];
        status = TaskStatus::default();
        context.pending = pending.as_mut_ptr();
        context.pending_quota_limit = 1;
        context.task_work_limit = 0;
        context.path_work_limit = 0;
        context.status = &mut status;
        compiled.execute(&mut context);
        assert_eq!(status.error, 4);
        assert_eq!(status.result_count, 0);

        status = TaskStatus::default();
        context.current_steps = u32::MAX;
        context.task_work_limit = u32::MAX;
        context.path_work_limit = u32::MAX;
        context.status = &mut status;
        compiled.execute(&mut context);
        assert_eq!(status.error, 4);
        assert_eq!(status.result_count, 0);
    }

    #[test]
    fn scheduler_error_invalidates_results_written_by_an_earlier_path() {
        use crate::code_rule_dsl::transducer::packed::FORK;

        let program = Program::new(
            vec![word(FORK, 0, 2), word(EMIT, 0, 0), word(EMIT, 0, 0)],
            vec![b"match".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let native = native_ir::lower(&program).unwrap();
        let compiled = compile_straight_line(&native).unwrap();
        let nodes = [MetalNode::default()];
        let mut pending = [PendingRecord::default(); 1];
        let mut results = [ResultRecord::default(); 1];
        let mut status = TaskStatus::default();
        let mut context = NativeTaskContext {
            nodes: nodes.as_ptr(),
            node_count: 1,
            children: std::ptr::null(),
            child_count: 0,
            supertypes: std::ptr::null(),
            supertype_count: 0,
            source: std::ptr::null(),
            source_length: 0,
            values: std::ptr::null(),
            value_count: 0,
            nfa_states: std::ptr::null(),
            nfa_state_count: 0,
            nfa_data: std::ptr::null(),
            nfa_data_count: 0,
            nfa_ranges: std::ptr::null(),
            nfa_range_count: 0,
            lua_instructions: std::ptr::null(),
            lua_instruction_count: 0,
            lua_members: std::ptr::null(),
            lua_member_count: 0,
            value_work: std::ptr::null_mut(),
            value_start: 0,
            value_capacity: 0,
            current_node: 0,
            start_node: 0,
            task_ordinal: 0,
            pending: pending.as_mut_ptr(),
            pending_count: 0,
            pending_quota_limit: 1,
            current_steps: 0,
            current_captures: 0,
            current_cursors: 0,
            current_repeats: 0,
            current_following: 0,
            current_capture_hash: 0,
            current_cursor_hash: 0,
            current_repeat_hash: 0,
            task_work_limit: 16,
            path_work_limit: 16,
            captures: std::ptr::null(),
            cursors: std::ptr::null(),
            repeats: std::ptr::null(),
            query_buckets: std::ptr::null_mut(),
            query_bucket_capacity: 0,
            query_members: std::ptr::null_mut(),
            query_member_count: 0,
            query_member_quota_limit: 0,
            queued_states: std::ptr::null_mut(),
            queued_state_quota_limit: 0,
            greedy_entries: std::ptr::null_mut(),
            greedy_entry_quota_limit: 0,
            maximum_cursor_slot: 0,
            capture_count: 0,
            capture_quota_limit: 0,
            cursor_count: 0,
            cursor_quota_limit: 0,
            repeat_count: 0,
            repeat_quota_limit: 0,
            results: results.as_mut_ptr(),
            result_quota_limit: 1,
            line_starts: std::ptr::null(),
            line_count: 0,
            final_captures: std::ptr::null_mut(),
            final_capture_capacity: 0,
            properties: std::ptr::null_mut(),
            property_quota_limit: 0,
            output_bytes: std::ptr::null_mut(),
            output_byte_quota_limit: 0,
            status: &mut status,
        };
        compiled.execute(&mut context);
        assert_eq!(status.error, 2);
        assert_eq!(status.result_count, 0);
        assert_eq!(status.capture_count, 0);
        assert_eq!(status.final_capture_count, 0);
        assert_eq!(status.property_count, 0);
        assert_eq!(status.output_byte_count, 0);
    }

    #[test]
    fn cranelift_scheduler_relations_compare_complete_packed_states() {
        let program = Program::new(
            vec![word(EMIT, 0, 0)],
            vec![b"match".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let native = native_ir::lower(&program).unwrap();
        let compiled = compile_straight_line(&native).unwrap();
        let nodes = [
            MetalNode {
                source_start: 0,
                source_end: 1,
                ..MetalNode::default()
            },
            MetalNode {
                source_start: 2,
                source_end: 2,
                ..MetalNode::default()
            },
        ];
        let captures = [
            LinkRecord {
                previous: 0,
                first: 7,
                second: 0,
            },
            LinkRecord {
                previous: 1,
                first: 7,
                second: 1,
            },
            LinkRecord {
                previous: 1,
                first: 7,
                second: 1,
            },
            LinkRecord {
                previous: 1,
                first: 8,
                second: 1,
            },
        ];
        let cursors = [
            LinkRecord {
                previous: 0,
                first: 0,
                second: 5,
            },
            LinkRecord {
                previous: 1,
                first: 1,
                second: 7,
            },
            LinkRecord {
                previous: 1,
                first: 1,
                second: 7,
            },
            LinkRecord {
                previous: 1,
                first: 1,
                second: 8,
            },
        ];
        let repeats = [
            RepeatRecord {
                previous: 0,
                value: 2,
                previous_hash: 0,
            },
            RepeatRecord {
                previous: 1,
                value: 3,
                previous_hash: 0,
            },
            RepeatRecord {
                previous: 1,
                value: 3,
                previous_hash: 0,
            },
            RepeatRecord {
                previous: 1,
                value: 4,
                previous_hash: 0,
            },
        ];
        let mut query_buckets = [QueryBucketRecord::default(); 4];
        let mut query_members = [QueryMemberRecord::default(); 2];
        let mut queued_states = [QueuedStateRecord::default(); 2];
        let mut greedy_entries = [GreedyEntryRecord::default(); 1];
        let mut status = TaskStatus::default();
        let mut context = NativeTaskContext {
            nodes: nodes.as_ptr(),
            node_count: nodes.len() as u32,
            children: std::ptr::null(),
            child_count: 0,
            supertypes: std::ptr::null(),
            supertype_count: 0,
            source: std::ptr::null(),
            source_length: 0,
            values: std::ptr::null(),
            value_count: 0,
            nfa_states: std::ptr::null(),
            nfa_state_count: 0,
            nfa_data: std::ptr::null(),
            nfa_data_count: 0,
            nfa_ranges: std::ptr::null(),
            nfa_range_count: 0,
            lua_instructions: std::ptr::null(),
            lua_instruction_count: 0,
            lua_members: std::ptr::null(),
            lua_member_count: 0,
            value_work: std::ptr::null_mut(),
            value_start: 0,
            value_capacity: 0,
            current_node: 0,
            start_node: 0,
            task_ordinal: 0,
            pending: std::ptr::null_mut(),
            pending_count: 0,
            pending_quota_limit: 0,
            current_steps: 0,
            current_captures: 0,
            current_cursors: 0,
            current_repeats: 0,
            current_following: 0,
            current_capture_hash: 0,
            current_cursor_hash: 0,
            current_repeat_hash: 0,
            task_work_limit: 16,
            path_work_limit: 16,
            captures: captures.as_ptr(),
            cursors: cursors.as_ptr(),
            repeats: repeats.as_ptr(),
            query_buckets: query_buckets.as_mut_ptr(),
            query_bucket_capacity: query_buckets.len() as u32,
            query_members: query_members.as_mut_ptr(),
            query_member_count: 0,
            query_member_quota_limit: query_members.len() as u32,
            queued_states: queued_states.as_mut_ptr(),
            queued_state_quota_limit: queued_states.len() as u32,
            greedy_entries: greedy_entries.as_mut_ptr(),
            greedy_entry_quota_limit: greedy_entries.len() as u32,
            maximum_cursor_slot: 2,
            capture_count: 0,
            capture_quota_limit: 0,
            cursor_count: 0,
            cursor_quota_limit: 0,
            repeat_count: 0,
            repeat_quota_limit: 0,
            results: std::ptr::null_mut(),
            result_quota_limit: 0,
            line_starts: std::ptr::null(),
            line_count: 0,
            final_captures: std::ptr::null_mut(),
            final_capture_capacity: 0,
            properties: std::ptr::null_mut(),
            property_quota_limit: 0,
            output_bytes: std::ptr::null_mut(),
            output_byte_quota_limit: 0,
            status: &mut status,
        };
        unsafe {
            assert_eq!((compiled.cursor_equal)(&mut context, 2, 3), 1);
            assert_eq!((compiled.cursor_equal)(&mut context, 2, 4), 0);
            assert_eq!((compiled.repeat_equal)(&mut context, 2, 3), 1);
            assert_eq!((compiled.repeat_equal)(&mut context, 2, 4), 0);
            assert_eq!((compiled.capture_contains)(&mut context, 2, 1), 1);
            assert_eq!((compiled.capture_contains)(&mut context, 1, 2), 0);
            assert_eq!((compiled.capture_contains)(&mut context, 2, 3), 1);
            assert_eq!((compiled.capture_contains)(&mut context, 2, 4), 0);
            assert_eq!(
                (compiled.exact_state_equal)(&mut context, 2, 2, 2, 3, 3, 3),
                1
            );
            assert_eq!(
                (compiled.exact_state_equal)(&mut context, 2, 2, 2, 4, 4, 4),
                0
            );
            context.current_captures = 1;
            context.current_cursors = 2;
            context.current_repeats = 2;
            context.current_capture_hash = 11;
            context.current_cursor_hash = 12;
            context.current_repeat_hash = 13;
            assert_eq!((compiled.queue_insert)(&mut context, 7, 0), 1);
            assert_eq!((compiled.queue_insert)(&mut context, 7, 0), 0);
            context.current_captures = 2;
            assert_eq!((compiled.queue_insert)(&mut context, 7, 0), 1);
            context.current_captures = 4;
            assert_eq!((compiled.queue_insert)(&mut context, 7, 0), 2);
            context.current_captures = 1;
            assert_eq!((compiled.query_admit)(&mut context, 5, 0), 1);
            assert_eq!((compiled.query_admit)(&mut context, 5, 0), 0);
            context.current_captures = 2;
            assert_eq!((compiled.query_admit)(&mut context, 5, 0), 1);
            context.current_captures = 4;
            // All three capture sequences are distinct. The third cannot replace
            // either retained state to conceal exhaustion of this two-slot arena.
            assert_eq!((compiled.query_admit)(&mut context, 5, 0), 2);
            context.current_captures = 2;
            assert_eq!((compiled.query_admit)(&mut context, 5, 0), 0);
            context.current_captures = 1;
            context.current_node = 0;
            let inserted = (compiled.greedy_schedule)(&mut context, 5);
            assert_eq!(inserted >> 32, 1);
            let slot = inserted as u32;
            context.current_node = 1;
            let replaced = (compiled.greedy_schedule)(&mut context, 5);
            assert_eq!(replaced >> 32, 0);
            assert_eq!(replaced as u32, slot);
            context.current_node = 0;
            assert_eq!((compiled.greedy_take)(&mut context, slot), 5);
            assert_eq!(context.current_node, 1);
            assert_eq!((compiled.greedy_take)(&mut context, slot), u64::MAX);
            assert_eq!(greedy_entries[slot as usize].occupied, 2);
            let reused = (compiled.greedy_schedule)(&mut context, 6);
            assert_eq!(reused >> 32, 1);
            assert_eq!(reused as u32, slot);
            assert_eq!((compiled.greedy_schedule)(&mut context, 7), u64::MAX);
            context.greedy_entry_quota_limit = 0;
            assert_eq!((compiled.greedy_schedule)(&mut context, 5), u64::MAX);
        }
        assert_eq!(context.query_member_count, 2);
        assert_eq!(
            query_members.map(|member| (member.captures, member.capture_count)),
            [(1, 1), (2, 2)]
        );

        // Compare all short histories over two identities at equal geometry and
        // a later zero-width node. Histories can arrive in arbitrary geometry order.
        let nodes = [
            MetalNode {
                source_start: 0,
                source_end: 6,
                ..MetalNode::default()
            },
            MetalNode {
                source_start: 0,
                source_end: 6,
                ..MetalNode::default()
            },
            MetalNode {
                source_start: 9,
                source_end: 9,
                ..MetalNode::default()
            },
        ];
        let alphabet = [(0_u16, 0_u32), (1, 0), (0, 1), (0, 2)];
        let mut sequences = vec![vec![]];
        let mut level = vec![vec![]];
        for _ in 0..3 {
            level = level
                .iter()
                .flat_map(|prefix| {
                    alphabet.iter().map(move |&item| {
                        let mut sequence = prefix.clone();
                        sequence.push(item);
                        sequence
                    })
                })
                .collect();
            sequences.extend(level.clone());
        }
        let mut links = Vec::new();
        let mut heads = Vec::new();
        let ordered = sequences
            .iter()
            .map(|sequence| {
                let mut head = 0;
                let mut ordered = Vec::new();
                for &(label, node) in sequence {
                    links.push(LinkRecord {
                        previous: head,
                        first: u32::from(label),
                        second: node,
                    });
                    head = links.len() as u32;
                    let value = nodes[node as usize];
                    ordered.push(super::super::CaptureIdentity {
                        label,
                        node,
                        start: value.source_start as usize,
                        end: value.source_end as usize,
                    });
                }
                heads.push(head);
                ordered.sort_by(|a, b| super::super::compare_capture_nodes(*a, *b));
                ordered
            })
            .collect::<Vec<_>>();
        context.nodes = nodes.as_ptr();
        context.node_count = nodes.len() as u32;
        context.captures = links.as_ptr();
        for (left, captures) in ordered.iter().enumerate() {
            for (right, other) in ordered.iter().enumerate() {
                let (contains, reverse) =
                    super::super::capture_identity_containment(captures, other);
                unsafe {
                    assert_eq!(
                        (compiled.capture_contains)(&mut context, heads[left], heads[right]),
                        u32::from(contains),
                        "left={captures:?}, right={other:?}"
                    );
                    assert_eq!(
                        (compiled.exact_state_equal)(
                            &mut context,
                            heads[left],
                            0,
                            0,
                            heads[right],
                            0,
                            0
                        ),
                        u32::from(contains && reverse),
                        "left={captures:?}, right={other:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn cranelift_scheduler_capacity_matrix_has_exact_boundaries() {
        use crate::code_rule_dsl::transducer::packed::{FORK, KIND};

        #[derive(Clone, Copy)]
        struct Capacities {
            pending: u32,
            buckets: u32,
            members: u32,
            queued: u32,
            greedy: u32,
            work: u32,
        }

        fn run(program: &Program, query: bool, capacities: Capacities) -> TaskStatus {
            let mut native = native_ir::lower(program).unwrap();
            native.tree_sitter_query = query;
            native.has_repeats = query;
            let compiled = compile_straight_line(&native).unwrap();
            let nodes = [MetalNode {
                parent: NONE,
                next_any: NONE,
                next_named: NONE,
                kind: 0,
                ..MetalNode::default()
            }];
            let mut pending = [PendingRecord::default(); 4];
            let mut buckets = [QueryBucketRecord::default(); 16];
            let mut members = [QueryMemberRecord::default(); 32];
            let mut queued = [QueuedStateRecord::default(); 4];
            let mut greedy = [GreedyEntryRecord::default(); 4];
            let mut results = [ResultRecord::default(); 8];
            let mut status = TaskStatus::default();
            let mut context = NativeTaskContext {
                nodes: nodes.as_ptr(),
                node_count: 1,
                children: std::ptr::null(),
                child_count: 0,
                supertypes: std::ptr::null(),
                supertype_count: 0,
                source: std::ptr::null(),
                source_length: 0,
                values: std::ptr::null(),
                value_count: 0,
                nfa_states: std::ptr::null(),
                nfa_state_count: 0,
                nfa_data: std::ptr::null(),
                nfa_data_count: 0,
                nfa_ranges: std::ptr::null(),
                nfa_range_count: 0,
                lua_instructions: std::ptr::null(),
                lua_instruction_count: 0,
                lua_members: std::ptr::null(),
                lua_member_count: 0,
                value_work: std::ptr::null_mut(),
                value_start: 0,
                value_capacity: 0,
                current_node: 0,
                start_node: 0,
                task_ordinal: 0,
                pending: pending.as_mut_ptr(),
                pending_count: 0,
                pending_quota_limit: capacities.pending,
                current_steps: 0,
                current_captures: 0,
                current_cursors: 0,
                current_repeats: 0,
                current_following: 0,
                current_capture_hash: 0,
                current_cursor_hash: 0,
                current_repeat_hash: 0,
                task_work_limit: capacities.work,
                path_work_limit: capacities.work,
                captures: std::ptr::null(),
                cursors: std::ptr::null(),
                repeats: std::ptr::null(),
                query_buckets: buckets.as_mut_ptr(),
                query_bucket_capacity: capacities.buckets,
                query_members: members.as_mut_ptr(),
                query_member_count: 0,
                query_member_quota_limit: capacities.members,
                queued_states: queued.as_mut_ptr(),
                queued_state_quota_limit: capacities.queued,
                greedy_entries: greedy.as_mut_ptr(),
                greedy_entry_quota_limit: capacities.greedy,
                maximum_cursor_slot: 0,
                capture_count: 0,
                capture_quota_limit: 0,
                cursor_count: 0,
                cursor_quota_limit: 0,
                repeat_count: 0,
                repeat_quota_limit: 0,
                results: results.as_mut_ptr(),
                result_quota_limit: results.len() as u32,
                line_starts: std::ptr::null(),
                line_count: 0,
                final_captures: std::ptr::null_mut(),
                final_capture_capacity: 0,
                properties: std::ptr::null_mut(),
                property_quota_limit: 0,
                output_bytes: std::ptr::null_mut(),
                output_byte_quota_limit: 0,
                status: &mut status,
            };
            compiled.execute(&mut context);
            status
        }

        let normal_forks = Program::new(
            vec![
                word(FORK, 0, 4),
                word(FORK, 0, 3),
                word(EMIT, 0, 0),
                word(EMIT, 0, 0),
                word(EMIT, 0, 0),
            ],
            vec![b"match".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let greedy_forks = Program::new(
            vec![
                word(FORK, 1, 4),
                word(FORK, 1, 3),
                word(EMIT, 0, 0),
                word(EMIT, 0, 0),
                word(EMIT, 0, 0),
            ],
            vec![b"match".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let query = Program::new(
            vec![word(KIND, 0, 0), word(EMIT, 0, 0)],
            vec![b"match".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let ample = Capacities {
            pending: 4,
            buckets: 16,
            members: 32,
            queued: 4,
            greedy: 4,
            work: 64,
        };
        for (capacity, succeeds) in [(0, false), (2, true), (1, false), (3, true)] {
            let status = run(
                &normal_forks,
                false,
                Capacities {
                    pending: capacity,
                    ..ample
                },
            );
            assert_eq!(status.error == 0, succeeds, "pending capacity {capacity}");
            let status = run(
                &query,
                false,
                Capacities {
                    work: capacity,
                    ..ample
                },
            );
            assert_eq!(status.error == 0, succeeds, "work limit {capacity}");
        }
        for (capacity, succeeds) in [(0, false), (2, true), (1, false), (3, true)] {
            let status = run(
                &query,
                true,
                Capacities {
                    buckets: capacity,
                    ..ample
                },
            );
            assert_eq!(status.error == 0, succeeds, "bucket capacity {capacity}");
            if capacity == 0 {
                assert_eq!(status.program_counter, 0);
            }
            let status = run(
                &query,
                true,
                Capacities {
                    members: capacity,
                    ..ample
                },
            );
            assert_eq!(status.error == 0, succeeds, "member capacity {capacity}");
        }
        for (capacity, succeeds) in [(0, false), (2, true), (1, false), (3, true)] {
            let status = run(
                &normal_forks,
                true,
                Capacities {
                    queued: capacity,
                    ..ample
                },
            );
            assert_eq!(status.error == 0, succeeds, "queued capacity {capacity}");
            let status = run(
                &greedy_forks,
                false,
                Capacities {
                    greedy: capacity,
                    ..ample
                },
            );
            assert_eq!(status.error == 0, succeeds, "greedy capacity {capacity}");
        }
    }

    #[test]
    fn cranelift_lowers_capture_cursor_and_repeat_link_creation() {
        use crate::code_rule_dsl::transducer::packed::{
            CAPTURE, CHILD, REPEAT_BEGIN, REPEAT_BUMP, REPEAT_END, RESTORE_CURSOR, SAVE_CURSOR,
        };

        let program = Program::new(
            vec![
                word(SAVE_CURSOR, 0, 0),
                word(CHILD, 0, 0),
                word(CAPTURE, 0, 0),
                word(REPEAT_BEGIN, 0, 3),
                word(REPEAT_BUMP, 0, 3),
                word(REPEAT_END, 0, 3),
                word(RESTORE_CURSOR, 0, 0),
                word(EMIT, 0, 0),
            ],
            vec![b"match".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let native = native_ir::lower(&program).unwrap();
        let compiled = compile_straight_line(&native).unwrap();
        let mut nodes = [MetalNode {
            parent: NONE,
            next_any: NONE,
            next_named: NONE,
            descendant_end: 3,
            ..MetalNode::default()
        }; 3];
        nodes[0].child_count = 2;
        nodes[1].parent = 0;
        nodes[2].parent = 0;
        let children = [1_u32, 2];
        let mut pending = [PendingRecord::default(); 1];
        let mut captures = [LinkRecord::default(); 2];
        let mut cursors = [LinkRecord::default(); 1];
        let mut repeats = [RepeatRecord::default(); 2];
        let mut final_captures = [FinalCaptureRecord::default(); 2];
        let mut results = [ResultRecord::default(); 2];
        let mut status = TaskStatus::default();
        let mut context = NativeTaskContext {
            nodes: nodes.as_ptr(),
            node_count: nodes.len() as u32,
            children: children.as_ptr(),
            child_count: children.len() as u32,
            supertypes: std::ptr::null(),
            supertype_count: 0,
            source: std::ptr::null(),
            source_length: 0,
            values: std::ptr::null(),
            value_count: 0,
            nfa_states: std::ptr::null(),
            nfa_state_count: 0,
            nfa_data: std::ptr::null(),
            nfa_data_count: 0,
            nfa_ranges: std::ptr::null(),
            nfa_range_count: 0,
            lua_instructions: std::ptr::null(),
            lua_instruction_count: 0,
            lua_members: std::ptr::null(),
            lua_member_count: 0,
            value_work: std::ptr::null_mut(),
            value_start: 0,
            value_capacity: 0,
            current_node: 0,
            start_node: 0,
            task_ordinal: 0,
            pending: pending.as_mut_ptr(),
            pending_count: 0,
            pending_quota_limit: pending.len() as u32,
            current_steps: 0,
            current_captures: 0,
            current_cursors: 0,
            current_repeats: 0,
            current_following: 0,
            current_capture_hash: 0,
            current_cursor_hash: 0,
            current_repeat_hash: 0,
            task_work_limit: 64,
            path_work_limit: 64,
            captures: captures.as_mut_ptr(),
            cursors: cursors.as_mut_ptr(),
            repeats: repeats.as_mut_ptr(),
            query_buckets: std::ptr::null_mut(),
            query_bucket_capacity: 0,
            query_members: std::ptr::null_mut(),
            query_member_count: 0,
            query_member_quota_limit: 0,
            queued_states: std::ptr::null_mut(),
            queued_state_quota_limit: 0,
            greedy_entries: std::ptr::null_mut(),
            greedy_entry_quota_limit: 0,
            maximum_cursor_slot: 1,
            capture_count: 0,
            capture_quota_limit: captures.len() as u32,
            cursor_count: 0,
            cursor_quota_limit: cursors.len() as u32,
            repeat_count: 0,
            repeat_quota_limit: repeats.len() as u32,
            results: results.as_mut_ptr(),
            result_quota_limit: results.len() as u32,
            line_starts: std::ptr::null(),
            line_count: 0,
            final_captures: final_captures.as_mut_ptr(),
            final_capture_capacity: final_captures.len() as u32,
            properties: std::ptr::null_mut(),
            property_quota_limit: 0,
            output_bytes: std::ptr::null_mut(),
            output_byte_quota_limit: 0,
            status: &mut status,
        };
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 2);
        assert_eq!(status.capture_count, 2);
        assert_eq!(status.final_capture_count, 2);
        assert_eq!(status.property_count, 0);
        assert_eq!(context.capture_count, 2);
        assert_eq!(context.cursor_count, 1);
        assert_eq!(context.repeat_count, 2);
        assert_eq!([captures[0].second, captures[1].second], [1, 2]);
        assert_eq!([results[0].capture_start, results[1].capture_start], [0, 1]);
        assert_eq!([results[0].capture_count, results[1].capture_count], [1, 1]);
        assert_eq!(
            cursors[0],
            LinkRecord {
                previous: 0,
                first: 0,
                second: 0
            }
        );
        assert_eq!([repeats[0].value, repeats[1].value], [3, 3]);
        assert_eq!(context.current_node, 0);
        let mix = |hash: u32, value: u32| {
            hash ^ value
                .wrapping_add(0x9e37_79b9)
                .wrapping_add(hash << 6)
                .wrapping_add(hash >> 2)
        };
        assert_eq!(context.current_capture_hash, mix(0, mix(0, 2)));
        assert_eq!(context.current_cursor_hash, mix(0, NONE) ^ mix(0, 0));
        assert_eq!(context.current_repeat_hash, 0);

        status = TaskStatus::default();
        pending.fill(PendingRecord::default());
        context.current_node = 0;
        context.pending_count = 0;
        context.current_steps = 0;
        context.current_captures = 0;
        context.current_cursors = 0;
        context.current_repeats = 0;
        context.current_capture_hash = 0;
        context.current_cursor_hash = 0;
        context.current_repeat_hash = 0;
        context.capture_count = 0;
        context.cursor_count = 0;
        context.repeat_count = 0;
        context.capture_quota_limit = 1;
        compiled.execute(&mut context);
        assert_eq!(status.error, 2);
        assert_eq!(status.result_count, 0);
        assert_eq!(status.capture_count, 0);
        assert_eq!(status.final_capture_count, 0);
        assert_eq!(status.property_count, 0);

        status = TaskStatus::default();
        context.current_node = 0;
        context.pending_count = 0;
        context.current_steps = 0;
        context.current_captures = 0;
        context.current_cursors = 0;
        context.current_repeats = 0;
        context.current_capture_hash = 0;
        context.current_cursor_hash = 0;
        context.current_repeat_hash = 0;
        context.capture_count = 0;
        context.cursor_count = 0;
        context.repeat_count = 0;
        context.capture_quota_limit = 2;
        context.cursor_quota_limit = 0;
        compiled.execute(&mut context);
        assert_eq!(status.error, 2);
        assert_eq!(status.result_count, 0);

        status = TaskStatus::default();
        context.current_node = 0;
        context.pending_count = 0;
        context.current_steps = 0;
        context.current_captures = 0;
        context.current_cursors = 0;
        context.current_repeats = 0;
        context.current_capture_hash = 0;
        context.current_cursor_hash = 0;
        context.current_repeat_hash = 0;
        context.capture_count = 0;
        context.cursor_count = 0;
        context.repeat_count = 0;
        context.cursor_quota_limit = 1;
        context.repeat_quota_limit = 1;
        compiled.execute(&mut context);
        assert_eq!(status.error, 2);
        assert_eq!(status.result_count, 0);
        assert_eq!(status.capture_count, 0);
        assert_eq!(status.final_capture_count, 0);
        assert_eq!(status.property_count, 0);
    }

    #[test]
    fn cranelift_lowers_child_and_descendant_multitarget_navigation() {
        use crate::code_rule_dsl::transducer::packed::{CHILD, DESCEND, NAMED};

        fn run(operation: u8, mode: u8, _expected: u32) {
            let program = Program::new(
                vec![word(operation, mode, 0), word(EMIT, 0, 0)],
                vec![b"match".to_vec()],
                Vec::new(),
                vec![0],
                Vec::new(),
            )
            .unwrap();
            let mut native = native_ir::lower(&program).unwrap();
            native.tree_sitter_query = true;
            native.has_repeats = true;
            let compiled = compile_straight_line(&native).unwrap();
            let mut nodes = [MetalNode {
                parent: NONE,
                next_any: NONE,
                next_named: NONE,
                descendant_end: 4,
                ..MetalNode::default()
            }; 4];
            nodes[0].child_count = 3;
            nodes[1].parent = 0;
            nodes[1].flags = 1;
            nodes[2].parent = 0;
            nodes[2].source_start = 3;
            nodes[2].source_end = 3;
            nodes[3].parent = 0;
            nodes[3].flags = 1;
            let children = [1_u32, 2, 3];
            let mut pending = [PendingRecord::default(); 2];
            let mut query_buckets = [QueryBucketRecord::default(); 8];
            let mut query_members = [QueryMemberRecord::default(); 8];
            let mut queued_states = [QueuedStateRecord::default(); 2];
            let mut results = [ResultRecord::default(); 3];
            let mut status = TaskStatus::default();
            let mut context = NativeTaskContext {
                nodes: nodes.as_ptr(),
                node_count: nodes.len() as u32,
                children: children.as_ptr(),
                child_count: children.len() as u32,
                supertypes: std::ptr::null(),
                supertype_count: 0,
                source: std::ptr::null(),
                source_length: 0,
                values: std::ptr::null(),
                value_count: 0,
                nfa_states: std::ptr::null(),
                nfa_state_count: 0,
                nfa_data: std::ptr::null(),
                nfa_data_count: 0,
                nfa_ranges: std::ptr::null(),
                nfa_range_count: 0,
                lua_instructions: std::ptr::null(),
                lua_instruction_count: 0,
                lua_members: std::ptr::null(),
                lua_member_count: 0,
                value_work: std::ptr::null_mut(),
                value_start: 0,
                value_capacity: 0,
                current_node: 0,
                start_node: 0,
                task_ordinal: 0,
                pending: pending.as_mut_ptr(),
                pending_count: 0,
                pending_quota_limit: pending.len() as u32,
                current_steps: 0,
                current_captures: 0,
                current_cursors: 0,
                current_repeats: 0,
                current_following: 0,
                current_capture_hash: 0,
                current_cursor_hash: 0,
                current_repeat_hash: 0,
                task_work_limit: 32,
                path_work_limit: 32,
                captures: std::ptr::null(),
                cursors: std::ptr::null(),
                repeats: std::ptr::null(),
                query_buckets: query_buckets.as_mut_ptr(),
                query_bucket_capacity: query_buckets.len() as u32,
                query_members: query_members.as_mut_ptr(),
                query_member_count: 0,
                query_member_quota_limit: query_members.len() as u32,
                queued_states: queued_states.as_mut_ptr(),
                queued_state_quota_limit: queued_states.len() as u32,
                greedy_entries: std::ptr::null_mut(),
                greedy_entry_quota_limit: 0,
                maximum_cursor_slot: 0,
                capture_count: 0,
                capture_quota_limit: 0,
                cursor_count: 0,
                cursor_quota_limit: 0,
                repeat_count: 0,
                repeat_quota_limit: 0,
                results: results.as_mut_ptr(),
                result_quota_limit: results.len() as u32,
                line_starts: std::ptr::null(),
                line_count: 0,
                final_captures: std::ptr::null_mut(),
                final_capture_capacity: 0,
                properties: std::ptr::null_mut(),
                property_quota_limit: 0,
                output_bytes: std::ptr::null_mut(),
                output_byte_quota_limit: 0,
                status: &mut status,
            };
            compiled.execute(&mut context);
            assert_eq!(status.error, 0, "operation {operation} mode {mode}");
            assert_eq!(status.result_count, 1, "operation {operation} mode {mode}");
            assert_eq!(context.current_node, 3, "operation {operation} mode {mode}");
            assert_eq!(context.pending_count, 0);
        }

        run(CHILD, 0, 3);
        run(CHILD, NAMED, 2);
        run(DESCEND, 0, 3);
        run(DESCEND, NAMED, 2);
    }

    fn execute_generated_text(
        predicate: crate::code_rule_dsl::transducer::packed::TextPredicate,
        strings: Vec<Vec<u8>>,
        source: &[u8],
        value_capacity: usize,
    ) -> TaskStatus {
        use crate::code_rule_dsl::transducer::packed::TEXT;

        let program = Program::new(
            vec![word(TEXT, 0, 0), word(EMIT, 0, 0)],
            strings,
            vec![predicate],
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let native = native_ir::lower(&program).unwrap();
        let compiled = compile_straight_line(&native).unwrap();
        let nodes = [MetalNode {
            parent: NONE,
            next_any: NONE,
            next_named: NONE,
            source_end: source.len() as u32,
            ..MetalNode::default()
        }];
        let mut work = vec![0_u32; value_capacity];
        let mut results = [ResultRecord::default(); 1];
        let mut status = TaskStatus::default();
        let mut context: NativeTaskContext = unsafe { mem::zeroed() };
        context.nodes = nodes.as_ptr();
        context.node_count = nodes.len() as u32;
        context.source = source.as_ptr();
        context.source_length = source.len() as u32;
        context.current_node = 0;
        context.task_work_limit = 1 << 20;
        context.path_work_limit = 1 << 20;
        context.value_work = work.as_mut_ptr();
        context.value_capacity = work.len() as u32;
        context.results = results.as_mut_ptr();
        context.result_quota_limit = results.len() as u32;
        context.status = &mut status;
        compiled.execute(&mut context);
        status
    }

    #[test]
    fn cranelift_assert_lowers_single_capture_programs() {
        use crate::code_rule_dsl::transducer::packed::{ASSERT, CAPTURE, CapturePredicate, TextPredicate};

        for (predicate, strings, source, expected) in [
            (
                CapturePredicate::Text {
                    capture: 0,
                    predicate: 0,
                    negated: false,
                    any: false,
                },
                vec![b"name".to_vec()],
                b"name".as_slice(),
                1,
            ),
            (
                CapturePredicate::AnyOf {
                    capture: 0,
                    strings: vec![0, 1],
                    negated: false,
                    any: false,
                },
                vec![b"other".to_vec(), b"name".to_vec()],
                b"name".as_slice(),
                1,
            ),
            (
                CapturePredicate::AnyOf {
                    capture: 0,
                    strings: vec![0],
                    negated: true,
                    any: false,
                },
                vec![b"name".to_vec()],
                b"name".as_slice(),
                0,
            ),
            (
                CapturePredicate::AnyOf {
                    capture: 0,
                    strings: vec![0],
                    negated: false,
                    any: false,
                },
                vec![b"longer".to_vec()],
                b"".as_slice(),
                0,
            ),
            (
                CapturePredicate::Kind {
                    capture: 0,
                    kind: 1,
                    negated: true,
                    any: false,
                },
                vec![b"capture".to_vec()],
                b"".as_slice(),
                1,
            ),
            (
                CapturePredicate::Contains {
                    capture: 0,
                    strings: vec![0],
                    negated: false,
                    any: false,
                },
                vec![b"amp".to_vec()],
                b"example".as_slice(),
                1,
            ),
            (
                CapturePredicate::Relation {
                    capture: 0,
                    kinds: vec![1],
                    ancestor: true,
                    negated: true,
                },
                vec![b"capture".to_vec()],
                b"".as_slice(),
                1,
            ),
        ] {
            let text_predicates = if matches!(predicate, CapturePredicate::Text { .. }) {
                vec![TextPredicate::Equal(0)]
            } else {
                Vec::new()
            };
            let program = Program::new_with_capture_predicates(
                vec![word(CAPTURE, 0, 0), word(ASSERT, 0, 0), word(EMIT, 0, 0)],
                strings,
                text_predicates,
                vec![predicate],
                vec![0],
                Vec::new(),
            )
            .unwrap();
            let native = native_ir::lower(&program).unwrap();
            let compiled = compile_straight_line(&native).unwrap();
            let nodes = [MetalNode {
                parent: NONE,
                next_any: NONE,
                next_named: NONE,
                source_end: source.len() as u32,
                ..MetalNode::default()
            }];
            let mut captures = [LinkRecord::default(); 1];
            let mut final_captures = [FinalCaptureRecord::default(); 1];
            let mut results = [ResultRecord::default(); 1];
            let mut status = TaskStatus::default();
            let mut context: NativeTaskContext = unsafe { mem::zeroed() };
            context.nodes = nodes.as_ptr();
            context.node_count = 1;
            context.source = source.as_ptr();
            context.source_length = source.len() as u32;
            context.current_node = 0;
            context.task_work_limit = 1 << 20;
            context.path_work_limit = 1 << 20;
            context.captures = captures.as_mut_ptr();
            context.capture_quota_limit = 1;
            context.results = results.as_mut_ptr();
            context.result_quota_limit = 1;
            context.final_captures = final_captures.as_mut_ptr();
            context.final_capture_capacity = final_captures.len() as u32;
            context.status = &mut status;
            compiled.execute(&mut context);
            assert_eq!(status.error, 0);
            assert_eq!(status.result_count, expected);
        }
    }

    #[test]
    fn cranelift_assert_lowers_paired_capture_programs() {
        use crate::code_rule_dsl::transducer::packed::{ASSERT, CAPTURE, CapturePredicate};

        for predicate in [
            CapturePredicate::Equal {
                left: 0,
                right: 1,
                negated: false,
                any: false,
            },
            CapturePredicate::SameLine {
                left: 0,
                right: 1,
                negated: false,
            },
        ] {
            let program = Program::new_with_capture_predicates(
                vec![
                    word(CAPTURE, 0, 1),
                    word(CAPTURE, 0, 0),
                    word(ASSERT, 0, 0),
                    word(EMIT, 0, 0),
                ],
                vec![b"left".to_vec(), b"right".to_vec()],
                Vec::new(),
                vec![predicate],
                vec![0, 1],
                Vec::new(),
            )
            .unwrap();
            let native = native_ir::lower(&program).unwrap();
            let compiled = compile_straight_line(&native).unwrap();
            let source = b"same";
            let nodes = [MetalNode {
                parent: NONE,
                next_any: NONE,
                next_named: NONE,
                source_end: source.len() as u32,
                ..MetalNode::default()
            }];
            let mut captures = [LinkRecord::default(); 2];
            let mut final_captures = [FinalCaptureRecord::default(); 2];
            let mut results = [ResultRecord::default(); 1];
            let mut status = TaskStatus::default();
            let mut context: NativeTaskContext = unsafe { mem::zeroed() };
            context.nodes = nodes.as_ptr();
            context.node_count = 1;
            context.source = source.as_ptr();
            context.source_length = source.len() as u32;
            context.current_node = 0;
            context.task_work_limit = 1 << 20;
            context.path_work_limit = 1 << 20;
            context.captures = captures.as_mut_ptr();
            context.capture_quota_limit = captures.len() as u32;
            context.results = results.as_mut_ptr();
            context.result_quota_limit = 1;
            context.final_captures = final_captures.as_mut_ptr();
            context.final_capture_capacity = final_captures.len() as u32;
            context.status = &mut status;
            compiled.execute(&mut context);
            assert_eq!(status.error, 0);
            assert_eq!(status.result_count, 1);
            assert_eq!([final_captures[0].label, final_captures[1].label], [0, 1]);
        }
    }

    #[test]
    fn cranelift_output_writes_typed_match_properties() {
        use crate::code_rule_dsl::transducer::packed::Directive;

        let program = Program::new(
            vec![
                word(crate::code_rule_dsl::transducer::packed::CAPTURE, 0, 0),
                word(EMIT, 0, 0),
            ],
            vec![b"key".to_vec(), b"value".to_vec(), b"match".to_vec()],
            Vec::new(),
            vec![2],
            Vec::new(),
        )
        .unwrap()
        .with_directives(vec![
            Directive::Set {
                key: 0,
                value: Some(1),
            },
            Directive::SetCapture {
                capture: 0,
                key: 0,
                value: Some(1),
            },
        ])
        .unwrap();
        let native = native_ir::lower(&program).unwrap();
        let compiled = compile_straight_line(&native).unwrap();
        let nodes = [MetalNode {
            parent: NONE,
            next_any: NONE,
            next_named: NONE,
            ..MetalNode::default()
        }];
        let mut captures = [LinkRecord::default(); 1];
        let mut final_captures = [FinalCaptureRecord::default(); 1];
        let mut results = [ResultRecord::default(); 1];
        let mut properties = [PropertyRecord::default(); 2];
        let mut status = TaskStatus::default();
        let mut context: NativeTaskContext = unsafe { mem::zeroed() };
        context.nodes = nodes.as_ptr();
        context.node_count = 1;
        context.current_node = 0;
        context.task_work_limit = 16;
        context.path_work_limit = 16;
        context.captures = captures.as_mut_ptr();
        context.capture_quota_limit = 1;
        context.results = results.as_mut_ptr();
        context.result_quota_limit = 1;
        context.final_captures = final_captures.as_mut_ptr();
        context.final_capture_capacity = 1;
        context.properties = properties.as_mut_ptr();
        context.property_quota_limit = 2;
        context.status = &mut status;
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 1);
        assert_eq!(status.property_count, 2);
        assert_eq!(results[0].property_count, 1);
        assert_eq!(properties[0].kind, 0);
        assert_eq!(properties[0].key, 0);
        assert_eq!(properties[0].value, 1);
        assert_eq!(properties[1].kind, 1);
        assert_eq!(properties[1].owner, 0);
        assert_eq!(final_captures[0].property_count, 1);
    }

    #[test]
    fn cranelift_output_applies_capture_offsets() {
        use crate::code_rule_dsl::transducer::packed::{CAPTURE, Directive};

        let program = Program::new(
            vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
            vec![b"capture".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap()
        .with_directives(vec![Directive::Offset {
            capture: 0,
            offsets: [0, 1, 0, 0],
        }])
        .unwrap();
        let native = native_ir::lower(&program).unwrap();
        let compiled = compile_straight_line(&native).unwrap();
        let source = b"abc";
        let nodes = [MetalNode {
            parent: NONE,
            next_any: NONE,
            next_named: NONE,
            source_end: source.len() as u32,
            ..MetalNode::default()
        }];
        let line_starts = [0_u32];
        let mut captures = [LinkRecord::default(); 1];
        let mut final_captures = [FinalCaptureRecord::default(); 1];
        let mut results = [ResultRecord::default(); 1];
        let mut status = TaskStatus::default();
        let mut context: NativeTaskContext = unsafe { mem::zeroed() };
        context.nodes = nodes.as_ptr();
        context.node_count = 1;
        context.source = source.as_ptr();
        context.source_length = source.len() as u32;
        context.current_node = 0;
        context.task_work_limit = 16;
        context.path_work_limit = 16;
        context.captures = captures.as_mut_ptr();
        context.capture_quota_limit = 1;
        context.results = results.as_mut_ptr();
        context.result_quota_limit = 1;
        context.line_starts = line_starts.as_ptr();
        context.line_count = 1;
        context.final_captures = final_captures.as_mut_ptr();
        context.final_capture_capacity = 1;
        context.status = &mut status;
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 1);
        assert_eq!((final_captures[0].start, final_captures[0].end), (1, 3));
    }

    #[test]
    fn cranelift_output_rejects_invalid_offsets_without_visible_records() {
        use crate::code_rule_dsl::transducer::packed::{CAPTURE, Directive};

        let program = Program::new(
            vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
            vec![b"capture".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap()
        .with_directives(vec![Directive::Offset {
            capture: 0,
            offsets: [0, 99, 0, 0],
        }])
        .unwrap();
        let compiled = compile_straight_line(&native_ir::lower(&program).unwrap()).unwrap();
        let source = b"abc";
        let nodes = [MetalNode {
            parent: NONE,
            next_any: NONE,
            next_named: NONE,
            source_end: source.len() as u32,
            ..MetalNode::default()
        }];
        let line_starts = [0_u32];
        let mut captures = [LinkRecord::default(); 1];
        let mut final_captures = [FinalCaptureRecord::default(); 1];
        let mut results = [ResultRecord::default(); 1];
        let mut status = TaskStatus::default();
        let mut context: NativeTaskContext = unsafe { mem::zeroed() };
        context.nodes = nodes.as_ptr();
        context.node_count = 1;
        context.source = source.as_ptr();
        context.source_length = source.len() as u32;
        context.current_node = 0;
        context.task_work_limit = 16;
        context.path_work_limit = 16;
        context.captures = captures.as_mut_ptr();
        context.capture_quota_limit = 1;
        context.results = results.as_mut_ptr();
        context.result_quota_limit = 1;
        context.line_starts = line_starts.as_ptr();
        context.line_count = 1;
        context.final_captures = final_captures.as_mut_ptr();
        context.final_capture_capacity = 1;
        context.status = &mut status;
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 0);
        assert_eq!(status.final_capture_count, 0);
        assert_eq!(status.property_count, 0);
    }

    #[test]
    fn cranelift_output_selects_adjacent_captures_in_metal_order() {
        use crate::code_rule_dsl::transducer::packed::{CAPTURE, CHILD, Directive, NEXT};

        let program = Program::new(
            vec![
                word(CHILD, 0, 0),
                word(CAPTURE, 0, 0),
                word(NEXT, 0, 0),
                word(CAPTURE, 0, 0),
                word(NEXT, 0, 0),
                word(CAPTURE, 0, 1),
                word(EMIT, 0, 0),
            ],
            vec![b"source".to_vec(), b"target".to_vec()],
            Vec::new(),
            vec![0, 1],
            Vec::new(),
        )
        .unwrap()
        .with_directives(vec![Directive::SelectAdjacent {
            capture: 0,
            target: 1,
        }])
        .unwrap();
        let native = native_ir::lower(&program).unwrap();
        let compiled = compile_straight_line(&native).unwrap();
        let source = b"a\n\nb t";
        let mut nodes = [MetalNode {
            parent: NONE,
            next_any: NONE,
            next_named: NONE,
            descendant_end: 4,
            ..MetalNode::default()
        }; 4];
        nodes[0].child_count = 1;
        nodes[1].parent = 0;
        nodes[1].next_any = 2;
        nodes[1].source_end = 1;
        nodes[2].parent = 0;
        nodes[2].next_any = 3;
        nodes[2].source_start = 3;
        nodes[2].source_end = 4;
        nodes[3].parent = 0;
        nodes[3].source_start = 5;
        nodes[3].source_end = 6;
        let children = [1_u32];
        let line_starts = [0_u32, 2, 3];
        let mut captures = [LinkRecord::default(); 3];
        let mut final_captures = [FinalCaptureRecord::default(); 3];
        let mut results = [ResultRecord::default(); 1];
        let mut status = TaskStatus::default();
        let mut context: NativeTaskContext = unsafe { mem::zeroed() };
        context.nodes = nodes.as_ptr();
        context.node_count = nodes.len() as u32;
        context.children = children.as_ptr();
        context.child_count = 1;
        context.source = source.as_ptr();
        context.source_length = source.len() as u32;
        context.current_node = 0;
        context.task_work_limit = 32;
        context.path_work_limit = 32;
        context.captures = captures.as_mut_ptr();
        context.capture_quota_limit = captures.len() as u32;
        context.results = results.as_mut_ptr();
        context.result_quota_limit = 1;
        context.line_starts = line_starts.as_ptr();
        context.line_count = line_starts.len() as u32;
        context.final_captures = final_captures.as_mut_ptr();
        context.final_capture_capacity = final_captures.len() as u32;
        context.status = &mut status;
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 1);
        assert_eq!(
            [
                final_captures[0].retained,
                final_captures[1].retained,
                final_captures[2].retained,
            ],
            [0, 1, 1],
        );
    }

    #[test]
    fn cranelift_output_downcases_into_bounded_transformed_storage() {
        use crate::code_rule_dsl::transducer::packed::{CAPTURE, Directive};

        let program = Program::new(
            vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
            vec![b"capture".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap()
        .with_directives(vec![
            Directive::Downcase { capture: 0 },
            Directive::Downcase { capture: 0 },
        ])
        .unwrap();
        let native = native_ir::lower(&program).unwrap();
        let compiled = compile_straight_line(&native).unwrap();
        let source = "ÄABCz".as_bytes();
        let nodes = [MetalNode {
            parent: NONE,
            next_any: NONE,
            next_named: NONE,
            source_end: source.len() as u32,
            ..MetalNode::default()
        }];
        let mut captures = [LinkRecord::default(); 1];
        let mut final_captures = [FinalCaptureRecord::default(); 1];
        let mut results = [ResultRecord::default(); 1];
        let mut bytes = [0_u8; 16];
        let mut status = TaskStatus::default();
        let mut context: NativeTaskContext = unsafe { mem::zeroed() };
        context.nodes = nodes.as_ptr();
        context.node_count = 1;
        context.source = source.as_ptr();
        context.source_length = source.len() as u32;
        context.current_node = 0;
        context.task_work_limit = 16;
        context.path_work_limit = 16;
        context.captures = captures.as_mut_ptr();
        context.capture_quota_limit = 1;
        context.results = results.as_mut_ptr();
        context.result_quota_limit = 1;
        context.final_captures = final_captures.as_mut_ptr();
        context.final_capture_capacity = 1;
        context.output_bytes = bytes.as_mut_ptr();
        context.output_byte_quota_limit = bytes.len() as u32;
        context.status = &mut status;
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.output_byte_count, (source.len() * 2) as u32);
        assert_eq!(&bytes[..source.len()], "Äabcz".as_bytes());
        assert_eq!(&bytes[source.len()..source.len() * 2], "Äabcz".as_bytes());
        assert_eq!(final_captures[0].text_start, source.len() as u32);
        assert_eq!(final_captures[0].text_length, source.len() as u32);
    }

    #[test]
    fn cranelift_output_strips_ordered_nfa_matches() {
        use crate::code_rule_dsl::transducer::packed::{CAPTURE, Directive};

        let program = Program::new(
            vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
            vec![b"n".to_vec(), b"capture".to_vec()],
            Vec::new(),
            vec![1],
            Vec::new(),
        )
        .unwrap()
        .with_directives(vec![Directive::Strip {
            capture: 0,
            source: 0,
            compiled: regex::Regex::new("n").unwrap(),
        }])
        .unwrap();
        let compiled = compile_straight_line(&native_ir::lower(&program).unwrap()).unwrap();
        let source = b"banana";
        let nodes = [MetalNode {
            parent: NONE,
            next_any: NONE,
            next_named: NONE,
            source_end: source.len() as u32,
            ..MetalNode::default()
        }];
        let mut raw_captures = [LinkRecord::default(); 1];
        let mut final_captures = [FinalCaptureRecord::default(); 1];
        let mut results = [ResultRecord::default(); 1];
        let mut value_work = [0_u32; 256];
        let mut bytes = [0_u8; 16];
        let mut status = TaskStatus::default();
        let mut context: NativeTaskContext = unsafe { mem::zeroed() };
        context.nodes = nodes.as_ptr();
        context.node_count = 1;
        context.source = source.as_ptr();
        context.source_length = source.len() as u32;
        context.current_node = 0;
        context.task_work_limit = 4096;
        context.path_work_limit = 4096;
        context.captures = raw_captures.as_mut_ptr();
        context.capture_quota_limit = 1;
        context.results = results.as_mut_ptr();
        context.result_quota_limit = 1;
        context.value_work = value_work.as_mut_ptr();
        context.value_capacity = value_work.len() as u32;
        context.final_captures = final_captures.as_mut_ptr();
        context.final_capture_capacity = 1;
        context.output_bytes = bytes.as_mut_ptr();
        context.output_byte_quota_limit = bytes.len() as u32;
        context.status = &mut status;
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 1);
        assert_eq!(status.output_byte_count, 4);
        assert_eq!(&bytes[..4], b"baaa");
        assert_eq!(final_captures[0].text_start, 0);
        assert_eq!(final_captures[0].text_length, 4);
    }

    #[test]
    fn cranelift_output_normalizes_language_from_mime() {
        use crate::code_rule_dsl::transducer::packed::{CAPTURE, Directive};

        let program = Program::new(
            vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
            vec![b"capture".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap()
        .with_directives(vec![Directive::SetLanguageFromMimeType { capture: 0 }])
        .unwrap();
        let compiled = compile_straight_line(&native_ir::lower(&program).unwrap()).unwrap();
        let source = b"text/module";
        let nodes = [MetalNode {
            parent: NONE,
            next_any: NONE,
            next_named: NONE,
            source_end: source.len() as u32,
            ..MetalNode::default()
        }];
        let mut raw_captures = [LinkRecord::default(); 1];
        let mut final_captures = [FinalCaptureRecord::default(); 1];
        let mut results = [ResultRecord::default(); 1];
        let mut properties = [PropertyRecord::default(); 1];
        let mut bytes = [0_u8; 32];
        let mut status = TaskStatus::default();
        let mut context: NativeTaskContext = unsafe { mem::zeroed() };
        context.nodes = nodes.as_ptr();
        context.node_count = 1;
        context.source = source.as_ptr();
        context.source_length = source.len() as u32;
        context.current_node = 0;
        context.task_work_limit = 64;
        context.path_work_limit = 64;
        context.captures = raw_captures.as_mut_ptr();
        context.capture_quota_limit = 1;
        context.results = results.as_mut_ptr();
        context.result_quota_limit = 1;
        context.final_captures = final_captures.as_mut_ptr();
        context.final_capture_capacity = 1;
        context.properties = properties.as_mut_ptr();
        context.property_quota_limit = 1;
        context.output_bytes = bytes.as_mut_ptr();
        context.output_byte_quota_limit = bytes.len() as u32;
        context.status = &mut status;
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 1);
        assert_eq!(status.property_count, 1);
        assert_eq!(status.output_byte_count, 18);
        assert_eq!(&bytes[..18], b"javascriptlanguage");
        assert_eq!(results[0].dynamic_count, 1);
        assert_eq!(properties[0].kind, 2);
        assert_eq!(properties[0].value, 1);
        assert_eq!(properties[0].value_start, 0);
        assert_eq!(properties[0].value_length, 10);
        assert_eq!(properties[0].key_start, 10);
        assert_eq!(properties[0].key_length, 8);
    }

    #[test]
    fn cranelift_text_lowers_lua_value_programs() {
        use crate::code_rule_dsl::transducer::packed::{LuaPattern, TextPredicate};

        for (pattern, source, expected) in [
            (b"^%a+$".as_slice(), b"name".as_slice(), 1),
            (b"^%a+$".as_slice(), b"name1".as_slice(), 0),
            (b"^(%a+)%s+%1$".as_slice(), b"go go".as_slice(), 1),
            (b"^%b()$".as_slice(), b"(a(b)c)".as_slice(), 1),
            (b"^a.-b$".as_slice(), b"axxxbxxb".as_slice(), 1),
            (b"%f[%a]word".as_slice(), b"a word".as_slice(), 1),
            (b"%f[%a]word".as_slice(), b"sword".as_slice(), 0),
            (b"^[-][-](%s?)@$".as_slice(), b"-- @".as_slice(), 1),
            (b"^()value$".as_slice(), b"value".as_slice(), 1),
            (b"^%d*$".as_slice(), b"12345".as_slice(), 1),
            (b"^.$".as_slice(), b"?".as_slice(), 1),
            (b"^x$".as_slice(), b"x".as_slice(), 1),
            (b"^%c$".as_slice(), b"\n".as_slice(), 1),
            (b"^%d$".as_slice(), b"7".as_slice(), 1),
            (b"^%g$".as_slice(), b"!".as_slice(), 1),
            (b"^%l$".as_slice(), b"q".as_slice(), 1),
            (b"^%p$".as_slice(), b"!".as_slice(), 1),
            (b"^%s$".as_slice(), b"\t".as_slice(), 1),
            (b"^%u$".as_slice(), b"Q".as_slice(), 1),
            (b"^%w$".as_slice(), b"7".as_slice(), 1),
            (b"^%x$".as_slice(), b"f".as_slice(), 1),
            (b"^%z$".as_slice(), b"\0".as_slice(), 1),
            (b"^%A$".as_slice(), b"7".as_slice(), 1),
            (b"^[a-c]$".as_slice(), b"b".as_slice(), 1),
            (b"^[^a-c]$".as_slice(), b"z".as_slice(), 1),
            (b"^a?$".as_slice(), b"".as_slice(), 1),
            (b"^a+$".as_slice(), b"aaa".as_slice(), 1),
            (b"^a*$".as_slice(), b"".as_slice(), 1),
        ] {
            let status = execute_generated_text(
                TextPredicate::LuaPattern {
                    source: 1,
                    compiled: LuaPattern::parse(pattern).unwrap(),
                },
                vec![b"match".to_vec(), pattern.to_vec()],
                source,
                4096,
            );
            assert_eq!(status.error, 0, "Lua {pattern:?} on {source:?}");
            assert_eq!(
                status.result_count, expected,
                "Lua {pattern:?} on {source:?}"
            );
        }
    }

    #[test]
    fn cranelift_lua_reports_exact_capacity_errors() {
        use crate::code_rule_dsl::transducer::packed::{LuaPattern, TextPredicate};

        let predicate = || TextPredicate::LuaPattern {
            source: 1,
            compiled: LuaPattern::parse(b"^a$").unwrap(),
        };
        let strings = || vec![b"match".to_vec(), b"^a$".to_vec()];
        let status = execute_generated_text(predicate(), strings(), b"a", 1);
        assert_eq!(status.error, 2);
        let status = execute_generated_text(predicate(), strings(), b"a", 2);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 1);
    }

    #[test]
    fn cranelift_regex_matches_the_byte_oracle_across_look_states() {
        use crate::code_rule_dsl::transducer::packed::TextPredicate;

        let cases: &[(&str, &[u8])] = &[
            (r"^$", b""),
            (r"^a", b"abc"),
            (r"a$", b"ba"),
            (r"(?m)^b", b"a\nb"),
            (r"(?m)b$", b"b\na"),
            (r"(?-u:\bword\b)", b" word "),
            (r"(?-u:\Bword\B)", b"swords"),
            (r"(?u)\bπ\b", " π ".as_bytes()),
            (r"(?u)\Bπ\B", "xπx".as_bytes()),
            (r"(?:ab|a)c", b"ac"),
            (r"(?:a*?)(?:ab|b)", b"aaab"),
            (r"a?", b""),
            (r"[a-z]+", b"abc"),
            (r"[^a]+", b"bbb"),
            (r".", &[0xff]),
        ];
        for &(pattern, source) in cases {
            let expected = u32::from(regex::bytes::Regex::new(pattern).unwrap().is_match(source));
            let status = execute_generated_text(
                TextPredicate::Regex {
                    source: 1,
                    compiled: regex::Regex::new(pattern).unwrap(),
                },
                vec![b"match".to_vec(), pattern.as_bytes().to_vec()],
                source,
                16_384,
            );
            assert_eq!(status.error, 0, "regex {pattern:?} on {source:?}");
            assert_eq!(
                status.result_count, expected,
                "regex {pattern:?} on {source:?}"
            );
        }
    }

    #[test]
    fn cranelift_regex_uses_the_exact_metal_workspace_boundary() {
        use crate::code_rule_dsl::transducer::packed::{TextPredicate, value::ByteNfa};

        let pattern = r"^a(?:bc|d)+$";
        let required = ByteNfa::compile(pattern).unwrap().states.len() * 4;
        let predicate = || TextPredicate::Regex {
            source: 1,
            compiled: regex::Regex::new(pattern).unwrap(),
        };
        let strings = || vec![b"match".to_vec(), pattern.as_bytes().to_vec()];
        let status = execute_generated_text(predicate(), strings(), b"abcbcd", required - 1);
        assert_eq!(status.error, 2);
        let status = execute_generated_text(predicate(), strings(), b"abcbcd", required);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 1);
        let status = execute_generated_text(predicate(), strings(), b"abcbcd", required + 1);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 1);
    }

    #[test]
    fn cranelift_text_lowers_regex_value_programs() {
        use crate::code_rule_dsl::transducer::packed::TextPredicate;

        for (pattern, source, expected) in [
            (r"^a(?:bc|d)+$", b"abcbcd".as_slice(), 1),
            (r"^a(?:bc|d)+$", b"abce".as_slice(), 0),
            (r"(?u)\bπ\b", " π ".as_bytes(), 1),
            (r"(?u)\bπ\b", "xπx".as_bytes(), 0),
        ] {
            let status = execute_generated_text(
                TextPredicate::Regex {
                    source: 1,
                    compiled: regex::Regex::new(pattern).unwrap(),
                },
                vec![b"match".to_vec(), pattern.as_bytes().to_vec()],
                source,
                4096,
            );
            assert_eq!(status.error, 0, "regex {pattern:?} on {source:?}");
            assert_eq!(
                status.result_count, expected,
                "regex {pattern:?} on {source:?}"
            );
        }
    }

    #[test]
    fn cranelift_text_lowers_equal_and_prefix_value_programs() {
        use crate::code_rule_dsl::transducer::packed::{TEXT, TextPredicate};

        for (predicate, source, expected) in [
            (TextPredicate::Equal(1), b"value".as_slice(), 1),
            (TextPredicate::Equal(1), b"values".as_slice(), 0),
            (TextPredicate::Prefix(1), b"values".as_slice(), 1),
            (TextPredicate::Prefix(1), b"val".as_slice(), 0),
        ] {
            let program = Program::new(
                vec![word(TEXT, 0, 0), word(EMIT, 0, 0)],
                vec![b"match".to_vec(), b"value".to_vec()],
                vec![predicate],
                vec![0],
                Vec::new(),
            )
            .unwrap();
            let native = native_ir::lower(&program).unwrap();
            let compiled = compile_straight_line(&native).unwrap();
            let nodes = [MetalNode {
                parent: NONE,
                next_any: NONE,
                next_named: NONE,
                source_end: source.len() as u32,
                ..MetalNode::default()
            }];
            let mut results = [ResultRecord::default(); 1];
            let mut status = TaskStatus::default();
            let mut context = NativeTaskContext {
                nodes: nodes.as_ptr(),
                node_count: 1,
                children: std::ptr::null(),
                child_count: 0,
                supertypes: std::ptr::null(),
                supertype_count: 0,
                source: source.as_ptr(),
                source_length: source.len() as u32,
                values: std::ptr::null(),
                value_count: 0,
                nfa_states: std::ptr::null(),
                nfa_state_count: 0,
                nfa_data: std::ptr::null(),
                nfa_data_count: 0,
                nfa_ranges: std::ptr::null(),
                nfa_range_count: 0,
                lua_instructions: std::ptr::null(),
                lua_instruction_count: 0,
                lua_members: std::ptr::null(),
                lua_member_count: 0,
                value_work: std::ptr::null_mut(),
                value_start: 0,
                value_capacity: 0,
                current_node: 0,
                start_node: 0,
                task_ordinal: 0,
                pending: std::ptr::null_mut(),
                pending_count: 0,
                pending_quota_limit: 0,
                current_steps: 0,
                current_captures: 0,
                current_cursors: 0,
                current_repeats: 0,
                current_following: 0,
                current_capture_hash: 0,
                current_cursor_hash: 0,
                current_repeat_hash: 0,
                task_work_limit: 4,
                path_work_limit: 4,
                captures: std::ptr::null(),
                cursors: std::ptr::null(),
                repeats: std::ptr::null(),
                query_buckets: std::ptr::null_mut(),
                query_bucket_capacity: 0,
                query_members: std::ptr::null_mut(),
                query_member_count: 0,
                query_member_quota_limit: 0,
                queued_states: std::ptr::null_mut(),
                queued_state_quota_limit: 0,
                greedy_entries: std::ptr::null_mut(),
                greedy_entry_quota_limit: 0,
                maximum_cursor_slot: 0,
                capture_count: 0,
                capture_quota_limit: 0,
                cursor_count: 0,
                cursor_quota_limit: 0,
                repeat_count: 0,
                repeat_quota_limit: 0,
                results: results.as_mut_ptr(),
                result_quota_limit: results.len() as u32,
                line_starts: std::ptr::null(),
                line_count: 0,
                final_captures: std::ptr::null_mut(),
                final_capture_capacity: 0,
                properties: std::ptr::null_mut(),
                property_quota_limit: 0,
                output_bytes: std::ptr::null_mut(),
                output_byte_quota_limit: 0,
                status: &mut status,
            };
            compiled.execute(&mut context);
            assert_eq!(status.error, 0);
            assert_eq!(status.result_count, expected);
        }
    }

    #[test]
    fn cranelift_supertype_reads_checked_persisted_memberships() {
        use crate::code_rule_dsl::transducer::packed::SUPERTYPE;

        let program = Program::new(
            vec![word(SUPERTYPE, 0, 9), word(EMIT, 0, 0)],
            vec![b"match".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let native = native_ir::lower(&program).unwrap();
        let compiled = compile_straight_line(&native).unwrap();
        let nodes = [MetalNode {
            parent: NONE,
            next_any: NONE,
            next_named: NONE,
            ..MetalNode::default()
        }];
        let memberships = [
            MetalSupertype { node: 0, kind: 9 },
            MetalSupertype { node: 0, kind: 9 },
        ];
        let malformed = [MetalSupertype { node: 1, kind: 9 }];
        let mut results = [ResultRecord::default(); 1];
        let mut status = TaskStatus::default();
        let mut context = NativeTaskContext {
            nodes: nodes.as_ptr(),
            node_count: nodes.len() as u32,
            children: std::ptr::null(),
            child_count: 0,
            supertypes: memberships.as_ptr(),
            supertype_count: memberships.len() as u32,
            source: std::ptr::null(),
            source_length: 0,
            values: std::ptr::null(),
            value_count: 0,
            nfa_states: std::ptr::null(),
            nfa_state_count: 0,
            nfa_data: std::ptr::null(),
            nfa_data_count: 0,
            nfa_ranges: std::ptr::null(),
            nfa_range_count: 0,
            lua_instructions: std::ptr::null(),
            lua_instruction_count: 0,
            lua_members: std::ptr::null(),
            lua_member_count: 0,
            value_work: std::ptr::null_mut(),
            value_start: 0,
            value_capacity: 0,
            current_node: 0,
            start_node: 0,
            task_ordinal: 0,
            pending: std::ptr::null_mut(),
            pending_count: 0,
            pending_quota_limit: 0,
            current_steps: 0,
            current_captures: 0,
            current_cursors: 0,
            current_repeats: 0,
            current_following: 0,
            current_capture_hash: 0,
            current_cursor_hash: 0,
            current_repeat_hash: 0,
            task_work_limit: 4,
            path_work_limit: 4,
            captures: std::ptr::null(),
            cursors: std::ptr::null(),
            repeats: std::ptr::null(),
            query_buckets: std::ptr::null_mut(),
            query_bucket_capacity: 0,
            query_members: std::ptr::null_mut(),
            query_member_count: 0,
            query_member_quota_limit: 0,
            queued_states: std::ptr::null_mut(),
            queued_state_quota_limit: 0,
            greedy_entries: std::ptr::null_mut(),
            greedy_entry_quota_limit: 0,
            maximum_cursor_slot: 0,
            capture_count: 0,
            capture_quota_limit: 0,
            cursor_count: 0,
            cursor_quota_limit: 0,
            repeat_count: 0,
            repeat_quota_limit: 0,
            results: results.as_mut_ptr(),
            result_quota_limit: results.len() as u32,
            line_starts: std::ptr::null(),
            line_count: 0,
            final_captures: std::ptr::null_mut(),
            final_capture_capacity: 0,
            properties: std::ptr::null_mut(),
            property_quota_limit: 0,
            output_bytes: std::ptr::null_mut(),
            output_byte_quota_limit: 0,
            status: &mut status,
        };
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 1);

        status = TaskStatus::default();
        context.current_steps = 0;
        context.supertype_count = 0;
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 0);

        status = TaskStatus::default();
        context.current_steps = 0;
        context.supertypes = malformed.as_ptr();
        context.supertype_count = 1;
        compiled.execute(&mut context);
        assert_eq!(status.error, 1);
        assert_eq!(status.result_count, 0);
    }

    #[test]
    fn cranelift_absent_field_scans_the_checked_child_range() {
        use crate::code_rule_dsl::transducer::packed::ABSENT_FIELD;

        let program = Program::new(
            vec![word(ABSENT_FIELD, 0, 7), word(EMIT, 0, 0)],
            vec![b"match".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let native = native_ir::lower(&program).unwrap();
        let compiled = compile_straight_line(&native).unwrap();
        let mut nodes = [MetalNode {
            parent: NONE,
            next_any: NONE,
            next_named: NONE,
            ..MetalNode::default()
        }; 3];
        nodes[0].child_count = 2;
        nodes[1].field = 3;
        nodes[2].field = 7;
        nodes[2].flags = 0b1100;
        let children = [1_u32, 2];
        let mut results = [ResultRecord::default(); 1];
        let mut status = TaskStatus::default();
        let mut context = NativeTaskContext {
            nodes: nodes.as_ptr(),
            node_count: nodes.len() as u32,
            children: children.as_ptr(),
            child_count: children.len() as u32,
            supertypes: std::ptr::null(),
            supertype_count: 0,
            source: std::ptr::null(),
            source_length: 0,
            values: std::ptr::null(),
            value_count: 0,
            nfa_states: std::ptr::null(),
            nfa_state_count: 0,
            nfa_data: std::ptr::null(),
            nfa_data_count: 0,
            nfa_ranges: std::ptr::null(),
            nfa_range_count: 0,
            lua_instructions: std::ptr::null(),
            lua_instruction_count: 0,
            lua_members: std::ptr::null(),
            lua_member_count: 0,
            value_work: std::ptr::null_mut(),
            value_start: 0,
            value_capacity: 0,
            current_node: 0,
            start_node: 0,
            task_ordinal: 0,
            pending: std::ptr::null_mut(),
            pending_count: 0,
            pending_quota_limit: 0,
            current_steps: 0,
            current_captures: 0,
            current_cursors: 0,
            current_repeats: 0,
            current_following: 0,
            current_capture_hash: 0,
            current_cursor_hash: 0,
            current_repeat_hash: 0,
            task_work_limit: 4,
            path_work_limit: 4,
            captures: std::ptr::null(),
            cursors: std::ptr::null(),
            repeats: std::ptr::null(),
            query_buckets: std::ptr::null_mut(),
            query_bucket_capacity: 0,
            query_members: std::ptr::null_mut(),
            query_member_count: 0,
            query_member_quota_limit: 0,
            queued_states: std::ptr::null_mut(),
            queued_state_quota_limit: 0,
            greedy_entries: std::ptr::null_mut(),
            greedy_entry_quota_limit: 0,
            maximum_cursor_slot: 0,
            capture_count: 0,
            capture_quota_limit: 0,
            cursor_count: 0,
            cursor_quota_limit: 0,
            repeat_count: 0,
            repeat_quota_limit: 0,
            results: results.as_mut_ptr(),
            result_quota_limit: results.len() as u32,
            line_starts: std::ptr::null(),
            line_count: 0,
            final_captures: std::ptr::null_mut(),
            final_capture_capacity: 0,
            properties: std::ptr::null_mut(),
            property_quota_limit: 0,
            output_bytes: std::ptr::null_mut(),
            output_byte_quota_limit: 0,
            status: &mut status,
        };
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 0);

        let mut without_field = nodes;
        without_field[0].child_count = 1;
        status = TaskStatus::default();
        context.nodes = without_field.as_ptr();
        context.current_steps = 0;
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 1);

        let mut malformed = without_field;
        malformed[0].child_start = 2;
        malformed[0].child_count = 1;
        status = TaskStatus::default();
        context.nodes = malformed.as_ptr();
        context.current_steps = 0;
        compiled.execute(&mut context);
        assert_eq!(status.error, 1);
        assert_eq!(status.result_count, 0);
    }

    #[test]
    fn cranelift_long_size_uses_the_verified_u64_constant() {
        use crate::code_rule_dsl::transducer::packed::SIZE;

        for (mode, expected_results) in [(0x80, 1), (0x81, 1), (0x82, 1)] {
            let program = Program::new(
                vec![word(SIZE, mode, 0), word(EMIT, 0, 0)],
                vec![b"match".to_vec()],
                Vec::new(),
                vec![0],
                vec![70_000],
            )
            .unwrap();
            let native = native_ir::lower(&program).unwrap();
            let compiled = compile_straight_line(&native).unwrap();
            let nodes = [MetalNode {
                parent: NONE,
                next_any: NONE,
                next_named: NONE,
                source_start: 1,
                source_end: 70_001,
                ..MetalNode::default()
            }];
            let mut results = [ResultRecord::default(); 1];
            let mut status = TaskStatus::default();
            let mut context = NativeTaskContext {
                nodes: nodes.as_ptr(),
                node_count: 1,
                children: std::ptr::null(),
                child_count: 0,
                supertypes: std::ptr::null(),
                supertype_count: 0,
                source: std::ptr::null(),
                source_length: 0,
                values: std::ptr::null(),
                value_count: 0,
                nfa_states: std::ptr::null(),
                nfa_state_count: 0,
                nfa_data: std::ptr::null(),
                nfa_data_count: 0,
                nfa_ranges: std::ptr::null(),
                nfa_range_count: 0,
                lua_instructions: std::ptr::null(),
                lua_instruction_count: 0,
                lua_members: std::ptr::null(),
                lua_member_count: 0,
                value_work: std::ptr::null_mut(),
                value_start: 0,
                value_capacity: 0,
                current_node: 0,
                start_node: 0,
                task_ordinal: 0,
                pending: std::ptr::null_mut(),
                pending_count: 0,
                pending_quota_limit: 0,
                current_steps: 0,
                current_captures: 0,
                current_cursors: 0,
                current_repeats: 0,
                current_following: 0,
                current_capture_hash: 0,
                current_cursor_hash: 0,
                current_repeat_hash: 0,
                task_work_limit: 4,
                path_work_limit: 4,
                captures: std::ptr::null(),
                cursors: std::ptr::null(),
                repeats: std::ptr::null(),
                query_buckets: std::ptr::null_mut(),
                query_bucket_capacity: 0,
                query_members: std::ptr::null_mut(),
                query_member_count: 0,
                query_member_quota_limit: 0,
                queued_states: std::ptr::null_mut(),
                queued_state_quota_limit: 0,
                greedy_entries: std::ptr::null_mut(),
                greedy_entry_quota_limit: 0,
                maximum_cursor_slot: 0,
                capture_count: 0,
                capture_quota_limit: 0,
                cursor_count: 0,
                cursor_quota_limit: 0,
                repeat_count: 0,
                repeat_quota_limit: 0,
                results: results.as_mut_ptr(),
                result_quota_limit: results.len() as u32,
                line_starts: std::ptr::null(),
                line_count: 0,
                final_captures: std::ptr::null_mut(),
                final_capture_capacity: 0,
                properties: std::ptr::null_mut(),
                property_quota_limit: 0,
                output_bytes: std::ptr::null_mut(),
                output_byte_quota_limit: 0,
                status: &mut status,
            };
            compiled.execute(&mut context);
            assert_eq!(status.error, 0);
            assert_eq!(status.result_count, expected_results);
        }
    }

    #[test]
    fn straight_line_cranelift_reports_exact_result_capacity() {
        let program = Program::new(
            vec![word(EMIT, 0, 0)],
            vec![b"match".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let native = native_ir::lower(&program).unwrap();
        let compiled = compile_straight_line(&native).unwrap();
        let nodes = [MetalNode::default()];
        let mut status = TaskStatus::default();
        let mut context = NativeTaskContext {
            nodes: nodes.as_ptr(),
            node_count: 1,
            children: std::ptr::null(),
            child_count: 0,
            supertypes: std::ptr::null(),
            supertype_count: 0,
            source: std::ptr::null(),
            source_length: 0,
            values: std::ptr::null(),
            value_count: 0,
            nfa_states: std::ptr::null(),
            nfa_state_count: 0,
            nfa_data: std::ptr::null(),
            nfa_data_count: 0,
            nfa_ranges: std::ptr::null(),
            nfa_range_count: 0,
            lua_instructions: std::ptr::null(),
            lua_instruction_count: 0,
            lua_members: std::ptr::null(),
            lua_member_count: 0,
            value_work: std::ptr::null_mut(),
            value_start: 0,
            value_capacity: 0,
            current_node: 0,
            start_node: 0,
            task_ordinal: 0,
            pending: std::ptr::null_mut(),
            pending_count: 0,
            pending_quota_limit: 0,
            current_steps: 0,
            current_captures: 0,
            current_cursors: 0,
            current_repeats: 0,
            current_following: 0,
            current_capture_hash: 0,
            current_cursor_hash: 0,
            current_repeat_hash: 0,
            task_work_limit: 16,
            path_work_limit: 16,
            captures: std::ptr::null(),
            cursors: std::ptr::null(),
            repeats: std::ptr::null(),
            query_buckets: std::ptr::null_mut(),
            query_bucket_capacity: 0,
            query_members: std::ptr::null_mut(),
            query_member_count: 0,
            query_member_quota_limit: 0,
            queued_states: std::ptr::null_mut(),
            queued_state_quota_limit: 0,
            greedy_entries: std::ptr::null_mut(),
            greedy_entry_quota_limit: 0,
            maximum_cursor_slot: 0,
            capture_count: 0,
            capture_quota_limit: 0,
            cursor_count: 0,
            cursor_quota_limit: 0,
            repeat_count: 0,
            repeat_quota_limit: 0,
            results: std::ptr::null_mut(),
            result_quota_limit: 0,
            line_starts: std::ptr::null(),
            line_count: 0,
            final_captures: std::ptr::null_mut(),
            final_capture_capacity: 0,
            properties: std::ptr::null_mut(),
            property_quota_limit: 0,
            output_bytes: std::ptr::null_mut(),
            output_byte_quota_limit: 0,
            status: &mut status,
        };
        compiled.execute(&mut context);
        assert_eq!(status.error, 2);
        assert_eq!(status.result_count, 0);
    }
    #[test]
    fn strip_transform_work_is_owned_by_each_candidate_across_captures_and_searches() {
        use crate::code_rule_dsl::transducer::packed::{Directive, value::ByteNfa};
        let pattern = "^# a+$";
        let nfa = ByteNfa::compile(pattern).unwrap();
        let (source, cost) = [40_000, 80_000, 120_000, 160_000]
            .into_iter()
            .find_map(|length| {
                let source = format!("# {}", "a".repeat(length));
                let (found, first) = nfa.test_find_work_count_from(source.as_bytes(), 0);
                assert_eq!(found, Some([0, source.len() as u32]));
                let (found, last) = nfa.test_find_work_count_from(source.as_bytes(), source.len());
                assert_eq!(found, None);
                let cost = (first + last) * 2;
                (cost > super::super::MAX_STEPS_PER_PATH / 2
                    && cost < super::super::MAX_STEPS_PER_PATH - 128)
                    .then_some((source, cost))
            })
            .expect("two capture searches fit one budget but two candidates do not");
        let files = [tape(&format!("{source}\n{source}\n"))];
        let anchors = [vec![vec![0]]];
        for query in [false, true] {
            let base = Program::new(
                vec![
                    word(super::super::CHILD, 1, 0),
                    word(CAPTURE, 0, 0),
                    word(CAPTURE, 0, 0),
                    word(EMIT, 0, 0),
                ],
                vec![b"text".to_vec(), pattern.as_bytes().to_vec()],
                vec![],
                vec![0],
                vec![],
            )
            .unwrap()
            .with_directives(vec![Directive::Strip {
                capture: 0,
                source: 1,
                compiled: regex::Regex::new(pattern).unwrap(),
            }])
            .unwrap();
            let programs = [if query {
                base.with_tree_sitter_query_semantics().unwrap()
            } else {
                base
            }];
            let expected =
                crate::code_rule_dsl::transducer::packed::execute_batch(&programs, &files, Some(&anchors))
                    .unwrap();
            assert_eq!(expected[0][0].len(), 2);
            for (index, output) in expected[0][0].iter().enumerate() {
                assert_eq!(output.start, 0);
                assert_eq!(output.captures[0].node, index as u32 + 1);
                assert_eq!(output.captures.len(), 2);
                assert_eq!(output.captures[0], output.captures[1]);
                assert_eq!(output.captures[0].text.as_deref(), Some(b"".as_slice()));
            }
            for limit in [cost - 1, cost, cost + 1] {
                let actual = execute_batch_packed_with_work_limit(
                    &programs,
                    &files,
                    &anchors,
                    None,
                    limit as u32,
                )
                .and_then(PackedExecution::into_outcome)
                .and_then(BatchOutcome::into_result);
                if limit < cost {
                    assert_eq!(actual, Err(Error::WorkLimit));
                } else {
                    assert_eq!(
                        actual,
                        Ok(expected.clone()),
                        "query={query}, limit={limit}, candidate cost={cost}"
                    );
                }
            }
        }
    }
    #[test]
    fn strip_searches_use_full_input_nfa_work_at_every_budget_boundary() {
        use crate::code_rule_dsl::transducer::packed::{Directive, value::ByteNfa};
        for (pattern, source) in [
            ("n", "banana"),
            ("a|aa", "aaab"),
            ("aa|a", "aaab"),
            ("", "é😀"),
            (r"\b", "ab é"),
            ("a*", "aaab"),
            ("^a|b", "aaab"),
        ] {
            let nfa = ByteNfa::compile(pattern).unwrap();
            let mut cost = 0;
            let mut search = 0;
            loop {
                let (found, used) = nfa.test_find_work_count_from(source.as_bytes(), search);
                cost += used;
                let Some([start, end]) = found else { break };
                search = end as usize;
                if start == end {
                    if search == source.len() {
                        break;
                    }
                    search += source[search..].chars().next().unwrap().len_utf8();
                }
            }
            cost *= 2;
            let programs = [Program::new(
                vec![word(CAPTURE, 0, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
                vec![b"text".to_vec(), pattern.as_bytes().to_vec()],
                vec![],
                vec![0],
                vec![],
            )
            .unwrap()
            .with_directives(vec![Directive::Strip {
                capture: 0,
                source: 1,
                compiled: regex::Regex::new(pattern).unwrap(),
            }])
            .unwrap()];
            let files = [tape(source)];
            let anchors = [vec![vec![0]]];
            let expected =
                crate::code_rule_dsl::transducer::packed::execute_batch(&programs, &files, Some(&anchors))
                    .unwrap();
            assert_eq!(expected[0][0].len(), 1);
            assert_eq!(expected[0][0][0].captures.len(), 2);
            assert_eq!(expected[0][0][0].captures[0], expected[0][0][0].captures[1]);
            for limit in [cost - 1, cost, cost + 1] {
                let actual = execute_batch_packed_with_work_limit(
                    &programs,
                    &files,
                    &anchors,
                    None,
                    limit as u32,
                )
                .and_then(PackedExecution::into_outcome)
                .and_then(BatchOutcome::into_result);
                if limit < cost {
                    assert_eq!(
                        actual,
                        Err(Error::WorkLimit),
                        "pattern={pattern:?}, cost={cost}, limit={limit}"
                    );
                } else {
                    assert_eq!(
                        actual,
                        Ok(expected.clone()),
                        "pattern={pattern:?}, cost={cost}, limit={limit}"
                    );
                }
            }
        }
    }
}
