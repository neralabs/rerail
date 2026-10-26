use super::{
    metal_batch_abi::{FinalCaptureRecord, NONE, PropertyRecord, TaskStatus},
    native_cpu::NativeTaskContext,
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

fn indexed(
    function: &mut FunctionBuilder<'_>,
    base: cranelift_codegen::ir::Value,
    index: cranelift_codegen::ir::Value,
    stride: usize,
) -> cranelift_codegen::ir::Value {
    let ty = function.func.dfg.value_type(base);
    let offset = function.ins().imul_imm(index, stride as i64);
    let offset = function.ins().uextend(ty, offset);
    function.ins().iadd(base, offset)
}

fn indexed_imm(
    function: &mut FunctionBuilder<'_>,
    base: cranelift_codegen::ir::Value,
    index: cranelift_codegen::ir::Value,
    offset: i64,
) -> cranelift_codegen::ir::Value {
    let index = function.ins().iadd_imm(index, offset);
    indexed(function, base, index, 1)
}

fn store_byte_imm(
    function: &mut FunctionBuilder<'_>,
    address: cranelift_codegen::ir::Value,
    byte: u8,
) {
    let byte = function.ins().iconst(types::I8, i64::from(byte));
    function.ins().store(MemFlags::trusted(), byte, address, 0);
}

pub(super) fn define_downcase(
    module: &mut JITModule,
) -> Result<FuncId, Box<cranelift_module::ModuleError>> {
    let pointer_type = module.target_config().pointer_type();
    let mut signature = module.make_signature();
    signature.params.push(AbiParam::new(pointer_type));
    for _ in 0..3 {
        signature.params.push(AbiParam::new(types::I32));
    }
    signature.returns.push(AbiParam::new(types::I32));
    let id = module.declare_function("tempo_output_downcase", Linkage::Local, &signature)?;
    let mut context = module.make_context();
    context.func.signature = signature;
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = function.create_block();
        function.append_block_params_for_function_params(entry);
        let captures = function.create_block();
        function.append_block_param(captures, types::I32);
        let inspect = function.create_block();
        function.append_block_param(inspect, types::I32);
        let prepare = function.create_block();
        function.append_block_param(prepare, types::I32);
        let copy = function.create_block();
        function.append_block_param(copy, types::I32);
        function.append_block_param(copy, types::I32);
        function.append_block_param(copy, types::I32);
        function.append_block_param(copy, pointer_type);
        function.append_block_param(copy, pointer_type);
        let copy_byte = function.create_block();
        function.append_block_param(copy_byte, types::I32);
        function.append_block_param(copy_byte, types::I32);
        function.append_block_param(copy_byte, types::I32);
        function.append_block_param(copy_byte, pointer_type);
        function.append_block_param(copy_byte, pointer_type);
        let finish_capture = function.create_block();
        function.append_block_param(finish_capture, types::I32);
        function.append_block_param(finish_capture, types::I32);
        function.append_block_param(finish_capture, types::I32);
        let next_capture = function.create_block();
        function.append_block_param(next_capture, types::I32);
        let done = function.create_block();
        let capacity = function.create_block();

        function.switch_to_block(entry);
        let params = function.block_params(entry).to_vec();
        let task = params[0];
        let first = params[1];
        let count = params[2];
        let label = params[3];
        let zero = function.ins().iconst(types::I32, 0);
        function.ins().jump(captures, &[zero.into()]);
        function.switch_to_block(captures);
        let index = function.block_params(captures)[0];
        let complete = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, index, count);
        function
            .ins()
            .brif(complete, done, &[], inspect, &[index.into()]);
        function.switch_to_block(inspect);
        let index = function.block_params(inspect)[0];
        let output = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, final_captures),
        );
        let absolute = function.ins().iadd(first, index);
        let capture = indexed(
            &mut function,
            output,
            absolute,
            mem::size_of::<FinalCaptureRecord>(),
        );
        let retained = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            capture,
            mem::offset_of!(FinalCaptureRecord, retained) as i32,
        );
        let found_label = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            capture,
            mem::offset_of!(FinalCaptureRecord, label) as i32,
        );
        let retained = function.ins().icmp_imm(IntCC::NotEqual, retained, 0);
        let same_label = function.ins().icmp(IntCC::Equal, found_label, label);
        let selected = function.ins().band(retained, same_label);
        function.ins().brif(
            selected,
            prepare,
            &[index.into()],
            next_capture,
            &[index.into()],
        );
        function.switch_to_block(prepare);
        let index = function.block_params(prepare)[0];
        let absolute = function.ins().iadd(first, index);
        let capture = indexed(
            &mut function,
            output,
            absolute,
            mem::size_of::<FinalCaptureRecord>(),
        );
        let text_start = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            capture,
            mem::offset_of!(FinalCaptureRecord, text_start) as i32,
        );
        let start = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            capture,
            mem::offset_of!(FinalCaptureRecord, start) as i32,
        );
        let end = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            capture,
            mem::offset_of!(FinalCaptureRecord, end) as i32,
        );
        let text_length = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            capture,
            mem::offset_of!(FinalCaptureRecord, text_length) as i32,
        );
        let transformed = function
            .ins()
            .icmp_imm(IntCC::NotEqual, text_start, NONE as i64);
        let span_length = function.ins().isub(end, start);
        let input_start = function.ins().select(transformed, text_start, start);
        let length = function.ins().select(transformed, text_length, span_length);
        let status = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, status),
        );
        let byte_count = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            status,
            mem::offset_of!(TaskStatus, output_byte_count) as i32,
        );
        let next_count = function.ins().iadd(byte_count, length);
        let no_overflow =
            function
                .ins()
                .icmp(IntCC::UnsignedGreaterThanOrEqual, next_count, byte_count);
        let byte_capacity = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, output_byte_quota_limit) as i32,
        );
        let in_capacity =
            function
                .ins()
                .icmp(IntCC::UnsignedLessThanOrEqual, next_count, byte_capacity);
        let enough = function.ins().band(no_overflow, in_capacity);
        let source = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, source),
        );
        let transformed_bytes = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, output_bytes),
        );
        let input = function
            .ins()
            .select(transformed, transformed_bytes, source);
        let input = indexed(&mut function, input, input_start, 1);
        let target = indexed(&mut function, transformed_bytes, byte_count, 1);
        function.ins().brif(
            enough,
            copy,
            &[
                index.into(),
                zero.into(),
                length.into(),
                input.into(),
                target.into(),
            ],
            capacity,
            &[],
        );
        function.switch_to_block(copy);
        let values = function.block_params(copy).to_vec();
        let complete = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, values[1], values[2]);
        function.ins().brif(
            complete,
            finish_capture,
            &[values[0].into(), byte_count.into(), next_count.into()],
            copy_byte,
            &[
                values[0].into(),
                values[1].into(),
                values[2].into(),
                values[3].into(),
                values[4].into(),
            ],
        );
        function.switch_to_block(copy_byte);
        let values = function.block_params(copy_byte).to_vec();
        let source_at = indexed(&mut function, values[3], values[1], 1);
        let target_at = indexed(&mut function, values[4], values[1], 1);
        let byte = function
            .ins()
            .load(types::I8, MemFlags::trusted(), source_at, 0);
        let lower_bound =
            function
                .ins()
                .icmp_imm(IntCC::UnsignedGreaterThanOrEqual, byte, b'A' as i64);
        let upper_bound =
            function
                .ins()
                .icmp_imm(IntCC::UnsignedLessThanOrEqual, byte, b'Z' as i64);
        let uppercase = function.ins().band(lower_bound, upper_bound);
        let lowered = function.ins().iadd_imm(byte, 32);
        let byte = function.ins().select(uppercase, lowered, byte);
        function
            .ins()
            .store(MemFlags::trusted(), byte, target_at, 0);
        let next = function.ins().iadd_imm(values[1], 1);
        function.ins().jump(
            copy,
            &[
                values[0].into(),
                next.into(),
                values[2].into(),
                values[3].into(),
                values[4].into(),
            ],
        );
        function.switch_to_block(finish_capture);
        let values = function.block_params(finish_capture).to_vec();
        let absolute = function.ins().iadd(first, values[0]);
        let capture = indexed(
            &mut function,
            output,
            absolute,
            mem::size_of::<FinalCaptureRecord>(),
        );
        function.ins().store(
            MemFlags::trusted(),
            values[1],
            capture,
            mem::offset_of!(FinalCaptureRecord, text_start) as i32,
        );
        let length = function.ins().isub(values[2], values[1]);
        function.ins().store(
            MemFlags::trusted(),
            length,
            capture,
            mem::offset_of!(FinalCaptureRecord, text_length) as i32,
        );
        function.ins().store(
            MemFlags::trusted(),
            values[2],
            status,
            mem::offset_of!(TaskStatus, output_byte_count) as i32,
        );
        function.ins().jump(next_capture, &[values[0].into()]);
        function.switch_to_block(next_capture);
        let previous = function.block_params(next_capture)[0];
        let next = function.ins().iadd_imm(previous, 1);
        function.ins().jump(captures, &[next.into()]);
        function.switch_to_block(done);
        let ok = function.ins().iconst(types::I32, 0);
        function.ins().return_(&[ok]);
        function.switch_to_block(capacity);
        let error = function.ins().iconst(types::I32, 4);
        function.ins().return_(&[error]);
        function.seal_all_blocks();
        function.finalize();
    }
    module.define_function(id, &mut context)?;
    module.clear_context(&mut context);
    Ok(id)
}

pub(super) fn define_strip(
    module: &mut JITModule,
    find: FuncId,
) -> Result<FuncId, Box<cranelift_module::ModuleError>> {
    let pointer_type = module.target_config().pointer_type();
    let mut signature = module.make_signature();
    signature.params.push(AbiParam::new(pointer_type));
    for _ in 0..4 {
        signature.params.push(AbiParam::new(types::I32));
    }
    signature.returns.push(AbiParam::new(types::I32));
    let id = module.declare_function("tempo_output_strip", Linkage::Local, &signature)?;
    let mut context = module.make_context();
    context.func.signature = signature;
    let find = module.declare_func_in_func(find, &mut context.func);
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = function.create_block();
        function.append_block_params_for_function_params(entry);
        let captures = function.create_block();
        function.append_block_param(captures, types::I32);
        let inspect = function.create_block();
        function.append_block_param(inspect, types::I32);
        let prepare = function.create_block();
        function.append_block_param(prepare, types::I32);
        let search = function.create_block();
        for ty in [
            types::I32,
            types::I32,
            types::I32,
            types::I32,
            types::I32,
            pointer_type,
            pointer_type,
        ] {
            function.append_block_param(search, ty);
        }
        let candidate = function.create_block();
        for ty in [
            types::I32,
            types::I32,
            types::I32,
            types::I32,
            types::I32,
            pointer_type,
            pointer_type,
            types::I32,
        ] {
            function.append_block_param(candidate, ty);
        }
        let found = function.create_block();
        for ty in [
            types::I32,
            types::I32,
            types::I32,
            types::I32,
            types::I32,
            pointer_type,
            pointer_type,
            types::I64,
        ] {
            function.append_block_param(found, ty);
        }
        let copy = function.create_block();
        for ty in [
            types::I32,
            types::I32,
            types::I32,
            types::I32,
            pointer_type,
            pointer_type,
            types::I32,
            types::I32,
            types::I32,
            types::I32,
        ] {
            function.append_block_param(copy, ty);
        }
        let copy_byte = function.create_block();
        for ty in [
            types::I32,
            types::I32,
            types::I32,
            types::I32,
            pointer_type,
            pointer_type,
            types::I32,
            types::I32,
            types::I32,
            types::I32,
        ] {
            function.append_block_param(copy_byte, ty);
        }
        let copy_done = function.create_block();
        for ty in [
            types::I32,
            types::I32,
            types::I32,
            types::I32,
            pointer_type,
            pointer_type,
            types::I32,
            types::I32,
            types::I32,
            types::I32,
        ] {
            function.append_block_param(copy_done, ty);
        }
        let finish_capture = function.create_block();
        for ty in [types::I32, types::I32, types::I32, pointer_type] {
            function.append_block_param(finish_capture, ty);
        }
        let next_capture = function.create_block();
        function.append_block_param(next_capture, types::I32);
        let done = function.create_block();
        let capacity = function.create_block();
        let return_error = function.create_block();
        function.append_block_param(return_error, types::I32);

        function.switch_to_block(entry);
        let params = function.block_params(entry).to_vec();
        let task = params[0];
        let first = params[1];
        let count = params[2];
        let label = params[3];
        let value = params[4];
        let zero = function.ins().iconst(types::I32, 0);
        function.ins().jump(captures, &[zero.into()]);
        function.switch_to_block(captures);
        let index = function.block_params(captures)[0];
        let complete = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, index, count);
        function
            .ins()
            .brif(complete, done, &[], inspect, &[index.into()]);
        function.switch_to_block(inspect);
        let index = function.block_params(inspect)[0];
        let outputs = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, final_captures),
        );
        let absolute = function.ins().iadd(first, index);
        let capture = indexed(
            &mut function,
            outputs,
            absolute,
            mem::size_of::<FinalCaptureRecord>(),
        );
        let retained = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            capture,
            mem::offset_of!(FinalCaptureRecord, retained) as i32,
        );
        let found_label = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            capture,
            mem::offset_of!(FinalCaptureRecord, label) as i32,
        );
        let retained = function.ins().icmp_imm(IntCC::NotEqual, retained, 0);
        let same = function.ins().icmp(IntCC::Equal, found_label, label);
        let selected = function.ins().band(retained, same);
        function.ins().brif(
            selected,
            prepare,
            &[index.into()],
            next_capture,
            &[index.into()],
        );
        function.switch_to_block(prepare);
        let index = function.block_params(prepare)[0];
        let absolute = function.ins().iadd(first, index);
        let capture = indexed(
            &mut function,
            outputs,
            absolute,
            mem::size_of::<FinalCaptureRecord>(),
        );
        let text_start = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            capture,
            mem::offset_of!(FinalCaptureRecord, text_start) as i32,
        );
        let start = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            capture,
            mem::offset_of!(FinalCaptureRecord, start) as i32,
        );
        let end = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            capture,
            mem::offset_of!(FinalCaptureRecord, end) as i32,
        );
        let text_length = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            capture,
            mem::offset_of!(FinalCaptureRecord, text_length) as i32,
        );
        let transformed = function
            .ins()
            .icmp_imm(IntCC::NotEqual, text_start, NONE as i64);
        let span_length = function.ins().isub(end, start);
        let input_start = function.ins().select(transformed, text_start, start);
        let length = function.ins().select(transformed, text_length, span_length);
        let input_end = function.ins().iadd(input_start, length);
        let source = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, source),
        );
        let storage = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, output_bytes),
        );
        let input = function.ins().select(transformed, storage, source);
        let status = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, status),
        );
        let output_start = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            status,
            mem::offset_of!(TaskStatus, output_byte_count) as i32,
        );
        let reserved_end = function.ins().iadd(output_start, length);
        let no_overflow = function.ins().icmp(
            IntCC::UnsignedGreaterThanOrEqual,
            reserved_end,
            output_start,
        );
        let capacity_value = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, output_byte_quota_limit) as i32,
        );
        let bounded =
            function
                .ins()
                .icmp(IntCC::UnsignedLessThanOrEqual, reserved_end, capacity_value);
        let enough = function.ins().band(no_overflow, bounded);
        function.ins().brif(
            enough,
            search,
            &[
                index.into(),
                input_start.into(),
                input_end.into(),
                output_start.into(),
                output_start.into(),
                input.into(),
                capture.into(),
            ],
            capacity,
            &[],
        );

        function.switch_to_block(search);
        let values = function.block_params(search).to_vec();
        function.ins().jump(
            candidate,
            &[
                values[0].into(),
                values[1].into(),
                values[2].into(),
                values[3].into(),
                values[4].into(),
                values[5].into(),
                values[6].into(),
                values[1].into(),
            ],
        );
        function.switch_to_block(candidate);
        let values = function.block_params(candidate).to_vec();
        let exhausted = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThan, values[7], values[2]);
        let no_match = function.create_block();
        let try_match = function.create_block();
        function
            .ins()
            .brif(exhausted, no_match, &[], try_match, &[]);
        function.switch_to_block(no_match);
        let finish_mode = function.ins().iconst(types::I32, 1);
        function.ins().jump(
            copy,
            &[
                values[0].into(),
                values[2].into(),
                values[3].into(),
                values[4].into(),
                values[5].into(),
                values[6].into(),
                values[1].into(),
                values[2].into(),
                values[2].into(),
                finish_mode.into(),
            ],
        );
        function.switch_to_block(try_match);
        let call = function.ins().call(
            find,
            &[task, values[5], value, values[1], values[2], values[7]],
        );
        let span = function.inst_results(call)[0];
        let error = function.inst_results(call)[1];
        let failed = function.ins().icmp_imm(IntCC::NotEqual, error, 0);
        let classify = function.create_block();
        function
            .ins()
            .brif(failed, return_error, &[error.into()], classify, &[]);
        function.switch_to_block(classify);
        let absent = function.ins().icmp_imm(IntCC::Equal, span, -1);
        let next_candidate = function.create_block();
        function.ins().brif(
            absent,
            next_candidate,
            &[],
            found,
            &[
                values[0].into(),
                values[1].into(),
                values[2].into(),
                values[3].into(),
                values[4].into(),
                values[5].into(),
                values[6].into(),
                span.into(),
            ],
        );
        function.switch_to_block(next_candidate);
        let next = function.ins().iadd_imm(values[7], 1);
        function.ins().jump(
            candidate,
            &[
                values[0].into(),
                values[1].into(),
                values[2].into(),
                values[3].into(),
                values[4].into(),
                values[5].into(),
                values[6].into(),
                next.into(),
            ],
        );

        function.switch_to_block(found);
        let values = function.block_params(found).to_vec();
        let shifted = function.ins().ushr_imm(values[7], 32);
        let match_start = function.ins().ireduce(types::I32, shifted);
        let match_end = function.ins().ireduce(types::I32, values[7]);
        let nonempty = function
            .ins()
            .icmp(IntCC::UnsignedLessThan, match_start, match_end);
        let nonempty_block = function.create_block();
        let empty_block = function.create_block();
        function
            .ins()
            .brif(nonempty, nonempty_block, &[], empty_block, &[]);
        function.switch_to_block(nonempty_block);
        function.ins().jump(
            copy,
            &[
                values[0].into(),
                values[2].into(),
                values[3].into(),
                values[4].into(),
                values[5].into(),
                values[6].into(),
                values[1].into(),
                match_start.into(),
                match_end.into(),
                zero.into(),
            ],
        );
        function.switch_to_block(empty_block);
        let terminal = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, match_end, values[2]);
        let preserve = function.create_block();
        function.ins().brif(
            terminal,
            finish_capture,
            &[
                values[0].into(),
                values[3].into(),
                values[4].into(),
                values[6].into(),
            ],
            preserve,
            &[],
        );
        function.switch_to_block(preserve);
        let address = indexed(&mut function, values[5], match_end, 1);
        let lead = function
            .ins()
            .load(types::I8, MemFlags::trusted(), address, 0);
        let two = function.ins().band_imm(lead, 0xe0);
        let two = function.ins().icmp_imm(IntCC::Equal, two, 0xc0);
        let three = function.ins().band_imm(lead, 0xf0);
        let three = function.ins().icmp_imm(IntCC::Equal, three, 0xe0);
        let four = function.ins().band_imm(lead, 0xf8);
        let four = function.ins().icmp_imm(IntCC::Equal, four, 0xf0);
        let onev = function.ins().iconst(types::I32, 1);
        let twov = function.ins().iconst(types::I32, 2);
        let threev = function.ins().iconst(types::I32, 3);
        let fourv = function.ins().iconst(types::I32, 4);
        let scalar_length = function.ins().select(two, twov, onev);
        let scalar_length = function.ins().select(three, threev, scalar_length);
        let scalar_length = function.ins().select(four, fourv, scalar_length);
        let scalar_end = function.ins().iadd(match_end, scalar_length);
        let copy_end = function.ins().umin(scalar_end, values[2]);
        function.ins().jump(
            copy,
            &[
                values[0].into(),
                values[2].into(),
                values[3].into(),
                values[4].into(),
                values[5].into(),
                values[6].into(),
                match_end.into(),
                copy_end.into(),
                scalar_end.into(),
                zero.into(),
            ],
        );

        function.switch_to_block(copy);
        let values = function.block_params(copy).to_vec();
        let complete = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, values[6], values[7]);
        function.ins().brif(
            complete,
            copy_done,
            &values
                .iter()
                .copied()
                .map(cranelift_codegen::ir::BlockArg::from)
                .collect::<Vec<_>>(),
            copy_byte,
            &values
                .iter()
                .copied()
                .map(cranelift_codegen::ir::BlockArg::from)
                .collect::<Vec<_>>(),
        );
        function.switch_to_block(copy_byte);
        let values = function.block_params(copy_byte).to_vec();
        let input_at = indexed(&mut function, values[4], values[6], 1);
        let output = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, output_bytes),
        );
        let output_at = indexed(&mut function, output, values[2], 1);
        let byte = function
            .ins()
            .load(types::I8, MemFlags::trusted(), input_at, 0);
        function
            .ins()
            .store(MemFlags::trusted(), byte, output_at, 0);
        let input_next = function.ins().iadd_imm(values[6], 1);
        let write_next = function.ins().iadd_imm(values[2], 1);
        function.ins().jump(
            copy,
            &[
                values[0].into(),
                values[1].into(),
                write_next.into(),
                values[3].into(),
                values[4].into(),
                values[5].into(),
                input_next.into(),
                values[7].into(),
                values[8].into(),
                values[9].into(),
            ],
        );
        function.switch_to_block(copy_done);
        let values = function.block_params(copy_done).to_vec();
        let finish = function.ins().icmp_imm(IntCC::NotEqual, values[9], 0);
        function.ins().brif(
            finish,
            finish_capture,
            &[
                values[0].into(),
                values[3].into(),
                values[2].into(),
                values[5].into(),
            ],
            search,
            &[
                values[0].into(),
                values[8].into(),
                values[1].into(),
                values[2].into(),
                values[3].into(),
                values[4].into(),
                values[5].into(),
            ],
        );

        function.switch_to_block(finish_capture);
        let values = function.block_params(finish_capture).to_vec();
        function.ins().store(
            MemFlags::trusted(),
            values[1],
            values[3],
            mem::offset_of!(FinalCaptureRecord, text_start) as i32,
        );
        let length = function.ins().isub(values[2], values[1]);
        function.ins().store(
            MemFlags::trusted(),
            length,
            values[3],
            mem::offset_of!(FinalCaptureRecord, text_length) as i32,
        );
        function.ins().store(
            MemFlags::trusted(),
            values[2],
            status,
            mem::offset_of!(TaskStatus, output_byte_count) as i32,
        );
        function.ins().jump(next_capture, &[values[0].into()]);
        function.switch_to_block(next_capture);
        let previous = function.block_params(next_capture)[0];
        let next = function.ins().iadd_imm(previous, 1);
        function.ins().jump(captures, &[next.into()]);
        function.switch_to_block(done);
        function.ins().return_(&[zero]);
        function.switch_to_block(capacity);
        let error = function.ins().iconst(types::I32, 4);
        function.ins().return_(&[error]);
        function.switch_to_block(return_error);
        let error = function.block_params(return_error)[0];
        function.ins().return_(&[error]);
        function.seal_all_blocks();
        function.finalize();
    }
    module.define_function(id, &mut context)?;
    module.clear_context(&mut context);
    Ok(id)
}

pub(super) fn define_language_from_mime(
    module: &mut JITModule,
) -> Result<FuncId, Box<cranelift_module::ModuleError>> {
    let pointer_type = module.target_config().pointer_type();
    let mut signature = module.make_signature();
    signature.params.push(AbiParam::new(pointer_type));
    for _ in 0..5 {
        signature.params.push(AbiParam::new(types::I32));
    }
    signature.returns.push(AbiParam::new(types::I32));
    let id = module.declare_function(
        "tempo_output_language_from_mime",
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
        let captures = function.create_block();
        function.append_block_param(captures, types::I32);
        let inspect = function.create_block();
        function.append_block_param(inspect, types::I32);
        let found = function.create_block();
        function.append_block_param(found, pointer_type);
        let slash_loop = function.create_block();
        function.append_block_param(slash_loop, types::I32);
        function.append_block_param(slash_loop, types::I32);
        let normalize = function.create_block();
        function.append_block_param(normalize, types::I32);
        function.append_block_param(normalize, types::I32);
        let write_json = function.create_block();
        let write_javascript = function.create_block();
        let property = function.create_block();
        function.append_block_param(property, types::I32);
        function.append_block_param(property, types::I32);
        function.append_block_param(property, types::I32);
        let done = function.create_block();
        let reject = function.create_block();
        let capacity = function.create_block();
        function.switch_to_block(entry);
        let params = function.block_params(entry).to_vec();
        let task = params[0];
        let first = params[1];
        let count = params[2];
        let label = params[3];
        let property_index = params[4];
        let owner = params[5];
        let zero = function.ins().iconst(types::I32, 0);
        function.ins().jump(captures, &[zero.into()]);
        function.switch_to_block(captures);
        let index = function.block_params(captures)[0];
        let complete = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, index, count);
        function
            .ins()
            .brif(complete, reject, &[], inspect, &[index.into()]);
        function.switch_to_block(inspect);
        let index = function.block_params(inspect)[0];
        let outputs = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, final_captures),
        );
        let absolute = function.ins().iadd(first, index);
        let capture = indexed(
            &mut function,
            outputs,
            absolute,
            mem::size_of::<FinalCaptureRecord>(),
        );
        let retained = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            capture,
            mem::offset_of!(FinalCaptureRecord, retained) as i32,
        );
        let capture_label = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            capture,
            mem::offset_of!(FinalCaptureRecord, label) as i32,
        );
        let retained = function.ins().icmp_imm(IntCC::NotEqual, retained, 0);
        let same = function.ins().icmp(IntCC::Equal, capture_label, label);
        let selected = function.ins().band(retained, same);
        let next = function.create_block();
        function
            .ins()
            .brif(selected, found, &[capture.into()], next, &[]);
        function.switch_to_block(next);
        let next_index = function.ins().iadd_imm(index, 1);
        function.ins().jump(captures, &[next_index.into()]);
        function.switch_to_block(found);
        let capture = function.block_params(found)[0];
        let value_start = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            capture,
            mem::offset_of!(FinalCaptureRecord, start) as i32,
        );
        let value_end = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            capture,
            mem::offset_of!(FinalCaptureRecord, end) as i32,
        );
        let value_length = function.ins().isub(value_end, value_start);
        let source = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, source),
        );
        let is_nine = function.ins().icmp_imm(IntCC::Equal, value_length, 9);
        let check_import = function.create_block();
        let not_import = function.create_block();
        function
            .ins()
            .brif(is_nine, check_import, &[], not_import, &[]);
        function.switch_to_block(check_import);
        let mut importmap = function
            .ins()
            .icmp(IntCC::Equal, value_length, value_length);
        for (offset, expected) in b"importmap".iter().copied().enumerate() {
            let address = indexed_imm(&mut function, source, value_start, offset as i64);
            let byte = function
                .ins()
                .load(types::I8, MemFlags::trusted(), address, 0);
            let equal = function
                .ins()
                .icmp_imm(IntCC::Equal, byte, i64::from(expected));
            importmap = function.ins().band(importmap, equal);
        }
        function
            .ins()
            .brif(importmap, write_json, &[], not_import, &[]);
        function.switch_to_block(not_import);
        let is_four = function.ins().icmp_imm(IntCC::Equal, value_length, 4);
        let slash = function.create_block();
        function.ins().brif(
            is_four,
            normalize,
            &[value_start.into(), value_length.into()],
            slash,
            &[],
        );
        function.switch_to_block(slash);
        function
            .ins()
            .jump(slash_loop, &[value_start.into(), value_start.into()]);
        function.switch_to_block(slash_loop);
        let scan = function.block_params(slash_loop)[0];
        let selected_start = function.block_params(slash_loop)[1];
        let complete = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, scan, value_end);
        let slash_done = function.create_block();
        let slash_byte = function.create_block();
        function
            .ins()
            .brif(complete, slash_done, &[], slash_byte, &[]);
        function.switch_to_block(slash_done);
        let length = function.ins().isub(value_end, selected_start);
        function
            .ins()
            .jump(normalize, &[selected_start.into(), length.into()]);
        function.switch_to_block(slash_byte);
        let address = indexed(&mut function, source, scan, 1);
        let byte = function
            .ins()
            .load(types::I8, MemFlags::trusted(), address, 0);
        let is_slash = function.ins().icmp_imm(IntCC::Equal, byte, b'/' as i64);
        let after = function.ins().iadd_imm(scan, 1);
        let selected_start = function.ins().select(is_slash, after, selected_start);
        function
            .ins()
            .jump(slash_loop, &[after.into(), selected_start.into()]);
        function.switch_to_block(normalize);
        let value_start = function.block_params(normalize)[0];
        let value_length = function.block_params(normalize)[1];
        let is_module_len = function.ins().icmp_imm(IntCC::Equal, value_length, 6);
        let check_module = function.create_block();
        let maybe_ecma = function.create_block();
        function
            .ins()
            .brif(is_module_len, check_module, &[], maybe_ecma, &[]);
        function.switch_to_block(check_module);
        let mut module_name = function
            .ins()
            .icmp(IntCC::Equal, value_length, value_length);
        for (offset, expected) in b"module".iter().copied().enumerate() {
            let address = indexed_imm(&mut function, source, value_start, offset as i64);
            let byte = function
                .ins()
                .load(types::I8, MemFlags::trusted(), address, 0);
            let equal = function
                .ins()
                .icmp_imm(IntCC::Equal, byte, i64::from(expected));
            module_name = function.ins().band(module_name, equal);
        }
        function.ins().brif(
            module_name,
            write_javascript,
            &[],
            property,
            &[value_start.into(), value_length.into(), zero.into()],
        );
        function.switch_to_block(maybe_ecma);
        let is_ecma_len = function.ins().icmp_imm(IntCC::Equal, value_length, 10);
        let check_ecma = function.create_block();
        function.ins().brif(
            is_ecma_len,
            check_ecma,
            &[],
            property,
            &[value_start.into(), value_length.into(), zero.into()],
        );
        function.switch_to_block(check_ecma);
        let mut ecma = function
            .ins()
            .icmp(IntCC::Equal, value_length, value_length);
        for (offset, expected) in b"ecmascript".iter().copied().enumerate() {
            let address = indexed_imm(&mut function, source, value_start, offset as i64);
            let byte = function
                .ins()
                .load(types::I8, MemFlags::trusted(), address, 0);
            let equal = function
                .ins()
                .icmp_imm(IntCC::Equal, byte, i64::from(expected));
            ecma = function.ins().band(ecma, equal);
        }
        function.ins().brif(
            ecma,
            write_javascript,
            &[],
            property,
            &[value_start.into(), value_length.into(), zero.into()],
        );
        function.switch_to_block(write_json);
        let status = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, status),
        );
        let storage = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, output_bytes),
        );
        let byte_count = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            status,
            mem::offset_of!(TaskStatus, output_byte_count) as i32,
        );
        let next_count = function.ins().iadd_imm(byte_count, 4);
        let byte_capacity = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, output_byte_quota_limit) as i32,
        );
        let enough = function
            .ins()
            .icmp(IntCC::UnsignedLessThanOrEqual, next_count, byte_capacity);
        let json_ready = function.create_block();
        function.ins().brif(enough, json_ready, &[], capacity, &[]);
        function.switch_to_block(json_ready);
        for (offset, byte) in b"json".iter().copied().enumerate() {
            let address = indexed_imm(&mut function, storage, byte_count, offset as i64);
            store_byte_imm(&mut function, address, byte);
        }
        function.ins().store(
            MemFlags::trusted(),
            next_count,
            status,
            mem::offset_of!(TaskStatus, output_byte_count) as i32,
        );
        let one = function.ins().iconst(types::I32, 1);
        let four = function.ins().iconst(types::I32, 4);
        function
            .ins()
            .jump(property, &[byte_count.into(), four.into(), one.into()]);
        function.switch_to_block(write_javascript);
        let status = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, status),
        );
        let storage = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, output_bytes),
        );
        let byte_count = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            status,
            mem::offset_of!(TaskStatus, output_byte_count) as i32,
        );
        let next_count = function.ins().iadd_imm(byte_count, 10);
        let byte_capacity = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, output_byte_quota_limit) as i32,
        );
        let enough = function
            .ins()
            .icmp(IntCC::UnsignedLessThanOrEqual, next_count, byte_capacity);
        let js_ready = function.create_block();
        function.ins().brif(enough, js_ready, &[], capacity, &[]);
        function.switch_to_block(js_ready);
        for (offset, byte) in b"javascript".iter().copied().enumerate() {
            let address = indexed_imm(&mut function, storage, byte_count, offset as i64);
            store_byte_imm(&mut function, address, byte);
        }
        function.ins().store(
            MemFlags::trusted(),
            next_count,
            status,
            mem::offset_of!(TaskStatus, output_byte_count) as i32,
        );
        let one = function.ins().iconst(types::I32, 1);
        let ten = function.ins().iconst(types::I32, 10);
        function
            .ins()
            .jump(property, &[byte_count.into(), ten.into(), one.into()]);
        function.switch_to_block(property);
        let value_start = function.block_params(property)[0];
        let value_length = function.block_params(property)[1];
        let value_storage = function.block_params(property)[2];
        let status = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, status),
        );
        let storage = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, output_bytes),
        );
        let byte_count = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            status,
            mem::offset_of!(TaskStatus, output_byte_count) as i32,
        );
        let key_end = function.ins().iadd_imm(byte_count, 8);
        let byte_capacity = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, output_byte_quota_limit) as i32,
        );
        let bytes_enough =
            function
                .ins()
                .icmp(IntCC::UnsignedLessThanOrEqual, key_end, byte_capacity);
        let property_quota_limit = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, property_quota_limit) as i32,
        );
        let property_enough = function.ins().icmp(
            IntCC::UnsignedLessThan,
            property_index,
            property_quota_limit,
        );
        let enough = function.ins().band(bytes_enough, property_enough);
        let write_property = function.create_block();
        function
            .ins()
            .brif(enough, write_property, &[], capacity, &[]);
        function.switch_to_block(write_property);
        for (offset, byte) in b"language".iter().copied().enumerate() {
            let address = indexed_imm(&mut function, storage, byte_count, offset as i64);
            store_byte_imm(&mut function, address, byte);
        }
        function.ins().store(
            MemFlags::trusted(),
            key_end,
            status,
            mem::offset_of!(TaskStatus, output_byte_count) as i32,
        );
        let properties = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, properties),
        );
        let record = indexed(
            &mut function,
            properties,
            property_index,
            mem::size_of::<PropertyRecord>(),
        );
        for (offset, value) in [
            (
                mem::offset_of!(PropertyRecord, kind),
                function.ins().iconst(types::I32, 2),
            ),
            (mem::offset_of!(PropertyRecord, owner), owner),
            (mem::offset_of!(PropertyRecord, key), zero),
            (mem::offset_of!(PropertyRecord, value), value_storage),
            (mem::offset_of!(PropertyRecord, key_start), byte_count),
            (
                mem::offset_of!(PropertyRecord, key_length),
                function.ins().iconst(types::I32, 8),
            ),
            (mem::offset_of!(PropertyRecord, value_start), value_start),
            (mem::offset_of!(PropertyRecord, value_length), value_length),
        ] {
            function
                .ins()
                .store(MemFlags::trusted(), value, record, offset as i32);
        }
        function.ins().jump(done, &[]);
        function.switch_to_block(done);
        function.ins().return_(&[zero]);
        function.switch_to_block(reject);
        let one = function.ins().iconst(types::I32, 1);
        function.ins().return_(&[one]);
        function.switch_to_block(capacity);
        let four = function.ins().iconst(types::I32, 4);
        function.ins().return_(&[four]);
        function.seal_all_blocks();
        function.finalize();
    }
    module.define_function(id, &mut context)?;
    module.clear_context(&mut context);
    Ok(id)
}
