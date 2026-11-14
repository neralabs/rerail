//! Writes the `.rr` parser's code as Rust: tier 1's through `clif_rust` (the JIT's functions'
//! IR) into `rr_aot/src/tier1_aot.rs`, tier 2's through `rust_stream` (the native IR, the
//! structured backend) into `tier2_aot.rs`. Each carries its fingerprint, which the
//! ahead-of-time build checks against the program when it runs.
//!
//! `cargo run -p rerail-rr-aot-gen` before building `rerail-rr-aot`.

use rerail_compiler::{clif_rust, native_cpu_stream, native_ir, rust_stream};

fn main() {
    native_cpu_stream::KEEP_IR.store(true, std::sync::atomic::Ordering::Relaxed);
    let c = rerail_rr_parser::compiled().expect("the grammar compiles");
    let one = clif_rust::rust_source(&c.parser).unwrap_or_else(|e| panic!("to Rust: {e}"));
    let native = native_ir::lower(&c.program, 0, 0).expect("lowers");
    let (widths, tag) = c.parser.tier2.widths_tag();
    let two = rust_stream::rust_source_at(&native, widths, tag).unwrap_or_else(|e| panic!("tier 2 to Rust: {e}"));
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../rr_aot/src");
    for (file, src, n) in [("tier1_aot.rs", &one, c.parser.tier1.ir.len()), ("tier2_aot.rs", &two, two.matches("\nfn r").count())] {
        std::fs::write(dir.join(file), src).expect("writes the source");
        eprintln!("{file}: {n} functions, {} bytes of Rust", src.len());
    }
}
