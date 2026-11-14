// The language registry `code_rule_dsl::language` includes (`OUT_DIR/language_registry.rs`), generated as the
// engine's build script generates it (`native/src/build_languages.rs`, the same code), and the vendored grammars the
// registry declares (`native/src/build_grammars.rs`): built here, by the crate that declares their symbols, so every
// crate's tests link them (the engine's build script leaves them to this one once it is split).
#[path = "../../src/build_identity.rs"]
#[allow(dead_code)]
mod identity;
#[path = "../../src/build_languages.rs"]
mod languages;
#[path = "../../src/build_grammars.rs"]
mod grammars;

use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    // `native/` is two levels up. Not `canonicalize()`: on Windows that gives a `\\?\` path, in which the grammar
    // build's `/`-joined paths are invalid (os error 123).
    let native = manifest.ancestors().nth(2).expect("native/ is the manifest's grandparent").to_path_buf();
    let omitted = languages::omitted_languages(&native.parent().expect("repository root").join("languages"));
    grammars::build_all(&native, &omitted);
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("out dir"));
    languages::generate_registry(&native, &omitted, &out);
}
