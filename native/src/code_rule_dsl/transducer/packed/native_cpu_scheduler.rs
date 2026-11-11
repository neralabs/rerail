use super::{
    metal_batch_abi::{
        GreedyEntryRecord, LinkRecord, NONE, PathRecord, PendingRecord, QueryBucketRecord,
        QueryMemberRecord, QueuedStateRecord, RepeatRecord, TaskStatus,
    },
    native_cpu::NativeTaskContext,
};
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlags, Value, condcodes::IntCC, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Linkage, Module};
use std::mem;

pub(super) struct SchedulerFunctions {
    pub cursor_value: FuncId,
    pub cursor_equal: FuncId,
    pub repeat_equal: FuncId,
    pub capture_contains: FuncId,
    pub query_admit: FuncId,
    pub exact_state_equal: FuncId,
    pub queue_insert: FuncId,
    pub queue_remove: FuncId,
    pub greedy_schedule: FuncId,
    pub greedy_take: FuncId,
    pub path_enqueue: FuncId,
}

fn pointer_at(
    function: &mut FunctionBuilder<'_>,
    base: Value,
    index: Value,
    stride: usize,
) -> Value {
    let pointer_type = function.func.dfg.value_type(base);
    let offset = function.ins().imul_imm(index, stride as i64);
    let offset = function.ins().uextend(pointer_type, offset);
    function.ins().iadd(base, offset)
}

fn context_pointer(function: &mut FunctionBuilder<'_>, context: Value, offset: usize) -> Value {
    let pointer_type = function.func.dfg.value_type(context);
    function
        .ins()
        .load(pointer_type, MemFlags::trusted(), context, offset as i32)
}

fn charge_task_work(function: &mut FunctionBuilder<'_>, task: Value) -> Value {
    let status = context_pointer(function, task, mem::offset_of!(NativeTaskContext, status));
    let used = function.ins().load(
        types::I32,
        MemFlags::trusted(),
        status,
        mem::offset_of!(TaskStatus, reserved) as i32,
    );
    let next = function.ins().iadd_imm(used, 1);
    function.ins().store(
        MemFlags::trusted(),
        next,
        status,
        mem::offset_of!(TaskStatus, reserved) as i32,
    );
    let limit = function.ins().load(
        types::I32,
        MemFlags::trusted(),
        task,
        mem::offset_of!(NativeTaskContext, task_work_limit) as i32,
    );
    let within = function
        .ins()
        .icmp(IntCC::UnsignedLessThanOrEqual, next, limit);
    let no_overflow = function
        .ins()
        .icmp(IntCC::UnsignedGreaterThanOrEqual, next, used);
    function.ins().band(within, no_overflow)
}

fn declare(
    module: &mut JITModule,
    name: &str,
    parameter_count: usize,
) -> Result<FuncId, Box<cranelift_module::ModuleError>> {
    let mut signature = module.make_signature();
    signature
        .params
        .push(AbiParam::new(module.target_config().pointer_type()));
    for _ in 1..parameter_count {
        signature.params.push(AbiParam::new(types::I32));
    }
    signature.returns.push(AbiParam::new(types::I32));
    Ok(module.declare_function(name, Linkage::Local, &signature)?)
}

fn declare_i64(
    module: &mut JITModule,
    name: &str,
    parameter_count: usize,
) -> Result<FuncId, Box<cranelift_module::ModuleError>> {
    let mut signature = module.make_signature();
    signature
        .params
        .push(AbiParam::new(module.target_config().pointer_type()));
    for _ in 1..parameter_count {
        signature.params.push(AbiParam::new(types::I32));
    }
    signature.returns.push(AbiParam::new(types::I64));
    Ok(module.declare_function(name, Linkage::Local, &signature)?)
}

fn define_cursor_value(
    module: &mut JITModule,
    id: FuncId,
) -> Result<(), Box<cranelift_module::ModuleError>> {
    let mut context = module.make_context();
    context.func.signature = module.make_signature();
    context
        .func
        .signature
        .params
        .push(AbiParam::new(module.target_config().pointer_type()));
    context
        .func
        .signature
        .params
        .push(AbiParam::new(types::I32));
    context
        .func
        .signature
        .params
        .push(AbiParam::new(types::I32));
    context
        .func
        .signature
        .returns
        .push(AbiParam::new(types::I32));
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = function.create_block();
        function.append_block_params_for_function_params(entry);
        let walk = function.create_block();
        function.append_block_param(walk, types::I32);
        let inspect = function.create_block();
        let found = function.create_block();
        let absent = function.create_block();
        function.switch_to_block(entry);
        let task = function.block_params(entry)[0];
        let head = function.block_params(entry)[1];
        function.ins().jump(walk, &[head.into()]);
        function.switch_to_block(walk);
        let head = function.block_params(walk)[0];
        let present = function.ins().icmp_imm(IntCC::NotEqual, head, 0);
        function.ins().brif(present, inspect, &[], absent, &[]);
        function.switch_to_block(inspect);
        let one = function.ins().iconst(types::I32, 1);
        let index = function.ins().isub(head, one);
        let links = context_pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, cursors),
        );
        let link = pointer_at(&mut function, links, index, mem::size_of::<LinkRecord>());
        let actual_slot = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            link,
            mem::offset_of!(LinkRecord, first) as i32,
        );
        let slot = function.block_params(entry)[2];
        let same = function.ins().icmp(IntCC::Equal, actual_slot, slot);
        let previous = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            link,
            mem::offset_of!(LinkRecord, previous) as i32,
        );
        function
            .ins()
            .brif(same, found, &[], walk, &[previous.into()]);
        function.switch_to_block(found);
        let value = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            link,
            mem::offset_of!(LinkRecord, second) as i32,
        );
        function.ins().return_(&[value]);
        function.switch_to_block(absent);
        let none = function.ins().iconst(types::I32, i64::from(NONE));
        function.ins().return_(&[none]);
        function.seal_all_blocks();
        function.finalize();
    }
    super::native_cpu_codegen::define_function(module, id, &mut context)?;
    module.clear_context(&mut context);
    Ok(())
}

fn define_cursor_equal(
    module: &mut JITModule,
    id: FuncId,
    cursor_value: FuncId,
) -> Result<(), Box<cranelift_module::ModuleError>> {
    let mut context = module.make_context();
    context.func.signature = module.make_signature();
    context
        .func
        .signature
        .params
        .push(AbiParam::new(module.target_config().pointer_type()));
    context
        .func
        .signature
        .params
        .push(AbiParam::new(types::I32));
    context
        .func
        .signature
        .params
        .push(AbiParam::new(types::I32));
    context
        .func
        .signature
        .returns
        .push(AbiParam::new(types::I32));
    let cursor_value = module.declare_func_in_func(cursor_value, &mut context.func);
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = function.create_block();
        function.append_block_params_for_function_params(entry);
        let walk = function.create_block();
        function.append_block_param(walk, types::I32);
        let compare = function.create_block();
        let unequal = function.create_block();
        let equal = function.create_block();
        function.switch_to_block(entry);
        let task = function.block_params(entry)[0];
        let zero = function.ins().iconst(types::I32, 0);
        function.ins().jump(walk, &[zero.into()]);
        function.switch_to_block(walk);
        let slot = function.block_params(walk)[0];
        let maximum = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, maximum_cursor_slot) as i32,
        );
        let more = function.ins().icmp(IntCC::UnsignedLessThan, slot, maximum);
        function.ins().brif(more, compare, &[], equal, &[]);
        function.switch_to_block(compare);
        let left = function.block_params(entry)[1];
        let right = function.block_params(entry)[2];
        let left_call = function.ins().call(cursor_value, &[task, left, slot]);
        let left_value = function.inst_results(left_call)[0];
        let right_call = function.ins().call(cursor_value, &[task, right, slot]);
        let right_value = function.inst_results(right_call)[0];
        let same = function.ins().icmp(IntCC::Equal, left_value, right_value);
        let next_slot = function.ins().iadd_imm(slot, 1);
        function
            .ins()
            .brif(same, walk, &[next_slot.into()], unequal, &[]);
        function.switch_to_block(unequal);
        let zero = function.ins().iconst(types::I32, 0);
        function.ins().return_(&[zero]);
        function.switch_to_block(equal);
        let one = function.ins().iconst(types::I32, 1);
        function.ins().return_(&[one]);
        function.seal_all_blocks();
        function.finalize();
    }
    super::native_cpu_codegen::define_function(module, id, &mut context)?;
    module.clear_context(&mut context);
    Ok(())
}

// N's (5bf45ff0), which bb4c9605's merge dropped where G's version of this file replaced the
// mechanism; kept as copied (open item O21).
#[allow(dead_code)]
fn define_capture_occurrences(
    module: &mut JITModule,
    id: FuncId,
) -> Result<(), Box<cranelift_module::ModuleError>> {
    let mut context = module.make_context();
    context.func.signature = module.make_signature();
    context
        .func
        .signature
        .params
        .push(AbiParam::new(module.target_config().pointer_type()));
    for _ in 0..3 {
        context
            .func
            .signature
            .params
            .push(AbiParam::new(types::I32));
    }
    context
        .func
        .signature
        .returns
        .push(AbiParam::new(types::I32));
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = function.create_block();
        function.append_block_params_for_function_params(entry);
        let walk = function.create_block();
        function.append_block_param(walk, types::I32);
        function.append_block_param(walk, types::I32);
        let inspect = function.create_block();
        let done = function.create_block();
        function.append_block_param(done, types::I32);
        function.switch_to_block(entry);
        let zero = function.ins().iconst(types::I32, 0);
        let initial_head = function.block_params(entry)[1];
        function
            .ins()
            .jump(walk, &[initial_head.into(), zero.into()]);
        function.switch_to_block(walk);
        let head = function.block_params(walk)[0];
        let count = function.block_params(walk)[1];
        let present = function.ins().icmp_imm(IntCC::NotEqual, head, 0);
        function
            .ins()
            .brif(present, inspect, &[], done, &[count.into()]);
        function.switch_to_block(inspect);
        let task = function.block_params(entry)[0];
        let links = context_pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, captures),
        );
        let one = function.ins().iconst(types::I32, 1);
        let index = function.ins().isub(head, one);
        let link = pointer_at(&mut function, links, index, mem::size_of::<LinkRecord>());
        let label = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            link,
            mem::offset_of!(LinkRecord, first) as i32,
        );
        let node = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            link,
            mem::offset_of!(LinkRecord, second) as i32,
        );
        let expected_label = function.block_params(entry)[2];
        let expected_node = function.block_params(entry)[3];
        let label_same = function.ins().icmp(IntCC::Equal, label, expected_label);
        let node_same = function.ins().icmp(IntCC::Equal, node, expected_node);
        let matches = function.ins().band(label_same, node_same);
        let incremented = function.ins().iadd_imm(count, 1);
        let next_count = function.ins().select(matches, incremented, count);
        let previous = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            link,
            mem::offset_of!(LinkRecord, previous) as i32,
        );
        function
            .ins()
            .jump(walk, &[previous.into(), next_count.into()]);
        function.switch_to_block(done);
        let result = function.block_params(done)[0];
        function.ins().return_(&[result]);
        function.seal_all_blocks();
        function.finalize();
    }
    module.define_function(id, &mut context)?;
    module.clear_context(&mut context);
    Ok(())
}

#[allow(dead_code)]
fn define_capture_contains(
    module: &mut JITModule,
    id: FuncId,
    occurrences: FuncId,
) -> Result<(), Box<cranelift_module::ModuleError>> {
    let mut context = module.make_context();
    context.func.signature = module.make_signature();
    context
        .func
        .signature
        .params
        .push(AbiParam::new(module.target_config().pointer_type()));
    context
        .func
        .signature
        .params
        .push(AbiParam::new(types::I32));
    context
        .func
        .signature
        .params
        .push(AbiParam::new(types::I32));
    context
        .func
        .signature
        .returns
        .push(AbiParam::new(types::I32));
    let occurrences = module.declare_func_in_func(occurrences, &mut context.func);
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = function.create_block();
        function.append_block_params_for_function_params(entry);
        let walk = function.create_block();
        function.append_block_param(walk, types::I32);
        let inspect = function.create_block();
        let reject = function.create_block();
        let accept = function.create_block();
        function.switch_to_block(entry);
        let initial_right = function.block_params(entry)[2];
        function.ins().jump(walk, &[initial_right.into()]);
        function.switch_to_block(walk);
        let head = function.block_params(walk)[0];
        let present = function.ins().icmp_imm(IntCC::NotEqual, head, 0);
        function.ins().brif(present, inspect, &[], accept, &[]);
        function.switch_to_block(inspect);
        let task = function.block_params(entry)[0];
        let links = context_pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, captures),
        );
        let one = function.ins().iconst(types::I32, 1);
        let index = function.ins().isub(head, one);
        let link = pointer_at(&mut function, links, index, mem::size_of::<LinkRecord>());
        let label = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            link,
            mem::offset_of!(LinkRecord, first) as i32,
        );
        let node = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            link,
            mem::offset_of!(LinkRecord, second) as i32,
        );
        let left = function.block_params(entry)[1];
        let right = function.block_params(entry)[2];
        let left_call = function.ins().call(occurrences, &[task, left, label, node]);
        let left_count = function.inst_results(left_call)[0];
        let right_call = function
            .ins()
            .call(occurrences, &[task, right, label, node]);
        let right_count = function.inst_results(right_call)[0];
        let enough =
            function
                .ins()
                .icmp(IntCC::UnsignedGreaterThanOrEqual, left_count, right_count);
        let previous = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            link,
            mem::offset_of!(LinkRecord, previous) as i32,
        );
        function
            .ins()
            .brif(enough, walk, &[previous.into()], reject, &[]);
        function.switch_to_block(reject);
        let zero = function.ins().iconst(types::I32, 0);
        function.ins().return_(&[zero]);
        function.switch_to_block(accept);
        let one = function.ins().iconst(types::I32, 1);
        function.ins().return_(&[one]);
        function.seal_all_blocks();
        function.finalize();
    }
    module.define_function(id, &mut context)?;
    module.clear_context(&mut context);
    Ok(())
}

fn define_repeat_equal(
    module: &mut JITModule,
    id: FuncId,
) -> Result<(), Box<cranelift_module::ModuleError>> {
    let mut context = module.make_context();
    context.func.signature = module.make_signature();
    context
        .func
        .signature
        .params
        .push(AbiParam::new(module.target_config().pointer_type()));
    context
        .func
        .signature
        .params
        .push(AbiParam::new(types::I32));
    context
        .func
        .signature
        .params
        .push(AbiParam::new(types::I32));
    context
        .func
        .signature
        .returns
        .push(AbiParam::new(types::I32));
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = function.create_block();
        function.append_block_params_for_function_params(entry);
        let walk = function.create_block();
        function.append_block_param(walk, types::I32);
        function.append_block_param(walk, types::I32);
        let inspect = function.create_block();
        let finish = function.create_block();
        function.append_block_param(finish, types::I32);
        function.switch_to_block(entry);
        let initial_left = function.block_params(entry)[1];
        let initial_right = function.block_params(entry)[2];
        function
            .ins()
            .jump(walk, &[initial_left.into(), initial_right.into()]);
        function.switch_to_block(walk);
        let left = function.block_params(walk)[0];
        let right = function.block_params(walk)[1];
        let left_present = function.ins().icmp_imm(IntCC::NotEqual, left, 0);
        let right_present = function.ins().icmp_imm(IntCC::NotEqual, right, 0);
        let both = function.ins().band(left_present, right_present);
        let same_end = function.ins().icmp(IntCC::Equal, left, right);
        let one = function.ins().iconst(types::I32, 1);
        let zero = function.ins().iconst(types::I32, 0);
        let same_end = function.ins().select(same_end, one, zero);
        function
            .ins()
            .brif(both, inspect, &[], finish, &[same_end.into()]);
        function.switch_to_block(inspect);
        let task = function.block_params(entry)[0];
        let links = context_pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, repeats),
        );
        let one = function.ins().iconst(types::I32, 1);
        let left_index = function.ins().isub(left, one);
        let right_index = function.ins().isub(right, one);
        let left_link = pointer_at(
            &mut function,
            links,
            left_index,
            mem::size_of::<RepeatRecord>(),
        );
        let right_link = pointer_at(
            &mut function,
            links,
            right_index,
            mem::size_of::<RepeatRecord>(),
        );
        let left_value = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            left_link,
            mem::offset_of!(RepeatRecord, value) as i32,
        );
        let right_value = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            right_link,
            mem::offset_of!(RepeatRecord, value) as i32,
        );
        let same = function.ins().icmp(IntCC::Equal, left_value, right_value);
        let continue_walk = function.create_block();
        let zero = function.ins().iconst(types::I32, 0);
        function
            .ins()
            .brif(same, continue_walk, &[], finish, &[zero.into()]);
        function.switch_to_block(continue_walk);
        let left_previous = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            left_link,
            mem::offset_of!(RepeatRecord, previous) as i32,
        );
        let right_previous = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            right_link,
            mem::offset_of!(RepeatRecord, previous) as i32,
        );
        function
            .ins()
            .jump(walk, &[left_previous.into(), right_previous.into()]);
        function.switch_to_block(finish);
        let result = function.block_params(finish)[0];
        function.ins().return_(&[result]);
        function.seal_all_blocks();
        function.finalize();
    }
    super::native_cpu_codegen::define_function(module, id, &mut context)?;
    module.clear_context(&mut context);
    Ok(())
}

#[path = "native_cpu_capture_relation.rs"]
mod capture_relation;

fn define_exact_state_equal(
    module: &mut JITModule,
    id: FuncId,
    cursor_equal: FuncId,
    repeat_equal: FuncId,
    capture_contains: FuncId,
) -> Result<(), Box<cranelift_module::ModuleError>> {
    let mut context = module.make_context();
    context.func.signature = module.make_signature();
    context
        .func
        .signature
        .params
        .push(AbiParam::new(module.target_config().pointer_type()));
    for _ in 0..6 {
        context
            .func
            .signature
            .params
            .push(AbiParam::new(types::I32));
    }
    context
        .func
        .signature
        .returns
        .push(AbiParam::new(types::I32));
    let cursor_equal = module.declare_func_in_func(cursor_equal, &mut context.func);
    let repeat_equal = module.declare_func_in_func(repeat_equal, &mut context.func);
    let capture_contains = module.declare_func_in_func(capture_contains, &mut context.func);
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = function.create_block();
        function.append_block_params_for_function_params(entry);
        function.switch_to_block(entry);
        let values = function.block_params(entry);
        let task = values[0];
        let left_captures = values[1];
        let left_cursors = values[2];
        let left_repeats = values[3];
        let right_captures = values[4];
        let right_cursors = values[5];
        let right_repeats = values[6];
        let left_contains = function
            .ins()
            .call(capture_contains, &[task, left_captures, right_captures]);
        let left_contains = function.inst_results(left_contains)[0];
        let right_contains = function
            .ins()
            .call(capture_contains, &[task, right_captures, left_captures]);
        let right_contains = function.inst_results(right_contains)[0];
        let cursors_equal = function
            .ins()
            .call(cursor_equal, &[task, left_cursors, right_cursors]);
        let cursors_equal = function.inst_results(cursors_equal)[0];
        let repeats_equal = function
            .ins()
            .call(repeat_equal, &[task, left_repeats, right_repeats]);
        let repeats_equal = function.inst_results(repeats_equal)[0];
        let equal = function.ins().band(left_contains, right_contains);
        let equal = function.ins().band(equal, cursors_equal);
        let equal = function.ins().band(equal, repeats_equal);
        function.ins().return_(&[equal]);
        function.seal_all_blocks();
        function.finalize();
    }
    super::native_cpu_codegen::define_function(module, id, &mut context)?;
    module.clear_context(&mut context);
    Ok(())
}

fn mix(function: &mut FunctionBuilder<'_>, hash: Value, value: Value) -> Value {
    let constant = function.ins().iconst(types::I32, 0x9e37_79b9_u32 as i64);
    let sum = function.ins().iadd(value, constant);
    let left = function.ins().ishl_imm(hash, 6);
    let sum = function.ins().iadd(sum, left);
    let right = function.ins().ushr_imm(hash, 2);
    let sum = function.ins().iadd(sum, right);
    function.ins().bxor(hash, sum)
}

fn store_current_path(function: &mut FunctionBuilder<'_>, task: Value, path: Value, pc: Value) {
    let flags = MemFlags::trusted();
    function
        .ins()
        .store(flags, pc, path, mem::offset_of!(PathRecord, pc) as i32);
    for (context_offset, path_offset) in [
        (
            mem::offset_of!(NativeTaskContext, current_node),
            mem::offset_of!(PathRecord, node),
        ),
        (
            mem::offset_of!(NativeTaskContext, start_node),
            mem::offset_of!(PathRecord, start),
        ),
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
            mem::offset_of!(NativeTaskContext, current_steps),
            mem::offset_of!(PathRecord, steps),
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
        let value = function
            .ins()
            .load(types::I32, flags, task, context_offset as i32);
        function.ins().store(flags, value, path, path_offset as i32);
    }
}

fn restore_path(function: &mut FunctionBuilder<'_>, task: Value, path: Value) -> Value {
    let flags = MemFlags::trusted();
    let pc = function.ins().load(
        types::I32,
        flags,
        path,
        mem::offset_of!(PathRecord, pc) as i32,
    );
    for (context_offset, path_offset) in [
        (
            mem::offset_of!(NativeTaskContext, current_node),
            mem::offset_of!(PathRecord, node),
        ),
        (
            mem::offset_of!(NativeTaskContext, start_node),
            mem::offset_of!(PathRecord, start),
        ),
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
            mem::offset_of!(NativeTaskContext, current_steps),
            mem::offset_of!(PathRecord, steps),
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
        let value = function
            .ins()
            .load(types::I32, flags, path, path_offset as i32);
        function
            .ins()
            .store(flags, value, task, context_offset as i32);
    }
    pc
}

fn define_queue_insert(
    module: &mut JITModule,
    id: FuncId,
    exact_state_equal: FuncId,
) -> Result<(), Box<cranelift_module::ModuleError>> {
    let mut context = module.make_context();
    context.func.signature = module.make_signature();
    context
        .func
        .signature
        .params
        .push(AbiParam::new(module.target_config().pointer_type()));
    context
        .func
        .signature
        .params
        .push(AbiParam::new(types::I32));
    context
        .func
        .signature
        .params
        .push(AbiParam::new(types::I32));
    context
        .func
        .signature
        .returns
        .push(AbiParam::new(types::I32));
    let exact_state_equal = module.declare_func_in_func(exact_state_equal, &mut context.func);
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = function.create_block();
        function.append_block_params_for_function_params(entry);
        let prepare = function.create_block();
        let probe = function.create_block();
        function.append_block_param(probe, types::I32);
        function.append_block_param(probe, types::I32);
        function.append_block_param(probe, types::I32);
        let inspect = function.create_block();
        function.append_block_param(inspect, types::I32);
        function.append_block_param(inspect, types::I32);
        function.append_block_param(inspect, types::I32);
        let occupied = function.create_block();
        function.append_block_param(
            occupied,
            function
                .func
                .dfg
                .value_type(function.block_params(entry)[0]),
        );
        function.append_block_param(occupied, types::I32);
        function.append_block_param(occupied, types::I32);
        function.append_block_param(occupied, types::I32);
        let compare = function.create_block();
        function.append_block_param(
            compare,
            function
                .func
                .dfg
                .value_type(function.block_params(entry)[0]),
        );
        function.append_block_param(compare, types::I32);
        function.append_block_param(compare, types::I32);
        function.append_block_param(compare, types::I32);
        let advance = function.create_block();
        function.append_block_param(advance, types::I32);
        function.append_block_param(advance, types::I32);
        function.append_block_param(advance, types::I32);
        let write = function.create_block();
        function.append_block_param(write, types::I32);
        let duplicate = function.create_block();
        let capacity = function.create_block();

        function.switch_to_block(entry);
        let task = function.block_params(entry)[0];
        let pc = function.block_params(entry)[1];
        let sibling = function.block_params(entry)[2];
        let table_capacity = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, queued_state_quota_limit) as i32,
        );
        let available = function.ins().icmp_imm(IntCC::NotEqual, table_capacity, 0);
        function.ins().brif(available, prepare, &[], capacity, &[]);

        function.switch_to_block(prepare);
        let node = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, current_node) as i32,
        );
        let captures = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, current_captures) as i32,
        );
        let cursors = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, current_cursors) as i32,
        );
        let repeats = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, current_repeats) as i32,
        );
        let capture_hash = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, current_capture_hash) as i32,
        );
        let cursor_hash = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, current_cursor_hash) as i32,
        );
        let repeat_hash = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, current_repeat_hash) as i32,
        );
        let hash = mix(&mut function, pc, node);
        let hash = mix(&mut function, hash, capture_hash);
        let hash = mix(&mut function, hash, cursor_hash);
        let hash = mix(&mut function, hash, repeat_hash);
        let hash = mix(&mut function, hash, sibling);
        let slot = function.ins().urem(hash, table_capacity);
        let zero = function.ins().iconst(types::I32, 0);
        let none = function.ins().iconst(types::I32, i64::from(NONE));
        function
            .ins()
            .jump(probe, &[slot.into(), zero.into(), none.into()]);

        function.switch_to_block(probe);
        let slot = function.block_params(probe)[0];
        let probe_count = function.block_params(probe)[1];
        let available_slot = function.block_params(probe)[2];
        let in_range = function
            .ins()
            .icmp(IntCC::UnsignedLessThan, probe_count, table_capacity);
        let exhausted = function.create_block();
        function.append_block_param(exhausted, types::I32);
        function.ins().brif(
            in_range,
            inspect,
            &[slot.into(), probe_count.into(), available_slot.into()],
            exhausted,
            &[available_slot.into()],
        );
        function.switch_to_block(exhausted);
        let available_slot = function.block_params(exhausted)[0];
        let has_available =
            function
                .ins()
                .icmp_imm(IntCC::NotEqual, available_slot, i64::from(NONE));
        function.ins().brif(
            has_available,
            write,
            &[available_slot.into()],
            capacity,
            &[],
        );

        function.switch_to_block(inspect);
        let slot = function.block_params(inspect)[0];
        let probe_count = function.block_params(inspect)[1];
        let available_slot = function.block_params(inspect)[2];
        let records = context_pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, queued_states),
        );
        let record = pointer_at(
            &mut function,
            records,
            slot,
            mem::size_of::<QueuedStateRecord>(),
        );
        let state = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            record,
            mem::offset_of!(QueuedStateRecord, occupied) as i32,
        );
        let empty = function.ins().icmp_imm(IntCC::Equal, state, 0);
        let choose_empty = function.create_block();
        function.append_block_param(choose_empty, types::I32);
        function.append_block_param(choose_empty, types::I32);
        function.ins().brif(
            empty,
            choose_empty,
            &[slot.into(), available_slot.into()],
            occupied,
            &[
                record.into(),
                slot.into(),
                probe_count.into(),
                available_slot.into(),
            ],
        );
        function.switch_to_block(choose_empty);
        let slot = function.block_params(choose_empty)[0];
        let available_slot = function.block_params(choose_empty)[1];
        let no_prior = function
            .ins()
            .icmp_imm(IntCC::Equal, available_slot, i64::from(NONE));
        let selected = function.ins().select(no_prior, slot, available_slot);
        function.ins().jump(write, &[selected.into()]);

        function.switch_to_block(occupied);
        let record = function.block_params(occupied)[0];
        let slot = function.block_params(occupied)[1];
        let probe_count = function.block_params(occupied)[2];
        let available_slot = function.block_params(occupied)[3];
        let state = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            record,
            mem::offset_of!(QueuedStateRecord, occupied) as i32,
        );
        let tombstone = function.ins().icmp_imm(IntCC::Equal, state, 2);
        let no_prior = function
            .ins()
            .icmp_imm(IntCC::Equal, available_slot, i64::from(NONE));
        let use_tombstone = function.ins().band(tombstone, no_prior);
        let available_slot = function.ins().select(use_tombstone, slot, available_slot);
        let live = function.ins().icmp_imm(IntCC::Equal, state, 1);
        function.ins().brif(
            live,
            compare,
            &[
                record.into(),
                slot.into(),
                probe_count.into(),
                available_slot.into(),
            ],
            advance,
            &[slot.into(), probe_count.into(), available_slot.into()],
        );

        function.switch_to_block(compare);
        let record = function.block_params(compare)[0];
        let slot = function.block_params(compare)[1];
        let probe_count = function.block_params(compare)[2];
        let available_slot = function.block_params(compare)[3];
        let old_hash = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            record,
            mem::offset_of!(QueuedStateRecord, hash) as i32,
        );
        let old_pc = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            record,
            mem::offset_of!(QueuedStateRecord, pc) as i32,
        );
        let old_node = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            record,
            mem::offset_of!(QueuedStateRecord, node) as i32,
        );
        let old_sibling = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            record,
            mem::offset_of!(QueuedStateRecord, sibling_requirement) as i32,
        );
        let same_hash = function.ins().icmp(IntCC::Equal, old_hash, hash);
        let same_pc = function.ins().icmp(IntCC::Equal, old_pc, pc);
        let same_node = function.ins().icmp(IntCC::Equal, old_node, node);
        let same_sibling = function.ins().icmp(IntCC::Equal, old_sibling, sibling);
        let scalars = function.ins().band(same_hash, same_pc);
        let scalars = function.ins().band(scalars, same_node);
        let scalars = function.ins().band(scalars, same_sibling);
        let compare_state = function.create_block();
        function.ins().brif(
            scalars,
            compare_state,
            &[],
            advance,
            &[slot.into(), probe_count.into(), available_slot.into()],
        );
        function.switch_to_block(compare_state);
        let old_captures = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            record,
            mem::offset_of!(QueuedStateRecord, captures) as i32,
        );
        let old_cursors = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            record,
            mem::offset_of!(QueuedStateRecord, cursors) as i32,
        );
        let old_repeats = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            record,
            mem::offset_of!(QueuedStateRecord, repeats) as i32,
        );
        let call = function.ins().call(
            exact_state_equal,
            &[
                task,
                old_captures,
                old_cursors,
                old_repeats,
                captures,
                cursors,
                repeats,
            ],
        );
        let same = function.inst_results(call)[0];
        let same = function.ins().icmp_imm(IntCC::NotEqual, same, 0);
        function.ins().brif(
            same,
            duplicate,
            &[],
            advance,
            &[slot.into(), probe_count.into(), available_slot.into()],
        );

        function.switch_to_block(advance);
        let slot = function.block_params(advance)[0];
        let probe_count = function.block_params(advance)[1];
        let available_slot = function.block_params(advance)[2];
        let one = function.ins().iconst(types::I32, 1);
        let slot = function.ins().iadd(slot, one);
        let slot = function.ins().urem(slot, table_capacity);
        let probe_count = function.ins().iadd(probe_count, one);
        function.ins().jump(
            probe,
            &[slot.into(), probe_count.into(), available_slot.into()],
        );

        function.switch_to_block(write);
        let slot = function.block_params(write)[0];
        let records = context_pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, queued_states),
        );
        let record = pointer_at(
            &mut function,
            records,
            slot,
            mem::size_of::<QueuedStateRecord>(),
        );
        let one = function.ins().iconst(types::I32, 1);
        for (offset, value) in [
            (mem::offset_of!(QueuedStateRecord, occupied), one),
            (mem::offset_of!(QueuedStateRecord, hash), hash),
            (mem::offset_of!(QueuedStateRecord, pc), pc),
            (mem::offset_of!(QueuedStateRecord, node), node),
            (
                mem::offset_of!(QueuedStateRecord, sibling_requirement),
                sibling,
            ),
            (mem::offset_of!(QueuedStateRecord, captures), captures),
            (mem::offset_of!(QueuedStateRecord, cursors), cursors),
            (mem::offset_of!(QueuedStateRecord, repeats), repeats),
        ] {
            function
                .ins()
                .store(MemFlags::trusted(), value, record, offset as i32);
        }
        function.ins().return_(&[one]);
        function.switch_to_block(duplicate);
        let zero = function.ins().iconst(types::I32, 0);
        function.ins().return_(&[zero]);
        function.switch_to_block(capacity);
        let two = function.ins().iconst(types::I32, 2);
        function.ins().return_(&[two]);
        function.seal_all_blocks();
        function.finalize();
    }
    super::native_cpu_codegen::define_function(module, id, &mut context)?;
    module.clear_context(&mut context);
    Ok(())
}

fn define_queue_remove(
    module: &mut JITModule,
    id: FuncId,
    exact_state_equal: FuncId,
) -> Result<(), Box<cranelift_module::ModuleError>> {
    let mut context = module.make_context();
    context.func.signature = module.make_signature();
    context
        .func
        .signature
        .params
        .push(AbiParam::new(module.target_config().pointer_type()));
    context
        .func
        .signature
        .params
        .push(AbiParam::new(types::I32));
    context
        .func
        .signature
        .params
        .push(AbiParam::new(types::I32));
    context
        .func
        .signature
        .returns
        .push(AbiParam::new(types::I32));
    let exact_state_equal = module.declare_func_in_func(exact_state_equal, &mut context.func);
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = function.create_block();
        function.append_block_params_for_function_params(entry);
        let prepare = function.create_block();
        let probe = function.create_block();
        function.append_block_param(probe, types::I32);
        function.append_block_param(probe, types::I32);
        let inspect = function.create_block();
        function.append_block_param(inspect, types::I32);
        function.append_block_param(inspect, types::I32);
        let compare = function.create_block();
        function.append_block_param(
            compare,
            function
                .func
                .dfg
                .value_type(function.block_params(entry)[0]),
        );
        function.append_block_param(compare, types::I32);
        function.append_block_param(compare, types::I32);
        let advance = function.create_block();
        function.append_block_param(advance, types::I32);
        function.append_block_param(advance, types::I32);
        let found = function.create_block();
        function.append_block_param(
            found,
            function
                .func
                .dfg
                .value_type(function.block_params(entry)[0]),
        );
        let missing = function.create_block();

        function.switch_to_block(entry);
        let task = function.block_params(entry)[0];
        let pc = function.block_params(entry)[1];
        let sibling = function.block_params(entry)[2];
        let table_capacity = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, queued_state_quota_limit) as i32,
        );
        let available = function.ins().icmp_imm(IntCC::NotEqual, table_capacity, 0);
        function.ins().brif(available, prepare, &[], missing, &[]);

        function.switch_to_block(prepare);
        let node = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, current_node) as i32,
        );
        let captures = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, current_captures) as i32,
        );
        let cursors = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, current_cursors) as i32,
        );
        let repeats = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, current_repeats) as i32,
        );
        let capture_hash = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, current_capture_hash) as i32,
        );
        let cursor_hash = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, current_cursor_hash) as i32,
        );
        let repeat_hash = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, current_repeat_hash) as i32,
        );
        let hash = mix(&mut function, pc, node);
        let hash = mix(&mut function, hash, capture_hash);
        let hash = mix(&mut function, hash, cursor_hash);
        let hash = mix(&mut function, hash, repeat_hash);
        let hash = mix(&mut function, hash, sibling);
        let slot = function.ins().urem(hash, table_capacity);
        let zero = function.ins().iconst(types::I32, 0);
        function.ins().jump(probe, &[slot.into(), zero.into()]);

        function.switch_to_block(probe);
        let slot = function.block_params(probe)[0];
        let probe_count = function.block_params(probe)[1];
        let in_range = function
            .ins()
            .icmp(IntCC::UnsignedLessThan, probe_count, table_capacity);
        function.ins().brif(
            in_range,
            inspect,
            &[slot.into(), probe_count.into()],
            missing,
            &[],
        );

        function.switch_to_block(inspect);
        let slot = function.block_params(inspect)[0];
        let probe_count = function.block_params(inspect)[1];
        let records = context_pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, queued_states),
        );
        let record = pointer_at(
            &mut function,
            records,
            slot,
            mem::size_of::<QueuedStateRecord>(),
        );
        let state = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            record,
            mem::offset_of!(QueuedStateRecord, occupied) as i32,
        );
        let empty = function.ins().icmp_imm(IntCC::Equal, state, 0);
        let nonempty = function.create_block();
        function.ins().brif(empty, missing, &[], nonempty, &[]);
        function.switch_to_block(nonempty);
        let live = function.ins().icmp_imm(IntCC::Equal, state, 1);
        function.ins().brif(
            live,
            compare,
            &[record.into(), slot.into(), probe_count.into()],
            advance,
            &[slot.into(), probe_count.into()],
        );

        function.switch_to_block(compare);
        let record = function.block_params(compare)[0];
        let slot = function.block_params(compare)[1];
        let probe_count = function.block_params(compare)[2];
        let old_hash = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            record,
            mem::offset_of!(QueuedStateRecord, hash) as i32,
        );
        let old_pc = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            record,
            mem::offset_of!(QueuedStateRecord, pc) as i32,
        );
        let old_node = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            record,
            mem::offset_of!(QueuedStateRecord, node) as i32,
        );
        let old_sibling = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            record,
            mem::offset_of!(QueuedStateRecord, sibling_requirement) as i32,
        );
        let same_hash = function.ins().icmp(IntCC::Equal, old_hash, hash);
        let same_pc = function.ins().icmp(IntCC::Equal, old_pc, pc);
        let same_node = function.ins().icmp(IntCC::Equal, old_node, node);
        let same_sibling = function.ins().icmp(IntCC::Equal, old_sibling, sibling);
        let scalars = function.ins().band(same_hash, same_pc);
        let scalars = function.ins().band(scalars, same_node);
        let scalars = function.ins().band(scalars, same_sibling);
        let compare_state = function.create_block();
        function.ins().brif(
            scalars,
            compare_state,
            &[],
            advance,
            &[slot.into(), probe_count.into()],
        );
        function.switch_to_block(compare_state);
        let old_captures = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            record,
            mem::offset_of!(QueuedStateRecord, captures) as i32,
        );
        let old_cursors = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            record,
            mem::offset_of!(QueuedStateRecord, cursors) as i32,
        );
        let old_repeats = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            record,
            mem::offset_of!(QueuedStateRecord, repeats) as i32,
        );
        let call = function.ins().call(
            exact_state_equal,
            &[
                task,
                old_captures,
                old_cursors,
                old_repeats,
                captures,
                cursors,
                repeats,
            ],
        );
        let same = function.inst_results(call)[0];
        let same = function.ins().icmp_imm(IntCC::NotEqual, same, 0);
        function.ins().brif(
            same,
            found,
            &[record.into()],
            advance,
            &[slot.into(), probe_count.into()],
        );

        function.switch_to_block(advance);
        let slot = function.block_params(advance)[0];
        let probe_count = function.block_params(advance)[1];
        let one = function.ins().iconst(types::I32, 1);
        let slot = function.ins().iadd(slot, one);
        let slot = function.ins().urem(slot, table_capacity);
        let probe_count = function.ins().iadd(probe_count, one);
        function
            .ins()
            .jump(probe, &[slot.into(), probe_count.into()]);

        function.switch_to_block(found);
        let record = function.block_params(found)[0];
        let tombstone = function.ins().iconst(types::I32, 2);
        function.ins().store(
            MemFlags::trusted(),
            tombstone,
            record,
            mem::offset_of!(QueuedStateRecord, occupied) as i32,
        );
        let one = function.ins().iconst(types::I32, 1);
        function.ins().return_(&[one]);
        function.switch_to_block(missing);
        let zero = function.ins().iconst(types::I32, 0);
        function.ins().return_(&[zero]);
        function.seal_all_blocks();
        function.finalize();
    }
    super::native_cpu_codegen::define_function(module, id, &mut context)?;
    module.clear_context(&mut context);
    Ok(())
}

fn define_greedy_schedule(
    module: &mut JITModule,
    id: FuncId,
    exact_state_equal: FuncId,
) -> Result<(), Box<cranelift_module::ModuleError>> {
    let mut context = module.make_context();
    context.func.signature = module.make_signature();
    context
        .func
        .signature
        .params
        .push(AbiParam::new(module.target_config().pointer_type()));
    context
        .func
        .signature
        .params
        .push(AbiParam::new(types::I32));
    context
        .func
        .signature
        .returns
        .push(AbiParam::new(types::I64));
    let exact_state_equal = module.declare_func_in_func(exact_state_equal, &mut context.func);
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = function.create_block();
        function.append_block_params_for_function_params(entry);
        let prepare = function.create_block();
        let probe = function.create_block();
        function.append_block_param(probe, types::I32);
        function.append_block_param(probe, types::I32);
        function.append_block_param(probe, types::I32);
        let inspect = function.create_block();
        function.append_block_param(inspect, types::I32);
        function.append_block_param(inspect, types::I32);
        function.append_block_param(inspect, types::I32);
        let live = function.create_block();
        function.append_block_param(
            live,
            function
                .func
                .dfg
                .value_type(function.block_params(entry)[0]),
        );
        function.append_block_param(live, types::I32);
        function.append_block_param(live, types::I32);
        function.append_block_param(live, types::I32);
        let compare_state = function.create_block();
        function.append_block_param(
            compare_state,
            function
                .func
                .dfg
                .value_type(function.block_params(entry)[0]),
        );
        function.append_block_param(compare_state, types::I32);
        function.append_block_param(compare_state, types::I32);
        function.append_block_param(compare_state, types::I32);
        let advance = function.create_block();
        function.append_block_param(advance, types::I32);
        function.append_block_param(advance, types::I32);
        function.append_block_param(advance, types::I32);
        let replace = function.create_block();
        function.append_block_param(
            replace,
            function
                .func
                .dfg
                .value_type(function.block_params(entry)[0]),
        );
        function.append_block_param(replace, types::I32);
        let insert = function.create_block();
        function.append_block_param(insert, types::I32);
        let capacity = function.create_block();

        function.switch_to_block(entry);
        let task = function.block_params(entry)[0];
        let target_pc = function.block_params(entry)[1];
        let table_capacity = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, greedy_entry_quota_limit) as i32,
        );
        let has_capacity = function.ins().icmp_imm(IntCC::NotEqual, table_capacity, 0);
        function
            .ins()
            .brif(has_capacity, prepare, &[], capacity, &[]);

        function.switch_to_block(prepare);
        let start = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, start_node) as i32,
        );
        let captures = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, current_captures) as i32,
        );
        let cursors = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, current_cursors) as i32,
        );
        let repeats = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, current_repeats) as i32,
        );
        let capture_hash = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, current_capture_hash) as i32,
        );
        let cursor_hash = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, current_cursor_hash) as i32,
        );
        let repeat_hash = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, current_repeat_hash) as i32,
        );
        let hash = mix(&mut function, target_pc, start);
        let hash = mix(&mut function, hash, capture_hash);
        let hash = mix(&mut function, hash, cursor_hash);
        let hash = mix(&mut function, hash, repeat_hash);
        let slot = function.ins().urem(hash, table_capacity);
        let zero = function.ins().iconst(types::I32, 0);
        let none = function.ins().iconst(types::I32, i64::from(NONE));
        function
            .ins()
            .jump(probe, &[slot.into(), zero.into(), none.into()]);

        function.switch_to_block(probe);
        let slot = function.block_params(probe)[0];
        let probe_count = function.block_params(probe)[1];
        let available_slot = function.block_params(probe)[2];
        let in_range = function
            .ins()
            .icmp(IntCC::UnsignedLessThan, probe_count, table_capacity);
        let exhausted = function.create_block();
        function.append_block_param(exhausted, types::I32);
        function.ins().brif(
            in_range,
            inspect,
            &[slot.into(), probe_count.into(), available_slot.into()],
            exhausted,
            &[available_slot.into()],
        );
        function.switch_to_block(exhausted);
        let available_slot = function.block_params(exhausted)[0];
        let has_slot = function
            .ins()
            .icmp_imm(IntCC::NotEqual, available_slot, i64::from(NONE));
        function
            .ins()
            .brif(has_slot, insert, &[available_slot.into()], capacity, &[]);

        function.switch_to_block(inspect);
        let slot = function.block_params(inspect)[0];
        let probe_count = function.block_params(inspect)[1];
        let available_slot = function.block_params(inspect)[2];
        let entries = context_pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, greedy_entries),
        );
        let entry_record = pointer_at(
            &mut function,
            entries,
            slot,
            mem::size_of::<GreedyEntryRecord>(),
        );
        let state = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            entry_record,
            mem::offset_of!(GreedyEntryRecord, occupied) as i32,
        );
        let empty = function.ins().icmp_imm(IntCC::Equal, state, 0);
        let choose = function.create_block();
        function.append_block_param(choose, types::I32);
        function.append_block_param(choose, types::I32);
        function.ins().brif(
            empty,
            choose,
            &[slot.into(), available_slot.into()],
            live,
            &[
                entry_record.into(),
                slot.into(),
                probe_count.into(),
                available_slot.into(),
            ],
        );
        function.switch_to_block(choose);
        let slot = function.block_params(choose)[0];
        let available_slot = function.block_params(choose)[1];
        let no_prior = function
            .ins()
            .icmp_imm(IntCC::Equal, available_slot, i64::from(NONE));
        let selected = function.ins().select(no_prior, slot, available_slot);
        function.ins().jump(insert, &[selected.into()]);

        function.switch_to_block(live);
        let entry_record = function.block_params(live)[0];
        let slot = function.block_params(live)[1];
        let probe_count = function.block_params(live)[2];
        let available_slot = function.block_params(live)[3];
        let state = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            entry_record,
            mem::offset_of!(GreedyEntryRecord, occupied) as i32,
        );
        let tombstone = function.ins().icmp_imm(IntCC::Equal, state, 2);
        let no_prior = function
            .ins()
            .icmp_imm(IntCC::Equal, available_slot, i64::from(NONE));
        let use_slot = function.ins().band(tombstone, no_prior);
        let available_slot = function.ins().select(use_slot, slot, available_slot);
        let occupied = function.ins().icmp_imm(IntCC::Equal, state, 1);
        let compare_scalars = function.create_block();
        function.ins().brif(
            occupied,
            compare_scalars,
            &[],
            advance,
            &[slot.into(), probe_count.into(), available_slot.into()],
        );
        function.switch_to_block(compare_scalars);
        let old_hash = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            entry_record,
            mem::offset_of!(GreedyEntryRecord, hash) as i32,
        );
        let old_pc = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            entry_record,
            mem::offset_of!(GreedyEntryRecord, pc) as i32,
        );
        let old_start = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            entry_record,
            mem::offset_of!(GreedyEntryRecord, start) as i32,
        );
        let same_hash = function.ins().icmp(IntCC::Equal, old_hash, hash);
        let same_pc = function.ins().icmp(IntCC::Equal, old_pc, target_pc);
        let same_start = function.ins().icmp(IntCC::Equal, old_start, start);
        let scalars = function.ins().band(same_hash, same_pc);
        let scalars = function.ins().band(scalars, same_start);
        function.ins().brif(
            scalars,
            compare_state,
            &[
                entry_record.into(),
                slot.into(),
                probe_count.into(),
                available_slot.into(),
            ],
            advance,
            &[slot.into(), probe_count.into(), available_slot.into()],
        );

        function.switch_to_block(compare_state);
        let entry_record = function.block_params(compare_state)[0];
        let slot = function.block_params(compare_state)[1];
        let probe_count = function.block_params(compare_state)[2];
        let available_slot = function.block_params(compare_state)[3];
        let old_captures = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            entry_record,
            mem::offset_of!(GreedyEntryRecord, captures) as i32,
        );
        let old_cursors = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            entry_record,
            mem::offset_of!(GreedyEntryRecord, cursors) as i32,
        );
        let old_repeats = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            entry_record,
            mem::offset_of!(GreedyEntryRecord, repeats) as i32,
        );
        let call = function.ins().call(
            exact_state_equal,
            &[
                task,
                old_captures,
                old_cursors,
                old_repeats,
                captures,
                cursors,
                repeats,
            ],
        );
        let same = function.inst_results(call)[0];
        let same = function.ins().icmp_imm(IntCC::NotEqual, same, 0);
        function.ins().brif(
            same,
            replace,
            &[entry_record.into(), slot.into()],
            advance,
            &[slot.into(), probe_count.into(), available_slot.into()],
        );

        function.switch_to_block(advance);
        let slot = function.block_params(advance)[0];
        let probe_count = function.block_params(advance)[1];
        let available_slot = function.block_params(advance)[2];
        let one = function.ins().iconst(types::I32, 1);
        let slot = function.ins().iadd(slot, one);
        let slot = function.ins().urem(slot, table_capacity);
        let probe_count = function.ins().iadd(probe_count, one);
        function.ins().jump(
            probe,
            &[slot.into(), probe_count.into(), available_slot.into()],
        );

        function.switch_to_block(replace);
        let entry_record = function.block_params(replace)[0];
        let slot = function.block_params(replace)[1];
        let path = function.ins().iadd_imm(
            entry_record,
            mem::offset_of!(GreedyEntryRecord, path) as i64,
        );
        store_current_path(&mut function, task, path, target_pc);
        let slot = function.ins().uextend(types::I64, slot);
        function.ins().return_(&[slot]);

        function.switch_to_block(insert);
        let slot = function.block_params(insert)[0];
        let entries = context_pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, greedy_entries),
        );
        let entry_record = pointer_at(
            &mut function,
            entries,
            slot,
            mem::size_of::<GreedyEntryRecord>(),
        );
        let one = function.ins().iconst(types::I32, 1);
        for (offset, value) in [
            (mem::offset_of!(GreedyEntryRecord, occupied), one),
            (mem::offset_of!(GreedyEntryRecord, hash), hash),
            (mem::offset_of!(GreedyEntryRecord, pc), target_pc),
            (mem::offset_of!(GreedyEntryRecord, start), start),
            (mem::offset_of!(GreedyEntryRecord, captures), captures),
            (mem::offset_of!(GreedyEntryRecord, cursors), cursors),
            (mem::offset_of!(GreedyEntryRecord, repeats), repeats),
        ] {
            function
                .ins()
                .store(MemFlags::trusted(), value, entry_record, offset as i32);
        }
        let path = function.ins().iadd_imm(
            entry_record,
            mem::offset_of!(GreedyEntryRecord, path) as i64,
        );
        store_current_path(&mut function, task, path, target_pc);
        let slot = function.ins().uextend(types::I64, slot);
        let inserted_bit = function.ins().iconst(types::I64, 1_i64 << 32);
        let outcome = function.ins().bor(slot, inserted_bit);
        function.ins().return_(&[outcome]);

        function.switch_to_block(capacity);
        let failed = function.ins().iconst(types::I64, -1);
        function.ins().return_(&[failed]);
        function.seal_all_blocks();
        function.finalize();
    }
    super::native_cpu_codegen::define_function(module, id, &mut context)?;
    module.clear_context(&mut context);
    Ok(())
}

fn define_greedy_take(
    module: &mut JITModule,
    id: FuncId,
) -> Result<(), Box<cranelift_module::ModuleError>> {
    let mut context = module.make_context();
    context.func.signature = module.make_signature();
    context
        .func
        .signature
        .params
        .push(AbiParam::new(module.target_config().pointer_type()));
    context
        .func
        .signature
        .params
        .push(AbiParam::new(types::I32));
    context
        .func
        .signature
        .returns
        .push(AbiParam::new(types::I64));
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = function.create_block();
        function.append_block_params_for_function_params(entry);
        let inspect = function.create_block();
        let take = function.create_block();
        function.append_block_param(
            take,
            function
                .func
                .dfg
                .value_type(function.block_params(entry)[0]),
        );
        let stale = function.create_block();
        function.switch_to_block(entry);
        let task = function.block_params(entry)[0];
        let slot = function.block_params(entry)[1];
        let capacity = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, greedy_entry_quota_limit) as i32,
        );
        let valid_slot = function.ins().icmp(IntCC::UnsignedLessThan, slot, capacity);
        function.ins().brif(valid_slot, inspect, &[], stale, &[]);
        function.switch_to_block(inspect);
        let entries = context_pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, greedy_entries),
        );
        let entry_record = pointer_at(
            &mut function,
            entries,
            slot,
            mem::size_of::<GreedyEntryRecord>(),
        );
        let occupied = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            entry_record,
            mem::offset_of!(GreedyEntryRecord, occupied) as i32,
        );
        let live = function.ins().icmp_imm(IntCC::Equal, occupied, 1);
        function
            .ins()
            .brif(live, take, &[entry_record.into()], stale, &[]);
        function.switch_to_block(take);
        let entry_record = function.block_params(take)[0];
        let tombstone = function.ins().iconst(types::I32, 2);
        function.ins().store(
            MemFlags::trusted(),
            tombstone,
            entry_record,
            mem::offset_of!(GreedyEntryRecord, occupied) as i32,
        );
        let path = function.ins().iadd_imm(
            entry_record,
            mem::offset_of!(GreedyEntryRecord, path) as i64,
        );
        let pc = restore_path(&mut function, task, path);
        let pc = function.ins().uextend(types::I64, pc);
        function.ins().return_(&[pc]);
        function.switch_to_block(stale);
        let none = function.ins().iconst(types::I64, -1);
        function.ins().return_(&[none]);
        function.seal_all_blocks();
        function.finalize();
    }
    super::native_cpu_codegen::define_function(module, id, &mut context)?;
    module.clear_context(&mut context);
    Ok(())
}

fn define_path_enqueue(
    module: &mut JITModule,
    id: FuncId,
    queue_insert: FuncId,
) -> Result<(), Box<cranelift_module::ModuleError>> {
    let mut context = module.make_context();
    context.func.signature = module.make_signature();
    context
        .func
        .signature
        .params
        .push(AbiParam::new(module.target_config().pointer_type()));
    for _ in 0..5 {
        context
            .func
            .signature
            .params
            .push(AbiParam::new(types::I32));
    }
    context
        .func
        .signature
        .returns
        .push(AbiParam::new(types::I32));
    let queue_insert = module.declare_func_in_func(queue_insert, &mut context.func);
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = function.create_block();
        function.append_block_params_for_function_params(entry);
        let queue = function.create_block();
        let append = function.create_block();
        let queue_outcome = function.create_block();
        function.append_block_param(queue_outcome, types::I32);
        let write = function.create_block();
        let duplicate = function.create_block();
        let capacity = function.create_block();

        function.switch_to_block(entry);
        let task = function.block_params(entry)[0];
        let pc = function.block_params(entry)[1];
        let node = function.block_params(entry)[2];
        let mode = function.block_params(entry)[3];
        let sibling = function.block_params(entry)[4];
        let kind = function.block_params(entry)[5];
        let query_capacity = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, query_bucket_capacity) as i32,
        );
        let query_enabled = function.ins().icmp_imm(IntCC::NotEqual, query_capacity, 0);
        function.ins().brif(query_enabled, queue, &[], append, &[]);

        function.switch_to_block(queue);
        let old_node = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, current_node) as i32,
        );
        function.ins().store(
            MemFlags::trusted(),
            node,
            task,
            mem::offset_of!(NativeTaskContext, current_node) as i32,
        );
        let call = function.ins().call(queue_insert, &[task, pc, sibling]);
        let outcome = function.inst_results(call)[0];
        function.ins().store(
            MemFlags::trusted(),
            old_node,
            task,
            mem::offset_of!(NativeTaskContext, current_node) as i32,
        );
        function.ins().jump(queue_outcome, &[outcome.into()]);

        function.switch_to_block(queue_outcome);
        let outcome = function.block_params(queue_outcome)[0];
        let inserted = function.ins().icmp_imm(IntCC::Equal, outcome, 1);
        let not_inserted = function.create_block();
        function
            .ins()
            .brif(inserted, append, &[], not_inserted, &[]);
        function.switch_to_block(not_inserted);
        let is_duplicate = function.ins().icmp_imm(IntCC::Equal, outcome, 0);
        function
            .ins()
            .brif(is_duplicate, duplicate, &[], capacity, &[]);

        function.switch_to_block(append);
        let count = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, pending_count) as i32,
        );
        let pending_quota_limit = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, pending_quota_limit) as i32,
        );
        let available = function
            .ins()
            .icmp(IntCC::UnsignedLessThan, count, pending_quota_limit);
        function.ins().brif(available, write, &[], capacity, &[]);

        function.switch_to_block(write);
        let pending = context_pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, pending),
        );
        let record = pointer_at(
            &mut function,
            pending,
            count,
            mem::size_of::<PendingRecord>(),
        );
        function.ins().store(
            MemFlags::trusted(),
            kind,
            record,
            mem::offset_of!(PendingRecord, kind) as i32,
        );
        let path = function
            .ins()
            .iadd_imm(record, mem::offset_of!(PendingRecord, path) as i64);
        store_current_path(&mut function, task, path, pc);
        function.ins().store(
            MemFlags::trusted(),
            node,
            path,
            mem::offset_of!(PathRecord, node) as i32,
        );
        function.ins().store(
            MemFlags::trusted(),
            mode,
            path,
            mem::offset_of!(PathRecord, following) as i32,
        );
        let next_count = function.ins().iadd_imm(count, 1);
        function.ins().store(
            MemFlags::trusted(),
            next_count,
            task,
            mem::offset_of!(NativeTaskContext, pending_count) as i32,
        );
        let one = function.ins().iconst(types::I32, 1);
        function.ins().return_(&[one]);

        function.switch_to_block(duplicate);
        let zero = function.ins().iconst(types::I32, 0);
        function.ins().return_(&[zero]);
        function.switch_to_block(capacity);
        let two = function.ins().iconst(types::I32, 2);
        function.ins().return_(&[two]);
        function.seal_all_blocks();
        function.finalize();
    }
    super::native_cpu_codegen::define_function(module, id, &mut context)?;
    module.clear_context(&mut context);
    Ok(())
}

fn define_query_admit(
    module: &mut JITModule,
    id: FuncId,
    cursor_equal: FuncId,
    repeat_equal: FuncId,
    capture_contains: FuncId,
) -> Result<(), Box<cranelift_module::ModuleError>> {
    let mut context = module.make_context();
    context.func.signature = module.make_signature();
    context
        .func
        .signature
        .params
        .push(AbiParam::new(module.target_config().pointer_type()));
    context
        .func
        .signature
        .params
        .push(AbiParam::new(types::I32));
    context
        .func
        .signature
        .params
        .push(AbiParam::new(types::I32));
    context
        .func
        .signature
        .returns
        .push(AbiParam::new(types::I32));
    let cursor_equal = module.declare_func_in_func(cursor_equal, &mut context.func);
    let repeat_equal = module.declare_func_in_func(repeat_equal, &mut context.func);
    let capture_contains = module.declare_func_in_func(capture_contains, &mut context.func);
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = function.create_block();
        function.append_block_params_for_function_params(entry);
        let start_probe = function.create_block();
        let probe = function.create_block();
        function.append_block_param(probe, types::I32);
        function.append_block_param(probe, types::I32);
        let check_probe = function.create_block();
        function.append_block_param(check_probe, types::I32);
        function.append_block_param(check_probe, types::I32);
        let inspect = function.create_block();
        function.append_block_param(inspect, types::I32);
        function.append_block_param(inspect, types::I32);
        let initialize = function.create_block();
        function.append_block_param(initialize, types::I32);
        let compare_hash = function.create_block();
        function.append_block_param(compare_hash, types::I32);
        function.append_block_param(compare_hash, types::I32);
        let compare_key = function.create_block();
        function.append_block_param(
            compare_key,
            function
                .func
                .dfg
                .value_type(function.block_params(entry)[0]),
        );
        function.append_block_param(compare_key, types::I32);
        function.append_block_param(compare_key, types::I32);
        let advance = function.create_block();
        function.append_block_param(advance, types::I32);
        function.append_block_param(advance, types::I32);
        let found = function.create_block();
        function.append_block_param(
            found,
            function
                .func
                .dfg
                .value_type(function.block_params(entry)[0]),
        );
        let members = function.create_block();
        function.append_block_param(
            members,
            function
                .func
                .dfg
                .value_type(function.block_params(entry)[0]),
        );
        function.append_block_param(members, types::I32);
        let inspect_member = function.create_block();
        function.append_block_param(
            inspect_member,
            function
                .func
                .dfg
                .value_type(function.block_params(entry)[0]),
        );
        function.append_block_param(inspect_member, types::I32);
        let append = function.create_block();
        function.append_block_param(
            append,
            function
                .func
                .dfg
                .value_type(function.block_params(entry)[0]),
        );
        let check_member = function.create_block();
        function.append_block_param(
            check_member,
            function
                .func
                .dfg
                .value_type(function.block_params(entry)[0]),
        );
        function.append_block_param(check_member, types::I32);
        let accepted = function.create_block();
        let rejected = function.create_block();
        let capacity = function.create_block();
        let work_limit = function.create_block();

        function.switch_to_block(entry);
        let task = function.block_params(entry)[0];
        let pc = function.block_params(entry)[1];
        let sibling = function.block_params(entry)[2];
        let bucket_capacity = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, query_bucket_capacity) as i32,
        );
        let has_capacity = function.ins().icmp_imm(IntCC::NotEqual, bucket_capacity, 0);
        function
            .ins()
            .brif(has_capacity, start_probe, &[], capacity, &[]);

        function.switch_to_block(start_probe);
        let node = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, current_node) as i32,
        );
        let captures = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, current_captures) as i32,
        );
        let cursors = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, current_cursors) as i32,
        );
        let repeats = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, current_repeats) as i32,
        );
        let cursor_hash = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, current_cursor_hash) as i32,
        );
        let repeat_hash = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, current_repeat_hash) as i32,
        );
        let capture_hash = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, current_capture_hash) as i32,
        );
        let hash = mix(&mut function, pc, node);
        let hash = mix(&mut function, hash, cursor_hash);
        let hash = mix(&mut function, hash, repeat_hash);
        let hash = mix(&mut function, hash, sibling);
        let slot = function.ins().urem(hash, bucket_capacity);
        let zero = function.ins().iconst(types::I32, 0);
        function.ins().jump(probe, &[slot.into(), zero.into()]);

        function.switch_to_block(probe);
        let slot = function.block_params(probe)[0];
        let probe_count = function.block_params(probe)[1];
        let within_work = charge_task_work(&mut function, task);
        function.ins().brif(
            within_work,
            check_probe,
            &[slot.into(), probe_count.into()],
            work_limit,
            &[],
        );

        function.switch_to_block(check_probe);
        let slot = function.block_params(check_probe)[0];
        let probe_count = function.block_params(check_probe)[1];
        let in_range = function
            .ins()
            .icmp(IntCC::UnsignedLessThan, probe_count, bucket_capacity);
        function.ins().brif(
            in_range,
            inspect,
            &[slot.into(), probe_count.into()],
            capacity,
            &[],
        );

        function.switch_to_block(inspect);
        let slot = function.block_params(inspect)[0];
        let probe_count = function.block_params(inspect)[1];
        let buckets = context_pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, query_buckets),
        );
        let bucket = pointer_at(
            &mut function,
            buckets,
            slot,
            mem::size_of::<QueryBucketRecord>(),
        );
        let occupied = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            bucket,
            mem::offset_of!(QueryBucketRecord, occupied) as i32,
        );
        // Evaluations reuse these arenas for every root. The bucket's links
        // point into task-local histories, so a previous task is never a hit.
        let task_ordinal = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, task_ordinal) as i32,
        );
        let epoch = function.ins().iadd_imm(task_ordinal, 1);
        let empty = function.ins().icmp(IntCC::NotEqual, occupied, epoch);
        function.ins().brif(
            empty,
            initialize,
            &[slot.into()],
            compare_hash,
            &[slot.into(), probe_count.into()],
        );

        function.switch_to_block(initialize);
        let slot = function.block_params(initialize)[0];
        let buckets = context_pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, query_buckets),
        );
        let bucket = pointer_at(
            &mut function,
            buckets,
            slot,
            mem::size_of::<QueryBucketRecord>(),
        );
        let zero = function.ins().iconst(types::I32, 0);
        for (offset, value) in [
            (mem::offset_of!(QueryBucketRecord, occupied), epoch),
            (mem::offset_of!(QueryBucketRecord, hash), hash),
            (mem::offset_of!(QueryBucketRecord, pc), pc),
            (mem::offset_of!(QueryBucketRecord, node), node),
            (
                mem::offset_of!(QueryBucketRecord, sibling_requirement),
                sibling,
            ),
            (mem::offset_of!(QueryBucketRecord, cursors), cursors),
            (mem::offset_of!(QueryBucketRecord, repeats), repeats),
            (mem::offset_of!(QueryBucketRecord, first_member), zero),
        ] {
            function
                .ins()
                .store(MemFlags::trusted(), value, bucket, offset as i32);
        }
        function.ins().jump(found, &[bucket.into()]);

        function.switch_to_block(compare_hash);
        let slot = function.block_params(compare_hash)[0];
        let probe_count = function.block_params(compare_hash)[1];
        let buckets = context_pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, query_buckets),
        );
        let bucket = pointer_at(
            &mut function,
            buckets,
            slot,
            mem::size_of::<QueryBucketRecord>(),
        );
        let old_hash = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            bucket,
            mem::offset_of!(QueryBucketRecord, hash) as i32,
        );
        let same_hash = function.ins().icmp(IntCC::Equal, old_hash, hash);
        function.ins().brif(
            same_hash,
            compare_key,
            &[bucket.into(), slot.into(), probe_count.into()],
            advance,
            &[slot.into(), probe_count.into()],
        );

        function.switch_to_block(compare_key);
        let bucket = function.block_params(compare_key)[0];
        let slot = function.block_params(compare_key)[1];
        let probe_count = function.block_params(compare_key)[2];
        let old_pc = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            bucket,
            mem::offset_of!(QueryBucketRecord, pc) as i32,
        );
        let old_node = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            bucket,
            mem::offset_of!(QueryBucketRecord, node) as i32,
        );
        let old_sibling = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            bucket,
            mem::offset_of!(QueryBucketRecord, sibling_requirement) as i32,
        );
        let pc_equal = function.ins().icmp(IntCC::Equal, old_pc, pc);
        let node_equal = function.ins().icmp(IntCC::Equal, old_node, node);
        let sibling_equal = function.ins().icmp(IntCC::Equal, old_sibling, sibling);
        let scalar_equal = function.ins().band(pc_equal, node_equal);
        let scalar_equal = function.ins().band(scalar_equal, sibling_equal);
        let compare_links = function.create_block();
        function.append_block_param(compare_links, function.func.dfg.value_type(bucket));
        function.append_block_param(compare_links, types::I32);
        function.append_block_param(compare_links, types::I32);
        function.ins().brif(
            scalar_equal,
            compare_links,
            &[bucket.into(), slot.into(), probe_count.into()],
            advance,
            &[slot.into(), probe_count.into()],
        );
        function.switch_to_block(compare_links);
        let bucket = function.block_params(compare_links)[0];
        let slot = function.block_params(compare_links)[1];
        let probe_count = function.block_params(compare_links)[2];
        let old_cursors = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            bucket,
            mem::offset_of!(QueryBucketRecord, cursors) as i32,
        );
        let old_repeats = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            bucket,
            mem::offset_of!(QueryBucketRecord, repeats) as i32,
        );
        let cursor_call = function
            .ins()
            .call(cursor_equal, &[task, old_cursors, cursors]);
        let cursor_same = function.inst_results(cursor_call)[0];
        let repeat_call = function
            .ins()
            .call(repeat_equal, &[task, old_repeats, repeats]);
        let repeat_same = function.inst_results(repeat_call)[0];
        let links_same = function.ins().band(cursor_same, repeat_same);
        let links_same = function.ins().icmp_imm(IntCC::NotEqual, links_same, 0);
        function.ins().brif(
            links_same,
            found,
            &[bucket.into()],
            advance,
            &[slot.into(), probe_count.into()],
        );

        function.switch_to_block(advance);
        let slot = function.block_params(advance)[0];
        let probe_count = function.block_params(advance)[1];
        let one = function.ins().iconst(types::I32, 1);
        let slot = function.ins().iadd(slot, one);
        let slot = function.ins().urem(slot, bucket_capacity);
        let probe_count = function.ins().iadd(probe_count, one);
        function
            .ins()
            .jump(probe, &[slot.into(), probe_count.into()]);

        function.switch_to_block(found);
        let bucket = function.block_params(found)[0];
        let first_member = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            bucket,
            mem::offset_of!(QueryBucketRecord, first_member) as i32,
        );
        function
            .ins()
            .jump(members, &[bucket.into(), first_member.into()]);

        function.switch_to_block(members);
        let bucket = function.block_params(members)[0];
        let member = function.block_params(members)[1];
        let has_member = function.ins().icmp_imm(IntCC::NotEqual, member, 0);
        function.ins().brif(
            has_member,
            inspect_member,
            &[bucket.into(), member.into()],
            append,
            &[bucket.into()],
        );

        function.switch_to_block(inspect_member);
        let bucket = function.block_params(inspect_member)[0];
        let member = function.block_params(inspect_member)[1];
        let within_work = charge_task_work(&mut function, task);
        function.ins().brif(
            within_work,
            check_member,
            &[bucket.into(), member.into()],
            work_limit,
            &[],
        );

        function.switch_to_block(check_member);
        let bucket = function.block_params(check_member)[0];
        let member = function.block_params(check_member)[1];
        let member_index = function.ins().iadd_imm(member, -1);
        let member_records = context_pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, query_members),
        );
        let member_record = pointer_at(
            &mut function,
            member_records,
            member_index,
            mem::size_of::<QueryMemberRecord>(),
        );
        let old_captures = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            member_record,
            mem::offset_of!(QueryMemberRecord, captures) as i32,
        );
        let contains_call = function
            .ins()
            .call(capture_contains, &[task, old_captures, captures]);
        let contains = function.inst_results(contains_call)[0];
        let does_contain = function.ins().icmp_imm(IntCC::NotEqual, contains, 0);
        let next_member = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            member_record,
            mem::offset_of!(QueryMemberRecord, next_member) as i32,
        );
        let reverse_call = function
            .ins()
            .call(capture_contains, &[task, captures, old_captures]);
        let reverse_contains = function.inst_results(reverse_call)[0];
        let reverse_contains = function
            .ins()
            .icmp_imm(IntCC::NotEqual, reverse_contains, 0);
        let equal = function.ins().band(does_contain, reverse_contains);
        function.ins().brif(
            equal,
            rejected,
            &[],
            members,
            &[bucket.into(), next_member.into()],
        );

        function.switch_to_block(append);
        let bucket = function.block_params(append)[0];
        let member_count = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, query_member_count) as i32,
        );
        let member_capacity = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, query_member_quota_limit) as i32,
        );
        let available = function
            .ins()
            .icmp(IntCC::UnsignedLessThan, member_count, member_capacity);
        let count_captures = function.create_block();
        function.append_block_param(count_captures, function.func.dfg.value_type(bucket));
        function.append_block_param(count_captures, types::I32);
        function.append_block_param(count_captures, types::I32);
        let inspect_capture = function.create_block();
        function.append_block_param(inspect_capture, function.func.dfg.value_type(bucket));
        function.append_block_param(inspect_capture, types::I32);
        function.append_block_param(inspect_capture, types::I32);
        let write_member = function.create_block();
        function.append_block_param(write_member, function.func.dfg.value_type(bucket));
        function.append_block_param(write_member, types::I32);
        let zero = function.ins().iconst(types::I32, 0);
        function.ins().brif(
            available,
            count_captures,
            &[bucket.into(), captures.into(), zero.into()],
            capacity,
            &[],
        );
        function.switch_to_block(count_captures);
        let bucket = function.block_params(count_captures)[0];
        let capture_head = function.block_params(count_captures)[1];
        let capture_count = function.block_params(count_captures)[2];
        let has_capture = function.ins().icmp_imm(IntCC::NotEqual, capture_head, 0);
        function.ins().brif(
            has_capture,
            inspect_capture,
            &[bucket.into(), capture_head.into(), capture_count.into()],
            write_member,
            &[bucket.into(), capture_count.into()],
        );
        function.switch_to_block(inspect_capture);
        let bucket = function.block_params(inspect_capture)[0];
        let capture_head = function.block_params(inspect_capture)[1];
        let capture_count = function.block_params(inspect_capture)[2];
        let capture_records = context_pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, captures),
        );
        let capture_index = function.ins().iadd_imm(capture_head, -1);
        let capture_record = pointer_at(
            &mut function,
            capture_records,
            capture_index,
            mem::size_of::<LinkRecord>(),
        );
        let previous = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            capture_record,
            mem::offset_of!(LinkRecord, previous) as i32,
        );
        let capture_count = function.ins().iadd_imm(capture_count, 1);
        function.ins().jump(
            count_captures,
            &[bucket.into(), previous.into(), capture_count.into()],
        );
        function.switch_to_block(write_member);
        let bucket = function.block_params(write_member)[0];
        let capture_count = function.block_params(write_member)[1];
        let member_records = context_pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, query_members),
        );
        let member_record = pointer_at(
            &mut function,
            member_records,
            member_count,
            mem::size_of::<QueryMemberRecord>(),
        );
        let first_member = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            bucket,
            mem::offset_of!(QueryBucketRecord, first_member) as i32,
        );
        for (offset, value) in [
            (
                mem::offset_of!(QueryMemberRecord, next_member),
                first_member,
            ),
            (mem::offset_of!(QueryMemberRecord, captures), captures),
            (
                mem::offset_of!(QueryMemberRecord, capture_count),
                capture_count,
            ),
            (
                mem::offset_of!(QueryMemberRecord, capture_hash),
                capture_hash,
            ),
        ] {
            function
                .ins()
                .store(MemFlags::trusted(), value, member_record, offset as i32);
        }
        let member_count = function.ins().iadd_imm(member_count, 1);
        function.ins().store(
            MemFlags::trusted(),
            member_count,
            task,
            mem::offset_of!(NativeTaskContext, query_member_count) as i32,
        );
        function.ins().store(
            MemFlags::trusted(),
            member_count,
            bucket,
            mem::offset_of!(QueryBucketRecord, first_member) as i32,
        );
        function.ins().jump(accepted, &[]);

        function.switch_to_block(accepted);
        let one = function.ins().iconst(types::I32, 1);
        function.ins().return_(&[one]);
        function.switch_to_block(rejected);
        let zero = function.ins().iconst(types::I32, 0);
        function.ins().return_(&[zero]);
        function.switch_to_block(capacity);
        let two = function.ins().iconst(types::I32, 2);
        function.ins().return_(&[two]);
        function.switch_to_block(work_limit);
        let three = function.ins().iconst(types::I32, 3);
        function.ins().return_(&[three]);
        function.seal_all_blocks();
        function.finalize();
    }
    super::native_cpu_codegen::define_function(module, id, &mut context)?;
    module.clear_context(&mut context);
    Ok(())
}

pub(super) fn define(
    module: &mut JITModule,
) -> Result<SchedulerFunctions, Box<cranelift_module::ModuleError>> {
    let cursor_value = declare(module, "tempo_cursor_value", 3)?;
    let cursor_equal = declare(module, "tempo_cursor_equal", 3)?;
    let repeat_equal = declare(module, "tempo_repeat_equal", 3)?;
    let capture_contains = declare(module, "tempo_capture_contains", 3)?;
    let query_admit = declare(module, "tempo_query_admit", 3)?;
    let exact_state_equal = declare(module, "tempo_exact_state_equal", 7)?;
    let queue_insert = declare(module, "tempo_queue_insert", 3)?;
    let queue_remove = declare(module, "tempo_queue_remove", 3)?;
    let greedy_schedule = declare_i64(module, "tempo_greedy_schedule", 2)?;
    let greedy_take = declare_i64(module, "tempo_greedy_take", 2)?;
    let path_enqueue = declare(module, "tempo_path_enqueue", 6)?;
    define_cursor_value(module, cursor_value)?;
    define_cursor_equal(module, cursor_equal, cursor_value)?;
    define_repeat_equal(module, repeat_equal)?;
    capture_relation::define(module, capture_contains)?;
    define_exact_state_equal(
        module,
        exact_state_equal,
        cursor_equal,
        repeat_equal,
        capture_contains,
    )?;
    define_query_admit(
        module,
        query_admit,
        cursor_equal,
        repeat_equal,
        capture_contains,
    )?;
    define_queue_insert(module, queue_insert, exact_state_equal)?;
    define_queue_remove(module, queue_remove, exact_state_equal)?;
    define_greedy_schedule(module, greedy_schedule, exact_state_equal)?;
    define_greedy_take(module, greedy_take)?;
    define_path_enqueue(module, path_enqueue, queue_insert)?;
    Ok(SchedulerFunctions {
        cursor_value,
        cursor_equal,
        repeat_equal,
        capture_contains,
        query_admit,
        exact_state_equal,
        queue_insert,
        queue_remove,
        greedy_schedule,
        greedy_take,
        path_enqueue,
    })
}
