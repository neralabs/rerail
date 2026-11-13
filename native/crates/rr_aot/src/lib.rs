//! The `.rr` parser built ahead of time, run on the JIT's harness, compiled by rustc and LLVM
//! from Rust that `rerail-rr-aot-gen` writes: tier 1's the JIT's own functions (their Cranelift
//! IR, `clif_rust`); tier 2's the structured backend of the native IR (`rust_stream`), a
//! function a rule, as the lean parser's.
//!
//! Build it optimized (`--profile profiling`): a dev build leaves this crate unoptimized.

mod tier1_aot;
mod tier2_aot;

use rerail_compiler::{clif_rust, native_cpu_block, native_cpu_stream, native_ir, rust_stream};

/// Which tiers run the ahead-of-time build.
#[derive(Clone, Copy, Debug)]
pub struct Tiers {
    pub one: bool,
    pub two: bool,
}

/// The grammar compiled as `rerail_rr_parser::compile_with` does, the tiers of `tiers` running
/// the ahead-of-time build; or why not: the JIT's program no longer translates to the source
/// (run `cargo run -p rerail-rr-aot-gen` again).
pub fn aot(tiers: Tiers) -> Result<rerail_rr_parser::Compiled, String> {
    native_cpu_stream::KEEP_IR.store(true, std::sync::atomic::Ordering::Relaxed);
    let mut c = rerail_rr_parser::compile_with(rerail_compiler::bitlex_masks::Shape::Rules)?;
    let one = clif_rust::rust_source(&c.parser)?;
    let two = rust_stream::rust_source(&native_ir::lower(&c.program, 0, 0).map_err(|e| format!("lowers: {e:?}"))?)?;
    let sealed = |src: &str| src.lines().last().and_then(|l| l.strip_prefix("pub const SOURCE: u64 = 0x")?.strip_suffix(';').and_then(|h| u64::from_str_radix(h, 16).ok()));
    if sealed(&one) != Some(tier1_aot::SOURCE) || sealed(&two) != Some(tier2_aot::SOURCE) {
        return Err("the ahead-of-time source is of another program: run `cargo run -p rerail-rr-aot-gen`".into());
    }
    c.parser.tier1.ir.clear();
    c.parser.tier2.ir.clear();
    if tiers.one {
        let tables = c.parser.tier1.tables_address().unwrap_or(0);
        tier1_aot::TABLES.store(tables as u64, std::sync::atomic::Ordering::Relaxed);
        type Raw = unsafe extern "C" fn(u64, u64, u64, u64, u64, u64, u64);
        let raw: &[Raw] = tier1_aot::ENTRIES;
        // SAFETY: each is the pass's code (the fingerprint), taking the JIT's entry's
        // arguments, all 64-bit integers or pointers in the same registers.
        let entries = raw.iter().map(|&e| unsafe { std::mem::transmute::<Raw, native_cpu_block::Entry>(e) }).collect();
        // SAFETY: as above, in the passes' order.
        unsafe { c.parser.tier1.set_entries(entries) };
    }
    if tiers.two {
        // SAFETY: the source is this program's code (the fingerprint), reading and writing
        // `Ctx` as the JIT's does. It counts its calls open itself (the frame shift unused).
        unsafe { c.parser.tier2.set_entry(entry, 4) };
    }
    Ok(c)
}

/// The generated entry, on the harness's `Ctx`.
unsafe extern "C" fn entry(ctx: *const native_cpu_stream::Ctx) -> u64 {
    // SAFETY: the harness's contract (`CompiledStream::call`).
    unsafe { tier2_aot::tier2_entry(ctx as u64) }
}

#[cfg(test)]
mod tests;
