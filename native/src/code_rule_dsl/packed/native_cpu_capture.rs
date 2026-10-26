use super::{
    metal_batch_abi::{LinkRecord, MetalNode},
    native_cpu::NativeTaskContext,
    native_cpu_capture_pair,
    native_ir::NativeModule,
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

fn node_pointer(
    function: &mut FunctionBuilder<'_>,
    task: cranelift_codegen::ir::Value,
    node: cranelift_codegen::ir::Value,
) -> cranelift_codegen::ir::Value {
    let ty = function.func.dfg.value_type(task);
    let nodes = pointer(function, task, mem::offset_of!(NativeTaskContext, nodes));
    let stride = function
        .ins()
        .imul_imm(node, mem::size_of::<MetalNode>() as i64);
    let stride = function.ins().uextend(ty, stride);
    function.ins().iadd(nodes, stride)
}

fn string_equal(
    function: &mut FunctionBuilder<'_>,
    task: cranelift_codegen::ir::Value,
    node_id: cranelift_codegen::ir::Value,
    expected: &[u8],
) -> cranelift_codegen::ir::Value {
    let ty = function.func.dfg.value_type(task);
    let node = node_pointer(function, task, node_id);
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
    let length = function.ins().isub(end, start);
    let length_matches = function
        .ins()
        .icmp_imm(IntCC::Equal, length, expected.len() as i64);
    let compare = function.create_block();
    let reject = function.create_block();
    let merge = function.create_block();
    function.append_block_param(merge, types::I8);
    function
        .ins()
        .brif(length_matches, compare, &[], reject, &[]);
    function.switch_to_block(compare);
    let source = pointer(function, task, mem::offset_of!(NativeTaskContext, source));
    let start = function.ins().uextend(ty, start);
    let source = function.ins().iadd(source, start);
    let mut equal = function.ins().iconst(types::I8, 1);
    for (offset, byte) in expected.iter().copied().enumerate() {
        let actual = function
            .ins()
            .load(types::I8, MemFlags::trusted(), source, offset as i32);
        let found = function
            .ins()
            .icmp_imm(IntCC::Equal, actual, i64::from(byte));
        equal = function.ins().band(equal, found);
    }
    function.ins().jump(merge, &[equal.into()]);
    function.switch_to_block(reject);
    let false_value = function.ins().iconst(types::I8, 0);
    function.ins().jump(merge, &[false_value.into()]);
    function.switch_to_block(merge);
    function.block_params(merge)[0]
}

fn string_contains(
    function: &mut FunctionBuilder<'_>,
    task: cranelift_codegen::ir::Value,
    node_id: cranelift_codegen::ir::Value,
    expected: &[u8],
) -> cranelift_codegen::ir::Value {
    if expected.is_empty() {
        return function.ins().iconst(types::I8, 1);
    }
    let ty = function.func.dfg.value_type(task);
    let node = node_pointer(function, task, node_id);
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
    let length = function.ins().isub(end, start);
    let search = function.create_block();
    function.append_block_param(search, types::I32);
    let compare = function.create_block();
    function.append_block_param(compare, types::I32);
    let advance = function.create_block();
    function.append_block_param(advance, types::I32);
    let accept = function.create_block();
    let reject = function.create_block();
    let merge = function.create_block();
    function.append_block_param(merge, types::I8);
    let enough = function.ins().icmp_imm(
        IntCC::UnsignedGreaterThanOrEqual,
        length,
        expected.len() as i64,
    );
    let zero = function.ins().iconst(types::I32, 0);
    function
        .ins()
        .brif(enough, search, &[zero.into()], reject, &[]);
    function.switch_to_block(search);
    let at = function.block_params(search)[0];
    let limit = function.ins().iadd_imm(at, expected.len() as i64);
    let in_range = function
        .ins()
        .icmp(IntCC::UnsignedLessThanOrEqual, limit, length);
    function
        .ins()
        .brif(in_range, compare, &[at.into()], reject, &[]);
    function.switch_to_block(compare);
    let at = function.block_params(compare)[0];
    let source = pointer(function, task, mem::offset_of!(NativeTaskContext, source));
    let offset = function.ins().iadd(start, at);
    let offset = function.ins().uextend(ty, offset);
    let source = function.ins().iadd(source, offset);
    let mut equal = function.ins().iconst(types::I8, 1);
    for (offset, byte) in expected.iter().copied().enumerate() {
        let actual = function
            .ins()
            .load(types::I8, MemFlags::trusted(), source, offset as i32);
        let found = function
            .ins()
            .icmp_imm(IntCC::Equal, actual, i64::from(byte));
        equal = function.ins().band(equal, found);
    }
    function
        .ins()
        .brif(equal, accept, &[], advance, &[at.into()]);
    function.switch_to_block(advance);
    let previous = function.block_params(advance)[0];
    let at = function.ins().iadd_imm(previous, 1);
    function.ins().jump(search, &[at.into()]);
    function.switch_to_block(accept);
    let true_value = function.ins().iconst(types::I8, 1);
    function.ins().jump(merge, &[true_value.into()]);
    function.switch_to_block(reject);
    let false_value = function.ins().iconst(types::I8, 0);
    function.ins().jump(merge, &[false_value.into()]);
    function.switch_to_block(merge);
    function.block_params(merge)[0]
}

fn relation_matches(
    function: &mut FunctionBuilder<'_>,
    task: cranelift_codegen::ir::Value,
    node_id: cranelift_codegen::ir::Value,
    kinds: &[u16],
    ancestor: bool,
) -> cranelift_codegen::ir::Value {
    let node = node_pointer(function, task, node_id);
    let parent = function.ins().load(
        types::I32,
        MemFlags::trusted(),
        node,
        mem::offset_of!(MetalNode, parent) as i32,
    );
    let inspect = function.create_block();
    function.append_block_param(inspect, types::I32);
    let check = function.create_block();
    function.append_block_param(check, types::I32);
    let advance = function.create_block();
    function.append_block_param(advance, types::I32);
    let accept = function.create_block();
    let reject = function.create_block();
    let merge = function.create_block();
    function.append_block_param(merge, types::I8);
    function.ins().jump(inspect, &[parent.into()]);
    function.switch_to_block(inspect);
    let parent = function.block_params(inspect)[0];
    let exists = function
        .ins()
        .icmp_imm(IntCC::NotEqual, parent, u32::MAX as i64);
    function
        .ins()
        .brif(exists, check, &[parent.into()], reject, &[]);
    function.switch_to_block(check);
    let parent = function.block_params(check)[0];
    let node = node_pointer(function, task, parent);
    let actual = function.ins().load(
        types::I32,
        MemFlags::trusted(),
        node,
        mem::offset_of!(MetalNode, kind) as i32,
    );
    let mut found = function.ins().iconst(types::I8, 0);
    for kind in kinds {
        let equal = function
            .ins()
            .icmp_imm(IntCC::Equal, actual, i64::from(*kind));
        found = function.ins().bor(found, equal);
    }
    if ancestor {
        function
            .ins()
            .brif(found, accept, &[], advance, &[parent.into()]);
        function.switch_to_block(advance);
        let parent = function.block_params(advance)[0];
        let node = node_pointer(function, task, parent);
        let parent = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            node,
            mem::offset_of!(MetalNode, parent) as i32,
        );
        function.ins().jump(inspect, &[parent.into()]);
    } else {
        function.ins().brif(found, accept, &[], reject, &[]);
    }
    function.switch_to_block(accept);
    let true_value = function.ins().iconst(types::I8, 1);
    function.ins().jump(merge, &[true_value.into()]);
    function.switch_to_block(reject);
    let false_value = function.ins().iconst(types::I8, 0);
    function.ins().jump(merge, &[false_value.into()]);
    function.switch_to_block(merge);
    function.block_params(merge)[0]
}

fn define_one(
    module: &mut JITModule,
    id: FuncId,
    native: &NativeModule,
    program: &CaptureProgram,
    values: &[FuncId],
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
        .returns
        .push(AbiParam::new(types::I32));
    let value = match program {
        CaptureProgram::Text { value, .. } => {
            Some(module.declare_func_in_func(values[usize::from(*value)], &mut context.func))
        }
        _ => None,
    };
    let (label, negated, any) = match program {
        CaptureProgram::Text {
            capture,
            negated,
            any,
            ..
        }
        | CaptureProgram::AnyOf {
            capture,
            negated,
            any,
            ..
        }
        | CaptureProgram::Kind {
            capture,
            negated,
            any,
            ..
        }
        | CaptureProgram::Contains {
            capture,
            negated,
            any,
            ..
        } => (*capture, *negated, *any),
        CaptureProgram::Relation {
            capture, negated, ..
        } => (*capture, *negated, false),
        _ => unreachable!("unsupported capture program rejected before lowering"),
    };
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = function.create_block();
        function.append_block_params_for_function_params(entry);
        let loop_block = function.create_block();
        function.append_block_param(loop_block, types::I32);
        function.append_block_param(loop_block, types::I8);
        function.append_block_param(loop_block, types::I8);
        function.append_block_param(loop_block, types::I32);
        let inspect = function.create_block();
        let matched_label = function.create_block();
        let aggregate = function.create_block();
        function.append_block_param(aggregate, types::I32);
        function.append_block_param(aggregate, types::I8);
        function.append_block_param(aggregate, types::I8);
        function.append_block_param(aggregate, types::I32);
        function.append_block_param(aggregate, types::I8);
        function.append_block_param(aggregate, types::I8);
        let finish = function.create_block();
        function.append_block_param(finish, types::I8);
        function.append_block_param(finish, types::I8);
        function.append_block_param(finish, types::I32);
        let invalid = function.create_block();
        function.append_block_param(invalid, types::I32);

        function.switch_to_block(entry);
        let task = function.block_params(entry)[0];
        let head = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, current_captures) as i32,
        );
        let initial = function.ins().iconst(types::I8, i64::from(!any));
        let zero = function.ins().iconst(types::I32, 0);
        function.ins().jump(
            loop_block,
            &[head.into(), initial.into(), initial.into(), zero.into()],
        );

        function.switch_to_block(loop_block);
        let params = function.block_params(loop_block).to_vec();
        let done = function.ins().icmp_imm(IntCC::Equal, params[0], 0);
        function.ins().brif(
            done,
            finish,
            &[params[1].into(), params[2].into(), params[3].into()],
            inspect,
            &[],
        );

        function.switch_to_block(inspect);
        let captures = pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, captures),
        );
        let index = function.ins().iadd_imm(params[0], -1);
        let stride = function
            .ins()
            .imul_imm(index, mem::size_of::<LinkRecord>() as i64);
        let stride = function.ins().uextend(pointer_type, stride);
        let link = function.ins().iadd(captures, stride);
        let previous = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            link,
            mem::offset_of!(LinkRecord, previous) as i32,
        );
        let actual_label = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            link,
            mem::offset_of!(LinkRecord, first) as i32,
        );
        let has_label = function
            .ins()
            .icmp_imm(IntCC::Equal, actual_label, i64::from(label));
        function.ins().brif(
            has_label,
            matched_label,
            &[],
            loop_block,
            &[
                previous.into(),
                params[1].into(),
                params[2].into(),
                params[3].into(),
            ],
        );

        function.switch_to_block(matched_label);
        let node = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            link,
            mem::offset_of!(LinkRecord, second) as i32,
        );
        let (result, negative_result) = match program {
            CaptureProgram::Text { .. } => {
                let call = function
                    .ins()
                    .call(value.expect("text value function"), &[task, node]);
                let outcome = function.inst_results(call)[0];
                let valid = function
                    .ins()
                    .icmp_imm(IntCC::UnsignedLessThanOrEqual, outcome, 1);
                let valid_result = function.create_block();
                function
                    .ins()
                    .brif(valid, valid_result, &[], invalid, &[outcome.into()]);
                function.switch_to_block(valid_result);
                let result = function.ins().icmp_imm(IntCC::Equal, outcome, 1);
                let negative = function.ins().bxor_imm(result, 1);
                (result, negative)
            }
            CaptureProgram::AnyOf { strings, .. } => {
                let mut result = function.ins().iconst(types::I8, 0);
                for string in strings {
                    let found = string_equal(
                        &mut function,
                        task,
                        node,
                        &native.strings[usize::from(*string)],
                    );
                    result = function.ins().bor(result, found);
                }
                let negative = function.ins().bxor_imm(result, 1);
                (result, negative)
            }
            CaptureProgram::Kind { kinds, .. } => {
                let node = node_pointer(&mut function, task, node);
                let actual = function.ins().load(
                    types::I32,
                    MemFlags::trusted(),
                    node,
                    mem::offset_of!(MetalNode, kind) as i32,
                );
                let mut result = function.ins().iconst(types::I8, 0);
                for kind in kinds {
                    let found = function
                        .ins()
                        .icmp_imm(IntCC::Equal, actual, i64::from(*kind));
                    result = function.ins().bor(result, found);
                }
                let negative = function.ins().bxor_imm(result, 1);
                (result, negative)
            }
            CaptureProgram::Contains { strings, .. } => {
                let mut positive = function.ins().iconst(types::I8, i64::from(!any));
                let mut negative = positive;
                for string in strings {
                    let found = string_contains(
                        &mut function,
                        task,
                        node,
                        &native.strings[usize::from(*string)],
                    );
                    let not_found = function.ins().bxor_imm(found, 1);
                    positive = if any {
                        function.ins().bor(positive, found)
                    } else {
                        function.ins().band(positive, found)
                    };
                    negative = if any {
                        function.ins().bor(negative, not_found)
                    } else {
                        function.ins().band(negative, not_found)
                    };
                }
                (positive, negative)
            }
            CaptureProgram::Relation {
                kinds, ancestor, ..
            } => {
                let result = relation_matches(&mut function, task, node, kinds, *ancestor);
                let negative = function.ins().bxor_imm(result, 1);
                (result, negative)
            }
            _ => unreachable!(),
        };
        function.ins().jump(
            aggregate,
            &[
                previous.into(),
                params[1].into(),
                params[2].into(),
                params[3].into(),
                result.into(),
                negative_result.into(),
            ],
        );

        function.switch_to_block(aggregate);
        let values = function.block_params(aggregate).to_vec();
        let positive = if any {
            function.ins().bor(values[1], values[4])
        } else {
            function.ins().band(values[1], values[4])
        };
        let negative = if any {
            function.ins().bor(values[2], values[5])
        } else {
            function.ins().band(values[2], values[5])
        };
        let count = function.ins().iadd_imm(values[3], 1);
        function.ins().jump(
            loop_block,
            &[
                values[0].into(),
                positive.into(),
                negative.into(),
                count.into(),
            ],
        );

        function.switch_to_block(finish);
        let values = function.block_params(finish).to_vec();
        let empty = function.ins().icmp_imm(IntCC::Equal, values[2], 0);
        let selected = if negated { values[1] } else { values[0] };
        let empty_value = function.ins().iconst(
            types::I8,
            i64::from(match program {
                CaptureProgram::Kind {
                    empty_all: true, ..
                } => true,
                CaptureProgram::Contains { .. } | CaptureProgram::Relation { .. } => !negated,
                _ => false,
            }),
        );
        let selected = function.ins().select(empty, empty_value, selected);
        let selected = function.ins().uextend(types::I32, selected);
        function.ins().return_(&[selected]);

        function.switch_to_block(invalid);
        let outcome = function.block_params(invalid)[0];
        function.ins().return_(&[outcome]);
        function.seal_all_blocks();
        function.finalize();
    }
    module.define_function(id, &mut context)?;
    module.clear_context(&mut context);
    Ok(())
}

pub(super) fn define(
    module: &mut JITModule,
    native: &NativeModule,
    values: &[FuncId],
) -> Result<Vec<FuncId>, Box<cranelift_module::ModuleError>> {
    let mut functions = Vec::with_capacity(native.capture_programs.len());
    for (index, program) in native.capture_programs.iter().enumerate() {
        if matches!(
            program,
            CaptureProgram::Equal { .. } | CaptureProgram::SameLine { .. }
        ) {
            functions.push(native_cpu_capture_pair::define(module, index, program)?);
            continue;
        }
        let mut signature = module.make_signature();
        signature
            .params
            .push(AbiParam::new(module.target_config().pointer_type()));
        signature.returns.push(AbiParam::new(types::I32));
        let id = module.declare_function(
            &format!("tempo_capture_{index}"),
            Linkage::Local,
            &signature,
        )?;
        define_one(module, id, native, program, values)?;
        functions.push(id);
    }
    Ok(functions)
}
