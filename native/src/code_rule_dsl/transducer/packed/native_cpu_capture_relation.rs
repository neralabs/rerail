//! Ordered capture containment for the independent CPU JIT.
//! Capture links are immutable insertion histories. Geometry ties retain insertion order.
use super::{context_pointer, declare, pointer_at};
use crate::policy_dsl::transducer::{
    metal_batch_abi::{LinkRecord, MetalNode},
    native_cpu::NativeTaskContext,
};
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlags, Value, condcodes::IntCC, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Module};
use std::mem;

fn link(function: &mut FunctionBuilder<'_>, task: Value, head: Value) -> [Value; 3] {
    let links = context_pointer(function, task, mem::offset_of!(NativeTaskContext, captures));
    let index = function.ins().iadd_imm(head, -1);
    let record = pointer_at(function, links, index, mem::size_of::<LinkRecord>());
    [
        mem::offset_of!(LinkRecord, previous),
        mem::offset_of!(LinkRecord, first),
        mem::offset_of!(LinkRecord, second),
    ]
    .map(|offset| {
        function
            .ins()
            .load(types::I32, MemFlags::trusted(), record, offset as i32)
    })
}

fn geometry(function: &mut FunctionBuilder<'_>, task: Value, node: Value) -> [Value; 2] {
    let nodes = context_pointer(function, task, mem::offset_of!(NativeTaskContext, nodes));
    let record = pointer_at(function, nodes, node, mem::size_of::<MetalNode>());
    [
        mem::offset_of!(MetalNode, source_start),
        mem::offset_of!(MetalNode, source_end),
    ]
    .map(|offset| {
        function
            .ins()
            .load(types::I32, MemFlags::trusted(), record, offset as i32)
    })
}

fn signature(module: &JITModule, parameters: usize) -> cranelift_codegen::ir::Signature {
    let mut signature = module.make_signature();
    signature
        .params
        .push(AbiParam::new(module.target_config().pointer_type()));
    for _ in 1..parameters {
        signature.params.push(AbiParam::new(types::I32));
    }
    signature.returns.push(AbiParam::new(types::I32));
    signature
}

// Generate either a geometry count or a reverse-position lookup. These are
// distinct callable functions with distinct return meanings.
fn define_geometry_walk(
    module: &mut JITModule,
    id: FuncId,
    lookup: bool,
) -> Result<(), Box<cranelift_module::ModuleError>> {
    let mut context = module.make_context();
    context.func.signature = signature(module, if lookup { 4 } else { 3 });
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut f = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = f.create_block();
        f.append_block_params_for_function_params(entry);
        let walk = f.create_block();
        f.append_block_param(walk, types::I32);
        f.append_block_param(walk, types::I32);
        let inspect = f.create_block();
        let advance = f.create_block();
        let done = f.create_block();
        f.append_block_param(done, types::I32);
        f.switch_to_block(entry);
        let task = f.block_params(entry)[0];
        let head = f.block_params(entry)[1];
        let node = f.block_params(entry)[2];
        let expected = geometry(&mut f, task, node);
        let zero = f.ins().iconst(types::I32, 0);
        f.ins().jump(walk, &[head.into(), zero.into()]);
        f.switch_to_block(walk);
        let head = f.block_params(walk)[0];
        let count = f.block_params(walk)[1];
        let present = f.ins().icmp_imm(IntCC::NotEqual, head, 0);
        let exhausted = if lookup { zero } else { count };
        f.ins()
            .brif(present, inspect, &[], done, &[exhausted.into()]);
        f.switch_to_block(inspect);
        let [previous, _, node] = link(&mut f, task, head);
        let actual = geometry(&mut f, task, node);
        let start_same = f.ins().icmp(IntCC::Equal, actual[0], expected[0]);
        let end_same = f.ins().icmp(IntCC::Equal, actual[1], expected[1]);
        let same = f.ins().band(start_same, end_same);
        if lookup {
            let target = f.block_params(entry)[3];
            let position = f.ins().icmp(IntCC::Equal, count, target);
            let found = f.ins().band(same, position);
            f.ins().brif(found, done, &[head.into()], advance, &[]);
        } else {
            f.ins().jump(advance, &[]);
        }
        f.switch_to_block(advance);
        let incremented = f.ins().iadd_imm(count, 1);
        let count = f.ins().select(same, incremented, count);
        f.ins().jump(walk, &[previous.into(), count.into()]);
        f.switch_to_block(done);
        let value = f.block_params(done)[0];
        f.ins().return_(&[value]);
        f.seal_all_blocks();
        f.finalize();
    }
    crate::policy_dsl::transducer::native_cpu_codegen::define_function(module, id, &mut context)?;
    module.clear_context(&mut context);
    Ok(())
}

pub(super) fn define(
    module: &mut JITModule,
    id: FuncId,
) -> Result<(), Box<cranelift_module::ModuleError>> {
    let count = declare(module, "tempo_capture_geometry_count", 3)?;
    let at = declare(module, "tempo_capture_geometry_at", 4)?;
    define_geometry_walk(module, count, false)?;
    define_geometry_walk(module, at, true)?;
    let mut context = module.make_context();
    context.func.signature = signature(module, 3);
    let count = module.declare_func_in_func(count, &mut context.func);
    let at = module.declare_func_in_func(at, &mut context.func);
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut f = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = f.create_block();
        f.append_block_params_for_function_params(entry);
        let walk = f.create_block();
        f.append_block_param(walk, types::I32);
        let inspect = f.create_block();
        let locate = f.create_block();
        let compare = f.create_block();
        let accept = f.create_block();
        let reject = f.create_block();
        f.switch_to_block(entry);
        let task = f.block_params(entry)[0];
        let left = f.block_params(entry)[1];
        let right = f.block_params(entry)[2];
        f.ins().jump(walk, &[right.into()]);
        f.switch_to_block(walk);
        let head = f.block_params(walk)[0];
        let present = f.ins().icmp_imm(IntCC::NotEqual, head, 0);
        f.ins().brif(present, inspect, &[], accept, &[]);
        f.switch_to_block(inspect);
        let [previous, label, node] = link(&mut f, task, head);
        // Rank from the beginning of this geometry group, not the end. A
        // longer containing group must preserve the shorter group's prefix.
        let call = f.ins().call(count, &[task, previous, node]);
        let rank = f.inst_results(call)[0];
        let call = f.ins().call(count, &[task, left, node]);
        let total = f.inst_results(call)[0];
        let available = f.ins().icmp(IntCC::UnsignedGreaterThan, total, rank);
        f.ins().brif(available, locate, &[], reject, &[]);
        f.switch_to_block(locate);
        let reversed = f.ins().isub(total, rank);
        let reversed = f.ins().iadd_imm(reversed, -1);
        let call = f.ins().call(at, &[task, left, node, reversed]);
        let target = f.inst_results(call)[0];
        let found = f.ins().icmp_imm(IntCC::NotEqual, target, 0);
        f.ins().brif(found, compare, &[], reject, &[]);
        f.switch_to_block(compare);
        let [_, actual_label, actual_node] = link(&mut f, task, target);
        let label_same = f.ins().icmp(IntCC::Equal, actual_label, label);
        let node_same = f.ins().icmp(IntCC::Equal, actual_node, node);
        let same = f.ins().band(label_same, node_same);
        f.ins().brif(same, walk, &[previous.into()], reject, &[]);
        f.switch_to_block(accept);
        let one = f.ins().iconst(types::I32, 1);
        f.ins().return_(&[one]);
        f.switch_to_block(reject);
        let zero = f.ins().iconst(types::I32, 0);
        f.ins().return_(&[zero]);
        f.seal_all_blocks();
        f.finalize();
    }
    crate::policy_dsl::transducer::native_cpu_codegen::define_function(module, id, &mut context)?;
    module.clear_context(&mut context);
    Ok(())
}
