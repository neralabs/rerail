use super::{
    metal_batch_abi::MetalNode,
    native_cpu::NativeTaskContext,
    value::{LuaInstruction, LuaProgram, ValueMatcher, ValueSetMember},
};
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlags, Value, condcodes::IntCC, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Switch};
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Module};
use std::mem;

fn pointer(function: &mut FunctionBuilder<'_>, base: Value, offset: usize) -> Value {
    let ty = function.func.dfg.value_type(base);
    function
        .ins()
        .load(ty, MemFlags::trusted(), base, offset as i32)
}

fn word_address(function: &mut FunctionBuilder<'_>, base: Value, index: Value) -> Value {
    let ty = function.func.dfg.value_type(base);
    let offset = function.ins().imul_imm(index, 4);
    let offset = function.ins().uextend(ty, offset);
    function.ins().iadd(base, offset)
}

fn load_word(function: &mut FunctionBuilder<'_>, base: Value, index: Value) -> Value {
    let address = word_address(function, base, index);
    function
        .ins()
        .load(types::I32, MemFlags::trusted(), address, 0)
}

fn store_word(function: &mut FunctionBuilder<'_>, base: Value, index: Value, value: Value) {
    let address = word_address(function, base, index);
    function.ins().store(MemFlags::trusted(), value, address, 0);
}

fn bool_not(function: &mut FunctionBuilder<'_>, value: Value) -> Value {
    function.ins().bxor_imm(value, 1)
}

fn byte_between(function: &mut FunctionBuilder<'_>, byte: Value, start: u8, end: u8) -> Value {
    let low = function
        .ins()
        .icmp_imm(IntCC::UnsignedGreaterThanOrEqual, byte, i64::from(start));
    let high = function
        .ins()
        .icmp_imm(IntCC::UnsignedLessThanOrEqual, byte, i64::from(end));
    function.ins().band(low, high)
}

fn class_match(function: &mut FunctionBuilder<'_>, class: u8, byte: Value) -> Value {
    let lower = class.to_ascii_lowercase();
    let alpha_lower = byte_between(function, byte, b'a', b'z');
    let alpha_upper = byte_between(function, byte, b'A', b'Z');
    let alpha = function.ins().bor(alpha_lower, alpha_upper);
    let digit = byte_between(function, byte, b'0', b'9');
    match lower {
        b'a' => alpha,
        b'c' => {
            let low = function.ins().icmp_imm(IntCC::UnsignedLessThan, byte, 32);
            let del = function.ins().icmp_imm(IntCC::Equal, byte, 127);
            function.ins().bor(low, del)
        }
        b'd' => digit,
        b'g' => byte_between(function, byte, 33, 126),
        b'l' => alpha_lower,
        b'p' => {
            let graphic = byte_between(function, byte, 33, 126);
            let alphanumeric = function.ins().bor(alpha, digit);
            let not_alphanumeric = bool_not(function, alphanumeric);
            function.ins().band(graphic, not_alphanumeric)
        }
        b's' => {
            let space = function.ins().icmp_imm(IntCC::Equal, byte, b' ' as i64);
            let tab_cr = byte_between(function, byte, b'\t', b'\r');
            function.ins().bor(space, tab_cr)
        }
        b'u' => alpha_upper,
        b'w' => function.ins().bor(alpha, digit),
        b'x' => {
            let lower = byte_between(function, byte, b'a', b'f');
            let upper = byte_between(function, byte, b'A', b'F');
            let letters = function.ins().bor(lower, upper);
            function.ins().bor(digit, letters)
        }
        b'z' => function.ins().icmp_imm(IntCC::Equal, byte, 0),
        _ => function.ins().icmp(IntCC::NotEqual, byte, byte),
    }
}

fn matcher(function: &mut FunctionBuilder<'_>, found: &ValueMatcher, byte: Value) -> Value {
    match found {
        ValueMatcher::Any => function.ins().icmp(IntCC::Equal, byte, byte),
        ValueMatcher::Byte(expected) => {
            function
                .ins()
                .icmp_imm(IntCC::Equal, byte, i64::from(*expected))
        }
        ValueMatcher::Class { class, negated } => {
            let result = class_match(function, *class, byte);
            if *negated {
                bool_not(function, result)
            } else {
                result
            }
        }
        ValueMatcher::Set { members, negated } => {
            let mut result = function.ins().icmp(IntCC::NotEqual, byte, byte);
            for member in members {
                let one = match member {
                    ValueSetMember::Byte(expected) => {
                        function
                            .ins()
                            .icmp_imm(IntCC::Equal, byte, i64::from(*expected))
                    }
                    ValueSetMember::Range(start, end) => byte_between(function, byte, *start, *end),
                    ValueSetMember::Class { class, negated } => {
                        let value = class_match(function, *class, byte);
                        if *negated {
                            bool_not(function, value)
                        } else {
                            value
                        }
                    }
                };
                result = function.ins().bor(result, one);
            }
            if *negated {
                bool_not(function, result)
            } else {
                result
            }
        }
    }
}

fn return_code(function: &mut FunctionBuilder<'_>, block: cranelift_codegen::ir::Block, code: i64) {
    function.switch_to_block(block);
    let value = function.ins().iconst(types::I32, code);
    function.ins().return_(&[value]);
}

pub(super) fn define(
    module: &mut JITModule,
    id: FuncId,
    program: &LuaProgram,
) -> Result<(), Box<cranelift_module::ModuleError>> {
    let mut context = module.make_context();
    context.func.signature = module.make_signature();
    let pointer_type = module.target_config().pointer_type();
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
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = function.create_block();
        function.append_block_params_for_function_params(entry);
        let initialize = function.create_block();
        let clear = function.create_block();
        function.append_block_param(clear, types::I32);
        let initialize_anchored = function.create_block();
        let initialize_unanchored = function.create_block();
        function.append_block_param(initialize_unanchored, types::I32);
        function.append_block_param(initialize_unanchored, types::I32);
        let pop = function.create_block();
        function.append_block_param(pop, types::I32);
        let copy_capture = function.create_block();
        function.append_block_param(copy_capture, types::I32);
        function.append_block_param(copy_capture, types::I32);
        function.append_block_param(copy_capture, types::I32);
        function.append_block_param(copy_capture, types::I32);
        let dispatch = function.create_block();
        function.append_block_param(dispatch, types::I32);
        function.append_block_param(dispatch, types::I32);
        function.append_block_param(dispatch, types::I32);
        let reject_path = function.create_block();
        function.append_block_param(reject_path, types::I32);
        let success = function.create_block();
        let reject = function.create_block();
        let invalid = function.create_block();
        let work_limit = function.create_block();
        let capacity = function.create_block();

        function.switch_to_block(entry);
        let task = function.block_params(entry)[0];
        let node_index = function.block_params(entry)[1];
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
        let source = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, source),
        );
        let node_offset = function
            .ins()
            .imul_imm(node_index, mem::size_of::<MetalNode>() as i64);
        let node_offset = function.ins().uextend(pointer_type, node_offset);
        let node = function.ins().iadd(nodes, node_offset);
        let source_start = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            node,
            mem::offset_of!(MetalNode, source_start) as i32,
        );
        let source_end = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            node,
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
        let in_bounds =
            function
                .ins()
                .icmp(IntCC::UnsignedLessThanOrEqual, source_end, source_length);
        let valid_range = function.ins().band(ordered, in_bounds);
        let workspace = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, value_work),
        );
        let value_start = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, value_start) as i32,
        );
        let value_capacity = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, value_capacity) as i32,
        );
        let capture_words = program.capture_count * 3;
        let stride = 2 + capture_words;
        let enough_workspace = function.ins().icmp_imm(
            IntCC::UnsignedGreaterThanOrEqual,
            value_capacity,
            i64::from(stride + capture_words),
        );
        let valid = function.ins().band(valid_range, enough_workspace);
        let workspace_ready = function.create_block();
        function
            .ins()
            .brif(valid, workspace_ready, &[], capacity, &[]);

        function.switch_to_block(workspace_ready);
        let remaining = function
            .ins()
            .iadd_imm(value_capacity, -i64::from(capture_words));
        let stride_value = function.ins().iconst(types::I32, i64::from(stride));
        let maximum = function.ins().udiv(remaining, stride_value);
        let records_words = function.ins().imul(maximum, stride_value);
        let current = function.ins().iadd(value_start, records_words);
        let zero = function.ins().iconst(types::I32, 0);
        function.ins().jump(clear, &[zero.into()]);

        function.switch_to_block(clear);
        let clear_at = function.block_params(clear)[0];
        let clear_done = function.ins().icmp_imm(
            IntCC::UnsignedGreaterThanOrEqual,
            clear_at,
            i64::from(capture_words),
        );
        let cleared = function.create_block();
        let clear_one = function.create_block();
        function
            .ins()
            .brif(clear_done, cleared, &[], clear_one, &[]);
        function.switch_to_block(clear_one);
        let clear_index = function.ins().iadd(current, clear_at);
        store_word(&mut function, workspace, clear_index, zero);
        let next_clear = function.ins().iadd_imm(clear_at, 1);
        function.ins().jump(clear, &[next_clear.into()]);

        function.switch_to_block(cleared);
        if program.anchored {
            function.ins().jump(initialize_anchored, &[]);
        } else {
            let cursor = function.ins().iadd_imm(source_end, 1);
            function
                .ins()
                .jump(initialize_unanchored, &[cursor.into(), zero.into()]);
        }

        function.switch_to_block(initialize_anchored);
        let has_record = function
            .ins()
            .icmp_imm(IntCC::UnsignedGreaterThan, maximum, 0);
        let anchored_push = function.create_block();
        function
            .ins()
            .brif(has_record, anchored_push, &[], capacity, &[]);
        function.switch_to_block(anchored_push);
        store_word(&mut function, workspace, value_start, zero);
        let anchored_at = function.ins().iadd_imm(value_start, 1);
        store_word(&mut function, workspace, anchored_at, source_start);
        let one = function.ins().iconst(types::I32, 1);
        function.ins().jump(pop, &[one.into()]);

        function.switch_to_block(initialize_unanchored);
        let cursor = function.block_params(initialize_unanchored)[0];
        let count = function.block_params(initialize_unanchored)[1];
        let done = function
            .ins()
            .icmp(IntCC::UnsignedLessThanOrEqual, cursor, source_start);
        let init_push = function.create_block();
        function
            .ins()
            .brif(done, pop, &[count.into()], init_push, &[]);
        function.switch_to_block(init_push);
        let full = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, count, maximum);
        let init_store = function.create_block();
        function.ins().brif(full, capacity, &[], init_store, &[]);
        function.switch_to_block(init_store);
        let record_offset = function.ins().imul(count, stride_value);
        let record = function.ins().iadd(value_start, record_offset);
        store_word(&mut function, workspace, record, zero);
        let at_index = function.ins().iadd_imm(record, 1);
        let at = function.ins().iadd_imm(cursor, -1);
        store_word(&mut function, workspace, at_index, at);
        for offset in 0..capture_words {
            let index = function.ins().iadd_imm(record, i64::from(2 + offset));
            store_word(&mut function, workspace, index, zero);
        }
        let next_cursor = function.ins().iadd_imm(cursor, -1);
        let next_count = function.ins().iadd_imm(count, 1);
        function.ins().jump(
            initialize_unanchored,
            &[next_cursor.into(), next_count.into()],
        );

        function.switch_to_block(pop);
        let count = function.block_params(pop)[0];
        let empty = function.ins().icmp_imm(IntCC::Equal, count, 0);
        let pop_one = function.create_block();
        function.ins().brif(empty, reject, &[], pop_one, &[]);
        function.switch_to_block(pop_one);
        let next_count = function.ins().iadd_imm(count, -1);
        let record_offset = function.ins().imul(next_count, stride_value);
        let record = function.ins().iadd(value_start, record_offset);
        let item = load_word(&mut function, workspace, record);
        let at_index = function.ins().iadd_imm(record, 1);
        let at = load_word(&mut function, workspace, at_index);
        function.ins().jump(
            copy_capture,
            &[zero.into(), next_count.into(), item.into(), at.into()],
        );

        function.switch_to_block(copy_capture);
        let copy_at = function.block_params(copy_capture)[0];
        let count = function.block_params(copy_capture)[1];
        let item = function.block_params(copy_capture)[2];
        let at = function.block_params(copy_capture)[3];
        let copied = function.ins().icmp_imm(
            IntCC::UnsignedGreaterThanOrEqual,
            copy_at,
            i64::from(capture_words),
        );
        let copy_one = function.create_block();
        function.ins().brif(
            copied,
            dispatch,
            &[count.into(), item.into(), at.into()],
            copy_one,
            &[],
        );
        function.switch_to_block(copy_one);
        let source_index = function.ins().iadd_imm(record, 2);
        let source_index = function.ins().iadd(source_index, copy_at);
        let value = load_word(&mut function, workspace, source_index);
        let target_index = function.ins().iadd(current, copy_at);
        store_word(&mut function, workspace, target_index, value);
        let next = function.ins().iadd_imm(copy_at, 1);
        function.ins().jump(
            copy_capture,
            &[next.into(), count.into(), item.into(), at.into()],
        );

        let instruction_blocks = (0..program.items.len())
            .map(|_| function.create_block())
            .collect::<Vec<_>>();
        function.switch_to_block(dispatch);
        let count = function.block_params(dispatch)[0];
        let item = function.block_params(dispatch)[1];
        let at = function.block_params(dispatch)[2];
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
        function.ins().brif(overflow, work_limit, &[], step_ok, &[]);
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
            mem::offset_of!(NativeTaskContext, work_limit) as i32,
        );
        let over_limit = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThan, steps, limit);
        let choose = function.create_block();
        function
            .ins()
            .brif(over_limit, work_limit, &[], choose, &[]);
        function.switch_to_block(choose);
        let finished = function.ins().icmp_imm(
            IntCC::UnsignedGreaterThanOrEqual,
            item,
            program.items.len() as i64,
        );
        let switch_block = function.create_block();
        if program.end_anchored {
            let end_match = function.ins().icmp(IntCC::Equal, at, source_end);
            let completed = function.ins().band(finished, end_match);
            let not_finished = function.create_block();
            function
                .ins()
                .brif(completed, success, &[], not_finished, &[]);
            function.switch_to_block(not_finished);
            function
                .ins()
                .brif(finished, reject_path, &[count.into()], switch_block, &[]);
        } else {
            function
                .ins()
                .brif(finished, success, &[], switch_block, &[]);
        }
        function.switch_to_block(switch_block);
        let mut switch = Switch::new();
        for (index, block) in instruction_blocks.iter().copied().enumerate() {
            switch.set_entry(index as u128, block);
        }
        switch.emit(&mut function, item, invalid);

        function.switch_to_block(reject_path);
        let rejected_count = function.block_params(reject_path)[0];
        function.ins().jump(pop, &[rejected_count.into()]);

        for (index, instruction) in program.items.iter().enumerate() {
            let block = instruction_blocks[index];
            function.switch_to_block(block);
            match instruction {
                LuaInstruction::Atom {
                    matcher: found,
                    repetition,
                } => {
                    let scan = function.create_block();
                    function.append_block_param(scan, types::I32);
                    let scanned = function.create_block();
                    function.append_block_param(scanned, types::I32);
                    function.ins().jump(scan, &[zero.into()]);
                    function.switch_to_block(scan);
                    let available = function.block_params(scan)[0];
                    let cursor = function.ins().iadd(at, available);
                    let at_end =
                        function
                            .ins()
                            .icmp(IntCC::UnsignedGreaterThanOrEqual, cursor, source_end);
                    let read = function.create_block();
                    function
                        .ins()
                        .brif(at_end, scanned, &[available.into()], read, &[]);
                    function.switch_to_block(read);
                    let cursor_offset = function.ins().uextend(pointer_type, cursor);
                    let address = function.ins().iadd(source, cursor_offset);
                    let byte = function
                        .ins()
                        .load(types::I8, MemFlags::trusted(), address, 0);
                    let matches = matcher(&mut function, found, byte);
                    let scan_next = function.create_block();
                    function
                        .ins()
                        .brif(matches, scan_next, &[], scanned, &[available.into()]);
                    function.switch_to_block(scan_next);
                    let next = function.ins().iadd_imm(available, 1);
                    function.ins().jump(scan, &[next.into()]);
                    function.switch_to_block(scanned);
                    let available = function.block_params(scanned)[0];
                    let (minimum, greedy) = match repetition {
                        0 => (1_u32, true),
                        1 => (0, true),
                        2 => (0, true),
                        3 => (0, false),
                        4 => (1, true),
                        _ => {
                            function.ins().jump(invalid, &[]);
                            continue;
                        }
                    };
                    let maximum_count = match repetition {
                        0 => function.ins().iconst(types::I32, 1),
                        1 => {
                            let one = function.ins().iconst(types::I32, 1);
                            function.ins().umin(available, one)
                        }
                        _ => available,
                    };
                    let enough = function.ins().icmp_imm(
                        IntCC::UnsignedGreaterThanOrEqual,
                        available,
                        i64::from(minimum),
                    );
                    let push_setup = function.create_block();
                    function
                        .ins()
                        .brif(enough, push_setup, &[], pop, &[count.into()]);
                    function.switch_to_block(push_setup);
                    let push_loop = function.create_block();
                    function.append_block_param(push_loop, types::I32);
                    function.append_block_param(push_loop, types::I32);
                    let first_used = if greedy {
                        function.ins().iconst(types::I32, i64::from(minimum))
                    } else {
                        maximum_count
                    };
                    function
                        .ins()
                        .jump(push_loop, &[first_used.into(), count.into()]);
                    function.switch_to_block(push_loop);
                    let used = function.block_params(push_loop)[0];
                    let pushed = function.block_params(push_loop)[1];
                    let done = function
                        .ins()
                        .icmp(IntCC::UnsignedGreaterThan, used, maximum_count);
                    let push_one = function.create_block();
                    function
                        .ins()
                        .brif(done, pop, &[pushed.into()], push_one, &[]);
                    function.switch_to_block(push_one);
                    let full =
                        function
                            .ins()
                            .icmp(IntCC::UnsignedGreaterThanOrEqual, pushed, maximum);
                    let store = function.create_block();
                    function.ins().brif(full, capacity, &[], store, &[]);
                    function.switch_to_block(store);
                    let offset = function.ins().imul(pushed, stride_value);
                    let target = function.ins().iadd(value_start, offset);
                    let next_item = function.ins().iconst(types::I32, index as i64 + 1);
                    store_word(&mut function, workspace, target, next_item);
                    let target_at = function.ins().iadd_imm(target, 1);
                    let next_at = function.ins().iadd(at, used);
                    store_word(&mut function, workspace, target_at, next_at);
                    for capture_offset in 0..capture_words {
                        let from = function.ins().iadd_imm(current, i64::from(capture_offset));
                        let value = load_word(&mut function, workspace, from);
                        let to = function
                            .ins()
                            .iadd_imm(target, i64::from(2 + capture_offset));
                        store_word(&mut function, workspace, to, value);
                    }
                    let next_used = if greedy {
                        function.ins().iadd_imm(used, 1)
                    } else {
                        function.ins().iadd_imm(used, -1)
                    };
                    let next_count = function.ins().iadd_imm(pushed, 1);
                    function
                        .ins()
                        .jump(push_loop, &[next_used.into(), next_count.into()]);
                }
                LuaInstruction::Frontier(found) => {
                    let previous_zero = function.ins().iconst(types::I8, 0);
                    let has_previous =
                        function
                            .ins()
                            .icmp(IntCC::UnsignedGreaterThan, at, source_start);
                    let previous_read = function.create_block();
                    let previous_done = function.create_block();
                    function.append_block_param(previous_done, types::I8);
                    function.ins().brif(
                        has_previous,
                        previous_read,
                        &[],
                        previous_done,
                        &[previous_zero.into()],
                    );
                    function.switch_to_block(previous_read);
                    let previous_at = function.ins().iadd_imm(at, -1);
                    let previous_offset = function.ins().uextend(pointer_type, previous_at);
                    let address = function.ins().iadd(source, previous_offset);
                    let previous = function
                        .ins()
                        .load(types::I8, MemFlags::trusted(), address, 0);
                    function.ins().jump(previous_done, &[previous.into()]);
                    function.switch_to_block(previous_done);
                    let previous = function.block_params(previous_done)[0];
                    let current_read = function.create_block();
                    let current_done = function.create_block();
                    function.append_block_param(current_done, types::I8);
                    let has_current = function.ins().icmp(IntCC::UnsignedLessThan, at, source_end);
                    function.ins().brif(
                        has_current,
                        current_read,
                        &[],
                        current_done,
                        &[previous_zero.into()],
                    );
                    function.switch_to_block(current_read);
                    let offset = function.ins().uextend(pointer_type, at);
                    let address = function.ins().iadd(source, offset);
                    let current_byte =
                        function
                            .ins()
                            .load(types::I8, MemFlags::trusted(), address, 0);
                    function.ins().jump(current_done, &[current_byte.into()]);
                    function.switch_to_block(current_done);
                    let current_byte = function.block_params(current_done)[0];
                    let previous_matches = matcher(&mut function, found, previous);
                    let current_matches = matcher(&mut function, found, current_byte);
                    let previous_outside = bool_not(&mut function, previous_matches);
                    let frontier = function.ins().band(previous_outside, current_matches);
                    let next_item = function.ins().iconst(types::I32, index as i64 + 1);
                    function.ins().brif(
                        frontier,
                        dispatch,
                        &[count.into(), next_item.into(), at.into()],
                        pop,
                        &[count.into()],
                    );
                }
                LuaInstruction::Balanced { open, close } => {
                    let has_byte = function.ins().icmp(IntCC::UnsignedLessThan, at, source_end);
                    let read = function.create_block();
                    function
                        .ins()
                        .brif(has_byte, read, &[], pop, &[count.into()]);
                    function.switch_to_block(read);
                    let offset = function.ins().uextend(pointer_type, at);
                    let address = function.ins().iadd(source, offset);
                    let byte = function
                        .ins()
                        .load(types::I8, MemFlags::trusted(), address, 0);
                    let opens = function
                        .ins()
                        .icmp_imm(IntCC::Equal, byte, i64::from(*open));
                    let balance_start = function.create_block();
                    function
                        .ins()
                        .brif(opens, balance_start, &[], pop, &[count.into()]);
                    function.switch_to_block(balance_start);
                    let balance = function.create_block();
                    function.append_block_param(balance, types::I32);
                    function.append_block_param(balance, types::I32);
                    let one = function.ins().iconst(types::I32, 1);
                    let end = function.ins().iadd_imm(at, 1);
                    function.ins().jump(balance, &[one.into(), end.into()]);
                    function.switch_to_block(balance);
                    let depth = function.block_params(balance)[0];
                    let end = function.block_params(balance)[1];
                    let exhausted =
                        function
                            .ins()
                            .icmp(IntCC::UnsignedGreaterThanOrEqual, end, source_end);
                    let balance_read = function.create_block();
                    function
                        .ins()
                        .brif(exhausted, pop, &[count.into()], balance_read, &[]);
                    function.switch_to_block(balance_read);
                    let steps = function.ins().load(
                        types::I32,
                        MemFlags::trusted(),
                        task,
                        mem::offset_of!(NativeTaskContext, current_steps) as i32,
                    );
                    let overflow = function
                        .ins()
                        .icmp_imm(IntCC::Equal, steps, u32::MAX as i64);
                    let balance_step = function.create_block();
                    function
                        .ins()
                        .brif(overflow, work_limit, &[], balance_step, &[]);
                    function.switch_to_block(balance_step);
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
                        mem::offset_of!(NativeTaskContext, work_limit) as i32,
                    );
                    let over = function
                        .ins()
                        .icmp(IntCC::UnsignedGreaterThan, steps, limit);
                    let balance_byte = function.create_block();
                    function
                        .ins()
                        .brif(over, work_limit, &[], balance_byte, &[]);
                    function.switch_to_block(balance_byte);
                    let offset = function.ins().uextend(pointer_type, end);
                    let address = function.ins().iadd(source, offset);
                    let byte = function
                        .ins()
                        .load(types::I8, MemFlags::trusted(), address, 0);
                    let is_open = function
                        .ins()
                        .icmp_imm(IntCC::Equal, byte, i64::from(*open));
                    let open_block = function.create_block();
                    let not_open = function.create_block();
                    function.ins().brif(is_open, open_block, &[], not_open, &[]);
                    function.switch_to_block(open_block);
                    let next_depth = function.ins().iadd_imm(depth, 1);
                    let next_end = function.ins().iadd_imm(end, 1);
                    function
                        .ins()
                        .jump(balance, &[next_depth.into(), next_end.into()]);
                    function.switch_to_block(not_open);
                    let is_close = function
                        .ins()
                        .icmp_imm(IntCC::Equal, byte, i64::from(*close));
                    let close_block = function.create_block();
                    let other_block = function.create_block();
                    function
                        .ins()
                        .brif(is_close, close_block, &[], other_block, &[]);
                    function.switch_to_block(other_block);
                    let next_end = function.ins().iadd_imm(end, 1);
                    function
                        .ins()
                        .jump(balance, &[depth.into(), next_end.into()]);
                    function.switch_to_block(close_block);
                    let next_depth = function.ins().iadd_imm(depth, -1);
                    let closed = function.ins().icmp_imm(IntCC::Equal, next_depth, 0);
                    let continue_balance = function.create_block();
                    let next_item = function.ins().iconst(types::I32, index as i64 + 1);
                    let after = function.ins().iadd_imm(end, 1);
                    function.ins().brif(
                        closed,
                        dispatch,
                        &[count.into(), next_item.into(), after.into()],
                        continue_balance,
                        &[],
                    );
                    function.switch_to_block(continue_balance);
                    function
                        .ins()
                        .jump(balance, &[next_depth.into(), after.into()]);
                }
                LuaInstruction::CaptureStart(slot)
                | LuaInstruction::CaptureEnd(slot)
                | LuaInstruction::PositionCapture(slot)
                | LuaInstruction::Backreference(slot) => {
                    if *slot >= program.capture_count {
                        function.ins().jump(invalid, &[]);
                        continue;
                    }
                    let capture = function.ins().iadd_imm(current, i64::from(*slot * 3));
                    let status = load_word(&mut function, workspace, capture);
                    let next_item = function.ins().iconst(types::I32, index as i64 + 1);
                    match instruction {
                        LuaInstruction::CaptureStart(_) => {
                            let open = function.ins().iconst(types::I32, 1);
                            store_word(&mut function, workspace, capture, open);
                            let start = function.ins().iadd_imm(capture, 1);
                            store_word(&mut function, workspace, start, at);
                            function
                                .ins()
                                .jump(dispatch, &[count.into(), next_item.into(), at.into()]);
                        }
                        LuaInstruction::CaptureEnd(_) => {
                            let is_open = function.ins().icmp_imm(IntCC::Equal, status, 1);
                            let close = function.create_block();
                            function
                                .ins()
                                .brif(is_open, close, &[], pop, &[count.into()]);
                            function.switch_to_block(close);
                            let closed = function.ins().iconst(types::I32, 2);
                            store_word(&mut function, workspace, capture, closed);
                            let end = function.ins().iadd_imm(capture, 2);
                            store_word(&mut function, workspace, end, at);
                            function
                                .ins()
                                .jump(dispatch, &[count.into(), next_item.into(), at.into()]);
                        }
                        LuaInstruction::PositionCapture(_) => {
                            let position = function.ins().iconst(types::I32, 3);
                            store_word(&mut function, workspace, capture, position);
                            let start = function.ins().iadd_imm(capture, 1);
                            store_word(&mut function, workspace, start, at);
                            function
                                .ins()
                                .jump(dispatch, &[count.into(), next_item.into(), at.into()]);
                        }
                        LuaInstruction::Backreference(_) => {
                            let is_closed = function.ins().icmp_imm(IntCC::Equal, status, 2);
                            let compare = function.create_block();
                            function
                                .ins()
                                .brif(is_closed, compare, &[], pop, &[count.into()]);
                            function.switch_to_block(compare);
                            let start_index = function.ins().iadd_imm(capture, 1);
                            let end_index = function.ins().iadd_imm(capture, 2);
                            let start = load_word(&mut function, workspace, start_index);
                            let end = load_word(&mut function, workspace, end_index);
                            let ordered =
                                function
                                    .ins()
                                    .icmp(IntCC::UnsignedGreaterThanOrEqual, end, start);
                            let length = function.ins().isub(end, start);
                            let target_end = function.ins().iadd(at, length);
                            let no_overflow = function.ins().icmp(
                                IntCC::UnsignedGreaterThanOrEqual,
                                target_end,
                                at,
                            );
                            let in_range = function.ins().icmp(
                                IntCC::UnsignedLessThanOrEqual,
                                target_end,
                                source_end,
                            );
                            let valid = function.ins().band(ordered, no_overflow);
                            let valid = function.ins().band(valid, in_range);
                            let compare_loop = function.create_block();
                            function.append_block_param(compare_loop, types::I32);
                            function.ins().brif(
                                valid,
                                compare_loop,
                                &[zero.into()],
                                pop,
                                &[count.into()],
                            );
                            function.switch_to_block(compare_loop);
                            let offset = function.block_params(compare_loop)[0];
                            let done = function.ins().icmp(
                                IntCC::UnsignedGreaterThanOrEqual,
                                offset,
                                length,
                            );
                            let compare_byte = function.create_block();
                            function.ins().brif(
                                done,
                                dispatch,
                                &[count.into(), next_item.into(), target_end.into()],
                                compare_byte,
                                &[],
                            );
                            function.switch_to_block(compare_byte);
                            let left_at = function.ins().iadd(start, offset);
                            let right_at = function.ins().iadd(at, offset);
                            let left_offset = function.ins().uextend(pointer_type, left_at);
                            let right_offset = function.ins().uextend(pointer_type, right_at);
                            let left_address = function.ins().iadd(source, left_offset);
                            let right_address = function.ins().iadd(source, right_offset);
                            let left = function.ins().load(
                                types::I8,
                                MemFlags::trusted(),
                                left_address,
                                0,
                            );
                            let right = function.ins().load(
                                types::I8,
                                MemFlags::trusted(),
                                right_address,
                                0,
                            );
                            let equal = function.ins().icmp(IntCC::Equal, left, right);
                            let compare_next = function.create_block();
                            function
                                .ins()
                                .brif(equal, compare_next, &[], pop, &[count.into()]);
                            function.switch_to_block(compare_next);
                            let next = function.ins().iadd_imm(offset, 1);
                            function.ins().jump(compare_loop, &[next.into()]);
                        }
                        _ => unreachable!(),
                    }
                }
            }
        }

        return_code(&mut function, success, 1);
        return_code(&mut function, reject, 0);
        return_code(&mut function, invalid, 2);
        return_code(&mut function, work_limit, 3);
        return_code(&mut function, capacity, 4);
        function.seal_all_blocks();
        function.finalize();
    }
    if let Err(error) = module.define_function(id, &mut context) {
        #[cfg(test)]
        eprintln!("{}", context.func.display());
        return Err(Box::new(error));
    }
    module.clear_context(&mut context);
    Ok(())
}
