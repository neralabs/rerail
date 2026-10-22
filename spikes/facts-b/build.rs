//! Builds Rerail's vendored, patched tree-sitter-python (the grammar the native engine parses
//! with), the way `native/build.rs` does: parser.c generated from grammar.json, then compiled with
//! the vendored scanner.
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let source = manifest.join("../../native/grammars/tree-sitter-python/src");
    for input in ["grammar.json", "scanner.c", "tree_sitter/parser.h", "tree_sitter/array.h", "tree_sitter/alloc.h"] {
        println!("cargo:rerun-if-changed={}", source.join(input).display());
    }
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let json = std::fs::read_to_string(source.join("grammar.json")).expect("grammar.json");
    let (name, parser) = tree_sitter_generate::generate_parser_for_grammar(
        &json,
        Some((0, 25, 0)),
        tree_sitter_generate::OptLevel::default(),
        &mut Vec::new(),
    )
    .expect("python parser generates");
    assert_eq!(name, "python");
    let parser_path = out.join("python_parser.c");
    std::fs::write(&parser_path, parser).unwrap();
    cc::Build::new()
        .include(&source)
        .file(parser_path)
        .file(source.join("scanner.c"))
        .warnings(false)
        .compile("tree-sitter-python-facts-b");
}
