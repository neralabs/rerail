//! Kotlin's Bitlex parser built ahead of time, run on the JIT's harness, compiled by rustc and
//! LLVM from Rust the build script writes, as `rerail-rr-aot` builds `.rerail`'s: tier 1's the JIT's
//! own functions (their Cranelift IR, `clif_rust`); tier 2's the structured backend of the native
//! IR (`rust_stream`).
//!
//! The generated tiers are the `generated` feature's; without it `aot` refuses. Build it optimized
//! (`cargo test -p rerail-kotlin-aot --features generated --profile profiling`): a dev build leaves
//! this crate unoptimized.

#[cfg(feature = "generated")]
#[allow(unused_mut, unused_variables, unused_assignments, unreachable_code, unused_labels, unused_unsafe, unused_imports, clippy::all)]
mod tier1_aot {
    include!(concat!(env!("OUT_DIR"), "/tier1_aot.rs"));
}
#[cfg(feature = "generated")]
// The lints `rust_stream` allows in what it writes (its own `#![allow]`, which the build script drops).
#[allow(unused_mut, unused_variables, unused_assignments, unreachable_code, unused_labels, unused_unsafe, unused_parens, non_snake_case, dead_code, clippy::all)]
mod tier2_aot {
    include!(concat!(env!("OUT_DIR"), "/tier2_aot.rs"));
}

#[cfg(feature = "generated")]

// Each of tier 1's functions is structured control, as rustc lays out a hand-written loop: none
// is the irreducible fallback, a loop over a `match` (`clif_rust`).
const _: () = assert!(tier1_aot::STRUCTURED.0 == tier1_aot::STRUCTURED.1, "a tier 1 function is irreducible: a loop over a match");

#[cfg(feature = "generated")]
use rerail_compiler::{clif_rust, native_cpu_block, native_cpu_stream, native_ir, rust_stream};

/// Which tiers run the ahead-of-time build.
#[derive(Clone, Copy, Debug)]
pub struct Tiers {
    pub one: bool,
    pub two: bool,
}

/// The grammar compiled as `rerail_kotlin_parser::compile_timed` does, `known` its proofs' answers
/// (decision 0414), the tiers of `tiers` running
/// the ahead-of-time build; or why not: the JIT's program does not translate to the source the
/// build script wrote.
#[cfg(feature = "generated")]
pub fn aot(tiers: Tiers, known: rerail_kotlin_parser::Proofs) -> Result<rerail_kotlin_parser::Compiled, String> {
    // The compile in a run of its own (decision 0442), on the caller's shared object: its options
    // end with it, and the caller's compiles after keep theirs. Each function's IR kept, for its
    // Rust; compiled for Rust's NEON as the source was (the build script): its bit tests
    // (`Options::bit_test`), the same meaning either way.
    let c = {
        let _run = rerail_context::within(rerail_context::shared_here());
        let options = rerail_compiler::native_cpu_codegen::options();
        options.keep_ir.store(true, std::sync::atomic::Ordering::Relaxed);
        options.bit_test.store(true, std::sync::atomic::Ordering::Relaxed);
        rerail_kotlin_parser::compile_timed(rerail_kotlin_parser::GRAMMAR, known, &mut |_, _| {})
    };
    let mut c = c?;
    let one = clif_rust::rust_source(&c.parser)?;
    let (widths, tag) = c.parser.tier2.widths_tag();
    let two = rust_stream::rust_source_at(&native_ir::lower(&c.program, 0, 0).map_err(|e| format!("lowers: {e:?}"))?, widths, tag)?;
    let sealed = |src: &str| src.lines().last().and_then(|l| l.strip_prefix("pub const SOURCE: u64 = 0x")?.strip_suffix(';').and_then(|h| u64::from_str_radix(h, 16).ok()));
    if sealed(&one) != Some(tier1_aot::SOURCE) || sealed(&two) != Some(tier2_aot::SOURCE) {
        return Err("the ahead-of-time source is of another program than the JIT compiles".into());
    }
    c.parser.tier1.ir.clear();
    c.parser.tier2.ir.clear();
    if tiers.one {
        // The code reads its own copy of the tables (`TABLES_DATA`): the program's.
        if c.parser.tier1.tables_words() != Some(&tier1_aot::TABLES_DATA[..]) {
            return Err("the ahead-of-time tables are of another program: the build script's program is another".into());
        }
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

/// Without the generated tiers there is no ahead-of-time build.
#[cfg(not(feature = "generated"))]
pub fn aot(_: Tiers, _: rerail_kotlin_parser::Proofs) -> Result<rerail_kotlin_parser::Compiled, String> {
    Err("built without the `generated` feature (`--features generated`)".into())
}

/// The generated entry, on the harness's `Ctx`.
#[cfg(feature = "generated")]
unsafe extern "C" fn entry(ctx: *const native_cpu_stream::Ctx) -> u64 {
    // SAFETY: the harness's contract (`CompiledStream::call`).
    unsafe { tier2_aot::tier2_entry(ctx as u64) }
}

#[cfg(all(test, feature = "generated"))]
mod tests;
