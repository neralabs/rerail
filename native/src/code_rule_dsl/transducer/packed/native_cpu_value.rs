use super::{
    metal_batch_abi::MetalNode, native_cpu::NativeTaskContext, native_cpu_lua, native_cpu_regex,
    native_ir::NativeModule, output::OutputInstruction, value::ValueProgram,
};
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlags, condcodes::IntCC, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Linkage, Module};
use std::mem;

fn context_pointer(
    function: &mut FunctionBuilder<'_>,
    context: cranelift_codegen::ir::Value,
    offset: usize,
) -> cranelift_codegen::ir::Value {
    let pointer_type = function.func.dfg.value_type(context);
    function
        .ins()
        .load(pointer_type, MemFlags::trusted(), context, offset as i32)
}

fn define_bytes(
    module: &mut JITModule,
    id: FuncId,
    expected: &[u8],
    prefix: bool,
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
        let inspect = function.create_block();
        let compare = function.create_block();
        let accept = function.create_block();
        let reject = function.create_block();
        let invalid = function.create_block();
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
        function.ins().brif(valid_node, inspect, &[], invalid, &[]);

        function.switch_to_block(inspect);
        let nodes = context_pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, nodes),
        );
        let stride = function
            .ins()
            .imul_imm(node_index, mem::size_of::<MetalNode>() as i64);
        let stride = function.ins().uextend(pointer_type, stride);
        let node = function.ins().iadd(nodes, stride);
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
        let ordered = function
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, end, start);
        let source_length = function.ins().load(
            types::I32,
            MemFlags::trusted(),
            task,
            mem::offset_of!(NativeTaskContext, source_length) as i32,
        );
        let in_bounds = function
            .ins()
            .icmp(IntCC::UnsignedLessThanOrEqual, end, source_length);
        let valid = function.ins().band(ordered, in_bounds);
        function.ins().brif(valid, compare, &[], invalid, &[]);

        function.switch_to_block(compare);
        let length = function.ins().isub(end, start);
        let length_ok = if prefix {
            function.ins().icmp_imm(
                IntCC::UnsignedGreaterThanOrEqual,
                length,
                expected.len() as i64,
            )
        } else {
            function
                .ins()
                .icmp_imm(IntCC::Equal, length, expected.len() as i64)
        };
        let compare_bytes = function.create_block();
        function
            .ins()
            .brif(length_ok, compare_bytes, &[], reject, &[]);
        function.switch_to_block(compare_bytes);
        let source = context_pointer(
            &mut function,
            task,
            mem::offset_of!(NativeTaskContext, source),
        );
        let start_offset = function.ins().uextend(pointer_type, start);
        let source = function.ins().iadd(source, start_offset);
        let mut equal = function.ins().icmp(IntCC::Equal, length, length);
        for (offset, expected) in expected.iter().copied().enumerate() {
            let actual = function
                .ins()
                .load(types::I8, MemFlags::trusted(), source, offset as i32);
            let byte_equal = function
                .ins()
                .icmp_imm(IntCC::Equal, actual, i64::from(expected));
            equal = function.ins().band(equal, byte_equal);
        }
        function.ins().brif(equal, accept, &[], reject, &[]);

        function.switch_to_block(accept);
        let one = function.ins().iconst(types::I32, 1);
        function.ins().return_(&[one]);
        function.switch_to_block(reject);
        let zero = function.ins().iconst(types::I32, 0);
        function.ins().return_(&[zero]);
        function.switch_to_block(invalid);
        let two = function.ins().iconst(types::I32, 2);
        function.ins().return_(&[two]);
        function.seal_all_blocks();
        function.finalize();
    }
    super::native_cpu_codegen::define_function(module, id, &mut context)?;
    module.clear_context(&mut context);
    Ok(())
}

pub(super) struct ValueFunctions {
    pub(super) evaluators: Vec<FuncId>,
    pub(super) regex_find: Option<FuncId>,
}

pub(super) fn define(
    module: &mut JITModule,
    native: &NativeModule,
) -> Result<ValueFunctions, Box<cranelift_module::ModuleError>> {
    let mut functions = Vec::with_capacity(native.value_programs.len());
    let regex_helpers = (native
        .value_programs
        .iter()
        .any(|value| matches!(value, ValueProgram::Regex(_)))
        || native
            .output_program
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, OutputInstruction::Strip { .. })))
    .then(|| native_cpu_regex::define_helpers(module))
    .transpose()?;
    for (index, value) in native.value_programs.iter().enumerate() {
        let mut signature = module.make_signature();
        signature
            .params
            .push(AbiParam::new(module.target_config().pointer_type()));
        signature.params.push(AbiParam::new(types::I32));
        signature.returns.push(AbiParam::new(types::I32));
        let id =
            module.declare_function(&format!("tempo_value_{index}"), Linkage::Local, &signature)?;
        match value {
            ValueProgram::Equal { string } => {
                define_bytes(module, id, &native.strings[usize::from(*string)], false)?;
            }
            ValueProgram::Prefix { string } => {
                define_bytes(module, id, &native.strings[usize::from(*string)], true)?;
            }
            ValueProgram::Lua(program) => native_cpu_lua::define(module, id, program)?,
            ValueProgram::Regex(program) => native_cpu_regex::define(
                module,
                id,
                program,
                index as u32,
                regex_helpers.expect("regex helpers exist for a regex value"),
            )?,
        }
        functions.push(id);
    }
    Ok(ValueFunctions {
        evaluators: functions,
        regex_find: regex_helpers.map(|helpers| helpers.find),
    })
}
