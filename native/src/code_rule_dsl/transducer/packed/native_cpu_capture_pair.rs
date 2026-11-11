use super::{
    metal_batch_abi::{LinkRecord, MetalNode},
    native_cpu::NativeTaskContext,
    value::CaptureProgram,
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

fn link_pointer(
    function: &mut FunctionBuilder<'_>,
    task: cranelift_codegen::ir::Value,
    head: cranelift_codegen::ir::Value,
) -> cranelift_codegen::ir::Value {
    let ty = function.func.dfg.value_type(task);
    let captures = pointer(function, task, mem::offset_of!(NativeTaskContext, captures));
    let index = function.ins().iadd_imm(head, -1);
    let offset = function
        .ins()
        .imul_imm(index, mem::size_of::<LinkRecord>() as i64);
    let offset = function.ins().uextend(ty, offset);
    function.ins().iadd(captures, offset)
}

fn node_pointer(
    function: &mut FunctionBuilder<'_>,
    task: cranelift_codegen::ir::Value,
    node: cranelift_codegen::ir::Value,
) -> cranelift_codegen::ir::Value {
    let ty = function.func.dfg.value_type(task);
    let nodes = pointer(function, task, mem::offset_of!(NativeTaskContext, nodes));
    let offset = function
        .ins()
        .imul_imm(node, mem::size_of::<MetalNode>() as i64);
    let offset = function.ins().uextend(ty, offset);
    function.ins().iadd(nodes, offset)
}

fn label_count(
    function: &mut FunctionBuilder<'_>,
    task: cranelift_codegen::ir::Value,
    label: u16,
) -> cranelift_codegen::ir::Value {
    let head = function.ins().load(
        types::I32,
        MemFlags::trusted(),
        task,
        mem::offset_of!(NativeTaskContext, current_captures) as i32,
    );
    let walk = function.create_block();
    function.append_block_param(walk, types::I32);
    function.append_block_param(walk, types::I32);
    let inspect = function.create_block();
    function.append_block_param(inspect, types::I32);
    function.append_block_param(inspect, types::I32);
    let finish = function.create_block();
    function.append_block_param(finish, types::I32);
    let zero = function.ins().iconst(types::I32, 0);
    function.ins().jump(walk, &[head.into(), zero.into()]);

    function.switch_to_block(walk);
    let values = function.block_params(walk).to_vec();
    let done = function.ins().icmp_imm(IntCC::Equal, values[0], 0);
    function.ins().brif(
        done,
        finish,
        &[values[1].into()],
        inspect,
        &[values[0].into(), values[1].into()],
    );

    function.switch_to_block(inspect);
    let values = function.block_params(inspect).to_vec();
    let link = link_pointer(function, task, values[0]);
    let previous = function.ins().load(
        types::I32,
        MemFlags::trusted(),
        link,
        mem::offset_of!(LinkRecord, previous) as i32,
    );
    let actual = function.ins().load(
        types::I32,
        MemFlags::trusted(),
        link,
        mem::offset_of!(LinkRecord, first) as i32,
    );
    let found = function
        .ins()
        .icmp_imm(IntCC::Equal, actual, i64::from(label));
    let found = function.ins().uextend(types::I32, found);
    let count = function.ins().iadd(values[1], found);
    function.ins().jump(walk, &[previous.into(), count.into()]);

    function.switch_to_block(finish);
    function.block_params(finish)[0]
}

fn label_node(
    function: &mut FunctionBuilder<'_>,
    task: cranelift_codegen::ir::Value,
    label: u16,
    count: cranelift_codegen::ir::Value,
    ordinal: cranelift_codegen::ir::Value,
) -> cranelift_codegen::ir::Value {
    let head = function.ins().load(
        types::I32,
        MemFlags::trusted(),
        task,
        mem::offset_of!(NativeTaskContext, current_captures) as i32,
    );
    let wanted = function.ins().iadd_imm(count, -1);
    let wanted = function.ins().isub(wanted, ordinal);
    let walk = function.create_block();
    function.append_block_param(walk, types::I32);
    function.append_block_param(walk, types::I32);
    let inspect = function.create_block();
    function.append_block_param(inspect, types::I32);
    function.append_block_param(inspect, types::I32);
    let advance = function.create_block();
    function.append_block_param(advance, types::I32);
    function.append_block_param(advance, types::I32);
    let finish = function.create_block();
    function.append_block_param(finish, types::I32);
    let zero = function.ins().iconst(types::I32, 0);
    function.ins().jump(walk, &[head.into(), zero.into()]);

    function.switch_to_block(walk);
    let values = function.block_params(walk).to_vec();
    let exists = function.ins().icmp_imm(IntCC::NotEqual, values[0], 0);
    let invalid = function.ins().iconst(types::I32, u32::MAX as i64);
    function.ins().brif(
        exists,
        inspect,
        &[values[0].into(), values[1].into()],
        finish,
        &[invalid.into()],
    );

    function.switch_to_block(inspect);
    let values = function.block_params(inspect).to_vec();
    let link = link_pointer(function, task, values[0]);
    let previous = function.ins().load(
        types::I32,
        MemFlags::trusted(),
        link,
        mem::offset_of!(LinkRecord, previous) as i32,
    );
    let actual = function.ins().load(
        types::I32,
        MemFlags::trusted(),
        link,
        mem::offset_of!(LinkRecord, first) as i32,
    );
    let right_label = function
        .ins()
        .icmp_imm(IntCC::Equal, actual, i64::from(label));
    let right_ordinal = function.ins().icmp(IntCC::Equal, values[1], wanted);
    let selected = function.ins().band(right_label, right_ordinal);
    let node = function.ins().load(
        types::I32,
        MemFlags::trusted(),
        link,
        mem::offset_of!(LinkRecord, second) as i32,
    );
    function.ins().brif(
        selected,
        finish,
        &[node.into()],
        advance,
        &[previous.into(), values[1].into()],
    );

    function.switch_to_block(advance);
    let values = function.block_params(advance).to_vec();
    let increment = function.ins().uextend(types::I32, right_label);
    let index = function.ins().iadd(values[1], increment);
    function.ins().jump(walk, &[values[0].into(), index.into()]);

    function.switch_to_block(finish);
    function.block_params(finish)[0]
}

fn text_equal(
    function: &mut FunctionBuilder<'_>,
    task: cranelift_codegen::ir::Value,
    left: cranelift_codegen::ir::Value,
    right: cranelift_codegen::ir::Value,
) -> cranelift_codegen::ir::Value {
    let ty = function.func.dfg.value_type(task);
    let left_node = node_pointer(function, task, left);
    let right_node = node_pointer(function, task, right);
    let left_start = function.ins().load(
        types::I32,
        MemFlags::trusted(),
        left_node,
        mem::offset_of!(MetalNode, source_start) as i32,
    );
    let left_end = function.ins().load(
        types::I32,
        MemFlags::trusted(),
        left_node,
        mem::offset_of!(MetalNode, source_end) as i32,
    );
    let right_start = function.ins().load(
        types::I32,
        MemFlags::trusted(),
        right_node,
        mem::offset_of!(MetalNode, source_start) as i32,
    );
    let right_end = function.ins().load(
        types::I32,
        MemFlags::trusted(),
        right_node,
        mem::offset_of!(MetalNode, source_end) as i32,
    );
    let left_length = function.ins().isub(left_end, left_start);
    let right_length = function.ins().isub(right_end, right_start);
    let loop_block = function.create_block();
    function.append_block_param(loop_block, types::I32);
    let compare = function.create_block();
    function.append_block_param(compare, types::I32);
    let accept = function.create_block();
    let reject = function.create_block();
    let finish = function.create_block();
    function.append_block_param(finish, types::I8);
    let same_length = function.ins().icmp(IntCC::Equal, left_length, right_length);
    let zero = function.ins().iconst(types::I32, 0);
    function
        .ins()
        .brif(same_length, loop_block, &[zero.into()], reject, &[]);
    function.switch_to_block(loop_block);
    let index = function.block_params(loop_block)[0];
    let done = function
        .ins()
        .icmp(IntCC::UnsignedGreaterThanOrEqual, index, left_length);
    function
        .ins()
        .brif(done, accept, &[], compare, &[index.into()]);
    function.switch_to_block(compare);
    let index = function.block_params(compare)[0];
    let source = pointer(function, task, mem::offset_of!(NativeTaskContext, source));
    let left_at = function.ins().iadd(left_start, index);
    let right_at = function.ins().iadd(right_start, index);
    let left_at = function.ins().uextend(ty, left_at);
    let right_at = function.ins().uextend(ty, right_at);
    let left_source = function.ins().iadd(source, left_at);
    let right_source = function.ins().iadd(source, right_at);
    let left_byte = function
        .ins()
        .load(types::I8, MemFlags::trusted(), left_source, 0);
    let right_byte = function
        .ins()
        .load(types::I8, MemFlags::trusted(), right_source, 0);
    let equal = function.ins().icmp(IntCC::Equal, left_byte, right_byte);
    let next = function.ins().iadd_imm(index, 1);
    function
        .ins()
        .brif(equal, loop_block, &[next.into()], reject, &[]);
    function.switch_to_block(accept);
    let one = function.ins().iconst(types::I8, 1);
    function.ins().jump(finish, &[one.into()]);
    function.switch_to_block(reject);
    let zero = function.ins().iconst(types::I8, 0);
    function.ins().jump(finish, &[zero.into()]);
    function.switch_to_block(finish);
    function.block_params(finish)[0]
}

fn same_line(
    function: &mut FunctionBuilder<'_>,
    task: cranelift_codegen::ir::Value,
    left: cranelift_codegen::ir::Value,
    right: cranelift_codegen::ir::Value,
) -> cranelift_codegen::ir::Value {
    let ty = function.func.dfg.value_type(task);
    let left_node = node_pointer(function, task, left);
    let right_node = node_pointer(function, task, right);
    let left_start = function.ins().load(
        types::I32,
        MemFlags::trusted(),
        left_node,
        mem::offset_of!(MetalNode, source_start) as i32,
    );
    let right_start = function.ins().load(
        types::I32,
        MemFlags::trusted(),
        right_node,
        mem::offset_of!(MetalNode, source_start) as i32,
    );
    let left_first = function
        .ins()
        .icmp(IntCC::UnsignedLessThan, left_start, right_start);
    let start = function.ins().select(left_first, left_start, right_start);
    let end = function.ins().select(left_first, right_start, left_start);
    let loop_block = function.create_block();
    function.append_block_param(loop_block, types::I32);
    let inspect = function.create_block();
    function.append_block_param(inspect, types::I32);
    let accept = function.create_block();
    let reject = function.create_block();
    let finish = function.create_block();
    function.append_block_param(finish, types::I8);
    function.ins().jump(loop_block, &[start.into()]);
    function.switch_to_block(loop_block);
    let at = function.block_params(loop_block)[0];
    let done = function
        .ins()
        .icmp(IntCC::UnsignedGreaterThanOrEqual, at, end);
    function
        .ins()
        .brif(done, accept, &[], inspect, &[at.into()]);
    function.switch_to_block(inspect);
    let at = function.block_params(inspect)[0];
    let source = pointer(function, task, mem::offset_of!(NativeTaskContext, source));
    let offset = function.ins().uextend(ty, at);
    let source_at = function.ins().iadd(source, offset);
    let byte = function
        .ins()
        .load(types::I8, MemFlags::trusted(), source_at, 0);
    let newline = function.ins().icmp_imm(IntCC::Equal, byte, b'\n' as i64);
    let next = function.ins().iadd_imm(at, 1);
    function
        .ins()
        .brif(newline, reject, &[], loop_block, &[next.into()]);
    function.switch_to_block(accept);
    let one = function.ins().iconst(types::I8, 1);
    function.ins().jump(finish, &[one.into()]);
    function.switch_to_block(reject);
    let zero = function.ins().iconst(types::I8, 0);
    function.ins().jump(finish, &[zero.into()]);
    function.switch_to_block(finish);
    function.block_params(finish)[0]
}

pub(super) fn define(
    module: &mut JITModule,
    index: usize,
    program: &CaptureProgram,
) -> Result<FuncId, Box<cranelift_module::ModuleError>> {
    let (left, right, negated, any, compare_text) = match program {
        CaptureProgram::Equal {
            left,
            right,
            negated,
            any,
        } => (*left, *right, *negated, *any, true),
        CaptureProgram::SameLine {
            left,
            right,
            negated,
        } => (*left, *right, *negated, false, false),
        _ => unreachable!("only paired capture programs use paired lowering"),
    };
    let mut signature = module.make_signature();
    let pointer_type = module.target_config().pointer_type();
    signature.params.push(AbiParam::new(pointer_type));
    signature.returns.push(AbiParam::new(types::I32));
    let id = module.declare_function(
        &format!("tempo_capture_{index}"),
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
        function.switch_to_block(entry);
        let task = function.block_params(entry)[0];
        let left_count = label_count(&mut function, task, left);
        let right_count = label_count(&mut function, task, right);
        let left_smaller = function
            .ins()
            .icmp(IntCC::UnsignedLessThan, left_count, right_count);
        let count = function.ins().select(left_smaller, left_count, right_count);
        let loop_block = function.create_block();
        function.append_block_param(loop_block, types::I32);
        function.append_block_param(loop_block, types::I8);
        function.append_block_param(loop_block, types::I8);
        let inspect = function.create_block();
        function.append_block_param(inspect, types::I32);
        function.append_block_param(inspect, types::I8);
        function.append_block_param(inspect, types::I8);
        let finish = function.create_block();
        function.append_block_param(finish, types::I8);
        function.append_block_param(finish, types::I8);
        let zero = function.ins().iconst(types::I32, 0);
        let initial = function.ins().iconst(types::I8, i64::from(!any));
        function
            .ins()
            .jump(loop_block, &[zero.into(), initial.into(), initial.into()]);
        function.switch_to_block(loop_block);
        let values = function.block_params(loop_block).to_vec();
        let done = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, values[0], count);
        function.ins().brif(
            done,
            finish,
            &[values[1].into(), values[2].into()],
            inspect,
            &[values[0].into(), values[1].into(), values[2].into()],
        );
        function.switch_to_block(inspect);
        let values = function.block_params(inspect).to_vec();
        let left_node = label_node(&mut function, task, left, left_count, values[0]);
        let right_node = label_node(&mut function, task, right, right_count, values[0]);
        let result = if compare_text {
            text_equal(&mut function, task, left_node, right_node)
        } else {
            same_line(&mut function, task, left_node, right_node)
        };
        let inverse = function.ins().bxor_imm(result, 1);
        let positive = if any {
            function.ins().bor(values[1], result)
        } else {
            function.ins().band(values[1], result)
        };
        let negative = if any {
            function.ins().bor(values[2], inverse)
        } else {
            function.ins().band(values[2], inverse)
        };
        let next = function.ins().iadd_imm(values[0], 1);
        function
            .ins()
            .jump(loop_block, &[next.into(), positive.into(), negative.into()]);
        function.switch_to_block(finish);
        let values = function.block_params(finish).to_vec();
        let empty = function.ins().icmp_imm(IntCC::Equal, count, 0);
        let selected = if negated { values[1] } else { values[0] };
        let zero = function.ins().iconst(types::I8, 0);
        let selected = function.ins().select(empty, zero, selected);
        let selected = function.ins().uextend(types::I32, selected);
        function.ins().return_(&[selected]);
        function.seal_all_blocks();
        function.finalize();
    }
    super::native_cpu_codegen::define_function(module, id, &mut context)?;
    module.clear_context(&mut context);
    Ok(id)
}
