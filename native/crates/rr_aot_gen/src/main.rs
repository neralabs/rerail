//! Writes the `.rr` parser's code as Rust: the JIT's functions' IR, tier 1's and tier 2's,
//! through `clif_rust`, into `rr_aot/src/tier1_aot.rs` and `tier2_aot.rs`. Each carries its
//! fingerprint, which the ahead-of-time build checks against the JIT's program when it runs.
//!
//! `cargo run -p rerail-rr-aot-gen` before building `rerail-rr-aot`.

use rerail_compiler::{clif_rust, native_cpu_stream};

fn main() {
    native_cpu_stream::KEEP_IR.store(true, std::sync::atomic::Ordering::Relaxed);
    let c = rerail_rr_parser::compiled().expect("the grammar compiles");
    let (one, two) = clif_rust::rust_sources(&c.parser).unwrap_or_else(|e| panic!("to Rust: {e}"));
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../rr_aot/src");
    for (file, src, n) in [("tier1_aot.rs", &one, c.parser.tier1.ir.len()), ("tier2_aot.rs", &two, c.parser.tier2.ir.len())] {
        std::fs::write(dir.join(file), src).expect("writes the source");
        eprintln!("{file}: {n} functions, {} bytes of Rust", src.len());
    }
}
