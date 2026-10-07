// Inside `src/` so every checkout copy of the native sources (the dev container, the
// playground, release builds) carries it; the library compiles it only for tests.
#[path = "src/build_identity.rs"]
mod identity;

use identity::{grammar_build_id, hex};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn main() {
    napi_build::setup();
    build_blade_grammar();
    configure_oracles();
    build_identities();
}

const BLADE_SOURCE: &str = "grammars/tree-sitter-blade/src";

/// tree-sitter-blade 0.12.2, commit b5291d1ba207a8ebb8383b2ecb8a8a6535210a50. Its 22 MB generated
/// parser is not vendored: it is generated here from the grammar, at the ABI `tree-sitter` loads.
fn build_blade_grammar() {
    for file in [
        "grammar.json",
        "scanner.c",
        "tag.h",
        "tree_sitter/alloc.h",
        "tree_sitter/array.h",
        "tree_sitter/parser.h",
    ] {
        println!("cargo:rerun-if-changed={BLADE_SOURCE}/{file}");
    }
    let grammar = std::fs::read_to_string(format!("{BLADE_SOURCE}/grammar.json"))
        .expect("Blade grammar is readable");
    let (name, parser) = tree_sitter_generate::generate_parser_for_grammar(
        &grammar,
        Some((0, 12, 2)),
        tree_sitter_generate::OptLevel::default(),
        &mut Vec::new(),
    )
    .expect("Blade parser generates");
    assert_eq!(name, "blade");
    let parser_path =
        PathBuf::from(std::env::var_os("OUT_DIR").expect("out dir")).join("blade_parser.c");
    std::fs::write(&parser_path, parser).expect("Blade parser is writable");
    cc::Build::new()
        .include(BLADE_SOURCE)
        .file(parser_path)
        .file(format!("{BLADE_SOURCE}/scanner.c"))
        .warnings(false)
        .compile("tree-sitter-blade");
}

fn configure_oracles() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    for directory in [
        manifest.join("../../rerail-fixtures"),
        manifest.join("../rerail-fixtures"),
    ] {
        if directory.is_dir() {
            let oracle_dir = directory
                .canonicalize()
                .expect("oracle directory is readable");
            println!("cargo:rustc-env=RERAIL_FIXTURES_DIR={}", oracle_dir.display());
            println!("cargo:rerun-if-changed={}", oracle_dir.display());
            return;
        }
    }
    println!(
        "cargo:warning=Rerail conformance oracles not found; test-only oracle fixtures are unavailable"
    );
}

/// Identities persisted artifacts are keyed by, derived from what built them rather than from
/// hand-bumped strings. `RERAIL_ENGINE_BUILD_ID` covers every engine source file and every locked
/// dependency, so a stored check result is only reused by the build that would compute it.
/// `RERAIL_GRAMMAR_BUILD_ID` covers the parser and grammar versions a stored tape was parsed with.
fn build_identities() {
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
    engine.update(lock_text.as_bytes());
    let blade_source = |file: &str| {
        std::fs::read(manifest.join(BLADE_SOURCE).join(file)).expect("Blade source is readable")
    };
    let blade_grammar = blade_source("grammar.json");
    let blade_scanner = blade_source("scanner.c");
    let blade_tag = blade_source("tag.h");
    let blade_view =
        std::fs::read(manifest.join("src/policy_dsl/blade/view.rs")).expect("view is readable");
    println!(
        "cargo:rustc-env=RERAIL_ENGINE_BUILD_ID={}",
        hex(&engine.finalize())
    );
    println!(
        "cargo:rustc-env=RERAIL_GRAMMAR_BUILD_ID={}",
        grammar_build_id(
            &lock_text,
            &[
                // The Blade parser is generated at build time: its grammar and scanner are part
                // of the tape.
                ("grammar.json", &blade_grammar),
                ("scanner.c", &blade_scanner),
                ("tag.h", &blade_tag),
                // A Blade tape holds the PHP view of its template: how the view is built is part
                // of the tape.
                ("blade-view", &blade_view),
            ],
        )
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
