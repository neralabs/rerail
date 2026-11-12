//! Writes tier 2's code for `.rr` as Rust: the JIT's functions' IR (`CompiledStream::ir`)
//! through `clif_rust`, into `rr_aot/src/tier2_aot.rs`, with a fingerprint of the IR the
//! ahead-of-time build checks against the JIT's when it runs.
//!
//! `cargo run -p rerail-rr-aot-gen` before building `rerail-rr-aot`.

use rerail_compiler::{clif_rust, native_cpu_stream};

fn main() {
    native_cpu_stream::KEEP_IR.store(true, std::sync::atomic::Ordering::Relaxed);
    let c = rerail_rr_parser::compiled().expect("the grammar compiles");
    let t = &c.parser.tier2;
    let mut src = clif_rust::translate(&t.ir, t.entry_id).unwrap_or_else(|e| panic!("to Rust: {e}"));
    src.push_str(&format!("\n/// The IR this was written from (`clif_rust::fingerprint` of the JIT's CLIF).\npub const CLIF: u64 = {:#x};\n", clif_rust::fingerprint(&t.clif)));
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../rr_aot/src/tier2_aot.rs");
    std::fs::write(&out, &src).expect("writes the source");
    eprintln!("{} functions, {} bytes of Rust: {}", t.ir.len(), src.len(), out.display());
}
