//! The vendored tree-sitter grammars' build, for the build script of the crate that declares their symbols: the engine's
//! (`native/build.rs`), or, once the engine is split into crates, `rerail-lang`'s (tools/cratify writes it). Paths
//! are under `base`, the `native/` directory: Cargo reads a build script's relative paths from its own package.
use super::identity::{hex, locked_version, parser_cache_key};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Builds every vendored grammar but the `omitted` languages' (`build_languages::omitted_languages`), at the
/// tree-sitter-generate version `Cargo.lock` locks. `native` is the `native/` directory.
pub fn build_all(native: &Path, omitted: &[String]) {
    let root = native.parent().expect("repository root");
    let lock_text = std::fs::read_to_string(root.join("Cargo.lock")).expect("Cargo.lock is readable");
    // A Windows checkout with `core.autocrlf` has a CRLF Cargo.lock; locked_version reads LF lines.
    let generator = locked_version(&lock_text.replace("\r\n", "\n"), "tree-sitter-generate");
    let skipped = vendored_of(&root.join("languages"), omitted);
    for grammar in GRAMMARS.iter().filter(|grammar| !skipped.contains(&grammar.name.to_owned())) {
        build_grammar(native, grammar, &generator);
    }
}

/// The vendored grammars (`GRAMMARS` names) the `omitted` languages' dialects are parsed with.
fn vendored_of(directory: &Path, omitted: &[String]) -> Vec<String> {
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

/// The committed generated parser of a `committed_parser` grammar, in its source directory: gzip
/// level 9, in Git LFS (`.gitattributes`). Its first line is `generated_from`'s, which the build
/// checks against the grammar.json, grammar version and tree-sitter-generate it is built with.
const COMMITTED_PARSER: &str = "parser.generated.c.gz";

/// Set to `1` to regenerate every `committed_parser` grammar's COMMITTED_PARSER in the source tree,
/// as GENERATOR does after a change to its grammar.json. Any other value, `0` included, is off.
const GENERATE_PARSERS: &str = "RERAIL_GENERATE_PARSERS";

/// The generator scripts/generate/README.md lists for a committed parser, which builds with GENERATE_PARSERS=1.
const GENERATOR: &str = "scripts/generate/grammars/committed-parsers.mjs";

/// The line a committed parser starts with: what it was generated from. The grammar.json hash is
/// over LF lines, so a Windows checkout with `core.autocrlf` names the same grammar.
fn generated_from(grammar: &VendoredGrammar, json: &str, generator: &str) -> String {
    let (major, minor, patch) = grammar.version;
    format!(
        "/* Generated by {GENERATOR}; do not edit. tree-sitter-generate {generator}, version {major}.{minor}.{patch}, grammar.json sha256 {} */\n",
        hex(&Sha256::digest(json.replace("\r\n", "\n").as_bytes()))
    )
}

fn generate_parser(grammar: &VendoredGrammar, json: &str) -> String {
    let name = grammar.name;
    let (generated, parser) = tree_sitter_generate::generate_parser_for_grammar(
        json,
        Some(grammar.version),
        tree_sitter_generate::OptLevel::default(),
        &mut Vec::new(),
    )
    .unwrap_or_else(|error| panic!("{name} parser does not generate: {error}"));
    assert_eq!(generated, name);
    parser
}

/// A `committed_parser` grammar's parser: its COMMITTED_PARSER, when that was generated from this
/// grammar.json at this version by this tree-sitter-generate. Anything else fails the build with
/// the command that regenerates it, rather than spending half an hour generating it here; with
/// GENERATE_PARSERS set, it is generated and written back.
fn committed_parser(base: &Path, grammar: &VendoredGrammar, json: &str, generator: &str) -> String {
    use std::io::{Read, Write};
    let path = base.join(format!("{}/{COMMITTED_PARSER}", grammar.source)).display().to_string();
    println!("cargo:rerun-if-changed={path}");
    println!("cargo:rerun-if-env-changed={GENERATE_PARSERS}");
    let expected = generated_from(grammar, json, generator);
    if std::env::var_os(GENERATE_PARSERS).is_some_and(|value| value == "1") {
        let parser = format!("{expected}{}", generate_parser(grammar, json));
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        encoder
            .write_all(parser.as_bytes())
            .expect("gzip in memory");
        std::fs::write(&path, encoder.finish().expect("gzip in memory"))
            .unwrap_or_else(|error| panic!("{path} is not writable: {error}"));
        return parser;
    }
    let regenerate =
        format!("regenerate it with `node {GENERATOR}` (about 15 minutes) and commit {path}");
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|error| panic!("{path} is unreadable ({error}): {regenerate}"));
    if bytes.starts_with(LFS_POINTER) {
        panic!("{path} is a Git LFS pointer, not the file: run `{LFS_REMEDY}` (CONTRIBUTING.md)");
    }
    let mut parser = String::new();
    flate2::read::GzDecoder::new(bytes.as_slice())
        .read_to_string(&mut parser)
        .unwrap_or_else(|error| panic!("{path} is not a gzipped parser ({error}): {regenerate}"));
    if !parser.starts_with(&expected) {
        let found = parser.lines().next().unwrap_or("");
        panic!(
            "{path} was not generated from this grammar: it says\n  {found}\nnot\n  {}\n{regenerate}",
            expected.trim_end()
        );
    }
    parser
}

/// What a Git LFS pointer file starts with: a checkout without Git LFS holds these in place of the
/// binaries `.gitattributes` stores in LFS.
pub const LFS_POINTER: &[u8] = b"version https://git-lfs";
pub const LFS_REMEDY: &str = "git lfs install && git lfs pull";

pub fn build_grammar(base: &Path, grammar: &VendoredGrammar, generator: &str) {
    for input in grammar.inputs {
        println!("cargo:rerun-if-changed={}", base.join(input).display());
    }
    let name = grammar.name;
    let source = base.join(grammar.source).display().to_string();
    let json = std::fs::read_to_string(format!("{source}/grammar.json"))
        .unwrap_or_else(|error| panic!("{name} grammar is unreadable: {error}"));
    let parser_path = PathBuf::from(std::env::var_os("OUT_DIR").expect("out dir"))
        .join(format!("{name}_parser.c"));
    if grammar.committed_parser {
        let parser = committed_parser(base, grammar, &json, generator);
        std::fs::write(&parser_path, parser).expect("generated parser is writable");
    } else {
        // Rust-only edits rerun this script for the engine identity. Reuse a parser generated from
        // this grammar.json, version and generator, including after a bounded build was interrupted.
        let stamp_path = parser_path.with_extension("sha256");
        let key = parser_cache_key(
            generator.as_bytes(),
            json.as_bytes(),
            &format!("{:?}", grammar.version),
        );
        if !parser_path.is_file()
            || std::fs::read_to_string(&stamp_path).ok().as_deref() != Some(&key)
        {
            let parser = generate_parser(grammar, &json);
            std::fs::write(&parser_path, parser).expect("generated parser is writable");
            std::fs::write(stamp_path, key).expect("parser fingerprint is writable");
        }
    }
    // Every engine edit reruns this script for the engine identity; recompiling the grammars' C
    // each time cost ~5 s. The library is rebuilt only when its inputs or the C toolchain settings
    // change, and otherwise linked as `cc` links it.
    let library = format!("tree-sitter-{name}");
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("out dir"));
    let archive = out_dir.join(format!("lib{library}.a"));
    let compiled_stamp = out_dir.join(format!("{library}.compiled.sha256"));
    let scanner = format!("{source}/scanner.c");
    let mut inputs = Sha256::new();
    for variable in ["TARGET", "HOST", "OPT_LEVEL", "DEBUG", "PROFILE", "CC", "CFLAGS"] {
        println!("cargo:rerun-if-env-changed={variable}");
        inputs.update(variable.as_bytes());
        inputs.update(std::env::var(variable).unwrap_or_default().as_bytes());
    }
    // Python's scanner frees and reallocates its indent and delimiter stacks each time the parser restores its state
    // (`deserialize`); through the system allocator that was 6% of a parse (macOS's `free` reads the clock). With
    // this define its arrays use the allocator the parser has (`install_tree_sitter_allocator`: jemalloc). Python's
    // scanner allocates nothing else through `array.h` that it frees with `free`.
    let reuse_allocator = name == "python";
    if reuse_allocator {
        inputs.update(b"TREE_SITTER_REUSE_ALLOCATOR");
    }
    let mut headers = Vec::new();
    collect_sources(&base.join(grammar.headers), &mut headers);
    headers.sort();
    for path in std::iter::once(parser_path.clone())
        .chain(std::iter::once(PathBuf::from(&scanner)))
        .chain(headers)
    {
        inputs.update(path.to_string_lossy().as_bytes());
        inputs.update(std::fs::read(&path).expect("grammar C input is readable"));
    }
    let key = hex(&inputs.finalize());
    if archive.is_file() && std::fs::read_to_string(&compiled_stamp).ok().as_deref() == Some(&key) {
        println!("cargo:rustc-link-search=native={}", out_dir.display());
        println!("cargo:rustc-link-lib=static={library}");
        return;
    }
    let mut build = cc::Build::new();
    if reuse_allocator {
        build.define("TREE_SITTER_REUSE_ALLOCATOR", None);
    }
    build.include(base.join(grammar.headers)).file(parser_path).file(scanner).warnings(false).compile(&library);
    std::fs::write(compiled_stamp, key).expect("compiled grammar fingerprint is writable");
}

pub fn collect_sources(directory: &Path, sources: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(directory).expect("source directory is readable") {
        let path = entry.expect("source entry").path();
        if path.is_dir() {
            collect_sources(&path, sources);
        } else {
            sources.push(path);
        }
    }
}
