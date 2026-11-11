// The language registry `code_rule_dsl::language` includes (`OUT_DIR/language_registry.rs`), generated as the
// engine's build script generates it (`native/src/build_languages.rs`, the same code).
#[path = "../../src/build_languages.rs"]
mod languages;

use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let native = manifest.join("../..").canonicalize().expect("native/ is readable");
    let omitted = languages::omitted_languages(&native.parent().expect("repository root").join("languages"));
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("out dir"));
    languages::generate_registry(&native, &omitted, &out);
}
