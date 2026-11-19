//! Generates and compiles the parsers of the grammars vendored in native/grammars, as native/build.rs does (same
//! grammar.json, same ABI version), so the facts are the engine's trees. SQL's parser is committed gzipped.
use std::io::Read;
use std::path::PathBuf;

struct Grammar {
    name: &'static str,
    source: &'static str,
    headers: &'static str,
    version: (u8, u8, u8),
    committed: bool,
}

const G: &str = "../../../../native/grammars";

const GRAMMARS: &[Grammar] = &[
    Grammar { name: "javascript", source: "tree-sitter-javascript/src", headers: "tree-sitter-javascript/src", version: (0, 25, 0), committed: false },
    Grammar { name: "typescript", source: "tree-sitter-typescript/typescript/src", headers: "tree-sitter-typescript/common", version: (0, 23, 2), committed: false },
    Grammar { name: "tsx", source: "tree-sitter-typescript/tsx/src", headers: "tree-sitter-typescript/common", version: (0, 23, 2), committed: false },
    Grammar { name: "python", source: "tree-sitter-python/src", headers: "tree-sitter-python/src", version: (0, 25, 0), committed: false },
    Grammar { name: "vue", source: "tree-sitter-vue/src", headers: "tree-sitter-vue/src", version: (0, 1, 0), committed: false },
    Grammar { name: "sql", source: "tree-sitter-sql/src", headers: "tree-sitter-sql/src", version: (0, 3, 11), committed: true },
];

fn main() {
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    for g in GRAMMARS {
        let source = format!("{G}/{}", g.source);
        let headers = format!("{G}/{}", g.headers);
        println!("cargo:rerun-if-changed={source}/grammar.json");
        println!("cargo:rerun-if-changed={source}/scanner.c");
        let json = std::fs::read_to_string(format!("{source}/grammar.json")).expect("grammar.json");
        let parser = if g.committed {
            let bytes = std::fs::read(format!("{source}/parser.generated.c.gz")).expect("committed parser");
            assert!(!bytes.starts_with(b"version https://git-lfs"), "{}: a Git LFS pointer; run git lfs pull", g.name);
            let mut s = String::new();
            flate2::read::GzDecoder::new(bytes.as_slice()).read_to_string(&mut s).expect("gzip");
            s
        } else {
            let (name, parser) = tree_sitter_generate::generate_parser_for_grammar(
                &json, Some(g.version), tree_sitter_generate::OptLevel::default(), &mut Vec::new())
                .unwrap_or_else(|e| panic!("{} does not generate: {e}", g.name));
            assert_eq!(name, g.name);
            parser
        };
        let path = out.join(format!("{}_parser.c", g.name));
        std::fs::write(&path, parser).unwrap();
        cc::Build::new().include(&headers).include(&source).file(&path).file(format!("{source}/scanner.c"))
            .warnings(false).compile(&format!("rrlab-ts-{}", g.name));
    }
}
