//! With the `generated` feature, writes Kotlin's parser's code as Rust into `OUT_DIR`, as
//! `rerail-rr-aot`'s build script does for `.rerail`: tier 1's through `rust_block` (the block
//! program and its records, in the lean lexer's two stages, decision 0451) as `tier1_aot.rs`,
//! tier 2's through `rust_stream` (the native IR, the structured backend) as `tier2_aot.rs`. Each carries its fingerprint, which `aot` checks against the program
//! the JIT compiles when it runs.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../kotlin_parser/kotlin.bitlex");
    println!("cargo:rerun-if-changed=../kotlin_parser/kotlin.answers");
    println!("cargo:rerun-if-changed=src/tier1_source.rs");
    #[cfg(feature = "generated")]
    generate();
}

#[cfg(feature = "generated")]
fn generate() {
    use rerail_compiler::{native_ir, rust_stream};
    // The compiles in a run of the script's own: their options (decision 0442) are its.
    let _run = rerail_context::within(rerail_context::default_shared());
    rerail_compiler::native_cpu_codegen::options().keep_ir.store(true, std::sync::atomic::Ordering::Relaxed);
    // Rust's NEON tests bits in one instruction (`vtst`): classes as the tests themselves.
    rerail_compiler::native_cpu_codegen::options().bit_test.store(true, std::sync::atomic::Ordering::Relaxed);
    // With the shipped answers and the machine's (decision 0414): the provers ask nothing they answered before.
    let c = rerail_kotlin_parser::compile_answered(rerail_kotlin_parser::GRAMMAR).expect("the grammar compiles");
    let one = rerail_kotlin_aot_tier1(&c).unwrap_or_else(|e| panic!("tier 1 to Rust: {e}"));
    let native = native_ir::lower(&c.program, 0, 0).expect("lowers");
    let (widths, tag) = c.parser.tier2.widths_tag();
    let two = rust_stream::rust_source_at(&native, widths, tag).unwrap_or_else(|e| panic!("tier 2 to Rust: {e}"));
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    for (file, src) in [("tier1_aot.rs", &one), ("tier2_aot.rs", &two)] {
        let body: String = src.lines().filter(|l| !l.starts_with("#![") && !l.starts_with("//!")).flat_map(|l| [l, "\n"]).collect();
        std::fs::write(out.join(file), body).expect("writes the source");
    }
}

/// Tier 1's source (`rust_block`), as `aot` writes it again to check its fingerprint.
#[cfg(feature = "generated")]
fn rerail_kotlin_aot_tier1(c: &rerail_kotlin_parser::Compiled) -> Result<String, String> {
    include!("src/tier1_source.rs")
}
