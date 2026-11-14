// Generate the patched tree-sitter-python parser (native/grammars/tree-sitter-python, PATCHES.md) as native/build.rs does.
fn main() {
    let json = std::fs::read_to_string("grammar/grammar.json").unwrap();
    let (name, parser) = tree_sitter_generate::generate_parser_for_grammar(&json, Some((0, 25, 0)), tree_sitter_generate::OptLevel::default(), &mut Vec::new()).unwrap();
    assert_eq!(name, "python");
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let p = out.join("parser.c");
    std::fs::write(&p, parser).unwrap();
    cc::Build::new().include("grammar").file(&p).file("grammar/scanner.c").warnings(false).compile("tree-sitter-python-patched");
    println!("cargo:rerun-if-changed=grammar/grammar.json");
}
