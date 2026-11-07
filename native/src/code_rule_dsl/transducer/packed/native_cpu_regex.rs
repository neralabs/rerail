use super::{
    metal_batch_abi::{MetalNode, NONE, NfaRangeRecord, NfaStateRecord, ValueRecord},
    native_cpu::NativeTaskContext,
    value::ByteNfa,
};
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlags, Value, condcodes::IntCC, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Switch};
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Linkage, Module};
use std::mem;

const REJECT: i64 = 0;
const KEEP: i64 = 1;
const INVALID: i64 = 2;
const WORK_LIMIT: i64 = 3;
const CAPACITY: i64 = 4;

#[derive(Clone, Copy)]
pub(super) struct RegexHelpers {
    closure: FuncId,
    pub(super) find: FuncId,
}

fn pointer(function: &mut FunctionBuilder<'_>, task: Value, offset: usize) -> Value {
    let pointer_type = function.func.dfg.value_type(task);
    function
        .ins()
        .load(pointer_type, MemFlags::trusted(), task, offset as i32)
}

fn indexed_address(
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

fn load_u32(function: &mut FunctionBuilder<'_>, base: Value, index: Value) -> Value {
    let address = indexed_address(function, base, index, 4);
    function
        .ins()
        .load(types::I32, MemFlags::trusted(), address, 0)
}

fn store_u32(function: &mut FunctionBuilder<'_>, base: Value, index: Value, value: Value) {
    let address = indexed_address(function, base, index, 4);
    function.ins().store(MemFlags::trusted(), value, address, 0);
}

fn return_i32(function: &mut FunctionBuilder<'_>, block: cranelift_codegen::ir::Block, code: i64) {
    function.switch_to_block(block);
    let value = function.ins().iconst(types::I32, code);
    function.ins().return_(&[value]);
}

fn define_unicode_word(
    module: &mut JITModule,
) -> Result<FuncId, Box<cranelift_module::ModuleError>> {
    let pointer_type = module.target_config().pointer_type();
    let mut signature = module.make_signature();
    signature.params.push(AbiParam::new(pointer_type));
    signature.params.push(AbiParam::new(types::I32));
    signature.returns.push(AbiParam::new(types::I32));
    let id = module.declare_function("tempo_regex_unicode_word", Linkage::Local, &signature)?;
    let mut context = module.make_context();
    context.func.signature = signature;
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = function.create_block();
        function.append_block_params_for_function_params(entry);
        let loop_block = function.create_block();
        function.append_block_param(loop_block, types::I32);
        function.append_block_param(loop_block, types::I32);
        let inspect = function.create_block();
        function.append_block_param(inspect, types::I32);
        function.append_block_param(inspect, types::I32);
        function.append_block_param(inspect, types::I32);
        let lower = function.create_block();
        function.append_block_param(lower, types::I32);
        function.append_block_param(lower, types::I32);
        let higher = function.create_block();
        function.append_block_param(higher, types::I32);
        function.append_block_param(higher, types::I32);
        let yes = function.create_block();
        let no = function.create_block();
        let invalid = function.create_block();
        function.switch_to_block(entry);
        let task = function.block_params(entry)[0];
        let scalar = function.block_params(entry)[1];
        let data_count = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, nfa_data_count) as i32,
        );
        let has_header = function
            .ins()
            .icmp_imm(IntCC::UnsignedGreaterThanOrEqual, data_count, 1);
        let begin = function.create_block();
        function.ins().brif(has_header, begin, &[], invalid, &[]);
        function.switch_to_block(begin);
        let data = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, nfa_data),
        );
        let zero = function.ins().iconst(types::I32, 0);
        let count = load_u32(&mut function, data, zero);
        let twice = function.ins().imul_imm(count, 2);
        let required = function.ins().iadd_imm(twice, 1);
        let no_overflow = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, required, count);
        let bounded = function
            .ins()
            .icmp(IntCC::UnsignedLessThanOrEqual, required, data_count);
        let valid = function.ins().band(no_overflow, bounded);
        function.ins().brif(
            valid,
            loop_block,
            &[zero.into(), count.into()],
            invalid,
            &[],
        );
        function.switch_to_block(loop_block);
        let low = function.block_params(loop_block)[0];
        let high = function.block_params(loop_block)[1];
        let done = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, low, high);
        let search = function.create_block();
        function.ins().brif(done, no, &[], search, &[]);
        function.switch_to_block(search);
        let distance = function.ins().isub(high, low);
        let half = function.ins().ushr_imm(distance, 1);
        let middle = function.ins().iadd(low, half);
        let pair = function.ins().imul_imm(middle, 2);
        let start_index = function.ins().iadd_imm(pair, 1);
        let end_index = function.ins().iadd_imm(pair, 2);
        let start = load_u32(&mut function, data, start_index);
        let end = load_u32(&mut function, data, end_index);
        function
            .ins()
            .jump(inspect, &[middle.into(), start.into(), end.into()]);
        function.switch_to_block(inspect);
        let middle = function.block_params(inspect)[0];
        let start = function.block_params(inspect)[1];
        let end = function.block_params(inspect)[2];
        let before = function.ins().icmp(IntCC::UnsignedLessThan, scalar, start);
        let after_before = function.create_block();
        function.ins().brif(
            before,
            lower,
            &[low.into(), middle.into()],
            after_before,
            &[],
        );
        function.switch_to_block(after_before);
        let after = function.ins().icmp(IntCC::UnsignedGreaterThan, scalar, end);
        let next = function.ins().iadd_imm(middle, 1);
        function
            .ins()
            .brif(after, higher, &[next.into(), high.into()], yes, &[]);
        function.switch_to_block(lower);
        let low = function.block_params(lower)[0];
        let high = function.block_params(lower)[1];
        function.ins().jump(loop_block, &[low.into(), high.into()]);
        function.switch_to_block(higher);
        let low = function.block_params(higher)[0];
        let high = function.block_params(higher)[1];
        function.ins().jump(loop_block, &[low.into(), high.into()]);
        return_i32(&mut function, yes, 1);
        return_i32(&mut function, no, 0);
        return_i32(&mut function, invalid, INVALID);
        function.seal_all_blocks();
        function.finalize();
    }
    super::native_cpu_codegen::define_function(module, id, &mut context)?;
    module.clear_context(&mut context);
    Ok(id)
}

fn define_utf8_scalar(
    module: &mut JITModule,
) -> Result<FuncId, Box<cranelift_module::ModuleError>> {
    let pointer_type = module.target_config().pointer_type();
    let mut signature = module.make_signature();
    signature.params.push(AbiParam::new(pointer_type));
    signature.params.push(AbiParam::new(types::I32));
    signature.params.push(AbiParam::new(types::I32));
    signature.returns.push(AbiParam::new(types::I32));
    let id = module.declare_function("tempo_regex_utf8_scalar", Linkage::Local, &signature)?;
    let mut context = module.make_context();
    context.func.signature = signature;
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = function.create_block();
        function.append_block_params_for_function_params(entry);
        let read = function.create_block();
        let multibyte = function.create_block();
        function.append_block_param(multibyte, types::I32);
        let bounds = function.create_block();
        function.append_block_param(bounds, types::I32);
        let loop_block = function.create_block();
        function.append_block_param(loop_block, types::I32);
        function.append_block_param(loop_block, types::I32);
        function.append_block_param(loop_block, types::I32);
        let continuation = function.create_block();
        function.append_block_param(continuation, types::I32);
        function.append_block_param(continuation, types::I32);
        function.append_block_param(continuation, types::I32);
        let zero = function.create_block();
        let direct = function.create_block();
        function.append_block_param(direct, types::I32);
        function.switch_to_block(entry);
        let source = function.block_params(entry)[0];
        let at = function.block_params(entry)[1];
        let end = function.block_params(entry)[2];
        let available = function.ins().icmp(IntCC::UnsignedLessThan, at, end);
        function.ins().brif(available, read, &[], zero, &[]);
        function.switch_to_block(read);
        let address = indexed_address(&mut function, source, at, 1);
        let lead8 = function
            .ins()
            .load(types::I8, MemFlags::trusted(), address, 0);
        let lead = function.ins().uextend(types::I32, lead8);
        let ascii = function.ins().icmp_imm(IntCC::UnsignedLessThan, lead, 128);
        function
            .ins()
            .brif(ascii, direct, &[lead.into()], multibyte, &[lead.into()]);
        function.switch_to_block(multibyte);
        let lead = function.block_params(multibyte)[0];
        let two_bits = function.ins().band_imm(lead, 0xe0);
        let is_two = function.ins().icmp_imm(IntCC::Equal, two_bits, 0xc0);
        let three_bits = function.ins().band_imm(lead, 0xf0);
        let is_three = function.ins().icmp_imm(IntCC::Equal, three_bits, 0xe0);
        let four_bits = function.ins().band_imm(lead, 0xf8);
        let is_four = function.ins().icmp_imm(IntCC::Equal, four_bits, 0xf0);
        let one = function.ins().iconst(types::I32, 1);
        let two = function.ins().iconst(types::I32, 2);
        let three = function.ins().iconst(types::I32, 3);
        let four = function.ins().iconst(types::I32, 4);
        let count = function.ins().select(is_four, four, one);
        let count = function.ins().select(is_three, three, count);
        let count = function.ins().select(is_two, two, count);
        function.ins().jump(bounds, &[count.into()]);
        function.switch_to_block(bounds);
        let count = function.block_params(bounds)[0];
        let limit = function.ins().iadd(at, count);
        let no_overflow = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, limit, at);
        let fits = function
            .ins()
            .icmp(IntCC::UnsignedLessThanOrEqual, limit, end);
        let valid = function.ins().band(no_overflow, fits);
        let prepare = function.create_block();
        function.ins().brif(valid, prepare, &[], zero, &[]);
        function.switch_to_block(prepare);
        let seven = function.ins().iconst(types::I32, 7);
        let bits = function.ins().isub(seven, count);
        let mask = function.ins().iconst(types::I32, 1);
        let mask = function.ins().ishl(mask, bits);
        let mask = function.ins().iadd_imm(mask, -1);
        let scalar = function.ins().band(lead, mask);
        let index = function.ins().iconst(types::I32, 1);
        function
            .ins()
            .jump(loop_block, &[index.into(), scalar.into(), count.into()]);
        function.switch_to_block(loop_block);
        let index = function.block_params(loop_block)[0];
        let scalar = function.block_params(loop_block)[1];
        let count = function.block_params(loop_block)[2];
        let done = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, index, count);
        let next_byte = function.create_block();
        function
            .ins()
            .brif(done, direct, &[scalar.into()], next_byte, &[]);
        function.switch_to_block(next_byte);
        let byte_at = function.ins().iadd(at, index);
        let address = indexed_address(&mut function, source, byte_at, 1);
        let byte8 = function
            .ins()
            .load(types::I8, MemFlags::trusted(), address, 0);
        let byte = function.ins().uextend(types::I32, byte8);
        let marker = function.ins().band_imm(byte, 0xc0);
        let valid = function.ins().icmp_imm(IntCC::Equal, marker, 0x80);
        function.ins().brif(
            valid,
            continuation,
            &[index.into(), scalar.into(), byte.into()],
            zero,
            &[],
        );
        function.switch_to_block(continuation);
        let index = function.block_params(continuation)[0];
        let scalar = function.block_params(continuation)[1];
        let byte = function.block_params(continuation)[2];
        let scalar = function.ins().ishl_imm(scalar, 6);
        let byte = function.ins().band_imm(byte, 0x3f);
        let scalar = function.ins().bor(scalar, byte);
        let index = function.ins().iadd_imm(index, 1);
        function
            .ins()
            .jump(loop_block, &[index.into(), scalar.into(), count.into()]);
        function.switch_to_block(direct);
        let value = function.block_params(direct)[0];
        function.ins().return_(&[value]);
        return_i32(&mut function, zero, 0);
        function.seal_all_blocks();
        function.finalize();
    }
    super::native_cpu_codegen::define_function(module, id, &mut context).inspect_err(|_| {
        #[cfg(test)]
        eprintln!("utf8 scalar:\n{}", context.func.display());
    })?;
    module.clear_context(&mut context);
    Ok(id)
}

fn define_unicode_word_side(
    module: &mut JITModule,
    unicode_word: FuncId,
    utf8_scalar: FuncId,
    left: bool,
) -> Result<FuncId, Box<cranelift_module::ModuleError>> {
    let pointer_type = module.target_config().pointer_type();
    let mut signature = module.make_signature();
    signature.params.push(AbiParam::new(pointer_type));
    signature.params.push(AbiParam::new(pointer_type));
    signature.params.push(AbiParam::new(types::I32));
    signature.params.push(AbiParam::new(types::I32));
    signature.params.push(AbiParam::new(types::I32));
    signature.returns.push(AbiParam::new(types::I32));
    let name = if left {
        "tempo_regex_unicode_word_left"
    } else {
        "tempo_regex_unicode_word_right"
    };
    let id = module.declare_function(name, Linkage::Local, &signature)?;
    let mut context = module.make_context();
    context.func.signature = signature;
    let scalar_function = module.declare_func_in_func(utf8_scalar, &mut context.func);
    let word_function = module.declare_func_in_func(unicode_word, &mut context.func);
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = function.create_block();
        function.append_block_params_for_function_params(entry);
        let find = function.create_block();
        let scan = function.create_block();
        function.append_block_param(scan, types::I32);
        let inspect = function.create_block();
        function.append_block_param(inspect, types::I32);
        let decode = function.create_block();
        function.append_block_param(decode, types::I32);
        function.append_block_param(decode, types::I32);
        let no = function.create_block();
        function.switch_to_block(entry);
        let task = function.block_params(entry)[0];
        let source = function.block_params(entry)[1];
        let start = function.block_params(entry)[2];
        let end = function.block_params(entry)[3];
        let at = function.block_params(entry)[4];
        if left {
            let available = function.ins().icmp(IntCC::UnsignedGreaterThan, at, start);
            function.ins().brif(available, find, &[], no, &[]);
            function.switch_to_block(find);
            let before = function.ins().iadd_imm(at, -1);
            function.ins().jump(scan, &[before.into()]);
            function.switch_to_block(scan);
            let before = function.block_params(scan)[0];
            let can_scan = function
                .ins()
                .icmp(IntCC::UnsignedGreaterThan, before, start);
            let read = function.create_block();
            function
                .ins()
                .brif(can_scan, read, &[], decode, &[before.into(), at.into()]);
            function.switch_to_block(read);
            let address = indexed_address(&mut function, source, before, 1);
            let byte = function
                .ins()
                .load(types::I8, MemFlags::trusted(), address, 0);
            let marker = function.ins().band_imm(byte, 0xc0);
            let continuation = function.ins().icmp_imm(IntCC::Equal, marker, 0x80);
            let previous = function.ins().iadd_imm(before, -1);
            function.ins().brif(
                continuation,
                scan,
                &[previous.into()],
                decode,
                &[before.into(), at.into()],
            );
        } else {
            let available = function.ins().icmp(IntCC::UnsignedLessThan, at, end);
            function
                .ins()
                .brif(available, inspect, &[at.into()], no, &[]);
            function.switch_to_block(find);
            function.ins().jump(no, &[]);
            function.switch_to_block(scan);
            function.ins().jump(no, &[]);
            function.switch_to_block(inspect);
            let at = function.block_params(inspect)[0];
            function.ins().jump(decode, &[at.into(), end.into()]);
        }
        function.switch_to_block(decode);
        let scalar_start = function.block_params(decode)[0];
        let scalar_end = function.block_params(decode)[1];
        let call = function
            .ins()
            .call(scalar_function, &[source, scalar_start, scalar_end]);
        let scalar = function.inst_results(call)[0];
        let call = function.ins().call(word_function, &[task, scalar]);
        let result = function.inst_results(call)[0];
        function.ins().return_(&[result]);
        return_i32(&mut function, no, 0);
        function.seal_all_blocks();
        function.finalize();
    }
    super::native_cpu_codegen::define_function(module, id, &mut context)?;
    module.clear_context(&mut context);
    Ok(id)
}

pub(super) fn define_helpers(
    module: &mut JITModule,
) -> Result<RegexHelpers, Box<cranelift_module::ModuleError>> {
    let unicode_word = define_unicode_word(module)?;
    let utf8_scalar = define_utf8_scalar(module)?;
    let unicode_word_left = define_unicode_word_side(module, unicode_word, utf8_scalar, true)?;
    let unicode_word_right = define_unicode_word_side(module, unicode_word, utf8_scalar, false)?;
    let look_matches = define_look_matches(module, unicode_word_left, unicode_word_right)?;
    let closure = define_closure(module, look_matches, false)?;
    let ordered_closure = define_closure(module, look_matches, true)?;
    let find = define_find(module, ordered_closure)?;
    Ok(RegexHelpers { closure, find })
}

// These helpers implement look assertions and ordered NFA closures for the
// native CPU backend. The Metal backend uses its separate shared frontier.
fn define_look_matches(
    module: &mut JITModule,
    unicode_word_left: FuncId,
    unicode_word_right: FuncId,
) -> Result<FuncId, Box<cranelift_module::ModuleError>> {
    let pointer_type = module.target_config().pointer_type();
    let mut signature = module.make_signature();
    signature.params.push(AbiParam::new(pointer_type));
    signature.params.push(AbiParam::new(pointer_type));
    for _ in 0..4 {
        signature.params.push(AbiParam::new(types::I32));
    }
    signature.returns.push(AbiParam::new(types::I32));
    let id = module.declare_function("tempo_regex_look_matches", Linkage::Local, &signature)?;
    let mut context = module.make_context();
    context.func.signature = signature;
    let unicode_left = module.declare_func_in_func(unicode_word_left, &mut context.func);
    let unicode_right = module.declare_func_in_func(unicode_word_right, &mut context.func);
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = function.create_block();
        function.append_block_params_for_function_params(entry);
        let left_read = function.create_block();
        let left_done = function.create_block();
        function.append_block_param(left_done, types::I8);
        let right_read = function.create_block();
        function.append_block_param(right_read, types::I8);
        let bytes_ready = function.create_block();
        function.append_block_param(bytes_ready, types::I8);
        function.append_block_param(bytes_ready, types::I8);
        let unicode = function.create_block();
        function.append_block_param(unicode, types::I8);
        function.append_block_param(unicode, types::I8);
        let evaluate = function.create_block();
        function.append_block_param(evaluate, types::I8);
        function.append_block_param(evaluate, types::I8);
        function.append_block_param(evaluate, types::I32);
        function.append_block_param(evaluate, types::I32);
        let invalid = function.create_block();
        function.switch_to_block(entry);
        let task = function.block_params(entry)[0];
        let source = function.block_params(entry)[1];
        let kind = function.block_params(entry)[2];
        let start = function.block_params(entry)[3];
        let end = function.block_params(entry)[4];
        let at = function.block_params(entry)[5];
        let zero8 = function.ins().iconst(types::I8, 0);
        let zero32 = function.ins().iconst(types::I32, 0);
        let has_left = function.ins().icmp(IntCC::UnsignedGreaterThan, at, start);
        function
            .ins()
            .brif(has_left, left_read, &[], left_done, &[zero8.into()]);
        function.switch_to_block(left_read);
        let before = function.ins().iadd_imm(at, -1);
        let address = indexed_address(&mut function, source, before, 1);
        let previous = function
            .ins()
            .load(types::I8, MemFlags::trusted(), address, 0);
        function.ins().jump(left_done, &[previous.into()]);
        function.switch_to_block(left_done);
        let previous = function.block_params(left_done)[0];
        let has_right = function.ins().icmp(IntCC::UnsignedLessThan, at, end);
        function.ins().brif(
            has_right,
            right_read,
            &[previous.into()],
            bytes_ready,
            &[previous.into(), zero8.into()],
        );
        function.switch_to_block(right_read);
        let previous = function.block_params(right_read)[0];
        let address = indexed_address(&mut function, source, at, 1);
        let current = function
            .ins()
            .load(types::I8, MemFlags::trusted(), address, 0);
        function
            .ins()
            .jump(bytes_ready, &[previous.into(), current.into()]);
        function.switch_to_block(bytes_ready);
        let previous = function.block_params(bytes_ready)[0];
        let current = function.block_params(bytes_ready)[1];
        let unicode_kinds = [256_i64, 512, 4096, 8192, 65536, 131072];
        let mut unicode_kind = function
            .ins()
            .icmp_imm(IntCC::Equal, kind, unicode_kinds[0]);
        for value in unicode_kinds.into_iter().skip(1) {
            let one = function.ins().icmp_imm(IntCC::Equal, kind, value);
            unicode_kind = function.ins().bor(unicode_kind, one);
        }
        function.ins().brif(
            unicode_kind,
            unicode,
            &[previous.into(), current.into()],
            evaluate,
            &[
                previous.into(),
                current.into(),
                zero32.into(),
                zero32.into(),
            ],
        );
        function.switch_to_block(unicode);
        let previous = function.block_params(unicode)[0];
        let current = function.block_params(unicode)[1];
        let left_call = function
            .ins()
            .call(unicode_left, &[task, source, start, end, at]);
        let unicode_left_value = function.inst_results(left_call)[0];
        let right_call = function
            .ins()
            .call(unicode_right, &[task, source, start, end, at]);
        let unicode_right_value = function.inst_results(right_call)[0];
        let left_invalid = function
            .ins()
            .icmp_imm(IntCC::Equal, unicode_left_value, INVALID);
        let right_invalid = function
            .ins()
            .icmp_imm(IntCC::Equal, unicode_right_value, INVALID);
        let bad = function.ins().bor(left_invalid, right_invalid);
        let unicode_ok = function.create_block();
        function.ins().brif(bad, invalid, &[], unicode_ok, &[]);
        function.switch_to_block(unicode_ok);
        function.ins().jump(
            evaluate,
            &[
                previous.into(),
                current.into(),
                unicode_left_value.into(),
                unicode_right_value.into(),
            ],
        );
        function.switch_to_block(evaluate);
        let previous = function.block_params(evaluate)[0];
        let current = function.block_params(evaluate)[1];
        let unicode_left_value = function.block_params(evaluate)[2];
        let unicode_right_value = function.block_params(evaluate)[3];
        let ascii = |function: &mut FunctionBuilder<'_>, byte: Value| {
            let lower =
                function
                    .ins()
                    .icmp_imm(IntCC::UnsignedGreaterThanOrEqual, byte, b'a' as i64);
            let lower_end =
                function
                    .ins()
                    .icmp_imm(IntCC::UnsignedLessThanOrEqual, byte, b'z' as i64);
            let lower = function.ins().band(lower, lower_end);
            let upper =
                function
                    .ins()
                    .icmp_imm(IntCC::UnsignedGreaterThanOrEqual, byte, b'A' as i64);
            let upper_end =
                function
                    .ins()
                    .icmp_imm(IntCC::UnsignedLessThanOrEqual, byte, b'Z' as i64);
            let upper = function.ins().band(upper, upper_end);
            let digit =
                function
                    .ins()
                    .icmp_imm(IntCC::UnsignedGreaterThanOrEqual, byte, b'0' as i64);
            let digit_end =
                function
                    .ins()
                    .icmp_imm(IntCC::UnsignedLessThanOrEqual, byte, b'9' as i64);
            let digit = function.ins().band(digit, digit_end);
            let underscore = function.ins().icmp_imm(IntCC::Equal, byte, b'_' as i64);
            let letters = function.ins().bor(lower, upper);
            let word = function.ins().bor(letters, digit);
            function.ins().bor(word, underscore)
        };
        let left_word = ascii(&mut function, previous);
        let right_word = ascii(&mut function, current);
        let unicode_left_word = function
            .ins()
            .icmp_imm(IntCC::NotEqual, unicode_left_value, 0);
        let unicode_right_word = function
            .ins()
            .icmp_imm(IntCC::NotEqual, unicode_right_value, 0);
        let at_start = function.ins().icmp(IntCC::Equal, at, start);
        let at_end = function.ins().icmp(IntCC::Equal, at, end);
        let previous_lf = function
            .ins()
            .icmp_imm(IntCC::Equal, previous, b'\n' as i64);
        let previous_cr = function
            .ins()
            .icmp_imm(IntCC::Equal, previous, b'\r' as i64);
        let current_lf = function.ins().icmp_imm(IntCC::Equal, current, b'\n' as i64);
        let current_cr = function.ins().icmp_imm(IntCC::Equal, current, b'\r' as i64);
        let mut cases = Vec::new();
        for _ in 0..18 {
            cases.push(function.create_block());
        }
        let values = [
            1_u128, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768,
            65536, 131072,
        ];
        let mut switch = Switch::new();
        for (value, block) in values.into_iter().zip(cases.iter().copied()) {
            switch.set_entry(value, block);
        }
        let not =
            |function: &mut FunctionBuilder<'_>, value: Value| function.ins().bxor_imm(value, 1);
        let ascii_boundary = function.ins().bxor(left_word, right_word);
        let not_ascii_boundary = not(&mut function, ascii_boundary);
        let unicode_boundary = function.ins().bxor(unicode_left_word, unicode_right_word);
        let not_unicode_boundary = not(&mut function, unicode_boundary);
        let not_current_lf = not(&mut function, current_lf);
        let crlf_start_tail = function.ins().bor(at_end, not_current_lf);
        let crlf_start_tail = function.ins().band(previous_cr, crlf_start_tail);
        let crlf_start_line = function.ins().bor(previous_lf, crlf_start_tail);
        let crlf_start = function.ins().bor(at_start, crlf_start_line);
        let not_previous_cr = not(&mut function, previous_cr);
        let crlf_end_tail = function.ins().bor(at_start, not_previous_cr);
        let crlf_end_tail = function.ins().band(current_lf, crlf_end_tail);
        let crlf_end_line = function.ins().bor(current_cr, crlf_end_tail);
        let crlf_end = function.ins().bor(at_end, crlf_end_line);
        let not_left = not(&mut function, left_word);
        let not_right = not(&mut function, right_word);
        let not_unicode_left = not(&mut function, unicode_left_word);
        let not_unicode_right = not(&mut function, unicode_right_word);
        let results = [
            at_start,
            at_end,
            function.ins().bor(at_start, previous_lf),
            function.ins().bor(at_end, current_lf),
            crlf_start,
            crlf_end,
            ascii_boundary,
            not_ascii_boundary,
            unicode_boundary,
            not_unicode_boundary,
            function.ins().band(not_left, right_word),
            function.ins().band(left_word, not_right),
            function.ins().band(not_unicode_left, unicode_right_word),
            function.ins().band(unicode_left_word, not_unicode_right),
            not_left,
            not_right,
            not_unicode_left,
            not_unicode_right,
        ];
        switch.emit(&mut function, kind, invalid);
        for (block, result) in cases.into_iter().zip(results) {
            function.switch_to_block(block);
            let result = function.ins().uextend(types::I32, result);
            function.ins().return_(&[result]);
        }
        return_i32(&mut function, invalid, INVALID);
        function.seal_all_blocks();
        function.finalize();
    }
    super::native_cpu_codegen::define_function(module, id, &mut context)?;
    module.clear_context(&mut context);
    Ok(id)
}

fn define_closure(
    module: &mut JITModule,
    look_matches: FuncId,
    ordered: bool,
) -> Result<FuncId, Box<cranelift_module::ModuleError>> {
    let pointer_type = module.target_config().pointer_type();
    let mut signature = module.make_signature();
    signature.params.push(AbiParam::new(pointer_type));
    signature.params.push(AbiParam::new(pointer_type));
    for _ in 0..7 {
        signature.params.push(AbiParam::new(types::I32));
    }
    if ordered {
        signature.params.push(AbiParam::new(types::I32));
        signature.params.push(AbiParam::new(types::I32));
    }
    signature.returns.push(AbiParam::new(types::I64));
    let name = if ordered {
        "tempo_regex_nfa_ordered_closure"
    } else {
        "tempo_regex_nfa_closure"
    };
    let id = module.declare_function(name, Linkage::Local, &signature)?;
    let mut context = module.make_context();
    context.func.signature = signature;
    let look = module.declare_func_in_func(look_matches, &mut context.func);
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = function.create_block();
        function.append_block_params_for_function_params(entry);
        let initialize = function.create_block();
        let loop_block = function.create_block();
        function.append_block_param(loop_block, types::I32);
        function.append_block_param(loop_block, types::I32);
        let pop = function.create_block();
        function.append_block_param(pop, types::I32);
        function.append_block_param(pop, types::I32);
        let inspect = function.create_block();
        function.append_block_param(inspect, types::I32);
        function.append_block_param(inspect, types::I32);
        function.append_block_param(inspect, types::I32);
        let byte_state = function.create_block();
        function.append_block_param(byte_state, types::I32);
        function.append_block_param(byte_state, types::I32);
        function.append_block_param(byte_state, types::I32);
        let look_state = function.create_block();
        function.append_block_param(look_state, types::I32);
        function.append_block_param(look_state, types::I32);
        function.append_block_param(look_state, types::I32);
        let split_state = function.create_block();
        function.append_block_param(split_state, types::I32);
        function.append_block_param(split_state, types::I32);
        function.append_block_param(split_state, types::I32);
        let split_loop = function.create_block();
        function.append_block_param(split_loop, types::I32);
        function.append_block_param(split_loop, types::I32);
        function.append_block_param(split_loop, types::I32);
        function.append_block_param(split_loop, types::I32);
        let capture_state = function.create_block();
        function.append_block_param(capture_state, types::I32);
        function.append_block_param(capture_state, types::I32);
        function.append_block_param(capture_state, types::I32);
        let fail_state = function.create_block();
        function.append_block_param(fail_state, types::I32);
        function.append_block_param(fail_state, types::I32);
        let match_state = function.create_block();
        function.append_block_param(match_state, types::I32);
        function.append_block_param(match_state, types::I32);
        function.append_block_param(match_state, types::I32);
        let done = function.create_block();
        function.append_block_param(done, types::I32);
        let invalid = function.create_block();
        let work_error = function.create_block();
        let capacity_error = function.create_block();
        function.switch_to_block(entry);
        let task = function.block_params(entry)[0];
        let input = function.block_params(entry)[1];
        let value_index = function.block_params(entry)[2];
        let state = function.block_params(entry)[3];
        let at = function.block_params(entry)[4];
        let start = function.block_params(entry)[5];
        let end = function.block_params(entry)[6];
        let output_start = function.block_params(entry)[7];
        let output_count = function.block_params(entry)[8];
        let value_count = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, value_count) as i32,
        );
        let valid_value = function
            .ins()
            .icmp(IntCC::UnsignedLessThan, value_index, value_count);
        function
            .ins()
            .brif(valid_value, initialize, &[], invalid, &[]);
        function.switch_to_block(initialize);
        let values = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, values),
        );
        let value_address = indexed_address(
            &mut function,
            values,
            value_index,
            mem::size_of::<ValueRecord>(),
        );
        let n = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            value_address,
            mem::offset_of!(ValueRecord, state_count) as i32,
        );
        let state_start = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            value_address,
            mem::offset_of!(ValueRecord, state_start) as i32,
        );
        let valid_state = function.ins().icmp(IntCC::UnsignedLessThan, state, n);
        let state_end = function.ins().iadd(state_start, n);
        let state_no_overflow =
            function
                .ins()
                .icmp(IntCC::UnsignedGreaterThanOrEqual, state_end, state_start);
        let state_count = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, nfa_state_count) as i32,
        );
        let states_bounded =
            function
                .ins()
                .icmp(IntCC::UnsignedLessThanOrEqual, state_end, state_count);
        let valid = function.ins().band(valid_state, state_no_overflow);
        let valid = function.ins().band(valid, states_bounded);
        let ready = function.create_block();
        function.ins().brif(valid, ready, &[], invalid, &[]);
        function.switch_to_block(ready);
        let work = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, value_work),
        );
        let base = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, value_start) as i32,
        );
        let stack_start = function.ins().iadd(base, n);
        let stack_tag_start = if ordered {
            let offset = function.ins().imul_imm(n, 4);
            let tags = function.ins().iadd(base, offset);
            let initial_tag = function.block_params(entry)[9];
            store_u32(&mut function, work, tags, initial_tag);
            Some(tags)
        } else {
            None
        };
        store_u32(&mut function, work, stack_start, state);
        let one = function.ins().iconst(types::I32, 1);
        function
            .ins()
            .jump(loop_block, &[one.into(), output_count.into()]);
        function.switch_to_block(loop_block);
        let stack_count = function.block_params(loop_block)[0];
        let output_count = function.block_params(loop_block)[1];
        let empty = function.ins().icmp_imm(IntCC::Equal, stack_count, 0);
        function.ins().brif(
            empty,
            done,
            &[output_count.into()],
            pop,
            &[stack_count.into(), output_count.into()],
        );
        function.switch_to_block(pop);
        let stack_count = function.block_params(pop)[0];
        let output_count = function.block_params(pop)[1];
        let steps = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, current_steps) as i32,
        );
        let overflow = function
            .ins()
            .icmp_imm(IntCC::Equal, steps, u32::MAX as i64);
        let step = function.create_block();
        function.ins().brif(overflow, work_error, &[], step, &[]);
        function.switch_to_block(step);
        let steps = function.ins().iadd_imm(steps, 1);
        function.ins().store(
            MemFlags::trusted(),
            steps,
            task,
            mem::offset_of!(NativeTaskContext, current_steps) as i32,
        );
        let limit = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, path_work_limit) as i32,
        );
        let over = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThan, steps, limit);
        let read = function.create_block();
        function.ins().brif(over, work_error, &[], read, &[]);
        function.switch_to_block(read);
        let next_stack_count = function.ins().iadd_imm(stack_count, -1);
        let stack_index = function.ins().iadd(stack_start, next_stack_count);
        let current = load_u32(&mut function, work, stack_index);
        let current_tag = stack_tag_start.map(|tags| {
            let index = function.ins().iadd(tags, next_stack_count);
            load_u32(&mut function, work, index)
        });
        let valid_current = function.ins().icmp(IntCC::UnsignedLessThan, current, n);
        let current_ok = function.create_block();
        function
            .ins()
            .brif(valid_current, current_ok, &[], invalid, &[]);
        function.switch_to_block(current_ok);
        let seen_index = function.ins().iadd(base, current);
        let seen = load_u32(&mut function, work, seen_index);
        let already = function.ins().icmp_imm(IntCC::NotEqual, seen, 0);
        let mark = function.create_block();
        function.ins().brif(
            already,
            loop_block,
            &[next_stack_count.into(), output_count.into()],
            mark,
            &[],
        );
        function.switch_to_block(mark);
        store_u32(&mut function, work, seen_index, one);
        let global = function.ins().iadd(state_start, current);
        let states = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, nfa_states),
        );
        let state_address = indexed_address(
            &mut function,
            states,
            global,
            mem::size_of::<NfaStateRecord>(),
        );
        let kind = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            state_address,
            mem::offset_of!(NfaStateRecord, kind) as i32,
        );
        function.ins().jump(
            inspect,
            &[kind.into(), next_stack_count.into(), output_count.into()],
        );
        function.switch_to_block(inspect);
        let kind = function.block_params(inspect)[0];
        let stack_count = function.block_params(inspect)[1];
        let output_count = function.block_params(inspect)[2];
        let mut switch = Switch::new();
        switch.set_entry(0, byte_state);
        switch.set_entry(1, look_state);
        switch.set_entry(2, split_state);
        switch.set_entry(3, capture_state);
        switch.set_entry(4, fail_state);
        switch.set_entry(5, match_state);
        let dispatch_args = &[current.into(), stack_count.into(), output_count.into()];
        // Switch cannot pass block arguments, so dispatch through kind tests.
        let is_byte = function.ins().icmp_imm(IntCC::Equal, kind, 0);
        let not_byte = function.create_block();
        function
            .ins()
            .brif(is_byte, byte_state, dispatch_args, not_byte, &[]);
        function.switch_to_block(not_byte);
        let is_look = function.ins().icmp_imm(IntCC::Equal, kind, 1);
        let not_look = function.create_block();
        function
            .ins()
            .brif(is_look, look_state, dispatch_args, not_look, &[]);
        function.switch_to_block(not_look);
        let is_split = function.ins().icmp_imm(IntCC::Equal, kind, 2);
        let not_split = function.create_block();
        function
            .ins()
            .brif(is_split, split_state, dispatch_args, not_split, &[]);
        function.switch_to_block(not_split);
        let is_capture = function.ins().icmp_imm(IntCC::Equal, kind, 3);
        let not_capture = function.create_block();
        function
            .ins()
            .brif(is_capture, capture_state, dispatch_args, not_capture, &[]);
        function.switch_to_block(not_capture);
        let is_fail = function.ins().icmp_imm(IntCC::Equal, kind, 4);
        let not_fail = function.create_block();
        function.ins().brif(
            is_fail,
            fail_state,
            &[stack_count.into(), output_count.into()],
            not_fail,
            &[],
        );
        function.switch_to_block(not_fail);
        let is_match = function.ins().icmp_imm(IntCC::Equal, kind, 5);
        function
            .ins()
            .brif(is_match, match_state, dispatch_args, invalid, &[]);
        function.switch_to_block(byte_state);
        let current = function.block_params(byte_state)[0];
        let stack_count = function.block_params(byte_state)[1];
        let output_count = function.block_params(byte_state)[2];
        let output_full = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, output_count, n);
        let append = function.create_block();
        function
            .ins()
            .brif(output_full, capacity_error, &[], append, &[]);
        function.switch_to_block(append);
        let output_index = function.ins().iadd(output_start, output_count);
        store_u32(&mut function, work, output_index, current);
        if let Some(tag) = current_tag {
            let tags = function.block_params(entry)[10];
            let index = function.ins().iadd(tags, output_count);
            store_u32(&mut function, work, index, tag);
        }
        let output_count = function.ins().iadd_imm(output_count, 1);
        function
            .ins()
            .jump(loop_block, &[stack_count.into(), output_count.into()]);
        function.switch_to_block(look_state);
        let current = function.block_params(look_state)[0];
        let stack_count = function.block_params(look_state)[1];
        let output_count = function.block_params(look_state)[2];
        let global = function.ins().iadd(state_start, current);
        let state_address = indexed_address(
            &mut function,
            states,
            global,
            mem::size_of::<NfaStateRecord>(),
        );
        let look_kind = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            state_address,
            mem::offset_of!(NfaStateRecord, data_start) as i32,
        );
        let next = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            state_address,
            mem::offset_of!(NfaStateRecord, next) as i32,
        );
        let call = function
            .ins()
            .call(look, &[task, input, look_kind, start, end, at]);
        let matched = function.inst_results(call)[0];
        let bad = function.ins().icmp_imm(IntCC::Equal, matched, INVALID);
        let look_ok = function.create_block();
        function.ins().brif(bad, invalid, &[], look_ok, &[]);
        function.switch_to_block(look_ok);
        let matched = function.ins().icmp_imm(IntCC::NotEqual, matched, 0);
        let push = function.create_block();
        function.ins().brif(
            matched,
            push,
            &[],
            loop_block,
            &[stack_count.into(), output_count.into()],
        );
        function.switch_to_block(push);
        let full = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, stack_count, n);
        let push_write = function.create_block();
        function
            .ins()
            .brif(full, capacity_error, &[], push_write, &[]);
        function.switch_to_block(push_write);
        let index = function.ins().iadd(stack_start, stack_count);
        store_u32(&mut function, work, index, next);
        if let (Some(tags), Some(tag)) = (stack_tag_start, current_tag) {
            let index = function.ins().iadd(tags, stack_count);
            store_u32(&mut function, work, index, tag);
        }
        let stack_count = function.ins().iadd_imm(stack_count, 1);
        function
            .ins()
            .jump(loop_block, &[stack_count.into(), output_count.into()]);
        function.switch_to_block(split_state);
        let current = function.block_params(split_state)[0];
        let stack_count = function.block_params(split_state)[1];
        let output_count = function.block_params(split_state)[2];
        let global = function.ins().iadd(state_start, current);
        let state_address = indexed_address(
            &mut function,
            states,
            global,
            mem::size_of::<NfaStateRecord>(),
        );
        let data_start = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            state_address,
            mem::offset_of!(NfaStateRecord, data_start) as i32,
        );
        let data_count = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            state_address,
            mem::offset_of!(NfaStateRecord, data_count) as i32,
        );
        let data_end = function.ins().iadd(data_start, data_count);
        let no_overflow =
            function
                .ins()
                .icmp(IntCC::UnsignedGreaterThanOrEqual, data_end, data_start);
        let total_data = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, nfa_data_count) as i32,
        );
        let bounded = function
            .ins()
            .icmp(IntCC::UnsignedLessThanOrEqual, data_end, total_data);
        let valid = function.ins().band(no_overflow, bounded);
        function.ins().brif(
            valid,
            split_loop,
            &[
                data_count.into(),
                stack_count.into(),
                output_count.into(),
                data_start.into(),
            ],
            invalid,
            &[],
        );
        function.switch_to_block(split_loop);
        let index = function.block_params(split_loop)[0];
        let stack_count = function.block_params(split_loop)[1];
        let output_count = function.block_params(split_loop)[2];
        let data_start = function.block_params(split_loop)[3];
        let complete = function.ins().icmp_imm(IntCC::Equal, index, 0);
        let split_push = function.create_block();
        function.ins().brif(
            complete,
            loop_block,
            &[stack_count.into(), output_count.into()],
            split_push,
            &[],
        );
        function.switch_to_block(split_push);
        let full = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, stack_count, n);
        let split_write = function.create_block();
        function
            .ins()
            .brif(full, capacity_error, &[], split_write, &[]);
        function.switch_to_block(split_write);
        let index = function.ins().iadd_imm(index, -1);
        let data_index = function.ins().iadd(data_start, index);
        let data = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, nfa_data),
        );
        let target = load_u32(&mut function, data, data_index);
        let stack_index = function.ins().iadd(stack_start, stack_count);
        store_u32(&mut function, work, stack_index, target);
        if let (Some(tags), Some(tag)) = (stack_tag_start, current_tag) {
            let index = function.ins().iadd(tags, stack_count);
            store_u32(&mut function, work, index, tag);
        }
        let stack_count = function.ins().iadd_imm(stack_count, 1);
        function.ins().jump(
            split_loop,
            &[
                index.into(),
                stack_count.into(),
                output_count.into(),
                data_start.into(),
            ],
        );
        function.switch_to_block(capture_state);
        let current = function.block_params(capture_state)[0];
        let stack_count = function.block_params(capture_state)[1];
        let output_count = function.block_params(capture_state)[2];
        let global = function.ins().iadd(state_start, current);
        let state_address = indexed_address(
            &mut function,
            states,
            global,
            mem::size_of::<NfaStateRecord>(),
        );
        let next = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            state_address,
            mem::offset_of!(NfaStateRecord, next) as i32,
        );
        let full = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, stack_count, n);
        let capture_write = function.create_block();
        function
            .ins()
            .brif(full, capacity_error, &[], capture_write, &[]);
        function.switch_to_block(capture_write);
        let stack_index = function.ins().iadd(stack_start, stack_count);
        store_u32(&mut function, work, stack_index, next);
        if let (Some(tags), Some(tag)) = (stack_tag_start, current_tag) {
            let slot = function.ins().load(
                types::I32,
                MemFlags::trusted(),
                state_address,
                mem::offset_of!(NfaStateRecord, data_start) as i32,
            );
            let start_capture = function.ins().icmp_imm(IntCC::Equal, slot, 0);
            let tag = function.ins().select(start_capture, at, tag);
            let index = function.ins().iadd(tags, stack_count);
            store_u32(&mut function, work, index, tag);
        }
        let stack_count = function.ins().iadd_imm(stack_count, 1);
        function
            .ins()
            .jump(loop_block, &[stack_count.into(), output_count.into()]);
        function.switch_to_block(fail_state);
        let stack_count = function.block_params(fail_state)[0];
        let output_count = function.block_params(fail_state)[1];
        function
            .ins()
            .jump(loop_block, &[stack_count.into(), output_count.into()]);
        function.switch_to_block(match_state);
        let match_values = function.block_params(match_state).to_vec();
        if ordered {
            let output_full =
                function
                    .ins()
                    .icmp(IntCC::UnsignedGreaterThanOrEqual, match_values[2], n);
            let append = function.create_block();
            function
                .ins()
                .brif(output_full, capacity_error, &[], append, &[]);
            function.switch_to_block(append);
            let output_index = function.ins().iadd(output_start, match_values[2]);
            store_u32(&mut function, work, output_index, match_values[0]);
            if let Some(tag) = current_tag {
                let tags = function.block_params(entry)[10];
                let index = function.ins().iadd(tags, match_values[2]);
                store_u32(&mut function, work, index, tag);
            }
            let output_count = function.ins().iadd_imm(match_values[2], 1);
            function
                .ins()
                .jump(loop_block, &[match_values[1].into(), output_count.into()]);
        } else {
            let outcome = function.ins().iconst(types::I32, KEEP);
            let outcome = function.ins().uextend(types::I64, outcome);
            let outcome = function.ins().ishl_imm(outcome, 32);
            let count = function.ins().uextend(types::I64, match_values[2]);
            let packed = function.ins().bor(outcome, count);
            function.ins().return_(&[packed]);
        }
        function.switch_to_block(done);
        let output_count = function.block_params(done)[0];
        let count = function.ins().uextend(types::I64, output_count);
        function.ins().return_(&[count]);
        for (block, code) in [
            (invalid, INVALID),
            (work_error, WORK_LIMIT),
            (capacity_error, CAPACITY),
        ] {
            function.switch_to_block(block);
            let outcome = function.ins().iconst(types::I64, code << 32);
            function.ins().return_(&[outcome]);
        }
        function.seal_all_blocks();
        function.finalize();
    }
    super::native_cpu_codegen::define_function(module, id, &mut context)?;
    module.clear_context(&mut context);
    Ok(id)
}

fn define_find(
    module: &mut JITModule,
    ordered_closure: FuncId,
) -> Result<FuncId, Box<cranelift_module::ModuleError>> {
    let pointer_type = module.target_config().pointer_type();
    let mut signature = module.make_signature();
    signature.params.push(AbiParam::new(pointer_type));
    signature.params.push(AbiParam::new(pointer_type));
    for _ in 0..5 {
        signature.params.push(AbiParam::new(types::I32));
    }
    signature.returns.push(AbiParam::new(types::I64));
    signature.returns.push(AbiParam::new(types::I32));
    let id = module.declare_function("tempo_regex_nfa_find", Linkage::Local, &signature)?;
    let mut context = module.make_context();
    context.func.signature = signature;
    let closure = module.declare_func_in_func(ordered_closure, &mut context.func);
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = function.create_block();
        function.append_block_params_for_function_params(entry);
        let initialize = function.create_block();
        let clear_initial = function.create_block();
        function.append_block_param(clear_initial, types::I32);
        let initial_closure = function.create_block();
        let outer = function.create_block();
        for _ in 0..5 {
            function.append_block_param(outer, types::I32);
        }
        let scan = function.create_block();
        for _ in 0..6 {
            function.append_block_param(scan, types::I32);
        }
        let inspect_match = function.create_block();
        for _ in 0..6 {
            function.append_block_param(inspect_match, types::I32);
        }
        let transition = function.create_block();
        for _ in 0..6 {
            function.append_block_param(transition, types::I32);
        }
        let clear_seen = function.create_block();
        for _ in 0..6 {
            function.append_block_param(clear_seen, types::I32);
        }
        let state_loop = function.create_block();
        for _ in 0..7 {
            function.append_block_param(state_loop, types::I32);
        }
        let range_loop = function.create_block();
        for _ in 0..10 {
            function.append_block_param(range_loop, types::I32);
        }
        let range_step = function.create_block();
        for _ in 0..10 {
            function.append_block_param(range_step, types::I32);
        }
        let range_match = function.create_block();
        for _ in 0..10 {
            function.append_block_param(range_match, types::I32);
        }
        let next_range = function.create_block();
        for _ in 0..10 {
            function.append_block_param(next_range, types::I32);
        }
        let next_state = function.create_block();
        for _ in 0..7 {
            function.append_block_param(next_state, types::I32);
        }
        let next_byte = function.create_block();
        for _ in 0..5 {
            function.append_block_param(next_byte, types::I32);
        }
        let return_match = function.create_block();
        function.append_block_param(return_match, types::I32);
        let return_none = function.create_block();
        let return_error = function.create_block();
        function.append_block_param(return_error, types::I32);
        let invalid = function.create_block();
        let capacity = function.create_block();
        let work_error = function.create_block();

        function.switch_to_block(entry);
        let params = function.block_params(entry).to_vec();
        let task = params[0];
        let input = params[1];
        let value_index = params[2];
        let input_start = params[3];
        let input_end = params[4];
        let candidate = params[5];
        let value_count = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, value_count) as i32,
        );
        let valid_value = function
            .ins()
            .icmp(IntCC::UnsignedLessThan, value_index, value_count);
        let ordered_input =
            function
                .ins()
                .icmp(IntCC::UnsignedLessThanOrEqual, input_start, candidate);
        let candidate_bounded =
            function
                .ins()
                .icmp(IntCC::UnsignedLessThanOrEqual, candidate, input_end);
        let source_length = params[6];
        let input_bounded =
            function
                .ins()
                .icmp(IntCC::UnsignedLessThanOrEqual, input_end, source_length);
        let valid = function.ins().band(valid_value, ordered_input);
        let valid = function.ins().band(valid, candidate_bounded);
        let valid = function.ins().band(valid, input_bounded);
        function.ins().brif(valid, initialize, &[], invalid, &[]);

        function.switch_to_block(initialize);
        let values = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, values),
        );
        let value = indexed_address(
            &mut function,
            values,
            value_index,
            mem::size_of::<ValueRecord>(),
        );
        let n = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            value,
            mem::offset_of!(ValueRecord, state_count) as i32,
        );
        let unanchored = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            value,
            mem::offset_of!(ValueRecord, operand) as i32,
        );
        let nonempty = function.ins().icmp_imm(IntCC::NotEqual, n, 0);
        let seven_n = function.ins().imul_imm(n, 7);
        let required = function.ins().iadd_imm(seven_n, 1);
        let no_overflow = function.ins().icmp_imm(
            IntCC::UnsignedLessThanOrEqual,
            n,
            ((u32::MAX - 1) / 7) as i64,
        );
        let value_capacity = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, value_capacity) as i32,
        );
        let enough =
            function
                .ins()
                .icmp(IntCC::UnsignedGreaterThanOrEqual, value_capacity, required);
        let valid_n = function.ins().band(nonempty, no_overflow);
        let workspace_ready = function.create_block();
        let classify_capacity = function.create_block();
        function
            .ins()
            .brif(valid_n, classify_capacity, &[], invalid, &[]);
        function.switch_to_block(classify_capacity);
        function
            .ins()
            .brif(enough, workspace_ready, &[], capacity, &[]);
        function.switch_to_block(workspace_ready);
        let base = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, value_start) as i32,
        );
        let work = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, value_work),
        );
        let states = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, nfa_states),
        );
        let state_start = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            value,
            mem::offset_of!(ValueRecord, state_start) as i32,
        );
        let current_offset = function.ins().imul_imm(n, 2);
        let current = function.ins().iadd(base, current_offset);
        let next_offset = function.ins().imul_imm(n, 3);
        let next = function.ins().iadd(base, next_offset);
        let tag_stride = function.ins().imul_imm(n, 3);
        let current_tags = function.ins().iadd(current, tag_stride);
        let match_tag = function.ins().iadd(base, seven_n);
        let no_tag = function.ins().iconst(types::I32, NONE as i64);
        store_u32(&mut function, work, match_tag, no_tag);
        let zero = function.ins().iconst(types::I32, 0);
        function.ins().jump(clear_initial, &[zero.into()]);
        function.switch_to_block(clear_initial);
        let index = function.block_params(clear_initial)[0];
        let clear_done = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, index, n);
        let clear_one = function.create_block();
        function
            .ins()
            .brif(clear_done, initial_closure, &[], clear_one, &[]);
        function.switch_to_block(clear_one);
        let seen = function.ins().iadd(base, index);
        store_u32(&mut function, work, seen, zero);
        let index = function.ins().iadd_imm(index, 1);
        function.ins().jump(clear_initial, &[index.into()]);
        function.switch_to_block(initial_closure);
        let call = function.ins().call(
            closure,
            &[
                task,
                input,
                value_index,
                unanchored,
                candidate,
                input_start,
                input_end,
                current,
                zero,
                no_tag,
                current_tags,
            ],
        );
        let packed = function.inst_results(call)[0];
        let shifted = function.ins().ushr_imm(packed, 32);
        let outcome = function.ins().ireduce(types::I32, shifted);
        let count = function.ins().ireduce(types::I32, packed);
        let ok = function.ins().icmp_imm(IntCC::Equal, outcome, 0);
        let initial_ready = function.create_block();
        function
            .ins()
            .brif(ok, initial_ready, &[], return_error, &[outcome.into()]);
        function.switch_to_block(initial_ready);
        let none = function.ins().iconst(types::I32, NONE as i64);
        function.ins().jump(
            outer,
            &[
                candidate.into(),
                current.into(),
                next.into(),
                count.into(),
                none.into(),
            ],
        );

        function.switch_to_block(outer);
        let values = function.block_params(outer).to_vec();
        function.ins().jump(
            scan,
            &[
                values[0].into(),
                values[1].into(),
                values[2].into(),
                values[3].into(),
                values[4].into(),
                zero.into(),
            ],
        );
        function.switch_to_block(scan);
        let values = function.block_params(scan).to_vec();
        let complete = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, values[5], values[3]);
        function.ins().brif(
            complete,
            transition,
            &[
                values[0].into(),
                values[1].into(),
                values[2].into(),
                values[3].into(),
                values[4].into(),
                values[3].into(),
            ],
            inspect_match,
            &values
                .iter()
                .copied()
                .map(cranelift_codegen::ir::BlockArg::from)
                .collect::<Vec<_>>(),
        );
        function.switch_to_block(inspect_match);
        let values = function.block_params(inspect_match).to_vec();
        let current_index = function.ins().iadd(values[1], values[5]);
        let local = load_u32(&mut function, work, current_index);
        let global = function.ins().iadd(state_start, local);
        let state = indexed_address(
            &mut function,
            states,
            global,
            mem::size_of::<NfaStateRecord>(),
        );
        let kind = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            state,
            mem::offset_of!(NfaStateRecord, kind) as i32,
        );
        let matched = function.ins().icmp_imm(IntCC::Equal, kind, 5);
        let next_scan = function.create_block();
        let save_match = function.create_block();
        function
            .ins()
            .brif(matched, save_match, &[], next_scan, &[]);
        function.switch_to_block(save_match);
        let tag_index = function.ins().iadd(current_index, tag_stride);
        let tag = load_u32(&mut function, work, tag_index);
        store_u32(&mut function, work, match_tag, tag);
        function.ins().jump(
            transition,
            &[
                values[0].into(),
                values[1].into(),
                values[2].into(),
                values[3].into(),
                values[0].into(),
                values[5].into(),
            ],
        );
        function.switch_to_block(next_scan);
        let index = function.ins().iadd_imm(values[5], 1);
        function.ins().jump(
            scan,
            &[
                values[0].into(),
                values[1].into(),
                values[2].into(),
                values[3].into(),
                values[4].into(),
                index.into(),
            ],
        );

        function.switch_to_block(transition);
        let values = function.block_params(transition).to_vec();
        let at_end = function.ins().icmp(IntCC::Equal, values[0], input_end);
        let has_fallback = function
            .ins()
            .icmp_imm(IntCC::NotEqual, values[4], NONE as i64);
        let end_with_match = function.create_block();
        let prepare = function.create_block();
        function
            .ins()
            .brif(at_end, end_with_match, &[], prepare, &[]);
        function.switch_to_block(end_with_match);
        function.ins().brif(
            has_fallback,
            return_match,
            &[values[4].into()],
            return_none,
            &[],
        );
        function.switch_to_block(prepare);
        function.ins().jump(
            clear_seen,
            &[
                values[0].into(),
                values[1].into(),
                values[2].into(),
                values[4].into(),
                values[5].into(),
                zero.into(),
            ],
        );
        function.switch_to_block(clear_seen);
        let values = function.block_params(clear_seen).to_vec();
        let complete = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, values[5], n);
        let clear = function.create_block();
        function.ins().brif(
            complete,
            state_loop,
            &[
                values[0].into(),
                values[1].into(),
                values[2].into(),
                values[3].into(),
                values[4].into(),
                zero.into(),
                zero.into(),
            ],
            clear,
            &[],
        );
        function.switch_to_block(clear);
        let seen = function.ins().iadd(base, values[5]);
        store_u32(&mut function, work, seen, zero);
        let index = function.ins().iadd_imm(values[5], 1);
        function.ins().jump(
            clear_seen,
            &[
                values[0].into(),
                values[1].into(),
                values[2].into(),
                values[3].into(),
                values[4].into(),
                index.into(),
            ],
        );

        function.switch_to_block(state_loop);
        let values = function.block_params(state_loop).to_vec();
        let complete = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, values[5], values[4]);
        let state_body = function.create_block();
        function.ins().brif(
            complete,
            next_byte,
            &[
                values[0].into(),
                values[2].into(),
                values[1].into(),
                values[3].into(),
                values[6].into(),
            ],
            state_body,
            &[],
        );
        function.switch_to_block(state_body);
        let current_index = function.ins().iadd(values[1], values[5]);
        let local = load_u32(&mut function, work, current_index);
        let global = function.ins().iadd(state_start, local);
        let state = indexed_address(
            &mut function,
            states,
            global,
            mem::size_of::<NfaStateRecord>(),
        );
        let data_start = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            state,
            mem::offset_of!(NfaStateRecord, data_start) as i32,
        );
        let data_count = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            state,
            mem::offset_of!(NfaStateRecord, data_count) as i32,
        );
        function.ins().jump(
            range_loop,
            &[
                values[0].into(),
                values[1].into(),
                values[2].into(),
                values[3].into(),
                values[4].into(),
                values[5].into(),
                values[6].into(),
                data_start.into(),
                data_count.into(),
                zero.into(),
            ],
        );

        function.switch_to_block(range_loop);
        let values = function.block_params(range_loop).to_vec();
        let complete = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, values[9], values[8]);
        function.ins().brif(
            complete,
            next_state,
            &[
                values[0].into(),
                values[1].into(),
                values[2].into(),
                values[3].into(),
                values[4].into(),
                values[5].into(),
                values[6].into(),
            ],
            range_step,
            &values
                .iter()
                .copied()
                .map(cranelift_codegen::ir::BlockArg::from)
                .collect::<Vec<_>>(),
        );
        function.switch_to_block(range_step);
        let values = function.block_params(range_step).to_vec();
        let steps = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, current_steps) as i32,
        );
        let overflow = function
            .ins()
            .icmp_imm(IntCC::Equal, steps, u32::MAX as i64);
        let step_ok = function.create_block();
        function.ins().brif(overflow, work_error, &[], step_ok, &[]);
        function.switch_to_block(step_ok);
        let steps = function.ins().iadd_imm(steps, 1);
        function.ins().store(
            MemFlags::trusted(),
            steps,
            task,
            mem::offset_of!(NativeTaskContext, current_steps) as i32,
        );
        let limit = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, path_work_limit) as i32,
        );
        let over = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThan, steps, limit);
        function.ins().brif(
            over,
            work_error,
            &[],
            range_match,
            &values
                .iter()
                .copied()
                .map(cranelift_codegen::ir::BlockArg::from)
                .collect::<Vec<_>>(),
        );
        function.switch_to_block(range_match);
        let values = function.block_params(range_match).to_vec();
        let ranges = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, nfa_ranges),
        );
        let range_index = function.ins().iadd(values[7], values[9]);
        let range = indexed_address(
            &mut function,
            ranges,
            range_index,
            mem::size_of::<NfaRangeRecord>(),
        );
        let start_byte = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            range,
            mem::offset_of!(NfaRangeRecord, start) as i32,
        );
        let end_byte = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            range,
            mem::offset_of!(NfaRangeRecord, end) as i32,
        );
        let address = indexed_address(&mut function, input, values[0], 1);
        let byte = function
            .ins()
            .load(types::I8, MemFlags::trusted(), address, 0);
        let byte = function.ins().uextend(types::I32, byte);
        let lower = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, byte, start_byte);
        let upper = function
            .ins()
            .icmp(IntCC::UnsignedLessThanOrEqual, byte, end_byte);
        let matches = function.ins().band(lower, upper);
        let matched_range = function.create_block();
        function.ins().brif(
            matches,
            matched_range,
            &[],
            next_range,
            &values
                .iter()
                .copied()
                .map(cranelift_codegen::ir::BlockArg::from)
                .collect::<Vec<_>>(),
        );
        function.switch_to_block(matched_range);
        let target = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            range,
            mem::offset_of!(NfaRangeRecord, target) as i32,
        );
        let next_at = function.ins().iadd_imm(values[0], 1);
        let current_index = function.ins().iadd(values[1], values[5]);
        let tag_index = function.ins().iadd(current_index, tag_stride);
        let tag = load_u32(&mut function, work, tag_index);
        let next_tags = function.ins().iadd(values[2], tag_stride);
        let call = function.ins().call(
            closure,
            &[
                task,
                input,
                value_index,
                target,
                next_at,
                input_start,
                input_end,
                values[2],
                values[6],
                tag,
                next_tags,
            ],
        );
        let packed = function.inst_results(call)[0];
        let shifted = function.ins().ushr_imm(packed, 32);
        let outcome = function.ins().ireduce(types::I32, shifted);
        let next_count = function.ins().ireduce(types::I32, packed);
        let ok = function.ins().icmp_imm(IntCC::Equal, outcome, 0);
        let closure_ok = function.create_block();
        function
            .ins()
            .brif(ok, closure_ok, &[], return_error, &[outcome.into()]);
        function.switch_to_block(closure_ok);
        let mut args = values.clone();
        args[6] = next_count;
        function.ins().jump(
            next_range,
            &args
                .iter()
                .copied()
                .map(cranelift_codegen::ir::BlockArg::from)
                .collect::<Vec<_>>(),
        );
        function.switch_to_block(next_range);
        let values = function.block_params(next_range).to_vec();
        let range_index = function.ins().iadd_imm(values[9], 1);
        function.ins().jump(
            range_loop,
            &[
                values[0].into(),
                values[1].into(),
                values[2].into(),
                values[3].into(),
                values[4].into(),
                values[5].into(),
                values[6].into(),
                values[7].into(),
                values[8].into(),
                range_index.into(),
            ],
        );

        function.switch_to_block(next_state);
        let values = function.block_params(next_state).to_vec();
        let state_index = function.ins().iadd_imm(values[5], 1);
        function.ins().jump(
            state_loop,
            &[
                values[0].into(),
                values[1].into(),
                values[2].into(),
                values[3].into(),
                values[4].into(),
                state_index.into(),
                values[6].into(),
            ],
        );
        function.switch_to_block(next_byte);
        let values = function.block_params(next_byte).to_vec();
        let next_empty = function.ins().icmp_imm(IntCC::Equal, values[4], 0);
        let has_fallback = function
            .ins()
            .icmp_imm(IntCC::NotEqual, values[3], NONE as i64);
        let empty = function.create_block();
        let continue_outer = function.create_block();
        function
            .ins()
            .brif(next_empty, empty, &[], continue_outer, &[]);
        function.switch_to_block(empty);
        function.ins().brif(
            has_fallback,
            return_match,
            &[values[3].into()],
            return_none,
            &[],
        );
        function.switch_to_block(continue_outer);
        let at = function.ins().iadd_imm(values[0], 1);
        function.ins().jump(
            outer,
            &[
                at.into(),
                values[1].into(),
                values[2].into(),
                values[4].into(),
                values[3].into(),
            ],
        );

        function.switch_to_block(return_match);
        let end = function.block_params(return_match)[0];
        let start = load_u32(&mut function, work, match_tag);
        let valid_start = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, start, candidate);
        let ordered = function
            .ins()
            .icmp(IntCC::UnsignedLessThanOrEqual, start, end);
        let valid = function.ins().band(valid_start, ordered);
        let encode_match = function.create_block();
        function.ins().brif(valid, encode_match, &[], invalid, &[]);
        function.switch_to_block(encode_match);
        let start = function.ins().uextend(types::I64, start);
        let start = function.ins().ishl_imm(start, 32);
        let end = function.ins().uextend(types::I64, end);
        let span = function.ins().bor(start, end);
        function.ins().return_(&[span, zero]);
        function.switch_to_block(return_none);
        let none64 = function.ins().iconst(types::I64, -1);
        function.ins().return_(&[none64, zero]);
        function.switch_to_block(return_error);
        let error = function.block_params(return_error)[0];
        let none64 = function.ins().iconst(types::I64, -1);
        function.ins().return_(&[none64, error]);
        for (block, code) in [
            (invalid, INVALID),
            (capacity, CAPACITY),
            (work_error, WORK_LIMIT),
        ] {
            function.switch_to_block(block);
            let none64 = function.ins().iconst(types::I64, -1);
            let error = function.ins().iconst(types::I32, code);
            function.ins().return_(&[none64, error]);
        }
        function.seal_all_blocks();
        function.finalize();
    }
    super::native_cpu_codegen::define_function(module, id, &mut context)?;
    module.clear_context(&mut context);
    Ok(id)
}

pub(super) fn define(
    module: &mut JITModule,
    id: FuncId,
    _nfa: &ByteNfa,
    value_index: u32,
    helpers: RegexHelpers,
) -> Result<(), Box<cranelift_module::ModuleError>> {
    let pointer_type = module.target_config().pointer_type();
    let mut context = module.make_context();
    context.func.signature = module.make_signature();
    context
        .func
        .signature
        .params
        .push(AbiParam::new(pointer_type));
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
    let closure = module.declare_func_in_func(helpers.closure, &mut context.func);
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = function.create_block();
        function.append_block_params_for_function_params(entry);
        let initialize = function.create_block();
        let clear_initial = function.create_block();
        function.append_block_param(clear_initial, types::I32);
        let initial_closure = function.create_block();
        let outer = function.create_block();
        function.append_block_param(outer, types::I32);
        function.append_block_param(outer, types::I32);
        function.append_block_param(outer, types::I32);
        function.append_block_param(outer, types::I32);
        let clear_next = function.create_block();
        function.append_block_param(clear_next, types::I32);
        function.append_block_param(clear_next, types::I32);
        function.append_block_param(clear_next, types::I32);
        function.append_block_param(clear_next, types::I32);
        let state_loop = function.create_block();
        function.append_block_param(state_loop, types::I32);
        function.append_block_param(state_loop, types::I32);
        function.append_block_param(state_loop, types::I32);
        function.append_block_param(state_loop, types::I32);
        let range_loop = function.create_block();
        for _ in 0..7 {
            function.append_block_param(range_loop, types::I32);
        }
        let range_match = function.create_block();
        for _ in 0..7 {
            function.append_block_param(range_match, types::I32);
        }
        let next_state = function.create_block();
        for _ in 0..4 {
            function.append_block_param(next_state, types::I32);
        }
        let next_byte = function.create_block();
        for _ in 0..4 {
            function.append_block_param(next_byte, types::I32);
        }
        let return_outcome = function.create_block();
        function.append_block_param(return_outcome, types::I32);
        let reject = function.create_block();
        let invalid = function.create_block();
        let capacity = function.create_block();
        let work_error = function.create_block();
        function.switch_to_block(entry);
        let task = function.block_params(entry)[0];
        let node_index = function.block_params(entry)[1];
        let source = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, source),
        );
        let node_count = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, node_count) as i32,
        );
        let valid_node = function
            .ins()
            .icmp(IntCC::UnsignedLessThan, node_index, node_count);
        function
            .ins()
            .brif(valid_node, initialize, &[], invalid, &[]);
        function.switch_to_block(initialize);
        let nodes = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, nodes),
        );
        let node_address = indexed_address(
            &mut function,
            nodes,
            node_index,
            mem::size_of::<MetalNode>(),
        );
        let source_start = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            node_address,
            mem::offset_of!(MetalNode, source_start) as i32,
        );
        let source_end = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            node_address,
            mem::offset_of!(MetalNode, source_end) as i32,
        );
        let source_length = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, source_length) as i32,
        );
        let ordered =
            function
                .ins()
                .icmp(IntCC::UnsignedGreaterThanOrEqual, source_end, source_start);
        let source_bounded =
            function
                .ins()
                .icmp(IntCC::UnsignedLessThanOrEqual, source_end, source_length);
        let value_count = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, value_count) as i32,
        );
        let valid_value = function.ins().icmp_imm(
            IntCC::UnsignedGreaterThan,
            value_count,
            i64::from(value_index),
        );
        let valid = function.ins().band(ordered, source_bounded);
        let valid = function.ins().band(valid, valid_value);
        let value_ready = function.create_block();
        function.ins().brif(valid, value_ready, &[], invalid, &[]);
        function.switch_to_block(value_ready);
        let values = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, values),
        );
        let value_constant = function.ins().iconst(types::I32, i64::from(value_index));
        let value_address = indexed_address(
            &mut function,
            values,
            value_constant,
            mem::size_of::<ValueRecord>(),
        );
        let kind = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            value_address,
            mem::offset_of!(ValueRecord, kind) as i32,
        );
        let is_regex = function.ins().icmp_imm(IntCC::Equal, kind, 2);
        let n = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            value_address,
            mem::offset_of!(ValueRecord, state_count) as i32,
        );
        let nonzero = function.ins().icmp_imm(IntCC::NotEqual, n, 0);
        let four_n = function.ins().imul_imm(n, 4);
        let no_overflow = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, four_n, n);
        let value_capacity = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, value_capacity) as i32,
        );
        let enough = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, value_capacity, four_n);
        let valid = function.ins().band(is_regex, nonzero);
        let valid = function.ins().band(valid, no_overflow);
        let workspace_ready = function.create_block();
        function
            .ins()
            .brif(valid, workspace_ready, &[], invalid, &[]);
        function.switch_to_block(workspace_ready);
        let capacity_ready = function.create_block();
        function
            .ins()
            .brif(enough, capacity_ready, &[], capacity, &[]);
        function.switch_to_block(capacity_ready);
        let base = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, value_start) as i32,
        );
        let twice_n = function.ins().imul_imm(n, 2);
        let three_n = function.ins().imul_imm(n, 3);
        let current_start = function.ins().iadd(base, twice_n);
        let next_start = function.ins().iadd(base, three_n);
        let zero = function.ins().iconst(types::I32, 0);
        let work = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, value_work),
        );
        function.ins().jump(clear_initial, &[zero.into()]);
        function.switch_to_block(clear_initial);
        let index = function.block_params(clear_initial)[0];
        let done = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, index, n);
        let clear_one = function.create_block();
        function
            .ins()
            .brif(done, initial_closure, &[], clear_one, &[]);
        function.switch_to_block(clear_one);
        let target = function.ins().iadd(base, index);
        store_u32(&mut function, work, target, zero);
        let index = function.ins().iadd_imm(index, 1);
        function.ins().jump(clear_initial, &[index.into()]);
        function.switch_to_block(initial_closure);
        let start_state = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            value_address,
            mem::offset_of!(ValueRecord, operand) as i32,
        );
        let call = function.ins().call(
            closure,
            &[
                task,
                source,
                value_constant,
                start_state,
                source_start,
                source_start,
                source_end,
                current_start,
                zero,
            ],
        );
        let packed = function.inst_results(call)[0];
        let outcome64 = function.ins().ushr_imm(packed, 32);
        let outcome = function.ins().ireduce(types::I32, outcome64);
        let current_count = function.ins().ireduce(types::I32, packed);
        let failed = function.ins().icmp_imm(IntCC::NotEqual, outcome, 0);
        function.ins().brif(
            failed,
            return_outcome,
            &[outcome.into()],
            outer,
            &[
                source_start.into(),
                current_start.into(),
                next_start.into(),
                current_count.into(),
            ],
        );
        function.switch_to_block(outer);
        let at = function.block_params(outer)[0];
        let current = function.block_params(outer)[1];
        let next = function.block_params(outer)[2];
        let current_count = function.block_params(outer)[3];
        let complete = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, at, source_end);
        function.ins().brif(
            complete,
            reject,
            &[],
            clear_next,
            &[zero.into(), at.into(), current.into(), next.into()],
        );
        function.switch_to_block(clear_next);
        let index = function.block_params(clear_next)[0];
        let at = function.block_params(clear_next)[1];
        let current = function.block_params(clear_next)[2];
        let next = function.block_params(clear_next)[3];
        let done = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, index, n);
        let clear_one = function.create_block();
        function.ins().brif(
            done,
            state_loop,
            &[zero.into(), zero.into(), at.into(), current.into()],
            clear_one,
            &[],
        );
        function.switch_to_block(clear_one);
        let target = function.ins().iadd(base, index);
        store_u32(&mut function, work, target, zero);
        let index = function.ins().iadd_imm(index, 1);
        function.ins().jump(
            clear_next,
            &[index.into(), at.into(), current.into(), next.into()],
        );
        function.switch_to_block(state_loop);
        let index = function.block_params(state_loop)[0];
        let next_count = function.block_params(state_loop)[1];
        let at = function.block_params(state_loop)[2];
        let current = function.block_params(state_loop)[3];
        let all_states =
            function
                .ins()
                .icmp(IntCC::UnsignedGreaterThanOrEqual, index, current_count);
        function.ins().brif(
            all_states,
            next_byte,
            &[at.into(), current.into(), next.into(), next_count.into()],
            next_state,
            &[index.into(), next_count.into(), at.into(), current.into()],
        );
        function.switch_to_block(next_state);
        let index = function.block_params(next_state)[0];
        let next_count = function.block_params(next_state)[1];
        let at = function.block_params(next_state)[2];
        let current = function.block_params(next_state)[3];
        let current_index = function.ins().iadd(current, index);
        let local_state = load_u32(&mut function, work, current_index);
        let valid_state = function.ins().icmp(IntCC::UnsignedLessThan, local_state, n);
        let state_ok = function.create_block();
        function
            .ins()
            .brif(valid_state, state_ok, &[], invalid, &[]);
        function.switch_to_block(state_ok);
        let state_start = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            value_address,
            mem::offset_of!(ValueRecord, state_start) as i32,
        );
        let global_state = function.ins().iadd(state_start, local_state);
        let nfa_state_count = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, nfa_state_count) as i32,
        );
        let state_bounded =
            function
                .ins()
                .icmp(IntCC::UnsignedLessThan, global_state, nfa_state_count);
        let state_read = function.create_block();
        function
            .ins()
            .brif(state_bounded, state_read, &[], invalid, &[]);
        function.switch_to_block(state_read);
        let states = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, nfa_states),
        );
        let state_address = indexed_address(
            &mut function,
            states,
            global_state,
            mem::size_of::<NfaStateRecord>(),
        );
        let state_kind = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            state_address,
            mem::offset_of!(NfaStateRecord, kind) as i32,
        );
        let is_bytes = function.ins().icmp_imm(IntCC::Equal, state_kind, 0);
        let bytes_state = function.create_block();
        function
            .ins()
            .brif(is_bytes, bytes_state, &[], invalid, &[]);
        function.switch_to_block(bytes_state);
        let range_start = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            state_address,
            mem::offset_of!(NfaStateRecord, data_start) as i32,
        );
        let range_count = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            state_address,
            mem::offset_of!(NfaStateRecord, data_count) as i32,
        );
        let range_end = function.ins().iadd(range_start, range_count);
        let range_no_overflow =
            function
                .ins()
                .icmp(IntCC::UnsignedGreaterThanOrEqual, range_end, range_start);
        let total_ranges = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, nfa_range_count) as i32,
        );
        let ranges_bounded =
            function
                .ins()
                .icmp(IntCC::UnsignedLessThanOrEqual, range_end, total_ranges);
        let ranges_valid = function.ins().band(range_no_overflow, ranges_bounded);
        function.ins().brif(
            ranges_valid,
            range_loop,
            &[
                zero.into(),
                range_start.into(),
                range_count.into(),
                index.into(),
                next_count.into(),
                at.into(),
                current.into(),
            ],
            invalid,
            &[],
        );
        function.switch_to_block(range_loop);
        let range_index = function.block_params(range_loop)[0];
        let range_start = function.block_params(range_loop)[1];
        let range_count = function.block_params(range_loop)[2];
        let state_index = function.block_params(range_loop)[3];
        let next_count = function.block_params(range_loop)[4];
        let at = function.block_params(range_loop)[5];
        let current = function.block_params(range_loop)[6];
        let ranges_done =
            function
                .ins()
                .icmp(IntCC::UnsignedGreaterThanOrEqual, range_index, range_count);
        let advance_state = function.create_block();
        function.ins().brif(
            ranges_done,
            advance_state,
            &[],
            range_match,
            &[
                range_index.into(),
                range_start.into(),
                range_count.into(),
                state_index.into(),
                next_count.into(),
                at.into(),
                current.into(),
            ],
        );
        function.switch_to_block(advance_state);
        let state_index = function.ins().iadd_imm(state_index, 1);
        function.ins().jump(
            state_loop,
            &[
                state_index.into(),
                next_count.into(),
                at.into(),
                current.into(),
            ],
        );
        function.switch_to_block(range_match);
        let range_index = function.block_params(range_match)[0];
        let range_start = function.block_params(range_match)[1];
        let range_count = function.block_params(range_match)[2];
        let state_index = function.block_params(range_match)[3];
        let next_count = function.block_params(range_match)[4];
        let at = function.block_params(range_match)[5];
        let current = function.block_params(range_match)[6];
        let steps = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, current_steps) as i32,
        );
        let overflow = function
            .ins()
            .icmp_imm(IntCC::Equal, steps, u32::MAX as i64);
        let step = function.create_block();
        function.ins().brif(overflow, work_error, &[], step, &[]);
        function.switch_to_block(step);
        let steps = function.ins().iadd_imm(steps, 1);
        function.ins().store(
            MemFlags::trusted(),
            steps,
            task,
            mem::offset_of!(NativeTaskContext, current_steps) as i32,
        );
        let work_limit = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, path_work_limit) as i32,
        );
        let over = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThan, steps, work_limit);
        let read_range = function.create_block();
        function.ins().brif(over, work_error, &[], read_range, &[]);
        function.switch_to_block(read_range);
        let global_range = function.ins().iadd(range_start, range_index);
        let ranges = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, nfa_ranges),
        );
        let range_address = indexed_address(
            &mut function,
            ranges,
            global_range,
            mem::size_of::<NfaRangeRecord>(),
        );
        let first = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            range_address,
            mem::offset_of!(NfaRangeRecord, start) as i32,
        );
        let last = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            range_address,
            mem::offset_of!(NfaRangeRecord, end) as i32,
        );
        let target = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            range_address,
            mem::offset_of!(NfaRangeRecord, target) as i32,
        );
        let byte_address = indexed_address(&mut function, source, at, 1);
        let byte8 = function
            .ins()
            .load(types::I8, MemFlags::trusted(), byte_address, 0);
        let byte = function.ins().uextend(types::I32, byte8);
        let low = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, byte, first);
        let high = function
            .ins()
            .icmp(IntCC::UnsignedLessThanOrEqual, byte, last);
        let matches = function.ins().band(low, high);
        let matched = function.create_block();
        let range_next = function.create_block();
        function.append_block_param(range_next, types::I32);
        function
            .ins()
            .brif(matches, matched, &[], range_next, &[next_count.into()]);
        function.switch_to_block(matched);
        let next_at = function.ins().iadd_imm(at, 1);
        let call = function.ins().call(
            closure,
            &[
                task,
                source,
                value_constant,
                target,
                next_at,
                source_start,
                source_end,
                next,
                next_count,
            ],
        );
        let packed = function.inst_results(call)[0];
        let outcome64 = function.ins().ushr_imm(packed, 32);
        let outcome = function.ins().ireduce(types::I32, outcome64);
        let updated_count = function.ins().ireduce(types::I32, packed);
        let failed = function.ins().icmp_imm(IntCC::NotEqual, outcome, 0);
        function.ins().brif(
            failed,
            return_outcome,
            &[outcome.into()],
            range_next,
            &[updated_count.into()],
        );
        function.switch_to_block(range_next);
        let count = function.block_params(range_next)[0];
        let range_index = function.ins().iadd_imm(range_index, 1);
        function.ins().jump(
            range_loop,
            &[
                range_index.into(),
                range_start.into(),
                range_count.into(),
                state_index.into(),
                count.into(),
                at.into(),
                current.into(),
            ],
        );
        function.switch_to_block(next_byte);
        let at = function.block_params(next_byte)[0];
        let current = function.block_params(next_byte)[1];
        let next = function.block_params(next_byte)[2];
        let next_count = function.block_params(next_byte)[3];
        let next_at = function.ins().iadd_imm(at, 1);
        function.ins().jump(
            outer,
            &[
                next_at.into(),
                next.into(),
                current.into(),
                next_count.into(),
            ],
        );
        function.switch_to_block(return_outcome);
        let outcome = function.block_params(return_outcome)[0];
        function.ins().return_(&[outcome]);
        return_i32(&mut function, reject, REJECT);
        return_i32(&mut function, invalid, INVALID);
        return_i32(&mut function, capacity, CAPACITY);
        return_i32(&mut function, work_error, WORK_LIMIT);
        function.seal_all_blocks();
        function.finalize();
    }
    super::native_cpu_codegen::define_function(module, id, &mut context)?;
    module.clear_context(&mut context);
    Ok(())
}
