//! Kotlin's Bitlex parser built ahead of time, run on the JIT's harness, compiled by rustc and
//! LLVM from Rust the build script writes, as `rerail-rr-aot` builds `.rr`'s: tier 1's from the
//! block program and its records, in the lean lexer's two stages (`rust_block`, decision 0432);
//! tier 2's the structured backend of the native IR (`rust_stream`).
//!
//! The generated tiers are the `generated` feature's; without it `aot` refuses. Build it optimized
//! (`cargo test -p rerail-kotlin-aot --features generated --profile profiling`): a dev build leaves
//! this crate unoptimized.

#[cfg(feature = "generated")]
// The lints `rust_block`'s output may trip: a mask or a helper a grammar does not read.
#[allow(unused_mut, unused_variables, unused_assignments, unused_unsafe, unused_parens, dead_code, clippy::all)]
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
use rerail_compiler::{native_cpu_block, native_cpu_stream, native_ir, rust_stream};

/// Tier 1's source from `c` (`rust_block`), as the build script writes it.
#[cfg(feature = "generated")]
fn tier1_source(c: &rerail_kotlin_parser::Compiled) -> Result<String, String> {
    include!("tier1_source.rs")
}

/// Which tiers run the ahead-of-time build.
#[derive(Clone, Copy, Debug)]
pub struct Tiers {
    pub one: bool,
    pub two: bool,
}

/// The grammar compiled as `rerail_kotlin_parser::compile_timed` does, `known` its proofs' answers
/// (decision 0404), the tiers of `tiers` running
/// the ahead-of-time build; or why not: the JIT's program does not translate to the source the
/// build script wrote.
#[cfg(feature = "generated")]
pub fn aot(tiers: Tiers, known: rerail_kotlin_parser::Proofs) -> Result<rerail_kotlin_parser::Compiled, String> {
    native_cpu_stream::KEEP_IR.store(true, std::sync::atomic::Ordering::Relaxed);
    // Compiled for Rust's NEON as the source was (the build script): its bit tests (`BIT_TEST`).
    // The same meaning either way; the flag back as it was for the JIT's compiles after.
    let bit_test = rerail_compiler::native_cpu_lanes::BIT_TEST.swap(true, std::sync::atomic::Ordering::Relaxed);
    let c = rerail_kotlin_parser::compile_timed(rerail_kotlin_parser::GRAMMAR, known, &mut |_, _| {});
    rerail_compiler::native_cpu_lanes::BIT_TEST.store(bit_test, std::sync::atomic::Ordering::Relaxed);
    let mut c = c?;
    let one = tier1_source(&c)?;
    let (widths, tag) = c.parser.tier2.widths_tag();
    let two = rust_stream::rust_source_at(&native_ir::lower(&c.program, 0, 0).map_err(|e| format!("lowers: {e:?}"))?, widths, tag)?;
    let sealed = |src: &str| src.lines().last().and_then(|l| l.strip_prefix("pub const SOURCE: u64 = 0x")?.strip_suffix(';').and_then(|h| u64::from_str_radix(h, 16).ok()));
    if sealed(&one) != Some(tier1_aot::SOURCE) || sealed(&two) != Some(tier2_aot::SOURCE) {
        return Err("the ahead-of-time source is of another program than the JIT compiles".into());
    }
    c.parser.tier1.ir.clear();
    c.parser.tier2.ir.clear();
    if tiers.one {
        // One entry for the program's one pass: the generated tier 1 makes all of it.
        if c.parser.tier1.pass_count() != 1 {
            return Err(format!("tier 1 in {} passes: the generated tier 1 is one", c.parser.tier1.pass_count()));
        }
        type Raw = unsafe extern "C" fn(*const u8, *const tier1_aot::Row, u64, *mut u64, *mut u32, *mut u64, *mut u64);
        let raw: Raw = tier1_aot::tier1_entry;
        // SAFETY: the program's code (the fingerprint), taking the JIT's entry's arguments: its
        // `Row` is `native_cpu_block::Row`, the same `repr(C)` fields.
        let entry = unsafe { std::mem::transmute::<Raw, native_cpu_block::Entry>(raw) };
        // SAFETY: as above.
        unsafe { c.parser.tier1.set_entries(vec![entry]) };
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
