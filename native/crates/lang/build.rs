// The language registry `code_rule_dsl::language` includes (`OUT_DIR/language_registry.rs`), generated as the
// engine's build script generates it (`native/src/build_languages.rs`, the same code).
#[path = "../../src/build_languages.rs"]
mod languages;
#[path = "../../src/build_identity.rs"]
#[allow(dead_code)]
mod identity;
#[path = "../../src/build_grammars.rs"]
#[allow(dead_code)]
mod grammars;
#[path = "../../src/build_grammars_compile.rs"]
mod grammars_compile;

use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let native = manifest.join("../..").canonicalize().expect("native/ is readable");
    let omitted = languages::omitted_languages(&native.parent().expect("repository root").join("languages"));
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("out dir"));
    languages::generate_registry(&native, &omitted, &out);
    // The vendored grammars the registry declares (`tree_sitter_<name>`), built here so that every binary that
    // reaches the registry links them (`src/build_grammars.rs`).
    let lock = std::fs::read_to_string(native.parent().expect("repository root").join("Cargo.lock")).expect("Cargo.lock is readable");
    println!("cargo:rerun-if-changed={}", native.parent().expect("repository root").join("Cargo.lock").display());
    // A Windows checkout with `core.autocrlf` has a CRLF Cargo.lock; locked_version reads LF lines.
    let generator = identity::locked_version(&lock.replace("\r\n", "\n"), "tree-sitter-generate");
    let skipped = grammars::vendored_of(&native, &omitted);
    for grammar in grammars::GRAMMARS.iter().filter(|grammar| !skipped.contains(&grammar.name.to_owned())) {
        grammars_compile::build_grammar(&native, grammar, &generator);
    }
}
