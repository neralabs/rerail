use super::*;
use cranelift_codegen::ir::{AbiParam, InstBuilder, JumpTableData, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};

/// `switch_trees`: a table of runs, a gap of the default and one entry alone, over every
/// index and past its end, Cranelift's code gives the table's answer, with no `br_table` left
/// and, on Arm, no speculation barrier in the code: lowered as rustc does where no target takes
/// an argument (0322), as a balanced tree where one does (Q186).
#[test]
fn a_table_as_compares_is_the_table() {
    // Index 0..12: targets 0 0 0 1 1 D D 2 3 3 3 1 (D the default), past 12 the default.
    let table: [Option<i64>; 12] = [Some(0), Some(0), Some(0), Some(1), Some(1), None, None, Some(2), Some(3), Some(3), Some(3), Some(1)];
    for argument in [true, false] {
    let mut module = JITModule::new(cranelift_jit::JITBuilder::with_flags(&[("opt_level", "speed")], cranelift_module::default_libcall_names()).unwrap());
    let mut sig = module.make_signature();
    sig.params.push(AbiParam::new(types::I32));
    sig.returns.push(AbiParam::new(types::I64));
    let id = module.declare_function("table", cranelift_module::Linkage::Local, &sig).unwrap();
    let mut context = module.make_context();
    context.func.signature = sig;
    let mut fctx = FunctionBuilderContext::new();
    {
        let mut f = FunctionBuilder::new(&mut context.func, &mut fctx);
        let entry = f.create_block();
        f.append_block_params_for_function_params(entry);
        f.switch_to_block(entry);
        let x = f.block_params(entry)[0];
        // Each target returns its number, the default 99; target 1 takes it as a parameter
        // (with `argument`).
        let targets: Vec<_> = (0..4).map(|_| f.create_block()).collect();
        if argument {
            f.append_block_param(targets[1], types::I64);
        }
        let default = f.create_block();
        let one = f.ins().iconst(types::I64, 1);
        let calls: Vec<_> = table
            .iter()
            .map(|t| match t {
                Some(1) if argument => f.func.dfg.block_call(targets[1], &[one.into()]),
                Some(k) => f.func.dfg.block_call(targets[*k as usize], &[]),
                None => f.func.dfg.block_call(default, &[]),
            })
            .collect();
        let d = f.func.dfg.block_call(default, &[]);
        let jt = f.create_jump_table(JumpTableData::new(d, &calls));
        f.ins().br_table(x, jt);
        for (k, &b) in targets.iter().enumerate() {
            f.switch_to_block(b);
            let v = if k == 1 && argument { f.block_params(b)[0] } else { f.ins().iconst(types::I64, k as i64) };
            f.ins().return_(&[v]);
        }
        f.switch_to_block(default);
        let v = f.ins().iconst(types::I64, 99);
        f.ins().return_(&[v]);
        f.seal_all_blocks();
        f.finalize();
    }
    context.set_disasm(true);
    define_function(&mut module, id, &mut context).unwrap();
    assert!(!context.func.display().to_string().contains("br_table"));
    let code = context.compiled_code().unwrap().code_buffer().to_vec();
    if cfg!(target_arch = "aarch64") {
        assert!(!code.chunks_exact(4).any(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]) == 0xd503_229f), "a speculation barrier");
    }
    module.finalize_definitions().unwrap();
    // SAFETY: declared and defined with this signature.
    let run = unsafe { std::mem::transmute::<*const u8, extern "C" fn(u32) -> i64>(module.get_finalized_function(id)) };
    for x in 0..20u32 {
        let want = table.get(x as usize).copied().flatten().unwrap_or(99);
        assert_eq!(run(x), want, "index {x}, argument {argument}");
    }
    assert_eq!(run(u32::MAX), 99);
    }
}
