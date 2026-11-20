//! With the `generated` feature, writes the `.rr` parser's code as Rust into `OUT_DIR`: tier 1's
//! through `clif_rust` (the JIT's functions' IR) as `tier1_aot.rs`, tier 2's through `rust_stream`
//! (the native IR, the structured backend) as `tier2_aot.rs`, and in format 16, writing the tape by the writer's ops
//! as it parses (decision 0434), as `tier2_16_aot.rs`. Each carries its fingerprint, which
//! `aot` checks against the program the JIT compiles when it runs. Their inner attributes and
//! docs are dropped: `lib.rs` includes each inside a module and sets the attributes there.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    #[cfg(feature = "generated")]
    generate();
}

#[cfg(feature = "generated")]
fn generate() {
    use rerail_compiler::{clif_rust, native_ir, rust_stream};
    // The compiles in a run of the script's own: their options (decision 0442) are its.
    let _run = rerail_context::within(rerail_context::default_shared());
    rerail_compiler::native_cpu_codegen::options().keep_ir.store(true, std::sync::atomic::Ordering::Relaxed);
    // Rust's NEON tests bits in one instruction (`vtst`): classes as the tests themselves.
    rerail_compiler::native_cpu_codegen::options().bit_test.store(true, std::sync::atomic::Ordering::Relaxed);
    let c = rerail_rr_parser::compiled().expect("the grammar compiles");
    let one = clif_rust::rust_source(&c.parser).unwrap_or_else(|e| panic!("tier 1 to Rust: {e}"));
    let native = native_ir::lower(&c.program, 0, 0).expect("lowers");
    let (widths, tag) = c.parser.tier2.widths_tag();
    let two = rust_stream::rust_source_at(&native, widths, tag).unwrap_or_else(|e| panic!("tier 2 to Rust: {e}"));
    let (widths16, tag16) = c.parser.sixteen().expect("a format 16 tier 2").widths_tag();
    let sixteen = rust_stream::rust_source_16(&native, widths16, tag16).unwrap_or_else(|e| panic!("tier 2 in format 16 to Rust: {e}"));
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    for (file, src) in [("tier1_aot.rs", &one), ("tier2_aot.rs", &two), ("tier2_16_aot.rs", &sixteen)] {
        let body: String = src
            .lines()
            .filter(|l| !l.starts_with("#![") && !l.starts_with("//!"))
            .flat_map(|l| [l, "\n"])
            .collect();
        std::fs::write(out.join(file), body).expect("writes the source");
    }
}
