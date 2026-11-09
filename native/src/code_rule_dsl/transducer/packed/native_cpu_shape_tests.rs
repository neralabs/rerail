//! The shape guard on hand-built functions: a loop a byte at a time and a call are refused; the
//! same loop named as a progress point is not.

use super::*;
use cranelift_codegen::ir::{AbiParam, ExtFuncData, ExternalName, InstBuilder, Signature, UserFuncName, condcodes::IntCC, types};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};

/// `for i in 0..n { sum += bytes[i] }`: a loop that steps one byte. Returns the function and the
/// loop's head.
fn byte_loop() -> (Function, Block) {
    let mut sig = Signature::new(CallConv::SystemV);
    sig.params.push(AbiParam::new(types::I64));
    sig.params.push(AbiParam::new(types::I64));
    sig.returns.push(AbiParam::new(types::I64));
    let mut func = Function::with_name_signature(UserFuncName::default(), sig);
    let mut ctx = FunctionBuilderContext::new();
    let head;
    {
        let mut f = FunctionBuilder::new(&mut func, &mut ctx);
        let (entry, top, done) = (f.create_block(), f.create_block(), f.create_block());
        head = top;
        f.append_block_params_for_function_params(entry);
        f.append_block_param(top, types::I64);
        f.append_block_param(top, types::I64);
        f.switch_to_block(entry);
        let (bytes, n) = (f.block_params(entry)[0], f.block_params(entry)[1]);
        let zero = f.ins().iconst(types::I64, 0);
        f.ins().jump(top, &[zero.into(), zero.into()]);
        f.switch_to_block(top);
        let (i, sum) = (f.block_params(top)[0], f.block_params(top)[1]);
        let at = f.ins().iadd(bytes, i);
        let b = f.ins().uload8(types::I64, cranelift_codegen::ir::MemFlags::trusted(), at, 0);
        let sum = f.ins().iadd(sum, b);
        let i = f.ins().iadd_imm(i, 1);
        let more = f.ins().icmp(IntCC::UnsignedLessThan, i, n);
        f.ins().brif(more, top, &[i.into(), sum.into()], done, &[]);
        f.switch_to_block(done);
        f.ins().return_(&[sum]);
        f.seal_all_blocks();
        f.finalize();
    }
    (func, head)
}

#[test]
fn a_loop_a_byte_at_a_time_is_refused() {
    let (func, head) = byte_loop();
    assert!(matches!(check(&func, &[]), Err(Refusal::Loop(_))));
    // The guard trusts what the lowering names: a named head passes. The lowerings name only
    // the stream's, the blocks', the bits' and the files' loops.
    assert_eq!(check(&func, &[(head, Step::Block)]), Ok(()));
}

#[test]
fn a_call_is_refused() {
    let mut sig = Signature::new(CallConv::SystemV);
    sig.returns.push(AbiParam::new(types::I64));
    let mut func = Function::with_name_signature(UserFuncName::default(), sig.clone());
    let mut ctx = FunctionBuilderContext::new();
    {
        let mut f = FunctionBuilder::new(&mut func, &mut ctx);
        let entry = f.create_block();
        f.switch_to_block(entry);
        let sigref = f.import_signature(sig);
        let name = f.func.declare_imported_user_function(cranelift_codegen::ir::UserExternalName::new(0, 0));
        let callee = f.import_function(ExtFuncData { name: ExternalName::user(name), signature: sigref, colocated: false });
        let call = f.ins().call(callee, &[]);
        let r = f.inst_results(call)[0];
        f.ins().return_(&[r]);
        f.seal_all_blocks();
        f.finalize();
    }
    assert!(matches!(check(&func, &[]), Err(Refusal::Call(_))));
}
