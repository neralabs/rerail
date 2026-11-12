//! Tier 2 for `.rr` built ahead of time: the JIT's own functions (their Cranelift IR, after the
//! lowering's passes), written as Rust by `rerail-rr-aot-gen` (`clif_rust`) and compiled by
//! rustc and LLVM, run on the JIT's harness. The two builds are one IR through two compilers,
//! so a comparison of them is one of Cranelift's code against LLVM's.
//!
//! Build it optimized (`--profile profiling`): a dev build leaves this crate unoptimized.

mod tier2_aot;

use rerail_compiler::clif_rust;

/// The grammar compiled as `rerail_rr_parser::compile_with` does, tier 2 running the
/// ahead-of-time build, or why not: the JIT's IR is not the one the source was written from
/// (run `cargo run -p rerail-rr-aot-gen` again).
pub fn aot() -> Result<rerail_rr_parser::Compiled, String> {
    let mut c = rerail_rr_parser::compile_with(rerail_compiler::bitlex_masks::Shape::Rules)?;
    let now = clif_rust::fingerprint(&c.parser.tier2.clif);
    if now != tier2_aot::CLIF {
        return Err(format!("the ahead-of-time source is of another program ({:#x}, the JIT's {now:#x}): run `cargo run -p rerail-rr-aot-gen`", tier2_aot::CLIF));
    }
    // SAFETY: the source is this program's code (the fingerprint), reading and writing `Ctx`
    // as the JIT's does. A rule function's frame is at least its frame pointer and return
    // address (16 bytes, shift 4).
    unsafe { c.parser.tier2.set_entry(entry, 4) };
    Ok(c)
}

/// The generated entry, on the harness's `Ctx` (the source takes its address).
unsafe extern "C" fn entry(ctx: *const rerail_compiler::native_cpu_stream::Ctx) -> u64 {
    // SAFETY: the harness's contract (`CompiledStream::call`).
    unsafe { tier2_aot::tier2_aot_entry(ctx.cast()) }
}

#[cfg(test)]
mod tests;
