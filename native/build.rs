// Inside `src/` so every checkout copy of the native sources (the playground, release
// builds) carries it; the library compiles it only for tests.
#[path = "src/build_identity.rs"]
mod identity;
#[path = "src/build_languages.rs"]
mod languages;

use identity::{grammar_build_id, hex};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn main() {
    napi_build::setup();
    for grammar in GRAMMARS {
        build_grammar(grammar);
    }
    let registry = generate_languages();
    configure_fixtures();
    build_identities(&registry);
}

/// The language registry, `languages/` at the repository root: validated, and generated into
/// `OUT_DIR/language_registry.rs`. Returns each registry file and its bytes. The languages only
/// tests see (`native/test-languages/`, absent from a copy that carries none) are validated
/// against it and generated as `#[cfg(test)]` items; they are not returned, so they are no part of
/// the engine's identity.
fn generate_languages() -> Vec<(String, Vec<u8>)> {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let root = manifest.parent().expect("repository root");
    let directory = root.join("languages");
    let test_directory = manifest.join("test-languages");
    println!("cargo:rerun-if-changed={}", directory.display());
    println!("cargo:rerun-if-changed={}", test_directory.display());
    let vendored = GRAMMARS
        .iter()
        .map(|grammar| grammar.name)
        .collect::<Vec<_>>();
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("out dir"));
    languages::generate(
        &directory,
        test_directory.is_dir().then_some(test_directory.as_path()),
        &vendored,
        &out,
    )
}

/// A grammar vendored under `grammars/`. Its generated parser is not vendored (TypeScript's and
/// TSX's are 8 MB each): it is generated here from `grammar.json`, at the ABI `tree-sitter`
/// loads. Blade has none: Rerail's port of Forte parses it (`src/code_rule_dsl/blade/forte/`).
struct VendoredGrammar {
    name: &'static str,
    /// The directory holding `grammar.json` and `scanner.c`.
    source: &'static str,
    /// The directory holding `tree_sitter/parser.h`.
    headers: &'static str,
    version: (u8, u8, u8),
    /// Every file the parser is built from: a change regenerates it and changes the grammar build
    /// identity.
    inputs: &'static [&'static str],
}

const GRAMMARS: &[VendoredGrammar] = &[
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
    },
];

fn build_grammar(grammar: &VendoredGrammar) {
    for input in grammar.inputs {
        println!("cargo:rerun-if-changed={input}");
    }
    let name = grammar.name;
    let source = grammar.source;
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("out dir"));
    // The script reruns on every engine source edit (`rerun-if-changed=src`), where generating
    // and compiling the parsers took most of a rebuild: a parser whose inputs, generator and
    // compiler settings are those of the library an earlier run left in `OUT_DIR` is linked as
    // it is.
    let compiler = ["CC", "CFLAGS", "AR", "CRATE_CC_NO_DEFAULTS"];
    let target = std::env::var("TARGET").expect("target");
    let mut stamp = Sha256::new();
    stamp.update(format!("{:?}\0{}\0{source}\0", grammar.version, grammar.headers));
    for input in grammar.inputs {
        stamp.update(input.as_bytes());
        stamp.update([0]);
        stamp.update(
            std::fs::read(input).unwrap_or_else(|error| panic!("{input} is unreadable: {error}")),
        );
        stamp.update([0]);
    }
    let lock = std::fs::read("../Cargo.lock").expect("Cargo.lock is readable");
    stamp.update(&lock);
    for variable in compiler.iter().flat_map(|variable| {
        [
            (*variable).to_owned(),
            format!("{variable}_{target}"),
            format!("{variable}_{}", target.replace('-', "_")),
            format!("TARGET_{variable}"),
        ]
    }) {
        println!("cargo:rerun-if-env-changed={variable}");
        stamp.update(format!("{variable}={:?}\0", std::env::var_os(&variable)));
    }
    let stamp = hex(&stamp.finalize());
    let stamp_path = out.join(format!("{name}_parser.stamp"));
    let library = out.join(format!("libtree-sitter-{name}.a"));
    if library.is_file() && std::fs::read_to_string(&stamp_path).is_ok_and(|kept| kept == stamp) {
        println!("cargo:rustc-link-lib=static=tree-sitter-{name}");
        println!("cargo:rustc-link-search=native={}", out.display());
        return;
    }
    let json = std::fs::read_to_string(format!("{source}/grammar.json"))
        .unwrap_or_else(|error| panic!("{name} grammar is unreadable: {error}"));
    let (generated, parser) = tree_sitter_generate::generate_parser_for_grammar(
        &json,
        Some(grammar.version),
        tree_sitter_generate::OptLevel::default(),
        &mut Vec::new(),
    )
    .unwrap_or_else(|error| panic!("{name} parser does not generate: {error}"));
    assert_eq!(generated, name);
    let parser_path = out.join(format!("{name}_parser.c"));
    std::fs::write(&parser_path, parser).expect("generated parser is writable");
    cc::Build::new()
        .include(grammar.headers)
        .file(parser_path)
        .file(format!("{source}/scanner.c"))
        .warnings(false)
        .compile(&format!("tree-sitter-{name}"));
    std::fs::write(&stamp_path, stamp).expect("parser stamp is writable");
}

/// Where the engine's tests read the conformance fixtures: `RERAIL_ORACLES_DIR` if set (relative
/// to the repository root), else the repository's `fixtures/` directory. Only tests read them, so a
/// build without them (a sparse checkout) goes on; the tests that need them then fail to compile,
/// naming the fix.
fn configure_fixtures() {
    println!("cargo:rerun-if-env-changed=RERAIL_ORACLES_DIR");
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let root = manifest.parent().expect("repository root");
    let directory = match std::env::var_os("RERAIL_ORACLES_DIR") {
        Some(directory) => {
            let directory = root.join(directory);
            assert!(
                is_checkout(&directory),
                "RERAIL_ORACLES_DIR={} is not a fixtures directory",
                directory.display()
            );
            directory
        }
        None => {
            let fixtures = root.join("fixtures");
            println!("cargo:rerun-if-changed={}", fixtures.display());
            if !is_checkout(&fixtures) {
                println!(
                    "cargo:warning=Rerail fixtures not found in {}: check out the repository's fixtures/ directory, or set RERAIL_ORACLES_DIR; the engine's tests need them",
                    fixtures.display()
                );
                return;
            }
            fixtures
        }
    };
    let directory = directory
        .canonicalize()
        .expect("fixtures directory is readable");
    println!("cargo:rustc-env=RERAIL_ORACLES_DIR={}", directory.display());
    println!("cargo:rerun-if-changed={}", directory.display());
}

/// A directory with something in it: a sparse checkout can leave `fixtures/` empty.
fn is_checkout(directory: &Path) -> bool {
    std::fs::read_dir(directory).is_ok_and(|mut entries| entries.next().is_some())
}

/// Identities persisted artifacts are keyed by, derived from what built them rather than from
/// hand-bumped strings. `RERAIL_ENGINE_BUILD_ID` covers every engine source file and every locked
/// dependency, so a stored clause result is only reused by the build that would compute it.
/// `RERAIL_GRAMMAR_BUILD_ID` covers the parser and grammar versions a stored tape was parsed with.
/// The language registry is engine source: which dialects a language spans and what it supports.
fn build_identities(registry: &[(String, Vec<u8>)]) {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let lock = manifest
        .parent()
        .expect("workspace root")
        .join("Cargo.lock");

    let lock_text = std::fs::read_to_string(&lock).expect("Cargo.lock is readable");
    let mut sources = Vec::new();
    collect_sources(&manifest.join("src"), &mut sources);
    sources.sort();
    let mut engine = Sha256::new();
    engine.update(b"rerail-engine-build-v1\0");
    for path in &sources {
        let relative = path.strip_prefix(&manifest).expect("source under manifest");
        engine.update(relative.to_string_lossy().as_bytes());
        engine.update([0]);
        engine.update(std::fs::read(path).expect("engine source is readable"));
        engine.update([0]);
    }
    for (path, bytes) in registry {
        engine.update(path.as_bytes());
        engine.update([0]);
        engine.update(bytes);
        engine.update([0]);
    }
    engine.update(lock_text.as_bytes());
    // The vendored grammars are part of both identities: what a clause finds depends on the trees
    // they build, as it did on the locked grammar crates they replace.
    let mut grammar_inputs = Vec::new();
    for input in GRAMMARS.iter().flat_map(|grammar| grammar.inputs) {
        if !grammar_inputs.iter().any(|(path, _)| path == input) {
            let bytes = std::fs::read(manifest.join(input))
                .unwrap_or_else(|error| panic!("{input} is unreadable: {error}"));
            engine.update(input.as_bytes());
            engine.update([0]);
            engine.update(&bytes);
            engine.update([0]);
            grammar_inputs.push((*input, bytes));
        }
    }
    // A Blade tape is the template's Forte tree and the PHP view of it: the port, its directive
    // registry and how the view is built are part of the tape, as is the view a PHP file is
    // parsed from (`php_view`, which reads with Forte's scanners and `php_literal`, as Blade's
    // view does). Their tests are not.
    let mut view_sources = Vec::new();
    collect_sources(&manifest.join("src/code_rule_dsl/blade"), &mut view_sources);
    view_sources.push(manifest.join("src/code_rule_dsl/php_view.rs"));
    view_sources.push(manifest.join("src/code_rule_dsl/php_literal.rs"));
    view_sources.retain(|path| {
        !path
            .file_name()
            .expect("file name")
            .to_string_lossy()
            .ends_with("tests.rs")
    });
    view_sources.sort();
    let view_sources = view_sources
        .into_iter()
        .map(|path| {
            let relative = path
                .strip_prefix(&manifest)
                .expect("source under manifest")
                .to_string_lossy()
                .into_owned();
            let bytes = std::fs::read(&path).expect("view source is readable");
            (relative, bytes)
        })
        .collect::<Vec<_>>();
    println!(
        "cargo:rustc-env=RERAIL_ENGINE_BUILD_ID={}",
        hex(&engine.finalize())
    );
    let mut tape_sources: Vec<(&str, &[u8])> = grammar_inputs
        .iter()
        .map(|(path, bytes)| (*path, bytes.as_slice()))
        .collect();
    tape_sources.extend(
        view_sources
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice())),
    );
    println!(
        "cargo:rustc-env=RERAIL_GRAMMAR_BUILD_ID={}",
        grammar_build_id(&lock_text, &tape_sources)
    );
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed={}", lock.display());
}

fn collect_sources(directory: &Path, sources: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(directory).expect("source directory is readable") {
        let path = entry.expect("source entry").path();
        if path.is_dir() {
            collect_sources(&path, sources);
        } else {
            sources.push(path);
        }
    }
}
