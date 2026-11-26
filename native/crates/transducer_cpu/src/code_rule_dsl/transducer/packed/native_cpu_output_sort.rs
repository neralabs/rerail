use super::{
    metal_batch_abi::{FinalCaptureRecord, NONE, ResultRecord},
    native_cpu::NativeTaskContext,
};
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlags, Value, condcodes::IntCC, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Linkage, Module};
use std::mem;

fn pointer(function: &mut FunctionBuilder<'_>, base: Value, offset: usize) -> Value {
    let ty = function.func.dfg.value_type(base);
    function
        .ins()
        .load(ty, MemFlags::trusted(), base, offset as i32)
}

fn record(function: &mut FunctionBuilder<'_>, task: Value, index: Value) -> Value {
    let ty = function.func.dfg.value_type(task);
    let base = pointer(
        function,
        task,
        mem::offset_of!(NativeTaskContext, final_captures),
    );
    let offset = function
        .ins()
        .imul_imm(index, mem::size_of::<FinalCaptureRecord>() as i64);
    let offset = function.ins().uextend(ty, offset);
    function.ins().iadd(base, offset)
}

const FIELDS: [usize; 9] = [
    mem::offset_of!(FinalCaptureRecord, label),
    mem::offset_of!(FinalCaptureRecord, node),
    mem::offset_of!(FinalCaptureRecord, start),
    mem::offset_of!(FinalCaptureRecord, end),
    mem::offset_of!(FinalCaptureRecord, text_start),
    mem::offset_of!(FinalCaptureRecord, text_length),
    mem::offset_of!(FinalCaptureRecord, property_start),
    mem::offset_of!(FinalCaptureRecord, property_count),
    mem::offset_of!(FinalCaptureRecord, retained),
];

pub(super) fn define(
    module: &mut JITModule,
    query: bool,
) -> Result<FuncId, Box<cranelift_module::ModuleError>> {
    let pointer_type = module.target_config().pointer_type();
    let mut signature = module.make_signature();
    signature.params.push(AbiParam::new(pointer_type));
    signature.params.push(AbiParam::new(types::I32));
    signature.params.push(AbiParam::new(types::I32));
    let id = module.declare_function("tempo_sort_final_captures", Linkage::Local, &signature)?;
    let mut context = module.make_context();
    context.func.signature = signature;
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = function.create_block();
        function.append_block_params_for_function_params(entry);
        let outer = function.create_block();
        function.append_block_param(outer, types::I32);
        let load_value = function.create_block();
        function.append_block_param(load_value, types::I32);
        let shift = function.create_block();
        function.append_block_param(shift, types::I32);
        for _ in FIELDS {
            function.append_block_param(shift, types::I32);
        }
        let inspect = function.create_block();
        function.append_block_param(inspect, types::I32);
        for _ in FIELDS {
            function.append_block_param(inspect, types::I32);
        }
        let move_previous = function.create_block();
        function.append_block_param(move_previous, types::I32);
        for _ in FIELDS {
            function.append_block_param(move_previous, types::I32);
        }
        let place = function.create_block();
        function.append_block_param(place, types::I32);
        for _ in FIELDS {
            function.append_block_param(place, types::I32);
        }
        let done = function.create_block();

        function.switch_to_block(entry);
        let params = function.block_params(entry).to_vec();
        let task = params[0];
        let first = params[1];
        let count = params[2];
        let one = function.ins().iconst(types::I32, 1);
        function.ins().jump(outer, &[one.into()]);

        function.switch_to_block(outer);
        let index = function.block_params(outer)[0];
        let complete = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, index, count);
        function
            .ins()
            .brif(complete, done, &[], load_value, &[index.into()]);

        function.switch_to_block(load_value);
        let index = function.block_params(load_value)[0];
        let absolute = function.ins().iadd(first, index);
        let source = record(&mut function, task, absolute);
        let mut args = Vec::with_capacity(10);
        args.push(index.into());
        for offset in FIELDS {
            let value = function
                .ins()
                .load(types::I32, MemFlags::trusted(), source, offset as i32);
            args.push(value.into());
        }
        function.ins().jump(shift, &args);

        function.switch_to_block(shift);
        let values = function.block_params(shift).to_vec();
        let at_start = function.ins().icmp_imm(IntCC::Equal, values[0], 0);
        let place_args = values.iter().copied().map(Into::into).collect::<Vec<_>>();
        let inspect_args = place_args.clone();
        function
            .ins()
            .brif(at_start, place, &place_args, inspect, &inspect_args);

        function.switch_to_block(inspect);
        let values = function.block_params(inspect).to_vec();
        let previous_index = function.ins().iadd_imm(values[0], -1);
        let previous_absolute = function.ins().iadd(first, previous_index);
        let previous = record(&mut function, task, previous_absolute);
        let previous_start = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            previous,
            mem::offset_of!(FinalCaptureRecord, start) as i32,
        );
        let previous_end = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            previous,
            mem::offset_of!(FinalCaptureRecord, end) as i32,
        );
        let start_after =
            function
                .ins()
                .icmp(IntCC::UnsignedGreaterThan, previous_start, values[3]);
        let start_equal = function.ins().icmp(IntCC::Equal, previous_start, values[3]);
        let end_before = function
            .ins()
            .icmp(IntCC::UnsignedLessThan, previous_end, values[4]);
        let end_after = if query {
            end_before
        } else {
            let previous_label = function.ins().load(
                types::I32,
                MemFlags::trusted(),
                previous,
                mem::offset_of!(FinalCaptureRecord, label) as i32,
            );
            let end_equal = function.ins().icmp(IntCC::Equal, previous_end, values[4]);
            let label_after =
                function
                    .ins()
                    .icmp(IntCC::UnsignedGreaterThan, previous_label, values[1]);
            let equal_and_label = function.ins().band(end_equal, label_after);
            function.ins().bor(end_before, equal_and_label)
        };
        let tie_after = function.ins().band(start_equal, end_after);
        let after = function.ins().bor(start_after, tie_after);
        let args = values.iter().copied().map(Into::into).collect::<Vec<_>>();
        function
            .ins()
            .brif(after, move_previous, &args, place, &args);

        function.switch_to_block(move_previous);
        let values = function.block_params(move_previous).to_vec();
        let previous_index = function.ins().iadd_imm(values[0], -1);
        let previous_absolute = function.ins().iadd(first, previous_index);
        let previous = record(&mut function, task, previous_absolute);
        let current_absolute = function.ins().iadd(first, values[0]);
        let current = record(&mut function, task, current_absolute);
        for offset in FIELDS {
            let value =
                function
                    .ins()
                    .load(types::I32, MemFlags::trusted(), previous, offset as i32);
            function
                .ins()
                .store(MemFlags::trusted(), value, current, offset as i32);
        }
        let mut args = Vec::with_capacity(10);
        args.push(previous_index.into());
        args.extend(
            values[1..]
                .iter()
                .copied()
                .map(cranelift_codegen::ir::BlockArg::from),
        );
        function.ins().jump(shift, &args);

        function.switch_to_block(place);
        let values = function.block_params(place).to_vec();
        let absolute = function.ins().iadd(first, values[0]);
        let target = record(&mut function, task, absolute);
        for (offset, value) in FIELDS.into_iter().zip(values[1..].iter().copied()) {
            function
                .ins()
                .store(MemFlags::trusted(), value, target, offset as i32);
        }
        let next = function.ins().iadd_imm(index, 1);
        function.ins().jump(outer, &[next.into()]);

        function.switch_to_block(done);
        function.ins().return_(&[]);
        function.seal_all_blocks();
        function.finalize();
    }
    super::native_cpu_codegen::define_function(module, id, &mut context)?;
    module.clear_context(&mut context);
    Ok(id)
}

const RESULT_FIELDS: [usize; 10] = [
    mem::offset_of!(ResultRecord, task_ordinal),
    mem::offset_of!(ResultRecord, local_ordinal),
    mem::offset_of!(ResultRecord, start),
    mem::offset_of!(ResultRecord, label),
    mem::offset_of!(ResultRecord, capture_start),
    mem::offset_of!(ResultRecord, capture_count),
    mem::offset_of!(ResultRecord, property_start),
    mem::offset_of!(ResultRecord, property_count),
    mem::offset_of!(ResultRecord, dynamic_start),
    mem::offset_of!(ResultRecord, dynamic_count),
];

fn result_record(function: &mut FunctionBuilder<'_>, task: Value, index: Value) -> Value {
    let ty = function.func.dfg.value_type(task);
    let base = pointer(function, task, mem::offset_of!(NativeTaskContext, results));
    let offset = function
        .ins()
        .imul_imm(index, mem::size_of::<ResultRecord>() as i64);
    let offset = function.ins().uextend(ty, offset);
    function.ins().iadd(base, offset)
}

fn define_first_capture_node(
    module: &mut JITModule,
) -> Result<FuncId, Box<cranelift_module::ModuleError>> {
    let pointer_type = module.target_config().pointer_type();
    let mut signature = module.make_signature();
    signature.params.push(AbiParam::new(pointer_type));
    signature.params.push(AbiParam::new(types::I32));
    signature.params.push(AbiParam::new(types::I32));
    signature.returns.push(AbiParam::new(types::I32));
    let id = module.declare_function(
        "tempo_result_first_capture_node",
        Linkage::Local,
        &signature,
    )?;
    let mut context = module.make_context();
    context.func.signature = signature;
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = function.create_block();
        function.append_block_params_for_function_params(entry);
        let present = function.create_block();
        let absent = function.create_block();
        function.switch_to_block(entry);
        let values = function.block_params(entry).to_vec();
        let has = function.ins().icmp_imm(IntCC::NotEqual, values[2], 0);
        function.ins().brif(has, present, &[], absent, &[]);
        function.switch_to_block(present);
        let base = pointer(
            &mut function,
            values[0],
            mem::offset_of!(NativeTaskContext, final_captures),
        );
        let ty = function.func.dfg.value_type(base);
        let offset = function
            .ins()
            .imul_imm(values[1], mem::size_of::<FinalCaptureRecord>() as i64);
        let offset = function.ins().uextend(ty, offset);
        let capture = function.ins().iadd(base, offset);
        let node = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            capture,
            mem::offset_of!(FinalCaptureRecord, node) as i32,
        );
        function.ins().return_(&[node]);
        function.switch_to_block(absent);
        let none = function.ins().iconst(types::I32, NONE as i64);
        function.ins().return_(&[none]);
        function.seal_all_blocks();
        function.finalize();
    }
    super::native_cpu_codegen::define_function(module, id, &mut context)?;
    module.clear_context(&mut context);
    Ok(id)
}

pub(super) fn define_results(
    module: &mut JITModule,
) -> Result<FuncId, Box<cranelift_module::ModuleError>> {
    let first_capture_node = define_first_capture_node(module)?;
    let pointer_type = module.target_config().pointer_type();
    let mut signature = module.make_signature();
    signature.params.push(AbiParam::new(pointer_type));
    signature.params.push(AbiParam::new(types::I32));
    let id = module.declare_function("tempo_sort_final_results", Linkage::Local, &signature)?;
    let mut context = module.make_context();
    context.func.signature = signature;
    let first_capture_node = module.declare_func_in_func(first_capture_node, &mut context.func);
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = function.create_block();
        function.append_block_params_for_function_params(entry);
        let outer = function.create_block();
        function.append_block_param(outer, types::I32);
        let load_value = function.create_block();
        function.append_block_param(load_value, types::I32);
        let shift = function.create_block();
        function.append_block_param(shift, types::I32);
        for _ in RESULT_FIELDS {
            function.append_block_param(shift, types::I32);
        }
        let inspect = function.create_block();
        function.append_block_param(inspect, types::I32);
        for _ in RESULT_FIELDS {
            function.append_block_param(inspect, types::I32);
        }
        let move_previous = function.create_block();
        function.append_block_param(move_previous, types::I32);
        for _ in RESULT_FIELDS {
            function.append_block_param(move_previous, types::I32);
        }
        let place = function.create_block();
        function.append_block_param(place, types::I32);
        for _ in RESULT_FIELDS {
            function.append_block_param(place, types::I32);
        }
        let done = function.create_block();
        function.switch_to_block(entry);
        let params = function.block_params(entry).to_vec();
        let task = params[0];
        let count = params[1];
        let one = function.ins().iconst(types::I32, 1);
        function.ins().jump(outer, &[one.into()]);
        function.switch_to_block(outer);
        let index = function.block_params(outer)[0];
        let complete = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, index, count);
        function
            .ins()
            .brif(complete, done, &[], load_value, &[index.into()]);
        function.switch_to_block(load_value);
        let index = function.block_params(load_value)[0];
        let source = result_record(&mut function, task, index);
        let mut args = Vec::with_capacity(11);
        args.push(index.into());
        for offset in RESULT_FIELDS {
            let value = function
                .ins()
                .load(types::I32, MemFlags::trusted(), source, offset as i32);
            args.push(value.into());
        }
        function.ins().jump(shift, &args);
        function.switch_to_block(shift);
        let values = function.block_params(shift).to_vec();
        let at_start = function.ins().icmp_imm(IntCC::Equal, values[0], 0);
        let args = values.iter().copied().map(Into::into).collect::<Vec<_>>();
        function.ins().brif(at_start, place, &args, inspect, &args);
        function.switch_to_block(inspect);
        let values = function.block_params(inspect).to_vec();
        let previous_index = function.ins().iadd_imm(values[0], -1);
        let previous = result_record(&mut function, task, previous_index);
        let previous_start = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            previous,
            mem::offset_of!(ResultRecord, start) as i32,
        );
        let previous_capture_start = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            previous,
            mem::offset_of!(ResultRecord, capture_start) as i32,
        );
        let previous_capture_count = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            previous,
            mem::offset_of!(ResultRecord, capture_count) as i32,
        );
        let call = function.ins().call(
            first_capture_node,
            &[task, previous_capture_start, previous_capture_count],
        );
        let previous_node = function.inst_results(call)[0];
        let call = function
            .ins()
            .call(first_capture_node, &[task, values[5], values[6]]);
        let value_node = function.inst_results(call)[0];
        let start_after =
            function
                .ins()
                .icmp(IntCC::UnsignedGreaterThan, previous_start, values[3]);
        let start_equal = function.ins().icmp(IntCC::Equal, previous_start, values[3]);
        let node_after = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThan, previous_node, value_node);
        let tie_after = function.ins().band(start_equal, node_after);
        let after = function.ins().bor(start_after, tie_after);
        let args = values.iter().copied().map(Into::into).collect::<Vec<_>>();
        function
            .ins()
            .brif(after, move_previous, &args, place, &args);
        function.switch_to_block(move_previous);
        let values = function.block_params(move_previous).to_vec();
        let previous_index = function.ins().iadd_imm(values[0], -1);
        let previous = result_record(&mut function, task, previous_index);
        let current = result_record(&mut function, task, values[0]);
        for offset in RESULT_FIELDS {
            let value =
                function
                    .ins()
                    .load(types::I32, MemFlags::trusted(), previous, offset as i32);
            function
                .ins()
                .store(MemFlags::trusted(), value, current, offset as i32);
        }
        let mut args = Vec::with_capacity(11);
        args.push(previous_index.into());
        args.extend(
            values[1..]
                .iter()
                .copied()
                .map(cranelift_codegen::ir::BlockArg::from),
        );
        function.ins().jump(shift, &args);
        function.switch_to_block(place);
        let values = function.block_params(place).to_vec();
        let target = result_record(&mut function, task, values[0]);
        for (offset, value) in RESULT_FIELDS.into_iter().zip(values[1..].iter().copied()) {
            function
                .ins()
                .store(MemFlags::trusted(), value, target, offset as i32);
        }
        let next = function.ins().iadd_imm(index, 1);
        function.ins().jump(outer, &[next.into()]);
        function.switch_to_block(done);
        function.ins().return_(&[]);
        function.seal_all_blocks();
        function.finalize();
    }
    super::native_cpu_codegen::define_function(module, id, &mut context)?;
    module.clear_context(&mut context);
    Ok(id)
}
