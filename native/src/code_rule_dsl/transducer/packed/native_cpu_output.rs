use super::{
    metal_batch_abi::{
        FinalCaptureRecord, LinkRecord, MetalNode, NONE, PropertyRecord, ResultRecord, TaskStatus,
    },
    native_cpu::NativeTaskContext,
    native_cpu_output_sort, native_cpu_output_transform,
    native_ir::NativeModule,
    output::OutputInstruction,
};
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlags, condcodes::IntCC, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Linkage, Module};
use std::mem;

fn pointer(
    function: &mut FunctionBuilder<'_>,
    base: cranelift_codegen::ir::Value,
    offset: usize,
) -> cranelift_codegen::ir::Value {
    let ty = function.func.dfg.value_type(base);
    function
        .ins()
        .load(ty, MemFlags::trusted(), base, offset as i32)
}

fn indexed<T>(
    function: &mut FunctionBuilder<'_>,
    base: cranelift_codegen::ir::Value,
    index: cranelift_codegen::ir::Value,
) -> cranelift_codegen::ir::Value {
    let ty = function.func.dfg.value_type(base);
    let offset = function.ins().imul_imm(index, mem::size_of::<T>() as i64);
    let offset = function.ins().uextend(ty, offset);
    function.ins().iadd(base, offset)
}

fn capture_oldest(
    function: &mut FunctionBuilder<'_>,
    task: cranelift_codegen::ir::Value,
    head: cranelift_codegen::ir::Value,
    count: cranelift_codegen::ir::Value,
    ordinal: cranelift_codegen::ir::Value,
) -> (cranelift_codegen::ir::Value, cranelift_codegen::ir::Value) {
    let wanted = function.ins().iadd_imm(count, -1);
    let wanted = function.ins().isub(wanted, ordinal);
    let walk = function.create_block();
    function.append_block_param(walk, types::I32);
    function.append_block_param(walk, types::I32);
    let inspect = function.create_block();
    function.append_block_param(inspect, types::I32);
    function.append_block_param(inspect, types::I32);
    let finish = function.create_block();
    function.append_block_param(finish, types::I32);
    function.append_block_param(finish, types::I32);
    let zero = function.ins().iconst(types::I32, 0);
    function.ins().jump(walk, &[head.into(), zero.into()]);
    function.switch_to_block(walk);
    let values = function.block_params(walk).to_vec();
    let exists = function.ins().icmp_imm(IntCC::NotEqual, values[0], 0);
    let invalid = function.ins().iconst(types::I32, NONE as i64);
    function.ins().brif(
        exists,
        inspect,
        &[values[0].into(), values[1].into()],
        finish,
        &[zero.into(), invalid.into()],
    );
    function.switch_to_block(inspect);
    let values = function.block_params(inspect).to_vec();
    let links = pointer(function, task, mem::offset_of!(NativeTaskContext, captures));
    let index = function.ins().iadd_imm(values[0], -1);
    let link = indexed::<LinkRecord>(function, links, index);
    let previous = function.ins().load(
        types::I32,
        MemFlags::trusted(),
        link,
        mem::offset_of!(LinkRecord, previous) as i32,
    );
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
    let selected = function.ins().icmp(IntCC::Equal, values[1], wanted);
    let advance = function.create_block();
    function.append_block_param(advance, types::I32);
    function.append_block_param(advance, types::I32);
    function.ins().brif(
        selected,
        finish,
        &[label.into(), node.into()],
        advance,
        &[previous.into(), values[1].into()],
    );
    function.switch_to_block(advance);
    let values = function.block_params(advance).to_vec();
    let next = function.ins().iadd_imm(values[1], 1);
    function.ins().jump(walk, &[values[0].into(), next.into()]);
    function.switch_to_block(finish);
    (
        function.block_params(finish)[0],
        function.block_params(finish)[1],
    )
}

fn line_for(
    function: &mut FunctionBuilder<'_>,
    task: cranelift_codegen::ir::Value,
    byte: cranelift_codegen::ir::Value,
) -> cranelift_codegen::ir::Value {
    let low_high = function.create_block();
    function.append_block_param(low_high, types::I32);
    function.append_block_param(low_high, types::I32);
    let inspect = function.create_block();
    function.append_block_param(inspect, types::I32);
    function.append_block_param(inspect, types::I32);
    let move_low = function.create_block();
    function.append_block_param(move_low, types::I32);
    function.append_block_param(move_low, types::I32);
    function.append_block_param(move_low, types::I32);
    let move_high = function.create_block();
    function.append_block_param(move_high, types::I32);
    function.append_block_param(move_high, types::I32);
    function.append_block_param(move_high, types::I32);
    let finish = function.create_block();
    function.append_block_param(finish, types::I32);
    let zero = function.ins().iconst(types::I32, 0);
    let count = function.ins().load(
        types::I32,
        MemFlags::trusted(),
        task,
        mem::offset_of!(NativeTaskContext, line_count) as i32,
    );
    function.ins().jump(low_high, &[zero.into(), count.into()]);
    function.switch_to_block(low_high);
    let values = function.block_params(low_high).to_vec();
    let done = function
        .ins()
        .icmp(IntCC::UnsignedGreaterThanOrEqual, values[0], values[1]);
    let result = function.ins().iadd_imm(values[0], -1);
    function.ins().brif(
        done,
        finish,
        &[result.into()],
        inspect,
        &[values[0].into(), values[1].into()],
    );
    function.switch_to_block(inspect);
    let values = function.block_params(inspect).to_vec();
    let distance = function.ins().isub(values[1], values[0]);
    let half = function.ins().ushr_imm(distance, 1);
    let middle = function.ins().iadd(values[0], half);
    let starts = pointer(
        function,
        task,
        mem::offset_of!(NativeTaskContext, line_starts),
    );
    let address = indexed::<u32>(function, starts, middle);
    let start = function
        .ins()
        .load(types::I32, MemFlags::trusted(), address, 0);
    let before = function
        .ins()
        .icmp(IntCC::UnsignedLessThanOrEqual, start, byte);
    function.ins().brif(
        before,
        move_low,
        &[values[0].into(), values[1].into(), middle.into()],
        move_high,
        &[values[0].into(), values[1].into(), middle.into()],
    );
    function.switch_to_block(move_low);
    let values = function.block_params(move_low).to_vec();
    let low = function.ins().iadd_imm(values[2], 1);
    function
        .ins()
        .jump(low_high, &[low.into(), values[1].into()]);
    function.switch_to_block(move_high);
    let values = function.block_params(move_high).to_vec();
    function
        .ins()
        .jump(low_high, &[values[0].into(), values[2].into()]);
    function.switch_to_block(finish);
    function.block_params(finish)[0]
}

fn byte_for(
    function: &mut FunctionBuilder<'_>,
    task: cranelift_codegen::ir::Value,
    row: cranelift_codegen::ir::Value,
    column: cranelift_codegen::ir::Value,
) -> cranelift_codegen::ir::Value {
    let valid = function.create_block();
    let next_end = function.create_block();
    let file_end = function.create_block();
    let check = function.create_block();
    function.append_block_param(check, types::I32);
    function.append_block_param(check, types::I32);
    let inspect_byte = function.create_block();
    function.append_block_param(inspect_byte, types::I32);
    let accept = function.create_block();
    function.append_block_param(accept, types::I32);
    let reject = function.create_block();
    let finish = function.create_block();
    function.append_block_param(finish, types::I32);
    let row_nonnegative = function
        .ins()
        .icmp_imm(IntCC::SignedGreaterThanOrEqual, row, 0);
    let column_nonnegative = function
        .ins()
        .icmp_imm(IntCC::SignedGreaterThanOrEqual, column, 0);
    let nonnegative = function.ins().band(row_nonnegative, column_nonnegative);
    let count = function.ins().load(
        types::I32,
        MemFlags::trusted(),
        task,
        mem::offset_of!(NativeTaskContext, line_count) as i32,
    );
    let in_range = function.ins().icmp(IntCC::UnsignedLessThan, row, count);
    let valid_position = function.ins().band(nonnegative, in_range);
    function.ins().brif(valid_position, valid, &[], reject, &[]);
    function.switch_to_block(valid);
    let starts = pointer(
        function,
        task,
        mem::offset_of!(NativeTaskContext, line_starts),
    );
    let start_address = indexed::<u32>(function, starts, row);
    let start = function
        .ins()
        .load(types::I32, MemFlags::trusted(), start_address, 0);
    let next_row = function.ins().iadd_imm(row, 1);
    let has_next = function
        .ins()
        .icmp(IntCC::UnsignedLessThan, next_row, count);
    function.ins().brif(has_next, next_end, &[], file_end, &[]);
    function.switch_to_block(next_end);
    let next_address = indexed::<u32>(function, starts, next_row);
    let end = function
        .ins()
        .load(types::I32, MemFlags::trusted(), next_address, 0);
    let end = function.ins().iadd_imm(end, -1);
    function.ins().jump(check, &[start.into(), end.into()]);
    function.switch_to_block(file_end);
    let end = function.ins().load(
        types::I32,
        MemFlags::trusted(),
        task,
        mem::offset_of!(NativeTaskContext, source_length) as i32,
    );
    function.ins().jump(check, &[start.into(), end.into()]);
    function.switch_to_block(check);
    let values = function.block_params(check).to_vec();
    let byte = function.ins().iadd(values[0], column);
    let bounded = function
        .ins()
        .icmp(IntCC::UnsignedLessThanOrEqual, byte, values[1]);
    function
        .ins()
        .brif(bounded, inspect_byte, &[byte.into()], reject, &[]);
    function.switch_to_block(inspect_byte);
    let byte = function.block_params(inspect_byte)[0];
    let source_length = function.ins().load(
        types::I32,
        MemFlags::trusted(),
        task,
        mem::offset_of!(NativeTaskContext, source_length) as i32,
    );
    let at_end = function
        .ins()
        .icmp(IntCC::UnsignedGreaterThanOrEqual, byte, source_length);
    let inspect_scalar = function.create_block();
    function
        .ins()
        .brif(at_end, accept, &[byte.into()], inspect_scalar, &[]);
    function.switch_to_block(inspect_scalar);
    let source = pointer(function, task, mem::offset_of!(NativeTaskContext, source));
    let address = indexed::<u8>(function, source, byte);
    let value = function
        .ins()
        .load(types::I8, MemFlags::trusted(), address, 0);
    let continuation = function.ins().band_imm(value, 0xc0);
    let continuation = function.ins().icmp_imm(IntCC::Equal, continuation, 0x80);
    function
        .ins()
        .brif(continuation, reject, &[], accept, &[byte.into()]);
    function.switch_to_block(accept);
    let byte = function.block_params(accept)[0];
    function.ins().jump(finish, &[byte.into()]);
    function.switch_to_block(reject);
    let none = function.ins().iconst(types::I32, NONE as i64);
    function.ins().jump(finish, &[none.into()]);
    function.switch_to_block(finish);
    function.block_params(finish)[0]
}

pub(super) fn define(
    module: &mut JITModule,
    native: &NativeModule,
    regex_find: Option<FuncId>,
) -> Result<FuncId, Box<cranelift_module::ModuleError>> {
    if native
        .output_program
        .instructions
        .iter()
        .any(|instruction| {
            !matches!(
                instruction,
                OutputInstruction::MatchProperty { .. }
                    | OutputInstruction::CaptureProperty { .. }
                    | OutputInstruction::Offset { .. }
                    | OutputInstruction::Downcase { .. }
                    | OutputInstruction::SelectAdjacent { .. }
                    | OutputInstruction::Strip { .. }
                    | OutputInstruction::LanguageFromMime { .. }
            )
        })
    {
        return Err(Box::new(cranelift_module::ModuleError::Undeclared(
            "output instruction is not lowered".into(),
        )));
    }
    let sort_captures = native_cpu_output_sort::define(module)?;
    let sort_results = native_cpu_output_sort::define_results(module)?;
    let downcase = native_cpu_output_transform::define_downcase(module)?;
    let strip = if native
        .output_program
        .instructions
        .iter()
        .any(|instruction| matches!(instruction, OutputInstruction::Strip { .. }))
    {
        let find = regex_find.ok_or_else(|| {
            Box::new(cranelift_module::ModuleError::Undeclared(
                "strip requires regex helpers".into(),
            ))
        })?;
        Some(native_cpu_output_transform::define_strip(module, find)?)
    } else {
        None
    };
    let language_from_mime = native_cpu_output_transform::define_language_from_mime(module)?;
    let pointer_type = module.target_config().pointer_type();
    let mut signature = module.make_signature();
    signature.params.push(AbiParam::new(pointer_type));
    signature.returns.push(AbiParam::new(types::I32));
    let id = module.declare_function("tempo_apply_output", Linkage::Local, &signature)?;
    let mut context = module.make_context();
    context.func.signature = signature;
    let sort_captures = module.declare_func_in_func(sort_captures, &mut context.func);
    let sort_results = module.declare_func_in_func(sort_results, &mut context.func);
    let downcase = module.declare_func_in_func(downcase, &mut context.func);
    let strip = strip.map(|function| module.declare_func_in_func(function, &mut context.func));
    let language_from_mime = module.declare_func_in_func(language_from_mime, &mut context.func);
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = function.create_block();
        function.append_block_params_for_function_params(entry);
        let result_loop = function.create_block();
        function.append_block_param(result_loop, types::I32);
        function.append_block_param(result_loop, types::I32);
        function.append_block_param(result_loop, types::I32);
        function.append_block_param(result_loop, types::I32);
        let result_body = function.create_block();
        function.append_block_param(result_body, types::I32);
        function.append_block_param(result_body, types::I32);
        function.append_block_param(result_body, types::I32);
        function.append_block_param(result_body, types::I32);
        let capture_loop = function.create_block();
        function.append_block_param(capture_loop, types::I32);
        function.append_block_param(capture_loop, types::I32);
        function.append_block_param(capture_loop, types::I32);
        function.append_block_param(capture_loop, types::I32);
        function.append_block_param(capture_loop, types::I32);
        function.append_block_param(capture_loop, types::I32);
        let capture_body = function.create_block();
        function.append_block_param(capture_body, types::I32);
        function.append_block_param(capture_body, types::I32);
        function.append_block_param(capture_body, types::I32);
        function.append_block_param(capture_body, types::I32);
        function.append_block_param(capture_body, types::I32);
        function.append_block_param(capture_body, types::I32);
        let sorted = function.create_block();
        function.append_block_param(sorted, types::I32);
        function.append_block_param(sorted, types::I32);
        function.append_block_param(sorted, types::I32);
        function.append_block_param(sorted, types::I32);
        function.append_block_param(sorted, types::I32);
        let properties = function.create_block();
        function.append_block_param(properties, types::I32);
        function.append_block_param(properties, types::I32);
        function.append_block_param(properties, types::I32);
        function.append_block_param(properties, types::I32);
        function.append_block_param(properties, types::I32);
        let reject_result = function.create_block();
        function.append_block_param(reject_result, types::I32);
        function.append_block_param(reject_result, types::I32);
        function.append_block_param(reject_result, types::I32);
        function.append_block_param(reject_result, types::I32);
        let capacity = function.create_block();
        let output_error = function.create_block();
        function.append_block_param(output_error, types::I32);
        let sort = function.create_block();
        function.append_block_param(sort, types::I32);
        function.append_block_param(sort, types::I32);
        function.append_block_param(sort, types::I32);
        function.append_block_param(sort, types::I32);
        let finish = function.create_block();
        function.append_block_param(finish, types::I32);
        function.append_block_param(finish, types::I32);
        function.append_block_param(finish, types::I32);
        function.append_block_param(finish, types::I32);

        function.switch_to_block(entry);
        let task = function.block_params(entry)[0];
        let status = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, status),
        );
        let raw_count = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            status,
            mem::offset_of!(TaskStatus, result_count) as i32,
        );
        let zero = function.ins().iconst(types::I32, 0);
        function.ins().jump(
            result_loop,
            &[zero.into(), zero.into(), zero.into(), zero.into()],
        );

        function.switch_to_block(result_loop);
        let values = function.block_params(result_loop).to_vec();
        let done = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, values[0], raw_count);
        function.ins().brif(
            done,
            sort,
            &[
                zero.into(),
                values[1].into(),
                values[2].into(),
                values[3].into(),
            ],
            result_body,
            &[
                values[0].into(),
                values[1].into(),
                values[2].into(),
                values[3].into(),
            ],
        );

        function.switch_to_block(result_body);
        let values = function.block_params(result_body).to_vec();
        let results = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, results),
        );
        let result = indexed::<ResultRecord>(&mut function, results, values[0]);
        let head = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            result,
            mem::offset_of!(ResultRecord, capture_start) as i32,
        );
        let count = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            result,
            mem::offset_of!(ResultRecord, capture_count) as i32,
        );
        let selected_result = indexed::<ResultRecord>(&mut function, results, values[3]);
        for offset in [
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
        ] {
            let value = function
                .ins()
                .load(types::I32, MemFlags::trusted(), result, offset as i32);
            function
                .ins()
                .store(MemFlags::trusted(), value, selected_result, offset as i32);
        }
        let end = function.ins().iadd(values[1], count);
        let output_capacity = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, final_capture_capacity) as i32,
        );
        let enough = function
            .ins()
            .icmp(IntCC::UnsignedLessThanOrEqual, end, output_capacity);
        function.ins().brif(
            enough,
            capture_loop,
            &[
                values[0].into(),
                values[1].into(),
                values[2].into(),
                zero.into(),
                head.into(),
                values[3].into(),
            ],
            capacity,
            &[],
        );

        function.switch_to_block(capture_loop);
        let values = function.block_params(capture_loop).to_vec();
        let results = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, results),
        );
        let result = indexed::<ResultRecord>(&mut function, results, values[5]);
        let count = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            result,
            mem::offset_of!(ResultRecord, capture_count) as i32,
        );
        let done = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, values[3], count);
        function.ins().brif(
            done,
            sorted,
            &[
                values[0].into(),
                values[1].into(),
                values[2].into(),
                count.into(),
                values[5].into(),
            ],
            capture_body,
            &[
                values[0].into(),
                values[1].into(),
                values[2].into(),
                values[3].into(),
                values[4].into(),
                values[5].into(),
            ],
        );

        function.switch_to_block(capture_body);
        let values = function.block_params(capture_body).to_vec();
        let results = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, results),
        );
        let result = indexed::<ResultRecord>(&mut function, results, values[5]);
        let count = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            result,
            mem::offset_of!(ResultRecord, capture_count) as i32,
        );
        let (label, node_id) = capture_oldest(&mut function, task, values[4], count, values[3]);
        let nodes = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, nodes),
        );
        let node = indexed::<MetalNode>(&mut function, nodes, node_id);
        let start = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            node,
            mem::offset_of!(MetalNode, source_start) as i32,
        );
        let end = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            node,
            mem::offset_of!(MetalNode, source_end) as i32,
        );
        let output = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, final_captures),
        );
        let output_index = function.ins().iadd(values[1], values[3]);
        let capture = indexed::<FinalCaptureRecord>(&mut function, output, output_index);
        for (offset, value) in [
            (mem::offset_of!(FinalCaptureRecord, label), label),
            (mem::offset_of!(FinalCaptureRecord, node), node_id),
            (mem::offset_of!(FinalCaptureRecord, start), start),
            (mem::offset_of!(FinalCaptureRecord, end), end),
        ] {
            function
                .ins()
                .store(MemFlags::trusted(), value, capture, offset as i32);
        }
        let none = function.ins().iconst(types::I32, NONE as i64);
        function.ins().store(
            MemFlags::trusted(),
            none,
            capture,
            mem::offset_of!(FinalCaptureRecord, text_start) as i32,
        );
        for offset in [
            mem::offset_of!(FinalCaptureRecord, text_length),
            mem::offset_of!(FinalCaptureRecord, property_start),
            mem::offset_of!(FinalCaptureRecord, property_count),
        ] {
            function
                .ins()
                .store(MemFlags::trusted(), zero, capture, offset as i32);
        }
        let one = function.ins().iconst(types::I32, 1);
        function.ins().store(
            MemFlags::trusted(),
            one,
            capture,
            mem::offset_of!(FinalCaptureRecord, retained) as i32,
        );
        let next = function.ins().iadd_imm(values[3], 1);
        function.ins().jump(
            capture_loop,
            &[
                values[0].into(),
                values[1].into(),
                values[2].into(),
                next.into(),
                values[4].into(),
                values[5].into(),
            ],
        );

        function.switch_to_block(sorted);
        let values = function.block_params(sorted).to_vec();
        function
            .ins()
            .call(sort_captures, &[task, values[1], values[3]]);
        function.ins().jump(
            properties,
            &[
                values[0].into(),
                values[1].into(),
                values[2].into(),
                values[3].into(),
                values[4].into(),
            ],
        );

        function.switch_to_block(properties);
        let values = function.block_params(properties).to_vec();
        let results = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, results),
        );
        let result = indexed::<ResultRecord>(&mut function, results, values[4]);
        let property_start = values[2];
        let mut property_count = values[2];
        let mut dynamic_count = zero;
        let mut strip_index = native.value_programs.len() as u32;
        for instruction in &native.output_program.instructions {
            match instruction {
                OutputInstruction::MatchProperty { key, value } => {
                    let property_quota_limit = function.ins().load(
                        types::I32,
                        MemFlags::trusted(),
                        task,
                        mem::offset_of!(NativeTaskContext, property_quota_limit) as i32,
                    );
                    let available = function.ins().icmp(
                        IntCC::UnsignedLessThan,
                        property_count,
                        property_quota_limit,
                    );
                    let write = function.create_block();
                    function.ins().brif(available, write, &[], capacity, &[]);
                    function.switch_to_block(write);
                    let output = pointer(
                        &mut function,
                        task,
                        mem::offset_of!(NativeTaskContext, properties),
                    );
                    let property = indexed::<PropertyRecord>(&mut function, output, property_count);
                    let kind = function.ins().iconst(types::I32, 0);
                    let owner = function.ins().load(
                        types::I32,
                        MemFlags::trusted(),
                        result,
                        mem::offset_of!(ResultRecord, local_ordinal) as i32,
                    );
                    let key = function.ins().iconst(types::I32, i64::from(*key));
                    let value = function
                        .ins()
                        .iconst(types::I32, i64::from(value.map_or(NONE, u32::from)));
                    for (offset, found) in [
                        (mem::offset_of!(PropertyRecord, kind), kind),
                        (mem::offset_of!(PropertyRecord, owner), owner),
                        (mem::offset_of!(PropertyRecord, key), key),
                        (mem::offset_of!(PropertyRecord, value), value),
                    ] {
                        function
                            .ins()
                            .store(MemFlags::trusted(), found, property, offset as i32);
                    }
                    for offset in [
                        mem::offset_of!(PropertyRecord, key_start),
                        mem::offset_of!(PropertyRecord, key_length),
                        mem::offset_of!(PropertyRecord, value_start),
                        mem::offset_of!(PropertyRecord, value_length),
                    ] {
                        function
                            .ins()
                            .store(MemFlags::trusted(), zero, property, offset as i32);
                    }
                    property_count = function.ins().iadd_imm(property_count, 1);
                }
                OutputInstruction::CaptureProperty {
                    capture,
                    key,
                    value,
                } => {
                    let property_loop = function.create_block();
                    function.append_block_param(property_loop, types::I32);
                    function.append_block_param(property_loop, types::I32);
                    let property_body = function.create_block();
                    function.append_block_param(property_body, types::I32);
                    function.append_block_param(property_body, types::I32);
                    let property_write = function.create_block();
                    function.append_block_param(property_write, types::I32);
                    function.append_block_param(property_write, types::I32);
                    function.append_block_param(property_write, types::I32);
                    let property_next = function.create_block();
                    function.append_block_param(property_next, types::I32);
                    function.append_block_param(property_next, types::I32);
                    let property_done = function.create_block();
                    function.append_block_param(property_done, types::I32);
                    function
                        .ins()
                        .jump(property_loop, &[zero.into(), property_count.into()]);
                    function.switch_to_block(property_loop);
                    let loop_values = function.block_params(property_loop).to_vec();
                    let done = function.ins().icmp(
                        IntCC::UnsignedGreaterThanOrEqual,
                        loop_values[0],
                        values[3],
                    );
                    function.ins().brif(
                        done,
                        property_done,
                        &[loop_values[1].into()],
                        property_body,
                        &[loop_values[0].into(), loop_values[1].into()],
                    );
                    function.switch_to_block(property_body);
                    let body_values = function.block_params(property_body).to_vec();
                    let output = pointer(
                        &mut function,
                        task,
                        mem::offset_of!(NativeTaskContext, final_captures),
                    );
                    let output_index = function.ins().iadd(values[1], body_values[0]);
                    let found = indexed::<FinalCaptureRecord>(&mut function, output, output_index);
                    let retained = function.ins().load(
                        types::I32,
                        MemFlags::trusted(),
                        found,
                        mem::offset_of!(FinalCaptureRecord, retained) as i32,
                    );
                    let label = function.ins().load(
                        types::I32,
                        MemFlags::trusted(),
                        found,
                        mem::offset_of!(FinalCaptureRecord, label) as i32,
                    );
                    let retained = function.ins().icmp_imm(IntCC::NotEqual, retained, 0);
                    let label = function
                        .ins()
                        .icmp_imm(IntCC::Equal, label, i64::from(*capture));
                    let selected = function.ins().band(retained, label);
                    function.ins().brif(
                        selected,
                        property_write,
                        &[
                            body_values[0].into(),
                            body_values[1].into(),
                            output_index.into(),
                        ],
                        property_next,
                        &[body_values[0].into(), body_values[1].into()],
                    );
                    function.switch_to_block(property_write);
                    let write_values = function.block_params(property_write).to_vec();
                    let capacity_value = function.ins().load(
                        types::I32,
                        MemFlags::trusted(),
                        task,
                        mem::offset_of!(NativeTaskContext, property_quota_limit) as i32,
                    );
                    let available = function.ins().icmp(
                        IntCC::UnsignedLessThan,
                        write_values[1],
                        capacity_value,
                    );
                    let append = function.create_block();
                    function.ins().brif(available, append, &[], capacity, &[]);
                    function.switch_to_block(append);
                    let output = pointer(
                        &mut function,
                        task,
                        mem::offset_of!(NativeTaskContext, properties),
                    );
                    let property =
                        indexed::<PropertyRecord>(&mut function, output, write_values[1]);
                    let kind_value = function.ins().iconst(types::I32, 1);
                    let key_value = function.ins().iconst(types::I32, i64::from(*key));
                    let property_value = function
                        .ins()
                        .iconst(types::I32, i64::from(value.map_or(NONE, u32::from)));
                    for (offset, stored) in [
                        (mem::offset_of!(PropertyRecord, kind), kind_value),
                        (mem::offset_of!(PropertyRecord, owner), write_values[2]),
                        (mem::offset_of!(PropertyRecord, key), key_value),
                        (mem::offset_of!(PropertyRecord, value), property_value),
                    ] {
                        function
                            .ins()
                            .store(MemFlags::trusted(), stored, property, offset as i32);
                    }
                    for offset in [
                        mem::offset_of!(PropertyRecord, key_start),
                        mem::offset_of!(PropertyRecord, key_length),
                        mem::offset_of!(PropertyRecord, value_start),
                        mem::offset_of!(PropertyRecord, value_length),
                    ] {
                        function
                            .ins()
                            .store(MemFlags::trusted(), zero, property, offset as i32);
                    }
                    let count = function.ins().load(
                        types::I32,
                        MemFlags::trusted(),
                        found,
                        mem::offset_of!(FinalCaptureRecord, property_count) as i32,
                    );
                    let count = function.ins().iadd_imm(count, 1);
                    function.ins().store(
                        MemFlags::trusted(),
                        count,
                        found,
                        mem::offset_of!(FinalCaptureRecord, property_count) as i32,
                    );
                    let next_count = function.ins().iadd_imm(write_values[1], 1);
                    function
                        .ins()
                        .jump(property_next, &[write_values[0].into(), next_count.into()]);
                    function.switch_to_block(property_next);
                    let next_values = function.block_params(property_next).to_vec();
                    let next = function.ins().iadd_imm(next_values[0], 1);
                    function
                        .ins()
                        .jump(property_loop, &[next.into(), next_values[1].into()]);
                    function.switch_to_block(property_done);
                    property_count = function.block_params(property_done)[0];
                }
                OutputInstruction::Strip { capture, .. } => {
                    let capture = function.ins().iconst(types::I32, i64::from(*capture));
                    let value = function.ins().iconst(types::I32, i64::from(strip_index));
                    let call = function.ins().call(
                        strip.expect("strip helper exists for strip output"),
                        &[task, values[1], values[3], capture, value],
                    );
                    strip_index += 1;
                    let outcome = function.inst_results(call)[0];
                    let ok = function.ins().icmp_imm(IntCC::Equal, outcome, 0);
                    let next = function.create_block();
                    function
                        .ins()
                        .brif(ok, next, &[], output_error, &[outcome.into()]);
                    function.switch_to_block(next);
                }
                OutputInstruction::SelectAdjacent { capture, target } => {
                    let find_target = function.create_block();
                    function.append_block_param(find_target, types::I32);
                    let inspect_target = function.create_block();
                    function.append_block_param(inspect_target, types::I32);
                    let target_found = function.create_block();
                    function.append_block_param(target_found, types::I32);
                    let adjacent_reverse = function.create_block();
                    function.append_block_param(adjacent_reverse, types::I32);
                    function.append_block_param(adjacent_reverse, types::I32);
                    function.append_block_param(adjacent_reverse, types::I32);
                    let adjacent_inspect = function.create_block();
                    function.append_block_param(adjacent_inspect, types::I32);
                    function.append_block_param(adjacent_inspect, types::I32);
                    function.append_block_param(adjacent_inspect, types::I32);
                    let adjacent_match = function.create_block();
                    function.append_block_param(adjacent_match, types::I32);
                    function.append_block_param(adjacent_match, types::I32);
                    function.append_block_param(adjacent_match, types::I32);
                    let mark_loop = function.create_block();
                    function.append_block_param(mark_loop, types::I32);
                    function.append_block_param(mark_loop, types::I32);
                    let mark_body = function.create_block();
                    function.append_block_param(mark_body, types::I32);
                    function.append_block_param(mark_body, types::I32);
                    let adjacent_done = function.create_block();
                    function.ins().jump(find_target, &[zero.into()]);
                    function.switch_to_block(find_target);
                    let index = function.block_params(find_target)[0];
                    let complete =
                        function
                            .ins()
                            .icmp(IntCC::UnsignedGreaterThanOrEqual, index, values[3]);
                    function.ins().brif(
                        complete,
                        adjacent_done,
                        &[],
                        inspect_target,
                        &[index.into()],
                    );
                    function.switch_to_block(inspect_target);
                    let index = function.block_params(inspect_target)[0];
                    let output = pointer(
                        &mut function,
                        task,
                        mem::offset_of!(NativeTaskContext, final_captures),
                    );
                    let absolute = function.ins().iadd(values[1], index);
                    let found = indexed::<FinalCaptureRecord>(&mut function, output, absolute);
                    let retained = function.ins().load(
                        types::I32,
                        MemFlags::trusted(),
                        found,
                        mem::offset_of!(FinalCaptureRecord, retained) as i32,
                    );
                    let label = function.ins().load(
                        types::I32,
                        MemFlags::trusted(),
                        found,
                        mem::offset_of!(FinalCaptureRecord, label) as i32,
                    );
                    let retained = function.ins().icmp_imm(IntCC::NotEqual, retained, 0);
                    let label = function
                        .ins()
                        .icmp_imm(IntCC::Equal, label, i64::from(*target));
                    let selected = function.ins().band(retained, label);
                    let next_target = function.create_block();
                    function.ins().brif(
                        selected,
                        target_found,
                        &[absolute.into()],
                        next_target,
                        &[],
                    );
                    function.switch_to_block(next_target);
                    let next = function.ins().iadd_imm(index, 1);
                    function.ins().jump(find_target, &[next.into()]);
                    function.switch_to_block(target_found);
                    let target_index = function.block_params(target_found)[0];
                    let target_capture =
                        indexed::<FinalCaptureRecord>(&mut function, output, target_index);
                    let target_start = function.ins().load(
                        types::I32,
                        MemFlags::trusted(),
                        target_capture,
                        mem::offset_of!(FinalCaptureRecord, start) as i32,
                    );
                    let target_row = line_for(&mut function, task, target_start);
                    function.ins().jump(
                        adjacent_reverse,
                        &[values[3].into(), values[3].into(), target_row.into()],
                    );
                    function.switch_to_block(adjacent_reverse);
                    let reverse = function.block_params(adjacent_reverse).to_vec();
                    let complete = function.ins().icmp_imm(IntCC::Equal, reverse[0], 0);
                    function.ins().brif(
                        complete,
                        mark_loop,
                        &[zero.into(), reverse[1].into()],
                        adjacent_inspect,
                        &[reverse[0].into(), reverse[1].into(), reverse[2].into()],
                    );
                    function.switch_to_block(adjacent_inspect);
                    let reverse = function.block_params(adjacent_inspect).to_vec();
                    let index = function.ins().iadd_imm(reverse[0], -1);
                    let absolute = function.ins().iadd(values[1], index);
                    let found = indexed::<FinalCaptureRecord>(&mut function, output, absolute);
                    let retained = function.ins().load(
                        types::I32,
                        MemFlags::trusted(),
                        found,
                        mem::offset_of!(FinalCaptureRecord, retained) as i32,
                    );
                    let label = function.ins().load(
                        types::I32,
                        MemFlags::trusted(),
                        found,
                        mem::offset_of!(FinalCaptureRecord, label) as i32,
                    );
                    let retained = function.ins().icmp_imm(IntCC::NotEqual, retained, 0);
                    let label = function
                        .ins()
                        .icmp_imm(IntCC::Equal, label, i64::from(*capture));
                    let selected = function.ins().band(retained, label);
                    let adjacent_skip = function.create_block();
                    function.ins().brif(
                        selected,
                        adjacent_match,
                        &[index.into(), reverse[1].into(), reverse[2].into()],
                        adjacent_skip,
                        &[],
                    );
                    function.switch_to_block(adjacent_skip);
                    function.ins().jump(
                        adjacent_reverse,
                        &[index.into(), reverse[1].into(), reverse[2].into()],
                    );
                    function.switch_to_block(adjacent_match);
                    let reverse = function.block_params(adjacent_match).to_vec();
                    let absolute = function.ins().iadd(values[1], reverse[0]);
                    let found = indexed::<FinalCaptureRecord>(&mut function, output, absolute);
                    let end = function.ins().load(
                        types::I32,
                        MemFlags::trusted(),
                        found,
                        mem::offset_of!(FinalCaptureRecord, end) as i32,
                    );
                    let end_row = line_for(&mut function, task, end);
                    let adjacent_limit = function.ins().iadd_imm(end_row, 1);
                    let separated =
                        function
                            .ins()
                            .icmp(IntCC::UnsignedLessThan, adjacent_limit, reverse[2]);
                    let separated_block = function.create_block();
                    let retained_block = function.create_block();
                    function
                        .ins()
                        .brif(separated, separated_block, &[], retained_block, &[]);
                    function.switch_to_block(separated_block);
                    function
                        .ins()
                        .jump(mark_loop, &[zero.into(), reverse[1].into()]);
                    function.switch_to_block(retained_block);
                    let start = function.ins().load(
                        types::I32,
                        MemFlags::trusted(),
                        found,
                        mem::offset_of!(FinalCaptureRecord, start) as i32,
                    );
                    let next_start = line_for(&mut function, task, start);
                    function.ins().jump(
                        adjacent_reverse,
                        &[reverse[0].into(), reverse[0].into(), next_start.into()],
                    );
                    function.switch_to_block(mark_loop);
                    let mark = function.block_params(mark_loop).to_vec();
                    let complete =
                        function
                            .ins()
                            .icmp(IntCC::UnsignedGreaterThanOrEqual, mark[0], mark[1]);
                    function.ins().brif(
                        complete,
                        adjacent_done,
                        &[],
                        mark_body,
                        &[mark[0].into(), mark[1].into()],
                    );
                    function.switch_to_block(mark_body);
                    let mark = function.block_params(mark_body).to_vec();
                    let absolute = function.ins().iadd(values[1], mark[0]);
                    let found = indexed::<FinalCaptureRecord>(&mut function, output, absolute);
                    let label = function.ins().load(
                        types::I32,
                        MemFlags::trusted(),
                        found,
                        mem::offset_of!(FinalCaptureRecord, label) as i32,
                    );
                    let selected =
                        function
                            .ins()
                            .icmp_imm(IntCC::Equal, label, i64::from(*capture));
                    let clear = function.create_block();
                    let mark_next = function.create_block();
                    function.ins().brif(selected, clear, &[], mark_next, &[]);
                    function.switch_to_block(clear);
                    function.ins().store(
                        MemFlags::trusted(),
                        zero,
                        found,
                        mem::offset_of!(FinalCaptureRecord, retained) as i32,
                    );
                    function.ins().jump(mark_next, &[]);
                    function.switch_to_block(mark_next);
                    let next = function.ins().iadd_imm(mark[0], 1);
                    function
                        .ins()
                        .jump(mark_loop, &[next.into(), mark[1].into()]);
                    function.switch_to_block(adjacent_done);
                }
                OutputInstruction::Downcase { capture } => {
                    let label = function.ins().iconst(types::I32, i64::from(*capture));
                    let call = function
                        .ins()
                        .call(downcase, &[task, values[1], values[3], label]);
                    let outcome = function.inst_results(call)[0];
                    let succeeded = function.ins().icmp_imm(IntCC::Equal, outcome, 0);
                    let continued = function.create_block();
                    function
                        .ins()
                        .brif(succeeded, continued, &[], capacity, &[]);
                    function.switch_to_block(continued);
                }
                OutputInstruction::LanguageFromMime { capture } => {
                    let capture = function.ins().iconst(types::I32, i64::from(*capture));
                    let owner = function.ins().load(
                        types::I32,
                        MemFlags::trusted(),
                        result,
                        mem::offset_of!(ResultRecord, local_ordinal) as i32,
                    );
                    let call = function.ins().call(
                        language_from_mime,
                        &[task, values[1], values[3], capture, property_count, owner],
                    );
                    let outcome = function.inst_results(call)[0];
                    let accepted = function.ins().icmp_imm(IntCC::Equal, outcome, 0);
                    let not_accepted = function.create_block();
                    let language_done = function.create_block();
                    function
                        .ins()
                        .brif(accepted, language_done, &[], not_accepted, &[]);
                    function.switch_to_block(not_accepted);
                    let rejected = function.ins().icmp_imm(IntCC::Equal, outcome, 1);
                    function.ins().brif(
                        rejected,
                        reject_result,
                        &[
                            values[0].into(),
                            values[1].into(),
                            values[2].into(),
                            values[4].into(),
                        ],
                        output_error,
                        &[outcome.into()],
                    );
                    function.switch_to_block(language_done);
                    property_count = function.ins().iadd_imm(property_count, 1);
                    dynamic_count = function.ins().iadd_imm(dynamic_count, 1);
                }
                OutputInstruction::Offset { capture, offsets } => {
                    let offset_loop = function.create_block();
                    function.append_block_param(offset_loop, types::I32);
                    let offset_body = function.create_block();
                    function.append_block_param(offset_body, types::I32);
                    let offset_next = function.create_block();
                    function.append_block_param(offset_next, types::I32);
                    let offset_done = function.create_block();
                    function.ins().jump(offset_loop, &[zero.into()]);
                    function.switch_to_block(offset_loop);
                    let index = function.block_params(offset_loop)[0];
                    let done =
                        function
                            .ins()
                            .icmp(IntCC::UnsignedGreaterThanOrEqual, index, values[3]);
                    function
                        .ins()
                        .brif(done, offset_done, &[], offset_body, &[index.into()]);
                    function.switch_to_block(offset_body);
                    let index = function.block_params(offset_body)[0];
                    let output = pointer(
                        &mut function,
                        task,
                        mem::offset_of!(NativeTaskContext, final_captures),
                    );
                    let output_index = function.ins().iadd(values[1], index);
                    let found = indexed::<FinalCaptureRecord>(&mut function, output, output_index);
                    let retained = function.ins().load(
                        types::I32,
                        MemFlags::trusted(),
                        found,
                        mem::offset_of!(FinalCaptureRecord, retained) as i32,
                    );
                    let label = function.ins().load(
                        types::I32,
                        MemFlags::trusted(),
                        found,
                        mem::offset_of!(FinalCaptureRecord, label) as i32,
                    );
                    let retained = function.ins().icmp_imm(IntCC::NotEqual, retained, 0);
                    let label = function
                        .ins()
                        .icmp_imm(IntCC::Equal, label, i64::from(*capture));
                    let selected = function.ins().band(retained, label);
                    function.ins().brif(
                        selected,
                        offset_next,
                        &[index.into()],
                        offset_next,
                        &[index.into()],
                    );
                    function.switch_to_block(offset_next);
                    let index = function.block_params(offset_next)[0];
                    let output_index = function.ins().iadd(values[1], index);
                    let found = indexed::<FinalCaptureRecord>(&mut function, output, output_index);
                    let retained_value = function.ins().load(
                        types::I32,
                        MemFlags::trusted(),
                        found,
                        mem::offset_of!(FinalCaptureRecord, retained) as i32,
                    );
                    let label_value = function.ins().load(
                        types::I32,
                        MemFlags::trusted(),
                        found,
                        mem::offset_of!(FinalCaptureRecord, label) as i32,
                    );
                    let retained_match =
                        function.ins().icmp_imm(IntCC::NotEqual, retained_value, 0);
                    let label_match =
                        function
                            .ins()
                            .icmp_imm(IntCC::Equal, label_value, i64::from(*capture));
                    let apply = function.ins().band(retained_match, label_match);
                    let apply_block = function.create_block();
                    let continue_block = function.create_block();
                    function
                        .ins()
                        .brif(apply, apply_block, &[], continue_block, &[]);
                    function.switch_to_block(apply_block);
                    let start = function.ins().load(
                        types::I32,
                        MemFlags::trusted(),
                        found,
                        mem::offset_of!(FinalCaptureRecord, start) as i32,
                    );
                    let end = function.ins().load(
                        types::I32,
                        MemFlags::trusted(),
                        found,
                        mem::offset_of!(FinalCaptureRecord, end) as i32,
                    );
                    let start_row = line_for(&mut function, task, start);
                    let end_row = line_for(&mut function, task, end);
                    let starts = pointer(
                        &mut function,
                        task,
                        mem::offset_of!(NativeTaskContext, line_starts),
                    );
                    let start_line_address = indexed::<u32>(&mut function, starts, start_row);
                    let end_line_address = indexed::<u32>(&mut function, starts, end_row);
                    let start_line =
                        function
                            .ins()
                            .load(types::I32, MemFlags::trusted(), start_line_address, 0);
                    let end_line =
                        function
                            .ins()
                            .load(types::I32, MemFlags::trusted(), end_line_address, 0);
                    let start_column = function.ins().isub(start, start_line);
                    let end_column = function.ins().isub(end, end_line);
                    let new_start_row = function.ins().iadd_imm(start_row, i64::from(offsets[0]));
                    let new_start_column =
                        function.ins().iadd_imm(start_column, i64::from(offsets[1]));
                    let new_end_row = function.ins().iadd_imm(end_row, i64::from(offsets[2]));
                    let new_end_column = function.ins().iadd_imm(end_column, i64::from(offsets[3]));
                    let new_start = byte_for(&mut function, task, new_start_row, new_start_column);
                    let new_end = byte_for(&mut function, task, new_end_row, new_end_column);
                    let start_valid =
                        function
                            .ins()
                            .icmp_imm(IntCC::NotEqual, new_start, NONE as i64);
                    let end_valid = function
                        .ins()
                        .icmp_imm(IntCC::NotEqual, new_end, NONE as i64);
                    let ordered =
                        function
                            .ins()
                            .icmp(IntCC::UnsignedLessThanOrEqual, new_start, new_end);
                    let valid = function.ins().band(start_valid, end_valid);
                    let valid = function.ins().band(valid, ordered);
                    let write_offset = function.create_block();
                    function.ins().brif(
                        valid,
                        write_offset,
                        &[],
                        reject_result,
                        &[
                            values[0].into(),
                            values[1].into(),
                            values[2].into(),
                            values[4].into(),
                        ],
                    );
                    function.switch_to_block(write_offset);
                    function.ins().store(
                        MemFlags::trusted(),
                        new_start,
                        found,
                        mem::offset_of!(FinalCaptureRecord, start) as i32,
                    );
                    function.ins().store(
                        MemFlags::trusted(),
                        new_end,
                        found,
                        mem::offset_of!(FinalCaptureRecord, end) as i32,
                    );
                    function.ins().jump(continue_block, &[]);
                    function.switch_to_block(continue_block);
                    let next = function.ins().iadd_imm(index, 1);
                    function.ins().jump(offset_loop, &[next.into()]);
                    function.switch_to_block(offset_done);
                }
            }
        }
        let property_length = function.ins().iconst(
            types::I32,
            native
                .output_program
                .instructions
                .iter()
                .filter(|instruction| {
                    matches!(instruction, OutputInstruction::MatchProperty { .. })
                })
                .count() as i64,
        );
        let dynamic_start = function.ins().iadd(property_start, property_length);
        function.ins().store(
            MemFlags::trusted(),
            values[1],
            result,
            mem::offset_of!(ResultRecord, capture_start) as i32,
        );
        function.ins().store(
            MemFlags::trusted(),
            values[3],
            result,
            mem::offset_of!(ResultRecord, capture_count) as i32,
        );
        function.ins().store(
            MemFlags::trusted(),
            property_start,
            result,
            mem::offset_of!(ResultRecord, property_start) as i32,
        );
        function.ins().store(
            MemFlags::trusted(),
            property_length,
            result,
            mem::offset_of!(ResultRecord, property_count) as i32,
        );
        function.ins().store(
            MemFlags::trusted(),
            dynamic_start,
            result,
            mem::offset_of!(ResultRecord, dynamic_start) as i32,
        );
        function.ins().store(
            MemFlags::trusted(),
            dynamic_count,
            result,
            mem::offset_of!(ResultRecord, dynamic_count) as i32,
        );
        let next_result = function.ins().iadd_imm(values[0], 1);
        let next_capture = function.ins().iadd(values[1], values[3]);
        let next_selected = function.ins().iadd_imm(values[4], 1);
        function.ins().jump(
            result_loop,
            &[
                next_result.into(),
                next_capture.into(),
                property_count.into(),
                next_selected.into(),
            ],
        );

        function.switch_to_block(reject_result);
        let rejected = function.block_params(reject_result).to_vec();
        let next_result = function.ins().iadd_imm(rejected[0], 1);
        function.ins().jump(
            result_loop,
            &[
                next_result.into(),
                rejected[1].into(),
                rejected[2].into(),
                rejected[3].into(),
            ],
        );

        function.switch_to_block(sort);
        let sorted = function.block_params(sort).to_vec();
        function.ins().call(sort_results, &[task, sorted[3]]);
        function.ins().jump(
            finish,
            &[
                sorted[0].into(),
                sorted[1].into(),
                sorted[2].into(),
                sorted[3].into(),
            ],
        );
        function.switch_to_block(output_error);
        let output_code = function.block_params(output_error)[0];
        function.ins().jump(
            finish,
            &[output_code.into(), zero.into(), zero.into(), zero.into()],
        );
        function.switch_to_block(capacity);
        let capacity_code = function.ins().iconst(types::I32, 4);
        function.ins().jump(
            finish,
            &[capacity_code.into(), zero.into(), zero.into(), zero.into()],
        );
        function.switch_to_block(finish);
        let finish_values = function.block_params(finish).to_vec();
        let outcome = finish_values[0];
        let successful = function.ins().icmp_imm(IntCC::Equal, outcome, 0);
        let update = function.create_block();
        let return_block = function.create_block();
        function
            .ins()
            .brif(successful, update, &[], return_block, &[]);
        function.switch_to_block(update);
        let status = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, status),
        );
        function.ins().store(
            MemFlags::trusted(),
            finish_values[1],
            status,
            mem::offset_of!(TaskStatus, final_capture_count) as i32,
        );
        function.ins().store(
            MemFlags::trusted(),
            finish_values[2],
            status,
            mem::offset_of!(TaskStatus, property_count) as i32,
        );
        function.ins().store(
            MemFlags::trusted(),
            finish_values[3],
            status,
            mem::offset_of!(TaskStatus, result_count) as i32,
        );
        function.ins().jump(return_block, &[]);
        function.switch_to_block(return_block);
        function.ins().return_(&[outcome]);
        function.seal_all_blocks();
        function.finalize();
    }
    module.define_function(id, &mut context)?;
    module.clear_context(&mut context);
    Ok(id)
}
