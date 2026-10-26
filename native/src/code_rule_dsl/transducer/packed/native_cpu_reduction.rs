use super::{
    metal_batch_abi::{NONE, ResultRecord, TaskStatus},
    native_cpu::NativeTaskContext,
};
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlags, condcodes::IntCC, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Linkage, Module};
use std::mem;

const FIELDS: [usize; 10] = [
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

fn result(
    function: &mut FunctionBuilder<'_>,
    task: cranelift_codegen::ir::Value,
    index: cranelift_codegen::ir::Value,
) -> cranelift_codegen::ir::Value {
    let ty = function.func.dfg.value_type(task);
    let results = pointer(function, task, mem::offset_of!(NativeTaskContext, results));
    let offset = function
        .ins()
        .imul_imm(index, mem::size_of::<ResultRecord>() as i64);
    let offset = function.ins().uextend(ty, offset);
    function.ins().iadd(results, offset)
}

pub(super) fn define(
    module: &mut JITModule,
    capture_contains: FuncId,
    capture_functions: &[FuncId],
    structural_reduction: bool,
) -> Result<FuncId, Box<cranelift_module::ModuleError>> {
    let pointer_type = module.target_config().pointer_type();
    let mut signature = module.make_signature();
    signature.params.push(AbiParam::new(pointer_type));
    signature.returns.push(AbiParam::new(types::I32));
    let id = module.declare_function("tempo_reduce_candidates", Linkage::Local, &signature)?;
    let mut context = module.make_context();
    context.func.signature = signature;
    let capture_contains = module.declare_func_in_func(capture_contains, &mut context.func);
    let capture_functions = capture_functions
        .iter()
        .map(|function| module.declare_func_in_func(*function, &mut context.func))
        .collect::<Vec<_>>();
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = function.create_block();
        function.append_block_params_for_function_params(entry);
        let outer = function.create_block();
        function.append_block_param(outer, types::I32);
        let outer_body = function.create_block();
        function.append_block_param(outer_body, types::I32);
        let inner = function.create_block();
        function.append_block_param(inner, types::I32);
        function.append_block_param(inner, types::I32);
        let compare = function.create_block();
        function.append_block_param(compare, types::I32);
        function.append_block_param(compare, types::I32);
        let relation = function.create_block();
        function.append_block_param(relation, types::I32);
        function.append_block_param(relation, types::I32);
        let check_right = function.create_block();
        function.append_block_param(check_right, types::I32);
        function.append_block_param(check_right, types::I32);
        function.append_block_param(check_right, types::I32);
        let remove_right = function.create_block();
        function.append_block_param(remove_right, types::I32);
        function.append_block_param(remove_right, types::I32);
        let next_inner = function.create_block();
        function.append_block_param(next_inner, types::I32);
        function.append_block_param(next_inner, types::I32);
        let next_outer = function.create_block();
        function.append_block_param(next_outer, types::I32);
        let compact = function.create_block();
        function.append_block_param(compact, types::I32);
        function.append_block_param(compact, types::I32);
        let compact_body = function.create_block();
        function.append_block_param(compact_body, types::I32);
        function.append_block_param(compact_body, types::I32);
        let predicate_blocks = capture_functions
            .iter()
            .map(|_| {
                let block = function.create_block();
                function.append_block_param(block, types::I32);
                function.append_block_param(block, types::I32);
                block
            })
            .collect::<Vec<_>>();
        let predicate_error = function.create_block();
        function.append_block_param(predicate_error, types::I32);
        let copy = function.create_block();
        function.append_block_param(copy, types::I32);
        function.append_block_param(copy, types::I32);
        let compact_next = function.create_block();
        function.append_block_param(compact_next, types::I32);
        function.append_block_param(compact_next, types::I32);
        let done = function.create_block();
        function.append_block_param(done, types::I32);

        function.switch_to_block(entry);
        let task = function.block_params(entry)[0];
        let status = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, status),
        );
        let count = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            status,
            mem::offset_of!(TaskStatus, result_count) as i32,
        );
        let zero = function.ins().iconst(types::I32, 0);
        if structural_reduction {
            function.ins().jump(outer, &[zero.into()]);
        } else {
            function.ins().jump(compact, &[zero.into(), zero.into()]);
        }

        function.switch_to_block(outer);
        let i = function.block_params(outer)[0];
        let complete = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, i, count);
        function.ins().brif(
            complete,
            compact,
            &[zero.into(), zero.into()],
            outer_body,
            &[i.into()],
        );

        function.switch_to_block(outer_body);
        let i = function.block_params(outer_body)[0];
        let left = result(&mut function, task, i);
        let live = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            left,
            mem::offset_of!(ResultRecord, dynamic_count) as i32,
        );
        let removed = function.ins().icmp_imm(IntCC::Equal, live, NONE as i64);
        let j = function.ins().iadd_imm(i, 1);
        function.ins().brif(
            removed,
            next_outer,
            &[i.into()],
            inner,
            &[i.into(), j.into()],
        );

        function.switch_to_block(inner);
        let values = function.block_params(inner).to_vec();
        let complete = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, values[1], count);
        function.ins().brif(
            complete,
            next_outer,
            &[values[0].into()],
            compare,
            &[values[0].into(), values[1].into()],
        );

        function.switch_to_block(compare);
        let values = function.block_params(compare).to_vec();
        let left = result(&mut function, task, values[0]);
        let right = result(&mut function, task, values[1]);
        let right_live = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            right,
            mem::offset_of!(ResultRecord, dynamic_count) as i32,
        );
        let right_removed = function
            .ins()
            .icmp_imm(IntCC::Equal, right_live, NONE as i64);
        let left_start = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            left,
            mem::offset_of!(ResultRecord, start) as i32,
        );
        let right_start = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            right,
            mem::offset_of!(ResultRecord, start) as i32,
        );
        let same_start = function.ins().icmp(IntCC::Equal, left_start, right_start);
        let right_live = function.ins().bxor_imm(right_removed, 1);
        let comparable = function.ins().band(right_live, same_start);
        function.ins().brif(
            comparable,
            relation,
            &[values[0].into(), values[1].into()],
            next_inner,
            &[values[0].into(), values[1].into()],
        );

        function.switch_to_block(relation);
        let values = function.block_params(relation).to_vec();
        let left = result(&mut function, task, values[0]);
        let right = result(&mut function, task, values[1]);
        let left_head = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            left,
            mem::offset_of!(ResultRecord, capture_start) as i32,
        );
        let right_head = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            right,
            mem::offset_of!(ResultRecord, capture_start) as i32,
        );
        let call = function
            .ins()
            .call(capture_contains, &[task, left_head, right_head]);
        let left_contains = function.inst_results(call)[0];
        let contains = function.ins().icmp_imm(IntCC::NotEqual, left_contains, 0);
        function.ins().brif(
            contains,
            remove_right,
            &[values[0].into(), values[1].into()],
            check_right,
            &[values[0].into(), values[1].into(), right_head.into()],
        );

        function.switch_to_block(check_right);
        let values = function.block_params(check_right).to_vec();
        let left = result(&mut function, task, values[0]);
        let left_head = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            left,
            mem::offset_of!(ResultRecord, capture_start) as i32,
        );
        let call = function
            .ins()
            .call(capture_contains, &[task, values[2], left_head]);
        let right_contains = function.inst_results(call)[0];
        let contains = function.ins().icmp_imm(IntCC::NotEqual, right_contains, 0);
        let remove_left = function.create_block();
        function.ins().brif(
            contains,
            remove_left,
            &[],
            next_inner,
            &[values[0].into(), values[1].into()],
        );
        function.switch_to_block(remove_left);
        let left = result(&mut function, task, values[0]);
        let none = function.ins().iconst(types::I32, NONE as i64);
        function.ins().store(
            MemFlags::trusted(),
            none,
            left,
            mem::offset_of!(ResultRecord, dynamic_count) as i32,
        );
        function.ins().jump(next_outer, &[values[0].into()]);

        function.switch_to_block(remove_right);
        let values = function.block_params(remove_right).to_vec();
        let right = result(&mut function, task, values[1]);
        let none = function.ins().iconst(types::I32, NONE as i64);
        function.ins().store(
            MemFlags::trusted(),
            none,
            right,
            mem::offset_of!(ResultRecord, dynamic_count) as i32,
        );
        function
            .ins()
            .jump(next_inner, &[values[0].into(), values[1].into()]);

        function.switch_to_block(next_inner);
        let values = function.block_params(next_inner).to_vec();
        let j = function.ins().iadd_imm(values[1], 1);
        function.ins().jump(inner, &[values[0].into(), j.into()]);
        function.switch_to_block(next_outer);
        let previous = function.block_params(next_outer)[0];
        let i = function.ins().iadd_imm(previous, 1);
        function.ins().jump(outer, &[i.into()]);

        function.switch_to_block(compact);
        let values = function.block_params(compact).to_vec();
        let complete = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, values[0], count);
        function.ins().brif(
            complete,
            done,
            &[values[1].into()],
            compact_body,
            &[values[0].into(), values[1].into()],
        );
        function.switch_to_block(compact_body);
        let values = function.block_params(compact_body).to_vec();
        let source = result(&mut function, task, values[0]);
        let live = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            source,
            mem::offset_of!(ResultRecord, dynamic_count) as i32,
        );
        let removed = function.ins().icmp_imm(IntCC::Equal, live, NONE as i64);
        let predicate_target = predicate_blocks.first().copied().unwrap_or(copy);
        function.ins().brif(
            removed,
            compact_next,
            &[values[0].into(), values[1].into()],
            predicate_target,
            &[values[0].into(), values[1].into()],
        );

        for (predicate_index, (predicate_block, predicate_function)) in predicate_blocks
            .iter()
            .copied()
            .zip(capture_functions.iter().copied())
            .enumerate()
        {
            function.switch_to_block(predicate_block);
            let values = function.block_params(predicate_block).to_vec();
            let source = result(&mut function, task, values[0]);
            let head = function.ins().load(
                types::I32,
                MemFlags::trusted(),
                source,
                mem::offset_of!(ResultRecord, capture_start) as i32,
            );
            function.ins().store(
                MemFlags::trusted(),
                head,
                task,
                mem::offset_of!(NativeTaskContext, current_captures) as i32,
            );
            function.ins().store(
                MemFlags::trusted(),
                zero,
                task,
                mem::offset_of!(NativeTaskContext, current_steps) as i32,
            );
            let call = function.ins().call(predicate_function, &[task]);
            let outcome = function.inst_results(call)[0];
            let rejected = function.ins().icmp_imm(IntCC::Equal, outcome, 0);
            let accepted = function.ins().icmp_imm(IntCC::Equal, outcome, 1);
            let continue_predicates = function.create_block();
            function.ins().brif(
                rejected,
                compact_next,
                &[values[0].into(), values[1].into()],
                continue_predicates,
                &[],
            );
            function.switch_to_block(continue_predicates);
            let next = predicate_blocks
                .get(predicate_index + 1)
                .copied()
                .unwrap_or(copy);
            function.ins().brif(
                accepted,
                next,
                &[values[0].into(), values[1].into()],
                predicate_error,
                &[outcome.into()],
            );
        }

        function.switch_to_block(copy);
        let values = function.block_params(copy).to_vec();
        let source = result(&mut function, task, values[0]);
        let target = result(&mut function, task, values[1]);
        for offset in FIELDS {
            let value = function
                .ins()
                .load(types::I32, MemFlags::trusted(), source, offset as i32);
            function
                .ins()
                .store(MemFlags::trusted(), value, target, offset as i32);
        }
        let selected = function.ins().iadd_imm(values[1], 1);
        function
            .ins()
            .jump(compact_next, &[values[0].into(), selected.into()]);
        function.switch_to_block(compact_next);
        let values = function.block_params(compact_next).to_vec();
        let i = function.ins().iadd_imm(values[0], 1);
        function.ins().jump(compact, &[i.into(), values[1].into()]);

        function.switch_to_block(done);
        let selected = function.block_params(done)[0];
        function.ins().store(
            MemFlags::trusted(),
            selected,
            status,
            mem::offset_of!(TaskStatus, result_count) as i32,
        );
        function.ins().return_(&[zero]);

        function.switch_to_block(predicate_error);
        let outcome = function.block_params(predicate_error)[0];
        function.ins().return_(&[outcome]);
        function.seal_all_blocks();
        function.finalize();
    }
    module.define_function(id, &mut context)?;
    module.clear_context(&mut context);
    Ok(id)
}
