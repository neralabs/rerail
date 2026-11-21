//! **Deprecated** (decision 0550): Rust from an intermediate form. A grammar's Rust is the Bitlex compiler's own (`bitlex_lex_rust`, `bitlex_rust`); no new use. Kept until Go-C-and-C-support removes its use (`rerail-pp-aot`, `c_directives`).
//!
//! The translator is `rerail_clif::rust` (decision 0310), which the tape's reads share.

pub use rerail_clif::rust::*;

// The one test is aarch64's (NEON): elsewhere the module is empty.
#[cfg(all(test, target_arch = "aarch64"))]
mod tests {
    use cranelift_codegen::ir::{AbiParam, InstBuilder, Signature, types::I64};
    use cranelift_codegen::isa::CallConv;
    use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};

    /// A 16-byte compare's mask by NEON's narrowing shift (`native_cpu_block::short_mask`) is
    /// written as one `vshrn_n_u16`, its shift, `and` and casts not besides; and the lanes'
    /// count after it reads the nibbles (`lanes_set`).
    #[test]
    fn a_narrowing_shift_is_one_shrn() {
        use cranelift_codegen::ir::types::{I8, I8X16};
        use cranelift_codegen::ir::{MemFlags, condcodes::IntCC};
        let mut sig = Signature::new(CallConv::Tail);
        sig.params.push(AbiParam::new(I64));
        sig.returns.push(AbiParam::new(I64));
        let mut f = cranelift_codegen::ir::Function::with_name_signature(cranelift_codegen::ir::UserFuncName::user(0, 0), sig);
        let mut ctx = FunctionBuilderContext::new();
        let mut b = FunctionBuilder::new(&mut f, &mut ctx);
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        let p = b.block_params(entry)[0];
        let k = crate::native_cpu_block::Consts::make(&mut b);
        let x = b.ins().load(I8X16, MemFlags::trusted(), p, 0);
        let dot = b.ins().iconst(I8, i64::from(b'.'));
        let dot = b.ins().splat(I8X16, dot);
        let eq = b.ins().icmp(IntCC::Equal, x, dot);
        let m = crate::native_cpu_block::short_mask(&mut b, eq, k.weights);
        let n = crate::native_cpu_block::lanes_set(&mut b, m, crate::native_cpu_block::SHORT_STRIDE);
        b.ins().return_(&[n]);
        b.seal_all_blocks();
        b.finalize();
        let src = rerail_clif::rust::translate(&[(0, f)], &[(0, "short")], &[]).expect("translates");
        assert_eq!(src.matches("vshrn_n_u16::<4>").count(), 1, "{src}");
        for not in ["vandq_u16", "vshrq_n_u16", "vqmovn_u16", "vpaddq_u8", "vpadd_u8"] {
            assert!(!src.contains(not), "{not}: {src}");
        }
        assert!(src.contains(".trailing_zeros()") && src.contains(".wrapping_shr(2)"), "{src}");
    }
}
