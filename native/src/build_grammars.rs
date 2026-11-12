//! The vendored tree-sitter grammars: their list. `rerail-lang`'s build script compiles them
//! (`build_grammars_compile.rs`): it is the crate that declares their `tree_sitter_<name>` symbols (its generated
//! language registry), so every crate's binary that reaches the registry links them. The engine's build script reads
//! the list for its identity. Paths are relative to `native/` (`root`).
use std::path::Path;

/// The vendored grammars (`GRAMMARS` names) the `omitted` languages' dialects are parsed with.
pub fn vendored_of(root: &Path, omitted: &[String]) -> Vec<String> {
    let directory = root.parent().expect("repository root").join("languages");
    omitted
        .iter()
        .flat_map(|id| {
            let value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(directory.join(format!("{id}.json"))).expect("registry file"))
                    .expect("registry JSON");
            value["dialects"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|dialect| dialect["grammar"].as_str()?.strip_prefix("vendored:").map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// A grammar vendored under `grammars/`. Its generated parser is not vendored (TypeScript's and
/// TSX's are 8 MB each): it is generated here from `grammar.json`, at the ABI `tree-sitter`
/// loads. Blade has none: Rerail's port of Forte parses it (`src/code_rule_dsl/blade/forte/`).
/// The one exception is a grammar whose generation is too costly to repeat on every fresh build
/// (`committed_parser`).
pub struct VendoredGrammar {
    pub name: &'static str,
    /// The directory holding `grammar.json` and `scanner.c`.
    pub source: &'static str,
    /// The directory holding `tree_sitter/parser.h`.
    pub headers: &'static str,
    pub version: (u8, u8, u8),
    /// Every file the parser is built from: a change regenerates it and changes the grammar build
    /// identity.
    pub inputs: &'static [&'static str],
    /// Whether the generated parser is committed beside `grammar.json` as `parser.generated.c.gz`
    /// (COMMITTED_PARSER) instead of generated on every build. SQL's: tree-sitter-generate takes
    /// 7 to 10 minutes optimised, and about 30 minutes as a build script compiles it.
    pub committed_parser: bool,
}

pub const GRAMMARS: &[VendoredGrammar] = &[
    // tree-sitter-grammars/tree-sitter-vue, commit ce8011a414fdf8091f4e4071752efc376f4afb08 (no release;
    // package.json 0.1.0).
    VendoredGrammar {
        name: "vue",
        source: "grammars/tree-sitter-vue/src",
        headers: "grammars/tree-sitter-vue/src",
        version: (0, 1, 0),
        inputs: &[
            "grammars/tree-sitter-vue/src/grammar.json",
            "grammars/tree-sitter-vue/src/scanner.c",
            "grammars/tree-sitter-vue/src/tag.h",
            "grammars/tree-sitter-vue/src/tree_sitter/alloc.h",
            "grammars/tree-sitter-vue/src/tree_sitter/array.h",
            "grammars/tree-sitter-vue/src/tree_sitter/parser.h",
        ],
        committed_parser: false,
    },
    // tree-sitter-python 0.25.0 with the patches in grammars/tree-sitter-python/PATCHES.md.
    VendoredGrammar {
        name: "python",
        source: "grammars/tree-sitter-python/src",
        headers: "grammars/tree-sitter-python/src",
        version: (0, 25, 0),
        inputs: &[
            "grammars/tree-sitter-python/src/grammar.json",
            "grammars/tree-sitter-python/src/scanner.c",
            "grammars/tree-sitter-python/src/tree_sitter/alloc.h",
            "grammars/tree-sitter-python/src/tree_sitter/array.h",
            "grammars/tree-sitter-python/src/tree_sitter/parser.h",
        ],
        committed_parser: false,
    },
    // tree-sitter-sql (DerekStride) 0.3.11, commit 7b51ecda191d36b92f5a90a8d1bc3faef1c7b8b8, with
    // the patches in grammars/tree-sitter-sql/PATCHES.md.
    VendoredGrammar {
        name: "sql",
        source: "grammars/tree-sitter-sql/src",
        headers: "grammars/tree-sitter-sql/src",
        version: (0, 3, 11),
        inputs: &[
            "grammars/tree-sitter-sql/src/grammar.json",
            "grammars/tree-sitter-sql/src/scanner.c",
            "grammars/tree-sitter-sql/src/tree_sitter/alloc.h",
            "grammars/tree-sitter-sql/src/tree_sitter/array.h",
            "grammars/tree-sitter-sql/src/tree_sitter/parser.h",
        ],
        committed_parser: true,
    },
    // tree-sitter-typescript 0.23.2 with the patch in grammars/tree-sitter-typescript/PATCHES.md.
    VendoredGrammar {
        name: "typescript",
        source: "grammars/tree-sitter-typescript/typescript/src",
        headers: "grammars/tree-sitter-typescript/common",
        version: (0, 23, 2),
        inputs: &[
            "grammars/tree-sitter-typescript/typescript/src/grammar.json",
            "grammars/tree-sitter-typescript/typescript/src/scanner.c",
            "grammars/tree-sitter-typescript/common/scanner.h",
            "grammars/tree-sitter-typescript/common/tree_sitter/alloc.h",
            "grammars/tree-sitter-typescript/common/tree_sitter/array.h",
            "grammars/tree-sitter-typescript/common/tree_sitter/parser.h",
        ],
        committed_parser: false,
    },
    VendoredGrammar {
        name: "tsx",
        source: "grammars/tree-sitter-typescript/tsx/src",
        headers: "grammars/tree-sitter-typescript/common",
        version: (0, 23, 2),
        inputs: &[
            "grammars/tree-sitter-typescript/tsx/src/grammar.json",
            "grammars/tree-sitter-typescript/tsx/src/scanner.c",
            "grammars/tree-sitter-typescript/common/scanner.h",
            "grammars/tree-sitter-typescript/common/tree_sitter/alloc.h",
            "grammars/tree-sitter-typescript/common/tree_sitter/array.h",
            "grammars/tree-sitter-typescript/common/tree_sitter/parser.h",
        ],
        committed_parser: false,
    },
    // tree-sitter-javascript 0.25.0 with the patches in grammars/tree-sitter-javascript/PATCHES.md.
    VendoredGrammar {
        name: "javascript",
        source: "grammars/tree-sitter-javascript/src",
        headers: "grammars/tree-sitter-javascript/src",
        version: (0, 25, 0),
        inputs: &[
            "grammars/tree-sitter-javascript/src/grammar.json",
            "grammars/tree-sitter-javascript/src/scanner.c",
            "grammars/tree-sitter-javascript/src/tree_sitter/alloc.h",
            "grammars/tree-sitter-javascript/src/tree_sitter/array.h",
            "grammars/tree-sitter-javascript/src/tree_sitter/parser.h",
        ],
        committed_parser: false,
    },
];

/// What a Git LFS pointer file starts with: a checkout without Git LFS holds these in place of the

pub const LFS_POINTER: &[u8] = b"version https://git-lfs";
pub const LFS_REMEDY: &str = "git lfs install && git lfs pull";

