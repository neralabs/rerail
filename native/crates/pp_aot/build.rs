//! With the `generated` feature, writes the preprocessing words' compiled code as Rust into
//! `OUT_DIR` (decision 0348, stage 4b): `native_cpu_pp::rust_source`, the JIT's functions' IR
//! through `clif_rust`, at each of `AOT`'s `#if` depths. Each carries its
//! fingerprint, which the test checks against the words the JIT compiles when it runs. Their
//! inner attributes and docs are dropped: `lib.rs` includes each inside a module and sets them.
//! The words are `rerail-c-directives`' (`expand/expand.rir`), read from its directory.

/// The builds: the program (C's and C++'s) at `#if` depth 0 and 16.
#[cfg(feature = "generated")]
const AOT: &[(&str, usize)] = &[("pp_k0", 0), ("pp_k16", 16)];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../c_directives/expand/expand.rir");
    #[cfg(feature = "generated")]
    generate();
}

#[cfg(feature = "generated")]
fn generate() {
    let rir = std::fs::read_to_string("../c_directives/expand/expand.rir").expect("reads the words");
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let words = rerail_compiler::rir_asm::assemble(&rir).expect("the words assemble");
    for &(name, k) in AOT {
        let src = rerail_compiler::native_cpu_pp::rust_source(&words, k).unwrap_or_else(|e| panic!("{name} to Rust: {e}"));
        let body: String = src.lines().filter(|l| !l.starts_with("#![") && !l.starts_with("//!")).flat_map(|l| [l, "\n"]).collect();
        std::fs::write(out.join(format!("{name}.rs")), body).expect("writes the source");
    }
}
