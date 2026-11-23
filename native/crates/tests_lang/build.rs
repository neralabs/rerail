// `with_adapter!` for the adapter kit (`language/adapter/kit.rs` includes `OUT_DIR/with_adapter.rs`): the language
// registry generated as the lang crate's build script generates it (`native/src/build_languages.rs`, the same code).
#[path = "../../src/build_languages.rs"]
#[allow(dead_code)]
mod languages;

use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    // `native/` is two levels up (not `canonicalize()`: on Windows that gives a `\\?\` path).
    let native = manifest.ancestors().nth(2).expect("native/ is the manifest's grandparent").to_path_buf();
    let omitted = languages::omitted_languages(&native.parent().expect("repository root").join("languages"));
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("out dir"));
    languages::generate_registry(&native, &omitted, &out);
}
