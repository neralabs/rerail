//! The parser's Cranelift IR as Rust source: the translator is `rerail_clif::rust` (decision 0310), which the
//! tape's reads share; what is the compiler's own is tier 1's source of a compiled parser.

pub use rerail_clif::rust::*;
use std::fmt::Write;

/// Tier 1's source, from a parser compiled with `native_cpu_stream::KEEP_IR`: `tier1_entry_K`
/// a pass, the record pass's tables as data, `TABLES_DATA`; it ends with its fingerprint,
/// `SOURCE`.
/// (Tier 2's is `rust_stream`'s, from the native IR.)
pub fn rust_source(parser: &crate::native_cpu_stream::Parser) -> Result<String, String> {
    let t1 = &parser.tier1;
    let names: Vec<String> = (0..t1.entry_ids.len()).map(|k| format!("tier1_entry_{k}")).collect();
    let entries: Vec<(u32, &str)> = t1.entry_ids.iter().zip(&names).map(|(&id, n)| (id, n.as_str())).collect();
    // The tables as the binary's data, as the lean lexer's: an address in them is a constant
    // the code makes again where it needs it (`adrp`), not a register or a load.
    let words = t1.tables_words().unwrap_or(&[]);
    let addresses: Vec<(i64, i64, &str, Option<&[u64]>)> = t1.tables_span().map(|(a, n)| (a, n, "TABLES", Some(words))).into_iter().collect();
    let mut one = translate(&t1.ir, &entries, &addresses).map_err(|e| format!("tier 1: {e}"))?;
    writeln!(one, "/// The passes' entries, in order.\npub const ENTRIES: &[unsafe extern \"C\" fn(u64, u64, u64, u64, u64, u64, u64)] = &[{}];", names.join(", ")).unwrap();
    let h = fingerprint(&one);
    Ok(format!("{one}\n/// This source's fingerprint (`clif_rust::fingerprint` of what comes before).\npub const SOURCE: u64 = {h:#x};\n"))
}


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
