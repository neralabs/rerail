//! With the `generated` feature, writes the preprocessing words' compiled code as Rust into
//! `OUT_DIR` (decision 0348, stage 4b): `native_cpu_pp::rust_source`, the JIT's functions' IR
//! through `clif_rust`, as `pp.rs`. It carries its fingerprint, which the test checks against
//! the words the JIT compiles when it runs. Its inner attributes and docs are dropped: `lib.rs`
//! includes it inside a module and sets them.
//! The words are `rerail-c-directives`' (`expand/expand.rir`), read from its directory.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../c_directives/expand/expand.rir");
    // The compiler makes the code: a change of it makes the build again (cargo reruns the
    // script when a build-dependency changes; this says so to every builder, Relax's too).
    println!("cargo:rerun-if-changed=../compiler/src");
    // The generated Rust is NEON's (`clif_rust`): elsewhere the path is the JIT's code (`lib.rs`).
    #[cfg(feature = "generated")]
    if std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|a| a == "aarch64") {
        generate();
    }
}

#[cfg(feature = "generated")]
fn generate() {
    let rir = std::fs::read_to_string("../c_directives/expand/expand.rir").expect("reads the words");
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let words = rerail_compiler::rir_asm::assemble(&rir).expect("the words assemble");
    let src = rerail_compiler::native_cpu_pp::rust_source(&words).unwrap_or_else(|e| panic!("the words to Rust: {e}"));
    let body: String = src.lines().filter(|l| !l.starts_with("#![") && !l.starts_with("//!")).flat_map(|l| [l, "\n"]).collect();
    // The words' text it was made from, by fingerprint: the host runs the build only for the
    // words it has (`rerail_c_directives::expand::native`).
    let words_fp = rerail_compiler::clif_rust::fingerprint(&rir);
    let abi = rerail_compiler::native_cpu_pp::ABI;
    let body = format!("{body}/// The words' text it was made from (`clif_rust::fingerprint` of `expand.rir`).\npub const WORDS: u64 = {words_fp:#x};\n/// The harness's interface it was made for (`native_cpu_pp::ABI`).\npub const ABI: u64 = {abi:#x};\n");
    std::fs::write(out.join("pp.rs"), body).expect("writes the source");
}
